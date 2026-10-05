//! The overview's figures, worked out from what the existing RPCs return.
//! Pure: the page passes in the messages and today's date.

use crate::api::{ipb, lpb, ppb};
use crate::format::{amount, day_number, plus_days};

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

/// One line under "Att göra".
#[derive(Clone, Debug, PartialEq)]
pub struct Todo {
    /// Past its date: drawn with the destructive icon.
    pub urgent: bool,
    pub title: String,
    pub detail: String,
    pub action: &'static str,
    pub href: String,
}

/// What the to-do rules read.
pub struct TodoInput<'a> {
    pub supplier_invoices: &'a [ipb::SupplierInvoice],
    pub customer_invoices: &'a [ipb::CustomerInvoice],
    pub payroll_runs: &'a [ppb::PayrollRun],
    pub agi_months: &'a [ppb::AgiMonthSummary],
}

/// "1 faktura" or "2 fakturor".
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// An unpaid invoice as the rules see it: who, when it is due, how much.
struct Due<'a> {
    name: &'a str,
    date: &'a str,
    total: i64,
}

fn unpaid<'a>(status: &str, name: &'a str, date: &'a str, total: i64) -> Option<Due<'a>> {
    (status == "unpaid").then_some(Due { name, date, total })
}

/// "N … har förfallit": invoices due before `today`.
fn overdue(due: &[Due], today: &str, one: &str, many: &str, href: &str) -> Option<Todo> {
    let late: Vec<&Due> = due.iter().filter(|d| d.date < today).collect();
    let oldest = late.iter().min_by_key(|d| d.date)?;
    Some(Todo {
        urgent: true,
        title: format!("{} har förfallit", counted(late.len(), one, many)),
        detail: format!(
            "{} kr · äldst {}, förföll {}",
            amount(late.iter().map(|d| d.total).sum()),
            oldest.name,
            oldest.date
        ),
        action: "Visa fakturorna",
        href: href.to_owned(),
    })
}

/// The lines under "Att göra", most pressing first. Each rule gives at
/// most one line. Dates are `YYYY-MM-DD` and compare as strings.
pub fn todo_list(input: &TodoInput, today: &str) -> Vec<Todo> {
    let mut list = Vec::new();

    let suppliers: Vec<Due> = input
        .supplier_invoices
        .iter()
        .filter_map(|i| unpaid(&i.status, &i.supplier_name, &i.due_date, i.total))
        .collect();
    list.extend(overdue(
        &suppliers,
        today,
        "leverantörsfaktura",
        "leverantörsfakturor",
        "/supplier-invoices",
    ));
    if let Some(last) = plus_days(today, 30) {
        let soon: Vec<&Due> = suppliers
            .iter()
            .filter(|d| today <= d.date && d.date <= last.as_str())
            .collect();
        if let Some(next) = soon.iter().min_by_key(|d| d.date) {
            list.push(Todo {
                urgent: false,
                title: format!(
                    "{} förfaller inom 30 dagar",
                    counted(soon.len(), "leverantörsfaktura", "leverantörsfakturor")
                ),
                detail: format!(
                    "{} kr · nästa {}, {}",
                    amount(soon.iter().map(|d| d.total).sum()),
                    next.date,
                    next.name
                ),
                action: "Visa fakturorna",
                href: "/supplier-invoices".into(),
            });
        }
    }

    let customers: Vec<Due> = input
        .customer_invoices
        .iter()
        .filter_map(|i| unpaid(&i.status, &i.customer_name, &i.due_date, i.total))
        .collect();
    list.extend(overdue(
        &customers,
        today,
        "kundfaktura",
        "kundfakturor",
        "/customer-invoices",
    ));

    let named = |prefix: &str, run: &ppb::PayrollRun| {
        if run.text.is_empty() {
            format!("Utbetalning {}", run.pay_date)
        } else {
            format!("{prefix} {}, utbetalning {}", run.text, run.pay_date)
        }
    };
    let to_book: Vec<&ppb::PayrollRun> = input
        .payroll_runs
        .iter()
        .filter(|r| r.status() == ppb::PayrollRunStatus::Finalized && r.pay_date.as_str() <= today)
        .collect();
    if let Some(oldest) = to_book.iter().min_by_key(|r| r.pay_date.as_str()) {
        list.push(Todo {
            urgent: false,
            title: format!(
                "{} kan bokföras",
                counted(to_book.len(), "lönekörning", "lönekörningar")
            ),
            detail: named("Äldst", oldest),
            action: "Öppna körningen",
            href: format!("/payroll-runs/{}", oldest.id),
        });
    }
    let open: Vec<&ppb::PayrollRun> = input
        .payroll_runs
        .iter()
        .filter(|r| r.status() == ppb::PayrollRunStatus::Open)
        .collect();
    if let Some(nearest) = open.iter().min_by_key(|r| r.pay_date.as_str()) {
        list.push(Todo {
            urgent: false,
            title: format!(
                "{} är inte {}",
                counted(open.len(), "lönekörning", "lönekörningar"),
                if open.len() == 1 {
                    "färdigställd"
                } else {
                    "färdigställda"
                }
            ),
            detail: named("Närmast", nearest),
            action: "Öppna körningen",
            href: format!("/payroll-runs/{}", nearest.id),
        });
    }

    // A month is declared once it has ended: the period ("YYYYMM") is
    // before today's.
    let this_month: String = today.chars().filter(char::is_ascii_digit).take(6).collect();
    let mut periods: Vec<&str> = input
        .agi_months
        .iter()
        .filter(|m| {
            matches!(
                m.status(),
                ppb::AgiStatus::NotSubmitted | ppb::AgiStatus::Changed
            )
        })
        .map(|m| m.period.as_str())
        .filter(|p| *p < this_month.as_str())
        .collect();
    periods.sort_unstable();
    if !periods.is_empty() {
        let shown: Vec<String> = periods
            .iter()
            .take(3)
            .map(|p| match (p.get(..4), p.get(4..6)) {
                (Some(year), Some(month)) => format!("{year}-{month}"),
                _ => (*p).to_owned(),
            })
            .collect();
        let more = periods.len() - shown.len();
        list.push(Todo {
            urgent: false,
            title: format!(
                "Arbetsgivardeklarationen för {} är inte inlämnad",
                counted(periods.len(), "månad", "månader")
            ),
            detail: if more == 0 {
                shown.join(", ")
            } else {
                format!("{} och {more} till", shown.join(", "))
            },
            action: "Visa deklarationerna",
            href: "/agi".into(),
        });
    }

    list
}

/// The unpaid supplier invoices, earliest due first.
pub fn unpaid_supplier_invoices(invoices: &[ipb::SupplierInvoice]) -> Vec<&ipb::SupplierInvoice> {
    let mut unpaid: Vec<_> = invoices.iter().filter(|i| i.status == "unpaid").collect();
    unpaid.sort_by(|a, b| a.due_date.cmp(&b.due_date));
    unpaid
}

/// Whether the chart's axis counts thousands of kronor (from 2 000 kr up)
/// or kronor: below that the lines would read "0" and "1".
pub fn axis_in_thousands(scale: i64) -> bool {
    scale == 0 || scale >= 200_000
}

/// A gridline's label for an axis that tops out at `scale` (both öre), in
/// the axis's unit, with the decimals a 2.5 step needs: "125", "2,5".
pub fn axis_label(ore: i64, scale: i64) -> String {
    let unit = if axis_in_thousands(scale) {
        100_000
    } else {
        100
    };
    let (whole, hundredths) = (ore / unit, ore % unit * 100 / unit);
    let digits = whole.unsigned_abs().to_string();
    let mut label = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            label.push('\u{a0}');
        }
        label.push(digit);
    }
    match hundredths {
        0 => {}
        h if h % 10 == 0 => label.push_str(&format!(",{}", h / 10)),
        h => label.push_str(&format!(",{h:02}")),
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ipb, ppb};

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

    const TODAY: &str = "2026-10-04";

    fn supplier(name: &str, due: &str, total: i64, status: &str) -> ipb::SupplierInvoice {
        ipb::SupplierInvoice {
            supplier_name: name.into(),
            due_date: due.into(),
            total,
            status: status.into(),
            ..Default::default()
        }
    }

    fn customer(name: &str, due: &str, total: i64, status: &str) -> ipb::CustomerInvoice {
        ipb::CustomerInvoice {
            customer_name: name.into(),
            due_date: due.into(),
            total,
            status: status.into(),
            ..Default::default()
        }
    }

    fn run(id: &str, text: &str, pay_date: &str, status: ppb::PayrollRunStatus) -> ppb::PayrollRun {
        ppb::PayrollRun {
            id: id.into(),
            text: text.into(),
            pay_date: pay_date.into(),
            status: status as i32,
            ..Default::default()
        }
    }

    fn agi(period: &str, status: ppb::AgiStatus) -> ppb::AgiMonthSummary {
        ppb::AgiMonthSummary {
            period: period.into(),
            status: status as i32,
            ..Default::default()
        }
    }

    fn todos(input: TodoInput) -> Vec<Todo> {
        todo_list(&input, TODAY)
    }

    fn empty<'a>() -> TodoInput<'a> {
        TodoInput {
            supplier_invoices: &[],
            customer_invoices: &[],
            payroll_runs: &[],
            agi_months: &[],
        }
    }

    #[test]
    fn nothing_to_do_is_an_empty_list() {
        assert!(todos(empty()).is_empty());
    }

    #[test]
    fn overdue_supplier_invoices_are_counted_summed_and_urgent() {
        let invoices = [
            supplier("Kontorshuset AB", "2026-09-30", 12_500_00, "unpaid"),
            supplier("Telebolaget AB", "2026-10-03", 1_495_00, "unpaid"),
            supplier("Due today AB", "2026-10-04", 9_00, "unpaid"),
            supplier("Paid AB", "2026-09-01", 7_00, "paid"),
            supplier("Cancelled AB", "2026-09-01", 5_00, "cancelled"),
        ];
        let list = todos(TodoInput {
            supplier_invoices: &invoices,
            ..empty()
        });
        assert_eq!(
            list[0],
            Todo {
                urgent: true,
                title: "2 leverantörsfakturor har förfallit".into(),
                detail: "13\u{a0}995,00 kr · äldst Kontorshuset AB, förföll 2026-09-30".into(),
                action: "Visa fakturorna",
                href: "/supplier-invoices".into(),
            }
        );
    }

    #[test]
    fn supplier_invoices_due_within_thirty_days_are_listed_after_the_overdue() {
        let invoices = [
            supplier("Today AB", "2026-10-04", 100_00, "unpaid"),
            supplier("Day thirty AB", "2026-11-03", 200_00, "unpaid"),
            supplier("Day thirty-one AB", "2026-11-04", 400_00, "unpaid"),
            supplier("Overdue AB", "2026-10-03", 800_00, "unpaid"),
        ];
        let list = todos(TodoInput {
            supplier_invoices: &invoices,
            ..empty()
        });
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "1 leverantörsfaktura har förfallit");
        assert_eq!(
            list[1].title,
            "2 leverantörsfakturor förfaller inom 30 dagar"
        );
        assert_eq!(list[1].detail, "300,00 kr · nästa 2026-10-04, Today AB");
        assert!(!list[1].urgent);
    }

    #[test]
    fn overdue_customer_invoices_are_urgent_too() {
        let invoices = [
            customer("Kund AB", "2026-09-15", 62_500_00, "unpaid"),
            customer("Betald AB", "2026-09-15", 1_00, "paid"),
            customer("Ej förfallen AB", "2026-10-04", 1_00, "unpaid"),
        ];
        let list = todos(TodoInput {
            customer_invoices: &invoices,
            ..empty()
        });
        assert_eq!(
            list,
            [Todo {
                urgent: true,
                title: "1 kundfaktura har förfallit".into(),
                detail: "62\u{a0}500,00 kr · äldst Kund AB, förföll 2026-09-15".into(),
                action: "Visa fakturorna",
                href: "/customer-invoices".into(),
            }]
        );
    }

    #[test]
    fn payroll_runs_to_book_come_before_open_ones() {
        use ppb::PayrollRunStatus::*;
        let runs = [
            run("r1", "Lön oktober 2026", "2026-10-23", Open),
            run("r2", "Lön september 2026", "2026-09-25", Finalized),
            run("r3", "Lön augusti 2026", "2026-08-25", Finalized),
            run("r4", "Lön november 2026", "2026-11-25", Finalized),
            run("r5", "Lön juli 2026", "2026-07-24", Booked),
            run("r6", "Extra", "2026-10-10", Open),
        ];
        let list = todos(TodoInput {
            payroll_runs: &runs,
            ..empty()
        });
        assert_eq!(
            list,
            [
                Todo {
                    urgent: false,
                    title: "2 lönekörningar kan bokföras".into(),
                    detail: "Äldst Lön augusti 2026, utbetalning 2026-08-25".into(),
                    action: "Öppna körningen",
                    href: "/payroll-runs/r3".into(),
                },
                Todo {
                    urgent: false,
                    title: "2 lönekörningar är inte färdigställda".into(),
                    detail: "Närmast Extra, utbetalning 2026-10-10".into(),
                    action: "Öppna körningen",
                    href: "/payroll-runs/r6".into(),
                },
            ]
        );
    }

    #[test]
    fn a_single_payroll_run_reads_in_the_singular() {
        use ppb::PayrollRunStatus::*;
        let runs = [
            run("r1", "Lön oktober 2026", "2026-10-04", Finalized),
            run("r2", "", "2026-10-23", Open),
        ];
        let list = todos(TodoInput {
            payroll_runs: &runs,
            ..empty()
        });
        assert_eq!(list[0].title, "1 lönekörning kan bokföras");
        assert_eq!(list[1].title, "1 lönekörning är inte färdigställd");
        // A run without a text is named by its pay date alone.
        assert_eq!(list[1].detail, "Utbetalning 2026-10-23");
    }

    #[test]
    fn agi_months_that_have_ended_and_are_not_submitted_are_listed() {
        use ppb::AgiStatus::*;
        let months = [
            agi("202610", NotSubmitted),
            agi("202609", NotSubmitted),
            agi("202608", Changed),
            agi("202607", Submitted),
        ];
        let list = todos(TodoInput {
            agi_months: &months,
            ..empty()
        });
        assert_eq!(
            list,
            [Todo {
                urgent: false,
                title: "Arbetsgivardeklarationen för 2 månader är inte inlämnad".into(),
                detail: "2026-08, 2026-09".into(),
                action: "Visa deklarationerna",
                href: "/agi".into(),
            }]
        );
    }

    #[test]
    fn many_agi_months_are_cut_after_three() {
        use ppb::AgiStatus::NotSubmitted;
        let months: Vec<_> = ["202609", "202608", "202607", "202606", "202605"]
            .into_iter()
            .map(|p| agi(p, NotSubmitted))
            .collect();
        let list = todos(TodoInput {
            agi_months: &months,
            ..empty()
        });
        assert_eq!(
            list[0].title,
            "Arbetsgivardeklarationen för 5 månader är inte inlämnad"
        );
        assert_eq!(list[0].detail, "2026-05, 2026-06, 2026-07 och 2 till");
        let one = [agi("202609", NotSubmitted)];
        assert_eq!(
            todos(TodoInput {
                agi_months: &one,
                ..empty()
            })[0]
                .title,
            "Arbetsgivardeklarationen för 1 månad är inte inlämnad"
        );
    }

    #[test]
    fn the_rules_come_in_a_fixed_order() {
        use ppb::{AgiStatus, PayrollRunStatus};
        let suppliers = [
            supplier("A", "2026-09-01", 1_00, "unpaid"),
            supplier("B", "2026-10-20", 1_00, "unpaid"),
        ];
        let customers = [customer("C", "2026-09-01", 1_00, "unpaid")];
        let runs = [
            run("r1", "x", "2026-10-23", PayrollRunStatus::Open),
            run("r2", "y", "2026-09-25", PayrollRunStatus::Finalized),
        ];
        let months = [agi("202609", AgiStatus::NotSubmitted)];
        let list = todos(TodoInput {
            supplier_invoices: &suppliers,
            customer_invoices: &customers,
            payroll_runs: &runs,
            agi_months: &months,
        });
        let hrefs: Vec<_> = list.iter().map(|t| t.href.as_str()).collect();
        assert_eq!(
            hrefs,
            [
                "/supplier-invoices",
                "/supplier-invoices",
                "/customer-invoices",
                "/payroll-runs/r2",
                "/payroll-runs/r1",
                "/agi"
            ]
        );
    }

    #[test]
    fn unpaid_supplier_invoices_come_earliest_due_first() {
        let invoices = [
            supplier("Late", "2026-10-28", 1_00, "unpaid"),
            supplier("Paid", "2026-09-01", 1_00, "paid"),
            supplier("Early", "2026-09-30", 1_00, "unpaid"),
        ];
        let names: Vec<_> = unpaid_supplier_invoices(&invoices)
            .iter()
            .map(|i| i.supplier_name.as_str())
            .collect();
        assert_eq!(names, ["Early", "Late"]);
    }

    #[test]
    fn axis_labels_keep_the_half_steps() {
        // Thousands of kronor from 2 000 kr up, kronor below.
        assert!(axis_in_thousands(250_000_00));
        assert!(axis_in_thousands(2_000_00));
        assert!(!axis_in_thousands(1_999_00));
        assert!(axis_in_thousands(0));
        assert_eq!(axis_label(250_000_00, 250_000_00), "250");
        assert_eq!(axis_label(125_000_00, 250_000_00), "125");
        assert_eq!(axis_label(2_500_00, 5_000_00), "2,5");
        assert_eq!(axis_label(1_250_00, 2_500_00), "1,25");
        assert_eq!(axis_label(12_500_00, 25_000_00), "12,5");
        assert_eq!(axis_label(500_00, 1_000_00), "500");
        assert_eq!(axis_label(12_50, 25_00), "12,5");
        assert_eq!(axis_label(10_000_000_00, 10_000_000_00), "10\u{a0}000");
        assert_eq!(axis_label(0, 250_000_00), "0");
    }

    #[test]
    fn the_year_end_accounts_stay_out_of_the_months() {
        // The closing voucher books 8999 against the result account.
        let months = by_month(
            "2026-01-01",
            "2026-12-31",
            &[voucher(
                "2026-12-31",
                &[(8999, 27_700_00, 0), (2099, 0, 27_700_00), (8990, 5_00, 0)],
            )],
        );
        assert_eq!(months[11], month("2026-12", 0, 0));
    }
}
