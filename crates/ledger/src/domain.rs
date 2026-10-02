//! Pure ledger rules: the chart of accounts and vouchers. No I/O, no clock.

use doris_company::domain::FiscalYear;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("account number must be 1000-8999")]
    InvalidAccountNumber,
    #[error("account name must be 1-100 characters")]
    InvalidAccountName,
    #[error("account already exists")]
    AccountExists,
    #[error("no such account")]
    AccountNotFound,
    #[error("account is inactive")]
    AccountInactive,
    #[error("voucher text must be 1-200 characters")]
    InvalidVoucherText,
    #[error("a voucher has 2-100 lines")]
    InvalidVoucherLines,
    #[error("each line has exactly one of debit and credit, at most 10^13 öre")]
    InvalidAmount,
    #[error("debit and credit differ")]
    VoucherUnbalanced,
    #[error("voucher date is in the future")]
    VoucherDateInFuture,
    #[error("voucher date is before the first fiscal year")]
    VoucherDateBeforeFirstFiscalYear,
    #[error("correction date is outside the voucher's fiscal year")]
    CorrectionDateOutsideFiscalYear,
    #[error("no such voucher")]
    VoucherNotFound,
    #[error("voucher is already corrected")]
    AlreadyCorrected,
    #[error("a correction cannot be corrected")]
    CannotCorrectCorrection,
}

/// A BAS account number: four digits, 1000-8999.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountNumber(u16);

impl AccountNumber {
    pub fn parse(raw: u32) -> Result<Self, DomainError> {
        match raw {
            1000..=8999 => Ok(Self(raw as u16)),
            _ => Err(DomainError::InvalidAccountNumber),
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountName(String);

impl AccountName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if (1..=100).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidAccountName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartAccount {
    pub number: AccountNumber,
    pub name: AccountName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChartEvent {
    /// The whole starting chart, spelled out so the history reads the same
    /// even after the built-in selection changes.
    ChartSeeded {
        accounts: Vec<ChartAccount>,
    },
    AccountAdded {
        number: AccountNumber,
        name: AccountName,
    },
    AccountRenamed {
        number: AccountNumber,
        name: AccountName,
    },
    AccountDeactivated {
        number: AccountNumber,
    },
    AccountReactivated {
        number: AccountNumber,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub number: AccountNumber,
    pub name: AccountName,
    pub active: bool,
}

/// A company's chart of accounts. Accounts are never removed, only
/// deactivated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    accounts: BTreeMap<AccountNumber, Account>,
}

impl Chart {
    pub fn from_events(events: &[ChartEvent]) -> Self {
        let mut chart = Self::default();
        for event in events {
            chart.apply(event);
        }
        chart
    }

    pub fn apply(&mut self, event: &ChartEvent) {
        match event.clone() {
            ChartEvent::ChartSeeded { accounts } => {
                for ChartAccount { number, name } in accounts {
                    self.accounts.insert(
                        number,
                        Account {
                            number,
                            name,
                            active: true,
                        },
                    );
                }
            }
            ChartEvent::AccountAdded { number, name } => {
                self.accounts.insert(
                    number,
                    Account {
                        number,
                        name,
                        active: true,
                    },
                );
            }
            ChartEvent::AccountRenamed { number, name } => {
                if let Some(account) = self.accounts.get_mut(&number) {
                    account.name = name;
                }
            }
            ChartEvent::AccountDeactivated { number } => self.set_active(number, false),
            ChartEvent::AccountReactivated { number } => self.set_active(number, true),
        }
    }

    fn set_active(&mut self, number: AccountNumber, active: bool) {
        if let Some(account) = self.accounts.get_mut(&number) {
            account.active = active;
        }
    }

    pub fn get(&self, number: AccountNumber) -> Option<&Account> {
        self.accounts.get(&number)
    }

    /// By number.
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }
}

/// The built-in BAS selection every company starts with.
pub fn seed_chart() -> ChartEvent {
    ChartEvent::ChartSeeded {
        accounts: crate::bas::ACCOUNTS
            .iter()
            .map(|&(number, name)| ChartAccount {
                number: AccountNumber::parse(number.into()).expect("BAS numbers are valid"),
                name: AccountName::parse(name).expect("BAS names are valid"),
            })
            .collect(),
    }
}

pub fn add_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    if chart.get(number).is_some() {
        return Err(DomainError::AccountExists);
    }
    Ok(vec![ChartEvent::AccountAdded { number, name }])
}

/// Renaming to the current name yields no events.
pub fn rename_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    if account.name == name {
        return Ok(vec![]);
    }
    Ok(vec![ChartEvent::AccountRenamed { number, name }])
}

/// Idempotent: an account already in the wanted state yields no events.
pub fn set_account_active(
    chart: &Chart,
    number: AccountNumber,
    active: bool,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    Ok(match (account.active, active) {
        (true, false) => vec![ChartEvent::AccountDeactivated { number }],
        (false, true) => vec![ChartEvent::AccountReactivated { number }],
        _ => vec![],
    })
}

/// The most one line may carry: 100 miljarder kronor. With at most 100
/// lines, sums stay far below `i64::MAX`, so they cannot overflow.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

/// One kontering. Exactly one of `debit` and `credit` is positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoucherLine {
    pub account: AccountNumber,
    pub debit: i64,
    pub credit: i64,
}

impl VoucherLine {
    /// An out-of-range account number cannot be in any chart, so it is
    /// reported as an unknown account.
    pub fn new(account: u32, debit: i64, credit: i64) -> Result<Self, DomainError> {
        let account = AccountNumber::parse(account).map_err(|_| DomainError::AccountNotFound)?;
        Ok(Self {
            account,
            debit,
            credit,
        })
    }

    fn is_valid(&self) -> bool {
        let in_range = |amount: i64| (0..=MAX_AMOUNT).contains(&amount);
        in_range(self.debit) && in_range(self.credit) && (self.debit == 0) != (self.credit == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LedgerEvent {
    /// Who recorded it, and when, is in the event metadata (BFL 5 kap. 11 §).
    VoucherRecorded {
        number: u32,
        date: Date,
        text: String,
        lines: Vec<VoucherLine>,
        corrects: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Voucher {
    pub number: u32,
    pub date: Date,
    pub text: String,
    pub lines: Vec<VoucherLine>,
    pub corrects: Option<u32>,
    pub corrected_by: Option<u32>,
}

/// One account's totals in a fiscal year (saldobalans). Its balance is
/// `debit - credit`; positive is a debit balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBalanceRow {
    pub account: u32,
    pub name: String,
    pub debit: i64,
    pub credit: i64,
}

/// One line in an account's huvudbok, with the balance after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub date: Date,
    pub number: u32,
    pub text: String,
    pub debit: i64,
    pub credit: i64,
    pub balance: i64,
}

/// Adds the running balance to `(date, number, text, debit, credit)` lines,
/// in the order given. `None` if it outgrows `i64`, which no real ledger
/// reaches; a wrong figure would be worse than an error.
pub fn running_balance(lines: Vec<(Date, u32, String, i64, i64)>) -> Option<Vec<LedgerEntry>> {
    let mut balance = 0i64;
    lines
        .into_iter()
        .map(|(date, number, text, debit, credit)| {
            balance = balance.checked_add(debit)?.checked_sub(credit)?;
            Some(LedgerEntry {
                date,
                number,
                text,
                debit,
                credit,
                balance,
            })
        })
        .collect()
}

/// One fiscal year's vouchers, in number order.
#[derive(Debug, Clone, PartialEq)]
pub struct Ledger {
    pub fiscal_year: FiscalYear,
    vouchers: Vec<Voucher>,
}

impl Ledger {
    pub fn new(fiscal_year: FiscalYear) -> Self {
        Self {
            fiscal_year,
            vouchers: Vec::new(),
        }
    }

    pub fn from_events(fiscal_year: FiscalYear, events: &[LedgerEvent]) -> Self {
        let mut ledger = Self::new(fiscal_year);
        for event in events {
            ledger.apply(event);
        }
        ledger
    }

    pub fn apply(&mut self, event: &LedgerEvent) {
        let LedgerEvent::VoucherRecorded {
            number,
            date,
            text,
            lines,
            corrects,
        } = event.clone();
        if let Some(original) =
            corrects.and_then(|n| self.vouchers.iter_mut().find(|v| v.number == n))
        {
            original.corrected_by = Some(number);
        }
        self.vouchers.push(Voucher {
            number,
            date,
            text,
            lines,
            corrects,
            corrected_by: None,
        });
    }

    /// 0 before the first voucher.
    pub fn last_number(&self) -> u32 {
        self.vouchers.last().map_or(0, |v| v.number)
    }

    pub fn voucher(&self, number: u32) -> Option<&Voucher> {
        self.vouchers.iter().find(|v| v.number == number)
    }

    pub fn vouchers(&self) -> &[Voucher] {
        &self.vouchers
    }
}

/// The räkenskapsår a voucher dated `date` belongs to. A voucher records
/// something that happened, so it cannot be dated after `today`.
pub fn fiscal_year_for(
    first: FiscalYear,
    date: Date,
    today: Date,
) -> Result<FiscalYear, DomainError> {
    if date > today {
        return Err(DomainError::VoucherDateInFuture);
    }
    if date < first.start {
        return Err(DomainError::VoucherDateBeforeFirstFiscalYear);
    }
    Ok(first.containing(date))
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordVoucher {
    pub date: Date,
    pub text: String,
    pub lines: Vec<VoucherLine>,
}

/// Decides a new voucher in `ledger`, the fiscal year `cmd.date` falls in
/// (see [`fiscal_year_for`]). Its number is the next one in that year.
pub fn record_voucher(
    ledger: &Ledger,
    chart: &Chart,
    cmd: RecordVoucher,
) -> Result<LedgerEvent, DomainError> {
    let text = cmd.text.trim();
    if !(1..=200).contains(&text.chars().count()) {
        return Err(DomainError::InvalidVoucherText);
    }
    if !(2..=100).contains(&cmd.lines.len()) {
        return Err(DomainError::InvalidVoucherLines);
    }
    let (mut debit, mut credit) = (0_i64, 0_i64);
    for line in &cmd.lines {
        if !line.is_valid() {
            return Err(DomainError::InvalidAmount);
        }
        let account = chart
            .get(line.account)
            .ok_or(DomainError::AccountNotFound)?;
        if !account.active {
            return Err(DomainError::AccountInactive);
        }
        // Cannot overflow: at most 100 lines of at most MAX_AMOUNT.
        debit += line.debit;
        credit += line.credit;
    }
    if debit != credit {
        return Err(DomainError::VoucherUnbalanced);
    }
    Ok(LedgerEvent::VoucherRecorded {
        number: ledger.last_number() + 1,
        date: cmd.date,
        text: text.to_owned(),
        lines: cmd.lines,
        corrects: None,
    })
}

/// A rättelse (BFL 5 kap. 5 §): a new voucher in the same fiscal year with
/// debit and credit swapped on every line. It ignores whether the accounts
/// are still active, so a voucher can always be reversed.
pub fn correct_voucher(
    ledger: &Ledger,
    number: u32,
    date: Date,
    today: Date,
) -> Result<LedgerEvent, DomainError> {
    let original = ledger.voucher(number).ok_or(DomainError::VoucherNotFound)?;
    if original.corrects.is_some() {
        return Err(DomainError::CannotCorrectCorrection);
    }
    if original.corrected_by.is_some() {
        return Err(DomainError::AlreadyCorrected);
    }
    if date > today {
        return Err(DomainError::VoucherDateInFuture);
    }
    if date < ledger.fiscal_year.start || date > ledger.fiscal_year.end {
        return Err(DomainError::CorrectionDateOutsideFiscalYear);
    }
    Ok(LedgerEvent::VoucherRecorded {
        number: ledger.last_number() + 1,
        date,
        text: format!("Rättelse av ver {number}"),
        lines: original
            .lines
            .iter()
            .map(|l| VoucherLine {
                account: l.account,
                debit: l.credit,
                credit: l.debit,
            })
            .collect(),
        corrects: Some(number),
    })
}
