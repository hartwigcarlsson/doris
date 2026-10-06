# Plan 16: New Design, Part 2 (Overview Page) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The start page shows the active company's overview for a chosen fiscal year: key figures, a to-do list, the year's status, income and costs per month, unpaid supplier invoices and the latest vouchers.

**Architecture:**
- A new pure module `crates/web/src/overview.rs` does every calculation from the messages eight existing RPCs return. No server change.
- `crates/web/src/pages/home.rs` is rewritten: it loads the data (per company, and per year), and draws the cards with the shared components from Plan 15.
- Each card owns the result it needs, so a failed call only fails the cards that depend on it.

**Tech Stack:** Rust, Leptos 0.8 CSR, Tailwind v4, Playwright 1.63.

**Spec:** `docs/superpowers/specs/2026-10-05-ny-design-oversikten-design.md`. Visual reference: artboard "A2" on the page "Runda 2 · Meny" of https://claude.ai/artifact/FpdLiRfFggm3ZVSuM6nyUL.

## Global Constraints
- Frontend only: no events, commands, projections, migrations, protos or RPCs.
- No new dependency.
- Amounts are öre (`i64`); sums use plain `+` on values the server already bounded.
- Account ranges: income 3000–3999, costs 4000–8989, result 3000–8989, cash and bank 1900–1999.
- "Today" is the browser's date (`crate::format::today()`), passed into the pure functions as `&str` (`YYYY-MM-DD`). Dates compare as strings.
- The only change to `crates/web/style/input.css` is `--chart-1` and `--chart-2` (values in the spec).
- A personnummer or email is never logged; this plan logs nothing.
- Code, identifiers, comments and commits are English; UI text is Swedish.
- TDD: a failing test first. Each task ends green on `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`, and the Playwright suite (`make web && cargo build -p doris-server && cd e2e && npx playwright test`).
- Commits end with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01JHpfH7CxMW86ZHVLURbK65
  ```

## Review Focus
1. **A company whose current year has not started or has ended** (only a past or a future year exists): the progress must clamp, not go negative or past 100 %. Pinned in Task 1 (`progress` tests).
2. **A voucher dated outside the listed months, or a malformed date**: `by_month` must ignore it rather than panic on a slice. Pinned in Task 2.
3. **A month where corrections outweigh bookings** (negative income): the bar is drawn as 0 and the scale ignores it. Pinned in Task 2 (`scale`, `bar height`).
4. **Switching company or year while calls are in flight**: a late answer for the old company or year must not be shown. Pinned in Task 4 (e2e "the overview follows the chosen year") and by the stale checks in the loader.
5. **Very large amounts** (a 12-digit sum): whole-kronor formatting and the scale must not overflow or lose grouping. Pinned in Task 1 (`whole_kronor`) and Task 2 (`scale`).

---

## File Structure
| File | Responsibility |
|---|---|
| `crates/web/src/overview.rs` (create) | Pure calculations: `key_figures`, `whole_kronor`, `progress`, `default_year`, `by_month`, `scale`, `bar_height`, `todo`. |
| `crates/web/src/format.rs` (modify) | `day_number`, shared by `plus_days` and the overview. |
| `crates/web/src/main.rs` (modify) | `mod overview;` |
| `crates/web/src/pages/home.rs` (rewrite) | Loading and the cards. |
| `crates/web/style/input.css` (modify) | `--chart-1`, `--chart-2`, mapped to `--color-chart-1/2`. |
| `e2e/tests/overview.spec.ts` (create) | The overview's browser tests. |
| `e2e/tests/fixtures.ts`, `auth.spec.ts`, `companies.spec.ts`, `design.spec.ts` (modify) | No longer rely on the "Inloggad som" and "Aktivt företag" cards. |
| `AGENTS.md` (modify) | Frontend section: the overview. |

---

### Task 1: Key figures, whole kronor, the year's progress and the default year

**Files:**
- Create: `crates/web/src/overview.rs`
- Modify: `crates/web/src/format.rs`, `crates/web/src/main.rs`
- Test: `crates/web/src/overview.rs`, `crates/web/src/format.rs`

**Interfaces:**
- Produces:
  - `pub fn day_number(date: &str) -> Option<i64>` in `crate::format` (days since 1970-01-01)
  - `pub struct KeyFigures { pub income: i64, pub costs: i64, pub result: i64, pub cash: i64 }` (`Debug, Default, PartialEq, Clone, Copy`)
  - `pub fn key_figures(rows: &[lpb::TrialBalanceRow]) -> KeyFigures`
  - `pub fn whole_kronor(ore: i64) -> String` ("304 330 kr", no-break spaces)
  - `pub struct Progress { pub day: i64, pub days: i64, pub left: i64, pub percent: u8 }` (`Debug, PartialEq, Clone, Copy`)
  - `pub fn progress(start: &str, end: &str, today: &str) -> Option<Progress>`
  - `pub fn default_year(years: &[lpb::FiscalYear], preferred: &str, today: &str) -> String`

- [ ] **Step 1: Write the failing tests**

Create `crates/web/src/overview.rs`:

```rust
//! The overview's figures, worked out from what the existing RPCs return.
//! Pure: the page passes in the messages and today's date.

use crate::api::lpb;

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
        assert_eq!(whole_kronor(123_456_789_012_00), "123\u{a0}456\u{a0}789\u{a0}012\u{a0}kr");
    }

    #[test]
    fn progress_counts_days_into_the_year() {
        let p = |today| progress("2026-01-01", "2026-12-31", today).unwrap();
        assert_eq!(p("2026-01-01"), Progress { day: 1, days: 365, left: 364, percent: 0 });
        assert_eq!(p("2026-10-04"), Progress { day: 277, days: 365, left: 88, percent: 75 });
        assert_eq!(p("2026-12-31"), Progress { day: 365, days: 365, left: 0, percent: 100 });
    }

    #[test]
    fn progress_is_clamped_outside_the_year() {
        let p = |today| progress("2026-01-01", "2026-12-31", today).unwrap();
        assert_eq!(p("2025-06-01"), Progress { day: 0, days: 365, left: 365, percent: 0 });
        assert_eq!(p("2027-03-01"), Progress { day: 365, days: 365, left: 0, percent: 100 });
    }

    #[test]
    fn progress_knows_leap_years_and_refuses_nonsense() {
        assert_eq!(progress("2028-01-01", "2028-12-31", "2028-03-01").unwrap().days, 366);
        assert_eq!(progress("2028-01-01", "2028-12-31", "2028-03-01").unwrap().day, 61);
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
        assert_eq!(default_year(&years, "2025-01-01", "2026-10-04"), "2025-01-01");
        assert_eq!(default_year(&years, "", "2026-10-04"), "2026-01-01");
        assert_eq!(default_year(&years, "1999-01-01", "2026-10-04"), "2026-01-01");
        assert_eq!(default_year(&years, "", "2030-01-01"), "2027-01-01");
        assert_eq!(default_year(&[], "", "2026-10-04"), "");
    }
}
```

In `crates/web/src/format.rs`'s test module add:

```rust
    #[test]
    fn day_numbers_count_from_1970() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("1970-01-02"), Some(1));
        assert_eq!(day_number("2026-10-04"), Some(20_730));
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("nonsense"), None);
        assert_eq!(day_number(""), None);
    }
```

Add `mod overview;` between `mod nav;` and `mod pages;` in `crates/web/src/main.rs`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web overview:: format::`
Expected: does not compile (`key_figures`, `KeyFigures`, `whole_kronor`, `progress`, `Progress`, `default_year`, `day_number` are missing).

- [ ] **Step 3: Implement**

`crates/web/src/format.rs`: split the first half of `plus_days` out, and make `plus_days` use it. Replace `plus_days`'s body up to and including `let z = …;` so the file has:

```rust
/// Days from 1970-01-01 to `date` (`YYYY-MM-DD`), or `None` if it isn't a
/// date. Howard Hinnant's civil-day arithmetic, so no date library goes
/// into the wasm.
pub fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `date` (`YYYY-MM-DD`) plus `days`, or `None` if it isn't a date.
pub fn plus_days(date: &str, days: i64) -> Option<String> {
    let z = day_number(date)? + 719_468 + days;
    let era = z.div_euclid(146_097);
    // …the rest of the existing function, unchanged, from `let doe = z - era * 146_097;`…
}
```

Keep the lines after `let era = z.div_euclid(146_097);` exactly as they are today. The existing `plus_days` tests must still pass.

`crates/web/src/overview.rs`, above the tests:

```rust
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
        .or_else(|| years.iter().find(|y| y.start.as_str() <= today && today <= y.end.as_str()))
        .or(years.first())
        .map(|y| y.start.clone())
        .unwrap_or_default()
}
```

`percent` for day 1 of 365 is `1 * 100 / 365 = 0`, and for day 277 it is `75`: the tests say so.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web` and both clippy commands. The module is unused outside tests for now: put `#![allow(dead_code)]` at the top of `overview.rs` with the comment `// Used by the overview page from the next commits on.`; Task 6 removes it.
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src
git commit -m "Work out the overview's key figures and the year's progress"
```

---

### Task 2: Income and costs per month, and the chart's scale

**Files:**
- Modify: `crates/web/src/overview.rs`
- Test: `crates/web/src/overview.rs`

**Interfaces:**
- Consumes: the account ranges of Task 1.
- Produces:
  - `pub struct Month { pub month: String /* "YYYY-MM" */, pub income: i64, pub costs: i64 }` (`Debug, PartialEq, Clone`)
  - `pub fn by_month(start: &str, end: &str, vouchers: &[lpb::Voucher]) -> Vec<Month>`
  - `pub fn scale(months: &[Month]) -> i64` (the axis maximum in öre; 0 when there is nothing to draw)
  - `pub fn bar_height(value: i64, scale: i64, full: u32) -> u32` (pixels)
  - `pub fn month_label(month: &str) -> &'static str` ("2026-10" → "okt")

- [ ] **Step 1: Write the failing tests**

Add to the test module:

```rust
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
        Month { month: month.into(), income, costs }
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
                voucher("2026-01-15", &[(1930, 1_250_00, 0), (3001, 0, 1_000_00), (2611, 0, 250_00)]),
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
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web overview::`
Expected: does not compile (`Month`, `by_month`, `scale`, `bar_height`, `month_label` are missing).

- [ ] **Step 3: Implement**

```rust
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
        let Some(month) = voucher.date.get(..7).and_then(|key| months.iter_mut().find(|m| m.month == key)) else {
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

/// The chart's axis maximum: the smallest of 1, 2 or 5 times a power of
/// ten that holds the largest value. 0 when there is nothing to draw.
pub fn scale(months: &[Month]) -> i64 {
    let largest = months.iter().flat_map(|m| [m.income, m.costs]).max().unwrap_or(0);
    if largest <= 0 {
        return 0;
    }
    let mut power: i64 = 1;
    loop {
        for step in [1, 2, 5] {
            // Past i64 the axis is the value itself; no real ledger gets here.
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
```

`241_000_00` → the candidates run 1, 2, 5, 10, … 10 000 000, 20 000 000, 50 000 000 öre; `25_000_000` (250 000 kr) is not among them. The spec's example ("241 000 → 250 000") needs a 2.5 step: use the steps `[10, 20, 25, 50]` over powers of ten instead (so the sequence is 10, 20, 25, 50, 100, 200, 250, 500, …), and start with the special case `if largest < 10 { return largest; }`. With that, `of(1, 0) == 1`, `of(100_000_01, 0) == 200_000_00`, `of(0, 450_000_00) == 500_000_00` and `of(900_000_000_000_00, 0) == 1_000_000_000_000_00` all hold. Write the loop that way:

```rust
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
```

Update the doc comment to "1, 2, 2.5 or 5 times a power of ten".

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web overview::` and both clippy commands.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/overview.rs
git commit -m "Sum income and costs per month and scale the chart"
```

---

### Task 3: The to-do rules

**Files:**
- Modify: `crates/web/src/overview.rs`
- Test: `crates/web/src/overview.rs`

**Interfaces:**
- Consumes: `crate::format::{amount, plus_days}`, `crate::api::{ipb, ppb}`.
- Produces:
  - `pub struct Todo { pub urgent: bool, pub title: String, pub detail: String, pub action: &'static str, pub href: String }` (`Debug, PartialEq, Clone`)
  - `pub struct TodoInput<'a> { pub supplier_invoices: &'a [ipb::SupplierInvoice], pub customer_invoices: &'a [ipb::CustomerInvoice], pub payroll_runs: &'a [ppb::PayrollRun], pub agi_months: &'a [ppb::AgiMonthSummary] }`
  - `pub fn todo(input: &TodoInput, today: &str) -> Vec<Todo>`
  - `pub fn unpaid_supplier_invoices(invoices: &[ipb::SupplierInvoice]) -> Vec<&ipb::SupplierInvoice>` (earliest due first)

- [ ] **Step 1: Write the failing tests**

Add to the test module (`use crate::api::{ipb, ppb};` at the top of it):

```rust
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
        todo(&input, TODAY)
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
        let list = todos(TodoInput { supplier_invoices: &invoices, ..empty() });
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
        let list = todos(TodoInput { supplier_invoices: &invoices, ..empty() });
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "1 leverantörsfaktura har förfallit");
        assert_eq!(list[1].title, "2 leverantörsfakturor förfaller inom 30 dagar");
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
        let list = todos(TodoInput { customer_invoices: &invoices, ..empty() });
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
        let list = todos(TodoInput { payroll_runs: &runs, ..empty() });
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
        let runs = [run("r1", "Lön oktober 2026", "2026-10-04", Finalized), run("r2", "", "2026-10-23", Open)];
        let list = todos(TodoInput { payroll_runs: &runs, ..empty() });
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
        let list = todos(TodoInput { agi_months: &months, ..empty() });
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
        let list = todos(TodoInput { agi_months: &months, ..empty() });
        assert_eq!(list[0].title, "Arbetsgivardeklarationen för 5 månader är inte inlämnad");
        assert_eq!(list[0].detail, "2026-05, 2026-06, 2026-07 och 2 till");
        let one = [agi("202609", NotSubmitted)];
        assert_eq!(
            todos(TodoInput { agi_months: &one, ..empty() })[0].title,
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
            ["/supplier-invoices", "/supplier-invoices", "/customer-invoices", "/payroll-runs/r2", "/payroll-runs/r1", "/agi"]
        );
    }

    #[test]
    fn unpaid_supplier_invoices_come_earliest_due_first() {
        let invoices = [
            supplier("Late", "2026-10-28", 1_00, "unpaid"),
            supplier("Paid", "2026-09-01", 1_00, "paid"),
            supplier("Early", "2026-09-30", 1_00, "unpaid"),
        ];
        let names: Vec<_> = unpaid_supplier_invoices(&invoices).iter().map(|i| i.supplier_name.as_str()).collect();
        assert_eq!(names, ["Early", "Late"]);
    }
```

The AGI period is `ÅÅÅÅMM` ("202609"), as `proto/doris/payroll/v1/payroll.proto` says.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p doris-web overview::`
Expected: does not compile (`Todo`, `TodoInput`, `todo`, `unpaid_supplier_invoices` are missing).

- [ ] **Step 3: Implement**

```rust
use crate::api::{ipb, ppb};
use crate::format::{amount, plus_days};

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
    let verb = if late.len() == 1 { "har förfallit" } else { "har förfallit" };
    Some(Todo {
        urgent: true,
        title: format!("{} {verb}", counted(late.len(), one, many)),
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
pub fn todo(input: &TodoInput, today: &str) -> Vec<Todo> {
    let mut list = Vec::new();

    let suppliers: Vec<Due> = input
        .supplier_invoices
        .iter()
        .filter_map(|i| unpaid(&i.status, &i.supplier_name, &i.due_date, i.total))
        .collect();
    list.extend(overdue(&suppliers, today, "leverantörsfaktura", "leverantörsfakturor", "/supplier-invoices"));
    if let Some(last) = plus_days(today, 30) {
        let soon: Vec<&Due> = suppliers.iter().filter(|d| today <= d.date && d.date <= last.as_str()).collect();
        if let Some(next) = soon.iter().min_by_key(|d| d.date) {
            list.push(Todo {
                urgent: false,
                title: format!(
                    "{} {} inom 30 dagar",
                    counted(soon.len(), "leverantörsfaktura", "leverantörsfakturor"),
                    if soon.len() == 1 { "förfaller" } else { "förfaller" }
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
    list.extend(overdue(&customers, today, "kundfaktura", "kundfakturor", "/customer-invoices"));

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
            title: format!("{} kan bokföras", counted(to_book.len(), "lönekörning", "lönekörningar")),
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
                if open.len() == 1 { "färdigställd" } else { "färdigställda" }
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
        .filter(|m| matches!(m.status(), ppb::AgiStatus::NotSubmitted | ppb::AgiStatus::Changed))
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
```

The two `if … { "har förfallit" } else { "har förfallit" }` / `"förfaller"` conditionals above return the same word in both branches (Swedish does not inflect these verbs for number): delete the conditionals and write the word once. The payroll-run rule is the one that inflects ("färdigställd"/"färdigställda").

`amount` gives "13 995,00" with a no-break space; the tests expect that.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web overview::` and both clippy commands.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/overview.rs
git commit -m "Decide what the overview lists under Att göra"
```

---

### Task 4: The page: header, key figures, the year card, and the states

**Files:**
- Rewrite: `crates/web/src/pages/home.rs`
- Create: `e2e/tests/overview.spec.ts`
- Modify: `e2e/tests/fixtures.ts`, `e2e/tests/auth.spec.ts`, `e2e/tests/companies.spec.ts`, `e2e/tests/design.spec.ts`

**Interfaces:**
- Consumes: Task 1 (`key_figures`, `whole_kronor`, `progress`, `default_year`); `PageHeader`, `Card`, `Badge`, `BadgeVariant`, `LinkButton`, `Variant`, `IconName`, `Panel` from `crate::ui`; `FiscalYearSelect`, `keep_year_in_url` from `crate::fiscal_year`; `legal_form_label`, `accounting_method_label`, `today` from `crate::format`.
- Produces (inside `home.rs`, used by Tasks 5 and 6):
  - `type Loaded<T> = RwSignal<Option<Result<T, String>>>;` — `None` while loading, `Err` with the Swedish message.
  - `fn load<T>(target: Loaded<T>, …)`-style helpers as written below.
  - `#[component] fn OverviewCard(title: &'static str, #[prop(optional)] class: &'static str, children: Children)` — a `Panel` with an `<h2>`.
  - `fn pending<T: Clone + Send + Sync + 'static>(data: Loaded<T>, view: impl Fn(T) -> AnyView + 'static) -> impl Fn() -> AnyView` — "Laddar…", the error, or the content.
  - e2e: `expectSignedIn(page: Page, name: string)` in `fixtures.ts`.

- [ ] **Step 1: Write the failing browser tests**

`e2e/tests/fixtures.ts`: add, and use it in `register` instead of the "Inloggad som" line:

```ts
/** The account menu shows who is signed in. */
export async function expectSignedIn(page: Page, name: string) {
  await expect(page.getByRole("banner").locator("summary").filter({ hasText: name })).toBeVisible();
}
```

In `register`: replace `await expect(page.getByText(`Inloggad som ${opts.name}`)).toBeVisible();` with `await expectSignedIn(page, opts.name);`.

`e2e/tests/auth.spec.ts`: lines 23, 25 and 90 (`getByText("Inloggad som Anna")`) become `await expectSignedIn(page, "Anna");`. Line 10 (`getByText("administratör")`) is deleted: line 11 already checks what an administrator has (the Inbjudningar link). Import `expectSignedIn`.

`e2e/tests/companies.spec.ts`, the end of "the active company is chosen in the header and remembered" (the block from `await page.goto(app);` on) becomes:

```ts
  await page.goto(app);
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Exempel AB" })).toBeVisible();
  await expect(main.getByText("556016-0680")).toBeVisible();
```

Earlier in the same test, "Du har inga företag än." stays as it is.

`e2e/tests/design.spec.ts`: in "the account views follow the design, with narrow left-aligned forms" the first `expectDesign(page, "Översikt")` stays (no company yet). Nothing else changes there.

Create `e2e/tests/overview.spec.ts`:

```ts
import type { Page } from "@playwright/test";
import { addCompany, addSupplier, expect, register, test } from "./fixtures";

const year = new Date().getFullYear();
const figure = (page: Page, name: string) =>
  page.getByRole("main").locator("section").filter({ has: page.getByRole("heading", { name, exact: true }) });

async function book(page: Page, app: string, date: string, kronor: string) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Försäljning");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("a new company's overview is empty but complete", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(app);
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Exempel AB" })).toBeVisible();
  await expect(main.getByText("556016-0680 · Aktiebolag · Faktureringsmetoden")).toBeVisible();
  for (const name of ["Resultat hittills i år", "Intäkter", "Kostnader", "Kassa och bank"]) {
    await expect(figure(page, name)).toContainText(/^.*0\skr/);
  }
  const fiscalYear = figure(page, "Räkenskapsåret");
  await expect(fiscalYear.getByText("Öppet")).toBeVisible();
  await expect(fiscalYear.getByRole("progressbar")).toHaveAttribute("aria-valuemax", "100");
  await expect(fiscalYear).toContainText(`${year}-01-01 – ${year}-12-31`);
  await expect(fiscalYear).toContainText(/Dag \d+ av 36[56]/);
});

test("a booked sale shows in the key figures", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(app);
  await expect(figure(page, "Intäkter")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Resultat hittills i år")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Kassa och bank")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Kostnader")).toContainText(/\b0\skr/);
});

test("the overview follows the chosen year", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${year - 1}-01-01`);
  await book(page, app, `${year - 1}-03-01`, "700");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(app);
  // The year that contains today is chosen, not the newest or the oldest.
  await expect(page.getByLabel("Räkenskapsår")).toHaveValue(`${year}-01-01`);
  await expect(figure(page, "Intäkter")).toContainText(/1\s250\skr/);
  await page.getByLabel("Räkenskapsår").selectOption(`${year - 1}-01-01`);
  await expect(figure(page, "Intäkter")).toContainText(/700\skr/);
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Dag 365 av 365|Dag 366 av 366/);
  // The choice survives a reload.
  await page.reload();
  await expect(page.getByLabel("Räkenskapsår")).toHaveValue(`${year - 1}-01-01`);
});

test("a failed call only takes its own cards down", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.route("**/GetTrialBalance", (route) => route.abort());
  await page.goto(app);
  await expect(figure(page, "Intäkter").getByRole("alert")).toBeVisible();
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Dag \d+ av/);
});

test("without a company the start page says so", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Översikt" })).toBeVisible();
  await expect(main.getByText("Du har inga företag än.")).toBeVisible();
  await expect(main.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
});
```

(`addSupplier` is imported for Task 5.)

- [ ] **Step 2: Run them and see them fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test overview.spec.ts`
Expected: the first four FAIL (no `h1` "Exempel AB", no "Intäkter" card); "without a company" passes already.

- [ ] **Step 3: Implement the page**

Replace `crates/web/src/pages/home.rs` with:

```rust
//! The overview: the active company's key figures, what needs doing, and
//! the chosen fiscal year at a glance. Every number comes from
//! `crate::overview`; this file loads and draws.

use crate::active_company::Companies;
use crate::api::{company_api, cpb, ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url};
use crate::format::{accounting_method_label, legal_form_label, today};
use crate::overview::{default_year, key_figures, progress, whole_kronor};
use crate::ui::{Badge, BadgeVariant, Card, IconName, LinkButton, PageHeader, Panel, Variant};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

/// `None` while loading; `Err` holds the Swedish message.
type Loaded<T> = RwSignal<Option<Result<T, String>>>;

/// A card on the overview: a `Panel` with its heading.
#[component]
fn OverviewCard(
    title: &'static str,
    #[prop(optional)] class: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <Panel class=class>
            <div class="grid gap-3">
                <h2 class="text-sm font-medium">{title}</h2>
                {children()}
            </div>
        </Panel>
    }
}

/// "Laddar…", the error, or `view` of what arrived.
fn pending<T: Clone + Send + Sync + 'static>(
    data: Loaded<T>,
    view: impl Fn(T) -> AnyView + 'static,
) -> impl Fn() -> AnyView {
    move || match data.get() {
        None => view! { <p class="text-muted-foreground">"Laddar…"</p> }.into_any(),
        Some(Err(message)) => view! { <p role="alert" class="text-destructive">{message}</p> }.into_any(),
        Some(Ok(value)) => view(value),
    }
}

#[component]
pub fn Home() -> impl IntoView {
    let companies = expect_context::<Companies>();
    view! {
        <Show
            when=move || !companies.active.get().is_empty()
            fallback=move || view! { <NoCompany /> }
        >
            <Overview />
        </Show>
    }
}

/// The start page before the first company exists.
#[component]
fn NoCompany() -> impl IntoView {
    let companies = expect_context::<Companies>();
    view! {
        <div class="grid gap-6">
            <PageHeader title="Översikt" />
            <Show when=move || companies.loaded.get()>
                <Card title="Aktivt företag" narrow=true>
                    <div class="grid gap-2">
                        <p class="text-muted-foreground">"Du har inga företag än."</p>
                        <A href="/companies/new" attr:class="font-medium underline-offset-4 hover:underline">
                            "Lägg till företag"
                        </A>
                    </div>
                </Card>
            </Show>
        </div>
    }
}

#[component]
fn Overview() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let preferred = use_query_map().read_untracked().get("fy").unwrap_or_default();

    let company: Loaded<cpb::Company> = RwSignal::new(None);
    let years: Loaded<Vec<lpb::FiscalYear>> = RwSignal::new(None);
    let year = RwSignal::new(String::new());
    let balance: Loaded<Vec<lpb::TrialBalanceRow>> = RwSignal::new(None);
    let vouchers: Loaded<Vec<lpb::Voucher>> = RwSignal::new(None);
    keep_year_in_url("/".into(), year);

    // Per company: who it is and which years it has.
    Effect::new(move |_| {
        let company_id = companies.active.get();
        company.set(None);
        years.set(None);
        year.set(String::new());
        if company_id.is_empty() {
            return;
        }
        let preferred = preferred.clone();
        spawn_local(async move {
            let current = move || company_id_is_active(companies, &company_id);
            let (id_a, id_b) = (companies.active.get_untracked(), companies.active.get_untracked());
            let found = company_api().get_company(cpb::GetCompanyRequest { company_id: id_a }).await;
            let listed = ledger_api().list_fiscal_years(lpb::ListFiscalYearsRequest { company_id: id_b }).await;
            if !current() {
                return;
            }
            company.set(Some(found.map(|r| r.into_inner()).map_err(|s| describe(&s))));
            match listed {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    year.set(default_year(&list, &preferred, &today()));
                    years.set(Some(Ok(list)));
                }
                Err(status) => years.set(Some(Err(describe(&status)))),
            }
        });
    });

    // Per year: the trial balance and the vouchers.
    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        balance.set(None);
        vouchers.set(None);
        if start.is_empty() || company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            // Stale once the company or the year has changed.
            let current = {
                let (company_id, start) = (company_id.clone(), start.clone());
                move || company_id == companies.active.get_untracked() && start == year.get_untracked()
            };
            let rows = ledger_api()
                .get_trial_balance(lpb::GetTrialBalanceRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            if current() {
                balance.set(Some(rows.map(|r| r.into_inner().rows).map_err(|s| describe(&s))));
            }
            let listed = ledger_api()
                .list_vouchers(lpb::ListVouchersRequest {
                    company_id,
                    fiscal_year_start: start,
                })
                .await;
            if current() {
                vouchers.set(Some(listed.map(|r| r.into_inner().vouchers).map_err(|s| describe(&s))));
            }
        });
    });

    let title = Signal::derive(move || match (company.get(), companies.active_company()) {
        (Some(Ok(c)), _) => c.name,
        (_, Some(summary)) => summary.name,
        _ => String::new(),
    });
    let description = Signal::derive(move || match company.get() {
        Some(Ok(c)) => format!(
            "{} · {} · {}",
            c.org_nr,
            legal_form_label(c.legal_form()),
            accounting_method_label(c.accounting_method())
        ),
        _ => String::new(),
    });
    let year_list = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    Effect::new(move |_| year_list.set(years.get().and_then(Result::ok).unwrap_or_default()));
    let chosen = move || year_list.with(|ys| ys.iter().find(|y| y.start == year.get()).cloned());

    view! {
        <div class="grid gap-6">
            <PageHeader title=title description=description>
                <FiscalYearSelect years=year_list year=year />
                <LinkButton href="/payroll-runs/new" variant=Variant::Outline>"Ny lönekörning"</LinkButton>
                <LinkButton href="/supplier-invoices/new" variant=Variant::Outline>"Ny leverantörsfaktura"</LinkButton>
                <LinkButton href="/customer-invoices/new" variant=Variant::Outline>"Ny kundfaktura"</LinkButton>
                <LinkButton href="/vouchers/new" icon=IconName::Plus>"Ny verifikation"</LinkButton>
            </PageHeader>
            <div class="grid grid-cols-[repeat(auto-fit,minmax(min(220px,100%),1fr))] gap-4">
                <KeyFigure title="Resultat hittills i år" note="Efter finansiella poster" balance=balance pick=|f| f.result />
                <KeyFigure title="Intäkter" note="Konto 3000–3999" balance=balance pick=|f| f.income />
                <KeyFigure title="Kostnader" note="Konto 4000–8989" balance=balance pick=|f| f.costs />
                <KeyFigure title="Kassa och bank" note="Konto 1900–1999" balance=balance pick=|f| f.cash />
            </div>
            <div class="flex flex-wrap gap-4">
                // Task 5 puts "Att göra" here, before the year card.
                <FiscalYearCard years=years chosen=Signal::derive(chosen) vouchers=vouchers />
            </div>
            // Task 6: the chart and the unpaid supplier invoices. Task 5: the latest vouchers.
        </div>
    }
}

fn company_id_is_active(companies: Companies, company_id: &str) -> bool {
    companies.active.get_untracked() == company_id
}

/// One headline number from the trial balance.
#[component]
fn KeyFigure(
    title: &'static str,
    note: &'static str,
    balance: Loaded<Vec<lpb::TrialBalanceRow>>,
    pick: fn(crate::overview::KeyFigures) -> i64,
) -> impl IntoView {
    view! {
        <Panel>
            <div class="grid gap-1">
                <h2 class="text-xs/relaxed font-normal text-muted-foreground">{title}</h2>
                {pending(balance, move |rows| {
                    view! {
                        <p class="text-2xl/8 font-semibold tracking-tight tabular-nums">
                            {whole_kronor(pick(key_figures(&rows)))}
                        </p>
                    }
                        .into_any()
                })}
                <p class="text-muted-foreground">{note}</p>
            </div>
        </Panel>
    }
}

/// The chosen year: open or closed, how far in, and what is booked.
#[component]
fn FiscalYearCard(
    years: Loaded<Vec<lpb::FiscalYear>>,
    #[prop(into)] chosen: Signal<Option<lpb::FiscalYear>>,
    vouchers: Loaded<Vec<lpb::Voucher>>,
) -> impl IntoView {
    let row = "flex justify-between gap-3 border-t py-2";
    view! {
        <OverviewCard title="Räkenskapsåret" class="min-w-0 flex-[1_1_280px]">
            {pending(years, move |list| {
                let Some(fiscal_year) = chosen.get() else {
                    return view! { <p class="text-muted-foreground">"Inget räkenskapsår."</p> }.into_any();
                };
                let progress = progress(&fiscal_year.start, &fiscal_year.end, &today());
                // Years are newest first: the one before is listed next.
                let previous = list
                    .iter()
                    .position(|y| y.start == fiscal_year.start)
                    .and_then(|i| list.get(i + 1))
                    .cloned();
                view! {
                    <div>
                        {if fiscal_year.closed {
                            view! { <Badge variant=BadgeVariant::Outline>"Stängt"</Badge> }.into_any()
                        } else {
                            view! { <Badge>"Öppet"</Badge> }.into_any()
                        }}
                    </div>
                    {progress.map(|p| view! {
                        <div class="grid gap-2">
                            <div
                                role="progressbar"
                                aria-label="Andel av räkenskapsåret som har gått"
                                aria-valuemin="0"
                                aria-valuemax="100"
                                aria-valuenow=p.percent.to_string()
                                class="h-1.5 overflow-hidden rounded-full bg-muted"
                            >
                                <div class="h-full bg-chart-1" style=format!("width: {}%", p.percent)></div>
                            </div>
                            <div class="flex justify-between gap-2 text-muted-foreground">
                                <span>{format!("Dag {} av {}", p.day, p.days)}</span>
                                <span>{format!("{} dagar kvar", p.left)}</span>
                            </div>
                        </div>
                    })}
                    <dl>
                        <div class=row>
                            <dt class="text-muted-foreground">"Period"</dt>
                            <dd class="tabular-nums">{format!("{} – {}", fiscal_year.start, fiscal_year.end)}</dd>
                        </div>
                        {move || match vouchers.get() {
                            Some(Ok(list)) => {
                                let latest = list.iter().map(|v| v.date.clone()).max();
                                view! {
                                    <div class=row>
                                        <dt class="text-muted-foreground">"Verifikationer"</dt>
                                        <dd class="tabular-nums">{list.len()}</dd>
                                    </div>
                                    <div class=row>
                                        <dt class="text-muted-foreground">"Senast bokfört"</dt>
                                        <dd class="tabular-nums">{latest.unwrap_or_else(|| "–".into())}</dd>
                                    </div>
                                }
                                    .into_any()
                            }
                            _ => ().into_any(),
                        }}
                        {previous.map(|p| view! {
                            <div class=row>
                                <dt class="text-muted-foreground">{format!("Föregående år, {}", &p.start[..4.min(p.start.len())])}</dt>
                                <dd>{if p.closed { "Stängt" } else { "Öppet" }}</dd>
                            </div>
                        })}
                    </dl>
                    <A href="/fiscal-years" attr:class="font-medium underline-offset-4 hover:underline">"Visa räkenskapsår"</A>
                }
                    .into_any()
            })}
        </OverviewCard>
    }
}
```

Notes for the implementer:
- The first `spawn_local` above is written clumsily (`id_a`, `id_b`, `company_id_is_active`). Write it plainly instead: clone `company_id` for each request, and after both answers `if company_id != companies.active.get_untracked() { return; }`, exactly as `use_fiscal_years` in `crates/web/src/fiscal_year.rs` does. Delete `company_id_is_active`.
- `bg-chart-1` exists only after Task 6 adds the token. Until then use `bg-primary` here, and Task 6 switches it.
- "1 dagar kvar" is wrong Swedish: write `if p.left == 1 { "1 dag kvar".to_owned() } else { format!("{} dagar kvar", p.left) }`.
- `FiscalYearSelect` takes `RwSignal<Vec<lpb::FiscalYear>>`; `year_list` mirrors `years` for it.
- If `pending`'s closure cannot be called in a `view!` position because of `Fn` bounds with `#[component]` generics, turn it into a small component `Pending` with `children: ChildrenFn`-style render prop, keeping the three states and the `role="alert"`.
- `KeyFigure`'s `<h2>` is the accessible name the e2e `figure()` helper looks for; keep `exact` names.

- [ ] **Step 4: Run the new tests**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test overview.spec.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Run everything**

Run: `cargo test --workspace`, both clippy commands, and the whole Playwright suite.
Expected: PASS. A spec that still waits for "Inloggad som" or the "Aktivt företag" card for a company with data was missed in Step 1: fix it the same way.

- [ ] **Step 6: Commit**

```bash
git add crates/web/src e2e/tests
git commit -m "Show key figures and the fiscal year on the overview"
```

---

### Task 5: Att göra, unpaid supplier invoices and the latest vouchers

**Files:**
- Modify: `crates/web/src/pages/home.rs`
- Modify: `e2e/tests/overview.spec.ts`

**Interfaces:**
- Consumes: Task 3 (`todo`, `TodoInput`, `Todo`, `unpaid_supplier_invoices`); `Loaded`, `OverviewCard`, `pending` from Task 4; `invoicing_api`, `payroll_api`, `ipb`, `ppb`; `Icon`, `IconName`, `Table` constants from `crate::ui`; `amount` from `crate::format`.

- [ ] **Step 1: Write the failing browser tests**

Append to `e2e/tests/overview.spec.ts`:

```ts
const iso = (daysFromToday: number) => {
  const d = new Date();
  d.setDate(d.getDate() + daysFromToday);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

test("nothing to do says so", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(app);
  await expect(figure(page, "Att göra")).toContainText("Inget att göra just nu.");
  await expect(figure(page, "Senaste verifikationer")).toContainText("Inga verifikationer än.");
  await expect(figure(page, "Obetalda leverantörsfakturor")).toContainText("Inga obetalda leverantörsfakturor.");
});

test("an overdue supplier invoice is on the to-do list and leads to the invoices", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Kontorshuset AB");
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Kontorshuset AB" });
  await page.getByLabel("Fakturanummer").fill("20413");
  await page.getByLabel("Fakturadatum").fill(iso(-40));
  await page.getByLabel("Förfallodatum").fill(iso(-10));
  await page.getByLabel("Konto, rad 1").fill("5010");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("10000");
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Leverantörsfakturor" })).toBeVisible();

  await page.goto(app);
  const todo = figure(page, "Att göra");
  await expect(todo).toContainText("1 leverantörsfaktura har förfallit");
  await expect(todo).toContainText(/12\s500,00 kr · äldst Kontorshuset AB/);
  const unpaid = figure(page, "Obetalda leverantörsfakturor");
  await expect(unpaid).toContainText("Kontorshuset AB");
  await expect(unpaid.getByText("Förfallen", { exact: true })).toBeVisible();
  // Registered under faktureringsmetoden, so it is a voucher too.
  await expect(figure(page, "Senaste verifikationer").getByRole("row")).toHaveCount(2);
  await todo.getByRole("link", { name: "Visa fakturorna" }).click();
  await expect(page).toHaveURL(/\/supplier-invoices$/);
});
```

Before running, open `e2e/tests/supplier_invoices.spec.ts` and copy its way of registering an invoice (labels, the invoice date's fiscal year, how it waits for success) into the test above if any label differs; that spec is the authority on the form. If the invoice date 40 days ago falls in a year the company does not have (early January), use `${year}-01-01` as the company's start and dates inside the current year instead of `iso(-40)`/`iso(-10)`: `invoice date = max(iso(-40), year start)`, `due date = iso(-1)`, and assert "förföll" generically.

- [ ] **Step 2: Run them and see them fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test overview.spec.ts -g "to do|to-do"`
Expected: FAIL (no "Att göra" card).

- [ ] **Step 3: Implement**

In `Overview` add the four per-company signals and load them in the per-company effect's `spawn_local`, after the years (each guarded by the same stale check; reset to `None` at the top of the effect):

```rust
    let supplier_invoices: Loaded<Vec<ipb::SupplierInvoice>> = RwSignal::new(None);
    let customer_invoices: Loaded<Vec<ipb::CustomerInvoice>> = RwSignal::new(None);
    let payroll_runs: Loaded<Vec<ppb::PayrollRun>> = RwSignal::new(None);
    let agi_months: Loaded<Vec<ppb::AgiMonthSummary>> = RwSignal::new(None);
```

```rust
            let suppliers = invoicing_api()
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest { company_id: company_id.clone() })
                .await;
            let customers = invoicing_api()
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest { company_id: company_id.clone() })
                .await;
            let runs = payroll_api()
                .list_payroll_runs(ppb::ListPayrollRunsRequest { company_id: company_id.clone() })
                .await;
            let months = payroll_api()
                .list_agi_months(ppb::ListAgiMonthsRequest { company_id: company_id.clone() })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            supplier_invoices.set(Some(suppliers.map(|r| r.into_inner().invoices).map_err(|s| describe(&s))));
            customer_invoices.set(Some(customers.map(|r| r.into_inner().invoices).map_err(|s| describe(&s))));
            payroll_runs.set(Some(runs.map(|r| r.into_inner().payroll_runs).map_err(|s| describe(&s))));
            agi_months.set(Some(months.map(|r| r.into_inner().months).map_err(|s| describe(&s))));
```

The to-do list needs all four. Combine them into one `Loaded`-shaped memo: loading while any is `None`, the first error if any failed.

```rust
    let todos = Memo::new(move |_| {
        let (s, c, r, m) = (supplier_invoices.get()?, customer_invoices.get()?, payroll_runs.get()?, agi_months.get()?);
        Some((|| {
            let (s, c, r, m) = (s?, c?, r?, m?);
            Ok::<_, String>(todo(
                &TodoInput { supplier_invoices: &s, customer_invoices: &c, payroll_runs: &r, agi_months: &m },
                &today(),
            ))
        })())
    });
```

`pending` takes an `RwSignal`; give it a second form taking `Signal<Option<Result<T, String>>>` (make `pending` generic over `impl Fn() -> Option<Result<T, String>>`, and pass `move || data.get()` from the existing callers), so the memo fits.

The cards:

```rust
/// What needs doing, most pressing first.
#[component]
fn TodoCard(#[prop(into)] todos: Signal<Option<Result<Vec<Todo>, String>>>) -> impl IntoView {
    view! {
        <Panel class="min-w-0 flex-[2_1_480px] px-0 pb-1">
            <div class="grid gap-3">
                <div class="flex items-center gap-2 px-4">
                    <h2 class="text-sm font-medium">"Att göra"</h2>
                    {move || todos.get().and_then(Result::ok).filter(|l| !l.is_empty()).map(|l| view! { <Badge>{l.len()}</Badge> })}
                </div>
                {pending(move || todos.get(), |list: Vec<Todo>| {
                    if list.is_empty() {
                        return view! { <p class="px-4 pb-3 text-muted-foreground">"Inget att göra just nu."</p> }.into_any();
                    }
                    view! {
                        <ul>
                            {list
                                .into_iter()
                                .map(|item| {
                                    let (round, icon) = if item.urgent {
                                        ("bg-destructive/10 text-destructive dark:bg-destructive/20", IconName::CircleAlert)
                                    } else {
                                        ("bg-muted", IconName::Clock)
                                    };
                                    view! {
                                        <li class="flex flex-wrap items-center gap-3 border-t px-4 py-3">
                                            <span class=format!("flex size-7 shrink-0 items-center justify-center rounded-full {round}")>
                                                <Icon name=icon />
                                            </span>
                                            <div class="min-w-0 flex-[1_1_240px]">
                                                <p class="font-medium">{item.title}</p>
                                                <p class="text-muted-foreground">{item.detail}</p>
                                            </div>
                                            <LinkButton href=item.href variant=Variant::Outline>{item.action}</LinkButton>
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                })}
            </div>
        </Panel>
    }
}
```

`Panel`'s own padding is `p-4`; the list rows need full-width rules, so this card overrides the horizontal padding (`px-0`) and pads its heading and rows itself. Check in the browser that the later class wins; if Tailwind's order keeps `p-4`, give `Panel` a `#[prop(optional)] flush: bool` that leaves the padding out.

Add two icons to `IconName` in `crates/web/src/ui.rs` (and to `ALL`, bumping its length), shapes from lucide-static 1.52.0 (`curl -fsSL https://unpkg.com/lucide-static@1.52.0/icons/circle-alert.svg`, and `clock.svg`), with closing tags written out as for the others:

```rust
            IconName::CircleAlert => r#"<circle cx="12" cy="12" r="10"></circle><line x1="12" x2="12" y1="8" y2="12"></line><line x1="12" x2="12.01" y1="16" y2="16"></line>"#,
            IconName::Clock => r#"<path d="M12 6v6l4 2"></path><circle cx="12" cy="12" r="10"></circle>"#,
```

Use what the fetched files contain if it differs from the two lines above.

```rust
/// The unpaid supplier invoices: how many, how much, and the next four.
#[component]
fn UnpaidCard(invoices: Loaded<Vec<ipb::SupplierInvoice>>) -> impl IntoView {
    view! {
        <OverviewCard title="Obetalda leverantörsfakturor" class="min-w-0 flex-[1_1_280px]">
            {pending(move || invoices.get(), |list: Vec<ipb::SupplierInvoice>| {
                let unpaid = unpaid_supplier_invoices(&list);
                if unpaid.is_empty() {
                    return view! { <p class="text-muted-foreground">"Inga obetalda leverantörsfakturor."</p> }.into_any();
                }
                let today = today();
                let total: i64 = unpaid.iter().map(|i| i.total).sum();
                let summary = format!(
                    "{} {}, {} kr",
                    unpaid.len(),
                    if unpaid.len() == 1 { "faktura" } else { "fakturor" },
                    amount(total)
                );
                view! {
                    <p class="text-muted-foreground">{summary}</p>
                    <ul>
                        {unpaid
                            .into_iter()
                            .take(4)
                            .map(|invoice| {
                                let late = invoice.due_date < today;
                                view! {
                                    <li class="flex items-center justify-between gap-3 border-t py-2">
                                        <div class="min-w-0">
                                            <p class="truncate font-medium">{invoice.supplier_name.clone()}</p>
                                            <p class="flex items-center gap-1.5 text-muted-foreground">
                                                {format!("{} {}", if late { "Förföll" } else { "Förfaller" }, invoice.due_date)}
                                                {late.then(|| view! { <Badge variant=BadgeVariant::Destructive>"Förfallen"</Badge> })}
                                            </p>
                                        </div>
                                        <p class="whitespace-nowrap tabular-nums">{amount(invoice.total)}</p>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                    <A href="/supplier-invoices" attr:class="font-medium underline-offset-4 hover:underline">"Alla leverantörsfakturor"</A>
                }
                    .into_any()
            })}
        </OverviewCard>
    }
}

/// The five vouchers with the highest numbers.
#[component]
fn LatestVouchers(vouchers: Loaded<Vec<lpb::Voucher>>) -> impl IntoView {
    view! {
        <OverviewCard title="Senaste verifikationer">
            {pending(move || vouchers.get(), |mut list: Vec<lpb::Voucher>| {
                if list.is_empty() {
                    return view! { <p class="text-muted-foreground">"Inga verifikationer än."</p> }.into_any();
                }
                list.sort_by(|a, b| b.number.cmp(&a.number));
                view! {
                    <Table>
                        <thead class=TABLE_HEAD>
                            <tr class=TABLE_ROW>
                                <th class=TABLE_HEADER_CELL>"Nr"</th>
                                <th class=TABLE_HEADER_CELL>"Datum"</th>
                                <th class=TABLE_HEADER_CELL>"Text"</th>
                                <th class=format!("{TABLE_HEADER_CELL} text-right")>"Belopp"</th>
                            </tr>
                        </thead>
                        <tbody class=TABLE_BODY>
                            {list
                                .into_iter()
                                .take(5)
                                .map(|v| {
                                    let total: i64 = v.lines.iter().map(|l| l.debit).sum();
                                    view! {
                                        <tr class=TABLE_ROW>
                                            <td class=format!("{TABLE_CELL} tabular-nums")>{v.number}</td>
                                            <td class=format!("{TABLE_CELL} tabular-nums")>{v.date}</td>
                                            <td class=TABLE_CELL>{v.text}</td>
                                            <td class=TABLE_AMOUNT_CELL>{amount(total)}</td>
                                        </tr>
                                    }
                                })
                                .collect_view()}
                        </tbody>
                    </Table>
                    <A href="/vouchers" attr:class="font-medium underline-offset-4 hover:underline">"Alla verifikationer"</A>
                }
                    .into_any()
            })}
        </OverviewCard>
    }
}
```

Place them in `Overview`'s view: `<TodoCard todos=todos />` before `<FiscalYearCard … />` in the first row; a second row `<div class="flex flex-wrap gap-4">` holding `<UnpaidCard invoices=supplier_invoices />` (Task 6 puts the chart before it); then `<LatestVouchers vouchers=vouchers />`.

- [ ] **Step 4: Run the tests**

Run: the two new tests, then `cargo test --workspace`, both clippy commands and the whole Playwright suite.
Expected: PASS. `design.spec.ts`'s "every signed-in view" checks `/` at 390px: if a row scrolls sideways, the flex child is missing `min-w-0`.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src e2e/tests
git commit -m "List what needs doing, unpaid supplier invoices and the latest vouchers"
```

---

### Task 6: The chart and its colours

**Files:**
- Modify: `crates/web/src/pages/home.rs`, `crates/web/style/input.css`, `crates/web/src/overview.rs` (drop `#![allow(dead_code)]`)
- Modify: `e2e/tests/overview.spec.ts`

**Interfaces:**
- Consumes: Task 2 (`by_month`, `scale`, `bar_height`, `month_label`, `Month`).

- [ ] **Step 1: Write the failing browser tests**

Append to `e2e/tests/overview.spec.ts`:

```ts
test("the chart draws a bar for a month with income and none for an empty one", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1000");
  await page.goto(app);
  const chart = figure(page, "Intäkter och kostnader per månad");
  await expect(chart.getByRole("img")).toHaveAttribute("aria-label", /Intäkter och kostnader per månad/);
  await expect(chart.getByText("Intäkter", { exact: true })).toBeVisible();
  await expect(chart.getByText("Kostnader", { exact: true })).toBeVisible();
  const heights = await chart.locator("[data-month]").evaluateAll((months) =>
    months.map((m) => [m.getAttribute("data-month"), ...[...m.children].map((bar) => Math.round(bar.getBoundingClientRect().height))]),
  );
  expect(heights).toHaveLength(12);
  expect(heights[0]).toEqual([`${year}-01`, 160, 0]);
  expect(heights[1]).toEqual([`${year}-02`, 0, 0]);
  // The two series are told apart by lightness, in both schemes.
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const [income, costs] = await chart.locator("[data-legend]").evaluateAll((els) => els.map((el) => getComputedStyle(el).backgroundColor));
    expect(income, scheme).not.toBe(costs);
    expect(income, scheme).not.toBe("rgba(0, 0, 0, 0)");
  }
});

test("the overview fits a phone in both colour schemes", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1000");
  await page.setViewportSize({ width: 390, height: 844 });
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(app);
    await expect(figure(page, "Senaste verifikationer").getByRole("row")).toHaveCount(2);
    const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
    expect(wider, scheme).toBe(false);
  }
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test overview.spec.ts -g "chart"`
Expected: FAIL (no "Intäkter och kostnader per månad" card). The phone test may already pass; it guards the chart.

- [ ] **Step 3: Implement**

`crates/web/style/input.css`: in `@theme inline` add

```css
    --color-chart-1: var(--chart-1);
    --color-chart-2: var(--chart-2);
```

in `:root` add `--chart-1: oklch(0.555 0.163 48.998);` and `--chart-2: oklch(0.268 0.007 34.298);`, and in the dark block `--chart-1: oklch(0.769 0.188 70.08);` and `--chart-2: oklch(0.553 0.013 58.071);`. Update the comment at the top of the file: "…icons lucide. `--chart-1/2` are amber and stone from the same scales, for the overview's chart."

`home.rs`:

```rust
/// The chart's plot height in pixels.
const PLOT: u32 = 160;

/// Income and costs per month, as two bars a month.
#[component]
fn MonthChart(
    #[prop(into)] chosen: Signal<Option<lpb::FiscalYear>>,
    vouchers: Loaded<Vec<lpb::Voucher>>,
) -> impl IntoView {
    view! {
        <Panel class="min-w-0 flex-[2_1_480px]">
            <div class="grid gap-3">
                <div class="flex flex-wrap items-start justify-between gap-4">
                    <div>
                        <h2 class="text-sm font-medium">"Intäkter och kostnader per månad"</h2>
                        <p class="text-muted-foreground">"Tusental kronor"</p>
                    </div>
                    <ul class="flex gap-4">
                        <li class="flex items-center gap-1.5"><span data-legend class="size-2 rounded-xs bg-chart-1"></span>"Intäkter"</li>
                        <li class="flex items-center gap-1.5"><span data-legend class="size-2 rounded-xs bg-chart-2"></span>"Kostnader"</li>
                    </ul>
                </div>
                {pending(move || vouchers.get(), move |list: Vec<lpb::Voucher>| {
                    let Some(fiscal_year) = chosen.get() else {
                        return ().into_any();
                    };
                    let months = by_month(&fiscal_year.start, &fiscal_year.end, &list);
                    let top = scale(&months);
                    let this_month = today().get(..7).unwrap_or_default().to_owned();
                    // Gridlines at half and full scale, labelled in thousands of kronor.
                    let label = |ore: i64| (ore / 100_000).to_string();
                    let columns = format!("grid-template-columns: repeat({}, minmax(0, 1fr))", months.len().max(1));
                    view! {
                        <figure
                            role="img"
                            aria-label=format!("Intäkter och kostnader per månad, {} – {}", fiscal_year.start, fiscal_year.end)
                            class="grid gap-1.5"
                        >
                            <div class="flex gap-2">
                                <div class="relative w-8 text-right text-muted-foreground tabular-nums" style=format!("height: {PLOT}px")>
                                    <span class="absolute right-0 bottom-0 translate-y-1/2 leading-none">"0"</span>
                                    {(top > 0).then(|| view! {
                                        <span class="absolute right-0 bottom-1/2 translate-y-1/2 leading-none">{label(top / 2)}</span>
                                        <span class="absolute top-0 right-0 -translate-y-1/2 leading-none">{label(top)}</span>
                                    })}
                                </div>
                                <div class="relative min-w-0 flex-1 border-b" style=format!("height: {PLOT}px")>
                                    <div class="absolute inset-x-0 top-0 border-t"></div>
                                    <div class="absolute inset-x-0 top-1/2 border-t"></div>
                                    <div class="absolute inset-0 grid items-end" style=columns.clone()>
                                        {months
                                            .iter()
                                            .map(|m| view! {
                                                <div data-month=m.month.clone() class="flex items-end justify-center gap-0.5">
                                                    <div class="w-3 max-w-[40%] rounded-t-sm bg-chart-1" style=format!("height: {}px", bar_height(m.income, top, PLOT))></div>
                                                    <div class="w-3 max-w-[40%] rounded-t-sm bg-chart-2" style=format!("height: {}px", bar_height(m.costs, top, PLOT))></div>
                                                </div>
                                            })
                                            .collect_view()}
                                    </div>
                                </div>
                            </div>
                            <div class="ml-10 grid text-center text-muted-foreground" style=columns>
                                {months
                                    .iter()
                                    .map(|m| {
                                        let current = m.month == this_month;
                                        view! { <span class=if current { "font-medium text-foreground" } else { "" }>{month_label(&m.month)}</span> }
                                    })
                                    .collect_view()}
                            </div>
                        </figure>
                        <A href="/financial-statements" attr:class="font-medium underline-offset-4 hover:underline">"Visa resultaträkningen"</A>
                    }
                        .into_any()
                })}
            </div>
        </Panel>
    }
}
```

Place `<MonthChart chosen=Signal::derive(chosen) vouchers=vouchers />` before `<UnpaidCard … />` in the second row. Switch the progress bar's fill from `bg-primary` to `bg-chart-1`. Remove `#![allow(dead_code)]` from `overview.rs`; delete anything clippy then reports as unused (and its test).

The gridline labels show thousands of kronor: `top` is öre, so `top / 100_000`. A scale below 2 000 kr would label the half line "0": when `top < 200_000`, label in kronor instead and change the caption "Tusental kronor" to "Kronor" (compute both from `top` in one place).

A year of 18 months narrows the bars through `max-w-[40%]`; check it by eye in Task 7.

- [ ] **Step 4: Run the tests**

Run: the two new tests, then `cargo test --workspace`, both clippy commands and the whole Playwright suite.
Expected: PASS. The first test expects the January income bar to be exactly 160px: 1 000 kr is a round number, so the scale is 1 000 kr and the bar is full height.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Chart income and costs per month on the overview"
```

---

### Task 7: Docs and the check against the canvas

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Update `AGENTS.md`**

In "## Frontend", after the bullet about `src/nav.rs`:

```markdown
- The start page (`src/pages/home.rs`) is the overview for the active
  company and a chosen räkenskapsår (kept in `?fy=`; by default the year
  that contains today). It adds no RPC: it sends `GetCompany`,
  `ListFiscalYears`, `GetTrialBalance`, `ListVouchers`,
  `ListSupplierInvoices`, `ListCustomerInvoices`, `ListPayrollRuns` and
  `ListAgiMonths`, and `src/overview.rs` works everything out in pure
  functions: key figures (income 3000–3999, costs 4000–8989, cash
  1900–1999), income and costs per month, the year's progress and the
  "Att göra" rules. A card whose call failed shows the error; the others
  still show. The monthly sums read every voucher of the year: when that
  gets heavy, add a `GetMonthlyTotals` to the ledger.
```

In "## Style", append: "`--chart-1` (amber) and `--chart-2` (stone) colour the overview's chart and progress bar."

- [ ] **Step 2: Run everything and measure**

Run: `cargo test --workspace`, both clippy commands, the whole Playwright suite.
Run: `make dist && ls -l crates/web/dist/*_bg.wasm`; the size before this plan was 1 558 448 bytes.

- [ ] **Step 3: Check against the canvas**

Run the `verify` skill. With a company that has: vouchers in several months (income and costs), one overdue and two upcoming supplier invoices, an overdue customer invoice, an open payroll run and a finalized one past its pay date, and a previous closed year — open the overview at 1280 and 390px, light and dark, and compare with artboard A2: the order of the cards, the key figures' type size, the to-do rows (icon, two lines, button), the year card's rows, the chart's bars, gridlines and legend, the unpaid list and the voucher table. Also switch year and company, and reload. Report differences; fix those that contradict the spec, each with a failing test first.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Document the overview page"
```
