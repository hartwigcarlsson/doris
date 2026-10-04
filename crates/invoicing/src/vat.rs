//! Invoice lines and their VAT (moms). Shared by supplier invoices and,
//! later, customer invoices. Pure.

use crate::domain::DomainError;
use doris_ledger::domain::AccountNumber;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most one line may be: 10^13 öre, as for a voucher line. With at
/// most 50 lines no sum gets near `i64::MAX`.
pub const MAX_LINE_NET: i64 = 10_000_000_000_000;
const MAX_LINES: usize = 50;

/// Swedish VAT rates. Stored as the percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum VatRate {
    Rate25,
    Rate12,
    Rate6,
    Rate0,
}

impl VatRate {
    pub fn parse(percent: u32) -> Result<Self, DomainError> {
        match percent {
            25 => Ok(Self::Rate25),
            12 => Ok(Self::Rate12),
            6 => Ok(Self::Rate6),
            0 => Ok(Self::Rate0),
            _ => Err(DomainError::InvalidVatRate),
        }
    }

    pub fn percent(self) -> u32 {
        match self {
            Self::Rate25 => 25,
            Self::Rate12 => 12,
            Self::Rate6 => 6,
            Self::Rate0 => 0,
        }
    }
}

impl TryFrom<u32> for VatRate {
    type Error = DomainError;
    fn try_from(percent: u32) -> Result<Self, DomainError> {
        Self::parse(percent)
    }
}

impl From<VatRate> for u32 {
    fn from(rate: VatRate) -> u32 {
        rate.percent()
    }
}

/// One cost on an invoice: the account, the amount without VAT, the rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub account: AccountNumber,
    pub net: i64,
    pub vat_rate: VatRate,
}

impl InvoiceLine {
    /// 2440 and the VAT accounts 2600–2699 are booked by Doris itself, and
    /// a number outside 1000–8999 is no account at all.
    pub fn new(account: u32, net: i64, vat_rate: u32) -> Result<Self, DomainError> {
        let account =
            AccountNumber::parse(account).map_err(|_| DomainError::InvalidInvoiceAccount)?;
        if account.get() == 2440 || (2600..=2699).contains(&account.get()) {
            return Err(DomainError::InvalidInvoiceAccount);
        }
        if !(1..=MAX_LINE_NET).contains(&net) {
            return Err(DomainError::InvalidInvoiceLines);
        }
        Ok(Self {
            account,
            net,
            vat_rate: VatRate::parse(vat_rate)?,
        })
    }
}

pub fn check_lines(lines: &[InvoiceLine]) -> Result<(), DomainError> {
    if (1..=MAX_LINES).contains(&lines.len()) {
        Ok(())
    } else {
        Err(DomainError::InvalidInvoiceLines)
    }
}

/// The sum without VAT.
pub fn net(lines: &[InvoiceLine]) -> i64 {
    lines.iter().map(|l| l.net).sum()
}

/// VAT per rate on that rate's summed net, rounded half up to whole öre.
pub fn computed(lines: &[InvoiceLine]) -> i64 {
    let mut by_rate = BTreeMap::<VatRate, i64>::new();
    for line in lines {
        *by_rate.entry(line.vat_rate).or_default() += line.net;
    }
    by_rate
        .into_iter()
        .map(|(rate, net)| (net * i64::from(rate.percent()) + 50) / 100)
        .sum()
}

/// The VAT to book: the computed amount, or the given one if it is ≥ 0 and
/// within 1 krona of it (an invoice may round differently).
pub fn check(lines: &[InvoiceLine], given: Option<i64>) -> Result<i64, DomainError> {
    let computed = computed(lines);
    match given {
        None => Ok(computed),
        Some(vat) if vat >= 0 && (vat - computed).abs() <= 100 => Ok(vat),
        Some(_) => Err(DomainError::InvalidVatAmount),
    }
}
