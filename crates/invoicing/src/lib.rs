//! Customers and suppliers (kunder och leverantörer) of a company,
//! event-sourced into SQLite. Fakturor will join them here.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, load
//! the register, decide, append, project. A number is decided inside that
//! transaction, so concurrent writers never share one.

mod customer_invoice_store;
pub mod customer_invoices;
pub mod domain;
pub mod invoices;
mod projections;
pub mod supplier_invoices;
pub mod vat;

use domain::{
    Change, CustomerDetails, CustomerEvent, CustomerForm, DomainError, Party, PartyDetails,
    Register, SupplierDetails, SupplierEvent, SupplierForm,
};
use doris_company::domain::{AccountingMethod, Company};
use doris_eventstore::{Metadata, NewEvent};
use doris_ledger::domain::{Attachment, DomainError as LedgerError, RecordVoucher};
use doris_ledger::{NewAttachment, VoucherRef};
use jiff::civil::Date;
use projections::{CUSTOMERS, SUPPLIERS, Table};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::{SqliteConnection, SqlitePool};
use supplier_invoices::{
    NewSupplierInvoice, Registration, SupplierInvoice, SupplierInvoiceEvent, SupplierInvoices,
    SupplierSnapshot,
};
use uuid::Uuid;

pub use customer_invoice_store::{
    cancel_customer_invoice, customer_invoice_attachment, list_customer_invoices,
    pay_customer_invoice, register_customer_invoice, reverse_customer_invoice_payment,
};
pub use projections::rebuild_projections;

const CUSTOMERS_STREAM: &str = "customers-";
const SUPPLIERS_STREAM: &str = "suppliers-";
const SUPPLIER_INVOICES_STREAM: &str = "supplier-invoices-";
const CUSTOMER_INVOICES_STREAM: &str = "customer-invoices-";
const SCHEMA_VERSION: i64 = 1;

pub type Customer = Party<CustomerDetails>;
pub type Supplier = Party<SupplierDetails>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// A booking the ledger refused (inactive account, closed year, …).
    #[error(transparent)]
    Ledger(#[from] doris_ledger::Error),
    #[error(transparent)]
    Store(#[from] doris_eventstore::Error),
}

fn ledger(err: LedgerError) -> Error {
    Error::Ledger(doris_ledger::Error::Domain(err))
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<doris_company::Error> for Error {
    fn from(err: doris_company::Error) -> Self {
        match err {
            doris_company::Error::Store(err) => Error::Store(err),
            _ => Error::NotFound,
        }
    }
}

pub async fn list_customers(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Customer>> {
    list(pool, &CUSTOMERS, company_id, actor).await
}

pub async fn add_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &CustomerForm<'_>,
) -> Result<u32> {
    let details = CustomerDetails::parse(form)?;
    let changes = change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &CustomerForm<'_>,
) -> Result<()> {
    let details = CustomerDetails::parse(form)?;
    change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_customer_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<CustomerDetails, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

pub async fn list_suppliers(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Supplier>> {
    list(pool, &SUPPLIERS, company_id, actor).await
}

pub async fn add_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &SupplierForm<'_>,
) -> Result<u32> {
    let details = SupplierDetails::parse(form)?;
    let changes = change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &SupplierForm<'_>,
) -> Result<()> {
    let details = SupplierDetails::parse(form)?;
    change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_supplier_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<SupplierDetails, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of parties.
async fn list<D: DeserializeOwned>(
    pool: &SqlitePool,
    table: &Table,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Party<D>>> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(table.list)
        .bind(company_id.to_string())
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|(number, details, active)| {
            Ok(Party {
                number: u32::try_from(number).expect("projected numbers are u32"),
                details: serde_json::from_str(&details)?,
                active,
            })
        })
        .collect()
}

/// Loads one register's stream, decides, appends and projects, all in one
/// IMMEDIATE transaction. Returns the changes that were recorded.
async fn change<D, E>(
    pool: &SqlitePool,
    stream_prefix: &str,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Register<D>) -> Result<Vec<Change<D>>, DomainError>,
) -> Result<Vec<Change<D>>>
where
    D: PartyDetails,
    E: Serialize + DeserializeOwned + From<Change<D>> + Into<Change<D>>,
{
    let mut tx = doris_eventstore::begin(pool).await?;
    doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let stream = format!("{stream_prefix}{company_id}");
    let (history, version) = load::<E>(&mut tx, &stream).await?;
    let changes = decide(&Register::from_changes(history.into_iter().map(Into::into)))?;
    let events: Vec<E> = changes.iter().cloned().map(E::from).collect();
    append(&mut tx, &stream, version, &events, actor).await?;
    tx.commit().await?;
    Ok(changes)
}

/// Appends events and updates projections within the caller's transaction.
async fn append<E: Serialize>(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[E],
    actor: Uuid,
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: Some(actor.to_string()),
    };
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}

/// A stream's events and its version, in the caller's transaction.
async fn load<E: DeserializeOwned>(
    conn: &mut SqliteConnection,
    stream: &str,
) -> Result<(Vec<E>, i64)> {
    let version = doris_eventstore::stream_version(conn, stream).await?;
    let events = doris_eventstore::load(conn, stream)
        .await?
        .iter()
        .map(|e| e.decode::<E>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((events, version))
}

fn invoices_stream(company_id: Uuid) -> String {
    format!("{SUPPLIER_INVOICES_STREAM}{company_id}")
}

async fn load_invoices(
    conn: &mut SqliteConnection,
    company_id: Uuid,
) -> Result<(SupplierInvoices, i64)> {
    let (events, version) =
        load::<SupplierInvoiceEvent>(conn, &invoices_stream(company_id)).await?;
    Ok((SupplierInvoices::from_events(events), version))
}

/// Registers a supplier invoice and, under faktureringsmetoden, books it
/// with its underlag, all in one transaction.
pub async fn register_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    supplier: u32,
    new: NewSupplierInvoice<'_>,
    attachments: Vec<NewAttachment>,
    today: Date,
) -> Result<u32> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (supplier_events, _) =
        load::<SupplierEvent>(&mut tx, &format!("{SUPPLIERS_STREAM}{company_id}")).await?;
    let suppliers = Register::from_changes(supplier_events.into_iter().map(Into::into));
    let supplier = suppliers
        .get(supplier)
        .ok_or(DomainError::SupplierNotFound)?;
    let invoice = Registration::new(SupplierSnapshot::of(supplier)?, &new)?;
    if invoice.invoice_date > today {
        return Err(DomainError::InvoiceDateInFuture.into());
    }
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let number = supplier_invoices::register(&state, &invoice)?;
    let stored = store_all(&mut tx, attachments).await?;
    let voucher = match company.accounting_method {
        AccountingMethod::Invoice => {
            let booked = doris_ledger::record_voucher_in(
                &mut tx,
                company_id,
                actor,
                RecordVoucher {
                    date: invoice.invoice_date,
                    text: supplier_invoices::text(number, &invoice),
                    lines: supplier_invoices::registration_lines(&invoice),
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
    let event = SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments: stored,
        voucher,
    };
    append(
        &mut tx,
        &invoices_stream(company_id),
        version,
        &[event],
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(number)
}

/// Books the payment of an unpaid invoice. Under kontantmetoden that is
/// the whole cost, and the underlag go on the payment voucher.
pub async fn pay_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    date: Date,
    account: u32,
    today: Date,
) -> Result<()> {
    let account = supplier_invoices::payment_account(account)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = supplier_invoices::unpaid(&state, number)?;
    let lines =
        supplier_invoices::payment_lines(&invoice.invoice, company.accounting_method, account);
    let voucher = doris_ledger::record_voucher_in(
        &mut tx,
        company_id,
        actor,
        RecordVoucher {
            date,
            text: supplier_invoices::text(number, &invoice.invoice),
            lines,
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
    let event = SupplierInvoiceEvent::SupplierInvoicePaid {
        number,
        date,
        account,
        voucher,
    };
    append(
        &mut tx,
        &invoices_stream(company_id),
        version,
        &[event],
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Cancels an unpaid invoice; under faktureringsmetoden its registration
/// voucher is corrected.
pub async fn cancel_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason: &str,
    today: Date,
) -> Result<()> {
    let reason = supplier_invoices::reason(reason)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = supplier_invoices::unpaid(&state, number)?;
    let voucher = match invoice.registration_voucher {
        Some(registered) => Some(correct(&mut tx, &company, actor, registered, today).await?),
        None => None,
    };
    let event = SupplierInvoiceEvent::SupplierInvoiceCancelled {
        number,
        reason,
        voucher,
    };
    append(
        &mut tx,
        &invoices_stream(company_id),
        version,
        &[event],
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Corrects the payment voucher of a paid invoice, which is unpaid again.
pub async fn reverse_supplier_invoice_payment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason: &str,
    today: Date,
) -> Result<()> {
    let reason = supplier_invoices::reason(reason)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let (_, payment) = supplier_invoices::paid(&state, number)?;
    let voucher = correct(&mut tx, &company, actor, payment, today).await?;
    let event = SupplierInvoiceEvent::SupplierInvoicePaymentReversed {
        number,
        reason,
        voucher,
    };
    append(
        &mut tx,
        &invoices_stream(company_id),
        version,
        &[event],
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of invoices.
/// The company's supplier invoices, newest first.
pub async fn list_supplier_invoices(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<SupplierInvoice>> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT details FROM supplier_invoices WHERE company_id = ? ORDER BY number DESC",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|details| Ok(serde_json::from_str(details)?))
        .collect()
}

/// An underlag of the company's own invoice `number`: found in that
/// invoice first, never by its hash alone.
pub async fn supplier_invoice_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let details: Option<String> = sqlx::query_scalar(
        "SELECT details FROM supplier_invoices WHERE company_id = ? AND number = ?",
    )
    .bind(company_id.to_string())
    .bind(number)
    .fetch_optional(pool)
    .await?;
    let invoice: Option<SupplierInvoice> = details.map(|d| serde_json::from_str(&d)).transpose()?;
    let attachment = invoice
        .and_then(|i| i.attachments.into_iter().find(|a| a.sha256 == sha256))
        .ok_or_else(|| ledger(LedgerError::AttachmentNotFound))?;
    // Found on the company's own invoice above; the ledger keeps the bytes.
    let data = doris_ledger::attachment_data(pool, &attachment.sha256).await?;
    Ok((attachment, data))
}

/// Stores each underlag in the caller's transaction; the same file twice in
/// one request is `duplicate_attachment`.
async fn store_all(
    conn: &mut SqliteConnection,
    attachments: Vec<NewAttachment>,
) -> Result<Vec<Attachment>> {
    let mut stored: Vec<Attachment> = Vec::new();
    for new in attachments {
        let attachment = doris_ledger::store_attachment_in(conn, new).await?;
        if stored.iter().any(|s| s.sha256 == attachment.sha256) {
            return Err(ledger(LedgerError::DuplicateAttachment));
        }
        stored.push(attachment);
    }
    Ok(stored)
}

async fn link_all(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    voucher: VoucherRef,
    attachments: &[Attachment],
    today: Date,
) -> Result<()> {
    for attachment in attachments {
        doris_ledger::link_attachment_in(
            conn,
            company_id,
            actor,
            voucher.fiscal_year_start,
            voucher.number,
            attachment.clone(),
            today,
        )
        .await?;
    }
    Ok(())
}

/// Corrects `voucher`, dated today or its fiscal year's last day. A
/// voucher already corrected by hand in the grundbok keeps that correction,
/// so the invoice follows the ledger instead of getting stuck.
async fn correct(
    conn: &mut SqliteConnection,
    company: &Company,
    actor: Uuid,
    voucher: VoucherRef,
    today: Date,
) -> Result<VoucherRef> {
    if let Some(existing) = doris_ledger::correction_of_in(
        conn,
        company.id,
        actor,
        voucher.fiscal_year_start,
        voucher.number,
        today,
    )
    .await?
    {
        return Ok(existing);
    }
    let end = company
        .first_fiscal_year
        .containing(voucher.fiscal_year_start)
        .end;
    Ok(doris_ledger::correct_voucher_in(
        conn,
        company.id,
        actor,
        voucher.fiscal_year_start,
        voucher.number,
        supplier_invoices::correction_date(end, today),
        today,
    )
    .await?)
}
