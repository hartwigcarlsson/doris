//! Redovisningsperioder and their deklarationsdagar. Pure.

use doris_company::domain::FiscalYear;
use jiff::Span;
use jiff::civil::{Date, Weekday, date};
use serde::{Deserialize, Serialize};

/// How often a company declares VAT in a räkenskapsår.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VatPeriodKind {
    Monthly,
    #[default]
    Quarterly,
    Yearly,
    NotRegistered,
}

/// One period to declare: calendar months or quarters, or the whole
/// räkenskapsår.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VatPeriod {
    pub start: Date,
    pub end: Date,
}

const MONTHS: [&str; 12] = [
    "januari",
    "februari",
    "mars",
    "april",
    "maj",
    "juni",
    "juli",
    "augusti",
    "september",
    "oktober",
    "november",
    "december",
];

fn add_months(day: Date, months: i64) -> Date {
    day.checked_add(Span::new().months(months))
        .expect("periods are far from the date limits")
}

impl VatPeriod {
    /// "ÅÅÅÅMM" of the last month, as in the file and the URL.
    pub fn code(&self) -> String {
        format!("{:04}{:02}", self.end.year(), self.end.month())
    }

    /// "maj 2026", "juli–september 2026" or "maj 2026–april 2027".
    pub fn label(&self) -> String {
        let name = |d: Date| MONTHS[d.month() as usize - 1];
        let (start, end) = (self.start, self.end);
        if (start.year(), start.month()) == (end.year(), end.month()) {
            format!("{} {}", name(end), end.year())
        } else if start.year() == end.year() {
            format!("{}–{} {}", name(start), name(end), end.year())
        } else {
            format!(
                "{} {}–{} {}",
                name(start),
                start.year(),
                name(end),
                end.year()
            )
        }
    }
}

/// The periods declared for `fiscal_year`: months and calendar quarters
/// whose last month is in it (so a quarter may start in the year before),
/// or the year itself.
pub fn periods(fiscal_year: FiscalYear, kind: VatPeriodKind) -> Vec<VatPeriod> {
    let mut months = Vec::new();
    let mut month = fiscal_year.start.first_of_month();
    while month <= fiscal_year.end {
        months.push(month);
        month = add_months(month, 1);
    }
    match kind {
        VatPeriodKind::Monthly => months
            .into_iter()
            .map(|m| VatPeriod {
                start: m,
                end: m.last_of_month(),
            })
            .collect(),
        VatPeriodKind::Quarterly => months
            .into_iter()
            .filter(|m| m.month() % 3 == 0)
            .map(|m| VatPeriod {
                start: add_months(m, -2),
                end: m.last_of_month(),
            })
            .collect(),
        VatPeriodKind::Yearly => vec![VatPeriod {
            start: fiscal_year.start,
            end: fiscal_year.end,
        }],
        VatPeriodKind::NotRegistered => vec![],
    }
}

/// The periods of `fiscal_year` when `previous` (the year before and its
/// kind) is known: the first period starts the day after the year before's
/// last period, so a change of kind in a broken year neither declares a
/// month twice nor skips one. Unregistered years before change nothing.
pub fn periods_after(
    fiscal_year: FiscalYear,
    kind: VatPeriodKind,
    previous: Option<(FiscalYear, VatPeriodKind)>,
) -> Vec<VatPeriod> {
    let mut list = periods(fiscal_year, kind);
    let last_before = previous.and_then(|(year, kind)| periods(year, kind).last().copied());
    if let (Some(first), Some(before)) = (list.first_mut(), last_before) {
        first.start = before.end.tomorrow().expect("far from the date limits");
    }
    list
}

/// When a month or quarter must be declared (turnover up to 40 MSEK): the
/// 12th of the second month after it, the 17th when that month is January
/// or August, moved on to the next workday.
// ponytail: no date for helår (it depends on legal form and EU trade) nor
// for turnover over 40 MSEK (the 26th); add when such a company uses Doris.
pub fn due_date(period: VatPeriod, kind: VatPeriodKind) -> Option<Date> {
    if !matches!(kind, VatPeriodKind::Monthly | VatPeriodKind::Quarterly) {
        return None;
    }
    let month = add_months(period.end.first_of_month(), 2);
    let day = if matches!(month.month(), 1 | 8) {
        17
    } else {
        12
    };
    let mut due = date(month.year(), month.month(), day);
    while !is_workday(due) {
        due = due.tomorrow().expect("far from the date limits");
    }
    Some(due)
}

/// Not a weekend or a helgdag. Of the helgdagar only långfredagen,
/// annandag påsk and Kristi himmelsfärdsdag can fall on the days a
/// declaration is due.
fn is_workday(day: Date) -> bool {
    if matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
        return false;
    }
    let easter = easter_sunday(day.year());
    ![-2, 1, 39]
        .iter()
        .any(|&days| easter.checked_add(Span::new().days(days)).ok() == Some(day))
}

/// Påskdagen (the anonymous Gregorian algorithm).
fn easter_sunday(year: i16) -> Date {
    let y = i32::from(year);
    let (a, b, c) = (y % 19, y / 100, y % 100);
    let (d, e) = (b / 4, b % 4);
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let (i, k) = (c / 4, c % 4);
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let n = h + l - 7 * m + 114;
    date(year, (n / 31) as i8, (n % 31 + 1) as i8)
}
