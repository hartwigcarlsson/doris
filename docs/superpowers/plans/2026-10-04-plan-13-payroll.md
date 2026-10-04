# Anställda och lönekörning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An employee register and monthly payroll runs (Öppen → Färdigställd → Bokförd) that compute arbetsgivaravgift and net pay and are booked as a voucher no earlier than the pay date.

**Architecture:** A new crate `doris-payroll` with one event stream per company (`payroll-{company_id}`) holding employees and payroll runs, projected into `employees`, `payroll_runs`, `payroll_run_lines` and `payroll_run_bookings`. Booking calls `doris_ledger::record_voucher_in` and "backa bokföring" calls `doris_ledger::correct_voucher_in`, both inside the payroll write transaction. Whether a run is booked is derived: its latest booking voucher is in force unless a ledger voucher `corrects` it. A new `PayrollService` (gRPC-Web) and three Leptos pages expose it.

**Tech Stack:** Rust, sqlx/SQLite, jiff, tonic 0.14 + tonic-web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-lonekorning-design.md`

## Global Constraints

- Amounts are öre in `i64`. Monthly salary and gross: 1 ..= 10 000 000 000 000 öre (the ledger's `InvalidAmount` limit, `MAX_AMOUNT`). Tax: 0 ..= gross.
- Personnummer: `ÅÅÅÅMMDDNNNN` or `ÅÅÅÅMMDD-NNNN` (trimmed), valid date (samordningsnummer: day + 60, i.e. 61–91), Luhn over the last ten digits. Stored as twelve digits. Never logged, never sent to an external service, never changed on an employee.
- Employee name: trimmed, 1–100 characters. Salary account: 7010, 7210 (default) or 7220.
- Payroll run text: trimmed; empty becomes `"Lön {månad} {år}"` (Swedish month name, lower case, e.g. "Lön oktober 2026"); over 200 characters is `invalid_voucher_text`.
- Arbetsgivaravgift (basis points): 0 for born ≤ 1937; 1021 for born ≤ Y − 68 (Y = pay date's year); 2081 on the first 25 000 kr per calendar month for born Y − 23 ..= Y − 19 when the pay date is 2026-04-01 ..= 2027-09-30, 3142 on the rest; 3142 otherwise. `fee = (under_cap × rate + over_cap × 3142 + 5000) / 10000` öre, per line.
- The youth cap counts only other runs that are **booked** (booking in force) with a pay date in the same calendar month.
- Lifecycle: only an Öppen run can change; Färdigställd can be reopened; Bokförd cannot be reopened until its booking is reversed. Booking needs `pay_date <= today`. A rättelse for "backa bokföring" is dated `min(today, end of the voucher's fiscal year)`.
- No foreign key from payroll tables to `vouchers` (each crate rebuilds its own tables).
- Error codes (stable snake_case): `invalid_personal_identity_number`, `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`, `invalid_tax`, `empty_payroll_run`, `duplicate_payroll_run_line` → `InvalidArgument`; `duplicate_employee`, `employee_inactive`, `payroll_run_not_open`, `payroll_run_not_finalized`, `payroll_run_booked`, `payroll_run_not_booked`, `payroll_run_not_due`, `payroll_run_outdated` → `FailedPrecondition`; `employee_not_found`, `payroll_run_not_found` → `NotFound`. Text over 200 → `invalid_voucher_text` (`InvalidArgument`). Ledger errors keep the ledger's codes.
- All members of a company may use payroll; the server checks membership on every call.
- Code, identifiers, URLs, proto, events, commits: English. Only UI text is Swedish.
- TDD: every behaviour starts with a failing test; each task ends in a commit. Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv
  ```
- Wasm stays under `WASM_BUDGET` (500 KB gzipped, `make dist`). Never set `RUSTFLAGS` for wasm builds. No new dependencies beyond the workspace's.

## Review Focus

1. **A personnummer with non-ASCII characters or odd spacing** (`"19800101–1231"` with an en dash, `" 198001011231 "`, `"١٩٨٠٠١٠١١٢٣١"`) — refused or accepted cleanly, never a panic from slicing a multi-byte string. Pinned in Task 1.
2. **An employee deactivated, or moved to another salary account, after a run was finalized** — booking still works and uses the locked account; finalizing an *open* run with that employee is refused with `employee_inactive`. Pinned in Task 4.
3. **"Backa bokföring" after the fiscal year has ended** (pay date 2026-12-25, unbooked in January 2027) — the rättelse is dated 2026-12-31, not refused with `correction_date_outside_fiscal_year`. Pinned in Task 7.
4. **The payroll voucher corrected from the grundbok instead of the payroll page** — the run shows Färdigställd in both the list and the domain, and can be reopened. Pinned in Task 7.
5. **Rebuilding the ledger's projections while payroll bookings exist** — `doris_ledger::rebuild_projections` succeeds (no cross-crate foreign key), and the payroll projections rebuild to the same rows. Pinned in Task 7.

---

### Task 1: Crate scaffold and personnummer, name and salary account

**Files:**
- Modify: `Cargo.toml` (workspace members and `doris-payroll` dependency)
- Modify: `crates/company/src/domain.rs:69-80` (make `luhn` public)
- Create: `crates/payroll/Cargo.toml`
- Create: `crates/payroll/src/lib.rs`
- Create: `crates/payroll/src/domain.rs`
- Test: `crates/payroll/tests/domain.rs`

**Interfaces:**
- Produces: `doris_company::domain::luhn(digits: &str) -> bool` (public); `doris_payroll::domain::{DomainError, PersonalIdentityNumber, EmployeeName, SalaryAccount, MAX_AMOUNT}`. `PersonalIdentityNumber::parse(&str) -> Result<Self, DomainError>`, `.as_str() -> &str` (12 digits), `.formatted() -> String` (`ÅÅÅÅMMDD-NNNN`), `.birth_year() -> i16`. `EmployeeName::parse(&str)`, `.as_str()`. `SalaryAccount::parse(u32)`, `.get() -> u32`, `SalaryAccount::DEFAULT` (7210). All three derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` (`SalaryAccount` also `Copy, Hash, PartialOrd, Ord`).

- [ ] **Step 1: Make `luhn` public in the company crate**

In `crates/company/src/domain.rs`, change:

```rust
/// The Luhn check (weights 2,1,2,1…) over all ten digits.
fn luhn(digits: &str) -> bool {
```

to:

```rust
/// The Luhn check (weights 2,1,2,1…) over all ten digits. Also used for
/// personnummer in `doris-payroll`.
pub fn luhn(digits: &str) -> bool {
```

- [ ] **Step 2: Create the crate**

`crates/payroll/Cargo.toml`:

```toml
[package]
name = "doris-payroll"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
doris-company.workspace = true
doris-eventstore.workspace = true
doris-ledger.workspace = true
jiff.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
uuid.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio.workspace = true
```

In the root `Cargo.toml`, add `"crates/payroll"` to `members` and, under `[workspace.dependencies]`, `doris-payroll = { path = "crates/payroll" }` after `doris-ledger`.

`crates/payroll/src/lib.rs`:

```rust
//! Employees and payroll runs (lönekörningar) of a company, event-sourced
//! into SQLite. A run is booked as a ledger voucher in the same
//! transaction as its event.

pub mod domain;
```

`crates/payroll/src/domain.rs`:

```rust
//! Pure payroll rules: employees, payroll runs and arbetsgivaravgifter.
//! No I/O, no clock.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// The largest amount on a voucher line (the ledger's limit), in öre.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("personnummer must be ÅÅÅÅMMDD-NNNN with a valid date and check digit")]
    InvalidPersonalIdentityNumber,
    #[error("employee name must be 1-100 characters")]
    InvalidEmployeeName,
    #[error("salary must be more than zero")]
    InvalidSalary,
    #[error("salary account must be 7010, 7210 or 7220")]
    InvalidSalaryAccount,
}

/// A personnummer or samordningsnummer, as twelve digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalIdentityNumber(String);

impl PersonalIdentityNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let invalid = DomainError::InvalidPersonalIdentityNumber;
        let raw = raw.trim();
        // ASCII first: the slicing below is by byte.
        if !raw.is_ascii() {
            return Err(invalid);
        }
        let digits = match raw.len() {
            12 => raw.to_owned(),
            13 if raw.as_bytes()[8] == b'-' => format!("{}{}", &raw[..8], &raw[9..]),
            _ => return Err(invalid),
        };
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid);
        }
        let number = |range: std::ops::Range<usize>| digits[range].parse::<i16>().unwrap();
        let day = number(6..8);
        // A samordningsnummer adds 60 to the day.
        let day = if day > 60 { day - 60 } else { day };
        let date = Date::new(number(0..4), number(4..6) as i8, day as i8);
        if date.is_err() || !doris_company::domain::luhn(&digits[2..]) {
            return Err(invalid);
        }
        Ok(Self(digits))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `ÅÅÅÅMMDD-NNNN`.
    pub fn formatted(&self) -> String {
        format!("{}-{}", &self.0[..8], &self.0[8..])
    }

    pub fn birth_year(&self) -> i16 {
        self.0[..4].parse().expect("parsed digits")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeName(String);

impl EmployeeName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        match name.chars().count() {
            1..=100 => Ok(Self(name.to_owned())),
            _ => Err(DomainError::InvalidEmployeeName),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The BAS account an employee's gross salary is booked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SalaryAccount(u16);

impl SalaryAccount {
    /// 7210 Löner till tjänstemän.
    pub const DEFAULT: Self = Self(7210);

    pub fn parse(raw: u32) -> Result<Self, DomainError> {
        match raw {
            7010 | 7210 | 7220 => Ok(Self(raw as u16)),
            _ => Err(DomainError::InvalidSalaryAccount),
        }
    }

    pub fn get(self) -> u32 {
        self.0.into()
    }
}
```

- [ ] **Step 3: Write the failing tests**

`crates/payroll/tests/domain.rs`:

```rust
use doris_payroll::domain::*;

#[test]
fn a_personnummer_has_twelve_digits_a_date_and_a_check_digit() {
    let pin = PersonalIdentityNumber::parse(" 19800101-1231 ").unwrap();
    assert_eq!(pin.as_str(), "198001011231");
    assert_eq!(pin.formatted(), "19800101-1231");
    assert_eq!(pin.birth_year(), 1980);
    assert_eq!(PersonalIdentityNumber::parse("198001011231").unwrap(), pin);
    // A samordningsnummer: day + 60.
    assert!(PersonalIdentityNumber::parse("19800161-1238").is_ok());
}

#[test]
fn anything_else_is_not_a_personnummer() {
    for bad in [
        "",
        "19800101-1232",   // check digit
        "800101-1231",     // ten digits: the century is unknown
        "8001011231",
        "19800230-1235",   // 30 February
        "19800192-1231",   // samordningsnummer day 92
        "19800101+1231",
        "1980010l-1231",
        "19800101–1231",   // en dash: not ASCII, never sliced
        "١٩٨٠٠١٠١١٢٣١",
        "1980-01-01-1231",
    ] {
        assert_eq!(
            PersonalIdentityNumber::parse(bad),
            Err(DomainError::InvalidPersonalIdentityNumber),
            "{bad:?}"
        );
    }
}

#[test]
fn employee_names_are_trimmed_and_1_to_100_characters() {
    assert_eq!(EmployeeName::parse("  Åsa Öberg ").unwrap().as_str(), "Åsa Öberg");
    assert!(EmployeeName::parse(&"å".repeat(100)).is_ok());
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(EmployeeName::parse(bad), Err(DomainError::InvalidEmployeeName));
    }
}

#[test]
fn salary_accounts_are_7010_7210_or_7220() {
    for ok in [7010, 7210, 7220] {
        assert_eq!(SalaryAccount::parse(ok).unwrap().get(), ok);
    }
    assert_eq!(SalaryAccount::DEFAULT.get(), 7210);
    for bad in [0, 7211, 1930, 7510] {
        assert_eq!(SalaryAccount::parse(bad), Err(DomainError::InvalidSalaryAccount));
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-payroll --test domain`
Expected: PASS for all four. Then temporarily break the code once (for example, remove the `is_ascii` check) and confirm that `anything_else_is_not_a_personnummer` panics on the en dash before restoring it. That proves the test guards the slicing.

- [ ] **Step 5: Run the company tests and commit**

Run: `cargo test -p doris-company`
Expected: PASS.

```bash
git add Cargo.toml Cargo.lock crates/company/src/domain.rs crates/payroll
git commit -m "Add doris-payroll with personnummer, employee name and salary account

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 2: Arbetsgivaravgift

**Files:**
- Modify: `crates/payroll/src/domain.rs`
- Test: `crates/payroll/tests/domain.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub const FULL_RATE: u32 = 3142; pub const OLD_AGE_RATE: u32 = 1021; pub const YOUTH_RATE: u32 = 2081;` and `pub fn employer_fee(birth_year: i16, pay_date: Date, gross: i64, earlier_gross_same_month: i64) -> (u32, i64)`, returning the rate in basis points (for the youth reduction, the rate under the cap) and the fee in öre.

- [ ] **Step 1: Write the failing tests** — append to `crates/payroll/tests/domain.rs`:

```rust
use jiff::civil::{Date, date};

const KR: i64 = 100;

fn fee(birth_year: i16, pay_date: Date, gross: i64) -> (u32, i64) {
    employer_fee(birth_year, pay_date, gross, 0)
}

#[test]
fn the_fee_depends_on_the_year_of_birth() {
    let may = date(2026, 5, 25);
    assert_eq!(fee(1937, may, 10_000 * KR), (0, 0));
    assert_eq!(fee(1938, may, 10_000 * KR), (OLD_AGE_RATE, 102_100));
    assert_eq!(fee(1958, may, 10_000 * KR), (OLD_AGE_RATE, 102_100));
    assert_eq!(fee(1959, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(1980, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(2002, may, 10_000 * KR), (FULL_RATE, 314_200));
    assert_eq!(fee(2003, may, 10_000 * KR), (YOUTH_RATE, 208_100));
    assert_eq!(fee(2007, may, 10_000 * KR), (YOUTH_RATE, 208_100));
    assert_eq!(fee(2008, may, 10_000 * KR), (FULL_RATE, 314_200));
}

#[test]
fn the_youth_reduction_runs_from_april_2026_to_september_2027() {
    assert_eq!(fee(2005, date(2026, 3, 31), 10_000 * KR).0, FULL_RATE);
    assert_eq!(fee(2005, date(2026, 4, 1), 10_000 * KR).0, YOUTH_RATE);
    assert_eq!(fee(2005, date(2027, 9, 30), 10_000 * KR).0, YOUTH_RATE);
    assert_eq!(fee(2005, date(2027, 10, 1), 10_000 * KR).0, FULL_RATE);
    // In 2027 the age band is born 2004-2008.
    assert_eq!(fee(2003, date(2027, 5, 25), 10_000 * KR).0, FULL_RATE);
    assert_eq!(fee(2008, date(2027, 5, 25), 10_000 * KR).0, YOUTH_RATE);
}

#[test]
fn the_youth_rate_covers_25000_kr_a_month() {
    let may = date(2026, 5, 25);
    // 25 000 × 20,81 % + 5 000 × 31,42 % = 5 202,50 + 1 571 = 6 773,50 kr.
    assert_eq!(fee(2005, may, 30_000 * KR), (YOUTH_RATE, 677_350));
    // 20 000 kr already paid this month: 5 000 at 20,81 %, 15 000 at 31,42 %.
    assert_eq!(
        employer_fee(2005, may, 20_000 * KR, 20_000 * KR),
        (YOUTH_RATE, 104_050 + 471_300)
    );
    // The cap already used up: all at 31,42 %.
    assert_eq!(employer_fee(2005, may, 1_000 * KR, 25_000 * KR), (YOUTH_RATE, 31_420));
}

#[test]
fn the_fee_rounds_half_an_ore_up() {
    let may = date(2026, 5, 25);
    // 25,00 kr × 31,42 % = 7,855 kr.
    assert_eq!(fee(1980, may, 2_500), (FULL_RATE, 786));
    // 24,99 kr × 31,42 % = 7,85186 kr.
    assert_eq!(fee(1980, may, 2_499), (FULL_RATE, 785));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test domain`
Expected: FAIL to compile with "cannot find function `employer_fee`".

- [ ] **Step 3: Implement** — append to `crates/payroll/src/domain.rs` and change the `jiff` import to `use jiff::civil::{Date, date};`:

```rust
/// Full arbetsgivaravgift, in basis points of the gross salary.
pub const FULL_RATE: u32 = 3142;
/// Only ålderspensionsavgift: for those who were 67 when the year began.
pub const OLD_AGE_RATE: u32 = 1021;
/// The temporary reduction for 19–23-year-olds, on the first 25 000 kr a month.
pub const YOUTH_RATE: u32 = 2081;
const YOUTH_CAP: i64 = 2_500_000;
const YOUTH_FROM: Date = date(2026, 4, 1);
const YOUTH_UNTIL: Date = date(2027, 9, 30);

/// Arbetsgivaravgift on `gross` paid on `pay_date` to someone born in
/// `birth_year`, after `earlier_gross_same_month` already paid to them in
/// that calendar month. Returns the rate (for the youth reduction, the
/// rate under the cap) and the fee in öre, half an öre rounded up.
///
/// Skatteverket, "Arbetsgivaravgifter" (2026).
// ponytail: the rules live in code from 2026; a changed rate needs a new
// release. A table of rates by date pays off only if they change more
// often than Doris is released.
pub fn employer_fee(
    birth_year: i16,
    pay_date: Date,
    gross: i64,
    earlier_gross_same_month: i64,
) -> (u32, i64) {
    let year = pay_date.year();
    let (rate, under_cap) = if birth_year <= 1937 {
        (0, gross)
    } else if birth_year <= year - 68 {
        (OLD_AGE_RATE, gross)
    } else if (year - 23..=year - 19).contains(&birth_year)
        && (YOUTH_FROM..=YOUTH_UNTIL).contains(&pay_date)
    {
        let left = (YOUTH_CAP - earlier_gross_same_month).max(0);
        (YOUTH_RATE, gross.min(left))
    } else {
        (FULL_RATE, gross)
    };
    let over_cap = gross - under_cap;
    let fee = (under_cap * i64::from(rate) + over_cap * i64::from(FULL_RATE) + 5_000) / 10_000;
    (rate, fee)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-payroll --test domain`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/payroll
git commit -m "Compute arbetsgivaravgift by year of birth and pay date

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---
### Task 3: Employees: events, state and decisions

**Files:**
- Modify: `crates/payroll/src/domain.rs`
- Test: `crates/payroll/tests/domain.rs`

**Interfaces:**
- Consumes: Task 1's value objects.
- Produces:
  - `DomainError::{DuplicateEmployee, EmployeeNotFound, EmployeeInactive}` (added to the enum).
  - `#[serde(tag = "type")] pub enum PayrollEvent` with `EmployeeAdded { employee_id: Uuid, name: EmployeeName, personal_identity_number: PersonalIdentityNumber, monthly_salary: i64, salary_account: SalaryAccount }`, `EmployeeUpdated { employee_id: Uuid, name: EmployeeName, monthly_salary: i64, salary_account: SalaryAccount }`, `EmployeeDeactivated { employee_id: Uuid }`. Task 4 adds the run events.
  - `pub struct Employee { pub id: Uuid, pub name: EmployeeName, pub personal_identity_number: PersonalIdentityNumber, pub monthly_salary: i64, pub salary_account: SalaryAccount, pub active: bool }`.
  - `pub struct Payroll { pub employees: Vec<Employee>, pub runs: Vec<PayrollRun>, pub reversed: HashSet<BookedVoucher> }` with `Payroll::from_events(events: &[PayrollEvent], reversed: HashSet<BookedVoucher>) -> Self`, `apply(&mut self, &PayrollEvent)` and `employee(&self, Uuid) -> Option<&Employee>`. `PayrollRun` and `BookedVoucher` are declared here as stubs and filled in by Task 4.
  - `pub struct AddEmployee { pub employee_id: Uuid, pub name: EmployeeName, pub personal_identity_number: PersonalIdentityNumber, pub monthly_salary: i64, pub salary_account: SalaryAccount }` and `pub struct UpdateEmployee { pub employee_id: Uuid, pub name: EmployeeName, pub monthly_salary: i64, pub salary_account: SalaryAccount }`.
  - `pub fn add_employee(&Payroll, AddEmployee) -> Result<Vec<PayrollEvent>, DomainError>`, `pub fn update_employee(&Payroll, UpdateEmployee) -> Result<Vec<PayrollEvent>, DomainError>` and `pub fn deactivate_employee(&Payroll, Uuid) -> Result<Vec<PayrollEvent>, DomainError>`.

- [ ] **Step 1: Write the failing tests** — append to `crates/payroll/tests/domain.rs`:

```rust
use std::collections::HashSet;
use uuid::Uuid;

fn pin(raw: &str) -> PersonalIdentityNumber {
    PersonalIdentityNumber::parse(raw).unwrap()
}

fn name(raw: &str) -> EmployeeName {
    EmployeeName::parse(raw).unwrap()
}

fn hired(id: Uuid, personnummer: &str, monthly_salary: i64) -> PayrollEvent {
    PayrollEvent::EmployeeAdded {
        employee_id: id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin(personnummer),
        monthly_salary,
        salary_account: SalaryAccount::DEFAULT,
    }
}

fn given(events: &[PayrollEvent]) -> Payroll {
    Payroll::from_events(events, HashSet::new())
}

#[test]
fn an_employee_is_added_once_per_personnummer() {
    let id = Uuid::new_v4();
    let cmd = |employee_id| AddEmployee {
        employee_id,
        name: name("Åsa Öberg"),
        personal_identity_number: pin("19800101-1231"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
    };

    assert_eq!(add_employee(&given(&[]), cmd(id)), Ok(vec![hired(id, "19800101-1231", 35_000 * KR)]));

    let existing = given(&[hired(id, "19800101-1231", 35_000 * KR)]);
    assert_eq!(add_employee(&existing, cmd(Uuid::new_v4())), Err(DomainError::DuplicateEmployee));
    // Also when the existing employee is inactive.
    let inactive = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(add_employee(&inactive, cmd(Uuid::new_v4())), Err(DomainError::DuplicateEmployee));
}

#[test]
fn a_monthly_salary_is_more_than_zero_and_at_most_the_ledgers_limit() {
    for bad in [0, -1, MAX_AMOUNT + 1] {
        let cmd = AddEmployee {
            employee_id: Uuid::new_v4(),
            name: name("Åsa Öberg"),
            personal_identity_number: pin("19800101-1231"),
            monthly_salary: bad,
            salary_account: SalaryAccount::DEFAULT,
        };
        assert_eq!(add_employee(&given(&[]), cmd), Err(DomainError::InvalidSalary), "{bad}");
    }
}

#[test]
fn an_active_employee_is_updated_and_deactivated() {
    let id = Uuid::new_v4();
    let payroll = given(&[hired(id, "19800101-1231", 35_000 * KR)]);
    let update = |employee_id, monthly_salary| UpdateEmployee {
        employee_id,
        name: name("Åsa Öberg"),
        monthly_salary,
        salary_account: SalaryAccount::parse(7220).unwrap(),
    };

    assert_eq!(
        update_employee(&payroll, update(id, 36_000 * KR)),
        Ok(vec![PayrollEvent::EmployeeUpdated {
            employee_id: id,
            name: name("Åsa Öberg"),
            monthly_salary: 36_000 * KR,
            salary_account: SalaryAccount::parse(7220).unwrap(),
        }])
    );
    assert_eq!(update_employee(&payroll, update(id, 0)), Err(DomainError::InvalidSalary));
    assert_eq!(
        update_employee(&payroll, update(Uuid::new_v4(), 36_000 * KR)),
        Err(DomainError::EmployeeNotFound)
    );
    // No change, no event.
    let same = UpdateEmployee {
        employee_id: id,
        name: name("Åsa Öberg"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::DEFAULT,
    };
    assert_eq!(update_employee(&payroll, same), Ok(vec![]));

    assert_eq!(
        deactivate_employee(&payroll, id),
        Ok(vec![PayrollEvent::EmployeeDeactivated { employee_id: id }])
    );
    assert_eq!(deactivate_employee(&payroll, Uuid::new_v4()), Err(DomainError::EmployeeNotFound));

    let inactive = given(&[
        hired(id, "19800101-1231", 35_000 * KR),
        PayrollEvent::EmployeeDeactivated { employee_id: id },
    ]);
    assert_eq!(deactivate_employee(&inactive, id), Ok(vec![]));
    assert_eq!(
        update_employee(&inactive, update(id, 36_000 * KR)),
        Err(DomainError::EmployeeInactive)
    );
    assert!(!inactive.employee(id).unwrap().active);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test domain`
Expected: FAIL to compile with "cannot find type `PayrollEvent`".

- [ ] **Step 3: Implement**

Add to `DomainError`, after `InvalidSalaryAccount`:

```rust
    #[error("an employee with this personnummer exists")]
    DuplicateEmployee,
    #[error("no such employee")]
    EmployeeNotFound,
    #[error("the employee is inactive")]
    EmployeeInactive,
```

Add `use std::collections::HashSet;` and `use uuid::Uuid;` to the imports, then append:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PayrollEvent {
    EmployeeAdded {
        employee_id: Uuid,
        name: EmployeeName,
        personal_identity_number: PersonalIdentityNumber,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeUpdated {
        employee_id: Uuid,
        name: EmployeeName,
        monthly_salary: i64,
        salary_account: SalaryAccount,
    },
    EmployeeDeactivated {
        employee_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Employee {
    pub id: Uuid,
    pub name: EmployeeName,
    pub personal_identity_number: PersonalIdentityNumber,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
    pub active: bool,
}

/// A voucher a payroll run was booked as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BookedVoucher {
    pub fiscal_year_start: Date,
    pub number: u32,
}

/// Filled in with the payroll run events.
#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRun {
    pub id: Uuid,
}

/// A company's employees and payroll runs. `reversed` holds the ledger
/// vouchers that have been corrected (rättade): a run booked as one of
/// them is no longer booked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Payroll {
    pub employees: Vec<Employee>,
    pub runs: Vec<PayrollRun>,
    pub reversed: HashSet<BookedVoucher>,
}

impl Payroll {
    pub fn from_events(events: &[PayrollEvent], reversed: HashSet<BookedVoucher>) -> Self {
        let mut payroll = Self {
            reversed,
            ..Self::default()
        };
        for event in events {
            payroll.apply(event);
        }
        payroll
    }

    pub fn apply(&mut self, event: &PayrollEvent) {
        match event.clone() {
            PayrollEvent::EmployeeAdded {
                employee_id,
                name,
                personal_identity_number,
                monthly_salary,
                salary_account,
            } => self.employees.push(Employee {
                id: employee_id,
                name,
                personal_identity_number,
                monthly_salary,
                salary_account,
                active: true,
            }),
            PayrollEvent::EmployeeUpdated {
                employee_id,
                name,
                monthly_salary,
                salary_account,
            } => {
                if let Some(e) = self.employee_mut(employee_id) {
                    e.name = name;
                    e.monthly_salary = monthly_salary;
                    e.salary_account = salary_account;
                }
            }
            PayrollEvent::EmployeeDeactivated { employee_id } => {
                if let Some(e) = self.employee_mut(employee_id) {
                    e.active = false;
                }
            }
        }
    }

    pub fn employee(&self, id: Uuid) -> Option<&Employee> {
        self.employees.iter().find(|e| e.id == id)
    }

    fn employee_mut(&mut self, id: Uuid) -> Option<&mut Employee> {
        self.employees.iter_mut().find(|e| e.id == id)
    }

    fn active_employee(&self, id: Uuid) -> Result<&Employee, DomainError> {
        match self.employee(id) {
            None => Err(DomainError::EmployeeNotFound),
            Some(e) if !e.active => Err(DomainError::EmployeeInactive),
            Some(e) => Ok(e),
        }
    }
}

fn check_salary(ore: i64) -> Result<i64, DomainError> {
    if (1..=MAX_AMOUNT).contains(&ore) {
        Ok(ore)
    } else {
        Err(DomainError::InvalidSalary)
    }
}

#[derive(Debug, Clone)]
pub struct AddEmployee {
    pub employee_id: Uuid,
    pub name: EmployeeName,
    pub personal_identity_number: PersonalIdentityNumber,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
}

/// A personnummer is unique in the company, also among inactive
/// employees; the `employees` projection's UNIQUE backs this up.
pub fn add_employee(payroll: &Payroll, cmd: AddEmployee) -> Result<Vec<PayrollEvent>, DomainError> {
    check_salary(cmd.monthly_salary)?;
    if payroll
        .employees
        .iter()
        .any(|e| e.personal_identity_number == cmd.personal_identity_number)
    {
        return Err(DomainError::DuplicateEmployee);
    }
    Ok(vec![PayrollEvent::EmployeeAdded {
        employee_id: cmd.employee_id,
        name: cmd.name,
        personal_identity_number: cmd.personal_identity_number,
        monthly_salary: cmd.monthly_salary,
        salary_account: cmd.salary_account,
    }])
}

#[derive(Debug, Clone)]
pub struct UpdateEmployee {
    pub employee_id: Uuid,
    pub name: EmployeeName,
    pub monthly_salary: i64,
    pub salary_account: SalaryAccount,
}

/// The personnummer never changes: a wrong one is a new employee.
pub fn update_employee(
    payroll: &Payroll,
    cmd: UpdateEmployee,
) -> Result<Vec<PayrollEvent>, DomainError> {
    check_salary(cmd.monthly_salary)?;
    let e = payroll.active_employee(cmd.employee_id)?;
    if (&e.name, e.monthly_salary, e.salary_account)
        == (&cmd.name, cmd.monthly_salary, cmd.salary_account)
    {
        return Ok(vec![]);
    }
    Ok(vec![PayrollEvent::EmployeeUpdated {
        employee_id: cmd.employee_id,
        name: cmd.name,
        monthly_salary: cmd.monthly_salary,
        salary_account: cmd.salary_account,
    }])
}

pub fn deactivate_employee(
    payroll: &Payroll,
    employee_id: Uuid,
) -> Result<Vec<PayrollEvent>, DomainError> {
    let e = payroll
        .employee(employee_id)
        .ok_or(DomainError::EmployeeNotFound)?;
    if !e.active {
        return Ok(vec![]);
    }
    Ok(vec![PayrollEvent::EmployeeDeactivated { employee_id }])
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-payroll --test domain`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/payroll
git commit -m "Add, update and deactivate employees in the payroll domain

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 4: Payroll runs: drafts, lines, voucher lines and lifecycle

**Files:**
- Modify: `crates/payroll/src/domain.rs`
- Test: `crates/payroll/tests/domain.rs`

**Interfaces:**
- Consumes: Task 2's `employer_fee`, Task 3's `Payroll`, `Employee`, `BookedVoucher`, `PayrollEvent`.
- Produces:
  - `DomainError::{InvalidTax, InvalidText, EmptyPayrollRun, DuplicatePayrollRunLine, PayrollRunNotFound, PayrollRunNotOpen, PayrollRunNotFinalized, PayrollRunBooked, PayrollRunNotBooked, PayrollRunNotDue, PayrollRunOutdated}`.
  - `pub struct PayrollRunDraft { pub pay_date: Date, pub text: String, pub lines: Vec<DraftLine> }` and `pub struct DraftLine { pub employee_id: Uuid, pub gross: i64, pub tax: i64 }` (both serde).
  - `pub struct PayrollRunLine { pub employee_id: Uuid, pub salary_account: SalaryAccount, pub gross: i64, pub tax: i64, pub fee_rate: u32, pub fee: i64, pub net: i64 }` (serde, `Copy`).
  - `PayrollRun { pub id: Uuid, pub draft: PayrollRunDraft, pub lines: Option<Vec<PayrollRunLine>>, pub bookings: Vec<BookedVoucher> }` replaces the stub. `pub enum PayrollRunStatus { Open, Finalized, Booked(BookedVoucher) }`.
  - New `PayrollEvent` variants: `PayrollRunCreated { payroll_run_id: Uuid, draft: PayrollRunDraft }`, `PayrollRunUpdated { payroll_run_id: Uuid, draft: PayrollRunDraft }`, `PayrollRunFinalized { payroll_run_id: Uuid, lines: Vec<PayrollRunLine> }`, `PayrollRunReopened { payroll_run_id: Uuid }`, `PayrollRunBooked { payroll_run_id: Uuid, voucher: BookedVoucher }`.
  - `Payroll::run(&self, Uuid) -> Option<&PayrollRun>` and `Payroll::status(&self, &PayrollRun) -> PayrollRunStatus`.
  - `pub fn validate_draft(&Payroll, PayrollRunDraft) -> Result<PayrollRunDraft, DomainError>` (returns the draft with its final text), `pub fn compute_lines(&Payroll, Option<Uuid>, &PayrollRunDraft) -> Result<Vec<PayrollRunLine>, DomainError>` and `pub fn voucher_lines(&[PayrollRunLine]) -> Vec<doris_ledger::domain::VoucherLine>`.
  - `pub fn create_payroll_run(&Payroll, Uuid, PayrollRunDraft) -> Result<PayrollEvent, DomainError>`, `update_payroll_run(&Payroll, Uuid, PayrollRunDraft)`, `finalize_payroll_run(&Payroll, Uuid)`, `reopen_payroll_run(&Payroll, Uuid)`, all `-> Result<PayrollEvent, DomainError>`. Also `pub fn book_payroll_run(&Payroll, Uuid, today: Date) -> Result<doris_ledger::domain::RecordVoucher, DomainError>`, `pub fn booked(Uuid, BookedVoucher) -> PayrollEvent` and `pub fn unbook_payroll_run(&Payroll, Uuid) -> Result<BookedVoucher, DomainError>`.

- [ ] **Step 1: Write the failing tests** — append to `crates/payroll/tests/domain.rs`:

```rust
use doris_ledger::domain::VoucherLine;

/// Events so far, plus the ledger vouchers that have been corrected.
#[derive(Default)]
struct World {
    events: Vec<PayrollEvent>,
    reversed: HashSet<BookedVoucher>,
}

impl World {
    fn payroll(&self) -> Payroll {
        Payroll::from_events(&self.events, self.reversed.clone())
    }

    fn then(&mut self, event: PayrollEvent) {
        self.events.push(event);
    }

    fn hire(&mut self, personnummer: &str, monthly_salary: i64) -> Uuid {
        let id = Uuid::new_v4();
        self.then(hired(id, personnummer, monthly_salary));
        id
    }

    fn create(&mut self, draft: PayrollRunDraft) -> Uuid {
        let id = Uuid::new_v4();
        let event = create_payroll_run(&self.payroll(), id, draft).unwrap();
        self.then(event);
        id
    }

    fn finalize(&mut self, run: Uuid) {
        let event = finalize_payroll_run(&self.payroll(), run).unwrap();
        self.then(event);
    }

    fn book(&mut self, run: Uuid, number: u32) -> BookedVoucher {
        let voucher = BookedVoucher {
            fiscal_year_start: date(2026, 1, 1),
            number,
        };
        self.then(booked(run, voucher));
        voucher
    }

    fn status(&self, run: Uuid) -> PayrollRunStatus {
        let payroll = self.payroll();
        payroll.status(payroll.run(run).unwrap())
    }
}

fn draft(pay_date: Date, lines: &[(Uuid, i64, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date,
        text: String::new(),
        lines: lines
            .iter()
            .map(|&(employee_id, gross, tax)| DraftLine { employee_id, gross, tax })
            .collect(),
    }
}

fn vl(account: u32, debit: i64, credit: i64) -> VoucherLine {
    VoucherLine {
        account: doris_ledger::domain::AccountNumber::parse(account).unwrap(),
        debit,
        credit,
    }
}

#[test]
fn a_draft_needs_active_employees_once_each_with_a_salary_and_a_tax() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let gone = w.hire("19850709-9870", 30_000 * KR);
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: gone });
    let p = w.payroll();
    let oct = date(2026, 10, 25);
    let check = |lines: &[(Uuid, i64, i64)]| validate_draft(&p, draft(oct, lines)).map(|_| ());

    assert_eq!(check(&[]), Err(DomainError::EmptyPayrollRun));
    assert_eq!(
        check(&[(asa, 100, 0), (asa, 100, 0)]),
        Err(DomainError::DuplicatePayrollRunLine)
    );
    assert_eq!(check(&[(Uuid::new_v4(), 100, 0)]), Err(DomainError::EmployeeNotFound));
    assert_eq!(check(&[(gone, 100, 0)]), Err(DomainError::EmployeeInactive));
    assert_eq!(check(&[(asa, 0, 0)]), Err(DomainError::InvalidSalary));
    assert_eq!(check(&[(asa, MAX_AMOUNT + 1, 0)]), Err(DomainError::InvalidSalary));
    assert_eq!(check(&[(asa, 100, -1)]), Err(DomainError::InvalidTax));
    assert_eq!(check(&[(asa, 100, 101)]), Err(DomainError::InvalidTax));
    assert_eq!(check(&[(asa, 100, 100)]), Ok(()));

    let mut long = draft(oct, &[(asa, 100, 0)]);
    long.text = "å".repeat(201);
    assert_eq!(validate_draft(&p, long), Err(DomainError::InvalidText));
}

#[test]
fn an_empty_text_becomes_the_month_and_a_future_pay_date_is_fine() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let p = w.payroll();

    let valid = validate_draft(&p, draft(date(2030, 1, 25), &[(asa, 100, 0)])).unwrap();
    assert_eq!(valid.text, "Lön januari 2030");

    let mut typed = draft(date(2026, 12, 23), &[(asa, 100, 0)]);
    typed.text = "  Julbonus  ".into();
    assert_eq!(validate_draft(&p, typed).unwrap().text, "Julbonus");
    let mut blank = draft(date(2026, 12, 23), &[(asa, 100, 0)]);
    blank.text = "   ".into();
    assert_eq!(validate_draft(&p, blank).unwrap().text, "Lön december 2026");
}

#[test]
fn lines_carry_the_account_fee_and_net_pay() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let ung = w.hire("20050615-1232", 20_000 * KR);
    w.then(PayrollEvent::EmployeeUpdated {
        employee_id: ung,
        name: name("Unga Ung"),
        monthly_salary: 20_000 * KR,
        salary_account: SalaryAccount::parse(7010).unwrap(),
    });
    let d = draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR), (ung, 20_000 * KR, 3_500 * KR)]);

    let lines = compute_lines(&w.payroll(), None, &d).unwrap();

    assert_eq!(
        lines,
        vec![
            PayrollRunLine {
                employee_id: asa,
                salary_account: SalaryAccount::DEFAULT,
                gross: 35_000 * KR,
                tax: 8_000 * KR,
                fee_rate: FULL_RATE,
                fee: 1_099_700,
                net: 27_000 * KR,
            },
            PayrollRunLine {
                employee_id: ung,
                salary_account: SalaryAccount::parse(7010).unwrap(),
                gross: 20_000 * KR,
                tax: 3_500 * KR,
                fee_rate: YOUTH_RATE,
                fee: 416_200,
                net: 16_500 * KR,
            },
        ]
    );
}

#[test]
fn the_voucher_balances_grouped_by_account_without_zero_lines() {
    let line = |account: u32, gross: i64, tax: i64, fee: i64| PayrollRunLine {
        employee_id: Uuid::new_v4(),
        salary_account: SalaryAccount::parse(account).unwrap(),
        gross,
        tax,
        fee_rate: FULL_RATE,
        fee,
        net: gross - tax,
    };
    let lines = [
        line(7210, 35_000 * KR, 8_000 * KR, 1_099_700),
        line(7010, 20_000 * KR, 0, 628_400),
        line(7210, 10_000 * KR, 2_000 * KR, 314_200),
    ];

    let voucher = voucher_lines(&lines);

    assert_eq!(
        voucher,
        vec![
            vl(7010, 20_000 * KR, 0),
            vl(7210, 45_000 * KR, 0),
            vl(2710, 0, 10_000 * KR),
            vl(1930, 0, 55_000 * KR),
            vl(7510, 2_042_300, 0),
            vl(2731, 0, 2_042_300),
        ]
    );
    let debit: i64 = voucher.iter().map(|l| l.debit).sum();
    let credit: i64 = voucher.iter().map(|l| l.credit).sum();
    assert_eq!(debit, credit);

    // No tax and no fee (born 1937): only salary and bank.
    let free = [line(7210, 1_000 * KR, 0, 0)];
    assert_eq!(voucher_lines(&free), vec![vl(7210, 1_000 * KR, 0), vl(1930, 0, 1_000 * KR)]);
}

#[test]
fn a_run_goes_open_finalized_booked_and_back_when_its_voucher_is_corrected() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let run = w.create(draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]));
    assert_eq!(w.status(run), PayrollRunStatus::Open);

    let mut changed = draft(date(2026, 10, 24), &[(asa, 36_000 * KR, 8_200 * KR)]);
    changed.text = "Lön okt".into();
    let event = update_payroll_run(&w.payroll(), run, changed).unwrap();
    w.then(event);
    assert_eq!(w.payroll().run(run).unwrap().draft.lines[0].gross, 36_000 * KR);

    w.finalize(run);
    assert_eq!(w.status(run), PayrollRunStatus::Finalized);
    assert!(w.payroll().run(run).unwrap().lines.is_some());

    let event = reopen_payroll_run(&w.payroll(), run).unwrap();
    w.then(event);
    assert_eq!(w.status(run), PayrollRunStatus::Open);
    assert_eq!(w.payroll().run(run).unwrap().lines, None);

    w.finalize(run);
    let first = w.book(run, 7);
    assert_eq!(w.status(run), PayrollRunStatus::Booked(first));
    assert_eq!(unbook_payroll_run(&w.payroll(), run), Ok(first));

    // The rättelse, whether from the payroll page or the grundbok.
    w.reversed.insert(first);
    assert_eq!(w.status(run), PayrollRunStatus::Finalized);

    let second = w.book(run, 9);
    assert_eq!(w.status(run), PayrollRunStatus::Booked(second));
    assert_eq!(w.payroll().run(run).unwrap().bookings, vec![first, second]);
}

#[test]
fn each_step_needs_the_right_status() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let d = || draft(date(2026, 10, 1), &[(asa, 35_000 * KR, 8_000 * KR)]);
    let today = date(2026, 10, 4);
    let missing = Uuid::new_v4();
    let p = w.payroll();
    assert_eq!(update_payroll_run(&p, missing, d()), Err(DomainError::PayrollRunNotFound));
    assert_eq!(finalize_payroll_run(&p, missing), Err(DomainError::PayrollRunNotFound));
    assert_eq!(reopen_payroll_run(&p, missing), Err(DomainError::PayrollRunNotFound));
    assert_eq!(book_payroll_run(&p, missing, today), Err(DomainError::PayrollRunNotFound));
    assert_eq!(unbook_payroll_run(&p, missing), Err(DomainError::PayrollRunNotFound));

    let run = w.create(d());
    let p = w.payroll();
    assert_eq!(reopen_payroll_run(&p, run), Err(DomainError::PayrollRunNotFinalized));
    assert_eq!(book_payroll_run(&p, run, today), Err(DomainError::PayrollRunNotFinalized));
    assert_eq!(unbook_payroll_run(&p, run), Err(DomainError::PayrollRunNotBooked));

    w.finalize(run);
    let p = w.payroll();
    assert_eq!(update_payroll_run(&p, run, d()), Err(DomainError::PayrollRunNotOpen));
    assert_eq!(finalize_payroll_run(&p, run), Err(DomainError::PayrollRunNotOpen));
    assert_eq!(unbook_payroll_run(&p, run), Err(DomainError::PayrollRunNotBooked));

    w.book(run, 1);
    let p = w.payroll();
    assert_eq!(update_payroll_run(&p, run, d()), Err(DomainError::PayrollRunBooked));
    assert_eq!(finalize_payroll_run(&p, run), Err(DomainError::PayrollRunBooked));
    assert_eq!(reopen_payroll_run(&p, run), Err(DomainError::PayrollRunBooked));
    assert_eq!(book_payroll_run(&p, run, today), Err(DomainError::PayrollRunBooked));
}

#[test]
fn a_run_is_booked_on_its_pay_date_with_the_locked_lines() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let mut d = draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]);
    d.text = "Lön oktober".into();
    let run = w.create(d);
    w.finalize(run);

    assert_eq!(
        book_payroll_run(&w.payroll(), run, date(2026, 10, 24)),
        Err(DomainError::PayrollRunNotDue)
    );

    // Moved to 7220 and deactivated after finalizing: the locked account
    // is booked, and nothing stops the booking.
    w.then(PayrollEvent::EmployeeUpdated {
        employee_id: asa,
        name: name("Åsa Öberg"),
        monthly_salary: 35_000 * KR,
        salary_account: SalaryAccount::parse(7220).unwrap(),
    });
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: asa });

    let voucher = book_payroll_run(&w.payroll(), run, date(2026, 10, 25)).unwrap();
    assert_eq!(voucher.date, date(2026, 10, 25));
    assert_eq!(voucher.text, "Lön oktober");
    assert_eq!(
        voucher.lines,
        vec![
            vl(7210, 35_000 * KR, 0),
            vl(2710, 0, 8_000 * KR),
            vl(1930, 0, 27_000 * KR),
            vl(7510, 1_099_700, 0),
            vl(2731, 0, 1_099_700),
        ]
    );
}

#[test]
fn an_open_run_with_a_deactivated_employee_cannot_be_finalized() {
    let mut w = World::default();
    let asa = w.hire("19800101-1231", 35_000 * KR);
    let run = w.create(draft(date(2026, 10, 25), &[(asa, 35_000 * KR, 8_000 * KR)]));
    w.then(PayrollEvent::EmployeeDeactivated { employee_id: asa });

    assert_eq!(finalize_payroll_run(&w.payroll(), run), Err(DomainError::EmployeeInactive));
}

#[test]
fn the_youth_cap_counts_only_booked_runs_in_the_same_month() {
    let mut w = World::default();
    let ung = w.hire("20050615-1232", 20_000 * KR);
    let oct = |day| date(2026, 10, day);
    let other = w.create(draft(oct(10), &[(ung, 20_000 * KR, 0)]));
    w.finalize(other);
    let d = draft(oct(25), &[(ung, 20_000 * KR, 0)]);
    // Finalized but not booked: not counted.
    assert_eq!(compute_lines(&w.payroll(), None, &d).unwrap()[0].fee, 416_200);

    let voucher = w.book(other, 1);
    // 5 000 kr left under the cap: 1 040,50 + 4 713 = 5 753,50 kr.
    assert_eq!(compute_lines(&w.payroll(), None, &d).unwrap()[0].fee, 575_350);
    // Another month is not counted.
    let nov = draft(date(2026, 11, 25), &[(ung, 20_000 * KR, 0)]);
    assert_eq!(compute_lines(&w.payroll(), None, &nov).unwrap()[0].fee, 416_200);

    w.reversed.insert(voucher);
    assert_eq!(compute_lines(&w.payroll(), None, &d).unwrap()[0].fee, 416_200);
}

#[test]
fn a_booking_that_would_change_the_fee_is_outdated() {
    let mut w = World::default();
    let ung = w.hire("20050615-1232", 20_000 * KR);
    let late = w.create(draft(date(2026, 10, 3), &[(ung, 20_000 * KR, 0)]));
    w.finalize(late); // Fee 4 162 kr: nothing booked in October yet.
    let early = w.create(draft(date(2026, 10, 1), &[(ung, 20_000 * KR, 0)]));
    w.finalize(early);
    w.book(early, 1);

    assert_eq!(
        book_payroll_run(&w.payroll(), late, date(2026, 10, 4)),
        Err(DomainError::PayrollRunOutdated)
    );

    // Reopened and finalized again, the fee is recomputed and it books.
    let event = reopen_payroll_run(&w.payroll(), late).unwrap();
    w.then(event);
    w.finalize(late);
    assert!(book_payroll_run(&w.payroll(), late, date(2026, 10, 4)).is_ok());
}
```

Check of the expected fees: 35 000 kr × 31,42 % = 10 997,00 kr (1 099 700 öre). 20 000 kr × 20,81 % = 4 162,00 kr (416 200 öre). 20 000 × 31,42 % = 6 284 kr (628 400 öre).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test domain`
Expected: FAIL to compile with "cannot find type `PayrollRunDraft`".

- [ ] **Step 3: Implement**

Add to `DomainError`, after `EmployeeInactive`:

```rust
    #[error("tax must be 0 up to the gross salary")]
    InvalidTax,
    #[error("text must be at most 200 characters")]
    InvalidText,
    #[error("a payroll run needs at least one employee")]
    EmptyPayrollRun,
    #[error("an employee appears twice in the payroll run")]
    DuplicatePayrollRunLine,
    #[error("no such payroll run")]
    PayrollRunNotFound,
    #[error("the payroll run is finalized")]
    PayrollRunNotOpen,
    #[error("the payroll run is not finalized")]
    PayrollRunNotFinalized,
    #[error("the payroll run is booked")]
    PayrollRunBooked,
    #[error("the payroll run is not booked")]
    PayrollRunNotBooked,
    #[error("the pay date has not come")]
    PayrollRunNotDue,
    #[error("the fees changed since the payroll run was finalized")]
    PayrollRunOutdated,
```

Add the imports `use doris_ledger::domain::{AccountNumber, RecordVoucher, VoucherLine};` and `use std::collections::BTreeMap;`.

Add the run variants to `PayrollEvent`, after `EmployeeDeactivated`:

```rust
    /// A new, open run.
    PayrollRunCreated {
        payroll_run_id: Uuid,
        draft: PayrollRunDraft,
    },
    /// The open run's contents replaced.
    PayrollRunUpdated {
        payroll_run_id: Uuid,
        draft: PayrollRunDraft,
    },
    /// Amounts, fees and accounts locked; may precede the pay date.
    PayrollRunFinalized {
        payroll_run_id: Uuid,
        lines: Vec<PayrollRunLine>,
    },
    /// Open again; the locked lines are dropped.
    PayrollRunReopened {
        payroll_run_id: Uuid,
    },
    /// Booked as a voucher, on or after the pay date.
    PayrollRunBooked {
        payroll_run_id: Uuid,
        voucher: BookedVoucher,
    },
```

Replace the `PayrollRun` stub with:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayrollRunDraft {
    pub pay_date: Date,
    pub text: String,
    pub lines: Vec<DraftLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftLine {
    pub employee_id: Uuid,
    pub gross: i64,
    pub tax: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollRunLine {
    pub employee_id: Uuid,
    /// Copied from the employee when finalized, so a later change of
    /// account doesn't move the booking.
    pub salary_account: SalaryAccount,
    pub gross: i64,
    pub tax: i64,
    /// Basis points of the part under the youth cap, see `employer_fee`.
    pub fee_rate: u32,
    pub fee: i64,
    pub net: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRun {
    pub id: Uuid,
    pub draft: PayrollRunDraft,
    /// Set while finalized (and booked).
    pub lines: Option<Vec<PayrollRunLine>>,
    /// Every booking ever made, oldest first.
    pub bookings: Vec<BookedVoucher>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayrollRunStatus {
    Open,
    Finalized,
    /// The voucher in force.
    Booked(BookedVoucher),
}
```

Add the run arms to `Payroll::apply`'s `match`:

```rust
            PayrollEvent::PayrollRunCreated {
                payroll_run_id,
                draft,
            } => self.runs.push(PayrollRun {
                id: payroll_run_id,
                draft,
                lines: None,
                bookings: vec![],
            }),
            PayrollEvent::PayrollRunUpdated {
                payroll_run_id,
                draft,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.draft = draft;
                }
            }
            PayrollEvent::PayrollRunFinalized {
                payroll_run_id,
                lines,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.lines = Some(lines);
                }
            }
            PayrollEvent::PayrollRunReopened { payroll_run_id } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.lines = None;
                }
            }
            PayrollEvent::PayrollRunBooked {
                payroll_run_id,
                voucher,
            } => {
                if let Some(run) = self.run_mut(payroll_run_id) {
                    run.bookings.push(voucher);
                }
            }
```

Add to `impl Payroll`:

```rust
    pub fn run(&self, id: Uuid) -> Option<&PayrollRun> {
        self.runs.iter().find(|r| r.id == id)
    }

    fn run_mut(&mut self, id: Uuid) -> Option<&mut PayrollRun> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    /// Booked while the latest booking's voucher is not corrected.
    pub fn status(&self, run: &PayrollRun) -> PayrollRunStatus {
        match run.bookings.last() {
            Some(voucher) if !self.reversed.contains(voucher) => PayrollRunStatus::Booked(*voucher),
            _ if run.lines.is_some() => PayrollRunStatus::Finalized,
            _ => PayrollRunStatus::Open,
        }
    }

    /// Gross paid to `employee_id` by booked runs other than `except` with
    /// a pay date in `pay_date`'s calendar month: what the youth cap counts.
    // ponytail: only booked runs count, and a booking whose fee would
    // change is refused (PayrollRunOutdated); fine while several runs a
    // month for an employee under 24 are rare.
    fn booked_gross(&self, employee_id: Uuid, pay_date: Date, except: Option<Uuid>) -> i64 {
        let same_month = |d: Date| (d.year(), d.month()) == (pay_date.year(), pay_date.month());
        self.runs
            .iter()
            .filter(|r| Some(r.id) != except && same_month(r.draft.pay_date))
            .filter(|r| matches!(self.status(r), PayrollRunStatus::Booked(_)))
            .flat_map(|r| r.lines.iter().flatten())
            .filter(|l| l.employee_id == employee_id)
            .map(|l| l.gross)
            .sum()
    }
```

Append the functions:

```rust
const MONTHS: [&str; 12] = [
    "januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september",
    "oktober", "november", "december",
];

/// The draft as it will be stored: lines checked, text trimmed or, when
/// empty, "Lön {månad} {år}". The pay date may be in the future.
pub fn validate_draft(
    payroll: &Payroll,
    draft: PayrollRunDraft,
) -> Result<PayrollRunDraft, DomainError> {
    if draft.lines.is_empty() {
        return Err(DomainError::EmptyPayrollRun);
    }
    let mut seen = HashSet::new();
    for line in &draft.lines {
        if !seen.insert(line.employee_id) {
            return Err(DomainError::DuplicatePayrollRunLine);
        }
        payroll.active_employee(line.employee_id)?;
        check_salary(line.gross)?;
        if !(0..=line.gross).contains(&line.tax) {
            return Err(DomainError::InvalidTax);
        }
    }
    let text = match draft.text.trim() {
        "" => format!(
            "Lön {} {}",
            MONTHS[draft.pay_date.month() as usize - 1],
            draft.pay_date.year()
        ),
        // The ledger's limit for a voucher text, refused before booking.
        text if text.chars().count() > 200 => return Err(DomainError::InvalidText),
        text => text.to_owned(),
    };
    Ok(PayrollRunDraft { text, ..draft })
}

/// A line's fee and net pay, the youth cap counting runs other than
/// `except`.
fn line(
    payroll: &Payroll,
    except: Option<Uuid>,
    pay_date: Date,
    employee: &Employee,
    gross: i64,
    tax: i64,
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
    }
}

/// The lines `draft` would lock if run `run_id` (or a new run, `None`)
/// were finalized now.
pub fn compute_lines(
    payroll: &Payroll,
    run_id: Option<Uuid>,
    draft: &PayrollRunDraft,
) -> Result<Vec<PayrollRunLine>, DomainError> {
    let draft = validate_draft(payroll, draft.clone())?;
    Ok(draft
        .lines
        .iter()
        .map(|l| {
            let employee = payroll.employee(l.employee_id).expect("validated");
            line(payroll, run_id, draft.pay_date, employee, l.gross, l.tax)
        })
        .collect())
}

/// The payroll voucher: gross on each salary account, tax on 2710, net
/// pay from 1930, the fee on 7510 against 2731. Zero lines are left out.
pub fn voucher_lines(lines: &[PayrollRunLine]) -> Vec<VoucherLine> {
    let mut salaries = BTreeMap::<u32, i64>::new();
    for l in lines {
        *salaries.entry(l.salary_account.get()).or_default() += l.gross;
    }
    let sum = |amount: fn(&PayrollRunLine) -> i64| lines.iter().map(amount).sum::<i64>();
    let (tax, net, fee) = (sum(|l| l.tax), sum(|l| l.net), sum(|l| l.fee));
    salaries
        .into_iter()
        .map(|(account, gross)| (account, gross, 0))
        .chain([(2710, 0, tax), (1930, 0, net), (7510, fee, 0), (2731, 0, fee)])
        .filter(|&(_, debit, credit)| debit != 0 || credit != 0)
        .map(|(account, debit, credit)| VoucherLine {
            account: AccountNumber::parse(account).expect("payroll accounts are in range"),
            debit,
            credit,
        })
        .collect()
}

fn find_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    payroll.run(id).ok_or(DomainError::PayrollRunNotFound)
}

fn open_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    let run = find_run(payroll, id)?;
    match payroll.status(run) {
        PayrollRunStatus::Open => Ok(run),
        PayrollRunStatus::Finalized => Err(DomainError::PayrollRunNotOpen),
        PayrollRunStatus::Booked(_) => Err(DomainError::PayrollRunBooked),
    }
}

fn finalized_run(payroll: &Payroll, id: Uuid) -> Result<&PayrollRun, DomainError> {
    let run = find_run(payroll, id)?;
    match payroll.status(run) {
        PayrollRunStatus::Open => Err(DomainError::PayrollRunNotFinalized),
        PayrollRunStatus::Finalized => Ok(run),
        PayrollRunStatus::Booked(_) => Err(DomainError::PayrollRunBooked),
    }
}

pub fn create_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<PayrollEvent, DomainError> {
    Ok(PayrollEvent::PayrollRunCreated {
        payroll_run_id,
        draft: validate_draft(payroll, draft)?,
    })
}

pub fn update_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<PayrollEvent, DomainError> {
    open_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunUpdated {
        payroll_run_id,
        draft: validate_draft(payroll, draft)?,
    })
}

pub fn finalize_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<PayrollEvent, DomainError> {
    let run = open_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunFinalized {
        payroll_run_id,
        lines: compute_lines(payroll, Some(payroll_run_id), &run.draft)?,
    })
}

pub fn reopen_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<PayrollEvent, DomainError> {
    finalized_run(payroll, payroll_run_id)?;
    Ok(PayrollEvent::PayrollRunReopened { payroll_run_id })
}

/// The voucher a finalized run books on `today`, no earlier than its pay
/// date. The locked lines are booked as they are; only their fees are
/// checked again, since another run booked or reversed meanwhile can move
/// the youth cap.
pub fn book_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<RecordVoucher, DomainError> {
    let run = finalized_run(payroll, payroll_run_id)?;
    let pay_date = run.draft.pay_date;
    if pay_date > today {
        return Err(DomainError::PayrollRunNotDue);
    }
    let locked = run.lines.as_deref().expect("a finalized run has lines");
    let fees_hold = locked.iter().all(|l| {
        let employee = payroll.employee(l.employee_id).expect("a run's employees exist");
        let fresh = line(payroll, Some(payroll_run_id), pay_date, employee, l.gross, l.tax);
        (fresh.fee_rate, fresh.fee) == (l.fee_rate, l.fee)
    });
    if !fees_hold {
        return Err(DomainError::PayrollRunOutdated);
    }
    Ok(RecordVoucher {
        date: pay_date,
        text: run.draft.text.clone(),
        lines: voucher_lines(locked),
    })
}

pub fn booked(payroll_run_id: Uuid, voucher: BookedVoucher) -> PayrollEvent {
    PayrollEvent::PayrollRunBooked {
        payroll_run_id,
        voucher,
    }
}

/// The voucher to correct to take a booked run back to Färdigställd.
pub fn unbook_payroll_run(
    payroll: &Payroll,
    payroll_run_id: Uuid,
) -> Result<BookedVoucher, DomainError> {
    match payroll.status(find_run(payroll, payroll_run_id)?) {
        PayrollRunStatus::Booked(voucher) => Ok(voucher),
        _ => Err(DomainError::PayrollRunNotBooked),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-payroll --test domain`
Expected: PASS. If `RecordVoucher` or `VoucherLine` lacks `PartialEq`/`Debug` for the asserts, check `crates/ledger/src/domain.rs`: both already derive them (`RecordVoucher` at line ~607, `VoucherLine` at ~273).

- [ ] **Step 5: Commit**

```bash
git add crates/payroll
git commit -m "Add payroll runs: drafts, fees, voucher lines and the open/finalized/booked lifecycle

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 5: Employee storage: migration, write flow, projection and list

**Files:**
- Create: `migrations/0010_payroll.sql`
- Modify: `crates/payroll/src/lib.rs`
- Create: `crates/payroll/src/projections.rs`
- Create: `crates/payroll/src/queries.rs`
- Test: `crates/payroll/tests/store.rs`

**Interfaces:**
- Consumes: Task 3's employee decisions. `doris_company::get_company_in(&mut SqliteConnection, Uuid, Uuid)` and `doris_company::get_company(&SqlitePool, Uuid, Uuid)`, which give `doris_company::Error::NotFound` for a non-member.
- Produces:
  - `doris_payroll::Error { Domain(DomainError), NotFound, Ledger(doris_ledger::Error), Store(doris_eventstore::Error) }` and `doris_payroll::Result<T>`.
  - `pub struct NewEmployee<'a> { pub name: &'a str, pub personal_identity_number: &'a str, pub monthly_salary: i64, pub salary_account: u32 }`.
  - `pub async fn add_employee(&SqlitePool, company_id: Uuid, actor: Uuid, NewEmployee<'_>) -> Result<Uuid>`.
  - `pub async fn update_employee(&SqlitePool, company_id: Uuid, actor: Uuid, employee_id: Uuid, name: &str, monthly_salary: i64, salary_account: u32) -> Result<()>`.
  - `pub async fn deactivate_employee(&SqlitePool, company_id: Uuid, actor: Uuid, employee_id: Uuid) -> Result<()>`.
  - `pub async fn list_employees(&SqlitePool, company_id: Uuid, actor: Uuid) -> Result<Vec<domain::Employee>>`, sorted by name.
  - `pub async fn rebuild_projections(&SqlitePool) -> Result<()>`.
  - Crate-internal: `load(conn, company_id, actor) -> Result<(Company, Payroll, i64)>`, `append(..)`, `change(..)`, `reversed_vouchers(..)`. Tasks 6 and 7 use them.

- [ ] **Step 1: Write the migration**

`migrations/0010_payroll.sql`:

```sql
-- Projections of the payroll-{company_id} streams. Rebuildable from events.
-- No foreign key to companies or vouchers: each crate rebuilds its own
-- tables independently (doris_ledger::rebuild_projections empties vouchers).

CREATE TABLE employees (
    company_id               TEXT    NOT NULL,
    employee_id              TEXT    NOT NULL,
    name                     TEXT    NOT NULL,
    personal_identity_number TEXT    NOT NULL,
    monthly_salary           INTEGER NOT NULL,
    salary_account           INTEGER NOT NULL,
    active                   INTEGER NOT NULL,
    PRIMARY KEY (company_id, employee_id),
    -- A personnummer once per company, also among inactive employees.
    UNIQUE (company_id, personal_identity_number)
);

CREATE TABLE payroll_runs (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    pay_date       TEXT    NOT NULL,
    text           TEXT    NOT NULL,
    finalized      INTEGER NOT NULL, -- 1 between Finalized and Reopened
    updated_at     TEXT    NOT NULL,
    updated_by     TEXT    NOT NULL,
    PRIMARY KEY (company_id, payroll_run_id)
);

-- The draft's lines while open; the locked lines (with account and fee)
-- once finalized.
CREATE TABLE payroll_run_lines (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER NOT NULL,
    salary_account INTEGER, -- NULL while open
    fee_rate       INTEGER,
    fee            INTEGER,
    net            INTEGER,
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);

-- Every booking ever made, in order (rowid); the latest one is in force
-- unless a voucher corrects it.
CREATE TABLE payroll_run_bookings (
    company_id        TEXT    NOT NULL,
    payroll_run_id    TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    voucher_number    INTEGER NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, voucher_number),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
```

- [ ] **Step 2: Write the failing tests**

`crates/payroll/tests/store.rs`:

```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_payroll::domain::DomainError;
use doris_payroll::{
    Error, NewEmployee, add_employee, deactivate_employee, list_employees, rebuild_projections,
    update_employee,
};
use sqlx::SqlitePool;
use uuid::Uuid;

const KR: i64 = 100;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// A company whose first räkenskapsår is 2025, with `owner` as member.
async fn company(pool: &SqlitePool, owner: Uuid) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: "2025-01-01".parse().unwrap(),
            fiscal_year_end: "2025-12-31".parse().unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap()
}

fn asa() -> NewEmployee<'static> {
    NewEmployee {
        name: "Åsa Öberg",
        personal_identity_number: "19800101-1231",
        monthly_salary: 35_000 * KR,
        salary_account: 7210,
    }
}

async fn events_of(pool: &SqlitePool, prefix: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT event_type FROM events WHERE stream_id LIKE ? ORDER BY global_position",
    )
    .bind(format!("{prefix}%"))
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn table(pool: &SqlitePool, sql: &'static str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn employees_are_added_updated_deactivated_and_listed_by_name() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let bo = NewEmployee {
        name: "Bo Ek",
        personal_identity_number: "198507099870",
        monthly_salary: 30_000 * KR,
        salary_account: 7010,
    };
    let bo_id = add_employee(&pool, id, anna, bo).await.unwrap();
    update_employee(&pool, id, anna, asa_id, "Åsa Öberg Lind", 36_000 * KR, 7220)
        .await
        .unwrap();
    deactivate_employee(&pool, id, anna, bo_id).await.unwrap();

    let employees = list_employees(&pool, id, anna).await.unwrap();
    let rows: Vec<_> = employees
        .iter()
        .map(|e| {
            (
                e.name.as_str(),
                e.personal_identity_number.formatted(),
                e.monthly_salary,
                e.salary_account.get(),
                e.active,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Bo Ek", "19850709-9870".to_owned(), 30_000 * KR, 7010, false),
            ("Åsa Öberg Lind", "19800101-1231".to_owned(), 36_000 * KR, 7220, true),
        ]
    );
    assert_eq!(
        events_of(&pool, "payroll-").await,
        ["EmployeeAdded", "EmployeeAdded", "EmployeeUpdated", "EmployeeDeactivated"]
    );
}

#[tokio::test]
async fn invalid_or_duplicate_employees_are_refused_and_write_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    add_employee(&pool, id, anna, asa()).await.unwrap();

    let refused = |e: Error| match e {
        Error::Domain(e) => e,
        other => panic!("{other:?}"),
    };
    let again = add_employee(&pool, id, anna, asa()).await.unwrap_err();
    assert_eq!(refused(again), DomainError::DuplicateEmployee);
    let bad_pin = NewEmployee {
        personal_identity_number: "19800101-1232",
        ..asa()
    };
    assert_eq!(
        refused(add_employee(&pool, id, anna, bad_pin).await.unwrap_err()),
        DomainError::InvalidPersonalIdentityNumber
    );
    let bad_account = NewEmployee {
        personal_identity_number: "19850709-9870",
        salary_account: 7510,
        ..asa()
    };
    assert_eq!(
        refused(add_employee(&pool, id, anna, bad_account).await.unwrap_err()),
        DomainError::InvalidSalaryAccount
    );
    assert_eq!(
        refused(
            update_employee(&pool, id, anna, Uuid::new_v4(), "X", 1, 7210)
                .await
                .unwrap_err()
        ),
        DomainError::EmployeeNotFound
    );

    assert_eq!(events_of(&pool, "payroll-").await, ["EmployeeAdded"]);
}

#[tokio::test]
async fn the_database_refuses_a_duplicate_personnummer() {
    let pool = db().await;
    let insert = "INSERT INTO employees (company_id, employee_id, name, personal_identity_number,
                  monthly_salary, salary_account, active) VALUES ('c', ?, 'X', '198001011231', 1, 7210, 1)";
    sqlx::query(insert).bind("a").execute(&pool).await.unwrap();
    assert!(sqlx::query(insert).bind("b").execute(&pool).await.is_err());
}

#[tokio::test]
async fn non_members_get_not_found() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let eve = Uuid::new_v4();

    assert!(matches!(add_employee(&pool, id, eve, asa()).await, Err(Error::NotFound)));
    assert!(matches!(list_employees(&pool, id, eve).await, Err(Error::NotFound)));
    assert!(matches!(
        list_employees(&pool, Uuid::new_v4(), anna).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_employee_projection_rebuilds_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    update_employee(&pool, id, anna, asa_id, "Åsa Lind", 36_000 * KR, 7220)
        .await
        .unwrap();
    deactivate_employee(&pool, id, anna, asa_id).await.unwrap();
    let sql = "SELECT company_id || employee_id || name || personal_identity_number
               || monthly_salary || salary_account || active FROM employees ORDER BY 1";
    let before = table(&pool, sql).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(table(&pool, sql).await, before);
    assert_eq!(before.len(), 1);
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test store`
Expected: FAIL to compile with "unresolved imports `doris_payroll::Error`, `doris_payroll::NewEmployee`…".

- [ ] **Step 4: Implement**

Replace `crates/payroll/src/lib.rs` with:

```rust
//! Employees and payroll runs (lönekörningar) of a company, event-sourced
//! into SQLite. A run is booked as a ledger voucher in the same
//! transaction as its event.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, read
//! the corrected vouchers, load the company's payroll, decide, append,
//! project.

pub mod domain;
mod projections;
mod queries;

use doris_company::domain::Company;
use doris_eventstore::{Metadata, NewEvent};
use domain::{
    AddEmployee, BookedVoucher, DomainError, EmployeeName, Payroll, PayrollEvent,
    PersonalIdentityNumber, SalaryAccount, UpdateEmployee,
};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::list_employees;

const PAYROLL_STREAM: &str = "payroll-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// The ledger refused the voucher (closed year, inactive account, …).
    #[error(transparent)]
    Ledger(#[from] doris_ledger::Error),
    #[error(transparent)]
    Store(#[from] doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<doris_company::Error> for Error {
    fn from(err: doris_company::Error) -> Self {
        match err {
            doris_company::Error::Store(err) => Error::Store(err),
            _ => Error::NotFound,
        }
    }
}

/// Unvalidated input for a new employee, as it arrives from the API.
#[derive(Debug, Clone)]
pub struct NewEmployee<'a> {
    pub name: &'a str,
    pub personal_identity_number: &'a str,
    pub monthly_salary: i64,
    pub salary_account: u32,
}

pub async fn add_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    new: NewEmployee<'_>,
) -> Result<Uuid> {
    let cmd = AddEmployee {
        employee_id: Uuid::new_v4(),
        name: EmployeeName::parse(new.name)?,
        personal_identity_number: PersonalIdentityNumber::parse(new.personal_identity_number)?,
        monthly_salary: new.monthly_salary,
        salary_account: SalaryAccount::parse(new.salary_account)?,
    };
    let employee_id = cmd.employee_id;
    change(pool, company_id, actor, |payroll| {
        domain::add_employee(payroll, cmd)
    })
    .await?;
    Ok(employee_id)
}

pub async fn update_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
    name: &str,
    monthly_salary: i64,
    salary_account: u32,
) -> Result<()> {
    let cmd = UpdateEmployee {
        employee_id,
        name: EmployeeName::parse(name)?,
        monthly_salary,
        salary_account: SalaryAccount::parse(salary_account)?,
    };
    change(pool, company_id, actor, |payroll| {
        domain::update_employee(payroll, cmd)
    })
    .await
}

pub async fn deactivate_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        domain::deactivate_employee(payroll, employee_id)
    })
    .await
}

/// One write of payroll events, decided on the company's current payroll.
async fn change(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Payroll) -> Result<Vec<PayrollEvent>, DomainError>,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let events = decide(&payroll)?;
    append(&mut tx, company_id, version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

fn payroll_stream(company_id: Uuid) -> String {
    format!("{PAYROLL_STREAM}{company_id}")
}

/// The company (checking membership), its payroll and the stream version.
async fn load(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
) -> Result<(Company, Payroll, i64)> {
    let company = doris_company::get_company_in(conn, company_id, actor).await?;
    let reversed = reversed_vouchers(conn, company_id).await?;
    let recorded = doris_eventstore::load(conn, &payroll_stream(company_id)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<PayrollEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((company, Payroll::from_events(&events, reversed), version))
}

/// The company's vouchers that a rättelse points at, from the ledger's
/// projection. A rättelse is always in its original's fiscal year.
async fn reversed_vouchers(
    conn: &mut SqliteConnection,
    company_id: Uuid,
) -> Result<HashSet<BookedVoucher>> {
    let rows: Vec<(String, u32)> = sqlx::query_as(
        "SELECT fiscal_year_start, corrects FROM vouchers
         WHERE company_id = ? AND corrects IS NOT NULL",
    )
    .bind(company_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(start, number)| BookedVoucher {
            fiscal_year_start: start.parse().expect("the ledger stores YYYY-MM-DD"),
            number,
        })
        .collect())
}

/// Appends events and updates projections within the caller's transaction.
async fn append(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    expected_version: i64,
    events: &[PayrollEvent],
    actor: Uuid,
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: Some(actor.to_string()),
    };
    let stream = payroll_stream(company_id);
    let recorded =
        doris_eventstore::append(conn, &stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
```

`crates/payroll/src/projections.rs`:

```rust
//! Read models for employees and payroll runs. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::PAYROLL_STREAM;
use crate::domain::PayrollEvent;
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    let Some(company_id) = event.stream_id.strip_prefix(PAYROLL_STREAM) else {
        return Ok(());
    };
    match event.decode()? {
        PayrollEvent::EmployeeAdded {
            employee_id,
            name,
            personal_identity_number,
            monthly_salary,
            salary_account,
        } => {
            sqlx::query(
                "INSERT INTO employees (company_id, employee_id, name, personal_identity_number,
                     monthly_salary, salary_account, active)
                 VALUES (?, ?, ?, ?, ?, ?, 1)",
            )
            .bind(company_id)
            .bind(employee_id.to_string())
            .bind(name.as_str())
            .bind(personal_identity_number.as_str())
            .bind(monthly_salary)
            .bind(salary_account.get())
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::EmployeeUpdated {
            employee_id,
            name,
            monthly_salary,
            salary_account,
        } => {
            sqlx::query(
                "UPDATE employees SET name = ?, monthly_salary = ?, salary_account = ?
                 WHERE company_id = ? AND employee_id = ?",
            )
            .bind(name.as_str())
            .bind(monthly_salary)
            .bind(salary_account.get())
            .bind(company_id)
            .bind(employee_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::EmployeeDeactivated { employee_id } => {
            sqlx::query("UPDATE employees SET active = 0 WHERE company_id = ? AND employee_id = ?")
                .bind(company_id)
                .bind(employee_id.to_string())
                .execute(&mut *conn)
                .await?;
        }
        // Projected by Task 6 of the plan, which replaces this arm.
        PayrollEvent::PayrollRunCreated { .. }
        | PayrollEvent::PayrollRunUpdated { .. }
        | PayrollEvent::PayrollRunFinalized { .. }
        | PayrollEvent::PayrollRunReopened { .. }
        | PayrollEvent::PayrollRunBooked { .. } => {}
    }
    Ok(())
}

/// Empties the payroll projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in [
        "DELETE FROM payroll_run_bookings",
        "DELETE FROM payroll_run_lines",
        "DELETE FROM payroll_runs",
        "DELETE FROM employees",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

`crates/payroll/src/queries.rs`:

```rust
//! Reads over the payroll projections. Every read checks membership first.

use crate::Result;
use crate::domain::{Employee, EmployeeName, PersonalIdentityNumber, SalaryAccount};
use sqlx::SqlitePool;
use uuid::Uuid;

/// All employees, inactive too, by name.
pub async fn list_employees(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Employee>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(String, String, String, i64, u32, bool)> = sqlx::query_as(
        "SELECT employee_id, name, personal_identity_number, monthly_salary, salary_account, active
         FROM employees WHERE company_id = ? ORDER BY name, employee_id",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, name, pin, monthly_salary, account, active)| {
            Ok(Employee {
                id: id.parse().expect("stored uuids parse"),
                name: EmployeeName::parse(&name)?,
                personal_identity_number: PersonalIdentityNumber::parse(&pin)?,
                monthly_salary,
                salary_account: SalaryAccount::parse(account)?,
                active,
            })
        })
        .collect()
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-payroll`
Expected: PASS (domain and store).

- [ ] **Step 6: Commit**

```bash
git add migrations/0010_payroll.sql crates/payroll
git commit -m "Store employees with a projection that rebuilds from events

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 6: Payroll run storage: create, update, finalize, reopen, preview and read

**Files:**
- Modify: `crates/payroll/src/lib.rs`
- Modify: `crates/payroll/src/projections.rs` (replace the placeholder arm)
- Modify: `crates/payroll/src/queries.rs`
- Test: `crates/payroll/tests/store.rs`

**Interfaces:**
- Consumes: Task 4's run decisions, Task 5's `change`, `load`, `append`.
- Produces:
  - `pub struct Preview { pub text: String, pub lines: Vec<domain::PayrollRunLine> }` and `pub async fn preview_payroll_run(&SqlitePool, company_id: Uuid, actor: Uuid, domain::PayrollRunDraft) -> Result<Preview>`.
  - `pub async fn create_payroll_run(&SqlitePool, Uuid, Uuid, PayrollRunDraft) -> Result<Uuid>` and `pub async fn update_payroll_run(&SqlitePool, Uuid, Uuid, payroll_run_id: Uuid, PayrollRunDraft) -> Result<()>`.
  - `pub async fn finalize_payroll_run(&SqlitePool, Uuid, Uuid, payroll_run_id: Uuid) -> Result<()>` and `pub async fn reopen_payroll_run(&SqlitePool, Uuid, Uuid, payroll_run_id: Uuid) -> Result<()>`.
  - `pub struct PayrollRunView { pub id: Uuid, pub pay_date: Date, pub text: String, pub status: PayrollRunStatus, pub lines: Vec<PayrollRunLineView> }` with `fn voucher_lines(&self) -> Vec<doris_ledger::domain::VoucherLine>`, which is empty while the run is open.
  - `pub struct PayrollRunLineView { pub employee_id: Uuid, pub employee_name: String, pub gross: i64, pub tax: i64, pub locked: Option<PayrollRunLine> }`.
  - `pub async fn list_payroll_runs(&SqlitePool, Uuid, Uuid) -> Result<Vec<PayrollRunView>>`, newest pay date first, and `pub async fn get_payroll_run(&SqlitePool, Uuid, Uuid, payroll_run_id: Uuid) -> Result<PayrollRunView>`, which gives `Domain(PayrollRunNotFound)` for a run that doesn't exist.

- [ ] **Step 1: Write the failing tests** — append to `crates/payroll/tests/store.rs`, and extend the `use doris_payroll::{…}` list with `create_payroll_run, finalize_payroll_run, get_payroll_run, list_payroll_runs, preview_payroll_run, reopen_payroll_run, update_payroll_run`:

```rust
use doris_payroll::domain::{
    DraftLine, FULL_RATE, PayrollRunDraft, PayrollRunLine, PayrollRunStatus, SalaryAccount,
};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn draft(pay_date: &str, lines: &[(Uuid, i64, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date: d(pay_date),
        text: String::new(),
        lines: lines
            .iter()
            .map(|&(employee_id, gross, tax)| DraftLine { employee_id, gross, tax })
            .collect(),
    }
}

#[tokio::test]
async fn a_preview_computes_the_lines_and_writes_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();

    let preview = preview_payroll_run(
        &pool,
        id,
        anna,
        draft("2026-10-25", &[(asa_id, 35_000 * KR, 8_000 * KR)]),
    )
    .await
    .unwrap();

    assert_eq!(preview.text, "Lön oktober 2026");
    assert_eq!(
        preview.lines,
        [PayrollRunLine {
            employee_id: asa_id,
            salary_account: SalaryAccount::DEFAULT,
            gross: 35_000 * KR,
            tax: 8_000 * KR,
            fee_rate: FULL_RATE,
            fee: 1_099_700,
            net: 27_000 * KR,
        }]
    );
    assert_eq!(events_of(&pool, "payroll-").await, ["EmployeeAdded"]);
}

#[tokio::test]
async fn a_run_is_created_changed_finalized_and_reopened() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();

    let run = create_payroll_run(&pool, id, anna, draft("2026-10-25", &[(asa_id, 35_000 * KR, 8_000 * KR)]))
        .await
        .unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Open);
    assert_eq!(view.text, "Lön oktober 2026");
    assert_eq!(view.lines[0].employee_name, "Åsa Öberg");
    assert_eq!((view.lines[0].gross, view.lines[0].tax, view.lines[0].locked), (35_000 * KR, 8_000 * KR, None));
    assert!(view.voucher_lines().is_empty());

    update_payroll_run(&pool, id, anna, run, draft("2026-10-24", &[(asa_id, 36_000 * KR, 8_300 * KR)]))
        .await
        .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Finalized);
    assert_eq!(view.pay_date, d("2026-10-24"));
    let locked = view.lines[0].locked.unwrap();
    assert_eq!((locked.gross, locked.fee, locked.net), (36_000 * KR, 1_131_120, 27_700 * KR));
    assert_eq!(view.voucher_lines().len(), 5);

    // Finalized: no changes until it is opened again.
    let refused = update_payroll_run(&pool, id, anna, run, draft("2026-10-24", &[(asa_id, 1, 0)]))
        .await
        .unwrap_err();
    assert!(matches!(refused, Error::Domain(DomainError::PayrollRunNotOpen)));

    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Open);
    assert_eq!(view.lines[0].locked, None);
    assert_eq!(view.lines[0].gross, 36_000 * KR);

    assert_eq!(
        events_of(&pool, "payroll-").await,
        [
            "EmployeeAdded",
            "PayrollRunCreated",
            "PayrollRunUpdated",
            "PayrollRunFinalized",
            "PayrollRunReopened"
        ]
    );
}

#[tokio::test]
async fn runs_are_listed_newest_pay_date_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    for pay_date in ["2026-09-25", "2026-11-25", "2026-10-25"] {
        create_payroll_run(&pool, id, anna, draft(pay_date, &[(asa_id, 100, 0)]))
            .await
            .unwrap();
    }

    let runs = list_payroll_runs(&pool, id, anna).await.unwrap();

    let dates: Vec<_> = runs.iter().map(|r| r.pay_date.to_string()).collect();
    assert_eq!(dates, ["2026-11-25", "2026-10-25", "2026-09-25"]);
}

#[tokio::test]
async fn a_missing_run_or_a_stranger_finds_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let run = create_payroll_run(&pool, id, anna, draft("2026-10-25", &[(asa_id, 100, 0)]))
        .await
        .unwrap();
    let eve = Uuid::new_v4();

    assert!(matches!(
        get_payroll_run(&pool, id, anna, Uuid::new_v4()).await,
        Err(Error::Domain(DomainError::PayrollRunNotFound))
    ));
    assert!(matches!(get_payroll_run(&pool, id, eve, run).await, Err(Error::NotFound)));
    assert!(matches!(list_payroll_runs(&pool, id, eve).await, Err(Error::NotFound)));
    assert!(matches!(finalize_payroll_run(&pool, id, eve, run).await, Err(Error::NotFound)));
    assert!(matches!(
        preview_payroll_run(&pool, id, eve, draft("2026-10-25", &[(asa_id, 100, 0)])).await,
        Err(Error::NotFound)
    ));
}
```

Check of the fee: 36 000 kr × 31,42 % = 11 311,20 kr (1 131 120 öre). Net is 36 000 − 8 300 = 27 700 kr.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test store`
Expected: FAIL to compile with "unresolved imports `doris_payroll::create_payroll_run`…".

- [ ] **Step 3: Implement the writes** — append to `crates/payroll/src/lib.rs`, extend the `domain::{…}` import with `PayrollRunDraft, PayrollRunLine`, and replace the `pub use queries::…` line with `pub use queries::{PayrollRunLineView, PayrollRunView, get_payroll_run, list_employees, list_payroll_runs};`

```rust
/// What a draft would lock if finalized now.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub text: String,
    pub lines: Vec<PayrollRunLine>,
}

/// Computes a draft's lines without writing anything.
pub async fn preview_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    draft: PayrollRunDraft,
) -> Result<Preview> {
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    let draft = domain::validate_draft(&payroll, draft)?;
    let lines = domain::compute_lines(&payroll, None, &draft)?;
    Ok(Preview {
        text: draft.text,
        lines,
    })
}

pub async fn create_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    draft: PayrollRunDraft,
) -> Result<Uuid> {
    let payroll_run_id = Uuid::new_v4();
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::create_payroll_run(payroll, payroll_run_id, draft)?])
    })
    .await?;
    Ok(payroll_run_id)
}

pub async fn update_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::update_payroll_run(payroll, payroll_run_id, draft)?])
    })
    .await
}

pub async fn finalize_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::finalize_payroll_run(payroll, payroll_run_id)?])
    })
    .await
}

pub async fn reopen_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::reopen_payroll_run(payroll, payroll_run_id)?])
    })
    .await
}
```

- [ ] **Step 4: Implement the projection** — in `crates/payroll/src/projections.rs`, add `use uuid::Uuid;` and `use crate::domain::{DraftLine, PayrollRunLine};`, and replace the placeholder arm with:

```rust
        PayrollEvent::PayrollRunCreated {
            payroll_run_id,
            draft,
        } => {
            let run = payroll_run_id.to_string();
            sqlx::query(
                "INSERT INTO payroll_runs (company_id, payroll_run_id, pay_date, text, finalized,
                     updated_at, updated_by)
                 VALUES (?, ?, ?, ?, 0, ?, ?)",
            )
            .bind(company_id)
            .bind(&run)
            .bind(draft.pay_date.to_string())
            .bind(&draft.text)
            .bind(&event.recorded_at)
            .bind(actor(event))
            .execute(&mut *conn)
            .await?;
            insert_draft_lines(conn, company_id, &run, &draft.lines).await?;
        }
        PayrollEvent::PayrollRunUpdated {
            payroll_run_id,
            draft,
        } => {
            let run = payroll_run_id.to_string();
            sqlx::query(
                "UPDATE payroll_runs SET pay_date = ?, text = ?, updated_at = ?, updated_by = ?
                 WHERE company_id = ? AND payroll_run_id = ?",
            )
            .bind(draft.pay_date.to_string())
            .bind(&draft.text)
            .bind(&event.recorded_at)
            .bind(actor(event))
            .bind(company_id)
            .bind(&run)
            .execute(&mut *conn)
            .await?;
            delete_lines(conn, company_id, &run).await?;
            insert_draft_lines(conn, company_id, &run, &draft.lines).await?;
        }
        PayrollEvent::PayrollRunFinalized {
            payroll_run_id,
            lines,
        } => {
            let run = payroll_run_id.to_string();
            set_finalized(conn, company_id, &run, true, event).await?;
            delete_lines(conn, company_id, &run).await?;
            for line in &lines {
                insert_locked_line(conn, company_id, &run, line).await?;
            }
        }
        PayrollEvent::PayrollRunReopened { payroll_run_id } => {
            let run = payroll_run_id.to_string();
            set_finalized(conn, company_id, &run, false, event).await?;
            sqlx::query(
                "UPDATE payroll_run_lines
                 SET salary_account = NULL, fee_rate = NULL, fee = NULL, net = NULL
                 WHERE company_id = ? AND payroll_run_id = ?",
            )
            .bind(company_id)
            .bind(&run)
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::PayrollRunBooked {
            payroll_run_id,
            voucher,
        } => {
            sqlx::query(
                "INSERT INTO payroll_run_bookings (company_id, payroll_run_id, fiscal_year_start,
                     voucher_number)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(payroll_run_id.to_string())
            .bind(voucher.fiscal_year_start.to_string())
            .bind(voucher.number)
            .execute(&mut *conn)
            .await?;
        }
```

And add these helpers below `apply`:

```rust
fn actor(event: &RecordedEvent) -> &str {
    event.metadata.actor.as_deref().unwrap_or_default()
}

async fn set_finalized(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    finalized: bool,
    event: &RecordedEvent,
) -> crate::Result<()> {
    sqlx::query(
        "UPDATE payroll_runs SET finalized = ?, updated_at = ?, updated_by = ?
         WHERE company_id = ? AND payroll_run_id = ?",
    )
    .bind(finalized)
    .bind(&event.recorded_at)
    .bind(actor(event))
    .bind(company_id)
    .bind(run)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn delete_lines(conn: &mut SqliteConnection, company_id: &str, run: &str) -> crate::Result<()> {
    sqlx::query("DELETE FROM payroll_run_lines WHERE company_id = ? AND payroll_run_id = ?")
        .bind(company_id)
        .bind(run)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn insert_draft_lines(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    lines: &[DraftLine],
) -> crate::Result<()> {
    for line in lines {
        sqlx::query(
            "INSERT INTO payroll_run_lines (company_id, payroll_run_id, employee_id, gross, tax)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(company_id)
        .bind(run)
        .bind(line.employee_id.to_string())
        .bind(line.gross)
        .bind(line.tax)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_locked_line(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    line: &PayrollRunLine,
) -> crate::Result<()> {
    sqlx::query(
        "INSERT INTO payroll_run_lines (company_id, payroll_run_id, employee_id, gross, tax,
             salary_account, fee_rate, fee, net)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(company_id)
    .bind(run)
    .bind(line.employee_id.to_string())
    .bind(line.gross)
    .bind(line.tax)
    .bind(line.salary_account.get())
    .bind(line.fee_rate)
    .bind(line.fee)
    .bind(line.net)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
```

If `use uuid::Uuid;` ends up unused in `projections.rs`, drop it; the arms call `.to_string()` on the ids directly.

- [ ] **Step 5: Implement the reads** — append to `crates/payroll/src/queries.rs`, and replace its `use crate::domain::{…};` line with these imports (keep `use crate::Result;`, `use sqlx::SqlitePool;` and `use uuid::Uuid;`):

```rust
use crate::domain::{
    self, BookedVoucher, DomainError, Employee, EmployeeName, PayrollRunLine, PayrollRunStatus,
    PersonalIdentityNumber, SalaryAccount,
};
use doris_ledger::domain::VoucherLine;
use jiff::civil::Date;
use std::collections::HashMap;
```

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRunView {
    pub id: Uuid,
    pub pay_date: Date,
    pub text: String,
    pub status: PayrollRunStatus,
    pub lines: Vec<PayrollRunLineView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRunLineView {
    pub employee_id: Uuid,
    pub employee_name: String,
    pub gross: i64,
    pub tax: i64,
    /// Set while the run is finalized or booked.
    pub locked: Option<PayrollRunLine>,
}

impl PayrollRunView {
    /// The voucher the run books; empty while it is open.
    pub fn voucher_lines(&self) -> Vec<VoucherLine> {
        let locked: Option<Vec<PayrollRunLine>> = self.lines.iter().map(|l| l.locked).collect();
        locked.map_or_else(Vec::new, |lines| domain::voucher_lines(&lines))
    }
}

/// Every payroll run, newest pay date first. A run is booked while its
/// latest booking's voucher has no rättelse (the same rule as
/// `Payroll::status`).
pub async fn list_payroll_runs(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<PayrollRunView>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let company = company_id.to_string();
    // ponytail: every run of the company in one list; filter by year or
    // page when a company has several years of runs.
    type Head = (String, String, String, bool, Option<String>, Option<u32>, bool);
    // One read transaction: heads and lines from the same snapshot (WAL).
    let mut tx = pool.begin().await?;
    let heads: Vec<Head> = sqlx::query_as(
        "SELECT r.payroll_run_id, r.pay_date, r.text, r.finalized,
                b.fiscal_year_start, b.voucher_number,
                EXISTS (SELECT 1 FROM vouchers v
                        WHERE v.company_id = b.company_id
                          AND v.fiscal_year_start = b.fiscal_year_start
                          AND v.corrects = b.voucher_number)
         FROM payroll_runs r
         LEFT JOIN payroll_run_bookings b ON b.rowid = (
             SELECT MAX(x.rowid) FROM payroll_run_bookings x
             WHERE x.company_id = r.company_id AND x.payroll_run_id = r.payroll_run_id)
         WHERE r.company_id = ?
         ORDER BY r.pay_date DESC, r.rowid DESC",
    )
    .bind(&company)
    .fetch_all(&mut *tx)
    .await?;
    type Line = (String, String, String, i64, i64, Option<u32>, Option<u32>, Option<i64>, Option<i64>);
    let rows: Vec<Line> = sqlx::query_as(
        "SELECT l.payroll_run_id, l.employee_id, e.name, l.gross, l.tax,
                l.salary_account, l.fee_rate, l.fee, l.net
         FROM payroll_run_lines l
         JOIN employees e ON e.company_id = l.company_id AND e.employee_id = l.employee_id
         WHERE l.company_id = ?
         ORDER BY e.name, l.employee_id",
    )
    .bind(&company)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut lines = HashMap::<String, Vec<PayrollRunLineView>>::new();
    for (run, employee, name, gross, tax, account, fee_rate, fee, net) in rows {
        let employee_id: Uuid = employee.parse().expect("stored uuids parse");
        let locked = match (account, fee_rate, fee, net) {
            (Some(account), Some(fee_rate), Some(fee), Some(net)) => Some(PayrollRunLine {
                employee_id,
                salary_account: SalaryAccount::parse(account)?,
                gross,
                tax,
                fee_rate,
                fee,
                net,
            }),
            _ => None,
        };
        lines.entry(run).or_default().push(PayrollRunLineView {
            employee_id,
            employee_name: name,
            gross,
            tax,
            locked,
        });
    }
    Ok(heads
        .into_iter()
        .map(|(id, pay_date, text, finalized, start, number, reversed)| {
            let status = match (start, number) {
                (Some(start), Some(number)) if !reversed => PayrollRunStatus::Booked(BookedVoucher {
                    fiscal_year_start: start.parse().expect("stored dates parse"),
                    number,
                }),
                _ if finalized => PayrollRunStatus::Finalized,
                _ => PayrollRunStatus::Open,
            };
            PayrollRunView {
                lines: lines.remove(&id).unwrap_or_default(),
                id: id.parse().expect("stored uuids parse"),
                pay_date: pay_date.parse().expect("stored dates parse"),
                text,
                status,
            }
        })
        .collect())
}

pub async fn get_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    payroll_run_id: Uuid,
) -> Result<PayrollRunView> {
    // ponytail: reads the whole list; a single-run query when lists grow.
    list_payroll_runs(pool, company_id, user_id)
        .await?
        .into_iter()
        .find(|r| r.id == payroll_run_id)
        .ok_or_else(|| DomainError::PayrollRunNotFound.into())
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-payroll`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/payroll
git commit -m "Store payroll runs: create, change, finalize, reopen, preview and list

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 7: Book and unbook a payroll run in the ledger's transaction

**Files:**
- Modify: `crates/payroll/src/lib.rs`
- Test: `crates/payroll/tests/store.rs`

**Interfaces:**
- Consumes: `doris_ledger::record_voucher_in(conn, company_id, actor, RecordVoucher, today) -> doris_ledger::Result<VoucherRef>` and `doris_ledger::correct_voucher_in(conn, company_id, actor, fiscal_year_start, number, date, today) -> doris_ledger::Result<VoucherRef>`. Both need the caller's IMMEDIATE transaction. `VoucherRef { fiscal_year_start: Date, number: u32 }`. Also `Company::first_fiscal_year.containing(date).end`.
- Produces: `pub async fn book_payroll_run(&SqlitePool, company_id: Uuid, actor: Uuid, payroll_run_id: Uuid, today: Date) -> Result<BookedVoucher>` and `pub async fn unbook_payroll_run(&SqlitePool, company_id: Uuid, actor: Uuid, payroll_run_id: Uuid, today: Date) -> Result<BookedVoucher>`, which returns the rättelse.

- [ ] **Step 1: Write the failing tests** — append to `crates/payroll/tests/store.rs`, and add `book_payroll_run, unbook_payroll_run` to the `use doris_payroll::{…}` list:

```rust
use doris_ledger::domain::DomainError as LedgerError;

/// Åsa and a finalized run paying her 35 000 kr on `pay_date`.
async fn finalized_run(pool: &SqlitePool, id: Uuid, anna: Uuid, pay_date: &str) -> Uuid {
    let asa_id = add_employee(pool, id, anna, asa()).await.unwrap();
    let run = create_payroll_run(pool, id, anna, draft(pay_date, &[(asa_id, 35_000 * KR, 8_000 * KR)]))
        .await
        .unwrap();
    finalize_payroll_run(pool, id, anna, run).await.unwrap();
    run
}

fn ledger_refusal(err: Error) -> LedgerError {
    match err {
        Error::Ledger(doris_ledger::Error::Domain(e)) => e,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_run_books_on_its_pay_date_and_not_before() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;

    let early = book_payroll_run(&pool, id, anna, run, d("2025-10-24")).await.unwrap_err();
    assert!(matches!(early, Error::Domain(DomainError::PayrollRunNotDue)));
    assert!(events_of(&pool, "ledger-").await.is_empty());

    let voucher = book_payroll_run(&pool, id, anna, run, d("2025-10-25")).await.unwrap();

    assert_eq!((voucher.fiscal_year_start, voucher.number), (d("2025-01-01"), 1));
    assert_eq!(events_of(&pool, "ledger-").await, ["VoucherRecorded"]);
    assert_eq!(events_of(&pool, "payroll-").await.last().unwrap(), "PayrollRunBooked");
    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    let lines: Vec<_> = vouchers[0]
        .lines
        .iter()
        .map(|l| (l.account.get(), l.debit, l.credit))
        .collect();
    assert_eq!(
        lines,
        [
            (7210, 35_000 * KR, 0),
            (2710, 0, 8_000 * KR),
            (1930, 0, 27_000 * KR),
            (7510, 1_099_700, 0),
            (2731, 0, 1_099_700),
        ]
    );
    assert_eq!((vouchers[0].date, vouchers[0].text.as_str()), (d("2025-10-25"), "Lön oktober 2025"));
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Booked(voucher));
    assert!(matches!(
        reopen_payroll_run(&pool, id, anna, run).await,
        Err(Error::Domain(DomainError::PayrollRunBooked))
    ));
}

#[tokio::test]
async fn a_refused_booking_writes_nothing_and_the_run_stays_finalized() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-25");
    let payroll_events = events_of(&pool, "payroll-").await;

    doris_ledger::set_account_active(&pool, id, anna, 7210, false).await.unwrap();
    let refused = book_payroll_run(&pool, id, anna, run, today).await.unwrap_err();
    assert_eq!(ledger_refusal(refused), LedgerError::AccountInactive);
    assert_eq!(events_of(&pool, "payroll-").await, payroll_events);
    assert!(events_of(&pool, "ledger-").await.is_empty());
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Finalized);

    doris_ledger::set_account_active(&pool, id, anna, 7210, true).await.unwrap();
    assert_eq!(book_payroll_run(&pool, id, anna, run, today).await.unwrap().number, 1);
}

#[tokio::test]
async fn a_closed_year_refuses_the_booking() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-12-25").await;
    let today = d("2026-01-10");
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    let ledger_events = events_of(&pool, "ledger-").await;

    let refused = book_payroll_run(&pool, id, anna, run, today).await.unwrap_err();

    assert_eq!(ledger_refusal(refused), LedgerError::FiscalYearClosed);
    assert_eq!(events_of(&pool, "ledger-").await, ledger_events);
}

#[tokio::test]
async fn backa_bokforing_reverses_the_voucher_and_the_run_can_change_and_book_again() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    let first = book_payroll_run(&pool, id, anna, run, today).await.unwrap();

    let correction = unbook_payroll_run(&pool, id, anna, run, today).await.unwrap();

    assert_eq!(correction.number, 2);
    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!(vouchers[1].corrects, Some(first.number));
    assert_eq!(vouchers[1].date, today);
    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap().status, PayrollRunStatus::Finalized);
    assert!(matches!(
        unbook_payroll_run(&pool, id, anna, run, today).await,
        Err(Error::Domain(DomainError::PayrollRunNotBooked))
    ));

    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
    let asa_id = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].employee_id;
    update_payroll_run(&pool, id, anna, run, draft("2025-10-25", &[(asa_id, 36_000 * KR, 8_300 * KR)]))
        .await
        .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let again = book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    assert_eq!(again.number, 3);
    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap().status, PayrollRunStatus::Booked(again));
}

#[tokio::test]
async fn a_rattelse_from_the_grundbok_also_unbooks_the_run() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    let voucher = book_payroll_run(&pool, id, anna, run, today).await.unwrap();

    doris_ledger::correct_voucher(&pool, id, anna, voucher.fiscal_year_start, voucher.number, today, today)
        .await
        .unwrap();

    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap().status, PayrollRunStatus::Finalized);
    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
}

#[tokio::test]
async fn unbooking_after_the_year_ended_dates_the_rattelse_on_its_last_day() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-12-25").await;
    book_payroll_run(&pool, id, anna, run, d("2025-12-27")).await.unwrap();

    unbook_payroll_run(&pool, id, anna, run, d("2026-01-10")).await.unwrap();

    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!(vouchers[1].date, d("2025-12-31"));
}

#[tokio::test]
async fn all_projections_rebuild_with_bookings_in_place() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    unbook_payroll_run(&pool, id, anna, run, today).await.unwrap();
    book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    let runs = "SELECT company_id || payroll_run_id || pay_date || text || finalized || updated_by
                FROM payroll_runs ORDER BY 1";
    let lines = "SELECT payroll_run_id || employee_id || gross || tax
                 || COALESCE(salary_account, '-') || COALESCE(fee, '-') || COALESCE(net, '-')
                 FROM payroll_run_lines ORDER BY 1";
    let bookings = "SELECT payroll_run_id || fiscal_year_start || voucher_number
                    FROM payroll_run_bookings ORDER BY rowid";
    let before = (table(&pool, runs).await, table(&pool, lines).await, table(&pool, bookings).await);
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();

    // The ledger's rebuild empties vouchers: no payroll key may refer to it.
    doris_ledger::rebuild_projections(&pool).await.unwrap();
    rebuild_projections(&pool).await.unwrap();

    let after = (table(&pool, runs).await, table(&pool, lines).await, table(&pool, bookings).await);
    assert_eq!(after, before);
    assert_eq!(before.2.len(), 2);
    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap(), view);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-payroll --test store`
Expected: FAIL to compile with "unresolved imports `doris_payroll::book_payroll_run`, `doris_payroll::unbook_payroll_run`".

- [ ] **Step 3: Implement** — append to `crates/payroll/src/lib.rs` and add `use jiff::civil::Date;`:

```rust
/// Books a finalized run as a voucher, on or after its pay date, in one
/// transaction with its `PayrollRunBooked`: both, or nothing and no
/// voucher number used up.
pub async fn book_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<BookedVoucher> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let voucher = domain::book_payroll_run(&payroll, payroll_run_id, today)?;
    let recorded = doris_ledger::record_voucher_in(&mut tx, company_id, actor, voucher, today).await?;
    let booked = BookedVoucher {
        fiscal_year_start: recorded.fiscal_year_start,
        number: recorded.number,
    };
    let event = domain::booked(payroll_run_id, booked);
    append(&mut tx, company_id, version, &[event], actor).await?;
    tx.commit().await?;
    Ok(booked)
}

/// Backa bokföring: a rättelse of the run's voucher, after which the run
/// is Färdigställd again. The rättelse is dated `today`, but no later than
/// the end of the voucher's fiscal year, which the ledger requires. No
/// payroll event: the status follows from the rättelse.
pub async fn unbook_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<BookedVoucher> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, payroll, _) = load(&mut tx, company_id, actor).await?;
    let voucher = domain::unbook_payroll_run(&payroll, payroll_run_id)?;
    let year_end = company
        .first_fiscal_year
        .containing(voucher.fiscal_year_start)
        .end;
    let correction = doris_ledger::correct_voucher_in(
        &mut tx,
        company_id,
        actor,
        voucher.fiscal_year_start,
        voucher.number,
        today.min(year_end),
        today,
    )
    .await?;
    tx.commit().await?;
    Ok(BookedVoucher {
        fiscal_year_start: correction.fiscal_year_start,
        number: correction.number,
    })
}
```

- [ ] **Step 4: Run the tests, the ledger's stress test included**

Run: `cargo test -p doris-payroll && cargo test -p doris-ledger --test stress`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/payroll
git commit -m "Book a payroll run as a voucher from its pay date, and back the booking out with a rättelse

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 8: `PayrollService` over gRPC-Web

**Files:**
- Create: `proto/doris/payroll/v1/payroll.proto`
- Modify: `crates/proto/build.rs`, `crates/proto/src/lib.rs`
- Modify: `crates/server/Cargo.toml` (add `doris-payroll.workspace = true`)
- Create: `crates/server/src/payroll.rs`
- Modify: `crates/server/src/ledger.rs` (`fn status` becomes `pub(crate) fn status`)
- Modify: `crates/server/src/lib.rs` (module, re-export, `router` parameter)
- Modify: `crates/server/src/main.rs:84-90` (pass `PayrollApi`)
- Modify: `crates/server/tests/common/mod.rs` (pass `PayrollApi`, add a `payroll()` client)
- Test: `crates/server/tests/payroll.rs`

**Interfaces:**
- Consumes: everything `doris_payroll` exports (Tasks 5–7), `crate::grpc::{signed_in_user, today}`, `crate::ledger::status`.
- Produces: `doris_proto::payroll::v1` (package `doris.payroll.v1`) with `PayrollService`, `doris_server::PayrollApi::new(SqlitePool)`, and `router(api, companies, ledger, payroll, cors_origins, serve_frontend)`. Test harness: `TestServer::payroll() -> common::Payroll`.

- [ ] **Step 1: Write the proto**

`proto/doris/payroll/v1/payroll.proto`:

```proto
syntax = "proto3";

package doris.payroll.v1;

import "doris/ledger/v1/ledger.proto";

// Employees and payroll runs (lönekörningar). Every call needs a session
// and a company the caller is a member of. Amounts are öre.
service PayrollService {
  rpc ListEmployees(ListEmployeesRequest) returns (ListEmployeesResponse);
  rpc AddEmployee(AddEmployeeRequest) returns (AddEmployeeResponse);
  // Name, monthly salary and salary account; never the personnummer.
  rpc UpdateEmployee(UpdateEmployeeRequest) returns (UpdateEmployeeResponse);
  rpc DeactivateEmployee(DeactivateEmployeeRequest) returns (DeactivateEmployeeResponse);

  // What a draft would lock if finalized now. Writes nothing.
  rpc PreviewPayrollRun(PreviewPayrollRunRequest) returns (PreviewPayrollRunResponse);
  rpc CreatePayrollRun(CreatePayrollRunRequest) returns (CreatePayrollRunResponse);
  // Only an open run changes.
  rpc UpdatePayrollRun(UpdatePayrollRunRequest) returns (UpdatePayrollRunResponse);
  // Locks amounts, fees and accounts; the pay date may be in the future.
  rpc FinalizePayrollRun(PayrollRunRef) returns (FinalizePayrollRunResponse);
  // A finalized run that is not booked.
  rpc ReopenPayrollRun(PayrollRunRef) returns (ReopenPayrollRunResponse);
  // Books a finalized run as a voucher, no earlier than its pay date.
  rpc BookPayrollRun(PayrollRunRef) returns (VoucherRef);
  // Backa bokföring: a rättelse of the run's voucher; returns the rättelse.
  rpc UnbookPayrollRun(PayrollRunRef) returns (VoucherRef);
  rpc GetPayrollRun(PayrollRunRef) returns (PayrollRun);
  // Newest pay date first.
  rpc ListPayrollRuns(ListPayrollRunsRequest) returns (ListPayrollRunsResponse);
}

message Employee {
  string id = 1;
  string name = 2;
  string personal_identity_number = 3; // ÅÅÅÅMMDD-NNNN
  int64 monthly_salary = 4;
  uint32 salary_account = 5;
  bool active = 6;
}

message ListEmployeesRequest {
  string company_id = 1;
}
message ListEmployeesResponse {
  repeated Employee employees = 1;
}

message AddEmployeeRequest {
  string company_id = 1;
  string name = 2;
  string personal_identity_number = 3;
  int64 monthly_salary = 4;
  uint32 salary_account = 5;
}
message AddEmployeeResponse {
  string employee_id = 1;
}

message UpdateEmployeeRequest {
  string company_id = 1;
  string employee_id = 2;
  string name = 3;
  int64 monthly_salary = 4;
  uint32 salary_account = 5;
}
message UpdateEmployeeResponse {}

message DeactivateEmployeeRequest {
  string company_id = 1;
  string employee_id = 2;
}
message DeactivateEmployeeResponse {}

message PayrollRunLineInput {
  string employee_id = 1;
  int64 gross = 2;
  int64 tax = 3;
}
message PayrollRunDraft {
  string pay_date = 1; // YYYY-MM-DD
  string text = 2;     // empty: "Lön {månad} {år}"
  repeated PayrollRunLineInput lines = 3;
}
message PayrollRunRef {
  string company_id = 1;
  string payroll_run_id = 2;
}
message VoucherRef {
  string fiscal_year_start = 1;
  uint32 number = 2;
}

message PreviewPayrollRunRequest {
  string company_id = 1;
  PayrollRunDraft draft = 2;
}
message PreviewPayrollRunResponse {
  string text = 1;
  repeated PayrollRunLine lines = 2;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 3;
}

message CreatePayrollRunRequest {
  string company_id = 1;
  PayrollRunDraft draft = 2;
}
message CreatePayrollRunResponse {
  string payroll_run_id = 1;
}

message UpdatePayrollRunRequest {
  PayrollRunRef run = 1;
  PayrollRunDraft draft = 2;
}
message UpdatePayrollRunResponse {}

message FinalizePayrollRunResponse {}
message ReopenPayrollRunResponse {}

message ListPayrollRunsRequest {
  string company_id = 1;
}
message ListPayrollRunsResponse {
  repeated PayrollRun payroll_runs = 1;
}

enum PayrollRunStatus {
  PAYROLL_RUN_STATUS_UNSPECIFIED = 0;
  PAYROLL_RUN_STATUS_OPEN = 1;
  PAYROLL_RUN_STATUS_FINALIZED = 2;
  PAYROLL_RUN_STATUS_BOOKED = 3;
}

message PayrollRunLine {
  string employee_id = 1;
  string employee_name = 2;
  int64 gross = 3;
  int64 tax = 4;
  // Set once finalized (and in a preview).
  uint32 salary_account = 5;
  uint32 fee_rate = 6; // basis points
  int64 fee = 7;
  int64 net = 8;
}

message PayrollRun {
  string id = 1;
  string pay_date = 2;
  string text = 3;
  PayrollRunStatus status = 4;
  repeated PayrollRunLine lines = 5;
  repeated doris.ledger.v1.VoucherLine voucher_lines = 6; // once finalized
  VoucherRef voucher = 7;                                  // while booked
}
```

In `crates/proto/build.rs`, add `"../../proto/doris/payroll/v1/payroll.proto",` after the ledger proto. In `crates/proto/src/lib.rs` append:

```rust
pub mod payroll {
    pub mod v1 {
        tonic::include_proto!("doris.payroll.v1");
    }
}
```

Run: `cargo build -p doris-proto --features server`
Expected: builds. prost refers to `doris.ledger.v1.VoucherLine` as `super::super::ledger::v1::VoucherLine`, which resolves because `ledger` and `payroll` are siblings at the crate root.

- [ ] **Step 2: Write the failing integration tests**

In `crates/server/tests/common/mod.rs`, add `use doris_proto::payroll::v1::payroll_service_client::PayrollServiceClient;`, extend `use doris_server::{…}` with `PayrollApi`, add `pub type Payroll = PayrollServiceClient<Transport>;`, pass `PayrollApi::new(pool.clone()),` to `router` right after `LedgerApi::new(pool.clone()),`, and add to `impl TestServer`:

```rust
    pub fn payroll(&self) -> Payroll {
        PayrollServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }
```

`crates/server/tests/payroll.rs`:

```rust
mod common;

use common::{Payroll, TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::payroll::v1 as pb;
use tonic::{Code, Request};

const KR: i64 = 100;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company, first räkenskapsår 2026.
async fn company(server: &TestServer, session: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2026-01-01".into(),
                fiscal_year_end: "2026-12-31".into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
}

async fn hire(api: &mut Payroll, session: &str, company_id: &str) -> String {
    api.add_employee(authed(
        pb::AddEmployeeRequest {
            company_id: company_id.into(),
            name: "Åsa Öberg".into(),
            personal_identity_number: "19800101-1231".into(),
            monthly_salary: 35_000 * KR,
            salary_account: 7210,
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
    .employee_id
}

fn draft(pay_date: &str, employee_id: &str, gross: i64, tax: i64) -> Option<pb::PayrollRunDraft> {
    Some(pb::PayrollRunDraft {
        pay_date: pay_date.into(),
        text: String::new(),
        lines: vec![pb::PayrollRunLineInput {
            employee_id: employee_id.into(),
            gross,
            tax,
        }],
    })
}

fn run_ref(company_id: &str, run: &str) -> pb::PayrollRunRef {
    pb::PayrollRunRef {
        company_id: company_id.into(),
        payroll_run_id: run.into(),
    }
}

#[tokio::test]
async fn a_member_runs_payroll_from_employee_to_voucher_and_back() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;

    let employees = api
        .list_employees(authed(pb::ListEmployeesRequest { company_id: id.clone() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .employees;
    assert_eq!(employees[0].personal_identity_number, "19800101-1231");

    let preview = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.text, "Lön januari 2026");
    assert_eq!(preview.lines[0].employee_name, "Åsa Öberg");
    assert_eq!((preview.lines[0].fee_rate, preview.lines[0].fee), (3142, 1_099_700));
    assert_eq!(preview.voucher_lines.len(), 5);

    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna)).await.unwrap();
    let booked = api
        .book_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((booked.fiscal_year_start.as_str(), booked.number), ("2026-01-01", 1));

    let vouchers = server
        .ledger()
        .list_vouchers(authed(
            lpb::ListVouchersRequest {
                company_id: id.clone(),
                fiscal_year_start: "2026-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers;
    assert_eq!(vouchers[0].text, "Lön januari 2026");
    let shown = api
        .get_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(shown.status(), pb::PayrollRunStatus::Booked);
    assert_eq!(shown.voucher.unwrap().number, 1);

    let correction = api
        .unbook_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(correction.number, 2);
    api.reopen_payroll_run(authed(run_ref(&id, &run), &anna)).await.unwrap();
    api.update_payroll_run(authed(
        pb::UpdatePayrollRunRequest {
            run: Some(run_ref(&id, &run)),
            draft: draft("2026-01-25", &asa, 36_000 * KR, 8_300 * KR),
        },
        &anna,
    ))
    .await
    .unwrap();
    let runs = api
        .list_payroll_runs(authed(pb::ListPayrollRunsRequest { company_id: id.clone() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .payroll_runs;
    assert_eq!(runs[0].status(), pb::PayrollRunStatus::Open);
    assert_eq!(runs[0].lines[0].gross, 36_000 * KR);
}

#[tokio::test]
async fn the_lifecycle_and_the_pay_date_are_enforced() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;
    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2099-12-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna)).await.unwrap();

    let early = api.book_payroll_run(authed(run_ref(&id, &run), &anna)).await.unwrap_err();
    assert_eq!(code_of(early), (Code::FailedPrecondition, "payroll_run_not_due".into()));
    let locked = api
        .update_payroll_run(authed(
            pb::UpdatePayrollRunRequest {
                run: Some(run_ref(&id, &run)),
                draft: draft("2099-12-25", &asa, 1, 0),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(locked), (Code::FailedPrecondition, "payroll_run_not_open".into()));
    let missing = api
        .get_payroll_run(authed(run_ref(&id, "not-a-uuid"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(missing), (Code::NotFound, "payroll_run_not_found".into()));
}

#[tokio::test]
async fn invalid_input_gets_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;

    let bad_pin = api
        .add_employee(authed(
            pb::AddEmployeeRequest {
                company_id: id.clone(),
                name: "Bo".into(),
                personal_identity_number: "19800101-1232".into(),
                monthly_salary: 1,
                salary_account: 7210,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad_pin),
        (Code::InvalidArgument, "invalid_personal_identity_number".into())
    );
    let twice = api
        .add_employee(authed(
            pb::AddEmployeeRequest {
                company_id: id.clone(),
                name: "Åsa".into(),
                personal_identity_number: "198001011231".into(),
                monthly_salary: 1,
                salary_account: 7210,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(twice), (Code::FailedPrecondition, "duplicate_employee".into()));
    let mut long = draft("2026-01-25", &asa, 100, 0).unwrap();
    long.text = "x".repeat(201);
    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest { company_id: id.clone(), draft: Some(long) },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(refused), (Code::InvalidArgument, "invalid_voucher_text".into()));
    let bad_tax = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 100, 101),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(bad_tax), (Code::InvalidArgument, "invalid_tax".into()));
}

#[tokio::test]
async fn strangers_and_signed_out_callers_find_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.payroll();

    let stranger = api
        .list_employees(authed(pb::ListEmployeesRequest { company_id: id.clone() }, &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(stranger), (Code::NotFound, "company_not_found".into()));
    let signed_out = api
        .list_payroll_runs(Request::new(pb::ListPayrollRunsRequest { company_id: id }))
        .await
        .unwrap_err();
    assert_eq!(code_of(signed_out), (Code::Unauthenticated, "not_signed_in".into()));
}
```

Run: `cargo test -p doris-server --test payroll`
Expected: FAIL to compile with "unresolved import `doris_server::PayrollApi`".

- [ ] **Step 3: Implement the service**

In `crates/server/src/ledger.rs`, change `fn status(err: Error) -> Status {` to `pub(crate) fn status(err: Error) -> Status {`.

`crates/server/src/payroll.rs`:

```rust
//! `doris.payroll.v1.PayrollService`: maps gRPC calls onto `doris_payroll`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist. Personnummer and names are
//! personal data and never logged.

use crate::grpc::{signed_in_user, today};
use doris_ledger::domain::VoucherLine;
use doris_payroll::domain::{
    BookedVoucher, DomainError, DraftLine, Employee, PayrollRunDraft, PayrollRunLine,
    PayrollRunStatus,
};
use doris_payroll::{Error, NewEmployee, PayrollRunView};
use doris_proto::ledger::v1 as lpb;
use doris_proto::payroll::v1 as pb;
use doris_proto::payroll::v1::payroll_service_server::PayrollService;
use jiff::civil::Date;
use sqlx::SqlitePool;
use std::collections::HashMap;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct PayrollApi {
    pool: SqlitePool,
}

impl PayrollApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_payroll` itself.
    async fn caller<T>(
        &self,
        request: &Request<T>,
        company_id: &str,
    ) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id
            .parse()
            .map_err(|_| Status::not_found("company_not_found"))?;
        Ok((company, user.id))
    }

    /// Employee names by id, for lines that carry only the id.
    async fn names(&self, company: Uuid, user: Uuid) -> Result<HashMap<Uuid, String>, Status> {
        Ok(doris_payroll::list_employees(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(|e| (e.id, e.name.as_str().to_owned()))
            .collect())
    }
}

#[tonic::async_trait]
impl PayrollService for PayrollApi {
    async fn list_employees(
        &self,
        request: Request<pb::ListEmployeesRequest>,
    ) -> Result<Response<pb::ListEmployeesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let employees = doris_payroll::list_employees(&self.pool, company, user)
            .await
            .map_err(status)?
            .iter()
            .map(employee_message)
            .collect();
        Ok(Response::new(pb::ListEmployeesResponse { employees }))
    }

    async fn add_employee(
        &self,
        request: Request<pb::AddEmployeeRequest>,
    ) -> Result<Response<pb::AddEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let new = NewEmployee {
            name: &req.name,
            personal_identity_number: &req.personal_identity_number,
            monthly_salary: req.monthly_salary,
            salary_account: req.salary_account,
        };
        let employee_id = doris_payroll::add_employee(&self.pool, company, user, new)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddEmployeeResponse {
            employee_id: employee_id.to_string(),
        }))
    }

    async fn update_employee(
        &self,
        request: Request<pb::UpdateEmployeeRequest>,
    ) -> Result<Response<pb::UpdateEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_payroll::update_employee(
            &self.pool,
            company,
            user,
            employee_id(&req.employee_id)?,
            &req.name,
            req.monthly_salary,
            req.salary_account,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::UpdateEmployeeResponse {}))
    }

    async fn deactivate_employee(
        &self,
        request: Request<pb::DeactivateEmployeeRequest>,
    ) -> Result<Response<pb::DeactivateEmployeeResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let id = employee_id(&request.get_ref().employee_id)?;
        doris_payroll::deactivate_employee(&self.pool, company, user, id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::DeactivateEmployeeResponse {}))
    }

    async fn preview_payroll_run(
        &self,
        request: Request<pb::PreviewPayrollRunRequest>,
    ) -> Result<Response<pb::PreviewPayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        let preview = doris_payroll::preview_payroll_run(&self.pool, company, user, draft)
            .await
            .map_err(status)?;
        let names = self.names(company, user).await?;
        Ok(Response::new(pb::PreviewPayrollRunResponse {
            text: preview.text,
            lines: preview
                .lines
                .iter()
                .map(|l| locked_line_message(l, names.get(&l.employee_id).cloned().unwrap_or_default()))
                .collect(),
            voucher_lines: doris_payroll::domain::voucher_lines(&preview.lines)
                .iter()
                .map(voucher_line_message)
                .collect(),
        }))
    }

    async fn create_payroll_run(
        &self,
        request: Request<pb::CreatePayrollRunRequest>,
    ) -> Result<Response<pb::CreatePayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        let id = doris_payroll::create_payroll_run(&self.pool, company, user, draft)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CreatePayrollRunResponse {
            payroll_run_id: id.to_string(),
        }))
    }

    async fn update_payroll_run(
        &self,
        request: Request<pb::UpdatePayrollRunRequest>,
    ) -> Result<Response<pb::UpdatePayrollRunResponse>, Status> {
        let run = request.get_ref().run.clone().unwrap_or_default();
        let (company, user) = self.caller(&request, &run.company_id).await?;
        let draft = draft(request.into_inner().draft)?;
        doris_payroll::update_payroll_run(&self.pool, company, user, run_id(&run.payroll_run_id)?, draft)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::UpdatePayrollRunResponse {}))
    }

    async fn finalize_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::FinalizePayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        doris_payroll::finalize_payroll_run(&self.pool, company, user, run)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::FinalizePayrollRunResponse {}))
    }

    async fn reopen_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::ReopenPayrollRunResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        doris_payroll::reopen_payroll_run(&self.pool, company, user, run)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ReopenPayrollRunResponse {}))
    }

    async fn book_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::VoucherRef>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let booked = doris_payroll::book_payroll_run(&self.pool, company, user, run, today())
            .await
            .map_err(status)?;
        Ok(Response::new(voucher_message(booked)))
    }

    async fn unbook_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::VoucherRef>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let correction = doris_payroll::unbook_payroll_run(&self.pool, company, user, run, today())
            .await
            .map_err(status)?;
        Ok(Response::new(voucher_message(correction)))
    }

    async fn get_payroll_run(
        &self,
        request: Request<pb::PayrollRunRef>,
    ) -> Result<Response<pb::PayrollRun>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let run = run_id(&request.get_ref().payroll_run_id)?;
        let view = doris_payroll::get_payroll_run(&self.pool, company, user, run)
            .await
            .map_err(status)?;
        Ok(Response::new(run_message(view)))
    }

    async fn list_payroll_runs(
        &self,
        request: Request<pb::ListPayrollRunsRequest>,
    ) -> Result<Response<pb::ListPayrollRunsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let payroll_runs = doris_payroll::list_payroll_runs(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(run_message)
            .collect();
        Ok(Response::new(pb::ListPayrollRunsResponse { payroll_runs }))
    }
}

fn employee_message(e: &Employee) -> pb::Employee {
    pb::Employee {
        id: e.id.to_string(),
        name: e.name.as_str().to_owned(),
        personal_identity_number: e.personal_identity_number.formatted(),
        monthly_salary: e.monthly_salary,
        salary_account: e.salary_account.get(),
        active: e.active,
    }
}

fn locked_line_message(l: &PayrollRunLine, employee_name: String) -> pb::PayrollRunLine {
    pb::PayrollRunLine {
        employee_id: l.employee_id.to_string(),
        employee_name,
        gross: l.gross,
        tax: l.tax,
        salary_account: l.salary_account.get(),
        fee_rate: l.fee_rate,
        fee: l.fee,
        net: l.net,
    }
}

fn voucher_line_message(l: &VoucherLine) -> lpb::VoucherLine {
    lpb::VoucherLine {
        account: l.account.get().into(),
        debit: l.debit,
        credit: l.credit,
    }
}

fn voucher_message(v: BookedVoucher) -> pb::VoucherRef {
    pb::VoucherRef {
        fiscal_year_start: v.fiscal_year_start.to_string(),
        number: v.number,
    }
}

fn run_message(view: PayrollRunView) -> pb::PayrollRun {
    let voucher_lines = view.voucher_lines().iter().map(voucher_line_message).collect();
    let (status, voucher) = match view.status {
        PayrollRunStatus::Open => (pb::PayrollRunStatus::Open, None),
        PayrollRunStatus::Finalized => (pb::PayrollRunStatus::Finalized, None),
        PayrollRunStatus::Booked(v) => (pb::PayrollRunStatus::Booked, Some(voucher_message(v))),
    };
    pb::PayrollRun {
        id: view.id.to_string(),
        pay_date: view.pay_date.to_string(),
        text: view.text,
        status: status as i32,
        lines: view
            .lines
            .into_iter()
            .map(|l| match &l.locked {
                Some(locked) => locked_line_message(locked, l.employee_name),
                None => pb::PayrollRunLine {
                    employee_id: l.employee_id.to_string(),
                    employee_name: l.employee_name,
                    gross: l.gross,
                    tax: l.tax,
                    ..Default::default()
                },
            })
            .collect(),
        voucher_lines,
        voucher,
    }
}

fn draft(message: Option<pb::PayrollRunDraft>) -> Result<PayrollRunDraft, Status> {
    let message = message.unwrap_or_default();
    Ok(PayrollRunDraft {
        pay_date: date(&message.pay_date)?,
        text: message.text,
        lines: message
            .lines
            .iter()
            .map(|l| {
                Ok(DraftLine {
                    employee_id: employee_id(&l.employee_id)?,
                    gross: l.gross,
                    tax: l.tax,
                })
            })
            .collect::<Result<_, Status>>()?,
    })
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_date"))
}

/// A malformed id names no employee.
fn employee_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("employee_not_found"))
}

fn run_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("payroll_run_not_found"))
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidPersonalIdentityNumber => {
            Status::invalid_argument("invalid_personal_identity_number")
        }
        InvalidEmployeeName => Status::invalid_argument("invalid_employee_name"),
        InvalidSalary => Status::invalid_argument("invalid_salary"),
        InvalidSalaryAccount => Status::invalid_argument("invalid_salary_account"),
        InvalidTax => Status::invalid_argument("invalid_tax"),
        // The ledger's code: the text becomes the voucher's.
        InvalidText => Status::invalid_argument("invalid_voucher_text"),
        EmptyPayrollRun => Status::invalid_argument("empty_payroll_run"),
        DuplicatePayrollRunLine => Status::invalid_argument("duplicate_payroll_run_line"),
        DuplicateEmployee => Status::failed_precondition("duplicate_employee"),
        EmployeeInactive => Status::failed_precondition("employee_inactive"),
        PayrollRunNotOpen => Status::failed_precondition("payroll_run_not_open"),
        PayrollRunNotFinalized => Status::failed_precondition("payroll_run_not_finalized"),
        PayrollRunBooked => Status::failed_precondition("payroll_run_booked"),
        PayrollRunNotBooked => Status::failed_precondition("payroll_run_not_booked"),
        PayrollRunNotDue => Status::failed_precondition("payroll_run_not_due"),
        PayrollRunOutdated => Status::failed_precondition("payroll_run_outdated"),
        EmployeeNotFound => Status::not_found("employee_not_found"),
        PayrollRunNotFound => Status::not_found("payroll_run_not_found"),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::NotFound => Status::not_found("company_not_found"),
        Error::Ledger(err) => crate::ledger::status(err),
        Error::Store(err) => {
            // sqlx messages name columns, never values: no personal data.
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}
```

In `crates/server/src/lib.rs`: add `mod payroll;` after `mod ledger;`, `use doris_proto::payroll::v1::payroll_service_server::PayrollServiceServer;` with the other service imports, `pub use payroll::PayrollApi;` after `pub use ledger::LedgerApi;`, a `payroll: PayrollApi,` parameter after `ledger: LedgerApi,` in `router`, and `.add_service(PayrollServiceServer::new(payroll))` right after the `LedgerServiceServer` service. Payroll messages are small: tonic's 4 MiB default holds, and `session_gate` is not needed for it.

In `crates/server/src/main.rs`, pass `PayrollApi::new(pool.clone()),` before `LedgerApi::new(pool)` moves the pool. The order of arguments is `router(auth, companies, ledger, payroll, …)`, so write it as:

```rust
    let payroll = PayrollApi::new(pool.clone());
    let app = doris_server::router::<WebDist>(
        AuthApi::new(pool.clone(), auth),
        CompanyApi::new(pool.clone(), bolagsverket),
        LedgerApi::new(pool),
        payroll,
        cors_origins,
        config.serve_frontend,
    );
```

and add `PayrollApi` to main.rs's `use doris_server::{…}`. Also add `doris-payroll.workspace = true` to `crates/server/Cargo.toml` under `[dependencies]` after `doris-ledger`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-server`
Expected: PASS, the new `payroll` tests and all existing ones.

- [ ] **Step 5: Commit**

```bash
git add proto crates/proto crates/server Cargo.lock
git commit -m "Serve PayrollService over gRPC-Web with stable error codes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 9: Frontend: client, error texts, menu and the Anställda page

**Files:**
- Modify: `crates/web/src/api.rs`
- Modify: `crates/web/src/errors.rs` (messages and a test)
- Modify: `crates/web/src/app.rs` (route, menu link)
- Create: `crates/web/src/pages/employees.rs`
- Modify: `crates/web/src/pages/mod.rs`

**Interfaces:**
- Consumes: `doris_proto::payroll::v1` (Task 8); `format::{amount, parse_amount}`; `ui::{Button, Card, Checkbox, ErrorAlert, Field, Select, SELECT_OPTION, Table, TABLE_*}`; `active_company::Companies`.
- Produces: `api::{payroll_api, ppb, PayrollApi}`; `pages::Employees`; `pages::employees::SALARY_ACCOUNTS: [(u32, &str); 3]`.

- [ ] **Step 1: Write the failing test for the error texts** — in `crates/web/src/errors.rs`'s `mod tests`, add:

```rust
    #[test]
    fn payroll_codes_have_swedish_messages() {
        for code in [
            "invalid_personal_identity_number",
            "invalid_employee_name",
            "invalid_salary",
            "invalid_salary_account",
            "invalid_tax",
            "empty_payroll_run",
            "duplicate_payroll_run_line",
            "duplicate_employee",
            "employee_inactive",
            "employee_not_found",
            "payroll_run_not_found",
            "payroll_run_not_open",
            "payroll_run_not_finalized",
            "payroll_run_booked",
            "payroll_run_not_booked",
            "payroll_run_not_due",
            "payroll_run_outdated",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("payroll_run_not_due"),
            "Lönekörningen kan inte bokföras före utbetalningsdagen."
        );
    }
```

Run: `cargo test -p doris-web payroll_codes`
Expected: FAIL. The assertion shows the fallback "Något gick fel. Försök igen.". Check that this is the exact fallback string in `message`'s `_ =>` arm, and use that string if it differs.

- [ ] **Step 2: Add the messages** — in `message`'s `match`, before the fallback arm:

```rust
        "invalid_personal_identity_number" => "Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN).",
        "invalid_employee_name" => "Ange ett namn (högst 100 tecken).",
        "invalid_salary" => "Lönen måste vara större än noll.",
        "invalid_salary_account" => "Välj ett lönekonto.",
        "invalid_tax" => "Skatten får inte vara negativ eller större än bruttolönen.",
        "empty_payroll_run" => "Välj minst en anställd.",
        "duplicate_payroll_run_line" => "Samma anställd finns två gånger i körningen.",
        "duplicate_employee" => "Det finns redan en anställd med det personnumret.",
        "employee_inactive" => "Den anställda är inaktiverad.",
        "employee_not_found" => "Den anställda hittades inte.",
        "payroll_run_not_found" => "Lönekörningen hittades inte.",
        "payroll_run_not_open" => "Lönekörningen är färdigställd. Öppna den för att ändra.",
        "payroll_run_not_finalized" => "Lönekörningen är inte färdigställd.",
        "payroll_run_booked" => "Lönekörningen är bokförd. Backa bokföringen först.",
        "payroll_run_not_booked" => "Lönekörningen är inte bokförd.",
        "payroll_run_not_due" => "Lönekörningen kan inte bokföras före utbetalningsdagen.",
        "payroll_run_outdated" => {
            "Avgifterna har ändrats sedan körningen färdigställdes. Öppna och färdigställ den igen."
        }
```

Run: `cargo test -p doris-web`
Expected: PASS.

- [ ] **Step 3: Add the client** — in `crates/web/src/api.rs`, add `use doris_proto::payroll::v1::payroll_service_client::PayrollServiceClient;`, `pub use doris_proto::payroll::v1 as ppb;`, `pub type PayrollApi = PayrollServiceClient<Client>;` and:

```rust
pub fn payroll_api() -> PayrollApi {
    PayrollServiceClient::new(client())
}
```

- [ ] **Step 4: Write the page** — `crates/web/src/pages/employees.rs`:

```rust
//! The active company's employees: add, edit, deactivate. The
//! personnummer is set once and never edited.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, parse_amount};
use crate::ui::{
    Button, Card, Checkbox, ErrorAlert, Field, SELECT_OPTION, Select, TABLE_AMOUNT_CELL,
    TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The salary accounts an employee can have, in the order offered.
pub const SALARY_ACCOUNTS: [(u32, &str); 3] = [
    (7210, "7210 Löner till tjänstemän"),
    (7010, "7010 Löner till kollektivanställda"),
    (7220, "7220 Löner till företagsledare"),
];

#[component]
pub fn Employees() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The employees and the company they were loaded for, set together.
    let employees = RwSignal::new((String::new(), Vec::<ppb::Employee>::new()));
    let show_inactive = RwSignal::new(false);
    // The employee being edited; `None` while adding.
    let editing = RwSignal::new(None::<String>);
    let name = RwSignal::new(String::new());
    let personnummer = RwSignal::new(String::new());
    let salary = RwSignal::new(String::new());
    let account = RwSignal::new("7210".to_owned());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let clear_form = move || {
        editing.set(None);
        name.set(String::new());
        personnummer.set(String::new());
        salary.set(String::new());
        account.set("7210".to_owned());
    };
    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = payroll_api()
                .list_employees(ppb::ListEmployeesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => employees.set((company_id, response.into_inner().employees)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or forms) on screen.
        employees.set((String::new(), Vec::new()));
        error.set(None);
        clear_form();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |e: ppb::Employee| {
        error.set(None);
        editing.set(Some(e.id));
        name.set(e.name);
        personnummer.set(e.personal_identity_number);
        salary.set(amount(e.monthly_salary));
        account.set(e.salary_account.to_string());
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose employees are on screen, not whatever is active now.
        let company_id = employees.with_untracked(|(id, _)| id.clone());
        // Not an amount: 0, which the server refuses with its own message.
        let monthly_salary = parse_amount(&salary.get_untracked()).unwrap_or(0);
        let salary_account = account.get_untracked().parse().unwrap_or(0);
        spawn_local(async move {
            let result = match editing.get_untracked() {
                None => payroll_api()
                    .add_employee(ppb::AddEmployeeRequest {
                        company_id,
                        name: name.get_untracked(),
                        personal_identity_number: personnummer.get_untracked(),
                        monthly_salary,
                        salary_account,
                    })
                    .await
                    .map(|_| ()),
                Some(employee_id) => payroll_api()
                    .update_employee(ppb::UpdateEmployeeRequest {
                        company_id,
                        employee_id,
                        name: name.get_untracked(),
                        monthly_salary,
                        salary_account,
                    })
                    .await
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    clear_form();
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Anställda"</h1>
            <ErrorAlert message=error />
            <Card title="Anställd">
                <form class="grid grid-cols-2 items-end gap-4" novalidate on:submit=save>
                    <Field label="Namn" id="employee_name" value=name />
                    <Show when=move || editing.get().is_none()>
                        <Field
                            label="Personnummer"
                            id="personal_identity_number"
                            value=personnummer
                            placeholder="ÅÅÅÅMMDD-NNNN"
                        />
                    </Show>
                    <Field label="Månadslön (kr)" id="monthly_salary" value=salary />
                    <Select label="Lönekonto" id="salary_account" value=account>
                        {SALARY_ACCOUNTS
                            .map(|(number, label)| {
                                view! { <option class=SELECT_OPTION value=number.to_string()>{label}</option> }
                            })
                            .collect_view()}
                    </Select>
                    <div class="col-span-2 flex gap-2">
                        <Button disabled=busy>
                            {move || if editing.get().is_some() { "Spara ändringar" } else { "Lägg till anställd" }}
                        </Button>
                        <Show when=move || editing.get().is_some()>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| clear_form()>"Avbryt"</Button>
                        </Show>
                    </div>
                </form>
            </Card>
            <Checkbox label="Visa inaktiva" id="show_inactive" checked=show_inactive />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Personnummer"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Månadslön"</th>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = employees.get();
                            list.into_iter()
                                .filter(|e| e.active || show_inactive.get())
                                .map(|e| (company_id.clone(), e))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, e)| {
                            (company_id.clone(), e.id.clone(), e.name.clone(), e.monthly_salary, e.salary_account, e.active)
                        }
                        let((company_id, employee))
                    >
                        <EmployeeRow company_id=company_id employee=employee edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn EmployeeRow(
    company_id: String,
    employee: ppb::Employee,
    edit: Callback<ppb::Employee>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let stored = StoredValue::new(employee.clone());
    let confirming = RwSignal::new(false);
    let deactivate = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ppb::DeactivateEmployeeRequest {
                company_id: company_id.get_value(),
                employee_id: stored.with_value(|e| e.id.clone()),
            };
            match payroll_api().deactivate_employee(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let account = SALARY_ACCOUNTS
        .iter()
        .find(|(number, _)| *number == employee.salary_account)
        .map_or_else(|| employee.salary_account.to_string(), |(_, label)| (*label).to_owned());

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{employee.name.clone()}</td>
            <td class=format!("{TABLE_CELL} tabular-nums")>{employee.personal_identity_number.clone()}</td>
            <td class=TABLE_AMOUNT_CELL>{amount(employee.monthly_salary)}</td>
            <td class=TABLE_CELL>{account}</td>
            <td class=TABLE_CELL>{if employee.active { "Aktiv" } else { "Inaktiv" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || stored.with_value(|e| e.active)>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(stored.get_value())>
                        "Redigera"
                    </Button>
                    <Show
                        when=move || confirming.get()
                        fallback=move || view! {
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>
                                "Inaktivera"
                            </Button>
                        }
                    >
                        <Button kind="button" on:click=deactivate>"Bekräfta inaktivering"</Button>
                    </Show>
                </Show>
            </td>
        </tr>
    }
}
```

The `key` lists every shown field, as `Accounts` does: `ppb::Employee` is not `Hash`, and an edited employee must re-render.

- [ ] **Step 5: Wire it in**

In `crates/web/src/pages/mod.rs`, add `mod employees;` and `pub use employees::Employees;`.

In `crates/web/src/app.rs`, add `Employees` to the `crate::pages::{…}` import, add the route:

```rust
                        <Route path=path!("/employees") view=|| view! { <SignedIn><Employees /></SignedIn> } />
```

after the `/opening-balances` route, and the menu link after "Kontoplan", inside the `<Show when=move || !companies.active.get().is_empty()>`:

```rust
                        <A href="/employees" attr:class="text-muted-foreground hover:text-foreground">"Anställda"</A>
```

- [ ] **Step 6: Build and lint both targets**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo clippy --workspace -- -D warnings`
Expected: PASS with no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/web
git commit -m "Add the Anställda page and Swedish texts for the payroll codes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 10: Frontend: Lönekörningar list and the Lönekörning page

**Files:**
- Modify: `crates/web/src/ui.rs` (`Checkbox` takes `String` label and id)
- Create: `crates/web/src/pages/payroll_runs.rs`
- Create: `crates/web/src/pages/payroll_run.rs`
- Modify: `crates/web/src/pages/mod.rs`
- Modify: `crates/web/src/app.rs`

**Interfaces:**
- Consumes: Task 9's `api::{payroll_api, ppb}`; `api::lpb::VoucherLine`; `format::{amount, parse_amount, today}`.
- Produces:
  - `pages::{PayrollRuns, PayrollRunPage}`.
  - `pages::payroll_runs::status_label(status: ppb::PayrollRunStatus, pay_date: &str, today: &str) -> &'static str`.
  - `pages::payroll_runs::fee_rate(basis_points: u32) -> String`, which turns `3142` into `"31,42 %"`.
  - `pages::payroll_runs::RunLines` (component).
  - Routes `/payroll-runs`, `/payroll-runs/new`, `/payroll-runs/:id`.

- [ ] **Step 1: Write the failing tests** — `crates/web/src/pages/payroll_runs.rs`, starting with only the tests and the two helpers' signatures:

```rust
//! The active company's payroll runs, newest pay date first.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ppb::PayrollRunStatus::*;

    #[test]
    fn a_finalized_run_is_to_be_booked_from_its_pay_date() {
        assert_eq!(status_label(Open, "2026-10-25", "2026-10-04"), "Öppen");
        assert_eq!(status_label(Finalized, "2026-10-25", "2026-10-24"), "Färdigställd");
        assert_eq!(status_label(Finalized, "2026-10-25", "2026-10-25"), "Att bokföra");
        assert_eq!(status_label(Booked, "2026-10-25", "2026-10-26"), "Bokförd");
    }

    #[test]
    fn fee_rates_are_shown_as_percent() {
        assert_eq!(fee_rate(3142), "31,42 %");
        assert_eq!(fee_rate(1021), "10,21 %");
        assert_eq!(fee_rate(0), "0,00 %");
    }
}
```

Add `mod payroll_runs;` to `pages/mod.rs`, then run `cargo test -p doris-web payroll_runs`.
Expected: FAIL to compile with "cannot find function `status_label`".

- [ ] **Step 2: Implement the list page and helpers** — put this above the tests in `payroll_runs.rs`:

```rust
use crate::active_company::Companies;
use crate::api::{lpb, payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, today};
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// The status as the user sees it on `today` (both `YYYY-MM-DD`).
pub fn status_label(status: ppb::PayrollRunStatus, pay_date: &str, today: &str) -> &'static str {
    match status {
        ppb::PayrollRunStatus::Open => "Öppen",
        ppb::PayrollRunStatus::Finalized if pay_date <= today => "Att bokföra",
        ppb::PayrollRunStatus::Finalized => "Färdigställd",
        ppb::PayrollRunStatus::Booked => "Bokförd",
        ppb::PayrollRunStatus::Unspecified => "",
    }
}

/// Basis points as a Swedish percentage: 3142 → "31,42 %".
pub fn fee_rate(basis_points: u32) -> String {
    format!("{},{:02} %", basis_points / 100, basis_points % 100)
}

#[component]
pub fn PayrollRuns() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let runs = RwSignal::new(Vec::<ppb::PayrollRun>::new());
    let error = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's runs on screen.
        runs.set(Vec::new());
        error.set(None);
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = payroll_api()
                .list_payroll_runs(ppb::ListPayrollRunsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => runs.set(response.into_inner().payroll_runs),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let today = today();

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">"Lönekörningar"</h1>
                <A href="/payroll-runs/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ny lönekörning"</A>
            </div>
            <ErrorAlert message=error />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Utbetalningsdag"</th>
                        <th class=TABLE_HEADER_CELL>"Text"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Brutto"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatt"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgift"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Netto"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL>"Ver"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || runs.get() key=|run| (run.id.clone(), run.status, run.text.clone(), run.pay_date.clone()) let(run)>
                        {
                            let sum = |amount: fn(&ppb::PayrollRunLine) -> i64| run.lines.iter().map(amount).sum::<i64>();
                            let locked = run.status() != ppb::PayrollRunStatus::Open;
                            let (gross, tax, fee, net) = (sum(|l| l.gross), sum(|l| l.tax), sum(|l| l.fee), sum(|l| l.net));
                            let shown = move |ore: i64| if locked { amount(ore) } else { "–".to_owned() };
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>
                                        <A href=format!("/payroll-runs/{}", run.id)>{run.pay_date.clone()}</A>
                                    </td>
                                    <td class=TABLE_CELL>{run.text.clone()}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(gross)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(tax)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{shown(fee)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{shown(net)}</td>
                                    <td class=TABLE_CELL>{status_label(run.status(), &run.pay_date, &today)}</td>
                                    <td class=TABLE_CELL>{run.voucher.as_ref().map(|v| v.number.to_string())}</td>
                                </tr>
                            }
                        }
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

/// A run's lines (with fee and net once locked) and the voucher it books.
#[component]
pub fn RunLines(lines: Vec<ppb::PayrollRunLine>, voucher_lines: Vec<lpb::VoucherLine>) -> impl IntoView {
    view! {
        <Table>
            <thead class=TABLE_HEAD>
                <tr class=TABLE_ROW>
                    <th class=TABLE_HEADER_CELL>"Anställd"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Brutto"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatt"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgiftssats"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgift"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Netto"</th>
                </tr>
            </thead>
            <tbody class=TABLE_BODY>
                {lines
                    .into_iter()
                    .map(|l| view! {
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>{l.employee_name}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.gross)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.tax)}</td>
                            <td class=TABLE_AMOUNT_CELL>{fee_rate(l.fee_rate)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.fee)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.net)}</td>
                        </tr>
                    })
                    .collect_view()}
            </tbody>
        </Table>
        <h2 class="text-xs/relaxed font-medium">"Verifikation"</h2>
        <Table>
            <thead class=TABLE_HEAD>
                <tr class=TABLE_ROW>
                    <th class=TABLE_HEADER_CELL>"Konto"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                </tr>
            </thead>
            <tbody class=TABLE_BODY>
                {voucher_lines
                    .into_iter()
                    .map(|l| view! {
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>{l.account}</td>
                            <td class=TABLE_AMOUNT_CELL>{(l.debit != 0).then(|| amount(l.debit))}</td>
                            <td class=TABLE_AMOUNT_CELL>{(l.credit != 0).then(|| amount(l.credit))}</td>
                        </tr>
                    })
                    .collect_view()}
            </tbody>
        </Table>
    }
}
```

If `For`'s `key` complains that `run.status` (an `i32`) and friends need `Hash`, they already are: the key is a tuple of `String`s and `i32`.

Run: `cargo test -p doris-web payroll_runs`
Expected: PASS.

- [ ] **Step 3: Let `Checkbox` take runtime labels** — in `crates/web/src/ui.rs`, change the signature:

```rust
pub fn Checkbox(label: &'static str, id: &'static str, checked: RwSignal<bool>) -> impl IntoView {
```

to:

```rust
pub fn Checkbox(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    checked: RwSignal<bool>,
) -> impl IntoView {
```

In the body, use `id=id.clone()` for the input's `id` and `name=id` for its `name`. Existing callers pass `&'static str`, which `into` accepts.

- [ ] **Step 4: Write the run page** — `crates/web/src/pages/payroll_run.rs`:

```rust
//! One payroll run. Öppen: a form (preview, save, finalize). Färdigställd:
//! read-only, with Öppna and Bokför (from the pay date). Bokförd:
//! read-only, with Backa bokföring.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, parse_amount, today};
use crate::pages::payroll_runs::{RunLines, status_label};
use crate::ui::{
    Button, Checkbox, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

/// One RPC of the read-only view (Öppna, Bokför, Bekräfta backning).
type Call = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), tonic::Status>>>>;

/// One employee's row in the form.
#[derive(Clone, Copy)]
struct Row {
    employee_id: StoredValue<String>,
    name: StoredValue<String>,
    included: RwSignal<bool>,
    gross: RwSignal<String>,
    tax: RwSignal<String>,
}

/// Active employees, plus those already in `run` (who may have been
/// deactivated since). A new run includes everyone at their monthly salary.
fn form_rows(employees: &[ppb::Employee], run: Option<&ppb::PayrollRun>) -> Vec<Row> {
    employees
        .iter()
        .filter_map(|e| {
            let line = run.and_then(|r| r.lines.iter().find(|l| l.employee_id == e.id));
            if !e.active && line.is_none() {
                return None;
            }
            Some(Row {
                employee_id: StoredValue::new(e.id.clone()),
                name: StoredValue::new(e.name.clone()),
                included: RwSignal::new(run.is_none() || line.is_some()),
                gross: RwSignal::new(amount(line.map_or(e.monthly_salary, |l| l.gross))),
                tax: RwSignal::new(line.map(|l| amount(l.tax)).unwrap_or_default()),
            })
        })
        .collect()
}

#[component]
pub fn PayrollRunPage() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // None on /payroll-runs/new. Another run is another mount of the page.
    let run_id = use_params_map().read_untracked().get("id");
    let navigate = StoredValue::new_local(use_navigate());
    let go = move |path: String| navigate.with_value(|nav| nav(&path, Default::default()));
    // The run as last loaded, and the company it was loaded for.
    let run = RwSignal::new(None::<ppb::PayrollRun>);
    let company = RwSignal::new(String::new());
    let rows = RwSignal::new(Vec::<Row>::new());
    let pay_date = RwSignal::new(String::new());
    let text = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<ppb::PreviewPayrollRunResponse>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let confirming = RwSignal::new(false);
    let run_id = StoredValue::new(run_id);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        let id = run_id.get_value();
        spawn_local(async move {
            let employees = payroll_api()
                .list_employees(ppb::ListEmployeesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let loaded = match id {
                Some(payroll_run_id) => Some(
                    payroll_api()
                        .get_payroll_run(ppb::PayrollRunRef {
                            company_id: company_id.clone(),
                            payroll_run_id,
                        })
                        .await,
                ),
                None => None,
            };
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            let employees = match employees {
                Ok(response) => response.into_inner().employees,
                Err(status) => return error.set(Some(describe(&status))),
            };
            let loaded = match loaded {
                Some(Ok(response)) => Some(response.into_inner()),
                Some(Err(status)) => return error.set(Some(describe(&status))),
                None => None,
            };
            rows.set(form_rows(&employees, loaded.as_ref()));
            pay_date.set(loaded.as_ref().map_or_else(today, |r| r.pay_date.clone()));
            text.set(loaded.as_ref().map(|r| r.text.clone()).unwrap_or_default());
            preview.set(None);
            confirming.set(false);
            company.set(company_id);
            run.set(loaded);
        });
    };
    Effect::new(move |previous: Option<String>| {
        let active = companies.active.get();
        // A run belongs to one company: on a switch, back to the list.
        if previous.is_some_and(|p| p != active) {
            go("/payroll-runs".to_owned());
        } else {
            load();
        }
        active
    });

    let draft = move || ppb::PayrollRunDraft {
        pay_date: pay_date.get_untracked(),
        text: text.get_untracked(),
        lines: rows
            .get_untracked()
            .iter()
            .filter(|r| r.included.get_untracked())
            .map(|r| ppb::PayrollRunLineInput {
                employee_id: r.employee_id.get_value(),
                // Not an amount: 0 or -1, which the server refuses with its own message.
                gross: parse_amount(&r.gross.get_untracked()).unwrap_or(0),
                tax: parse_amount(&r.tax.get_untracked()).unwrap_or(-1),
            })
            .collect(),
    };
    // The run on screen, for the company it was loaded for (or, for a new
    // run, the active one).
    let reference = move || ppb::PayrollRunRef {
        company_id: company.get_untracked(),
        payroll_run_id: run.with_untracked(|r| r.as_ref().map(|r| r.id.clone()).unwrap_or_default()),
    };
    // Runs `call` and reloads, showing its error.
    let act = move |call: Call| {
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match call.await {
                Ok(()) => load(),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    let preview_click = move |_| {
        let request = ppb::PreviewPayrollRunRequest {
            company_id: company.get_untracked(),
            draft: Some(draft()),
        };
        error.set(None);
        spawn_local(async move {
            match payroll_api().preview_payroll_run(request).await {
                Ok(response) => preview.set(Some(response.into_inner())),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    // Saves the form (creating the run if new), then finalizes if asked.
    let save = move |finalize: bool| {
        let (company_id, existing, draft) = (company.get_untracked(), run.with_untracked(|r| r.as_ref().map(|r| r.id.clone())), draft());
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let saved = match existing.clone() {
                None => payroll_api()
                    .create_payroll_run(ppb::CreatePayrollRunRequest {
                        company_id: company_id.clone(),
                        draft: Some(draft),
                    })
                    .await
                    .map(|r| r.into_inner().payroll_run_id),
                Some(payroll_run_id) => payroll_api()
                    .update_payroll_run(ppb::UpdatePayrollRunRequest {
                        run: Some(ppb::PayrollRunRef {
                            company_id: company_id.clone(),
                            payroll_run_id: payroll_run_id.clone(),
                        }),
                        draft: Some(draft),
                    })
                    .await
                    .map(|_| payroll_run_id),
            };
            let result = match saved {
                Ok(id) if finalize => payroll_api()
                    .finalize_payroll_run(ppb::PayrollRunRef {
                        company_id,
                        payroll_run_id: id.clone(),
                    })
                    .await
                    .map(|_| id),
                other => other,
            };
            busy.set(false);
            match result {
                Ok(id) if existing.is_none() => go(format!("/payroll-runs/{id}")),
                Ok(_) => load(),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let form = move || {
        view! {
            <div class="grid grid-cols-2 gap-4">
                <Field label="Utbetalningsdag" id="pay_date" value=pay_date kind="date" />
                <Field label="Text" id="payroll_run_text" value=text placeholder="Lön {månad år}" />
            </div>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Anställd"</th>
                        <th class=TABLE_HEADER_CELL>"Brutto (kr)"</th>
                        <th class=TABLE_HEADER_CELL>"Skatt (kr)"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || rows.get() key=|row| row.employee_id.get_value() let(row)>
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>
                                <Checkbox
                                    label=row.name.get_value()
                                    id=format!("include-{}", row.employee_id.get_value())
                                    checked=row.included
                                />
                            </td>
                            <td class=TABLE_CELL>
                                <TextInput label=format!("Brutto, {}", row.name.get_value()) value=row.gross inputmode="decimal" />
                            </td>
                            <td class=TABLE_CELL>
                                <TextInput label=format!("Skatt, {}", row.name.get_value()) value=row.tax inputmode="decimal" />
                            </td>
                        </tr>
                    </For>
                </tbody>
            </Table>
            <div class="flex gap-2">
                <Button variant=Variant::Ghost kind="button" disabled=busy on:click=preview_click>"Förhandsgranska"</Button>
                <Button variant=Variant::Ghost kind="button" disabled=busy on:click=move |_| save(false)>"Spara"</Button>
                <Button kind="button" disabled=busy on:click=move |_| save(true)>"Färdigställ"</Button>
            </div>
            {move || preview.get().map(|p| view! { <RunLines lines=p.lines voucher_lines=p.voucher_lines /> })}
        }
    };

    let locked = move |r: ppb::PayrollRun| {
        let due = r.pay_date <= today();
        let status = r.status();
        let booked = r.voucher.clone();
        view! {
            <p class="text-xs/relaxed">
                "Utbetalningsdag " {r.pay_date.clone()} " · " {status_label(status, &r.pay_date, &today())}
            </p>
            <RunLines lines=r.lines.clone() voucher_lines=r.voucher_lines.clone() />
            {match booked {
                None => view! {
                    <div class="flex items-center gap-2">
                        <Button variant=Variant::Ghost kind="button" disabled=busy on:click=move |_| {
                            let r = reference();
                            act(Box::pin(async move { payroll_api().reopen_payroll_run(r).await.map(|_| ()) }))
                        }>"Öppna"</Button>
                        <Button kind="button" disabled=Signal::derive(move || busy.get() || !due) on:click=move |_| {
                            let r = reference();
                            act(Box::pin(async move { payroll_api().book_payroll_run(r).await.map(|_| ()) }))
                        }>"Bokför"</Button>
                        {(!due).then(|| view! {
                            <span class="text-muted-foreground">{format!("Kan bokföras från {}", r.pay_date)}</span>
                        })}
                    </div>
                }
                .into_any(),
                Some(voucher) => view! {
                    <div class="flex items-center gap-2">
                        <A href="/vouchers">{format!("Ver {}", voucher.number)}</A>
                        <Show
                            when=move || confirming.get()
                            fallback=move || view! {
                                <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>
                                    "Backa bokföring"
                                </Button>
                            }
                        >
                            <span class="text-muted-foreground">
                                "En rättelse bokförs med dagens datum. Körningen blir färdigställd igen."
                            </span>
                            <Button kind="button" disabled=busy on:click=move |_| {
                                let r = reference();
                                act(Box::pin(async move { payroll_api().unbook_payroll_run(r).await.map(|_| ()) }))
                            }>"Bekräfta backning"</Button>
                        </Show>
                    </div>
                }
                .into_any(),
            }}
        }
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">
                {move || run.with(|r| r.as_ref().map_or_else(|| "Ny lönekörning".to_owned(), |r| r.text.clone()))}
            </h1>
            <ErrorAlert message=error />
            {move || match run.get() {
                Some(r) if r.status() != ppb::PayrollRunStatus::Open => locked(r).into_any(),
                _ => form().into_any(),
            }}
        </div>
    }
}
```

Notes for the implementer:
- `company` is set by `load`. On `/payroll-runs/new` it is the active company once employees are loaded. Until then, the buttons send an empty `company_id` and the server answers `company_not_found`.
- `act` takes a boxed future so that one helper serves Öppna, Bokför and Backa bokföring. If the borrow checker or `Send` bounds object (`spawn_local` needs no `Send`), use three small closures instead. Behaviour must stay the same: busy, call, reload on success, error text on failure.
- The form's `For` key is the employee id, so typed values survive re-renders.

- [ ] **Step 5: Wire it in**

In `crates/web/src/pages/mod.rs`, make the list module public, because the run page uses its helpers. Add `pub mod payroll_runs;`, replacing the `mod payroll_runs;` from Step 1, and `mod payroll_run;`. Then add `pub use payroll_run::PayrollRunPage;` and `pub use payroll_runs::PayrollRuns;`.

In `crates/web/src/app.rs`, add `PayrollRunPage, PayrollRuns` to the `crate::pages::{…}` import and these routes after `/employees`:

```rust
                        <Route path=path!("/payroll-runs") view=|| view! { <SignedIn><PayrollRuns /></SignedIn> } />
                        <Route path=path!("/payroll-runs/new") view=|| view! { <SignedIn><PayrollRunPage /></SignedIn> } />
                        <Route path=path!("/payroll-runs/:id") view=|| view! { <SignedIn><PayrollRunPage /></SignedIn> } />
```

and the menu link, before "Anställda":

```rust
                        <A href="/payroll-runs" attr:class="text-muted-foreground hover:text-foreground">"Lönekörningar"</A>
```

- [ ] **Step 6: Build and lint both targets, and check the wasm budget**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo clippy --workspace -- -D warnings && make dist`
Expected: PASS. `make dist` stays within `WASM_BUDGET`. If it doesn't, report the size before and after. Don't raise the budget.

- [ ] **Step 7: Commit**

```bash
git add crates/web
git commit -m "Add the Lönekörningar list and the payroll run page

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```

---

### Task 11: End-to-end tests and documentation

**Files:**
- Create: `e2e/tests/payroll.spec.ts`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the pages from Tasks 9–10, `e2e/tests/fixtures.ts` (`register`, `addCompany`, `test`, `expect`). `addCompany` creates a company whose first räkenskapsår is the current calendar year.

- [ ] **Step 1: Write the e2e tests** — `e2e/tests/payroll.spec.ts`:

```ts
import { addCompany, expect, register, test } from "./fixtures";
import type { Page } from "@playwright/test";

/** A date `days` from today in the browser's sense, as YYYY-MM-DD. */
function isoDate(days = 0): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function addEmployee(page: Page, name: string, personnummer: string, salary: string) {
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();
  await page.getByLabel("Namn").fill(name);
  await page.getByLabel("Personnummer").fill(personnummer);
  await page.getByLabel("Månadslön (kr)").fill(salary);
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^${name}`) })).toBeVisible();
}

/** A finalized run paying Åsa on `payDate`, with 8 000 kr tax. */
async function finalizeRun(page: Page, payDate: string) {
  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await page.getByLabel("Utbetalningsdag").fill(payDate);
  await page.getByLabel("Skatt, Åsa Öberg").fill("8000");
  await page.getByRole("button", { name: "Färdigställ" }).click();
  await expect(page).toHaveURL(/\/payroll-runs\/[0-9a-f-]{36}$/);
}

test("an employee is paid: previewed, finalized, booked and backed out", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await expect(page.getByLabel("Brutto, Åsa Öberg")).toHaveValue(/35\s000,00/);
  await page.getByLabel("Utbetalningsdag").fill(isoDate());
  await page.getByLabel("Skatt, Åsa Öberg").fill("8000");
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  // 35 000 kr × 31,42 % and the net pay.
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText(/10\s997,00/);
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText(/27\s000,00/);

  await page.getByRole("button", { name: "Färdigställ" }).click();
  await expect(page.getByText("Att bokföra")).toBeVisible();
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("link", { name: "Ver 1" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Öppna" })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  const voucher = page.getByRole("row", { name: /^1 / });
  await expect(voucher).toContainText("Lön");
  await voucher.getByRole("button", { name: "1" }).click();
  for (const account of ["7210", "2710", "1930", "7510", "2731"]) {
    await expect(page.getByText(new RegExp(`^${account} `))).toBeVisible();
  }

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(isoDate()) })).toContainText("Bokförd");
  await page.getByRole("link", { name: isoDate() }).click();
  await page.getByRole("button", { name: "Backa bokföring" }).click();
  await page.getByRole("button", { name: "Bekräfta backning" }).click();
  await expect(page.getByText("Att bokföra")).toBeVisible();
  await page.getByRole("button", { name: "Öppna" }).click();
  await expect(page.getByLabel("Skatt, Åsa Öberg")).toHaveValue(/8\s000,00/);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /^2 / })).toContainText("Rättelse av ver 1");
});

test("a run for a later pay date is finalized now and booked only from that date", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");
  const tomorrow = isoDate(1);

  await finalizeRun(page, tomorrow);

  await expect(page.getByText("Färdigställd")).toBeVisible();
  await expect(page.getByRole("button", { name: "Bokför" })).toBeDisabled();
  await expect(page.getByText(`Kan bokföras från ${tomorrow}`)).toBeVisible();
  await page.getByRole("button", { name: "Öppna" }).click();
  await page.getByLabel("Skatt, Åsa Öberg").fill("8100");
  await page.getByRole("button", { name: "Spara" }).click();
  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(tomorrow) })).toContainText("Öppen");
});

test("a grundbok rättelse of the payroll voucher makes the run finalized again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");
  await finalizeRun(page, isoDate());
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("link", { name: "Ver 1" })).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await page.getByRole("row", { name: /^1 / }).getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(page.getByRole("row", { name: /^1 / })).toContainText("Rättad av ver 2");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(isoDate()) })).toContainText("Att bokföra");
});

test("employees are refused in Swedish and edited without their personnummer", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();

  await page.getByLabel("Namn").fill("Åsa Öberg");
  await page.getByLabel("Personnummer").fill("19800101-1232");
  await page.getByLabel("Månadslön (kr)").fill("35000");
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("alert")).toHaveText("Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN).");

  await page.getByLabel("Personnummer").fill("19800101-1231");
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  const row = page.getByRole("row", { name: /^Åsa Öberg/ });
  await expect(row).toContainText("19800101-1231");
  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Personnummer")).toHaveCount(0);
  await page.getByLabel("Månadslön (kr)").fill("36000");
  await page.getByRole("button", { name: "Spara ändringar" }).click();
  await expect(row).toContainText(/36\s000,00/);

  await row.getByRole("button", { name: "Inaktivera" }).click();
  await row.getByRole("button", { name: "Bekräfta inaktivering" }).click();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toHaveCount(0);
  await page.getByLabel("Visa inaktiva").check();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Inaktiv");
});
```

If `getByLabel("Namn")` also matches the Leptos header or another field, use `{ exact: true }`. `getByLabel("Åsa Öberg")` would match the "Brutto, …" and "Skatt, …" inputs too. That is why the tests only use the prefixed labels.

- [ ] **Step 2: Run the e2e suite**

Run: `make e2e`
Expected: the four new tests and all existing ones pass. A failing selector is a test bug only if the page matches the spec. Otherwise fix the page.

- [ ] **Step 3: Update AGENTS.md**

In `AGENTS.md`, make these changes:

- Under **Layout**, add after the `crates/ledger` line:

  ```
  crates/payroll      doris-payroll: employees, payroll runs and arbetsgivaravgift
  ```

- Under **Event sourcing rules**, add after the underlag paragraph:

  ```
  - Payroll (`payroll-{company_id}`: employees and runs) has its own
    stream. A run is Öppen, Färdigställd or Bokförd; only an open run
    changes, a finalized one can be reopened, and finalizing may precede
    the pay date. Booking (`PayrollRunBooked`) needs `pay_date <= today`
    and books the voucher with `doris_ledger::record_voucher_in` in the
    payroll write transaction. A booked run is never reopened directly:
    its booking is backed out with a rättelse (`correct_voucher_in`,
    dated today but no later than its fiscal year's end, from the run or
    the grundbok), and the run is Färdigställd again. Whether a run is
    booked is derived from `vouchers.corrects`, never stored. Payroll
    tables have no foreign key to `vouchers`.
  - Arbetsgivaravgift (`doris_payroll::domain::employer_fee`) is in code
    from 2026: 31,42 %; 10,21 % for those 67 when the year began; 0 for
    born 1937 or earlier; 20,81 % on the first 25 000 kr a month for
    19–23-year-olds from 2026-04-01 to 2027-09-30. The youth cap counts
    booked runs only; a booking whose fee would change is refused
    (`payroll_run_outdated`).
  ```

- Under **API**, add `proto/doris/payroll/v1/payroll.proto` to the list of contracts, and add:

  ```
  - `PayrollService` codes are mapped in `crates/server/src/payroll.rs`
    (`status`, `domain_status`): `invalid_personal_identity_number`,
    `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`,
    `invalid_tax`, `empty_payroll_run`, `duplicate_payroll_run_line`,
    `duplicate_employee`, `employee_inactive`, `employee_not_found`,
    `payroll_run_not_found`, `payroll_run_not_open`,
    `payroll_run_not_finalized`, `payroll_run_booked`,
    `payroll_run_not_booked`, `payroll_run_not_due` and
    `payroll_run_outdated`. A run text over 200 characters is
    `invalid_voucher_text`; ledger refusals keep the ledger's codes.
  - A personnummer is personal data: never log it and never send it to
    an external service. It is stored as twelve digits and never changed
    on an employee.
  ```

- [ ] **Step 4: Final verification**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make dist && cargo test -p doris-ledger --test stress`
Expected: all pass, and `make dist` stays within `WASM_BUDGET`.

- [ ] **Step 5: Commit**

```bash
git add e2e/tests/payroll.spec.ts AGENTS.md
git commit -m "Cover payroll end to end and document it in AGENTS.md

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UWwvcoHwU1CbnFSQf1rdLv"
```
