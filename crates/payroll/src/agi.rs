//! Arbetsgivardeklaration på individnivå (AGI): what a month declares to
//! Skatteverket, computed from booked payroll runs, and the file that
//! carries it. Pure: no I/O, no clock.
//!
//! Skatteverket, "Teknisk beskrivning 1.1.18.2 för arbetsgivardeklaration".

use crate::domain::{DomainError, Payroll, PayrollEvent, PayrollRunStatus, fee_bases};
use jiff::civil::{Date, date};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

/// A redovisningsperiod: the pay date's year and month, `ÅÅÅÅMM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Period(u32);

impl Period {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let raw = raw.trim();
        if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DomainError::InvalidPeriod);
        }
        let value: u32 = raw.parse().map_err(|_| DomainError::InvalidPeriod)?;
        let (year, month) = (value / 100, value % 100);
        if year < 1900 || !(1..=12).contains(&month) {
            return Err(DomainError::InvalidPeriod);
        }
        Ok(Self(value))
    }

    pub fn of(day: Date) -> Self {
        Self(day.year() as u32 * 100 + day.month() as u32)
    }

    pub fn first_day(self) -> Date {
        date((self.0 / 100) as i16, (self.0 % 100) as i8, 1)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Period {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who Skatteverket may contact about the company's AGI. The file needs a
/// name, a phone number and an e-mail address (schema types TEXT50, TEXT20
/// and EPOST).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgiContact {
    pub name: String,
    pub phone: String,
    pub email: String,
}

impl AgiContact {
    pub fn parse(name: &str, phone: &str, email: &str) -> Result<Self, DomainError> {
        let (name, phone, email) = (name.trim(), phone.trim(), email.trim());
        // TEXT50/TEXT20: not empty, not only whitespace (trimmed), no < or >.
        let text =
            |s: &str, max: usize| (1..=max).contains(&s.chars().count()) && !s.contains(['<', '>']);
        if text(name, 50) && text(phone, 20) && is_email(email) {
            Ok(Self {
                name: name.to_owned(),
                phone: phone.to_owned(),
                email: email.to_owned(),
            })
        } else {
            Err(DomainError::InvalidAgiContact)
        }
    }
}

/// Skatteverket's EPOST pattern, 5–254 characters:
/// `[a-zA-Z0-9_]+([-+.'][a-zA-Z0-9_]+)*@[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*\.[a-zA-Z0-9_]+([-.][a-zA-Z0-9_]+)*`
fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    (5..=254).contains(&s.chars().count())
        && words(local, &['-', '+', '.', '\'']).is_some()
        && words(domain, &['-', '.']).is_some_and(|dots| dots >= 1)
}

/// `s` as runs of `[A-Za-z0-9_]` joined by single separators from `seps`,
/// starting and ending with a run; returns how many separators were dots.
fn words(s: &str, seps: &[char]) -> Option<usize> {
    let (mut dots, mut in_word) = (0, false);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            in_word = true;
        } else if in_word && seps.contains(&c) {
            in_word = false;
            dots += usize::from(c == '.');
        } else {
            return None;
        }
    }
    in_word.then_some(dots)
}

/// One individuppgift: what one employee was paid and had deducted in a
/// period, in whole kronor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgiLine {
    pub employee_id: Uuid,
    /// Field 570: the same for an employee in every period.
    pub specification_number: u64,
    /// Field 011: cash gross salary.
    pub gross: i64,
    /// Field 001: deducted preliminary tax.
    pub tax: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgiSubmission {
    pub lines: Vec<AgiLine>,
    pub fee_sum: i64,
    pub tax_sum: i64,
}

/// How a line differs from the latest submission of its period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgiChange {
    New,
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgiStatus {
    NotSubmitted,
    Submitted,
    /// Submitted, but the books now say something else.
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgiMonth {
    pub period: Period,
    /// The individuppgifter now, by specification number.
    pub lines: Vec<(AgiLine, AgiChange)>,
    /// Submitted earlier, no longer paid this period: a Borttag each.
    pub removed: Vec<AgiLine>,
    /// Field 487.
    pub fee_sum: i64,
    /// Field 497.
    pub tax_sum: i64,
    /// Öre booked on 2731 for the period's runs, for comparison.
    pub booked_fees: i64,
    pub status: AgiStatus,
}

/// Gross, tax and fee in öre per employee, from the locked lines of the
/// booked runs paid in `period`.
fn booked_amounts(payroll: &Payroll, period: Period) -> BTreeMap<Uuid, (i64, i64, i64)> {
    let mut amounts = BTreeMap::<Uuid, (i64, i64, i64)>::new();
    let booked = payroll.runs.iter().filter(|r| {
        Period::of(r.draft.pay_date) == period
            && matches!(payroll.status(r), PayrollRunStatus::Booked(_))
    });
    for line in booked.flat_map(|r| r.lines.iter().flatten()) {
        let entry = amounts.entry(line.employee_id).or_default();
        entry.0 += line.gross;
        entry.1 += line.tax;
        entry.2 += line.fee;
    }
    amounts
}

/// The individuppgifter for `period`: one per employee paid by a booked
/// run, amounts summed and then rounded down to whole kronor. Employees
/// never submitted get the next numbers, in name order.
pub fn agi_lines(payroll: &Payroll, period: Period) -> Vec<AgiLine> {
    let amounts = booked_amounts(payroll, period);
    // Under one krona is not an individuppgift, and takes no number.
    let amounts: BTreeMap<_, _> = amounts
        .into_iter()
        .filter(|(_, (gross, tax, _))| gross / 100 > 0 || tax / 100 > 0)
        .collect();
    let mut numbers: HashMap<Uuid, u64> = payroll
        .specification_numbers
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    let next = numbers.values().max().copied().unwrap_or(0) + 1;
    let name = |id: &Uuid| {
        payroll
            .employee(*id)
            .map(|e| e.name.as_str().to_owned())
            .unwrap_or_default()
    };
    let mut new: Vec<Uuid> = amounts
        .keys()
        .filter(|id| !numbers.contains_key(id))
        .copied()
        .collect();
    new.sort_by_key(|id| (name(id), *id));
    numbers.extend(new.into_iter().zip(next..));
    let mut lines: Vec<AgiLine> = amounts
        .iter()
        .map(|(id, (gross, tax, _))| AgiLine {
            employee_id: *id,
            specification_number: numbers[id],
            gross: gross / 100,
            tax: tax / 100,
        })
        .collect();
    lines.sort_by_key(|l| l.specification_number);
    lines
}

/// Field 487 the way Skatteverket computes it (BeraknadSummaArbAvgSlf):
/// each individuppgift's gross split over the fee rates of the period
/// (the youth cap per individuppgift), multiplied, summed, rounded down.
pub fn agi_fee_sum(payroll: &Payroll, period: Period, lines: &[AgiLine]) -> i64 {
    let day = period.first_day();
    let total: i64 = lines
        .iter()
        .flat_map(|l| {
            let birth = payroll
                .employee(l.employee_id)
                .expect("lines come from employees")
                .personal_identity_number
                .birth_year();
            fee_bases(birth, day, l.gross * 100, 0)
        })
        .map(|(rate, base)| base / 100 * i64::from(rate))
        .sum();
    total / 10_000
}

pub fn agi_month(payroll: &Payroll, period: Period) -> AgiMonth {
    let previous = payroll.agi_submissions.get(&period);
    let before = |id: Uuid| previous.and_then(|s| s.lines.iter().find(|l| l.employee_id == id));
    let now = agi_lines(payroll, period);
    let lines: Vec<(AgiLine, AgiChange)> = now
        .iter()
        .map(|l| {
            let change = match before(l.employee_id) {
                None => AgiChange::New,
                Some(was) if was == l => AgiChange::Unchanged,
                Some(_) => AgiChange::Changed,
            };
            (*l, change)
        })
        .collect();
    let paid: BTreeSet<Uuid> = now.iter().map(|l| l.employee_id).collect();
    let removed: Vec<AgiLine> = previous
        .map(|s| {
            s.lines
                .iter()
                .filter(|l| !paid.contains(&l.employee_id))
                .copied()
                .collect()
        })
        .unwrap_or_default();
    let fee_sum = agi_fee_sum(payroll, period, &now);
    let tax_sum = now.iter().map(|l| l.tax).sum();
    let booked_fees = booked_amounts(payroll, period)
        .values()
        .map(|(_, _, fee)| fee)
        .sum();
    let status = match previous {
        None => AgiStatus::NotSubmitted,
        Some(s)
            if removed.is_empty()
                && lines.iter().all(|(_, c)| *c == AgiChange::Unchanged)
                && (s.fee_sum, s.tax_sum) == (fee_sum, tax_sum) =>
        {
            AgiStatus::Submitted
        }
        Some(_) => AgiStatus::Changed,
    };
    AgiMonth {
        period,
        lines,
        removed,
        fee_sum,
        tax_sum,
        booked_fees,
        status,
    }
}

/// Periods with booked runs or a submission, newest first.
pub fn agi_periods(payroll: &Payroll) -> Vec<Period> {
    let mut periods: BTreeSet<Period> = payroll.agi_submissions.keys().copied().collect();
    periods.extend(
        payroll
            .runs
            .iter()
            .filter(|r| matches!(payroll.status(r), PayrollRunStatus::Booked(_)))
            .map(|r| Period::of(r.draft.pay_date)),
    );
    periods.into_iter().rev().collect()
}

/// Records what the month declares now, after the user has uploaded the
/// file. A month emptied by backed-out runs may be submitted (as removals).
pub fn submit_agi_month(payroll: &Payroll, period: Period) -> Result<PayrollEvent, DomainError> {
    let month = agi_month(payroll, period);
    if month.lines.is_empty() && !payroll.agi_submissions.contains_key(&period) {
        return Err(DomainError::AgiPeriodEmpty);
    }
    if month.status == AgiStatus::Submitted {
        return Err(DomainError::AgiUnchanged);
    }
    if payroll.agi_contact.is_none() {
        return Err(DomainError::AgiContactMissing);
    }
    Ok(PayrollEvent::AgiMonthSubmitted {
        period,
        lines: month.lines.iter().map(|(l, _)| *l).collect(),
        fee_sum: month.fee_sum,
        tax_sum: month.tax_sum,
    })
}

pub fn set_agi_contact(payroll: &Payroll, contact: AgiContact) -> Vec<PayrollEvent> {
    if payroll.agi_contact.as_ref() == Some(&contact) {
        return vec![];
    }
    vec![PayrollEvent::AgiContactChanged { contact }]
}
