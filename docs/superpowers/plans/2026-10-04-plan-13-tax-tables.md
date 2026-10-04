# Skattetabeller Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compute each employee's preliminary tax in a payroll run from Skatteverket's monthly tax table (table + column) or a fixed percentage, fetching the year's tables from Skatteverket's open data the first time they are needed, while a typed tax still overrides.

**Architecture:** A pure module `doris_payroll::tax` holds `TaxSetting`, `TaxBasis`, `TaxTable` (with `validate`) and `preliminary_tax`. Employees get a setting through a new `EmployeeTaxChanged` event; a run line's tax becomes optional (`None` = computed) and every locked line records its `TaxBasis`. Tables are reference data in a new `tax_tables` SQLite table. Only the server does HTTP (`crates/server/src/skatteverket.rs`): when the payroll crate reports `TaxTableMissing(year)`, the service fetches, validates and stores the year, then retries once.

**Tech Stack:** Rust, sqlx/SQLite, jiff, reqwest (already a server dependency), tonic 0.14 + tonic-web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-skattetabeller-design.md`

## Global Constraints

- Data source: `https://skatteverket.entryscape.net/rowstore/dataset/88320397-5c32-4c16-ae79-d36d95b17b95`, queried `?år={year}&_limit=500&_offset={n}`; overridable with `--tax-tables-url` / `DORIS_TAX_TABLES_URL`.
- Row fields: `"år"`, `"tabellnr"`, `"antal dgr"` (`"30B"` = kronor, `"30%"` = percent), `"inkomst fr.o.m."`, `"inkomst t.o.m."` (`""` = no upper limit), `"kolumn 1"`…`"kolumn 6"` (`"kolumn 7"` ignored). All values are strings.
- A valid year: tables 29–42, each with amount rows covering exactly 1–80 000 kr without gaps/overlaps, and percent rows from 80 001 kr without gaps where exactly the last row has no upper limit; no other table numbers.
- Tax setting: table 29–42 + column 1–6 (`invalid_tax_table`), or whole percent 0–100 (`invalid_tax_percent`).
- Tax arithmetic in öre: income = `gross / 100` (whole kronor, öre dropped); ≤ 80 000 kr → table amount × 100; > 80 000 kr → `floor(income × percent / 100) × 100`; 0 kr → 0; fixed percent → `floor(income × percent / 100) × 100`, never more than `gross`.
- A line's `tax: None` means computed; `Some(x)` means manual (0 ≤ x ≤ gross, else `invalid_tax`). `None` without a setting → `tax_required`.
- `TaxBasis` locked per finalized line: `Table { year, table, column }`, `Percent { percent }` or `Manual`; old events without it read as `Manual`. `schema_version` stays 1.
- `tax_tables` is reference data, not events, not rebuilt; storing a year replaces it in one `BEGIN IMMEDIATE`.
- No HTTP from `doris-payroll`; the server never holds the SQLite write lock during a fetch.
- New codes: `invalid_tax_table`, `invalid_tax_percent`, `tax_required` → `InvalidArgument`; `tax_table_unavailable` → `Unavailable`.
- Swedish texts verbatim from the spec (form labels, column labels "1 – Lön (under 66 år)" … "6 – Pension (under 66 år)", "Tabell 33, kol 1", "T33 k1", "30 %", "Manuell", error texts).
- Code, identifiers, URLs, proto, events, commits: English. Only UI text is Swedish. Never log personnummer or names.
- TDD: every behaviour starts with a failing test; each task ends in a commit. Run `cargo fmt --all` before each commit. Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv
  ```
- Wasm stays under `WASM_BUDGET` (500 KB gzipped, `make dist`). Never set `RUSTFLAGS` for wasm builds. No new dependencies.

## Review Focus

1. **A salary with öre right at the table boundary** (80 000,50 kr) — income is 80 000 kr, so the amount row applies, not a percentage. Pinned in Task 1.
2. **A January pay date while only last year's table is stored** — the new year is looked up (and fetched), never last year's table reused. Pinned in Task 3 (store) and Task 5 (server).
3. **Skatteverket answering partially** (a page with fewer rows than `resultCount` promises, then an empty page) — the year is refused and nothing is stored. Pinned in Task 4.
4. **An employee's setting changed while a run is open, and after it is finalized** — the open run's next preview uses the new setting; the finalized run keeps its locked tax and basis. Pinned in Task 3.
5. **Runs finalized before this step** (events without `tax_basis`, projection rows migrated from step 9) — they read as `Manual` and still book. Pinned in Task 2 (upcast) and Task 3 (migration).

---

### Task 1: The tax module: settings, tables and the calculation

**Files:**
- Create: `crates/payroll/src/tax.rs`
- Modify: `crates/payroll/src/lib.rs` (add `pub mod tax;`)
- Modify: `crates/payroll/src/domain.rs` (four `DomainError` variants)
- Modify: `crates/server/src/payroll.rs` (`domain_status` arms for the new variants, so the workspace builds)
- Test: `crates/payroll/tests/tax.rs`

**Interfaces:**
- Produces:
  - `DomainError::{InvalidTaxTable, InvalidTaxPercent, TaxRequired, TaxTableMissing(i16)}`.
  - `doris_payroll::tax::TaxSetting` (`Copy`, serde): `Table { table: u8, column: u8 }` | `Percent { percent: u8 }`, with `TaxSetting::table(table: u32, column: u32) -> Result<Self, DomainError>` and `TaxSetting::percent(percent: u32) -> Result<Self, DomainError>`.
  - `doris_payroll::tax::TaxBasis` (`Copy`, `Default` = `Manual`, serde): `Table { year: i16, table: u8, column: u8 }` | `Percent { percent: u8 }` | `Manual`.
  - `doris_payroll::tax::{RowKind, TaxTableRow, TaxTable, TaxTableError}`. `RowKind` is `Amount | Percent`. `TaxTableRow { table: u8, kind: RowKind, from: i64, to: Option<i64>, columns: [i64; 6] }`. `TaxTable::validate(year: i16, rows: Vec<TaxTableRow>) -> Result<TaxTable, TaxTableError>`, with `.year() -> i16` and `.rows() -> &[TaxTableRow]`. `TaxTableError(pub String)`.
  - `doris_payroll::tax::preliminary_tax(setting: TaxSetting, year: i16, table: Option<&TaxTable>, gross: i64) -> Result<(i64, TaxBasis), DomainError>`.

- [ ] **Step 1: Write the failing tests** — `crates/payroll/tests/tax.rs`:

```rust
use doris_payroll::domain::DomainError;
use doris_payroll::tax::*;

const KR: i64 = 100;

fn amount(table: u8, from: i64, to: i64, columns: [i64; 6]) -> TaxTableRow {
    TaxTableRow { table, kind: RowKind::Amount, from, to: Some(to), columns }
}

fn percent(table: u8, from: i64, to: Option<i64>, columns: [i64; 6]) -> TaxTableRow {
    TaxTableRow { table, kind: RowKind::Percent, from, to, columns }
}

/// A complete 2026-shaped year: tables 29–42 with Skatteverket's intervals
/// (1–2 000, then 100 kr to 20 000, then 200 kr to 80 000; percent rows from
/// 80 001, the last one open). Values are synthetic, except table 33, whose
/// rows at the boundaries below are Skatteverket's real 2026 figures.
fn year_2026() -> Vec<TaxTableRow> {
    let real_33: [(i64, [i64; 6]); 7] = [
        (1, [0, 0, 0, 0, 0, 0]),
        (2001, [150, 0, 150, 0, 150, 2]),
        (2101, [152, 0, 150, 2, 152, 36]),
        (19901, [3391, 3316, 1613, 3391, 5498, 5498]),
        (20001, [3439, 3364, 1631, 3439, 5568, 5568]),
        (34801, [7134, 6986, 3881, 7134, 10918, 10918]),
        (79801, [26595, 26467, 23362, 23386, 30888, 30888]),
    ];
    let mut rows = Vec::new();
    for table in 29..=42u8 {
        let mut bands = vec![(1, 2000)];
        bands.extend((2001..20000).step_by(100).map(|f| (f, f + 99)));
        bands.extend((20001..80000).step_by(200).map(|f| (f, f + 199)));
        for (from, to) in bands {
            let synthetic = [1, 2, 3, 4, 5, 6].map(|k| from / 5 + k);
            let columns = match real_33.iter().find(|(f, _)| table == 33 && *f == from) {
                Some((_, real)) => *real,
                None => synthetic,
            };
            rows.push(amount(table, from, to, columns));
        }
        let first = if table == 33 { [33, 33, 29, 29, 39, 39] } else { [33; 6] };
        rows.push(percent(table, 80001, Some(81000), first));
        rows.push(percent(table, 81001, Some(1269000), [40; 6]));
        let last = if table == 33 { [52, 52, 52, 44, 52, 52] } else { [52; 6] };
        rows.push(percent(table, 1269001, None, last));
    }
    rows
}

fn table_2026() -> TaxTable {
    TaxTable::validate(2026, year_2026()).unwrap()
}

fn t33(column: u32) -> TaxSetting {
    TaxSetting::table(33, column).unwrap()
}

fn tax(setting: TaxSetting, gross: i64) -> i64 {
    preliminary_tax(setting, 2026, Some(&table_2026()), gross).unwrap().0
}

#[test]
fn a_setting_is_table_29_to_42_with_column_1_to_6_or_0_to_100_percent() {
    assert!(TaxSetting::table(29, 1).is_ok());
    assert!(TaxSetting::table(42, 6).is_ok());
    for (t, c) in [(28, 1), (43, 1), (33, 0), (33, 7)] {
        assert_eq!(TaxSetting::table(t, c), Err(DomainError::InvalidTaxTable), "{t}/{c}");
    }
    assert_eq!(TaxSetting::percent(0).unwrap(), TaxSetting::Percent { percent: 0 });
    assert!(TaxSetting::percent(100).is_ok());
    assert_eq!(TaxSetting::percent(101), Err(DomainError::InvalidTaxPercent));
}

#[test]
fn a_complete_year_validates() {
    let table = table_2026();
    assert_eq!(table.year(), 2026);
    assert_eq!(table.rows().len(), 14 * (1 + 180 + 300 + 3));
}

#[test]
fn an_incomplete_or_odd_year_is_refused() {
    let without = |pred: &dyn Fn(&TaxTableRow) -> bool| {
        year_2026().into_iter().filter(|r| !pred(r)).collect::<Vec<_>>()
    };
    let refused = |rows: Vec<TaxTableRow>| TaxTable::validate(2026, rows).is_err();

    assert!(refused(without(&|r| r.table == 40)), "a missing table");
    assert!(refused(without(&|r| r.table == 33 && r.from == 34801)), "a gap");
    assert!(refused(without(&|r| r.table == 33 && r.from == 79801)), "amounts end before 80 000");
    assert!(refused(without(&|r| r.table == 33 && r.from == 1269001)), "no open percent row");
    let mut overlap = year_2026();
    overlap.push(amount(33, 34900, 35100, [0; 6]));
    assert!(refused(overlap), "an overlap");
    let mut two_open = year_2026();
    two_open.push(percent(33, 1269001, None, [52; 6]));
    assert!(refused(two_open), "two open percent rows");
    let mut unknown = year_2026();
    unknown.push(amount(43, 1, 80000, [0; 6]));
    assert!(refused(unknown), "an unknown table");
    assert!(refused(vec![]), "nothing");
}

#[test]
fn a_salary_up_to_80000_kr_takes_the_amount_from_its_row() {
    assert_eq!(tax(t33(1), 35_000 * KR), 7_134 * KR);
    assert_eq!(tax(t33(3), 35_000 * KR), 3_881 * KR);
    assert_eq!(tax(t33(1), 2_000 * KR), 0);
    assert_eq!(tax(t33(1), 2_001 * KR), 150 * KR);
    assert_eq!(tax(t33(1), 20_000 * KR), 3_391 * KR);
    assert_eq!(tax(t33(1), 20_001 * KR), 3_439 * KR);
    assert_eq!(tax(t33(1), 80_000 * KR), 26_595 * KR);
    // Öre are dropped: 35 000,99 kr is 35 000 kr, and 80 000,50 kr is
    // still an amount row, not a percentage.
    assert_eq!(tax(t33(1), 35_000 * KR + 99), 7_134 * KR);
    assert_eq!(tax(t33(1), 80_000 * KR + 50), 26_595 * KR);
    assert_eq!(tax(t33(1), 99), 0);
}

#[test]
fn above_80000_kr_the_percentage_applies_to_the_whole_income() {
    // 33 % of 80 001 kr = 26 400,33 → 26 400 kr.
    assert_eq!(tax(t33(1), 80_001 * KR), 26_400 * KR);
    // Column 3 has 29 %: 23 200,29 → 23 200 kr.
    assert_eq!(tax(t33(3), 80_001 * KR), 23_200 * KR);
    // The open top row: 52 % of 2 000 000 kr.
    assert_eq!(tax(t33(1), 2_000_000 * KR), 1_040_000 * KR);
}

#[test]
fn the_basis_names_year_table_and_column() {
    let (_, basis) = preliminary_tax(t33(1), 2026, Some(&table_2026()), 35_000 * KR).unwrap();
    assert_eq!(basis, TaxBasis::Table { year: 2026, table: 33, column: 1 });
}

#[test]
fn a_table_setting_without_that_years_table_is_missing() {
    assert_eq!(
        preliminary_tax(t33(1), 2026, None, 35_000 * KR),
        Err(DomainError::TaxTableMissing(2026))
    );
    // A stored table for another year doesn't count.
    assert_eq!(
        preliminary_tax(t33(1), 2027, Some(&table_2026()), 35_000 * KR),
        Err(DomainError::TaxTableMissing(2027))
    );
}

#[test]
fn a_fixed_percentage_needs_no_table_and_rounds_down() {
    let thirty = TaxSetting::percent(30).unwrap();
    // 30 % of 12 345 kr (12 345,67 with öre dropped) = 3 703,5 → 3 703 kr.
    assert_eq!(
        preliminary_tax(thirty, 2026, None, 1_234_567),
        Ok((3_703 * KR, TaxBasis::Percent { percent: 30 }))
    );
    assert_eq!(preliminary_tax(TaxSetting::percent(0).unwrap(), 2026, None, 1_234_567).unwrap().0, 0);
    // 100 % takes the whole kronor, never more than the gross.
    assert_eq!(
        preliminary_tax(TaxSetting::percent(100).unwrap(), 2026, None, 1_234_567).unwrap().0,
        1_234_500
    );
}

#[test]
fn settings_and_bases_serialize_with_a_kind_tag() {
    assert_eq!(
        serde_json::to_string(&TaxBasis::Table { year: 2026, table: 33, column: 1 }).unwrap(),
        r#"{"kind":"table","year":2026,"table":33,"column":1}"#
    );
    assert_eq!(serde_json::to_string(&TaxBasis::Manual).unwrap(), r#"{"kind":"manual"}"#);
    assert_eq!(
        serde_json::from_str::<TaxSetting>(r#"{"kind":"percent","percent":30}"#).unwrap(),
        TaxSetting::Percent { percent: 30 }
    );
    assert_eq!(TaxBasis::default(), TaxBasis::Manual);
}
```

Check of the arithmetic: per table 1 + 180 (2001–19 901 step 100) + 300 (20 001–79 801 step 200) amount rows and 3 percent rows, so 14 × 484 = 6 776 rows. 80 001 × 0,33 = 26 400,33; 80 001 × 0,29 = 23 200,29; 2 000 000 × 0,52 = 1 040 000.

`serde_json` is already a dependency of `doris-payroll`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test tax`
Expected: FAIL to compile with "unresolved import `doris_payroll::tax`".

- [ ] **Step 3: Add the error variants** — in `crates/payroll/src/domain.rs`, add to `DomainError` after `PayrollRunOutdated`:

```rust
    #[error("tax table must be 29-42 and column 1-6")]
    InvalidTaxTable,
    #[error("tax percentage must be 0-100")]
    InvalidTaxPercent,
    #[error("the line needs a tax or the employee a tax setting")]
    TaxRequired,
    /// The year's table isn't stored yet; the server fetches it.
    #[error("no tax table for {0}")]
    TaxTableMissing(i16),
```

In `crates/server/src/payroll.rs`, add to `domain_status`'s `match`:

```rust
        InvalidTaxTable => Status::invalid_argument("invalid_tax_table"),
        InvalidTaxPercent => Status::invalid_argument("invalid_tax_percent"),
        TaxRequired => Status::invalid_argument("tax_required"),
        TaxTableMissing(_) => Status::unavailable("tax_table_unavailable"),
```

- [ ] **Step 4: Write the module** — `crates/payroll/src/tax.rs`, and `pub mod tax;` after `pub mod domain;` in `crates/payroll/src/lib.rs`:

```rust
//! Preliminary tax (A-skatt) from Skatteverket's monthly tables or a fixed
//! percentage. Pure: the tables are handed in, never fetched here.

use crate::domain::DomainError;
use serde::{Deserialize, Serialize};

const TABLES: std::ops::RangeInclusive<u8> = 29..=42;
/// The monthly table gives kronor up to here, a percentage above.
const AMOUNT_LIMIT: i64 = 80_000;

/// How an employee's tax is computed (from their A-skattsedel or a
/// jämkningsbeslut).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaxSetting {
    Table { table: u8, column: u8 },
    Percent { percent: u8 },
}

impl TaxSetting {
    pub fn table(table: u32, column: u32) -> Result<Self, DomainError> {
        match (u8::try_from(table), u8::try_from(column)) {
            (Ok(table), Ok(column)) if TABLES.contains(&table) && (1..=6).contains(&column) => {
                Ok(Self::Table { table, column })
            }
            _ => Err(DomainError::InvalidTaxTable),
        }
    }

    pub fn percent(percent: u32) -> Result<Self, DomainError> {
        match u8::try_from(percent) {
            Ok(percent) if percent <= 100 => Ok(Self::Percent { percent }),
            _ => Err(DomainError::InvalidTaxPercent),
        }
    }
}

/// How a locked line's tax came about. Lines from before this existed are
/// `Manual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaxBasis {
    Table { year: i16, table: u8, column: u8 },
    Percent { percent: u8 },
    #[default]
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// `columns` are kronor ("30B").
    Amount,
    /// `columns` are percent of the whole income ("30%").
    Percent,
}

/// One income band of one table; incomes in whole kronor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxTableRow {
    pub table: u8,
    pub kind: RowKind,
    pub from: i64,
    /// `None`: no upper limit (the last percent row).
    pub to: Option<i64>,
    pub columns: [i64; 6],
}

/// A whole year's monthly tables, checked to be complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxTable {
    year: i16,
    rows: Vec<TaxTableRow>,
}

/// Why a fetched year was refused. For the log; tables hold no personal data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TaxTableError(pub String);

impl TaxTable {
    pub fn validate(year: i16, rows: Vec<TaxTableRow>) -> Result<Self, TaxTableError> {
        let fail = |why: String| Err(TaxTableError(format!("{year}: {why}")));
        if let Some(r) = rows.iter().find(|r| !TABLES.contains(&r.table)) {
            return fail(format!("unknown table {}", r.table));
        }
        for table in TABLES {
            let bands = |kind: RowKind| {
                let mut b: Vec<_> = rows.iter().filter(|r| r.table == table && r.kind == kind).collect();
                b.sort_by_key(|r| r.from);
                b
            };
            let amounts = bands(RowKind::Amount);
            let percents = bands(RowKind::Percent);
            if !contiguous(&amounts, 1) || amounts.last().and_then(|r| r.to) != Some(AMOUNT_LIMIT) {
                return fail(format!("table {table}: amounts don't cover 1-{AMOUNT_LIMIT}"));
            }
            let open = percents.iter().filter(|r| r.to.is_none()).count();
            if !contiguous(&percents, AMOUNT_LIMIT + 1) || open != 1 || percents.last().is_some_and(|r| r.to.is_some()) {
                return fail(format!("table {table}: percentages don't run from {} up", AMOUNT_LIMIT + 1));
            }
        }
        Ok(Self { year, rows })
    }

    pub fn year(&self) -> i16 {
        self.year
    }

    pub fn rows(&self) -> &[TaxTableRow] {
        &self.rows
    }

    fn row(&self, table: u8, kind: RowKind, income: i64) -> Option<&TaxTableRow> {
        self.rows.iter().find(|r| {
            r.table == table && r.kind == kind && r.from <= income && r.to.is_none_or(|to| income <= to)
        })
    }
}

/// Bands sorted by `from`, starting at `start`, each beginning right after
/// the previous one ends; only the last may be open.
fn contiguous(bands: &[&TaxTableRow], start: i64) -> bool {
    let mut next = start;
    for (i, band) in bands.iter().enumerate() {
        if band.from != next {
            return false;
        }
        match band.to {
            Some(to) if to >= band.from => next = to + 1,
            None if i == bands.len() - 1 => {}
            _ => return false,
        }
    }
    !bands.is_empty()
}

/// Preliminary tax in öre on `gross` (öre) paid in `year`, and how it was
/// found. Whole kronor: öre in the income are dropped, percentages round
/// down.
pub fn preliminary_tax(
    setting: TaxSetting,
    year: i16,
    table: Option<&TaxTable>,
    gross: i64,
) -> Result<(i64, TaxBasis), DomainError> {
    let income = gross / 100;
    match setting {
        TaxSetting::Percent { percent } => {
            let tax = (income * i64::from(percent) / 100 * 100).min(gross);
            Ok((tax, TaxBasis::Percent { percent }))
        }
        TaxSetting::Table { table: number, column } => {
            let table = table
                .filter(|t| t.year == year)
                .ok_or(DomainError::TaxTableMissing(year))?;
            let col = usize::from(column - 1);
            let kronor = if income == 0 {
                0
            } else if income <= AMOUNT_LIMIT {
                table.row(number, RowKind::Amount, income).expect("validated").columns[col]
            } else {
                let percent = table.row(number, RowKind::Percent, income).expect("validated").columns[col];
                income * percent / 100
            };
            Ok((kronor * 100, TaxBasis::Table { year, table: number, column }))
        }
    }
}
```

`Option::is_none_or` is stable since Rust 1.82; if the toolchain is older, use `map_or(true, |to| income <= to)`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-payroll --test tax && cargo test -p doris-payroll && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/payroll crates/server/src/payroll.rs
git commit -m "Compute preliminary tax from a monthly tax table or a fixed percentage

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---


### Task 2: Employee tax settings and computed tax in the payroll domain

**Files:**
- Modify: `crates/payroll/src/domain.rs`
- Modify: `crates/payroll/src/projections.rs`, `crates/payroll/src/lib.rs`, `crates/payroll/src/queries.rs`, `crates/server/src/payroll.rs` (call-site updates only, so the workspace builds; Tasks 3 and 5 finish them)
- Test: `crates/payroll/tests/domain.rs`, `crates/payroll/tests/store.rs` (existing call sites + new tests)

**Interfaces:**
- Consumes: Task 1's `TaxSetting`, `TaxBasis`, `TaxTable`, `preliminary_tax`, `DomainError::{TaxRequired, TaxTableMissing}`.
- Produces:
  - `Employee.tax: Option<TaxSetting>`.
  - `PayrollEvent::EmployeeTaxChanged { employee_id: Uuid, tax: TaxSetting }`.
  - `AddEmployee.tax: Option<TaxSetting>`. `add_employee` emits `EmployeeAdded` plus `EmployeeTaxChanged` when it is set.
  - `pub fn set_employee_tax(&Payroll, employee_id: Uuid, tax: TaxSetting) -> Result<Vec<PayrollEvent>, DomainError>`.
  - `DraftLine.tax: Option<i64>` (`None` = computed).
  - `PayrollRunLine.tax_basis: TaxBasis` (`#[serde(default)]`).
  - `compute_lines(&Payroll, Option<Uuid>, &PayrollRunDraft, Option<&TaxTable>)`.
  - `finalize_payroll_run(&Payroll, Uuid, Option<&TaxTable>)`.

- [ ] **Step 1: Update the existing tests to the new shapes** — these are mechanical changes; the behaviour of every existing test stays the same:
  - In `crates/payroll/tests/domain.rs` and `crates/payroll/tests/store.rs`, the `draft` helpers build `DraftLine { employee_id, gross, tax: Some(tax) }`.
  - In `domain.rs`, every `compute_lines(…, &d)` call gets a fourth argument `None`, and every `finalize_payroll_run(&…, run)` call (including `World::finalize`) gets a third argument `None`.
  - Every `PayrollRunLine { … }` literal in both files gets `tax_basis: TaxBasis::Manual,` after `net`.
  - Add `use doris_payroll::tax::{TaxBasis, TaxSetting, TaxTable, TaxTableRow, RowKind};` at the top of `domain.rs`, and `use doris_payroll::tax::TaxBasis;` to the `use doris_payroll::domain::{…}` block in `store.rs` (as its own line).

- [ ] **Step 2: Write the failing tests** — append to `crates/payroll/tests/domain.rs`:

```rust
/// Every table 29–42 with one amount band (1–80 000 kr: `kronor` in every
/// column) and one open 40 % band: enough for the domain, which only looks
/// rows up.
fn flat_table(year: i16, kronor: i64) -> TaxTable {
    let rows = (29..=42u8)
        .flat_map(|table| {
            [
                TaxTableRow { table, kind: RowKind::Amount, from: 1, to: Some(80_000), columns: [kronor; 6] },
                TaxTableRow { table, kind: RowKind::Percent, from: 80_001, to: None, columns: [40; 6] },
            ]
        })
        .collect();
    TaxTable::validate(year, rows).unwrap()
}

fn computed(employee_id: Uuid, gross: i64) -> DraftLine {
    DraftLine { employee_id, gross, tax: None }
}

#[test]
fn an_employee_is_added_with_a_tax_setting_and_can_change_it() {
    let id = Uuid::new_v4();
    let t33 = TaxSetting::table(33, 1).unwrap();
    let cmd = AddEmployee {
        employee_id: id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin("19800101-1231"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
        tax: Some(t33),
    };
    assert_eq!(
        add_employee(&given(&[]), cmd),
        Ok(vec![
            hired(id, "19800101-1231", 35_000 * KR),
            PayrollEvent::EmployeeTaxChanged { employee_id: id, tax: t33 },
        ])
    );

    let p = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeTaxChanged { employee_id: id, tax: t33 },
    ]);
    assert_eq!(p.employee(id).unwrap().tax, Some(t33));
    assert_eq!(set_employee_tax(&p, id, t33), Ok(vec![]));
    let thirty = TaxSetting::percent(30).unwrap();
    assert_eq!(
        set_employee_tax(&p, id, thirty),
        Ok(vec![PayrollEvent::EmployeeTaxChanged { employee_id: id, tax: thirty }])
    );
    assert_eq!(set_employee_tax(&p, Uuid::new_v4(), thirty), Err(DomainError::EmployeeNotFound));
    let gone = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(set_employee_tax(&gone, id, thirty), Err(DomainError::EmployeeInactive));
    assert_eq!(gone.employee(id).unwrap().tax, None);
}

#[test]
fn a_blank_tax_is_computed_from_the_setting_and_a_typed_one_is_manual() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let bo = w.hire("19850709-9870", 30_000 * KR);
    let ung = w.hire("20050615-1232", 20_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged { employee_id: asa, tax: TaxSetting::table(33, 1).unwrap() });
    w.then(PayrollEvent::EmployeeTaxChanged { employee_id: bo, tax: TaxSetting::percent(30).unwrap() });
    let oct = date(2026, 10, 25);
    let table = flat_table(2026, 7_000);
    let d = PayrollRunDraft {
        pay_date: oct,
        text: String::new(),
        lines: vec![
            computed(asa, 35_000 * KR),
            computed(bo, 30_000 * KR),
            DraftLine { employee_id: ung, gross: 20_000 * KR, tax: Some(3_500 * KR) },
        ],
    };

    let lines = compute_lines(&w.payroll(), None, &d, Some(&table)).unwrap();

    let taxes: Vec<_> = lines.iter().map(|l| (l.tax, l.tax_basis, l.net)).collect();
    assert_eq!(
        taxes,
        vec![
            (7_000 * KR, TaxBasis::Table { year: 2026, table: 33, column: 1 }, 28_000 * KR),
            (9_000 * KR, TaxBasis::Percent { percent: 30 }, 21_000 * KR),
            (3_500 * KR, TaxBasis::Manual, 16_500 * KR),
        ]
    );
}

#[test]
fn a_blank_tax_without_a_setting_or_a_table_is_refused() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let bo = w.hire("19850709-9870", 30_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged { employee_id: bo, tax: TaxSetting::table(33, 1).unwrap() });
    let oct = date(2026, 10, 25);
    let one = |line| PayrollRunDraft { pay_date: oct, text: String::new(), lines: vec![line] };

    // A blank tax is a valid draft; it is computed when the lines are.
    assert!(validate_draft(&w.payroll(), one(computed(asa, 100))).is_ok());
    assert_eq!(
        compute_lines(&w.payroll(), None, &one(computed(asa, 35_000 * KR)), None),
        Err(DomainError::TaxRequired)
    );
    assert_eq!(
        compute_lines(&w.payroll(), None, &one(computed(bo, 30_000 * KR)), None),
        Err(DomainError::TaxTableMissing(2026))
    );
    // Last year's table is not this year's.
    assert_eq!(
        compute_lines(&w.payroll(), None, &one(computed(bo, 30_000 * KR)), Some(&flat_table(2025, 1))),
        Err(DomainError::TaxTableMissing(2026))
    );
    // A typed tax keeps its old checks.
    assert_eq!(
        validate_draft(&w.payroll(), one(DraftLine { employee_id: asa, gross: 100, tax: Some(101) })),
        Err(DomainError::InvalidTax)
    );
}

#[test]
fn finalizing_locks_the_computed_tax_and_booking_keeps_it() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    w.then(PayrollEvent::EmployeeTaxChanged { employee_id: asa, tax: TaxSetting::table(33, 1).unwrap() });
    let d = PayrollRunDraft { pay_date: date(2026, 10, 25), text: String::new(), lines: vec![computed(asa, 35_000 * KR)] };
    let run = Uuid::new_v4();
    let event = create_payroll_run(&w.payroll(), run, d).unwrap();
    w.then(event);
    assert_eq!(
        finalize_payroll_run(&w.payroll(), run, None),
        Err(DomainError::TaxTableMissing(2026))
    );
    let event = finalize_payroll_run(&w.payroll(), run, Some(&flat_table(2026, 7_000))).unwrap();
    w.then(event);
    // A later change of setting does not touch the locked line.
    w.then(PayrollEvent::EmployeeTaxChanged { employee_id: asa, tax: TaxSetting::percent(50).unwrap() });

    let voucher = book_payroll_run(&w.payroll(), run, date(2026, 10, 25)).unwrap();

    let locked = w.payroll().run(run).unwrap().lines.clone().unwrap();
    assert_eq!((locked[0].tax, locked[0].tax_basis), (7_000 * KR, TaxBasis::Table { year: 2026, table: 33, column: 1 }));
    assert!(voucher.lines.iter().any(|l| l.account.get() == 2710 && l.credit == 7_000 * KR));
}

#[test]
fn events_from_before_tax_settings_read_as_manual() {
    let line: PayrollRunLine = serde_json::from_str(
        r#"{"employee_id":"7d0e1a2b-0000-4000-8000-000000000001","salary_account":7210,
            "gross":3500000,"tax":800000,"fee_rate":3142,"fee":1099700,"net":2700000}"#,
    )
    .unwrap();
    assert_eq!(line.tax_basis, TaxBasis::Manual);
    let draft: DraftLine = serde_json::from_str(
        r#"{"employee_id":"7d0e1a2b-0000-4000-8000-000000000001","gross":3500000,"tax":800000}"#,
    )
    .unwrap();
    assert_eq!(draft.tax, Some(800_000));
}
```

`serde_json` must be available to the test crate: it is a regular dependency of `doris-payroll`, and integration tests can use it directly.

Check of the expected values: 30 % of 30 000 kr is 9 000 kr; 35 000 − 7 000 = 28 000; 30 000 − 9 000 = 21 000.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test domain`
Expected: FAIL to compile (`no field tax on Employee`, `EmployeeTaxChanged` not found, wrong argument counts).

- [ ] **Step 4: Implement the domain** — in `crates/payroll/src/domain.rs`:

Add `use crate::tax::{TaxBasis, TaxSetting, TaxTable, preliminary_tax};` to the imports.

Add the event variant after `EmployeeDeactivated`:

```rust
    /// The employee's A-skatt setting: a table and column, or a percentage.
    EmployeeTaxChanged {
        employee_id: Uuid,
        tax: TaxSetting,
    },
```

Add `pub tax: Option<TaxSetting>,` as the last field of `Employee`, set `tax: None,` in `apply`'s `EmployeeAdded` arm, and add the arm:

```rust
            PayrollEvent::EmployeeTaxChanged { employee_id, tax } => {
                if let Some(e) = self.employee_mut(employee_id) {
                    e.tax = Some(tax);
                }
            }
```

Change `DraftLine.tax` to:

```rust
    /// `None`: computed from the employee's tax setting.
    pub tax: Option<i64>,
```

and add to `PayrollRunLine`, after `net`:

```rust
    /// How the tax came about. Lines locked before tax settings existed
    /// have none and read as `Manual`.
    #[serde(default)]
    pub tax_basis: TaxBasis,
```

Add `pub tax: Option<TaxSetting>,` as the last field of `AddEmployee`, and in `add_employee` replace the final `Ok(vec![…])` with:

```rust
    let mut events = vec![PayrollEvent::EmployeeAdded {
        employee_id: cmd.employee_id,
        name: cmd.name,
        personal_identity_number: cmd.personal_identity_number,
        monthly_salary: cmd.monthly_salary,
        salary_account: cmd.salary_account,
    }];
    events.extend(cmd.tax.map(|tax| PayrollEvent::EmployeeTaxChanged {
        employee_id: cmd.employee_id,
        tax,
    }));
    Ok(events)
```

Add after `deactivate_employee`:

```rust
/// A new tax setting for an active employee. Runs already finalized keep
/// the tax they locked.
pub fn set_employee_tax(
    payroll: &Payroll,
    employee_id: Uuid,
    tax: TaxSetting,
) -> Result<Vec<PayrollEvent>, DomainError> {
    let e = payroll.active_employee(employee_id)?;
    if e.tax == Some(tax) {
        return Ok(vec![]);
    }
    Ok(vec![PayrollEvent::EmployeeTaxChanged { employee_id, tax }])
}
```

In `validate_draft`, replace the tax check with:

```rust
        if line.tax.is_some_and(|tax| !(0..=line.gross).contains(&tax)) {
            return Err(DomainError::InvalidTax);
        }
```

Change `line` to take the tax and its basis, and `compute_lines`/`finalize_payroll_run` to take the year's table:

```rust
/// A line's fee and net pay, the youth cap counting runs other than
/// `except`.
fn line(
    payroll: &Payroll,
    except: Option<Uuid>,
    pay_date: Date,
    employee: &Employee,
    gross: i64,
    (tax, tax_basis): (i64, TaxBasis),
) -> PayrollRunLine {
    let earlier = payroll.booked_gross(employee.id, pay_date, except);
    let birth_year = employee.personal_identity_number.birth_year();
    let (fee_rate, fee) = employer_fee(birth_year, pay_date, gross, earlier);
    PayrollRunLine {
        employee_id: employee.id,
        salary_account: employee.salary_account,
        gross,
        tax,
        fee_rate,
        fee,
        net: gross - tax,
        tax_basis,
    }
}

/// The lines `draft` would lock if run `run_id` (or a new run, `None`)
/// were finalized now. `table` is the pay date's year, if stored; a blank
/// tax for an employee with a table setting needs it.
pub fn compute_lines(
    payroll: &Payroll,
    run_id: Option<Uuid>,
    draft: &PayrollRunDraft,
    table: Option<&TaxTable>,
) -> Result<Vec<PayrollRunLine>, DomainError> {
    let draft = validate_draft(payroll, draft.clone())?;
    let year = draft.pay_date.year();
    draft
        .lines
        .iter()
        .map(|l| {
            let employee = payroll.employee(l.employee_id).expect("validated");
            let tax = match (l.tax, employee.tax) {
                (Some(tax), _) => (tax, TaxBasis::Manual),
                (None, Some(setting)) => preliminary_tax(setting, year, table, l.gross)?,
                (None, None) => return Err(DomainError::TaxRequired),
            };
            Ok(line(payroll, run_id, draft.pay_date, employee, l.gross, tax))
        })
        .collect()
}
```

```rust
pub fn finalize_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    table: Option<&TaxTable>,
) -> Result<PayrollEvent, DomainError> {
    let run = open_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunFinalized {
        payroll_run_id,
        lines: compute_lines(payroll, Some(payroll_run_id), &run.draft, table)?,
    })
}
```

In `book_payroll_run`, the fee re-check calls `line(payroll, Some(payroll_run_id), pay_date, employee, l.gross, (l.tax, l.tax_basis))`.

- [ ] **Step 5: Keep the rest of the workspace building**
  - `crates/payroll/src/projections.rs`: add an arm `PayrollEvent::EmployeeTaxChanged { .. } => {}` with the comment `// Projected by Task 3 of plan 13, which replaces this arm.` The draft-line insert keeps binding `line.tax` (an `Option<i64>`; every caller still passes `Some` until Task 3).
  - `crates/payroll/src/lib.rs`: in `add_employee`, add `tax: None,` to `AddEmployee` (Task 3 wires it through). In `preview_payroll_run`, call `domain::compute_lines(&payroll, None, &draft, None)`. In `finalize_payroll_run`, call `domain::finalize_payroll_run(payroll, payroll_run_id, None)`.
  - `crates/payroll/src/queries.rs`: add `tax_basis: crate::tax::TaxBasis::Manual,` to the `PayrollRunLine` literal (Task 3 reads it from the projection).
  - `crates/server/src/payroll.rs`: in `draft()`, build `DraftLine { …, tax: Some(l.tax) }` (Task 5 makes the proto field optional).

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-payroll && cargo test -p doris-server --test payroll && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, the existing tests unchanged in behaviour.

- [ ] **Step 7: Commit**

```bash
git add crates/payroll crates/server/src/payroll.rs
git commit -m "Give employees a tax setting and compute a blank tax from it

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 3: Storage: tax tables, employee settings and locked bases

**Files:**
- Create: `migrations/0010_tax_tables.sql`
- Modify: `crates/payroll/src/lib.rs`, `crates/payroll/src/projections.rs`, `crates/payroll/src/queries.rs`
- Modify: `crates/server/src/payroll.rs` (call-site updates only: `NewEmployee { tax: None, .. }`, `PayrollRunLineView.tax` is now `Option<i64>`)
- Test: `crates/payroll/tests/store.rs`

**Interfaces:**
- Consumes: Task 2's domain (`set_employee_tax`, `EmployeeTaxChanged`, `compute_lines(.., table)`, `finalize_payroll_run(.., table)`), Task 1's `TaxTable`.
- Produces:
  - `NewEmployee.tax: Option<TaxSetting>`.
  - `pub async fn set_employee_tax(&SqlitePool, company_id: Uuid, actor: Uuid, employee_id: Uuid, tax: TaxSetting) -> Result<()>`.
  - `pub async fn tax_table(&SqlitePool, year: i16) -> Result<Option<TaxTable>>`.
  - `pub async fn store_tax_table(&SqlitePool, &TaxTable) -> Result<()>`.
  - `preview_payroll_run` and `finalize_payroll_run` use the stored table for the pay date's year. A missing one gives `Error::Domain(DomainError::TaxTableMissing(year))`, and nothing is written.
  - `list_employees` fills `Employee.tax`. `PayrollRunLineView.tax: Option<i64>`, and a locked `PayrollRunLine` carries its `tax_basis`.

- [ ] **Step 1: Write the migration** — `migrations/0010_tax_tables.sql`:

```sql
-- Skatteverket's monthly tax tables: reference data, not events. Fetched
-- once per year and replaceable; what a run used is locked in its event.
CREATE TABLE tax_tables (
    year        INTEGER NOT NULL,
    table_no    INTEGER NOT NULL,
    kind        TEXT    NOT NULL CHECK (kind IN ('amount', 'percent')),
    income_from INTEGER NOT NULL,
    income_to   INTEGER,          -- NULL: no upper limit
    col1 INTEGER NOT NULL,
    col2 INTEGER NOT NULL,
    col3 INTEGER NOT NULL,
    col4 INTEGER NOT NULL,
    col5 INTEGER NOT NULL,
    col6 INTEGER NOT NULL,
    PRIMARY KEY (year, table_no, kind, income_from)
);

-- An employee's tax setting (EmployeeTaxChanged): table and column, or a
-- percentage; all NULL without one.
ALTER TABLE employees ADD COLUMN tax_table   INTEGER;
ALTER TABLE employees ADD COLUMN tax_column  INTEGER;
ALTER TABLE employees ADD COLUMN tax_percent INTEGER;

-- A run line's tax may be blank (computed), and a locked line records its
-- basis (JSON TaxBasis). SQLite can't drop NOT NULL, so the projection table
-- is recreated; lines locked before this step are manual.
CREATE TABLE payroll_run_lines_new (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER,          -- NULL: computed from the setting
    salary_account INTEGER,          -- NULL while open
    fee_rate       INTEGER,
    fee            INTEGER,
    net            INTEGER,
    tax_basis      TEXT,             -- NULL while open
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
INSERT INTO payroll_run_lines_new (company_id, payroll_run_id, employee_id, gross, tax,
    salary_account, fee_rate, fee, net, tax_basis)
SELECT company_id, payroll_run_id, employee_id, gross, tax, salary_account, fee_rate, fee, net,
       CASE WHEN salary_account IS NULL THEN NULL ELSE '{"kind":"manual"}' END
FROM payroll_run_lines;
DROP TABLE payroll_run_lines;
ALTER TABLE payroll_run_lines_new RENAME TO payroll_run_lines;
```

- [ ] **Step 2: Write the failing tests** — in `crates/payroll/tests/store.rs`, first give every `NewEmployee { … }` literal (`asa()` and the `bo` literal in the first test) a `tax: None,` field. Then add `set_employee_tax, store_tax_table, tax_table` to the `use doris_payroll::{…}` list, and append:

```rust
use doris_payroll::tax::{RowKind, TaxSetting, TaxTable, TaxTableRow};

/// Every table 29–42 with one amount band (1–80 000 kr, `kronor` in each
/// column) and one open 40 % band.
fn flat_table(year: i16, kronor: i64) -> TaxTable {
    let rows = (29..=42u8)
        .flat_map(|table| {
            [
                TaxTableRow { table, kind: RowKind::Amount, from: 1, to: Some(80_000), columns: [kronor; 6] },
                TaxTableRow { table, kind: RowKind::Percent, from: 80_001, to: None, columns: [40; 6] },
            ]
        })
        .collect();
    TaxTable::validate(year, rows).unwrap()
}

fn computed(pay_date: &str, lines: &[(Uuid, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date: d(pay_date),
        text: String::new(),
        lines: lines.iter().map(|&(employee_id, gross)| DraftLine { employee_id, gross, tax: None }).collect(),
    }
}

/// Åsa on tabell 33, kolumn 1.
async fn asa_on_table(pool: &SqlitePool, id: Uuid, anna: Uuid) -> Uuid {
    let new = NewEmployee { tax: Some(TaxSetting::table(33, 1).unwrap()), ..asa() };
    add_employee(pool, id, anna, new).await.unwrap()
}

#[tokio::test]
async fn a_stored_year_reads_back_and_storing_it_again_replaces_it() {
    let pool = db().await;
    assert_eq!(tax_table(&pool, 2026).await.unwrap(), None);

    store_tax_table(&pool, &flat_table(2026, 7_000)).await.unwrap();
    store_tax_table(&pool, &flat_table(2026, 7_100)).await.unwrap();

    assert_eq!(tax_table(&pool, 2026).await.unwrap(), Some(flat_table(2026, 7_100)));
    assert_eq!(table(&pool, "SELECT COUNT(*) || '' FROM tax_tables").await, ["28"]);
    // Another year is not this one.
    assert_eq!(tax_table(&pool, 2027).await.unwrap(), None);
}

#[tokio::test]
async fn employees_carry_their_tax_setting() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;

    assert_eq!(
        list_employees(&pool, id, anna).await.unwrap()[0].tax,
        Some(TaxSetting::Table { table: 33, column: 1 })
    );
    set_employee_tax(&pool, id, anna, asa_id, TaxSetting::percent(30).unwrap()).await.unwrap();
    assert_eq!(
        list_employees(&pool, id, anna).await.unwrap()[0].tax,
        Some(TaxSetting::Percent { percent: 30 })
    );
    assert_eq!(
        events_of(&pool, "payroll-").await,
        ["EmployeeAdded", "EmployeeTaxChanged", "EmployeeTaxChanged"]
    );
}

#[tokio::test]
async fn a_blank_tax_needs_the_years_table_and_nothing_is_written_without_it() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    let run = create_payroll_run(&pool, id, anna, computed("2026-01-25", &[(asa_id, 35_000 * KR)]))
        .await
        .unwrap();
    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].tax, None);
    // Only last year's table is stored: January needs the new year's.
    store_tax_table(&pool, &flat_table(2025, 6_000)).await.unwrap();
    let before = events_of(&pool, "payroll-").await;

    let missing = finalize_payroll_run(&pool, id, anna, run).await.unwrap_err();
    assert!(matches!(missing, Error::Domain(DomainError::TaxTableMissing(2026))));
    assert!(matches!(
        preview_payroll_run(&pool, id, anna, computed("2026-01-25", &[(asa_id, 35_000 * KR)])).await,
        Err(Error::Domain(DomainError::TaxTableMissing(2026)))
    ));
    assert_eq!(events_of(&pool, "payroll-").await, before);

    store_tax_table(&pool, &flat_table(2026, 7_000)).await.unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let line = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].locked.unwrap();
    assert_eq!(
        (line.tax, line.tax_basis),
        (7_000 * KR, TaxBasis::Table { year: 2026, table: 33, column: 1 })
    );
}

#[tokio::test]
async fn a_changed_setting_moves_an_open_run_but_not_a_finalized_one() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    store_tax_table(&pool, &flat_table(2026, 7_000)).await.unwrap();
    let locked = create_payroll_run(&pool, id, anna, computed("2026-01-25", &[(asa_id, 35_000 * KR)]))
        .await
        .unwrap();
    finalize_payroll_run(&pool, id, anna, locked).await.unwrap();

    set_employee_tax(&pool, id, anna, asa_id, TaxSetting::percent(30).unwrap()).await.unwrap();

    let preview = preview_payroll_run(&pool, id, anna, computed("2026-02-25", &[(asa_id, 35_000 * KR)]))
        .await
        .unwrap();
    assert_eq!(
        (preview.lines[0].tax, preview.lines[0].tax_basis),
        (10_500 * KR, TaxBasis::Percent { percent: 30 })
    );
    let kept = get_payroll_run(&pool, id, anna, locked).await.unwrap().lines[0].locked.unwrap();
    assert_eq!((kept.tax, kept.tax_basis), (7_000 * KR, TaxBasis::Table { year: 2026, table: 33, column: 1 }));
}

#[tokio::test]
async fn a_run_finalized_before_tax_bases_reads_as_manual() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let run = Uuid::new_v4();
    // Events as step 9 wrote them: a number for tax, no tax_basis.
    let old = [
        format!(
            r#"{{"type":"PayrollRunCreated","payroll_run_id":"{run}","draft":{{"pay_date":"2025-10-25","text":"Lön oktober 2025","lines":[{{"employee_id":"{asa_id}","gross":3500000,"tax":800000}}]}}}}"#
        ),
        format!(
            r#"{{"type":"PayrollRunFinalized","payroll_run_id":"{run}","lines":[{{"employee_id":"{asa_id}","salary_account":7210,"gross":3500000,"tax":800000,"fee_rate":3142,"fee":1099700,"net":2700000}}]}}"#
        ),
    ];
    for (version, payload) in (2..).zip(old) {
        let event_type = if version == 2 { "PayrollRunCreated" } else { "PayrollRunFinalized" };
        sqlx::query(
            "INSERT INTO events (stream_id, stream_version, event_type, schema_version, payload, metadata)
             VALUES (?, ?, ?, 1, ?, '{\"actor\":null}')",
        )
        .bind(format!("payroll-{id}"))
        .bind(version)
        .bind(event_type)
        .bind(payload)
        .execute(&pool)
        .await
        .unwrap();
    }
    rebuild_projections(&pool).await.unwrap();

    let line = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].locked.unwrap();
    assert_eq!((line.tax, line.tax_basis), (800_000, TaxBasis::Manual));
    // And it still books.
    let voucher = book_payroll_run(&pool, id, anna, run, d("2025-10-25")).await.unwrap();
    assert_eq!(voucher.number, 1);
}

#[tokio::test]
async fn tax_settings_and_bases_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    store_tax_table(&pool, &flat_table(2026, 7_000)).await.unwrap();
    let run = create_payroll_run(&pool, id, anna, computed("2026-01-25", &[(asa_id, 35_000 * KR)]))
        .await
        .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let open = create_payroll_run(&pool, id, anna, computed("2026-02-25", &[(asa_id, 35_000 * KR)]))
        .await
        .unwrap();
    let employees = "SELECT employee_id || ':' || COALESCE(tax_table, '-') || ':'
                     || COALESCE(tax_column, '-') || ':' || COALESCE(tax_percent, '-') FROM employees";
    let lines = "SELECT payroll_run_id || ':' || COALESCE(tax, '-') || ':' || COALESCE(tax_basis, '-')
                 FROM payroll_run_lines ORDER BY 1";
    let before = (table(&pool, employees).await, table(&pool, lines).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!((table(&pool, employees).await, table(&pool, lines).await), before);
    assert_eq!(get_payroll_run(&pool, id, anna, open).await.unwrap().lines[0].tax, None);
}
```

Check of the values: 30 % of 35 000 kr is 10 500 kr; `flat_table(2026, 7_000)` gives 7 000 kr in every column.

Note: the step-9 event literal uses `DraftLine`'s and `PayrollRunLine`'s field names from `crates/payroll/src/domain.rs`; the employee id and run id are interpolated, so the stream version 2 and 3 follow the `EmployeeAdded` at version 1.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test store`
Expected: FAIL to compile (`no field tax on NewEmployee`, unresolved `set_employee_tax`, `store_tax_table`, `tax_table`).

- [ ] **Step 4: Implement** — in `crates/payroll/src/lib.rs`:

Add `use crate::tax::{RowKind, TaxSetting, TaxTable, TaxTableRow};`. Add `pub tax: Option<TaxSetting>,` to `NewEmployee` and pass `tax: new.tax,` in `add_employee`'s `AddEmployee`. Then add:

```rust
pub async fn set_employee_tax(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
    tax: TaxSetting,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        domain::set_employee_tax(payroll, employee_id, tax)
    })
    .await
}

/// The stored monthly tables for `year`, if any.
pub async fn tax_table(pool: &SqlitePool, year: i16) -> Result<Option<TaxTable>> {
    let mut conn = pool.acquire().await?;
    load_tax_table(&mut conn, year).await
}

/// Stores a year's tables, replacing any earlier copy, in one transaction.
pub async fn store_tax_table(pool: &SqlitePool, table: &TaxTable) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    sqlx::query("DELETE FROM tax_tables WHERE year = ?")
        .bind(table.year())
        .execute(&mut *tx)
        .await?;
    for row in table.rows() {
        let kind = match row.kind {
            RowKind::Amount => "amount",
            RowKind::Percent => "percent",
        };
        let [c1, c2, c3, c4, c5, c6] = row.columns;
        sqlx::query(
            "INSERT INTO tax_tables (year, table_no, kind, income_from, income_to,
                 col1, col2, col3, col4, col5, col6)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(table.year())
        .bind(row.table)
        .bind(kind)
        .bind(row.from)
        .bind(row.to)
        .bind(c1)
        .bind(c2)
        .bind(c3)
        .bind(c4)
        .bind(c5)
        .bind(c6)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `None` when the year isn't stored (or no longer validates, which makes
/// the server fetch it again).
async fn load_tax_table(conn: &mut SqliteConnection, year: i16) -> Result<Option<TaxTable>> {
    type Row = (u8, String, i64, Option<i64>, i64, i64, i64, i64, i64, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT table_no, kind, income_from, income_to, col1, col2, col3, col4, col5, col6
         FROM tax_tables WHERE year = ?",
    )
    .bind(year)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let rows = rows
        .into_iter()
        .map(|(table, kind, from, to, c1, c2, c3, c4, c5, c6)| TaxTableRow {
            table,
            kind: if kind == "amount" { RowKind::Amount } else { RowKind::Percent },
            from,
            to,
            columns: [c1, c2, c3, c4, c5, c6],
        })
        .collect();
    Ok(TaxTable::validate(year, rows).ok())
}
```

In `preview_payroll_run`, load the table before computing:

```rust
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    let draft = domain::validate_draft(&payroll, draft)?;
    let table = load_tax_table(&mut conn, draft.pay_date.year()).await?;
    let lines = domain::compute_lines(&payroll, None, &draft, table.as_ref())?;
```

Replace `finalize_payroll_run` with a version that reads the table inside its write transaction:

```rust
pub async fn finalize_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let table = match payroll.run(payroll_run_id) {
        Some(run) => load_tax_table(&mut tx, run.draft.pay_date.year()).await?,
        None => None,
    };
    let event = domain::finalize_payroll_run(&payroll, payroll_run_id, table.as_ref())?;
    append(&mut tx, company_id, version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}
```

In `crates/payroll/src/projections.rs`, replace the `EmployeeTaxChanged` placeholder arm with:

```rust
        PayrollEvent::EmployeeTaxChanged { employee_id, tax } => {
            let (table, column, percent) = match tax {
                TaxSetting::Table { table, column } => (Some(table), Some(column), None),
                TaxSetting::Percent { percent } => (None, None, Some(percent)),
            };
            sqlx::query(
                "UPDATE employees SET tax_table = ?, tax_column = ?, tax_percent = ?
                 WHERE company_id = ? AND employee_id = ?",
            )
            .bind(table)
            .bind(column)
            .bind(percent)
            .bind(company_id)
            .bind(employee_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
```

with `use crate::tax::TaxSetting;`. In `insert_locked_line`, add `tax_basis` to the column list and values, bound as `serde_json::to_string(&line.tax_basis)?`. In the `PayrollRunReopened` arm, add `tax_basis = NULL` to the `SET` list.

In `crates/payroll/src/queries.rs`, `list_employees` selects `tax_table, tax_column, tax_percent` (as `Option<u32>`) and builds:

```rust
                tax: match (tax_table, tax_column, tax_percent) {
                    (Some(table), Some(column), _) => Some(TaxSetting::table(table, column)?),
                    (_, _, Some(percent)) => Some(TaxSetting::percent(percent)?),
                    _ => None,
                },
```

`PayrollRunLineView.tax` becomes `Option<i64>` (doc: "`None`: computed when the run is finalized"). The lines query also selects `l.tax_basis`, the `Line` tuple gets `Option<i64>` for `tax` and a trailing `Option<String>`, and a line is locked only when `tax`, `account`, `fee_rate`, `fee` and `net` are all `Some`:

```rust
        let locked = match (tax, account, fee_rate, fee, net) {
            (Some(tax), Some(account), Some(fee_rate), Some(fee), Some(net)) => Some(PayrollRunLine {
                employee_id,
                salary_account: SalaryAccount::parse(account)?,
                gross,
                tax,
                fee_rate,
                fee,
                net,
                tax_basis: tax_basis
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?
                    .unwrap_or_default(),
            }),
            _ => None,
        };
```

`serde_json::Error` already converts into `Error` (`impl From<serde_json::Error> for Error`). Remove the temporary `TaxBasis::Manual` from Task 2.

In `crates/server/src/payroll.rs`: add `tax: None,` to the `NewEmployee` built in `add_employee` (Task 5 maps the proto field), and in `run_message`'s open-line branch use `tax: l.tax.unwrap_or(0),` (Task 5 makes the proto field optional).

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-payroll && cargo test -p doris-server && cargo test -p doris-ledger --test stress && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/0010_tax_tables.sql crates/payroll crates/server/src/payroll.rs
git commit -m "Store tax tables, employee tax settings and each locked line's tax basis

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 4: Fetching a year from Skatteverket

**Files:**
- Create: `crates/server/src/skatteverket.rs`
- Modify: `crates/server/src/lib.rs` (`pub mod skatteverket;`)
- Modify: `crates/server/tests/common/mod.rs` (a fake Skatteverket for this and the next task)
- Test: `crates/server/tests/tax_tables.rs`

**Interfaces:**
- Consumes: Task 1's `TaxTable::validate`, `TaxTableRow`, `RowKind`.
- Produces:
  - `doris_server::skatteverket::{TAX_TABLES_URL, TaxTables}`.
  - `TaxTables::new(url: &str) -> Self`.
  - `TaxTables::fetch(&self, year: i16) -> Result<TaxTable, String>`. The `String` is a reason for the log.
  - Test harness: `common::fake_skatteverket(rows: Vec<serde_json::Value>) -> FakeSkatteverket { url: String, requests: Arc<AtomicUsize>, broken: Arc<AtomicBool>, truncated: Arc<AtomicBool> }` and `common::tax_rows(year: i16) -> Vec<serde_json::Value>`, a complete year of 1 134 rows in Skatteverket's JSON shape, in which table 33, column 1 on 35 000 kr is 7 134 kr.

- [ ] **Step 1: Add the fake to the test harness** — append to `crates/server/tests/common/mod.rs`:

```rust
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A stand-in for Skatteverket's rowstore dataset, serving `rows` page by
/// page like the real one (`år`, `_limit`, `_offset`).
pub struct FakeSkatteverket {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    /// Answer 500.
    pub broken: Arc<AtomicBool>,
    /// Promise every row but stop sending after the first page.
    pub truncated: Arc<AtomicBool>,
}

pub async fn fake_skatteverket(rows: Vec<Value>) -> FakeSkatteverket {
    let requests = Arc::new(AtomicUsize::new(0));
    let broken = Arc::new(AtomicBool::new(false));
    let truncated = Arc::new(AtomicBool::new(false));
    let (count, fail, cut) = (requests.clone(), broken.clone(), truncated.clone());
    let app = axum::Router::new().route(
        "/rowstore",
        get(move |Query(q): Query<HashMap<String, String>>| {
            let (rows, count, fail, cut) = (rows.clone(), count.clone(), fail.clone(), cut.clone());
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                let year = q.get("år").cloned().unwrap_or_default();
                let limit: usize = q.get("_limit").and_then(|v| v.parse().ok()).unwrap_or(100);
                let offset: usize = q.get("_offset").and_then(|v| v.parse().ok()).unwrap_or(0);
                let matching: Vec<&Value> = rows.iter().filter(|r| r["år"] == year.as_str()).collect();
                let page: Vec<&Value> = if cut.load(Ordering::SeqCst) && offset > 0 {
                    vec![]
                } else {
                    matching.iter().skip(offset).take(limit).copied().collect()
                };
                axum::Json(json!({
                    "resultCount": matching.len(),
                    "offset": offset,
                    "limit": limit,
                    "results": page,
                }))
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/rowstore", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeSkatteverket { url, requests, broken, truncated }
}

/// A complete year in Skatteverket's shape: tables 29–42, each with amount
/// bands 1–2 000 kr and then 1 000 kr wide up to 80 000 kr, and percent
/// bands 80 001–1 269 000 and 1 269 001 up. Values are synthetic except
/// tabell 33, kolumn 1 on 34 001–35 000 kr: 7 134 kr, as in 2026.
pub fn tax_rows(year: i16) -> Vec<Value> {
    let row = |table: u8, kind: &str, from: i64, to: Option<i64>, cols: [i64; 6]| {
        let mut r = json!({
            "år": year.to_string(),
            "tabellnr": table.to_string(),
            "antal dgr": kind,
            "inkomst fr.o.m.": from.to_string(),
            "inkomst t.o.m.": to.map(|t| t.to_string()).unwrap_or_default(),
            "kolumn 7": "",
        });
        for (i, c) in cols.iter().enumerate() {
            r[format!("kolumn {}", i + 1)] = json!(c.to_string());
        }
        r
    };
    let mut rows = Vec::new();
    for table in 29..=42u8 {
        let mut bands = vec![(1, 2000)];
        bands.extend((2001..80000).step_by(1000).map(|f| (f, f + 999)));
        for (from, to) in bands {
            let mut cols = [1, 2, 3, 4, 5, 6].map(|k| from / 5 + k);
            if table == 33 && from == 34001 {
                cols[0] = 7134;
            }
            rows.push(row(table, "30B", from, Some(to), cols));
        }
        rows.push(row(table, "30%", 80001, Some(1269000), [33; 6]));
        rows.push(row(table, "30%", 1269001, None, [52; 6]));
    }
    rows
}
```

Some of these imports may already be in the file (for example `Arc`). Keep one of each. 14 × (1 + 78 + 2) = 1 134 rows, so a 500-row page size needs three requests.

- [ ] **Step 2: Write the failing tests** — `crates/server/tests/tax_tables.rs`:

```rust
mod common;

use common::{fake_skatteverket, tax_rows};
use doris_payroll::tax::{TaxSetting, preliminary_tax};
use doris_server::skatteverket::TaxTables;
use serde_json::json;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn a_year_is_fetched_page_by_page_and_checked() {
    let fake = fake_skatteverket(tax_rows(2026)).await;

    let table = TaxTables::new(&fake.url).fetch(2026).await.unwrap();

    assert_eq!(table.year(), 2026);
    assert_eq!(table.rows().len(), 1134);
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3);
    let t33 = TaxSetting::table(33, 1).unwrap();
    assert_eq!(preliminary_tax(t33, 2026, Some(&table), 3_500_000).unwrap().0, 713_400);
}

#[tokio::test]
async fn a_year_not_published_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    assert!(TaxTables::new(&fake.url).fetch(2027).await.is_err());
}

#[tokio::test]
async fn an_error_answer_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.broken.store(true, Ordering::SeqCst);
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn a_year_that_stops_halfway_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.truncated.store(true, Ordering::SeqCst);
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn an_unknown_row_kind_or_a_missing_table_is_refused() {
    let mut odd = tax_rows(2026);
    odd[0]["antal dgr"] = json!("14D");
    let fake = fake_skatteverket(odd).await;
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());

    let without_40: Vec<_> = tax_rows(2026).into_iter().filter(|r| r["tabellnr"] != "40").collect();
    let fake = fake_skatteverket(without_40).await;
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn nothing_listening_is_refused_quickly() {
    assert!(TaxTables::new("http://127.0.0.1:9/rowstore").fetch(2026).await.is_err());
}
```

Check of the value: 35 000 kr falls in the band 34 001–35 000, whose column 1 is 7 134 kr, which is 713 400 öre.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-server --test tax_tables`
Expected: FAIL to compile with "unresolved import `doris_server::skatteverket`".

- [ ] **Step 4: Implement** — `crates/server/src/skatteverket.rs`, and `pub mod skatteverket;` after `pub mod bolagsverket;` in `crates/server/src/lib.rs`:

```rust
//! Skatteverket's open data: the monthly tax tables ("Skattetabeller för
//! månadslön") for one year, fetched page by page and checked before they
//! are used. No credentials, and no personal data either way.

use doris_payroll::tax::{RowKind, TaxTable, TaxTableRow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

pub const TAX_TABLES_URL: &str =
    "https://skatteverket.entryscape.net/rowstore/dataset/88320397-5c32-4c16-ae79-d36d95b17b95";
const PAGE: usize = 500;

pub struct TaxTables {
    http: reqwest::Client,
    url: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(rename = "resultCount")]
    result_count: usize,
    results: Vec<HashMap<String, Value>>,
}

impl TaxTables {
    pub fn new(url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("the system TLS library loads");
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
        }
    }

    /// The whole year, or why not (for the log). An unpublished year has
    /// no rows and is refused like any other incomplete answer.
    pub async fn fetch(&self, year: i16) -> Result<TaxTable, String> {
        let mut rows = Vec::new();
        let expected = loop {
            let page = self.page(year, rows.len()).await?;
            for row in &page.results {
                rows.push(parse_row(year, row)?);
            }
            if page.results.is_empty() || rows.len() >= page.result_count {
                break page.result_count;
            }
        };
        if rows.len() != expected {
            return Err(format!("{year}: got {} of {expected} rows", rows.len()));
        }
        TaxTable::validate(year, rows).map_err(|e| e.to_string())
    }

    async fn page(&self, year: i16, offset: usize) -> Result<Page, String> {
        let failed = |e: reqwest::Error| format!("{year}: {}", e.without_url());
        self.http
            .get(&self.url)
            .query(&[
                ("år", year.to_string()),
                ("_limit", PAGE.to_string()),
                ("_offset", offset.to_string()),
            ])
            .send()
            .await
            .map_err(failed)?
            .error_for_status()
            .map_err(failed)?
            .json()
            .await
            .map_err(failed)
    }
}

/// One rowstore row: every value is text ("" for no upper limit).
fn parse_row(year: i16, row: &HashMap<String, Value>) -> Result<TaxTableRow, String> {
    let text = |key: &str| match row.get(key) {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let number = |key: &str| {
        text(key)
            .parse::<i64>()
            .map_err(|_| format!("{year}: bad {key:?}"))
    };
    if text("år") != year.to_string() {
        return Err(format!("{year}: a row for {:?}", text("år")));
    }
    let kind = match text("antal dgr").as_str() {
        "30B" => RowKind::Amount,
        "30%" => RowKind::Percent,
        other => return Err(format!("{year}: unknown row kind {other:?}")),
    };
    let to = match text("inkomst t.o.m.").as_str() {
        "" => None,
        _ => Some(number("inkomst t.o.m.")?),
    };
    let column = |n: usize| number(&format!("kolumn {n}"));
    Ok(TaxTableRow {
        table: u8::try_from(number("tabellnr")?).map_err(|_| format!("{year}: bad table"))?,
        kind,
        from: number("inkomst fr.o.m.")?,
        to,
        columns: [column(1)?, column(2)?, column(3)?, column(4)?, column(5)?, column(6)?],
    })
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-server --test tax_tables && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/server
git commit -m "Fetch a year's monthly tax tables from Skatteverket's open data

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 5: Tax settings and computed tax over gRPC-Web

**Files:**
- Modify: `proto/doris/payroll/v1/payroll.proto`
- Modify: `crates/server/src/payroll.rs`
- Modify: `crates/server/src/main.rs` (the `--tax-tables-url` flag)
- Modify: `crates/server/tests/common/mod.rs` (`PayrollApi::new(pool, tax_tables)`, `TestServer::start_with_tax_tables`)
- Modify: `crates/web/src/pages/employees.rs`, `crates/web/src/pages/payroll_run.rs`, `crates/web/src/pages/payroll_runs.rs` (field-shape updates only, so `doris-web` builds; Task 6 adds the UI)
- Test: `crates/server/tests/payroll.rs`

**Interfaces:**
- Consumes: Task 3's `set_employee_tax`, `store_tax_table`, `NewEmployee.tax`, `PayrollRunLineView.tax: Option<i64>`, `PayrollRunLine.tax_basis`. Task 4's `TaxTables`.
- Produces:
  - Proto messages `TaxSetting { oneof kind { TableTax table = 1; uint32 percent = 2; } }`, `TableTax { table, column }`, `TaxBasis { oneof kind { TableBasis table = 1; uint32 percent = 2; bool manual = 3; } }`, `TableBasis { year, table, column }`.
  - New fields: `Employee.tax = 7`, `AddEmployeeRequest.tax = 6`, `PayrollRunLineInput` `optional int64 tax = 3`, `PayrollRunLine` `optional int64 tax = 4`, `PayrollRunLine.tax_basis = 9`.
  - `rpc SetEmployeeTax(SetEmployeeTaxRequest { company_id, employee_id, tax })`.
  - `PayrollApi::new(pool: SqlitePool, tax_tables: TaxTables)`.
  - `TestServer::start_with_tax_tables(url: &str)`. The other constructors point at an unreachable URL, so tests never touch the internet.

- [ ] **Step 1: Change the proto** — in `proto/doris/payroll/v1/payroll.proto`:

Add to the service, after `DeactivateEmployee`:

```proto
  // The employee's A-skatt setting: a table and column, or a percentage.
  rpc SetEmployeeTax(SetEmployeeTaxRequest) returns (SetEmployeeTaxResponse);
```

Add the messages:

```proto
message TableTax {
  uint32 table = 1;  // 29-42
  uint32 column = 2; // 1-6
}
message TaxSetting {
  oneof kind {
    TableTax table = 1;
    uint32 percent = 2; // 0-100
  }
}
message TableBasis {
  uint32 year = 1;
  uint32 table = 2;
  uint32 column = 3;
}
message TaxBasis {
  oneof kind {
    TableBasis table = 1;
    uint32 percent = 2;
    bool manual = 3;
  }
}
message SetEmployeeTaxRequest {
  string company_id = 1;
  string employee_id = 2;
  TaxSetting tax = 3;
}
message SetEmployeeTaxResponse {}
```

Then:
- `Employee` gets `TaxSetting tax = 7; // unset: no setting, tax is typed`.
- `AddEmployeeRequest` gets `TaxSetting tax = 6; // optional`.
- `PayrollRunLineInput.tax` becomes `optional int64 tax = 3; // unset: computed`.
- `PayrollRunLine.tax` becomes `optional int64 tax = 4; // unset: an open line, computed when finalized`.
- `PayrollRunLine` gets `TaxBasis tax_basis = 9; // once finalized, and in a preview`.

- [ ] **Step 2: Keep the frontend building** — prost now generates `Option<i64>` for both `tax` fields and `Option<TaxSetting>` for `tax`. Make these minimal shape updates; Task 6 builds the UI on them:
  - `crates/web/src/pages/employees.rs`: `AddEmployeeRequest { …, tax: None }`.
  - `crates/web/src/pages/payroll_run.rs`:
    - `form_rows` uses `line.and_then(|l| l.tax).map(amount).unwrap_or_default()` for the tax text.
    - `draft()` sends `tax: Some(parse_amount(&r.tax.get_untracked()).unwrap_or(-1))`.
  - `crates/web/src/pages/payroll_runs.rs`:
    - `RunLines` shows `amount(l.tax.unwrap_or(0))`.
    - The list's sum uses `|l| l.tax.unwrap_or(0)`.

  Run: `cargo build -p doris-web && cargo build -p doris-web --target wasm32-unknown-unknown`. Expected: builds.

- [ ] **Step 3: Write the failing tests** — in `crates/server/tests/payroll.rs`, first update the existing calls: `hire`'s `AddEmployeeRequest` gets `tax: None`, the `draft` helper's `PayrollRunLineInput` gets `tax: Some(tax)`, and the inline `AddEmployeeRequest`s in `invalid_input_gets_stable_codes` get `tax: None`. Then add `fake_skatteverket, tax_rows` to `use common::{…}` and append:

```rust
use std::sync::atomic::Ordering;

fn table_33() -> Option<pb::TaxSetting> {
    Some(pb::TaxSetting {
        kind: Some(pb::tax_setting::Kind::Table(pb::TableTax { table: 33, column: 1 })),
    })
}

async fn hire_with(api: &mut Payroll, session: &str, company_id: &str, pin: &str, tax: Option<pb::TaxSetting>) -> String {
    api.add_employee(authed(
        pb::AddEmployeeRequest {
            company_id: company_id.into(),
            name: "Åsa Öberg".into(),
            personal_identity_number: pin.into(),
            monthly_salary: 35_000 * KR,
            salary_account: 7210,
            tax,
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
    .employee_id
}

fn computed(pay_date: &str, employee_id: &str) -> Option<pb::PayrollRunDraft> {
    Some(pb::PayrollRunDraft {
        pay_date: pay_date.into(),
        text: String::new(),
        lines: vec![pb::PayrollRunLineInput { employee_id: employee_id.into(), gross: 35_000 * KR, tax: None }],
    })
}

#[tokio::test]
async fn a_table_employees_tax_is_fetched_once_and_computed() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let preview = |api: &mut Payroll| {
        api.preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: computed("2026-01-25", &asa) },
            &anna,
        ))
    };
    let line = preview(&mut api).await.unwrap().into_inner().lines.remove(0);
    assert_eq!(line.tax, Some(7_134 * KR));
    assert_eq!(
        line.tax_basis.and_then(|b| b.kind),
        Some(pb::tax_basis::Kind::Table(pb::TableBasis { year: 2026, table: 33, column: 1 }))
    );
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3);

    preview(&mut api).await.unwrap();
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3, "the stored year is reused");
    let employees = api
        .list_employees(authed(pb::ListEmployeesRequest { company_id: id.clone() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .employees;
    assert_eq!(employees[0].tax, table_33());
}

#[tokio::test]
async fn without_skatteverket_a_computed_tax_is_unavailable_but_a_typed_one_works() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.broken.store(true, Ordering::SeqCst);
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: computed("2026-01-25", &asa) },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(refused), (Code::Unavailable, "tax_table_unavailable".into()));

    let typed = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR) },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(typed.lines[0].tax, Some(8_000 * KR));
    assert_eq!(typed.lines[0].tax_basis.clone().and_then(|b| b.kind), Some(pb::tax_basis::Kind::Manual(true)));
}

#[tokio::test]
async fn a_year_skatteverket_has_not_published_is_unavailable() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: computed("2027-01-25", &asa) },
            &anna,
        ))
        .await
        .unwrap_err();

    assert_eq!(code_of(refused), (Code::Unavailable, "tax_table_unavailable".into()));
}

#[tokio::test]
async fn tax_settings_are_checked_and_a_blank_tax_needs_one() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", None).await;
    let set = |tax: pb::tax_setting::Kind| pb::SetEmployeeTaxRequest {
        company_id: id.clone(),
        employee_id: asa.clone(),
        tax: Some(pb::TaxSetting { kind: Some(tax) }),
    };

    let blank = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: computed("2026-01-25", &asa) },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(blank), (Code::InvalidArgument, "tax_required".into()));

    let bad = api
        .set_employee_tax(authed(set(pb::tax_setting::Kind::Table(pb::TableTax { table: 43, column: 1 })), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(bad), (Code::InvalidArgument, "invalid_tax_table".into()));
    let bad = api.set_employee_tax(authed(set(pb::tax_setting::Kind::Percent(101)), &anna)).await.unwrap_err();
    assert_eq!(code_of(bad), (Code::InvalidArgument, "invalid_tax_percent".into()));

    api.set_employee_tax(authed(set(pb::tax_setting::Kind::Percent(30)), &anna)).await.unwrap();
    // A percentage needs no table, so no Skatteverket either.
    let line = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: computed("2026-01-25", &asa) },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .lines
        .remove(0);
    assert_eq!(line.tax, Some(10_500 * KR));
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p doris-server --test payroll`
Expected: FAIL to compile (`start_with_tax_tables` and `set_employee_tax` don't exist yet on the server side).

- [ ] **Step 5: Implement the service** — in `crates/server/src/payroll.rs`:

Add `use crate::skatteverket::TaxTables;` and `use doris_payroll::tax::{TaxBasis, TaxSetting};`. Give `PayrollApi` a `tax_tables: TaxTables` field, and change `new` to `pub fn new(pool: SqlitePool, tax_tables: TaxTables) -> Self`. Add to `impl PayrollApi`:

```rust
    /// Runs `call`; when the pay date's tax table isn't stored, fetches it
    /// from Skatteverket, stores it and runs `call` once more. The fetch
    /// happens outside any write transaction.
    async fn with_tax_table<T, F, Fut>(&self, call: F) -> Result<T, Status>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = doris_payroll::Result<T>>,
    {
        match call().await {
            Err(Error::Domain(DomainError::TaxTableMissing(year))) => {
                let table = self.tax_tables.fetch(year).await.map_err(|reason| {
                    tracing::warn!("tax table: {reason}");
                    Status::unavailable("tax_table_unavailable")
                })?;
                doris_payroll::store_tax_table(&self.pool, &table)
                    .await
                    .map_err(status)?;
                call().await.map_err(status)
            }
            result => result.map_err(status),
        }
    }
```

In `preview_payroll_run`, replace the call with `let preview = self.with_tax_table(|| doris_payroll::preview_payroll_run(&self.pool, company, user, draft.clone())).await?;`. In `finalize_payroll_run`, use `self.with_tax_table(|| doris_payroll::finalize_payroll_run(&self.pool, company, user, run)).await?;`.

In `add_employee`, pass `tax: tax_setting(req.tax)?,` in `NewEmployee`. Add the RPC:

```rust
    async fn set_employee_tax(
        &self,
        request: Request<pb::SetEmployeeTaxRequest>,
    ) -> Result<Response<pb::SetEmployeeTaxResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        // A setting can be changed, not removed: none is not a valid input.
        let tax = tax_setting(req.tax)?.ok_or_else(|| Status::invalid_argument("invalid_tax_table"))?;
        doris_payroll::set_employee_tax(&self.pool, company, user, employee_id(&req.employee_id)?, tax)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetEmployeeTaxResponse {}))
    }
```

Add the conversions:

```rust
fn tax_setting(message: Option<pb::TaxSetting>) -> Result<Option<TaxSetting>, Status> {
    match message.and_then(|m| m.kind) {
        None => Ok(None),
        Some(pb::tax_setting::Kind::Table(t)) => {
            TaxSetting::table(t.table, t.column).map(Some).map_err(domain_status)
        }
        Some(pb::tax_setting::Kind::Percent(p)) => {
            TaxSetting::percent(p).map(Some).map_err(domain_status)
        }
    }
}

fn tax_setting_message(setting: TaxSetting) -> pb::TaxSetting {
    let kind = match setting {
        TaxSetting::Table { table, column } => pb::tax_setting::Kind::Table(pb::TableTax {
            table: table.into(),
            column: column.into(),
        }),
        TaxSetting::Percent { percent } => pb::tax_setting::Kind::Percent(percent.into()),
    };
    pb::TaxSetting { kind: Some(kind) }
}

fn tax_basis_message(basis: TaxBasis) -> pb::TaxBasis {
    let kind = match basis {
        TaxBasis::Table { year, table, column } => pb::tax_basis::Kind::Table(pb::TableBasis {
            year: u32::try_from(year).unwrap_or_default(),
            table: table.into(),
            column: column.into(),
        }),
        TaxBasis::Percent { percent } => pb::tax_basis::Kind::Percent(percent.into()),
        TaxBasis::Manual => pb::tax_basis::Kind::Manual(true),
    };
    pb::TaxBasis { kind: Some(kind) }
}
```

Then:
- `employee_message` sets `tax: e.tax.map(tax_setting_message)`.
- `locked_line_message` sets `tax: Some(l.tax)` and `tax_basis: Some(tax_basis_message(l.tax_basis))`.
- `run_message`'s open-line branch sets `tax: l.tax`.
- `draft()` builds `DraftLine { …, tax: l.tax }`.

In `crates/server/src/main.rs`, add to `Config`:

```rust
    /// Skatteverket's open dataset of monthly tax tables.
    #[arg(long, env = "DORIS_TAX_TABLES_URL", default_value = doris_server::skatteverket::TAX_TABLES_URL)]
    tax_tables_url: String,
```

and build `let payroll = PayrollApi::new(pool.clone(), TaxTables::new(&config.tax_tables_url));` with `use doris_server::skatteverket::TaxTables;`.

In `crates/server/tests/common/mod.rs`, give `launch` a fourth parameter `tax_tables_url: &str`. The existing constructors pass `UNREACHABLE`, defined as:

```rust
/// Nothing listens on the discard port: tests never reach Skatteverket.
const UNREACHABLE: &str = "http://127.0.0.1:9/rowstore";
```

Build `PayrollApi::new(pool.clone(), TaxTables::new(tax_tables_url))`, and add:

```rust
    pub async fn start_with_tax_tables(url: &str) -> Self {
        Self::launch(vec![], true, None, url).await
    }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-server && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add proto crates/server crates/web
git commit -m "Serve tax settings and computed tax, fetching a missing year from Skatteverket

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 6: Frontend: tax settings on employees, computed tax in runs

**Files:**
- Modify: `crates/web/src/ui.rs` (`TextInput` gets an optional `placeholder`)
- Modify: `crates/web/src/errors.rs`
- Modify: `crates/web/src/pages/payroll_runs.rs` (labels, the "Skattegrund" column, the list's Skatt column)
- Modify: `crates/web/src/pages/payroll_run.rs` (placeholder, blank = computed, `missing_tax` only without a setting)
- Modify: `crates/web/src/pages/employees.rs` (the "Skatt" fields and column)

**Interfaces:**
- Consumes: Task 5's proto (`ppb::TaxSetting`, `ppb::tax_setting::Kind`, `ppb::TableTax`, `ppb::TaxBasis`, `ppb::tax_basis::Kind`, `ppb::SetEmployeeTaxRequest`, `Employee.tax`, `PayrollRunLine.tax: Option<i64>`, `PayrollRunLine.tax_basis`, `PayrollRunLineInput.tax: Option<i64>`).
- Produces:
  - `pages::payroll_runs::tax_setting_label(Option<&ppb::TaxSetting>) -> String`, giving "Tabell 33, kol 1", "30 %" or "–".
  - `pages::payroll_runs::tax_basis_label(Option<&ppb::TaxBasis>) -> String`, giving "T33 k1", "30 %", "Manuell" or "".
  - `pages::employees::{TAX_COLUMNS, tax_input}`.
  - The labels "Skatt", "Tabell", "Kolumn" and "Procent", and the options "Skattetabell", "Fast procent" and "Ingen (skatten skrivs in för hand)", for Task 7's e2e.

- [ ] **Step 1: Write the failing unit tests**

In `crates/web/src/errors.rs`'s `mod tests`, add:

```rust
    #[test]
    fn tax_table_codes_have_swedish_messages() {
        assert_eq!(message("invalid_tax_table"), "Välj tabell 29–42 och kolumn 1–6.");
        assert_eq!(message("invalid_tax_percent"), "Procentsatsen måste vara 0–100.");
        assert_eq!(message("tax_required"), "Ange skatt eller en skatteinställning för den anställda.");
        assert_eq!(
            message("tax_table_unavailable"),
            "Skattetabellen kunde inte hämtas från Skatteverket. Försök igen senare eller skriv in skatten för hand."
        );
    }
```

In `crates/web/src/pages/payroll_runs.rs`'s `mod tests`, add:

```rust
    #[test]
    fn tax_settings_and_bases_read_as_short_swedish_labels() {
        use crate::api::ppb::{TableBasis, TableTax, TaxBasis, TaxSetting, tax_basis, tax_setting};
        let table = TaxSetting { kind: Some(tax_setting::Kind::Table(TableTax { table: 33, column: 1 })) };
        let percent = TaxSetting { kind: Some(tax_setting::Kind::Percent(30)) };
        assert_eq!(tax_setting_label(Some(&table)), "Tabell 33, kol 1");
        assert_eq!(tax_setting_label(Some(&percent)), "30 %");
        assert_eq!(tax_setting_label(None), "–");
        let basis = |kind| TaxBasis { kind: Some(kind) };
        assert_eq!(
            tax_basis_label(Some(&basis(tax_basis::Kind::Table(TableBasis { year: 2026, table: 33, column: 1 })))),
            "T33 k1"
        );
        assert_eq!(tax_basis_label(Some(&basis(tax_basis::Kind::Percent(30)))), "30 %");
        assert_eq!(tax_basis_label(Some(&basis(tax_basis::Kind::Manual(true)))), "Manuell");
        assert_eq!(tax_basis_label(None), "");
    }
```

In `crates/web/src/pages/payroll_run.rs`'s `mod tests`, replace `blank_tax_of_an_included_employee_is_reported_by_name` with:

```rust
    #[test]
    fn a_blank_tax_is_reported_only_for_an_employee_without_a_setting() {
        let row = |n: &str, included, tax: &str, setting| (n.to_owned(), included, tax.to_owned(), setting);
        let rows = [
            row("Ann", false, "", false),
            row("Bo", true, "  ", true),
            row("Cy", true, "", false),
        ];
        assert_eq!(missing_tax(&rows).as_deref(), Some("Ange skatt för Cy."));
        assert_eq!(missing_tax(&[row("Ann", true, "0", false), row("Bo", true, "", true)]), None);
    }
```

Add to `crates/web/src/pages/employees.rs` a test module:

```rust
#[cfg(test)]
mod tests {
    use super::tax_input;
    use crate::api::ppb::{TableTax, tax_setting::Kind};

    #[test]
    fn the_tax_fields_become_a_setting_or_none() {
        assert_eq!(
            tax_input("table", "33", "1", "").and_then(|t| t.kind),
            Some(Kind::Table(TableTax { table: 33, column: 1 }))
        );
        assert_eq!(tax_input("percent", "", "", " 30 ").and_then(|t| t.kind), Some(Kind::Percent(30)));
        assert_eq!(tax_input("none", "33", "1", "30"), None);
        // Not a number: 0, which the server refuses with its own message.
        assert_eq!(
            tax_input("table", "", "1", "").and_then(|t| t.kind),
            Some(Kind::Table(TableTax { table: 0, column: 1 }))
        );
        assert_eq!(tax_input("percent", "", "", "tre").and_then(|t| t.kind), Some(Kind::Percent(1000)));
    }
}
```

Run: `cargo test -p doris-web`
Expected: FAIL to compile (`tax_setting_label`, `tax_basis_label` and `tax_input` don't exist, and `missing_tax` takes 3-tuples).

- [ ] **Step 2: Error texts** — add to `message`'s `match` in `crates/web/src/errors.rs`, before the fallback:

```rust
        "invalid_tax_table" => "Välj tabell 29–42 och kolumn 1–6.",
        "invalid_tax_percent" => "Procentsatsen måste vara 0–100.",
        "tax_required" => "Ange skatt eller en skatteinställning för den anställda.",
        "tax_table_unavailable" => {
            "Skattetabellen kunde inte hämtas från Skatteverket. Försök igen senare eller skriv in skatten för hand."
        }
```

- [ ] **Step 3: Labels and the run tables** — in `crates/web/src/pages/payroll_runs.rs`, add:

```rust
/// An employee's setting: "Tabell 33, kol 1", "30 %", or "–" without one.
pub fn tax_setting_label(setting: Option<&ppb::TaxSetting>) -> String {
    match setting.and_then(|s| s.kind.as_ref()) {
        Some(ppb::tax_setting::Kind::Table(t)) => format!("Tabell {}, kol {}", t.table, t.column),
        Some(ppb::tax_setting::Kind::Percent(p)) => format!("{p} %"),
        None => "–".to_owned(),
    }
}

/// How a locked line's tax came about: "T33 k1", "30 %" or "Manuell".
pub fn tax_basis_label(basis: Option<&ppb::TaxBasis>) -> String {
    match basis.and_then(|b| b.kind.as_ref()) {
        Some(ppb::tax_basis::Kind::Table(t)) => format!("T{} k{}", t.table, t.column),
        Some(ppb::tax_basis::Kind::Percent(p)) => format!("{p} %"),
        Some(ppb::tax_basis::Kind::Manual(_)) => "Manuell".to_owned(),
        None => String::new(),
    }
}
```

In `RunLines`, add a header cell `<th class=TABLE_HEADER_CELL>"Skattegrund"</th>` after "Skatt", and the cell `<td class=TABLE_CELL>{tax_basis_label(l.tax_basis.as_ref())}</td>` after the tax cell. If `l` is moved into the `view!` before the label is needed, compute it into a `let basis = tax_basis_label(l.tax_basis.as_ref());` above the `view!` and use `{basis}`. In the list, show the Skatt sum like Avgift and Netto: `{shown(tax)}`, because an open run's computed tax isn't known yet.

- [ ] **Step 4: The run form** — in `crates/web/src/pages/payroll_run.rs`:
  - Add `computed: bool` (the employee has a setting) and `placeholder: StoredValue<String>` to `Row`.
  - In `form_rows`, set them from the employee: `computed: e.tax.is_some()` and `placeholder: StoredValue::new(e.tax.as_ref().map(|t| tax_setting_label(Some(t))).unwrap_or_default())`, with `use crate::pages::payroll_runs::tax_setting_label;`.
  - Change `missing_tax` to:

    ```rust
    /// The first included employee without a tax setting whose Skatt is
    /// blank, as the Swedish message. For an employee with a setting, blank
    /// means computed. Blank is never 0: that would silently under-withhold.
    fn missing_tax(rows: &[(String, bool, String, bool)]) -> Option<String> {
        rows.iter()
            .find(|(_, included, tax, computed)| *included && !*computed && tax.trim().is_empty())
            .map(|(name, _, _, _)| format!("Ange skatt för {name}."))
    }
    ```

    and pass `r.computed` as the fourth element where `tax_missing` builds its rows.
  - In `draft()`, send:

    ```rust
                tax: match r.tax.get_untracked().trim() {
                    "" => None,
                    typed => Some(parse_amount(typed).unwrap_or(-1)),
                },
    ```

    (A blank tax without a setting never gets here: `tax_missing` stops it first.)
  - The Skatt `TextInput` gets `placeholder=row.placeholder.get_value()`.

In `crates/web/src/ui.rs`, `TextInput` gets `#[prop(optional, into)] placeholder: String,` and `placeholder=(!placeholder.is_empty()).then_some(placeholder)` on the `<input>`.

- [ ] **Step 5: The employee form** — in `crates/web/src/pages/employees.rs`, add:

```rust
/// Skatteverket's columns, as offered in the form.
pub const TAX_COLUMNS: [(u32, &str); 6] = [
    (1, "1 – Lön (under 66 år)"),
    (2, "2 – Pension (66 år eller äldre)"),
    (3, "3 – Lön (66 år eller äldre)"),
    (4, "4 – Sjuk- och aktivitetsersättning"),
    (5, "5 – Annan pensionsgrundande ersättning"),
    (6, "6 – Pension (under 66 år)"),
];

/// The Skatt fields as a setting: "table", "percent" or "none". A field that
/// isn't a number becomes a value the server refuses with its own message.
pub fn tax_input(kind: &str, table: &str, column: &str, percent: &str) -> Option<ppb::TaxSetting> {
    let number = |s: &str, fallback: u32| s.trim().parse().unwrap_or(fallback);
    let kind = match kind {
        "table" => ppb::tax_setting::Kind::Table(ppb::TableTax {
            table: number(table, 0),
            column: number(column, 0),
        }),
        "percent" => ppb::tax_setting::Kind::Percent(number(percent, 1000)),
        _ => return None,
    };
    Some(ppb::TaxSetting { kind: Some(kind) })
}
```

Then:
- **Signals:** add `tax_kind` (default `"table"`), `tax_table` (default `""`), `tax_column` (default `"1"`), `tax_percent` (default `""`) and `edited_tax` (`None::<ppb::TaxSetting>`, the setting of the employee being edited). `clear_form` resets all of them.
- **`edit`:** fills the signals from `e.tax`: `kind` is `"table"`, `"percent"` or `"none"`, plus the numbers. It also stores `edited_tax.set(e.tax.clone())`.
- **Form, after "Lönekonto":**
  - A `<Select label="Skatt" id="tax_kind" value=tax_kind>` with options `table` "Skattetabell" and `percent` "Fast procent". Add the option `none` "Ingen (skatten skrivs in för hand)" only while `edited_tax.get().is_none()`, which covers a new employee or one without a setting. A setting can be changed but not removed.
  - When `tax_kind` is `"table"`: `<Select label="Tabell" id="tax_table" value=tax_table>` with a first option `""` "Välj…" and then 29–42, plus `<Select label="Kolumn" id="tax_column" value=tax_column>` with `TAX_COLUMNS`.
  - When it is `"percent"`: `<Field label="Procent" id="tax_percent" value=tax_percent />`.
- **Save:**
  - Compute `let tax = tax_input(&tax_kind.get_untracked(), &tax_table.get_untracked(), &tax_column.get_untracked(), &tax_percent.get_untracked());`.
  - Adding sends `AddEmployeeRequest { …, tax: tax.clone() }`.
  - Editing sends `UpdateEmployee` and then, when `tax.is_some() && tax != edited_tax.get_untracked()`, `payroll_api().set_employee_tax(ppb::SetEmployeeTaxRequest { company_id, employee_id, tax })`. The first failure stops and shows its error.
- **The view for the tax fields**, inside the form after the "Lönekonto" select:

  ```rust
                      <Select label="Skatt" id="tax_kind" value=tax_kind>
                          <option class=SELECT_OPTION value="table">"Skattetabell"</option>
                          <option class=SELECT_OPTION value="percent">"Fast procent"</option>
                          <Show when=move || edited_tax.get().is_none()>
                              <option class=SELECT_OPTION value="none">"Ingen (skatten skrivs in för hand)"</option>
                          </Show>
                      </Select>
                      <Show when=move || tax_kind.get() == "table">
                          <Select label="Tabell" id="tax_table" value=tax_table>
                              <option class=SELECT_OPTION value="">"Välj…"</option>
                              {(29..=42u32)
                                  .map(|t| view! { <option class=SELECT_OPTION value=t.to_string()>{t}</option> })
                                  .collect_view()}
                          </Select>
                          <Select label="Kolumn" id="tax_column" value=tax_column>
                              {TAX_COLUMNS
                                  .map(|(n, label)| view! { <option class=SELECT_OPTION value=n.to_string()>{label}</option> })
                                  .collect_view()}
                          </Select>
                      </Show>
                      <Show when=move || tax_kind.get() == "percent">
                          <Field label="Procent" id="tax_percent" value=tax_percent />
                      </Show>
  ```

  A native `<select>` whose current value has no matching `<option>` shows nothing selected. That is why `edit` sets `tax_kind` to `"none"` only for an employee without a setting, which is exactly when the "Ingen" option is present.
- **Table:** a header cell "Skatt" after "Konto", and in `EmployeeRow` a cell `{tax_setting_label(employee.tax.as_ref())}`. Add `tax_setting_label(e.tax.as_ref())` to the `For` key so a changed setting re-renders.

Import `tax_setting_label` from `crate::pages::payroll_runs`.

- [ ] **Step 6: Build, lint and check the budget**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo clippy --workspace -- -D warnings && make dist`
Expected: PASS, and `make dist` stays within `WASM_BUDGET`. If it doesn't, report the gzipped size before and after; don't raise the budget.

- [ ] **Step 7: Commit**

```bash
git add crates/web
git commit -m "Set an employee's tax table or percentage and show how each run line's tax came about

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 7: End-to-end tests and documentation

**Files:**
- Modify: `e2e/tests/fixtures.ts` (the spawned server never reaches Skatteverket)
- Modify: `e2e/tests/payroll.spec.ts` (`addEmployee` picks "Ingen", since the form now defaults to "Skattetabell")
- Create: `e2e/tests/tax.spec.ts`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the UI from Task 6.

- [ ] **Step 1: Keep e2e offline** — in `e2e/tests/fixtures.ts`, add `DORIS_TAX_TABLES_URL: "http://127.0.0.1:9/rowstore",` to the spawned server's `env`, next to the other `DORIS_*` variables.

- [ ] **Step 2: Update the step-9 helper** — in `e2e/tests/payroll.spec.ts`'s `addEmployee`, before clicking "Lägg till anställd", add:

```ts
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Ingen (skatten skrivs in för hand)" });
```

Do the same in the last test ("employees are refused in Swedish…") before each "Lägg till anställd". Those tests keep typing the tax themselves.

- [ ] **Step 3: Write the new e2e tests** — `e2e/tests/tax.spec.ts`:

```ts
import { addCompany, expect, register, test } from "./fixtures";
import type { Page } from "@playwright/test";

function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function addEmployee(page: Page, name: string, personnummer: string, salary: string, tax: () => Promise<void>) {
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();
  await page.getByLabel("Namn").fill(name);
  await page.getByLabel("Personnummer").fill(personnummer);
  await page.getByLabel("Månadslön (kr)").fill(salary);
  await tax();
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^${name}`) })).toBeVisible();
}

const percent = (page: Page, p: string) => async () => {
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Fast procent" });
  await page.getByLabel("Procent").fill(p);
};
const none = (page: Page) => async () => {
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Ingen (skatten skrivs in för hand)" });
};

test("a fixed percentage computes the tax, and a typed tax is manual", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Bo Ek", "19850709-9870", "30000", percent(page, "30"));
  await expect(page.getByRole("row", { name: /^Bo Ek/ })).toContainText("30 %");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await expect(page.getByLabel("Skatt, Bo Ek")).toHaveAttribute("placeholder", "30 %");
  await page.getByLabel("Utbetalningsdag").fill(today());
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  const preview = page.getByRole("row", { name: /^Bo Ek/ }).last();
  await expect(preview).toContainText(/9\s000,00/);
  await expect(preview).toContainText("30 %");

  await page.getByLabel("Skatt, Bo Ek").fill("8500");
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  await expect(page.getByRole("row", { name: /^Bo Ek/ }).last()).toContainText("Manuell");
});

test("an employee without a setting needs a typed tax", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000", none(page));

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await page.getByLabel("Utbetalningsdag").fill(today());
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange skatt för Åsa Öberg.");
});

test("an employee is moved onto a tax table", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000", none(page));
  const row = page.getByRole("row", { name: /^Åsa Öberg/ });
  await expect(row).toContainText("–");

  await row.getByRole("button", { name: "Redigera" }).click();
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Skattetabell" });
  await page.getByLabel("Tabell").selectOption("33");
  await page.getByLabel("Kolumn").selectOption({ label: "1 – Lön (under 66 år)" });
  await page.getByRole("button", { name: "Spara ändringar" }).click();

  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Tabell 33, kol 1");
  // A setting can be changed, not removed.
  await page.getByRole("row", { name: /^Åsa Öberg/ }).getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Skatt", { exact: true }).locator("option", { hasText: "Ingen" })).toHaveCount(0);
});
```

If a selector fails while the page matches the spec, fix the test; if the page deviates from the spec, fix the page and say so. Don't weaken an assertion just to pass. "Skatt" also appears in table headers, so `{ exact: true }` is there to pick the form's select. If `getByLabel("Tabell")` also matches something else, add `{ exact: true }`.

- [ ] **Step 4: Run the e2e suite**

Run: `make e2e`
Expected: the three new tests and every existing one pass.

- [ ] **Step 5: Update AGENTS.md**
  - **Event sourcing rules**, after the payroll bullets, add:

    ```
    - Preliminary tax (`doris_payroll::tax`) comes from an employee's setting
      (`EmployeeTaxChanged`: tabell 29–42 + kolumn 1–6, or a whole percent),
      or is typed on the run line (manual). Skatteverket's monthly tables are
      reference data in `tax_tables`, not events: the server fetches a year
      from Skatteverket's open data the first time it's needed and stores it,
      replacing any earlier copy. Every locked line records its `tax_basis`
      (table/year/column, percent or manual); lines from before have none
      and read as manual.
    ```

  - **API**, add:

    ```
    - `PayrollService` also has `SetEmployeeTax`, and a run line's `tax` is
      optional (unset: computed). Codes: `invalid_tax_table`,
      `invalid_tax_percent`, `tax_required` and `tax_table_unavailable`
      (Skatteverket unreachable, or the year not published yet; a typed tax
      still works).
    - The server's outbound HTTP also fetches tax tables from Skatteverket
      (`crates/server/src/skatteverket.rs`, no credentials). It never holds
      the SQLite write lock while fetching.
    ```

  - **Commands**, add `DORIS_TAX_TABLES_URL` to the list of server environment variables.

- [ ] **Step 6: Final verification**

Run: `make test && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo fmt --all --check && make dist && cargo test -p doris-ledger --test stress`
Expected: all pass, and `make dist` stays within `WASM_BUDGET`.

- [ ] **Step 7: Commit**

```bash
git add e2e AGENTS.md
git commit -m "Cover tax settings end to end and document tax tables in AGENTS.md

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```
