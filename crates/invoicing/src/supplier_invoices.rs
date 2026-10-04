//! Pure rules for supplier invoices (leverantörsfakturor): the values, the
//! events, the state and the vouchers Doris books for them. No I/O.

use crate::domain::{
    Bankgiro, Bic, DomainError, Iban, Party, PartyName, Plusgiro, SupplierDetails, optional,
};
use crate::vat::{self, InvoiceLine};
use doris_company::domain::{AccountingMethod, OrgNr};
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, Attachment, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACCOUNTS_PAYABLE: u32 = 2440;
const INPUT_VAT: u32 = 2640;
const MAX_VOUCHER_TEXT: usize = 200;

fn account(number: u32) -> AccountNumber {
    AccountNumber::parse(number).expect("a BAS account")
}

fn bounded(raw: &str, max: usize) -> Option<String> {
    let text = raw.trim();
    (1..=max)
        .contains(&text.chars().count())
        .then(|| text.to_owned())
}

/// The supplier's own invoice number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvoiceNumber(String);

impl InvoiceNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded(raw, 50)
            .map(Self)
            .ok_or(DomainError::InvalidInvoiceNumber)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// OCR number or message to send with the payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentReference(String);

impl PaymentReference {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded(raw, 50)
            .map(Self)
            .ok_or(DomainError::InvalidReference)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Payments go out from an account in 1900–1999 (kassa och bank).
pub fn payment_account(number: u32) -> Result<AccountNumber, DomainError> {
    match number {
        1900..=1999 => Ok(account(number)),
        _ => Err(DomainError::InvalidPaymentAccount),
    }
}

/// Why an invoice is cancelled or a payment reversed: 1–200 characters.
pub fn reason(raw: &str) -> Result<String, DomainError> {
    bounded(raw, 200).ok_or(DomainError::InvalidReason)
}

/// The supplier as it was when the invoice was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierSnapshot {
    pub number: u32,
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub bankgiro: Option<Bankgiro>,
    pub plusgiro: Option<Plusgiro>,
    pub iban: Option<Iban>,
    pub bic: Option<Bic>,
}

impl SupplierSnapshot {
    pub fn of(supplier: &Party<SupplierDetails>) -> Result<Self, DomainError> {
        if !supplier.active {
            return Err(DomainError::SupplierInactive);
        }
        let d = &supplier.details;
        Ok(Self {
            number: supplier.number,
            name: d.name.clone(),
            org_nr: d.org_nr.clone(),
            bankgiro: d.bankgiro.clone(),
            plusgiro: d.plusgiro.clone(),
            iban: d.iban.clone(),
            bic: d.bic.clone(),
        })
    }
}

/// A supplier invoice as typed in, before it has a number.
pub struct NewSupplierInvoice<'a> {
    pub invoice_number: &'a str,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: &'a str,
    pub lines: Vec<InvoiceLine>,
    /// `None`: the computed VAT.
    pub vat: Option<i64>,
}

/// What a registered invoice says. Never changes after registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub supplier: SupplierSnapshot,
    pub invoice_number: InvoiceNumber,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: Option<PaymentReference>,
    pub lines: Vec<InvoiceLine>,
    pub vat: i64,
    pub total: i64,
}

impl Registration {
    pub fn new(supplier: SupplierSnapshot, new: &NewSupplierInvoice) -> Result<Self, DomainError> {
        let invoice_number = InvoiceNumber::parse(new.invoice_number)?;
        if new.due_date < new.invoice_date {
            return Err(DomainError::InvalidDueDate);
        }
        let reference = optional(new.reference, PaymentReference::parse)?;
        vat::check_lines(&new.lines)?;
        let vat = vat::check(&new.lines, new.vat)?;
        Ok(Self {
            supplier,
            invoice_number,
            invoice_date: new.invoice_date,
            due_date: new.due_date,
            reference,
            total: vat::net(&new.lines) + vat,
            lines: new.lines.clone(),
            vat,
        })
    }
}

// The registration carries the whole invoice; events are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupplierInvoiceEvent {
    SupplierInvoiceRegistered {
        number: u32,
        invoice: Registration,
        attachments: Vec<Attachment>,
        /// `None` under kontantmetoden: nothing is booked until payment.
        voucher: Option<VoucherRef>,
    },
    SupplierInvoicePaid {
        number: u32,
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    SupplierInvoiceCancelled {
        number: u32,
        reason: String,
        /// The correction of the registration voucher, if there was one.
        voucher: Option<VoucherRef>,
    },
    SupplierInvoicePaymentReversed {
        number: u32,
        reason: String,
        /// The correction of the payment voucher.
        voucher: VoucherRef,
    },
}

impl SupplierInvoiceEvent {
    pub fn number(&self) -> u32 {
        match self {
            Self::SupplierInvoiceRegistered { number, .. }
            | Self::SupplierInvoicePaid { number, .. }
            | Self::SupplierInvoiceCancelled { number, .. }
            | Self::SupplierInvoicePaymentReversed { number, .. } => *number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Unpaid,
    Paid {
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    Cancelled,
}

/// A supplier invoice and what has happened to it. Also the projection's
/// `details` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupplierInvoice {
    pub number: u32,
    pub invoice: Registration,
    pub attachments: Vec<Attachment>,
    pub status: Status,
    /// Every voucher booked for it, in order.
    pub vouchers: Vec<VoucherRef>,
    pub registration_voucher: Option<VoucherRef>,
}

impl SupplierInvoice {
    pub fn registered(
        number: u32,
        invoice: Registration,
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
    pub fn apply(&mut self, event: &SupplierInvoiceEvent) {
        match event {
            SupplierInvoiceEvent::SupplierInvoiceRegistered { .. } => {}
            SupplierInvoiceEvent::SupplierInvoicePaid {
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
            SupplierInvoiceEvent::SupplierInvoiceCancelled { voucher, .. } => {
                self.status = Status::Cancelled;
                self.vouchers.extend(*voucher);
            }
            SupplierInvoiceEvent::SupplierInvoicePaymentReversed { voucher, .. } => {
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

/// One company's supplier invoices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SupplierInvoices {
    invoices: BTreeMap<u32, SupplierInvoice>,
}

impl SupplierInvoices {
    pub fn from_events(events: impl IntoIterator<Item = SupplierInvoiceEvent>) -> Self {
        let mut state = Self::default();
        events.into_iter().for_each(|e| state.apply(e));
        state
    }

    pub fn apply(&mut self, event: SupplierInvoiceEvent) {
        match event {
            SupplierInvoiceEvent::SupplierInvoiceRegistered {
                number,
                invoice,
                attachments,
                voucher,
            } => {
                self.invoices.insert(
                    number,
                    SupplierInvoice::registered(number, invoice, attachments, voucher),
                );
            }
            other => {
                if let Some(invoice) = self.invoices.get_mut(&other.number()) {
                    invoice.apply(&other);
                }
            }
        }
    }

    pub fn get(&self, number: u32) -> Option<&SupplierInvoice> {
        self.invoices.get(&number)
    }

    pub fn next_number(&self) -> u32 {
        self.invoices.keys().next_back().map_or(1, |n| n + 1)
    }
}

/// The new invoice's number. The same supplier's invoice number may not be
/// registered twice unless the earlier one is cancelled.
pub fn register(state: &SupplierInvoices, invoice: &Registration) -> Result<u32, DomainError> {
    let duplicate = state.invoices.values().any(|existing| {
        existing.status != Status::Cancelled
            && existing.invoice.supplier.number == invoice.supplier.number
            && existing.invoice.invoice_number == invoice.invoice_number
    });
    if duplicate {
        return Err(DomainError::DuplicateSupplierInvoice);
    }
    Ok(state.next_number())
}

/// An unpaid invoice, the only kind that can be paid or cancelled.
pub fn unpaid(state: &SupplierInvoices, number: u32) -> Result<&SupplierInvoice, DomainError> {
    let invoice = state
        .get(number)
        .ok_or(DomainError::SupplierInvoiceNotFound)?;
    match invoice.status {
        Status::Unpaid => Ok(invoice),
        Status::Paid { .. } => Err(DomainError::SupplierInvoicePaid),
        Status::Cancelled => Err(DomainError::SupplierInvoiceCancelled),
    }
}

/// A paid invoice and its payment voucher, for reversing the payment.
pub fn paid(
    state: &SupplierInvoices,
    number: u32,
) -> Result<(&SupplierInvoice, VoucherRef), DomainError> {
    let invoice = state
        .get(number)
        .ok_or(DomainError::SupplierInvoiceNotFound)?;
    match invoice.status {
        Status::Paid { voucher, .. } => Ok((invoice, voucher)),
        Status::Unpaid => Err(DomainError::SupplierInvoiceNotPaid),
        Status::Cancelled => Err(DomainError::SupplierInvoiceCancelled),
    }
}

/// "Leverantörsfaktura 12, Lev AB (F-4711)", cut to a voucher text's 200
/// characters.
pub fn text(number: u32, invoice: &Registration) -> String {
    format!(
        "Leverantörsfaktura {number}, {} ({})",
        invoice.supplier.name.as_str(),
        invoice.invoice_number.as_str()
    )
    .chars()
    .take(MAX_VOUCHER_TEXT)
    .collect()
}

/// Each line's net and the VAT, debited: the cost side, the same under
/// both methods.
fn cost_side(invoice: &Registration) -> Vec<VoucherLine> {
    let mut lines: Vec<VoucherLine> = invoice
        .lines
        .iter()
        .map(|l| VoucherLine {
            account: l.account,
            debit: l.net,
            credit: 0,
        })
        .collect();
    if invoice.vat > 0 {
        lines.push(VoucherLine {
            account: account(INPUT_VAT),
            debit: invoice.vat,
            credit: 0,
        });
    }
    lines
}

/// Faktureringsmetoden, at registration: the cost against 2440.
pub fn registration_lines(invoice: &Registration) -> Vec<VoucherLine> {
    let mut lines = cost_side(invoice);
    lines.push(VoucherLine {
        account: account(ACCOUNTS_PAYABLE),
        debit: 0,
        credit: invoice.total,
    });
    lines
}

/// At payment: 2440 against the bank under faktureringsmetoden, the whole
/// cost against the bank under kontantmetoden.
pub fn payment_lines(
    invoice: &Registration,
    method: AccountingMethod,
    paid_from: AccountNumber,
) -> Vec<VoucherLine> {
    let mut lines = match method {
        AccountingMethod::Invoice => vec![VoucherLine {
            account: account(ACCOUNTS_PAYABLE),
            debit: invoice.total,
            credit: 0,
        }],
        AccountingMethod::Cash => cost_side(invoice),
    };
    lines.push(VoucherLine {
        account: paid_from,
        debit: 0,
        credit: invoice.total,
    });
    lines
}

/// A correction is dated today, or the last day of the corrected voucher's
/// fiscal year once today is past it (it must stay in that year).
pub fn correction_date(fiscal_year_end: Date, today: Date) -> Date {
    today.min(fiscal_year_end)
}
