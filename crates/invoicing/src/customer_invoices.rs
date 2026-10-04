//! Pure rules for customer invoices (kundfakturor): the values, the events,
//! the state and the vouchers Doris books for them. No I/O.

use crate::domain::{CustomerDetails, DomainError, Email, Party, PartyName, VatNumber, optional};
use crate::invoices::{self, InvoiceKind, Side, Status};
use crate::supplier_invoices::{InvoiceNumber, PaymentReference};
use crate::vat::{self, InvoiceLine, VatAmount, VatRate};
use doris_company::domain::{AccountingMethod, Address, OrgNr};
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, Attachment, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACCOUNTS_RECEIVABLE: u32 = 1510;

fn account(number: u32) -> AccountNumber {
    AccountNumber::parse(number).expect("a BAS account")
}

/// Where output VAT goes: 2611, 2621 or 2631; nothing at 0 %.
pub fn output_vat_account(rate: VatRate) -> Option<AccountNumber> {
    match rate {
        VatRate::Rate25 => Some(account(2611)),
        VatRate::Rate12 => Some(account(2621)),
        VatRate::Rate6 => Some(account(2631)),
        VatRate::Rate0 => None,
    }
}

/// Customer invoices' transition errors.
pub struct CustomerInvoiceKind;

impl InvoiceKind for CustomerInvoiceKind {
    const PAID: DomainError = DomainError::CustomerInvoicePaid;
    const NOT_PAID: DomainError = DomainError::CustomerInvoiceNotPaid;
    const CANCELLED: DomainError = DomainError::CustomerInvoiceCancelled;
}

/// The customer as it was when the invoice was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerSnapshot {
    pub number: u32,
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
}

impl CustomerSnapshot {
    pub fn of(customer: &Party<CustomerDetails>) -> Result<Self, DomainError> {
        if !customer.active {
            return Err(DomainError::CustomerInactive);
        }
        let d = &customer.details;
        Ok(Self {
            number: customer.number,
            name: d.name.clone(),
            org_nr: d.org_nr.clone(),
            vat_number: d.vat_number.clone(),
            address: d.address.clone(),
            email: d.email.clone(),
        })
    }
}

/// A customer invoice as typed in, before it has a number. Its VAT is
/// always computed.
pub struct NewCustomerInvoice<'a> {
    pub invoice_number: &'a str,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: &'a str,
    pub lines: Vec<InvoiceLine>,
}

/// What a registered customer invoice says. Never changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerRegistration {
    pub customer: CustomerSnapshot,
    pub invoice_number: InvoiceNumber,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: Option<PaymentReference>,
    pub lines: Vec<InvoiceLine>,
    pub vat: Vec<VatAmount>,
    pub total: i64,
}

impl CustomerRegistration {
    pub fn new(customer: CustomerSnapshot, new: &NewCustomerInvoice) -> Result<Self, DomainError> {
        let invoice_number = InvoiceNumber::parse(new.invoice_number)?;
        if new.due_date < new.invoice_date {
            return Err(DomainError::InvalidDueDate);
        }
        let reference = optional(new.reference, PaymentReference::parse)?;
        vat::check_lines(&new.lines)?;
        let vat = vat::by_rate(&new.lines);
        let total = vat::net(&new.lines) + vat.iter().map(|v| v.amount).sum::<i64>();
        Ok(Self {
            customer,
            invoice_number,
            invoice_date: new.invoice_date,
            due_date: new.due_date,
            reference,
            lines: new.lines.clone(),
            vat,
            total,
        })
    }

    pub fn vat_total(&self) -> i64 {
        self.vat.iter().map(|v| v.amount).sum()
    }
}

// The registration carries the whole invoice; events are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustomerInvoiceEvent {
    CustomerInvoiceRegistered {
        number: u32,
        invoice: CustomerRegistration,
        attachments: Vec<Attachment>,
        /// `None` under kontantmetoden: nothing is booked until payment.
        voucher: Option<VoucherRef>,
    },
    CustomerInvoicePaid {
        number: u32,
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    CustomerInvoiceCancelled {
        number: u32,
        reason: String,
        voucher: Option<VoucherRef>,
    },
    CustomerInvoicePaymentReversed {
        number: u32,
        reason: String,
        voucher: VoucherRef,
    },
}

impl CustomerInvoiceEvent {
    pub fn number(&self) -> u32 {
        match self {
            Self::CustomerInvoiceRegistered { number, .. }
            | Self::CustomerInvoicePaid { number, .. }
            | Self::CustomerInvoiceCancelled { number, .. }
            | Self::CustomerInvoicePaymentReversed { number, .. } => *number,
        }
    }
}

/// A customer invoice and what has happened to it. Also the projection's
/// `details` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerInvoice {
    pub number: u32,
    pub invoice: CustomerRegistration,
    pub attachments: Vec<Attachment>,
    pub status: Status,
    pub vouchers: Vec<VoucherRef>,
    pub registration_voucher: Option<VoucherRef>,
}

impl CustomerInvoice {
    pub fn registered(
        number: u32,
        invoice: CustomerRegistration,
        attachments: Vec<Attachment>,
        voucher: Option<VoucherRef>,
    ) -> Self {
        Self {
            number,
            invoice,
            attachments,
            status: Status::Unpaid,
            vouchers: voucher.into_iter().collect(),
            registration_voucher: voucher,
        }
    }

    /// Applies a later event: paid, cancelled or payment reversed.
    pub fn apply(&mut self, event: &CustomerInvoiceEvent) {
        match event {
            CustomerInvoiceEvent::CustomerInvoiceRegistered { .. } => {}
            CustomerInvoiceEvent::CustomerInvoicePaid {
                date,
                account,
                voucher,
                ..
            } => {
                self.status = Status::Paid {
                    date: *date,
                    account: *account,
                    voucher: *voucher,
                };
                self.vouchers.push(*voucher);
            }
            CustomerInvoiceEvent::CustomerInvoiceCancelled { voucher, .. } => {
                self.status = Status::Cancelled;
                self.vouchers.extend(*voucher);
            }
            CustomerInvoiceEvent::CustomerInvoicePaymentReversed { voucher, .. } => {
                self.status = Status::Unpaid;
                self.vouchers.push(*voucher);
            }
        }
    }

    pub fn status_code(&self) -> &'static str {
        match self.status {
            Status::Unpaid => "unpaid",
            Status::Paid { .. } => "paid",
            Status::Cancelled => "cancelled",
        }
    }
}

/// One company's customer invoices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomerInvoices {
    invoices: BTreeMap<u32, CustomerInvoice>,
}

impl CustomerInvoices {
    pub fn from_events(events: impl IntoIterator<Item = CustomerInvoiceEvent>) -> Self {
        let mut state = Self::default();
        events.into_iter().for_each(|e| state.apply(e));
        state
    }

    pub fn apply(&mut self, event: CustomerInvoiceEvent) {
        match event {
            CustomerInvoiceEvent::CustomerInvoiceRegistered {
                number,
                invoice,
                attachments,
                voucher,
            } => {
                self.invoices.insert(
                    number,
                    CustomerInvoice::registered(number, invoice, attachments, voucher),
                );
            }
            other => {
                if let Some(invoice) = self.invoices.get_mut(&other.number()) {
                    invoice.apply(&other);
                }
            }
        }
    }

    pub fn get(&self, number: u32) -> Option<&CustomerInvoice> {
        self.invoices.get(&number)
    }

    pub fn next_number(&self) -> u32 {
        self.invoices.keys().next_back().map_or(1, |n| n + 1)
    }
}

/// The new invoice's internal number. An invoice number already used,
/// cancelled or not, is refused: an issued number is never reused.
pub fn register(
    state: &CustomerInvoices,
    invoice: &CustomerRegistration,
) -> Result<u32, DomainError> {
    if state
        .invoices
        .values()
        .any(|e| e.invoice.invoice_number == invoice.invoice_number)
    {
        return Err(DomainError::DuplicateCustomerInvoice);
    }
    Ok(state.next_number())
}

pub fn unpaid(state: &CustomerInvoices, number: u32) -> Result<&CustomerInvoice, DomainError> {
    let invoice = state
        .get(number)
        .ok_or(DomainError::CustomerInvoiceNotFound)?;
    invoices::check_unpaid::<CustomerInvoiceKind>(&invoice.status)?;
    Ok(invoice)
}

pub fn paid(
    state: &CustomerInvoices,
    number: u32,
) -> Result<(&CustomerInvoice, VoucherRef), DomainError> {
    let invoice = state
        .get(number)
        .ok_or(DomainError::CustomerInvoiceNotFound)?;
    let voucher = invoices::check_paid::<CustomerInvoiceKind>(&invoice.status)?;
    Ok((invoice, voucher))
}

/// The highest invoice number made only of digits, plus one; "1" if there
/// is none. A number too long for u64 counts as not a plain number.
pub fn next_invoice_number<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    existing
        .into_iter()
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|n| n.parse::<u64>().ok())
        .max()
        .map_or_else(|| "1".to_owned(), |n| n.saturating_add(1).to_string())
}

/// "Kundfaktura 1017, Kund AB", cut to a voucher text's 200 characters.
pub fn text(invoice: &CustomerRegistration) -> String {
    invoices::voucher_text(format!(
        "Kundfaktura {}, {}",
        invoice.invoice_number.as_str(),
        invoice.customer.name.as_str()
    ))
}

/// Each line's net and the output VAT per rate, credited.
fn income_side(invoice: &CustomerRegistration) -> Vec<VoucherLine> {
    let vat: Vec<(AccountNumber, i64)> = invoice
        .vat
        .iter()
        .filter_map(|v| output_vat_account(v.vat_rate).map(|a| (a, v.amount)))
        .collect();
    invoices::posting(&invoice.lines, &vat, Side::Credit)
}

/// Faktureringsmetoden, at registration: 1510 against income and VAT.
pub fn registration_lines(invoice: &CustomerRegistration) -> Vec<VoucherLine> {
    let mut lines = vec![invoices::entry(
        account(ACCOUNTS_RECEIVABLE),
        invoice.total,
        Side::Debit,
    )];
    lines.extend(income_side(invoice));
    lines
}

/// At payment: the bank against 1510 under faktureringsmetoden, against
/// income and VAT under kontantmetoden.
pub fn payment_lines(
    invoice: &CustomerRegistration,
    method: AccountingMethod,
    paid_into: AccountNumber,
) -> Vec<VoucherLine> {
    let mut lines = vec![invoices::entry(paid_into, invoice.total, Side::Debit)];
    match method {
        AccountingMethod::Invoice => lines.push(invoices::entry(
            account(ACCOUNTS_RECEIVABLE),
            invoice.total,
            Side::Credit,
        )),
        AccountingMethod::Cash => lines.extend(income_side(invoice)),
    }
    lines
}
