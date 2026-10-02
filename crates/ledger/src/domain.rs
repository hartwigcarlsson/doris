//! Pure ledger rules: the chart of accounts and vouchers. No I/O, no clock.

use doris_company::domain::{FiscalYear, LegalForm};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    #[error("opening balances are for accounts 1000-2999")]
    NotBalanceSheetAccount,
    #[error("an account appears more than once")]
    DuplicateAccount,
    #[error("opening balances must balance")]
    OpeningBalancesUnbalanced,
    #[error("reason must be 1-200 characters")]
    InvalidReason,
    #[error("no such fiscal year")]
    FiscalYearNotFound,
    #[error("fiscal year is closed")]
    FiscalYearClosed,
    #[error("fiscal year is open")]
    FiscalYearOpen,
    #[error("fiscal year has not ended")]
    FiscalYearNotEnded,
    #[error("the previous fiscal year is open")]
    PreviousFiscalYearOpen,
    #[error("a later fiscal year is closed")]
    LaterFiscalYearClosed,
    /// A sum outgrew `i64`; no real ledger gets there.
    #[error("amount overflow")]
    Overflow,
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
    /// The first fiscal year's ingående balanser, replacing any before.
    /// Not a voucher: it takes no number. Later years' are derived.
    OpeningBalancesSet { lines: Vec<VoucherLine> },
    /// The year is locked. `result_voucher` is the "Årets resultat" voucher
    /// booked just before, if the result wasn't already on equity.
    FiscalYearClosed { result_voucher: Option<u32> },
    /// The year is open again; the result voucher's reversal follows.
    FiscalYearReopened { reason: String },
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

/// One account's figures in a fiscal year (saldobalans). `opening` is its
/// ingående balans; the utgående balans is `opening + debit - credit`.
/// Balances are debit − credit; positive is a debit balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBalanceRow {
    pub account: u32,
    pub name: String,
    pub opening: i64,
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

/// One account's huvudbok for a fiscal year: its ingående balans and its
/// lines, each with the balance after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLedger {
    pub opening: i64,
    pub entries: Vec<LedgerEntry>,
}

/// A fiscal year and whether it is closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FiscalYearStatus {
    pub fiscal_year: FiscalYear,
    pub closed: bool,
}

/// Adds the running balance, starting from `opening`, to
/// `(date, number, text, debit, credit)` lines in the order given. `None` if
/// it outgrows `i64`, which no real ledger reaches; a wrong figure would be
/// worse than an error.
pub fn running_balance(
    opening: i64,
    lines: Vec<(Date, u32, String, i64, i64)>,
) -> Option<Vec<LedgerEntry>> {
    let mut balance = opening;
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

/// One fiscal year's vouchers, in number order, and whether it is closed.
#[derive(Debug, Clone, PartialEq)]
pub struct Ledger {
    pub fiscal_year: FiscalYear,
    vouchers: Vec<Voucher>,
    opening_balances: Vec<VoucherLine>,
    closed: bool,
    result_voucher: Option<u32>,
}

impl Ledger {
    pub fn new(fiscal_year: FiscalYear) -> Self {
        Self {
            fiscal_year,
            vouchers: Vec::new(),
            opening_balances: Vec::new(),
            closed: false,
            result_voucher: None,
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
        match event.clone() {
            LedgerEvent::VoucherRecorded {
                number,
                date,
                text,
                lines,
                corrects,
            } => {
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
            LedgerEvent::OpeningBalancesSet { lines } => self.opening_balances = lines,
            LedgerEvent::FiscalYearClosed { result_voucher } => {
                self.closed = true;
                self.result_voucher = result_voucher;
            }
            LedgerEvent::FiscalYearReopened { .. } => {
                self.closed = false;
                self.result_voucher = None;
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Only ever set in the first fiscal year.
    pub fn opening_balances(&self) -> &[VoucherLine] {
        &self.opening_balances
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
    if ledger.closed {
        return Err(DomainError::FiscalYearClosed);
    }
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
    if ledger.closed {
        return Err(DomainError::FiscalYearClosed);
    }
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
    Ok(reversal(ledger, original, date))
}

/// The most lines the first year's opening balances may have.
pub const MAX_OPENING_BALANCE_LINES: usize = 500;

/// Decides the first fiscal year's ingående balanser, replacing any before.
/// Accounts must exist but may be inactive: the balance is history.
pub fn set_opening_balances(
    ledger: &Ledger,
    chart: &Chart,
    lines: Vec<VoucherLine>,
) -> Result<LedgerEvent, DomainError> {
    if ledger.closed {
        return Err(DomainError::FiscalYearClosed);
    }
    if lines.len() > MAX_OPENING_BALANCE_LINES {
        return Err(DomainError::InvalidVoucherLines);
    }
    let mut seen = BTreeSet::new();
    let (mut debit, mut credit) = (0_i64, 0_i64);
    for line in &lines {
        if !line.is_valid() {
            return Err(DomainError::InvalidAmount);
        }
        if line.account.get() >= 3000 {
            return Err(DomainError::NotBalanceSheetAccount);
        }
        chart
            .get(line.account)
            .ok_or(DomainError::AccountNotFound)?;
        if !seen.insert(line.account) {
            return Err(DomainError::DuplicateAccount);
        }
        // Cannot overflow: at most 500 lines of at most MAX_AMOUNT.
        debit += line.debit;
        credit += line.credit;
    }
    if debit != credit {
        return Err(DomainError::OpeningBalancesUnbalanced);
    }
    Ok(LedgerEvent::OpeningBalancesSet { lines })
}

/// The rättelse of `original`: the next voucher in `ledger`, with debit and
/// credit swapped on every line.
fn reversal(ledger: &Ledger, original: &Voucher, date: Date) -> LedgerEvent {
    LedgerEvent::VoucherRecorded {
        number: ledger.last_number() + 1,
        date,
        text: format!("Rättelse av ver {}", original.number),
        lines: original
            .lines
            .iter()
            .map(|l| VoucherLine {
                account: l.account,
                debit: l.credit,
                credit: l.debit,
            })
            .collect(),
        corrects: Some(original.number),
    }
}

/// Debit minus credit over the resultaträkning (accounts 3000–8999), so a
/// profit is negative. `None` if it outgrows `i64`; a wrong figure would be
/// worse than an error.
pub fn result_of(ledger: &Ledger) -> Option<i64> {
    ledger
        .vouchers
        .iter()
        .flat_map(|v| &v.lines)
        .filter(|l| l.account.get() >= 3000)
        .try_fold(0_i64, |sum, l| {
            sum.checked_add(l.debit)?.checked_sub(l.credit)
        })
}

/// Where the year's result goes: 2019 for an enskild firma and partnerships,
/// whose owners are taxed on it personally, and 2099 for everyone else.
fn result_account(legal_form: LegalForm) -> AccountNumber {
    match legal_form {
        LegalForm::EnskildFirma | LegalForm::Handelsbolag | LegalForm::Kommanditbolag => {
            AccountNumber(2019)
        }
        _ => AccountNumber(2099),
    }
}

/// Closes a fiscal year that has ended, once the year before it (if any,
/// `previous_closed`) is closed. Unless the result is already on equity, a
/// voucher "Årets resultat" moves it there first: 8999 against 2099 or 2019,
/// dated the year's last day. Accounts aren't checked for being active, as
/// with a rättelse.
pub fn close_fiscal_year(
    ledger: &Ledger,
    previous_closed: Option<bool>,
    legal_form: LegalForm,
    today: Date,
) -> Result<Vec<LedgerEvent>, DomainError> {
    if ledger.closed {
        return Err(DomainError::FiscalYearClosed);
    }
    if ledger.fiscal_year.end >= today {
        return Err(DomainError::FiscalYearNotEnded);
    }
    if previous_closed == Some(false) {
        return Err(DomainError::PreviousFiscalYearOpen);
    }
    let result = result_of(ledger).ok_or(DomainError::Overflow)?;
    let amount = result.checked_abs().ok_or(DomainError::Overflow)?;
    let mut events = Vec::new();
    let mut result_voucher = None;
    if amount != 0 {
        let number = ledger.last_number() + 1;
        // A profit (a credit balance, result < 0) is debited to 8999.
        let (debit, credit) = if result < 0 { (amount, 0) } else { (0, amount) };
        events.push(LedgerEvent::VoucherRecorded {
            number,
            date: ledger.fiscal_year.end,
            text: "Årets resultat".into(),
            lines: vec![
                VoucherLine {
                    account: AccountNumber(8999),
                    debit,
                    credit,
                },
                VoucherLine {
                    account: result_account(legal_form),
                    debit: credit,
                    credit: debit,
                },
            ],
            corrects: None,
        });
        result_voucher = Some(number);
    }
    events.push(LedgerEvent::FiscalYearClosed { result_voucher });
    Ok(events)
}

/// Reopens a closed fiscal year whose next year (`next_closed`) is open.
/// The reopening comes first, so no voucher is ever recorded while the year
/// is closed; then the result voucher, if there was one, is reversed on the
/// year's last day.
pub fn reopen_fiscal_year(
    ledger: &Ledger,
    next_closed: bool,
    reason: &str,
) -> Result<Vec<LedgerEvent>, DomainError> {
    if !ledger.closed {
        return Err(DomainError::FiscalYearOpen);
    }
    if next_closed {
        return Err(DomainError::LaterFiscalYearClosed);
    }
    let reason = reason.trim();
    if !(1..=200).contains(&reason.chars().count()) {
        return Err(DomainError::InvalidReason);
    }
    let mut events = vec![LedgerEvent::FiscalYearReopened {
        reason: reason.to_owned(),
    }];
    if let Some(original) = ledger.result_voucher.and_then(|n| ledger.voucher(n)) {
        events.push(reversal(ledger, original, ledger.fiscal_year.end));
    }
    Ok(events)
}
