//! Pure VAT rules: boxes, the settlement voucher, status and decisions.

use crate::period::{VatPeriod, VatPeriodKind};
use doris_company::domain::FiscalYear;
use doris_ledger::VatAccountTotal;
use doris_ledger::VoucherRef;
use doris_ledger::domain::VoucherLine;
use doris_ledger::vat_box::{Side, VatBox};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

/// An account's saldo (debit − credit, öre) over a period, with its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSaldo {
    pub account: u16,
    pub vat_box: VatBox,
    pub saldo: i64,
}

/// What is declared: whole kronor per box that is not zero, and box 49.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Boxes {
    pub amounts: Vec<(VatBox, i64)>,
    pub vat_due: i64,
}

impl Boxes {
    /// Kronor in box `n`, 0 when empty.
    pub fn get(&self, n: u8) -> i64 {
        self.amounts
            .iter()
            .find(|(b, _)| b.get() == n)
            .map_or(0, |(_, kr)| *kr)
    }
}

const OUTPUT_VAT: [u8; 9] = [10, 11, 12, 30, 31, 32, 60, 61, 62];

pub fn saldos(totals: &[VatAccountTotal]) -> Vec<AccountSaldo> {
    totals
        .iter()
        .map(|t| AccountSaldo {
            account: t.number,
            vat_box: t.vat_box,
            saldo: t.saldo,
        })
        .collect()
}

/// An account's saldo as its box counts it: positive for sales and
/// output VAT on the credit side, purchases and input VAT on the debit side.
pub fn signed(a: &AccountSaldo) -> i64 {
    match a.vat_box.side() {
        Side::Debit => a.saldo,
        Side::Credit => -a.saldo,
    }
}

/// Each box summed in öre, then the öre struck off toward zero, as
/// Skatteverket asks. Box 49 comes from the rounded boxes, as Skatteverket
/// checks it.
pub fn boxes(accounts: &[AccountSaldo]) -> Boxes {
    let mut ore = BTreeMap::<VatBox, i64>::new();
    for a in accounts {
        *ore.entry(a.vat_box).or_default() += signed(a);
    }
    let amounts: Vec<(VatBox, i64)> = ore
        .into_iter()
        .map(|(b, o)| (b, o / 100))
        .filter(|(_, kr)| *kr != 0)
        .collect();
    let mut boxes = Boxes {
        amounts,
        vat_due: 0,
    };
    boxes.vat_due = OUTPUT_VAT.iter().map(|&n| boxes.get(n)).sum::<i64>() - boxes.get(48);
    boxes
}

/// Box 49 in öre before rounding: the VAT the books hold.
pub fn booked_vat(accounts: &[AccountSaldo]) -> i64 {
    accounts
        .iter()
        .filter(|a| a.vat_box.is_vat())
        .map(|a| -a.saldo)
        .sum()
}

/// Hex SHA-256 of what a submission would record, so marking a period
/// submitted is refused when the books changed after it was shown.
pub fn fingerprint(period_end: Date, accounts: &[AccountSaldo]) -> String {
    let json = serde_json::to_vec(&(period_end, accounts)).expect("plain data serializes");
    Sha256::digest(json)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("no such redovisningsperiod")]
    InvalidVatPeriod,
    #[error("the period has not ended")]
    VatPeriodNotEnded,
    #[error("a period of the year is submitted")]
    VatPeriodLocked,
    #[error("the books changed since the declaration was shown")]
    VatReturnOutdated,
    #[error("the period is submitted and unchanged")]
    VatReturnUnchanged,
    #[error("the company is not VAT registered that year")]
    VatNotRegistered,
}

/// What was declared for a period, and what its settlement voucher booked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub period_end: Date,
    pub accounts: Vec<AccountSaldo>,
    pub boxes: Boxes,
    /// Per VAT account, the saldo the voucher moved to 2650 (öre).
    pub settled: Vec<(u16, i64)>,
    /// What the voucher booked on 2650 (öre); positive: to pay.
    pub settled_vat_due: i64,
    /// None when there was nothing to book.
    pub voucher: Option<VoucherRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VatEvent {
    /// How often the company declares VAT in the räkenskapsår.
    VatPeriodSet {
        fiscal_year_start: Date,
        kind: VatPeriodKind,
    },
    /// As the user confirms after uploading the file to Skatteverket.
    VatReturnSubmitted(Submission),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vat {
    pub kinds: BTreeMap<Date, VatPeriodKind>,
    /// In order; the last one per period is what Skatteverket has.
    pub submissions: Vec<Submission>,
}

impl Vat {
    pub fn from_events(events: &[VatEvent]) -> Self {
        let mut vat = Self::default();
        for event in events {
            vat.apply(event);
        }
        vat
    }

    pub fn apply(&mut self, event: &VatEvent) {
        match event {
            VatEvent::VatPeriodSet {
                fiscal_year_start,
                kind,
            } => {
                self.kinds.insert(*fiscal_year_start, *kind);
            }
            VatEvent::VatReturnSubmitted(submission) => self.submissions.push(submission.clone()),
        }
    }

    /// Quarterly until set.
    pub fn kind(&self, fiscal_year_start: Date) -> VatPeriodKind {
        self.kinds
            .get(&fiscal_year_start)
            .copied()
            .unwrap_or_default()
    }

    /// Every settlement voucher Doris has booked.
    pub fn vouchers(&self) -> Vec<VoucherRef> {
        self.submissions.iter().filter_map(|s| s.voucher).collect()
    }

    pub fn submissions_for(&self, period_end: Date) -> Vec<&Submission> {
        self.submissions
            .iter()
            .filter(|s| s.period_end == period_end)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VatStatus {
    InProgress,
    ToSubmit,
    Submitted,
    Changed,
}

/// The settlement voucher's lines and what they settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub lines: Vec<VoucherLine>,
    pub settled: Vec<(u16, i64)>,
    pub vat_due: i64,
}

/// `amount` öre on `account`: a debit when positive, a credit when negative.
fn line(account: u16, amount: i64) -> VoucherLine {
    let (debit, credit) = if amount > 0 {
        (amount, 0)
    } else {
        (0, -amount)
    };
    VoucherLine::new(account.into(), debit, credit).expect("settlement accounts are valid")
}

/// Moves each VAT account's saldo not yet settled by `earlier` to 2650,
/// books box 49 less what `earlier` booked there, and puts the öre left
/// on 3740 (öres- och kronutjämning). No lines when nothing is left.
pub fn settlement(accounts: &[AccountSaldo], boxes: &Boxes, earlier: &[&Submission]) -> Settlement {
    let mut open = BTreeMap::<u16, i64>::new();
    for a in accounts.iter().filter(|a| a.vat_box.is_vat()) {
        *open.entry(a.account).or_default() += a.saldo;
    }
    for s in earlier {
        for &(account, saldo) in &s.settled {
            *open.entry(account).or_default() -= saldo;
        }
    }
    open.retain(|_, saldo| *saldo != 0);
    let vat_due = boxes.vat_due * 100 - earlier.iter().map(|s| s.settled_vat_due).sum::<i64>();
    let mut lines: Vec<VoucherLine> = open.iter().map(|(&a, &saldo)| line(a, -saldo)).collect();
    if vat_due != 0 {
        lines.push(line(2650, -vat_due));
    }
    let rest: i64 = lines.iter().map(|l| l.debit - l.credit).sum();
    if rest != 0 {
        lines.push(line(3740, -rest));
    }
    Settlement {
        lines,
        settled: open.into_iter().collect(),
        vat_due,
    }
}

/// The period's submissions whose voucher has not been corrected: what
/// the books still hold as settled.
pub fn standing<'a>(
    vat: &'a Vat,
    period_end: Date,
    corrected: &HashSet<VoucherRef>,
) -> Vec<&'a Submission> {
    vat.submissions_for(period_end)
        .into_iter()
        .filter(|s| !s.voucher.is_some_and(|v| corrected.contains(&v)))
        .collect()
}

/// Submitted while the latest submission declared these very accounts and
/// nothing is left to settle; Changed otherwise.
pub fn status(
    vat: &Vat,
    period: VatPeriod,
    today: Date,
    accounts: &[AccountSaldo],
    corrected: &HashSet<VoucherRef>,
) -> VatStatus {
    if period.end >= today {
        return VatStatus::InProgress;
    }
    let Some(latest) = vat.submissions_for(period.end).last().copied() else {
        return VatStatus::ToSubmit;
    };
    let left = settlement(
        accounts,
        &boxes(accounts),
        &standing(vat, period.end, corrected),
    );
    if latest.accounts == accounts && left.lines.is_empty() {
        VatStatus::Submitted
    } else {
        VatStatus::Changed
    }
}

/// A period of `fiscal_year` has been submitted, so its kind stays.
pub fn is_locked(vat: &Vat, fiscal_year: FiscalYear) -> bool {
    vat.submissions
        .iter()
        .any(|s| fiscal_year.start <= s.period_end && s.period_end <= fiscal_year.end)
}

/// Setting the kind it already has yields no events. Once a period of the
/// year is submitted, its kind stays.
pub fn set_vat_period(
    vat: &Vat,
    fiscal_year: FiscalYear,
    kind: VatPeriodKind,
) -> Result<Vec<VatEvent>, DomainError> {
    if vat.kind(fiscal_year.start) == kind {
        return Ok(vec![]);
    }
    if is_locked(vat, fiscal_year) {
        return Err(DomainError::VatPeriodLocked);
    }
    Ok(vec![VatEvent::VatPeriodSet {
        fiscal_year_start: fiscal_year.start,
        kind,
    }])
}

/// The settlement to book and the submission to record once it has its
/// voucher number.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub lines: Vec<VoucherLine>,
    pub submission: Submission,
}

pub fn submit(
    vat: &Vat,
    period: VatPeriod,
    kind: VatPeriodKind,
    today: Date,
    accounts: Vec<AccountSaldo>,
    fingerprint_seen: &str,
    corrected: &HashSet<VoucherRef>,
) -> Result<Prepared, DomainError> {
    if kind == VatPeriodKind::NotRegistered {
        return Err(DomainError::VatNotRegistered);
    }
    if period.end >= today {
        return Err(DomainError::VatPeriodNotEnded);
    }
    if fingerprint_seen != fingerprint(period.end, &accounts) {
        return Err(DomainError::VatReturnOutdated);
    }
    if status(vat, period, today, &accounts, corrected) == VatStatus::Submitted {
        return Err(DomainError::VatReturnUnchanged);
    }
    let boxes = boxes(&accounts);
    let Settlement {
        lines,
        settled,
        vat_due,
    } = settlement(&accounts, &boxes, &standing(vat, period.end, corrected));
    Ok(Prepared {
        lines,
        submission: Submission {
            period_end: period.end,
            accounts,
            boxes,
            settled,
            settled_vat_due: vat_due,
            voucher: None,
        },
    })
}
