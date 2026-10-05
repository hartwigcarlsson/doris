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

/// One month's income and costs, in öre.
#[derive(Clone, Debug, PartialEq)]
pub struct Month {
    /// `YYYY-MM`.
    pub month: String,
    pub income: i64,
    pub costs: i64,
}

/// `(year, month)` of a `YYYY-MM…` string.
fn year_month(date: &str) -> Option<(i32, u32)> {
    let year = date.get(..4)?.parse().ok()?;
    let month = date.get(5..7)?.parse().ok()?;
    (date.as_bytes().get(4) == Some(&b'-') && (1..=12).contains(&month)).then_some((year, month))
}

/// Income and costs for every month from `start` to `end`, in order, also
/// the empty ones. A correction is a voucher like any other, so it cancels
/// what it corrects. Vouchers outside the months are left out.
// ponytail: sums every voucher line in the browser; fine for a few
// thousand vouchers. When ListVouchers gets heavy, add GetMonthlyTotals
// to the ledger and read it here.
pub fn by_month(start: &str, end: &str, vouchers: &[lpb::Voucher]) -> Vec<Month> {
    let (Some((first_year, first_month)), Some((last_year, last_month))) =
        (year_month(start), year_month(end))
    else {
        return Vec::new();
    };
    let count = (last_year - first_year) * 12 + last_month as i32 - first_month as i32 + 1;
    let mut months: Vec<Month> = (0..count.max(0))
        .map(|i| {
            let index = first_month as i32 - 1 + i;
            Month {
                month: format!("{:04}-{:02}", first_year + index / 12, index % 12 + 1),
                income: 0,
                costs: 0,
            }
        })
        .collect();
    for voucher in vouchers {
        let Some(month) = voucher
            .date
            .get(..7)
            .and_then(|key| months.iter_mut().find(|m| m.month == key))
        else {
            continue;
        };
        for line in &voucher.lines {
            let movement = line.debit - line.credit;
            match line.account {
                3000..=3999 => month.income -= movement,
                4000..=8989 => month.costs += movement,
                _ => {}
            }
        }
    }
    months
}

/// The chart's axis maximum: the smallest of 1, 2, 2.5 or 5 times a power
/// of ten that holds the largest value. 0 when there is nothing to draw.
pub fn scale(months: &[Month]) -> i64 {
    let largest = months
        .iter()
        .flat_map(|m| [m.income, m.costs])
        .max()
        .unwrap_or(0);
    if largest <= 0 {
        return 0;
    }
    if largest < 10 {
        return largest;
    }
    let mut power: i64 = 1;
    loop {
        for step in [10, 20, 25, 50] {
            let Some(candidate) = power.checked_mul(step) else {
                return largest;
            };
            if candidate >= largest {
                return candidate;
            }
        }
        let Some(next) = power.checked_mul(10) else {
            return largest;
        };
        power = next;
    }
}

/// A bar's height in pixels out of `full`; at least 1 for any positive
/// value, 0 for none or a negative one.
pub fn bar_height(value: i64, scale: i64, full: u32) -> u32 {
    if value <= 0 || scale <= 0 {
        return 0;
    }
    let height = (i128::from(value) * i128::from(full) / i128::from(scale)) as u32;
    height.clamp(1, full)
}

/// "2026-10" → "okt"; "" for anything else.
pub fn month_label(month: &str) -> &'static str {
    const NAMES: [&str; 12] = [
        "jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
    ];
    year_month(month).map_or("", |(_, m)| NAMES[m as usize - 1])
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

    fn voucher(date: &str, lines: &[(u32, i64, i64)]) -> lpb::Voucher {
        lpb::Voucher {
            date: date.into(),
            lines: lines
                .iter()
                .map(|&(account, debit, credit)| lpb::VoucherLine {
                    account,
                    debit,
                    credit,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn month(month: &str, income: i64, costs: i64) -> Month {
        Month {
            month: month.into(),
            income,
            costs,
        }
    }

    #[test]
    fn a_calendar_year_has_twelve_months_in_order() {
        let months = by_month("2026-01-01", "2026-12-31", &[]);
        assert_eq!(months.len(), 12);
        assert_eq!(months[0], month("2026-01", 0, 0));
        assert_eq!(months[11], month("2026-12", 0, 0));
    }

    #[test]
    fn a_broken_or_extended_year_has_its_own_months() {
        let broken = by_month("2026-07-01", "2027-06-30", &[]);
        assert_eq!(broken.len(), 12);
        assert_eq!(broken[0].month, "2026-07");
        assert_eq!(broken[6].month, "2027-01");
        assert_eq!(by_month("2026-07-01", "2027-12-31", &[]).len(), 18);
        assert_eq!(by_month("2026-10-01", "2026-12-31", &[]).len(), 3);
    }

    #[test]
    fn vouchers_land_in_their_month_by_account() {
        let months = by_month(
            "2026-01-01",
            "2026-12-31",
            &[
                voucher(
                    "2026-01-15",
                    &[(1930, 1_250_00, 0), (3001, 0, 1_000_00), (2611, 0, 250_00)],
                ),
                voucher("2026-01-31", &[(5010, 400_00, 0), (1930, 0, 400_00)]),
                voucher("2026-03-01", &[(1930, 80_00, 0), (3001, 0, 80_00)]),
            ],
        );
        assert_eq!(months[0], month("2026-01", 1_000_00, 400_00));
        assert_eq!(months[1], month("2026-02", 0, 0));
        assert_eq!(months[2], month("2026-03", 80_00, 0));
    }

    #[test]
    fn a_correction_cancels_its_voucher() {
        let months = by_month(
            "2026-01-01",
            "2026-12-31",
            &[
                voucher("2026-02-10", &[(5010, 400_00, 0), (1930, 0, 400_00)]),
                voucher("2026-02-12", &[(5010, 0, 400_00), (1930, 400_00, 0)]),
            ],
        );
        assert_eq!(months[1], month("2026-02", 0, 0));
    }

    #[test]
    fn a_voucher_outside_the_year_or_without_a_date_is_left_out() {
        let months = by_month(
            "2026-01-01",
            "2026-12-31",
            &[
                voucher("2025-12-31", &[(3001, 0, 1_00)]),
                voucher("", &[(3001, 0, 1_00)]),
                voucher("x", &[(3001, 0, 1_00)]),
            ],
        );
        assert!(months.iter().all(|m| m.income == 0 && m.costs == 0));
        assert!(by_month("nonsense", "2026-12-31", &[]).is_empty());
        assert!(by_month("2026-12-01", "2026-01-31", &[]).is_empty());
    }

    #[test]
    fn the_scale_is_the_next_round_number() {
        let of = |income, costs| scale(&[month("2026-01", income, costs)]);
        assert_eq!(of(0, 0), 0);
        assert_eq!(of(1, 0), 1);
        assert_eq!(of(241_000_00, 173_000_00), 250_000_00);
        assert_eq!(of(100_000_00, 0), 100_000_00);
        assert_eq!(of(100_000_01, 0), 200_000_00);
        assert_eq!(of(0, 450_000_00), 500_000_00);
        // A month where corrections outweigh is not what the axis is for.
        assert_eq!(of(-5_000_00, 0), 0);
        assert_eq!(of(900_000_000_000_00, 0), 1_000_000_000_000_00);
    }

    #[test]
    fn a_bar_is_a_share_of_the_full_height() {
        assert_eq!(bar_height(125_000_00, 250_000_00, 160), 80);
        assert_eq!(bar_height(250_000_00, 250_000_00, 160), 160);
        assert_eq!(bar_height(0, 250_000_00, 160), 0);
        assert_eq!(bar_height(-1, 250_000_00, 160), 0);
        assert_eq!(bar_height(1, 0, 160), 0);
        // Something booked is always visible.
        assert_eq!(bar_height(1, 250_000_00, 160), 1);
    }

    #[test]
    fn months_have_swedish_short_names() {
        assert_eq!(month_label("2026-01"), "jan");
        assert_eq!(month_label("2026-05"), "maj");
        assert_eq!(month_label("2026-10"), "okt");
        assert_eq!(month_label("2026-13"), "");
        assert_eq!(month_label(""), "");
    }
}
