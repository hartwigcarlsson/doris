//! The overview's figures, worked out from what the existing RPCs return.
//! Pure: the page passes in the messages and today's date.

// Used by the overview page from the next commits on.
#![allow(dead_code)]

use crate::api::lpb;
use crate::format::day_number;

/// The year's key figures, in öre. A profit is a positive `result`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyFigures {
    pub income: i64,
    pub costs: i64,
    pub result: i64,
    pub cash: i64,
}

/// Income is accounts 3000–3999 and costs 4000–8989, the accounts behind
/// "Årets resultat" in the statements; cash and bank is 1900–1999 with
/// its opening balance.
pub fn key_figures(rows: &[lpb::TrialBalanceRow]) -> KeyFigures {
    let mut figures = KeyFigures::default();
    for row in rows {
        let movement = row.debit - row.credit;
        match row.account {
            1900..=1999 => figures.cash += row.opening + movement,
            3000..=3999 => figures.income -= movement,
            4000..=8989 => figures.costs += movement,
            _ => {}
        }
    }
    figures.result = figures.income - figures.costs;
    figures
}

/// Öre as whole kronor for a headline figure: 30433099 → "304 330 kr".
/// The öre are dropped, toward zero.
pub fn whole_kronor(ore: i64) -> String {
    let kronor = ore / 100;
    let digits = kronor.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('\u{a0}');
        }
        grouped.push(digit);
    }
    let sign = if kronor < 0 { "-" } else { "" };
    format!("{sign}{grouped}\u{a0}kr")
}

/// How far into a fiscal year `today` is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// 1 on the first day; 0 before the year, `days` after it.
    pub day: i64,
    pub days: i64,
    pub left: i64,
    pub percent: u8,
}

/// `None` unless `start <= end` are dates and `today` is one.
pub fn progress(start: &str, end: &str, today: &str) -> Option<Progress> {
    let (first, last, now) = (day_number(start)?, day_number(end)?, day_number(today)?);
    if last < first {
        return None;
    }
    let days = last - first + 1;
    let day = (now - first + 1).clamp(0, days);
    Some(Progress {
        day,
        days,
        left: days - day,
        percent: (day * 100 / days) as u8,
    })
}

/// The year the overview opens on: `preferred` (from the URL) if listed,
/// else the year that contains `today`, else the newest (the first).
pub fn default_year(years: &[lpb::FiscalYear], preferred: &str, today: &str) -> String {
    years
        .iter()
        .find(|y| y.start == preferred)
        .or_else(|| {
            years
                .iter()
                .find(|y| y.start.as_str() <= today && today <= y.end.as_str())
        })
        .or(years.first())
        .map(|y| y.start.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(account: u32, opening: i64, debit: i64, credit: i64) -> lpb::TrialBalanceRow {
        lpb::TrialBalanceRow {
            account,
            opening,
            debit,
            credit,
            ..Default::default()
        }
    }

    fn year(start: &str, end: &str) -> lpb::FiscalYear {
        lpb::FiscalYear {
            start: start.into(),
            end: end.into(),
            closed: false,
        }
    }

    #[test]
    fn key_figures_split_income_costs_and_cash() {
        let figures = key_figures(&[
            row(1930, 10_000_00, 50_000_00, 20_000_00),
            row(1510, 0, 5_000_00, 0),
            row(3001, 0, 0, 40_000_00),
            row(5010, 0, 12_000_00, 0),
            row(8410, 0, 300_00, 0),
        ]);
        assert_eq!(
            figures,
            KeyFigures {
                income: 40_000_00,
                costs: 12_300_00,
                result: 27_700_00,
                cash: 40_000_00,
            }
        );
    }

    #[test]
    fn key_figures_respect_the_account_boundaries() {
        // 2999 and 8990 belong to neither income nor costs; 1899 and 2000 are not cash.
        let figures = key_figures(&[
            row(2999, 0, 0, 1_00),
            row(3000, 0, 0, 2_00),
            row(3999, 0, 0, 4_00),
            row(4000, 0, 8_00, 0),
            row(8989, 0, 16_00, 0),
            row(8990, 0, 32_00, 0),
            row(8999, 0, 64_00, 0),
            row(1899, 5_00, 0, 0),
            row(1900, 7_00, 0, 0),
            row(1999, 0, 11_00, 0),
            row(2000, 13_00, 0, 0),
        ]);
        assert_eq!(figures.income, 6_00);
        assert_eq!(figures.costs, 24_00);
        assert_eq!(figures.result, -18_00);
        assert_eq!(figures.cash, 18_00);
    }

    #[test]
    fn key_figures_of_nothing_are_zero() {
        assert_eq!(key_figures(&[]), KeyFigures::default());
    }

    #[test]
    fn whole_kronor_groups_and_truncates_toward_zero() {
        assert_eq!(whole_kronor(0), "0\u{a0}kr");
        assert_eq!(whole_kronor(99), "0\u{a0}kr");
        assert_eq!(whole_kronor(304_330_99), "304\u{a0}330\u{a0}kr");
        assert_eq!(whole_kronor(-1_234_56), "-1\u{a0}234\u{a0}kr");
        assert_eq!(whole_kronor(-99), "0\u{a0}kr");
        assert_eq!(
            whole_kronor(123_456_789_012_00),
            "123\u{a0}456\u{a0}789\u{a0}012\u{a0}kr"
        );
    }

    #[test]
    fn progress_counts_days_into_the_year() {
        let p = |today| progress("2026-01-01", "2026-12-31", today).unwrap();
        assert_eq!(
            p("2026-01-01"),
            Progress {
                day: 1,
                days: 365,
                left: 364,
                percent: 0
            }
        );
        assert_eq!(
            p("2026-10-04"),
            Progress {
                day: 277,
                days: 365,
                left: 88,
                percent: 75
            }
        );
        assert_eq!(
            p("2026-12-31"),
            Progress {
                day: 365,
                days: 365,
                left: 0,
                percent: 100
            }
        );
    }

    #[test]
    fn progress_is_clamped_outside_the_year() {
        let p = |today| progress("2026-01-01", "2026-12-31", today).unwrap();
        assert_eq!(
            p("2025-06-01"),
            Progress {
                day: 0,
                days: 365,
                left: 365,
                percent: 0
            }
        );
        assert_eq!(
            p("2027-03-01"),
            Progress {
                day: 365,
                days: 365,
                left: 0,
                percent: 100
            }
        );
    }

    #[test]
    fn progress_knows_leap_years_and_refuses_nonsense() {
        assert_eq!(
            progress("2028-01-01", "2028-12-31", "2028-03-01")
                .unwrap()
                .days,
            366
        );
        assert_eq!(
            progress("2028-01-01", "2028-12-31", "2028-03-01")
                .unwrap()
                .day,
            61
        );
        assert_eq!(progress("", "2026-12-31", "2026-01-01"), None);
        assert_eq!(progress("2026-12-31", "2026-01-01", "2026-06-01"), None);
    }

    #[test]
    fn the_default_year_is_the_preferred_one_then_the_one_with_today_then_the_newest() {
        // Newest first, as the server lists them.
        let years = [
            year("2027-01-01", "2027-12-31"),
            year("2026-01-01", "2026-12-31"),
            year("2025-01-01", "2025-12-31"),
        ];
        assert_eq!(
            default_year(&years, "2025-01-01", "2026-10-04"),
            "2025-01-01"
        );
        assert_eq!(default_year(&years, "", "2026-10-04"), "2026-01-01");
        assert_eq!(
            default_year(&years, "1999-01-01", "2026-10-04"),
            "2026-01-01"
        );
        assert_eq!(default_year(&years, "", "2030-01-01"), "2027-01-01");
        assert_eq!(default_year(&[], "", "2026-10-04"), "");
    }
}
