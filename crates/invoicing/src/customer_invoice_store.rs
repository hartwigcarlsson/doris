//! Customer invoices in the database. Each write books its vouchers and
//! appends its event in one IMMEDIATE transaction.

use crate::customer_invoices::{
    self, CustomerInvoice, CustomerInvoiceEvent, CustomerInvoices, CustomerRegistration,
    CustomerSnapshot, NewCustomerInvoice,
};
use crate::domain::{CustomerEvent, DomainError, Register};
use crate::supplier_invoices::{payment_account, reason};
use crate::{
    CUSTOMER_INVOICES_STREAM, CUSTOMERS_STREAM, Result, append, correct, ledger, link_all, load,
    store_all,
};
use doris_company::domain::AccountingMethod;
use doris_ledger::NewAttachment;
use doris_ledger::domain::{Attachment, DomainError as LedgerError, RecordVoucher};
use jiff::civil::Date;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

fn stream(company_id: Uuid) -> String {
    format!("{CUSTOMER_INVOICES_STREAM}{company_id}")
}

async fn load_invoices(
    conn: &mut SqliteConnection,
    company_id: Uuid,
) -> Result<(CustomerInvoices, i64)> {
    let (events, version) = load::<CustomerInvoiceEvent>(conn, &stream(company_id)).await?;
    Ok((CustomerInvoices::from_events(events), version))
}

/// Registers a customer invoice and, under faktureringsmetoden, books it
/// with its underlag, all in one transaction.
pub async fn register_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    customer: u32,
    new: NewCustomerInvoice<'_>,
    attachments: Vec<NewAttachment>,
    today: Date,
) -> Result<u32> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (customer_events, _) =
        load::<CustomerEvent>(&mut tx, &format!("{CUSTOMERS_STREAM}{company_id}")).await?;
    let customers = Register::from_changes(customer_events.into_iter().map(Into::into));
    let customer = customers
        .get(customer)
        .ok_or(DomainError::CustomerNotFound)?;
    let invoice = CustomerRegistration::new(CustomerSnapshot::of(customer)?, &new)?;
    if invoice.invoice_date > today {
        return Err(DomainError::InvoiceDateInFuture.into());
    }
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let number = customer_invoices::register(&state, &invoice)?;
    let stored = store_all(&mut tx, attachments).await?;
    let voucher = match company.accounting_method {
        AccountingMethod::Invoice => {
            let booked = doris_ledger::record_voucher_in(
                &mut tx,
                company_id,
                actor,
                RecordVoucher {
                    date: invoice.invoice_date,
                    text: customer_invoices::text(&invoice),
                    lines: customer_invoices::registration_lines(&invoice),
                },
                today,
            )
            .await?;
            link_all(&mut tx, company_id, actor, booked, &stored, today).await?;
            Some(booked)
        }
        AccountingMethod::Cash => {
            let accounts: Vec<_> = invoice.lines.iter().map(|l| l.account).collect();
            doris_ledger::check_accounts_in(&mut tx, company_id, actor, &accounts).await?;
            None
        }
    };
    let event = CustomerInvoiceEvent::CustomerInvoiceRegistered {
        number,
        invoice,
        attachments: stored,
        voucher,
    };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(number)
}

/// Books the payment of an unpaid invoice. Under kontantmetoden that is the
/// income and VAT, and the underlag go on the payment voucher.
pub async fn pay_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    date: Date,
    account: u32,
    today: Date,
) -> Result<()> {
    let account = payment_account(account)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = customer_invoices::unpaid(&state, number)?;
    let voucher = doris_ledger::record_voucher_in(
        &mut tx,
        company_id,
        actor,
        RecordVoucher {
            date,
            text: customer_invoices::text(&invoice.invoice),
            lines: customer_invoices::payment_lines(
                &invoice.invoice,
                company.accounting_method,
                account,
            ),
        },
        today,
    )
    .await?;
    if company.accounting_method == AccountingMethod::Cash {
        link_all(
            &mut tx,
            company_id,
            actor,
            voucher,
            &invoice.attachments,
            today,
        )
        .await?;
    }
    let event = CustomerInvoiceEvent::CustomerInvoicePaid {
        number,
        date,
        account,
        voucher,
    };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Cancels an unpaid invoice; under faktureringsmetoden its registration
/// voucher is corrected. Its number stays used.
pub async fn cancel_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason_text: &str,
    today: Date,
) -> Result<()> {
    let reason = reason(reason_text)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = customer_invoices::unpaid(&state, number)?;
    let voucher = match invoice.registration_voucher {
        Some(registered) => Some(correct(&mut tx, &company, actor, registered, today).await?),
        None => None,
    };
    let event = CustomerInvoiceEvent::CustomerInvoiceCancelled {
        number,
        reason,
        voucher,
    };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Corrects the payment voucher of a paid invoice, which is unpaid again.
pub async fn reverse_customer_invoice_payment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason_text: &str,
    today: Date,
) -> Result<()> {
    let reason = reason(reason_text)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let (_, payment) = customer_invoices::paid(&state, number)?;
    let voucher = correct(&mut tx, &company, actor, payment, today).await?;
    let event = CustomerInvoiceEvent::CustomerInvoicePaymentReversed {
        number,
        reason,
        voucher,
    };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of invoices.
/// The company's customer invoices, newest first, and the next invoice
/// number to propose.
pub async fn list_customer_invoices(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<(Vec<CustomerInvoice>, String)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT invoice_number, details FROM customer_invoices WHERE company_id = ? ORDER BY number DESC",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    let next = customer_invoices::next_invoice_number(rows.iter().map(|(n, _)| n.as_str()));
    let invoices = rows
        .iter()
        .map(|(_, details)| Ok(serde_json::from_str(details)?))
        .collect::<Result<Vec<_>>>()?;
    Ok((invoices, next))
}

/// An underlag of the company's own invoice `number`: found in that invoice
/// first, never by its hash alone.
pub async fn customer_invoice_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let details: Option<String> = sqlx::query_scalar(
        "SELECT details FROM customer_invoices WHERE company_id = ? AND number = ?",
    )
    .bind(company_id.to_string())
    .bind(number)
    .fetch_optional(pool)
    .await?;
    let invoice: Option<CustomerInvoice> = details.map(|d| serde_json::from_str(&d)).transpose()?;
    let attachment = invoice
        .and_then(|i| i.attachments.into_iter().find(|a| a.sha256 == sha256))
        .ok_or_else(|| ledger(LedgerError::AttachmentNotFound))?;
    // Found on the company's own invoice above; the ledger keeps the bytes.
    let data = doris_ledger::attachment_data(pool, &attachment.sha256).await?;
    Ok((attachment, data))
}
