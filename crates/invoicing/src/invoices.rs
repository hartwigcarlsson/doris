//! What supplier and customer invoices share: the status and its
//! transitions, how their vouchers are laid out, and how a correction is
//! dated. Pure.

use crate::domain::DomainError;
use crate::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

const MAX_VOUCHER_TEXT: usize = 200;

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

/// The errors one kind of invoice reports for a wrong transition.
pub trait InvoiceKind {
    const PAID: DomainError;
    const NOT_PAID: DomainError;
    const CANCELLED: DomainError;
}

/// Only an unpaid invoice can be paid or cancelled.
pub fn check_unpaid<K: InvoiceKind>(status: &Status) -> Result<(), DomainError> {
    match status {
        Status::Unpaid => Ok(()),
        Status::Paid { .. } => Err(K::PAID),
        Status::Cancelled => Err(K::CANCELLED),
    }
}

/// Only a paid invoice's payment can be reversed: its payment voucher.
pub fn check_paid<K: InvoiceKind>(status: &Status) -> Result<VoucherRef, DomainError> {
    match status {
        Status::Paid { voucher, .. } => Ok(*voucher),
        Status::Unpaid => Err(K::NOT_PAID),
        Status::Cancelled => Err(K::CANCELLED),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

pub fn entry(account: AccountNumber, amount: i64, side: Side) -> VoucherLine {
    match side {
        Side::Debit => VoucherLine {
            account,
            debit: amount,
            credit: 0,
        },
        Side::Credit => VoucherLine {
            account,
            debit: 0,
            credit: amount,
        },
    }
}

/// Each line's net, then each VAT entry, all on `side`.
pub fn posting(
    lines: &[InvoiceLine],
    vat: &[(AccountNumber, i64)],
    side: Side,
) -> Vec<VoucherLine> {
    lines
        .iter()
        .map(|l| entry(l.account, l.net, side))
        .chain(
            vat.iter()
                .map(|&(account, amount)| entry(account, amount, side)),
        )
        .collect()
}

/// Cut to the 200 characters a voucher text may have.
pub fn voucher_text(text: String) -> String {
    text.chars().take(MAX_VOUCHER_TEXT).collect()
}

/// A correction is dated today, or the last day of the corrected voucher's
/// fiscal year once today is past it (it must stay in that year).
pub fn correction_date(fiscal_year_end: Date, today: Date) -> Date {
    today.min(fiscal_year_end)
}
