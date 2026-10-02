# Plan 9: Opening Balances and Closing Fiscal Years Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Balance-sheet accounts carry their balance into the next fiscal year (ingående balanser), the first year's opening balances can be typed in, and a fiscal year can be closed (its result booked to equity and the year locked) and reopened with a reason.

**Architecture:**
- Everything lives in the existing `ledger-{company_id}-{fy_start}` stream. `LedgerEvent` gets three new variants: `OpeningBalancesSet`, `FiscalYearClosed` and `FiscalYearReopened`.
- Opening balances for later years are never stored. They are a query: the first year's typed-in balances plus every earlier year's lines on accounts 1000–2999.
- Closing books a normal voucher "Årets resultat" (8999 against 2099 or 2019) through the existing voucher projection. Reopening books a normal rättelse of it, so the gap-free numbering and its trigger cover both.
- Two new projections (`opening_balances`, `closed_fiscal_years`) in `migrations/0007_fiscal_year_closing.sql`.
- Four new RPCs. Two new pages (`/fiscal-years`, `/opening-balances`). The saldobalans, huvudbok and grundbok learn about opening balances and closed years.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, serde_json, tonic 0.14 gRPC-Web, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-02-ingaende-balanser-stangning-design.md`

## Global Constraints
- Amounts are `i64` öre everywhere, including proto `int64`. A balance is `debit - credit`, and a positive balance is a debit balance. A profit is a credit balance on the resultaträkning, so `result_of` is negative for a profit.
- Accounts 1000–2999 are the balansräkning and 3000–8999 the resultaträkning.
- The result account is 2019 for `EnskildFirma`, `Handelsbolag` and `Kommanditbolag`, and 2099 for every other legal form. The other side is always 8999.
- The result voucher's text is exactly `Årets resultat`, its date is the fiscal year's last day, and its number is `last_number + 1`.
- The reversal on reopening is a normal correction: text `Rättelse av ver {N}`, dated the fiscal year's last day, `corrects: Some(N)`.
- Event order: closing appends `[VoucherRecorded?, FiscalYearClosed]`; reopening appends `[FiscalYearReopened, VoucherRecorded?]`. A `VoucherRecorded` never sits between a `FiscalYearClosed` and the next `FiscalYearReopened`.
- Years close oldest first (the previous year must be closed) and only after they have ended (`end < today`). Years reopen newest first (the next year must not be closed).
- Opening balances: only the first fiscal year, at most 500 lines, accounts 1000–2999 that exist in the chart (inactive is fine), each account once, debit = credit. An empty list clears them. They can't change once the first year is closed.
- Existing events keep their meaning; `schema_version` stays 1.
- Every write is one `BEGIN IMMEDIATE` transaction (`doris_eventstore::begin`). Projections are updated in the same transaction and rebuild from `read_all`.
- New error codes, exactly:
  - `INVALID_ARGUMENT`: `not_balance_sheet_account`, `duplicate_account`, `opening_balances_unbalanced`, `invalid_reason`
  - `NOT_FOUND`: `fiscal_year_not_found`
  - `FAILED_PRECONDITION`: `fiscal_year_closed`, `fiscal_year_open`, `fiscal_year_not_ended`, `previous_fiscal_year_open`, `later_fiscal_year_closed`
  - More than 500 opening-balance lines reuse `invalid_voucher_lines`. An overflowing result is `internal`.
- User-visible text is Swedish and must match exactly:
  - "Räkenskapsår", "Öppet", "Stängt", "Stäng år", "Bekräfta stängning", "Öppna igen", "Anledning", "Bekräfta"
  - "Årets resultat bokförs som en verifikation och året låses för bokföring."
  - "Räkenskapsåret stängt." and "Räkenskapsåret stängt. Resultatet bokfördes som ver {N}."
  - "Räkenskapsåret öppnat igen."
  - "Ingående balanser", "Ingående balanser {start}", "Spara", "Ingående balanser sparade"
  - "Räkenskapsåret är stängt, så de ingående balanserna kan inte ändras."
  - "Ingående", "Utgående", "Ingående balans"
  - "Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära."
- URLs and identifiers are English: `/fiscal-years`, `/opening-balances`.
- The UI follows shadcn preset b1Gdz9bFY. Use the existing `Table`, `TABLE_*`, `Select`, `Button`, `TextInput`, `ErrorAlert` and `amount`. Invent no classes except layout utilities and the existing `text-xs/relaxed text-muted-foreground` text style.
- No new dependencies. The wasm budget is `WASM_BUDGET := 500000` (gzipped) in the `Makefile`. If `make dist` goes over it, stop and report; don't raise it.
- Lints: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- TDD: a failing test first for every behavior, and a commit per task. Commit messages are English and end with the session's attribution lines. Run the `verify` skill before the final commit of Task 10.

## Deviations from the spec (agreed simplifications)
- `set_opening_balances` in the domain takes no `is_first_year` flag, and there is no `not_first_fiscal_year` code: the API has no fiscal year parameter, so the server always targets the first year and the code could never be sent.
- The new commands have only the pool form (`close_fiscal_year(pool, …)`), not a `…_in(&mut conn, …)` twin. Nothing needs to hold their transaction open; add the twin when something does.
- `reopen_fiscal_year` returns the reversal's number (`Option<u32>`) so the stress test can count it. The RPC still answers with an empty message.

## Review Focus
1. **Reopening a year after the next year already shows its opening balances.** The result leaves 2099, so the next year's opening balances must go back to being off by exactly that result. *Test: Task 4, `later_years_open_with_the_balance_sheet_carried_forward` (the reopen step).*
2. **A booking racing a close.** However they interleave, no voucher may land in the year while it is closed, and numbering stays gap-free. *Test: Task 5, `no_voucher_lands_in_a_closed_year`.*
3. **Opening the IB form again.** Saved amounts must come back in the form in a form `parse_amount` reads back exactly, so saving again changes nothing. *Test: Task 7, `saved_amounts_read_back_as_typed`.*
4. **An account that has only an opening balance in a later year.** It must be listed in the saldobalans, and its huvudbok must show the "Ingående balans" row instead of "Inga transaktioner på kontot under räkenskapsåret." *Tests: Task 4, `an_accounts_ledger_starts_from_its_opening_balance`; Task 10 e2e (the 2081 step).*
5. **Switching the active company on Räkenskapsår.** The previous company's years and the "Räkenskapsåret stängt…" message must disappear. *Test: Task 10 e2e, `the fiscal years follow the active company`.*

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/ledger/src/domain.rs` | New events, errors and `Ledger` state; the lock in `record_voucher`/`correct_voucher`; `set_opening_balances`, `result_of`, `close_fiscal_year`, `reopen_fiscal_year`; `AccountLedger`, `FiscalYearStatus`; `running_balance` with an opening value. |
| `migrations/0007_fiscal_year_closing.sql` | `opening_balances` and `closed_fiscal_years` projections. |
| `crates/ledger/src/projections.rs` | Projects the new events; rebuild empties the new tables. |
| `crates/ledger/src/lib.rs` | `set_opening_balances`, `close_fiscal_year`, `reopen_fiscal_year`; shared `fiscal_year_at`. |
| `crates/ledger/src/queries.rs` | Opening balances in `trial_balance` and `account_ledger`; `closed` in `list_fiscal_years`; `opening_balances`. |
| `crates/ledger/tests/domain.rs`, `store.rs`, `stress.rs` | Tests at each level. |
| `proto/doris/ledger/v1/ledger.proto` | Four RPCs, three new fields. |
| `crates/server/src/ledger.rs` | Serves the RPCs and maps the new codes. |
| `crates/server/tests/ledger.rs` | gRPC-Web tests. |
| `crates/web/src/errors.rs` | Swedish text per new code. |
| `crates/web/src/fiscal_year.rs` | `opening_balances_preliminary`, `is_closed`, `closable`, `reopenable`. |
| `crates/web/src/voucher_lines.rs` (new) | The Konto/Debet/Kredit row editor shared by Ny verifikation and Ingående balanser. |
| `crates/web/src/pages/fiscal_years.rs` (new) | `/fiscal-years`. |
| `crates/web/src/pages/opening_balances.rs` (new) | `/opening-balances`. |
| `crates/web/src/pages/new_voucher.rs` | Uses `voucher_lines`. |
| `crates/web/src/pages/trial_balance.rs`, `account_ledger.rs`, `vouchers.rs` | Opening balances and closed years. |
| `crates/web/src/main.rs`, `pages/mod.rs`, `app.rs` | Module, routes, header link "Räkenskapsår". |
| `e2e/tests/fixtures.ts`, `e2e/tests/fiscal_year.spec.ts` (new), `e2e/tests/ledger.spec.ts` | End-to-end tests. |
| `AGENTS.md` | The opening-balance and closing rules. |

---

### Task 1: Ledger events, the lock and opening balances (domain)

**Files:**
- Modify: `crates/ledger/src/domain.rs`
- Modify: `crates/ledger/src/lib.rs` (`commit_voucher` only)
- Modify: `crates/ledger/src/projections.rs` (`apply_ledger` only)
- Modify: `crates/ledger/tests/stress.rs` (one destructuring)
- Modify: `crates/server/src/ledger.rs` (`domain_status` only)
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Produces:
  - New `DomainError` variants: `NotBalanceSheetAccount`, `DuplicateAccount`, `OpeningBalancesUnbalanced`, `InvalidReason`, `FiscalYearNotFound`, `FiscalYearClosed`, `FiscalYearOpen`, `FiscalYearNotEnded`, `PreviousFiscalYearOpen`, `LaterFiscalYearClosed`, `Overflow`. All of them are added now, so `domain_status` in the server is exhaustive once.
  - New `LedgerEvent` variants: `OpeningBalancesSet { lines: Vec<VoucherLine> }`, `FiscalYearClosed { result_voucher: Option<u32> }`, `FiscalYearReopened { reason: String }`.
  - `Ledger::is_closed(&self) -> bool`, `Ledger::opening_balances(&self) -> &[VoucherLine]`.
  - `pub fn set_opening_balances(ledger: &Ledger, chart: &Chart, lines: Vec<VoucherLine>) -> Result<LedgerEvent, DomainError>`.
  - `pub const MAX_OPENING_BALANCE_LINES: usize = 500;`

- [ ] **Step 1: Write the failing tests**

In `crates/ledger/tests/domain.rs`, replace `number_of` (it destructures a single-variant enum, which stops compiling once there are more variants):

```rust
fn number_of(event: &LedgerEvent) -> u32 {
    match event {
        LedgerEvent::VoucherRecorded { number, .. } => *number,
        other => panic!("not a voucher: {other:?}"),
    }
}
```

Then append:

```rust
/// The 2025 ledger after `given`, closed.
fn closed_year(mut given: Vec<LedgerEvent>) -> Ledger {
    given.push(LedgerEvent::FiscalYearClosed {
        result_voucher: None,
    });
    Ledger::from_events(first_year(), &given)
}

#[test]
fn a_closed_year_takes_no_voucher_and_no_correction() {
    let chart = seeded();
    let booked = record(&[], &chart, sale("2025-03-01", 100)).unwrap();
    let ledger = closed_year(vec![booked]);

    assert!(ledger.is_closed());
    assert_eq!(
        record_voucher(&ledger, &chart, sale("2025-03-02", 100)),
        Err(DomainError::FiscalYearClosed)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2025-03-02"), d("2026-10-02")),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn a_reopened_year_takes_vouchers_again() {
    let chart = seeded();
    let ledger = Ledger::from_events(
        first_year(),
        &[
            LedgerEvent::FiscalYearClosed {
                result_voucher: None,
            },
            LedgerEvent::FiscalYearReopened {
                reason: "Glömd faktura".into(),
            },
        ],
    );

    assert!(!ledger.is_closed());
    assert!(record_voucher(&ledger, &chart, sale("2025-03-02", 100)).is_ok());
}

fn lines(raw: &[(u32, i64, i64)]) -> Vec<VoucherLine> {
    raw.iter()
        .map(|&(account, debit, credit)| line(account, debit, credit))
        .collect()
}

#[test]
fn balanced_opening_balances_on_balance_sheet_accounts_are_set() {
    let chart = seeded();
    let ib = lines(&[(1930, 10_000, 0), (2081, 0, 10_000)]);

    let event = set_opening_balances(&Ledger::new(first_year()), &chart, ib.clone()).unwrap();

    assert_eq!(event, LedgerEvent::OpeningBalancesSet { lines: ib.clone() });
    let ledger = Ledger::from_events(first_year(), &[event]);
    assert_eq!(ledger.opening_balances(), &ib[..]);
    // An empty list clears them.
    assert_eq!(
        set_opening_balances(&ledger, &chart, Vec::new()),
        Ok(LedgerEvent::OpeningBalancesSet { lines: Vec::new() })
    );
}

#[test]
fn an_inactive_account_may_carry_an_opening_balance() {
    let mut chart = seeded();
    for event in set_account_active(&chart, n(1910), false).unwrap() {
        chart.apply(&event);
    }

    let ib = lines(&[(1910, 500, 0), (2081, 0, 500)]);

    assert!(set_opening_balances(&Ledger::new(first_year()), &chart, ib).is_ok());
}

#[test]
fn invalid_opening_balances_are_refused() {
    let chart = seeded();
    let open = Ledger::new(first_year());
    let too_many: Vec<VoucherLine> = (0..=MAX_OPENING_BALANCE_LINES)
        .map(|_| line(1930, 1, 0))
        .collect();
    for (ib, expected) in [
        (too_many, DomainError::InvalidVoucherLines),
        (lines(&[(1930, 0, 0), (2081, 0, 0)]), DomainError::InvalidAmount),
        (lines(&[(1930, 5, 5), (2081, 0, 0)]), DomainError::InvalidAmount),
        (
            lines(&[(1930, 100, 0), (3001, 0, 100)]),
            DomainError::NotBalanceSheetAccount,
        ),
        (
            lines(&[(1999, 100, 0), (2081, 0, 100)]),
            DomainError::AccountNotFound,
        ),
        (
            lines(&[(1930, 100, 0), (1930, 0, 100)]),
            DomainError::DuplicateAccount,
        ),
        (
            lines(&[(1930, 100, 0), (2081, 0, 99)]),
            DomainError::OpeningBalancesUnbalanced,
        ),
    ] {
        assert_eq!(
            set_opening_balances(&open, &chart, ib),
            Err(expected),
            "{expected:?}"
        );
    }
    assert_eq!(
        set_opening_balances(
            &closed_year(Vec::new()),
            &chart,
            lines(&[(1930, 100, 0), (2081, 0, 100)])
        ),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn closing_events_are_readable_json() {
    let as_json = |event: LedgerEvent| serde_json::to_value(event).unwrap();

    assert_eq!(
        as_json(LedgerEvent::OpeningBalancesSet {
            lines: vec![line(1930, 100, 0)]
        }),
        serde_json::json!({
            "type": "OpeningBalancesSet",
            "lines": [{"account": 1930, "debit": 100, "credit": 0}]
        })
    );
    assert_eq!(
        as_json(LedgerEvent::FiscalYearClosed {
            result_voucher: Some(7)
        }),
        serde_json::json!({"type": "FiscalYearClosed", "result_voucher": 7})
    );
    assert_eq!(
        as_json(LedgerEvent::FiscalYearReopened {
            reason: "Glömd faktura".into()
        }),
        serde_json::json!({"type": "FiscalYearReopened", "reason": "Glömd faktura"})
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger --test domain`
Expected: compile errors: no variant `FiscalYearClosed` in `LedgerEvent`, no function `set_opening_balances`, no method `is_closed`.

- [ ] **Step 3: Implement the domain**

In `crates/ledger/src/domain.rs`:

1. Change the `std::collections` import to `use std::collections::{BTreeMap, BTreeSet};`.

2. Append these variants to `DomainError`, after `CannotCorrectCorrection`:

```rust
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
```

3. Add the three variants to `LedgerEvent`, after `VoucherRecorded`:

```rust
    /// The first fiscal year's ingående balanser, replacing any before.
    /// Not a voucher: it takes no number. Later years' are derived.
    OpeningBalancesSet { lines: Vec<VoucherLine> },
    /// The year is locked. `result_voucher` is the "Årets resultat" voucher
    /// booked just before, if the result wasn't already on equity.
    FiscalYearClosed { result_voucher: Option<u32> },
    /// The year is open again; the result voucher's reversal follows.
    FiscalYearReopened { reason: String },
```

4. Replace the `Ledger` struct, `new` and `apply`, and add two getters:

```rust
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
```

(keep `from_events` as it is)

```rust
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
```

(keep `last_number`, `voucher` and `vouchers`)

5. Make `record_voucher` and `correct_voucher` refuse a closed year. Insert as the first statement of each function body:

```rust
    if ledger.closed {
        return Err(DomainError::FiscalYearClosed);
    }
```

6. Append:

```rust
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
```

- [ ] **Step 4: Keep the workspace compiling**

`LedgerEvent` now has four variants, so three places that destructured it with an irrefutable `let` must change.

In `crates/ledger/src/lib.rs`, `commit_voucher`, replace `let LedgerEvent::VoucherRecorded { number, .. } = event;` with this, which binds the number by reference so `event` can still be appended below:

```rust
    let &LedgerEvent::VoucherRecorded { number, .. } = &event else {
        unreachable!("record_voucher and correct_voucher decide a voucher");
    };
```

In `crates/ledger/src/projections.rs`, `apply_ledger`, wrap the current body in a `match` and leave the new events for Task 3:

```rust
async fn apply_ledger(
    conn: &mut SqliteConnection,
    company_id: &str,
    fiscal_year_start: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode()? {
        LedgerEvent::VoucherRecorded {
            number,
            date,
            text,
            lines,
            corrects,
        } => {
            // … the existing INSERT INTO vouchers, the loop over lines and the
            // UPDATE for `corrects`, unchanged …
        }
        // Projected in Task 3.
        LedgerEvent::OpeningBalancesSet { .. }
        | LedgerEvent::FiscalYearClosed { .. }
        | LedgerEvent::FiscalYearReopened { .. } => {}
    }
    Ok(())
}
```

In `crates/ledger/tests/stress.rs`, `assert_consistent`, replace the `let LedgerEvent::VoucherRecorded { number, .. } = event.decode().unwrap();` block with:

```rust
        if event.stream_id.starts_with("ledger-")
            && let LedgerEvent::VoucherRecorded { number, .. } = event.decode().unwrap()
        {
            numbers
                .entry(event.stream_id.clone())
                .or_default()
                .push(number);
        }
```

In `crates/server/src/ledger.rs`, append to the `match` in `domain_status`:

```rust
        NotBalanceSheetAccount => Status::invalid_argument("not_balance_sheet_account"),
        DuplicateAccount => Status::invalid_argument("duplicate_account"),
        OpeningBalancesUnbalanced => Status::invalid_argument("opening_balances_unbalanced"),
        InvalidReason => Status::invalid_argument("invalid_reason"),
        FiscalYearNotFound => Status::not_found("fiscal_year_not_found"),
        FiscalYearClosed => Status::failed_precondition("fiscal_year_closed"),
        FiscalYearOpen => Status::failed_precondition("fiscal_year_open"),
        FiscalYearNotEnded => Status::failed_precondition("fiscal_year_not_ended"),
        PreviousFiscalYearOpen => Status::failed_precondition("previous_fiscal_year_open"),
        LaterFiscalYearClosed => Status::failed_precondition("later_fiscal_year_closed"),
        Overflow => {
            tracing::error!("ledger: amount overflow");
            Status::internal("internal")
        }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-ledger && cargo test -p doris-server --test ledger`
Expected: all pass, including the stress test.

- [ ] **Step 6: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/ledger/src/domain.rs crates/ledger/src/lib.rs crates/ledger/src/projections.rs crates/ledger/tests/domain.rs crates/ledger/tests/stress.rs crates/server/src/ledger.rs
git commit -m "Lock a closed fiscal year and decide the first year's opening balances"
```

---

### Task 2: Closing and reopening a fiscal year (domain)

**Files:**
- Modify: `crates/ledger/src/domain.rs`
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Consumes: Task 1's events, errors and `Ledger` state.
- Produces:
  - `pub fn result_of(ledger: &Ledger) -> Option<i64>`: debit − credit over accounts 3000–8999. Negative is a profit. `None` on overflow.
  - `pub fn close_fiscal_year(ledger: &Ledger, previous_closed: Option<bool>, legal_form: LegalForm, today: Date) -> Result<Vec<LedgerEvent>, DomainError>`. `previous_closed` is `None` for the first fiscal year.
  - `pub fn reopen_fiscal_year(ledger: &Ledger, next_closed: bool, reason: &str) -> Result<Vec<LedgerEvent>, DomainError>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ledger/tests/domain.rs`:

```rust
const AFTER_2025: &str = "2026-10-02";

fn rent(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Hyra".into(),
        lines: vec![line(5010, ore, 0), line(1930, 0, ore)],
    }
}

/// The 2025 events after booking each command in turn.
fn year_with(cmds: Vec<RecordVoucher>) -> Vec<LedgerEvent> {
    let chart = seeded();
    let mut events = Vec::new();
    for cmd in cmds {
        let event = record(&events, &chart, cmd).unwrap();
        events.push(event);
    }
    events
}

fn close_2025(given: &[LedgerEvent], form: LegalForm) -> Vec<LedgerEvent> {
    close_fiscal_year(
        &Ledger::from_events(first_year(), given),
        None,
        form,
        d(AFTER_2025),
    )
    .unwrap()
}

#[test]
fn closing_a_profitable_year_books_8999_against_2099_on_its_last_day() {
    let given = year_with(vec![sale("2025-03-01", 1_000), rent("2025-04-01", 300)]);
    assert_eq!(
        result_of(&Ledger::from_events(first_year(), &given)),
        Some(-700)
    );

    let events = close_2025(&given, LegalForm::Aktiebolag);

    assert_eq!(
        events,
        vec![
            LedgerEvent::VoucherRecorded {
                number: 3,
                date: d("2025-12-31"),
                text: "Årets resultat".into(),
                lines: vec![line(8999, 700, 0), line(2099, 0, 700)],
                corrects: None,
            },
            LedgerEvent::FiscalYearClosed {
                result_voucher: Some(3)
            },
        ]
    );
}

#[test]
fn a_loss_is_booked_the_other_way_and_owners_taxed_personally_use_2019() {
    let given = year_with(vec![rent("2025-04-01", 300)]);
    for (form, equity) in [
        (LegalForm::Aktiebolag, 2099),
        (LegalForm::EkonomiskForening, 2099),
        (LegalForm::EnskildFirma, 2019),
        (LegalForm::Handelsbolag, 2019),
        (LegalForm::Kommanditbolag, 2019),
    ] {
        let events = close_2025(&given, form);
        let LedgerEvent::VoucherRecorded { lines, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(lines, &vec![line(8999, 0, 300), line(equity, 300, 0)], "{form:?}");
    }
}

#[test]
fn a_year_whose_result_is_already_on_equity_closes_without_a_voucher() {
    let booked_by_hand = year_with(vec![
        sale("2025-03-01", 1_000),
        RecordVoucher {
            date: d("2025-12-31"),
            text: "Årets resultat".into(),
            lines: vec![line(8999, 1_000, 0), line(2099, 0, 1_000)],
        },
    ]);
    let closed = vec![LedgerEvent::FiscalYearClosed {
        result_voucher: None,
    }];

    assert_eq!(close_2025(&booked_by_hand, LegalForm::Aktiebolag), closed);
    assert_eq!(close_2025(&[], LegalForm::Aktiebolag), closed);
}

#[test]
fn a_year_closes_once_after_it_has_ended_and_after_the_year_before() {
    let open = Ledger::new(first_year());
    let close = |ledger: &Ledger, previous: Option<bool>, today: &str| {
        close_fiscal_year(ledger, previous, LegalForm::Aktiebolag, d(today))
    };

    assert_eq!(
        close(&open, None, "2025-12-31"),
        Err(DomainError::FiscalYearNotEnded)
    );
    assert!(close(&open, None, "2026-01-01").is_ok());
    assert_eq!(
        close(&open, Some(false), AFTER_2025),
        Err(DomainError::PreviousFiscalYearOpen)
    );
    assert!(close(&open, Some(true), AFTER_2025).is_ok());
    assert_eq!(
        close(&closed_year(Vec::new()), Some(true), AFTER_2025),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn an_overflowing_result_is_an_error_not_a_wrong_voucher() {
    let huge = LedgerEvent::VoucherRecorded {
        number: 1,
        date: d("2025-03-01"),
        text: "x".into(),
        lines: vec![line(3001, 0, i64::MAX), line(3002, 0, i64::MAX)],
        corrects: None,
    };
    let ledger = Ledger::from_events(first_year(), &[huge]);

    assert_eq!(result_of(&ledger), None);
    assert_eq!(
        close_fiscal_year(&ledger, None, LegalForm::Aktiebolag, d(AFTER_2025)),
        Err(DomainError::Overflow)
    );
}

#[test]
fn reopening_comes_first_and_then_reverses_the_result_voucher() {
    let mut given = year_with(vec![sale("2025-03-01", 1_000)]);
    given.extend(close_2025(&given, LegalForm::Aktiebolag));
    let ledger = Ledger::from_events(first_year(), &given);

    let events = reopen_fiscal_year(&ledger, false, "  Glömd faktura ").unwrap();

    assert_eq!(
        events,
        vec![
            LedgerEvent::FiscalYearReopened {
                reason: "Glömd faktura".into()
            },
            LedgerEvent::VoucherRecorded {
                number: 3,
                date: d("2025-12-31"),
                text: "Rättelse av ver 2".into(),
                lines: vec![line(8999, 0, 1_000), line(2099, 1_000, 0)],
                corrects: Some(2),
            },
        ]
    );
}

#[test]
fn reopening_without_a_result_voucher_reverses_nothing() {
    assert_eq!(
        reopen_fiscal_year(&closed_year(Vec::new()), false, "Fel"),
        Ok(vec![LedgerEvent::FiscalYearReopened {
            reason: "Fel".into()
        }])
    );
}

#[test]
fn only_a_closed_year_without_a_closed_successor_reopens_and_only_with_a_reason() {
    assert_eq!(
        reopen_fiscal_year(&Ledger::new(first_year()), false, "Fel"),
        Err(DomainError::FiscalYearOpen)
    );
    assert_eq!(
        reopen_fiscal_year(&closed_year(Vec::new()), true, "Fel"),
        Err(DomainError::LaterFiscalYearClosed)
    );
    for bad in ["", "   ", &"å".repeat(201)] {
        assert_eq!(
            reopen_fiscal_year(&closed_year(Vec::new()), false, bad),
            Err(DomainError::InvalidReason),
            "{bad:?}"
        );
    }
    assert!(reopen_fiscal_year(&closed_year(Vec::new()), false, &"å".repeat(200)).is_ok());
}

#[test]
fn closing_again_after_a_reopen_books_the_new_result_without_gaps() {
    let chart = seeded();
    let mut given = year_with(vec![sale("2025-03-01", 1_000)]);
    given.extend(close_2025(&given, LegalForm::Aktiebolag)); // ver 2
    given.extend(
        reopen_fiscal_year(&Ledger::from_events(first_year(), &given), false, "Glömd hyra")
            .unwrap(),
    ); // ver 3 reverses ver 2
    let forgotten = record(&given, &chart, rent("2025-12-15", 400)).unwrap(); // ver 4
    given.push(forgotten);
    given.extend(close_2025(&given, LegalForm::Aktiebolag)); // ver 5

    let ledger = Ledger::from_events(first_year(), &given);
    assert!(ledger.is_closed());
    assert_eq!(
        ledger.vouchers().iter().map(|v| v.number).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(
        ledger.voucher(5).unwrap().lines,
        vec![line(8999, 600, 0), line(2099, 0, 600)]
    );
    assert_eq!(result_of(&ledger), Some(0));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger --test domain`
Expected: compile errors: cannot find `result_of`, `close_fiscal_year`, `reopen_fiscal_year`.

- [ ] **Step 3: Implement**

In `crates/ledger/src/domain.rs`, change the company import to `use doris_company::domain::{FiscalYear, LegalForm};`.

Pull the reversal out of `correct_voucher` so reopening can reuse it. Add:

```rust
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
```

and end `correct_voucher` with `Ok(reversal(ledger, original, date))` in place of the `Ok(LedgerEvent::VoucherRecorded { … })` it builds today.

Then append:

```rust
/// Debit minus credit over the resultaträkning (accounts 3000–8999), so a
/// profit is negative. `None` if it outgrows `i64`; a wrong figure would be
/// worse than an error.
pub fn result_of(ledger: &Ledger) -> Option<i64> {
    ledger
        .vouchers
        .iter()
        .flat_map(|v| &v.lines)
        .filter(|l| l.account.get() >= 3000)
        .try_fold(0_i64, |sum, l| sum.checked_add(l.debit)?.checked_sub(l.credit))
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
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-ledger --test domain`
Expected: all pass, including the existing correction tests (the reversal moved, its output didn't change).

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/ledger/src/domain.rs crates/ledger/tests/domain.rs
git commit -m "Decide closing a fiscal year with its result voucher, and reopening it"
```

---

### Task 3: Store the opening balances and closings

**Files:**
- Create: `migrations/0007_fiscal_year_closing.sql`
- Modify: `crates/ledger/src/projections.rs`
- Modify: `crates/ledger/src/lib.rs`
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: Task 1 and 2's domain functions.
- Produces (re-exported from `doris_ledger`):
  - `pub async fn set_opening_balances(pool: &SqlitePool, company_id: Uuid, actor: Uuid, lines: Vec<VoucherLine>) -> Result<()>`
  - `pub async fn close_fiscal_year(pool: &SqlitePool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, today: Date) -> Result<Option<u32>>`: the result voucher's number, if one was booked.
  - `pub async fn reopen_fiscal_year(pool: &SqlitePool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, reason: &str, today: Date) -> Result<Option<u32>>`: the reversal's number, if one was booked.
  - Tables `opening_balances(company_id, account, debit, credit)` and `closed_fiscal_years(company_id, fiscal_year_start, closed_at, closed_by)`.

- [ ] **Step 1: Write the failing tests**

In `crates/ledger/tests/store.rs`, extend the `doris_ledger::{…}` import with `close_fiscal_year, reopen_fiscal_year, set_opening_balances`. Then append:

```rust
fn ib(lines: &[(u32, i64, i64)]) -> Vec<VoucherLine> {
    lines
        .iter()
        .map(|&(account, debit, credit)| VoucherLine::new(account, debit, credit).unwrap())
        .collect()
}

#[tokio::test]
async fn opening_balances_are_set_and_replaced_in_the_first_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    set_opening_balances(&pool, id, anna, ib(&[(1930, 5_000, 0), (2081, 0, 5_000)]))
        .await
        .unwrap();

    assert_eq!(
        events_of(&pool, "ledger-").await,
        ["OpeningBalancesSet", "OpeningBalancesSet"]
    );
    assert_eq!(
        table(
            &pool,
            "SELECT account || ':' || debit || ':' || credit FROM opening_balances ORDER BY account"
        )
        .await,
        ["1930:5000:0", "2081:0:5000"]
    );
    assert!(matches!(
        set_opening_balances(&pool, id, anna, ib(&[(1930, 1, 0)])).await,
        Err(Error::Domain(DomainError::OpeningBalancesUnbalanced))
    ));
}

#[tokio::test]
async fn closing_books_the_result_and_locks_the_year_until_it_is_reopened() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();

    assert_eq!(
        close_fiscal_year(&pool, id, anna, start, today).await.unwrap(),
        Some(2)
    );
    assert!(matches!(
        record_voucher(&pool, id, anna, sale("2025-03-02", 1), today).await,
        Err(Error::Domain(DomainError::FiscalYearClosed))
    ));
    assert!(matches!(
        correct_voucher(&pool, id, anna, start, 1, d("2025-03-02"), today).await,
        Err(Error::Domain(DomainError::FiscalYearClosed))
    ));
    assert_eq!(
        table(
            &pool,
            "SELECT fiscal_year_start || ':' || closed_by FROM closed_fiscal_years"
        )
        .await,
        [format!("2025-01-01:{anna}")]
    );

    assert_eq!(
        reopen_fiscal_year(&pool, id, anna, start, "Glömd faktura", today)
            .await
            .unwrap(),
        Some(3)
    );
    assert!(
        table(&pool, "SELECT fiscal_year_start FROM closed_fiscal_years")
            .await
            .is_empty()
    );
    let vouchers = list_vouchers(&pool, id, anna, start).await.unwrap();
    assert_eq!(
        vouchers
            .iter()
            .map(|v| (v.number, v.text.as_str(), v.corrects))
            .collect::<Vec<_>>(),
        vec![
            (1, "Försäljning", None),
            (2, "Årets resultat", None),
            (3, "Rättelse av ver 2", Some(2)),
        ]
    );
    record_voucher(&pool, id, anna, sale("2025-03-02", 1), today)
        .await
        .unwrap();
}

#[tokio::test]
async fn years_close_oldest_first_and_reopen_newest_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    // 2025, 2026 and 2027 have ended; 2028 has not.
    let today = d("2028-02-01");
    let close = |start: &str| close_fiscal_year(&pool, id, anna, d(start), today);
    let reopen = |start: &str| reopen_fiscal_year(&pool, id, anna, d(start), "Fel", today);

    assert!(matches!(
        close("2026-01-01").await,
        Err(Error::Domain(DomainError::PreviousFiscalYearOpen))
    ));
    close("2025-01-01").await.unwrap();
    close("2026-01-01").await.unwrap();
    assert!(matches!(
        close("2028-01-01").await,
        Err(Error::Domain(DomainError::FiscalYearNotEnded))
    ));
    assert!(matches!(
        reopen("2025-01-01").await,
        Err(Error::Domain(DomainError::LaterFiscalYearClosed))
    ));
    assert!(matches!(
        reopen("2027-01-01").await,
        Err(Error::Domain(DomainError::FiscalYearOpen))
    ));
    reopen("2026-01-01").await.unwrap();
    reopen("2025-01-01").await.unwrap();
}

#[tokio::test]
async fn a_date_that_does_not_start_a_fiscal_year_is_not_found() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    // Mid-year, before the first year, and after today.
    for start in ["2025-02-01", "2024-01-01", "2027-01-01", "9999-01-01"] {
        assert!(
            matches!(
                close_fiscal_year(&pool, id, anna, d(start), today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "close {start}"
        );
        assert!(
            matches!(
                reopen_fiscal_year(&pool, id, anna, d(start), "Fel", today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "reopen {start}"
        );
    }
}

#[tokio::test]
async fn non_members_cannot_set_balances_close_or_reopen() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));

    assert!(matches!(
        set_opening_balances(&pool, id, bo, ib(&[(1930, 1, 0), (2081, 0, 1)])).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        close_fiscal_year(&pool, id, bo, start, today).await,
        Err(Error::NotFound)
    ));
    close_fiscal_year(&pool, id, anna, start, today)
        .await
        .unwrap();
    assert!(matches!(
        reopen_fiscal_year(&pool, id, bo, start, "Fel", today).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn opening_balances_and_closings_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();
    close_fiscal_year(&pool, id, anna, start, today).await.unwrap();
    reopen_fiscal_year(&pool, id, anna, start, "Fel", today)
        .await
        .unwrap();
    close_fiscal_year(&pool, id, anna, start, today).await.unwrap();
    let ib_sql = "SELECT company_id || account || ':' || debit || ':' || credit FROM opening_balances ORDER BY 1";
    let closed_sql = "SELECT company_id || fiscal_year_start || closed_at || closed_by FROM closed_fiscal_years ORDER BY 1";
    let lines_sql = "SELECT company_id || fiscal_year_start || number || line_no || account || debit || credit FROM voucher_lines ORDER BY 1";
    let (ib_rows, closed, lines) = (
        table(&pool, ib_sql).await,
        table(&pool, closed_sql).await,
        table(&pool, lines_sql).await,
    );

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(ib_rows.len(), 2);
    assert_eq!(closed.len(), 1);
    assert_eq!(table(&pool, ib_sql).await, ib_rows);
    assert_eq!(table(&pool, closed_sql).await, closed);
    assert_eq!(table(&pool, lines_sql).await, lines);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger --test store`
Expected: compile error, unresolved imports `close_fiscal_year`, `reopen_fiscal_year`, `set_opening_balances`.

- [ ] **Step 3: Add the migration**

Create `migrations/0007_fiscal_year_closing.sql`:

```sql
-- Projections of the ledger-* streams' opening balances and closings.
-- Rebuildable from events, like 0006.

-- The first fiscal year's ingående balanser. Later years' are derived from
-- these plus every earlier year's lines on accounts 1000-2999.
CREATE TABLE opening_balances (
    company_id TEXT    NOT NULL,
    account    INTEGER NOT NULL,
    debit      INTEGER NOT NULL,
    credit     INTEGER NOT NULL,
    PRIMARY KEY (company_id, account)
);

-- One row per closed fiscal year; reopening removes it.
CREATE TABLE closed_fiscal_years (
    company_id        TEXT NOT NULL,
    fiscal_year_start TEXT NOT NULL,
    closed_at         TEXT NOT NULL,
    closed_by         TEXT NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start)
);
```

`sqlx::migrate!` embeds the directory at compile time; `crates/eventstore/build.rs` makes a new file trigger a rebuild. If the table is still missing at runtime, `touch crates/eventstore/src/lib.rs`.

- [ ] **Step 4: Project the new events**

In `crates/ledger/src/projections.rs`, replace the `// Projected in Task 3.` arm of `apply_ledger` with:

```rust
        LedgerEvent::OpeningBalancesSet { lines } => {
            sqlx::query("DELETE FROM opening_balances WHERE company_id = ?")
                .bind(company_id)
                .execute(&mut *conn)
                .await?;
            for line in &lines {
                sqlx::query(
                    "INSERT INTO opening_balances (company_id, account, debit, credit)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(company_id)
                .bind(line.account.get())
                .bind(line.debit)
                .bind(line.credit)
                .execute(&mut *conn)
                .await?;
            }
        }
        LedgerEvent::FiscalYearClosed { .. } => {
            sqlx::query(
                "INSERT INTO closed_fiscal_years (company_id, fiscal_year_start, closed_at, closed_by)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(fiscal_year_start)
            .bind(&event.recorded_at)
            .bind(event.metadata.actor.as_deref().unwrap_or_default())
            .execute(&mut *conn)
            .await?;
        }
        LedgerEvent::FiscalYearReopened { .. } => {
            sqlx::query(
                "DELETE FROM closed_fiscal_years WHERE company_id = ? AND fiscal_year_start = ?",
            )
            .bind(company_id)
            .bind(fiscal_year_start)
            .execute(&mut *conn)
            .await?;
        }
```

In `rebuild_projections`, add `"DELETE FROM opening_balances"` and `"DELETE FROM closed_fiscal_years"` to the list of statements, before `"DELETE FROM voucher_lines"`.

- [ ] **Step 5: Add the commands**

In `crates/ledger/src/lib.rs`, add `VoucherLine` to the `use domain::{…}` list.

Add a helper and use it in `correct_voucher_in`:

```rust
/// The company's fiscal year starting on `start`, if there is one by
/// `today`. Checked against `today` first, so a far-future start never
/// steps fiscal years past the date limits.
fn fiscal_year_at(company: &Company, start: Date, today: Date) -> Option<FiscalYear> {
    if start > today {
        return None;
    }
    let fiscal_year = company.first_fiscal_year.containing(start);
    (fiscal_year.start == start).then_some(fiscal_year)
}
```

In `correct_voucher_in`, replace the block from the `// A year that starts after today…` comment through the second `return Err(DomainError::VoucherNotFound.into());` with:

```rust
    let fiscal_year = fiscal_year_at(&company, fiscal_year_start, today)
        .ok_or(DomainError::VoucherNotFound)?;
```

Then append the commands:

```rust
/// Replaces the first fiscal year's ingående balanser.
pub async fn set_opening_balances(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    lines: Vec<VoucherLine>,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = member_company(&mut tx, company_id, actor).await?;
    let chart = seeded_chart(&mut tx, company_id, actor).await?;
    let fiscal_year = company.first_fiscal_year;
    let (ledger, version) = load_ledger(&mut tx, company_id, fiscal_year).await?;
    let event = domain::set_opening_balances(&ledger, &chart, lines)?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(&mut tx, &stream, version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Closes the fiscal year starting on `fiscal_year_start`. Returns the
/// number of the "Årets resultat" voucher, if one was booked.
pub async fn close_fiscal_year(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    today: Date,
) -> Result<Option<u32>> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = member_company(&mut tx, company_id, actor).await?;
    let fiscal_year = fiscal_year_at(&company, fiscal_year_start, today)
        .ok_or(DomainError::FiscalYearNotFound)?;
    let (ledger, version) = load_ledger(&mut tx, company_id, fiscal_year).await?;
    let previous_closed = if fiscal_year == company.first_fiscal_year {
        None
    } else {
        let day_before = fiscal_year
            .start
            .yesterday()
            .expect("fiscal years are far from the date limits");
        let previous = company.first_fiscal_year.containing(day_before);
        Some(load_ledger(&mut tx, company_id, previous).await?.0.is_closed())
    };
    let events = domain::close_fiscal_year(&ledger, previous_closed, company.legal_form, today)?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(&mut tx, &stream, version, &events, actor).await?;
    tx.commit().await?;
    Ok(voucher_number(&events))
}

/// Reopens the fiscal year starting on `fiscal_year_start`. Returns the
/// number of the result voucher's reversal, if one was booked.
pub async fn reopen_fiscal_year(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    reason: &str,
    today: Date,
) -> Result<Option<u32>> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = member_company(&mut tx, company_id, actor).await?;
    let fiscal_year = fiscal_year_at(&company, fiscal_year_start, today)
        .ok_or(DomainError::FiscalYearNotFound)?;
    let (ledger, version) = load_ledger(&mut tx, company_id, fiscal_year).await?;
    // A next year without events is open.
    let next_closed = load_ledger(&mut tx, company_id, fiscal_year.next())
        .await?
        .0
        .is_closed();
    let events = domain::reopen_fiscal_year(&ledger, next_closed, reason)?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(&mut tx, &stream, version, &events, actor).await?;
    tx.commit().await?;
    Ok(voucher_number(&events))
}

/// The number of the voucher among `events`, if there is one.
fn voucher_number(events: &[LedgerEvent]) -> Option<u32> {
    events.iter().find_map(|e| match e {
        LedgerEvent::VoucherRecorded { number, .. } => Some(*number),
        _ => None,
    })
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-ledger`
Expected: all pass, including the existing correction tests that now go through `fiscal_year_at`.

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add migrations/0007_fiscal_year_closing.sql crates/ledger/src/projections.rs crates/ledger/src/lib.rs crates/ledger/tests/store.rs
git commit -m "Store opening balances and close and reopen fiscal years"
```

---

### Task 4: Read opening balances and closed years

**Files:**
- Modify: `crates/ledger/src/domain.rs` (`running_balance`, two new structs, `TrialBalanceRow.opening`)
- Modify: `crates/ledger/src/queries.rs`
- Modify: `crates/ledger/src/lib.rs` (re-exports)
- Modify: `crates/server/src/ledger.rs` (adapt to the new return types; proto fields come in Task 6)
- Test: `crates/ledger/tests/domain.rs`, `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: Task 3's projections.
- Produces:
  - `TrialBalanceRow` gains `pub opening: i64` (debit − credit). Field order: `account, name, opening, debit, credit`.
  - `pub struct AccountLedger { pub opening: i64, pub entries: Vec<LedgerEntry> }` (derives `Debug, Clone, PartialEq, Eq`).
  - `pub struct FiscalYearStatus { pub fiscal_year: FiscalYear, pub closed: bool }` (derives `Debug, Clone, Copy, PartialEq`).
  - `pub fn running_balance(opening: i64, lines: Vec<(Date, u32, String, i64, i64)>) -> Option<Vec<LedgerEntry>>`
  - `account_ledger(…) -> Result<AccountLedger>`, `list_fiscal_years(…) -> Result<Vec<FiscalYearStatus>>`
  - `pub async fn opening_balances(pool: &SqlitePool, company_id: Uuid, user_id: Uuid) -> Result<Vec<VoucherLine>>`: the first year's, by account.

- [ ] **Step 1: Write the failing tests**

In `crates/ledger/tests/domain.rs`, pass `0` as the new first argument in the three existing `running_balance(…)` calls, and append:

```rust
#[test]
fn the_running_balance_continues_from_the_opening_balance() {
    let entries = running_balance(500, vec![(day("2026-01-05"), 1, "a".into(), 0, 200)]).unwrap();
    assert_eq!(entries[0].balance, 300);
    assert_eq!(
        running_balance(i64::MAX, vec![(day("2026-01-05"), 1, "a".into(), 1, 0)]),
        None
    );
}
```

In `crates/ledger/tests/store.rs`:
- Add `opening_balances` to the `doris_ledger::{…}` import.
- In `the_trial_balance_sums_each_account_in_one_fiscal_year`, add `opening: 0,` to each of the three `TrialBalanceRow { … }` literals.
- In `an_accounts_ledger_runs_in_date_order_with_its_balance`, write `.entries` after the first `account_ledger(…).await.unwrap()` (`let entries = account_ledger(…).await.unwrap().entries;`), and use `.unwrap().entries.is_empty()` in the 1931 assertion.
- In `fiscal_years_run_from_the_first_to_the_current_newest_first`, map with `|y| y.fiscal_year.start` in both places.

Then append:

```rust
/// (account, opening, debit, credit) per row.
fn figures(rows: &[TrialBalanceRow]) -> Vec<(u32, i64, i64, i64)> {
    rows.iter()
        .map(|r| (r.account, r.opening, r.debit, r.credit))
        .collect()
}

#[tokio::test]
async fn later_years_open_with_the_balance_sheet_carried_forward() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let (y2025, y2026) = (d("2025-01-01"), d("2026-01-01"));
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    for cmd in [
        sale("2025-03-01", 1_000),
        booking("2025-04-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        sale("2026-02-01", 50),
    ] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    assert_eq!(
        figures(&trial_balance(&pool, id, anna, y2025).await.unwrap()),
        vec![
            (1930, 10_000, 1_000, 300),
            (2081, -10_000, 0, 0),
            (3001, 0, 0, 1_000),
            (5010, 0, 300, 0),
        ]
    );
    // 2025 is open, so its result (a 700 profit) isn't on 2099 yet and
    // 2026 opens off by exactly that.
    let open = trial_balance(&pool, id, anna, y2026).await.unwrap();
    assert_eq!(
        figures(&open),
        vec![(1930, 10_700, 50, 0), (2081, -10_000, 0, 0), (3001, 0, 0, 50)]
    );
    assert_eq!(open.iter().map(|r| r.opening).sum::<i64>(), 700);

    close_fiscal_year(&pool, id, anna, y2025, today).await.unwrap();
    let closed = trial_balance(&pool, id, anna, y2026).await.unwrap();
    assert_eq!(
        figures(&closed),
        vec![
            (1930, 10_700, 50, 0),
            (2081, -10_000, 0, 0),
            (2099, -700, 0, 0),
            (3001, 0, 0, 50),
        ]
    );
    assert_eq!(closed.iter().map(|r| r.opening).sum::<i64>(), 0);
    assert_eq!(closed[2].name, "Årets resultat");

    // Reopening reverses the result voucher: 2099 nets to 0 and drops out.
    reopen_fiscal_year(&pool, id, anna, y2025, "Glömd faktura", today)
        .await
        .unwrap();
    assert_eq!(
        figures(&trial_balance(&pool, id, anna, y2026).await.unwrap()),
        figures(&open)
    );
    // A year before the first has nothing, not even the opening balances.
    assert!(
        trial_balance(&pool, id, anna, d("2024-01-01"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_accounts_ledger_starts_from_its_opening_balance() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let y2026 = d("2026-01-01");
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2026-02-01", 50), today)
        .await
        .unwrap();

    let bank = account_ledger(&pool, id, anna, y2026, 1930).await.unwrap();
    assert_eq!(bank.opening, 11_000);
    assert_eq!(
        bank.entries.iter().map(|e| e.balance).collect::<Vec<_>>(),
        [11_050]
    );
    // Income accounts start every year at 0.
    assert_eq!(
        account_ledger(&pool, id, anna, y2026, 3001)
            .await
            .unwrap()
            .opening,
        0
    );
    // An account with only an opening balance has no entries.
    let capital = account_ledger(&pool, id, anna, y2026, 2081).await.unwrap();
    assert_eq!((capital.opening, capital.entries.len()), (-10_000, 0));
    let first_year = account_ledger(&pool, id, anna, d("2025-01-01"), 2081)
        .await
        .unwrap();
    assert_eq!(first_year.opening, -10_000);
}

#[tokio::test]
async fn fiscal_years_say_whether_they_are_closed() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let states = || async {
        list_fiscal_years(&pool, id, anna, today)
            .await
            .unwrap()
            .iter()
            .map(|y| (y.fiscal_year.start, y.closed))
            .collect::<Vec<_>>()
    };

    close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    assert_eq!(
        states().await,
        [(d("2026-01-01"), false), (d("2025-01-01"), true)]
    );

    reopen_fiscal_year(&pool, id, anna, d("2025-01-01"), "Fel", today)
        .await
        .unwrap();
    assert_eq!(
        states().await,
        [(d("2026-01-01"), false), (d("2025-01-01"), false)]
    );
}

#[tokio::test]
async fn the_first_years_opening_balances_are_read_back_by_account() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    assert!(opening_balances(&pool, id, anna).await.unwrap().is_empty());

    set_opening_balances(&pool, id, anna, ib(&[(2081, 0, 10_000), (1930, 10_000, 0)]))
        .await
        .unwrap();

    assert_eq!(
        opening_balances(&pool, id, anna).await.unwrap(),
        ib(&[(1930, 10_000, 0), (2081, 0, 10_000)])
    );
    assert!(matches!(
        opening_balances(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger`
Expected: compile errors: `running_balance` takes 1 argument, no field `opening` on `TrialBalanceRow`, no field `entries`, no field `fiscal_year`, unresolved `opening_balances`.

- [ ] **Step 3: Implement the domain types**

In `crates/ledger/src/domain.rs`:

Add `pub opening: i64,` to `TrialBalanceRow` between `name` and `debit`, and extend its doc comment:

```rust
/// One account's figures in a fiscal year (saldobalans). `opening` is its
/// ingående balans; the utgående balans is `opening + debit - credit`.
/// Balances are debit − credit; positive is a debit balance.
```

Replace `running_balance`:

```rust
/// Adds the running balance, starting from `opening`, to
/// `(date, number, text, debit, credit)` lines in the order given. `None` if
/// it outgrows `i64`, which no real ledger reaches; a wrong figure would be
/// worse than an error.
pub fn running_balance(
    opening: i64,
    lines: Vec<(Date, u32, String, i64, i64)>,
) -> Option<Vec<LedgerEntry>> {
    let mut balance = opening;
    // … the rest of the body is unchanged …
}
```

Add after `LedgerEntry`:

```rust
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
```

- [ ] **Step 4: Implement the queries**

In `crates/ledger/src/queries.rs`, extend the domain import with `AccountLedger, FiscalYearStatus` and append `VoucherLine` if it isn't there yet.

Replace `list_fiscal_years`:

```rust
/// The company's räkenskapsår from the first up to the one containing
/// `today`, newest first, each with whether it is closed.
pub async fn list_fiscal_years(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    today: Date,
) -> Result<Vec<FiscalYearStatus>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let closed: Vec<String> =
        sqlx::query_scalar("SELECT fiscal_year_start FROM closed_fiscal_years WHERE company_id = ?")
            .bind(company_id.to_string())
            .fetch_all(pool)
            .await?;
    let mut years = vec![company.first_fiscal_year];
    while let Some(last) = years.last().copied()
        && last.end < today
    {
        years.push(last.next());
    }
    Ok(years
        .into_iter()
        .rev()
        .map(|fiscal_year| FiscalYearStatus {
            closed: closed.contains(&fiscal_year.start.to_string()),
            fiscal_year,
        })
        .collect())
}
```

Replace `trial_balance`:

```rust
/// The saldobalans for one fiscal year, by account number: every account
/// with lines in the year or a non-zero ingående balans. The ingående balans
/// is the first year's opening balances plus every earlier year's lines on
/// accounts 1000–2999.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn trial_balance(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
) -> Result<Vec<TrialBalanceRow>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    // A year before the first has no opening balances either.
    let has_opening = fiscal_year_start >= company.first_fiscal_year.start;
    let (company_id, start) = (company_id.to_string(), fiscal_year_start.to_string());
    // One statement, so one snapshot. LEFT JOIN keeps an account even if the
    // chart somehow lacked it. SQLite's SUM fails on overflow, never wraps.
    let rows: Vec<(u32, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT t.account, COALESCE(a.name, ''), SUM(t.opening), SUM(t.debit), SUM(t.credit)
         FROM (
             SELECT account, debit - credit AS opening, 0 AS debit, 0 AS credit, 0 AS in_year
             FROM opening_balances WHERE company_id = ? AND ?
             UNION ALL
             SELECT account, debit - credit, 0, 0, 0 FROM voucher_lines
             WHERE company_id = ? AND fiscal_year_start < ? AND account < 3000
             UNION ALL
             SELECT account, 0, debit, credit, 1 FROM voucher_lines
             WHERE company_id = ? AND fiscal_year_start = ?
         ) t
         LEFT JOIN accounts a ON a.company_id = ? AND a.number = t.account
         GROUP BY t.account
         HAVING MAX(t.in_year) = 1 OR SUM(t.opening) != 0
         ORDER BY t.account",
    )
    .bind(&company_id)
    .bind(has_opening)
    .bind(&company_id)
    .bind(&start)
    .bind(&company_id)
    .bind(&start)
    .bind(&company_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(account, name, opening, debit, credit)| TrialBalanceRow {
            account,
            name,
            opening,
            debit,
            credit,
        })
        .collect())
}
```

Replace `account_ledger`:

```rust
/// One account's huvudbok for one fiscal year: its ingående balans, then its
/// lines by date, then voucher number, then line, each with the balance
/// after it.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn account_ledger(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    account: u32,
) -> Result<AccountLedger> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let account = AccountNumber::parse(account)?;
    let has_opening = fiscal_year_start >= company.first_fiscal_year.start;
    let (company_id, start) = (company_id.to_string(), fiscal_year_start.to_string());
    let number = i64::from(account.get());
    // One read transaction: the opening balance and the lines from the same
    // snapshot (WAL).
    let mut tx = pool.begin().await?;
    let opening: i64 = if account.get() < 3000 {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(debit - credit), 0) FROM (
                 SELECT debit, credit FROM opening_balances
                 WHERE company_id = ? AND account = ? AND ?
                 UNION ALL
                 SELECT debit, credit FROM voucher_lines
                 WHERE company_id = ? AND fiscal_year_start < ? AND account = ?
             )",
        )
        .bind(&company_id)
        .bind(number)
        .bind(has_opening)
        .bind(&company_id)
        .bind(&start)
        .bind(number)
        .fetch_one(&mut *tx)
        .await?
    } else {
        0
    };
    let lines: Vec<(String, u32, String, i64, i64)> = sqlx::query_as(
        "SELECT v.date, v.number, v.text, l.debit, l.credit
         FROM voucher_lines l
         JOIN vouchers v ON v.company_id = l.company_id
             AND v.fiscal_year_start = l.fiscal_year_start AND v.number = l.number
         WHERE l.company_id = ? AND l.fiscal_year_start = ? AND l.account = ?
         ORDER BY v.date, v.number, l.line_no",
    )
    .bind(&company_id)
    .bind(&start)
    .bind(number)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let entries = running_balance(
        opening,
        lines
            .into_iter()
            .map(|(date, number, text, debit, credit)| {
                let date = date.parse().expect("projected dates are valid");
                (date, number, text, debit, credit)
            })
            .collect(),
    )
    .ok_or(Error::Overflow)?;
    Ok(AccountLedger { opening, entries })
}
```

Append:

```rust
/// The first fiscal year's ingående balanser, by account.
pub async fn opening_balances(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<VoucherLine>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(u32, i64, i64)> = sqlx::query_as(
        "SELECT account, debit, credit FROM opening_balances WHERE company_id = ? ORDER BY account",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(account, debit, credit)| {
            VoucherLine::new(account, debit, credit).expect("projected accounts are valid")
        })
        .collect())
}
```

In `crates/ledger/src/lib.rs`, change the re-export to:

```rust
pub use queries::{
    account_ledger, list_accounts, list_fiscal_years, list_vouchers, opening_balances,
    trial_balance,
};
```

- [ ] **Step 5: Keep the server compiling**

In `crates/server/src/ledger.rs`:
- `list_fiscal_years`: map `|y| pb::FiscalYear { start: y.fiscal_year.start.to_string(), end: y.fiscal_year.end.to_string() }`.
- `get_account_ledger`: after `.map_err(status)?` add `.entries` before `.into_iter()`.

Task 6 sends `opening` and `closed` on the wire.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-ledger && cargo test -p doris-server --test ledger`
Expected: all pass.

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/ledger/src/domain.rs crates/ledger/src/queries.rs crates/ledger/src/lib.rs crates/ledger/tests/domain.rs crates/ledger/tests/store.rs crates/server/src/ledger.rs
git commit -m "Carry balance-sheet accounts into later years and list closed years"
```

---

### Task 5: Stress closing against bookings

**Files:**
- Test: `crates/ledger/tests/stress.rs`

**Interfaces:**
- Consumes: `close_fiscal_year`, `reopen_fiscal_year`, `list_fiscal_years` (returns `FiscalYearStatus`), `trial_balance`.

This task adds a test only. It is expected to pass at once, since Tasks 1–3 already serialize the writes; it guards the invariant from now on. If it fails, stop and report: that is a real bug in Tasks 1–3, not in the test.

- [ ] **Step 1: Write the test**

In `crates/ledger/tests/stress.rs`, extend the `doris_ledger::{…}` import with `close_fiscal_year, list_fiscal_years, reopen_fiscal_year, trial_balance`. Append:

```rust
/// After closers and writers raced on 2024: no voucher sits between a close
/// and the next reopen, numbers run 1..=n, and the projections agree.
async fn assert_closing_consistent(pool: &SqlitePool, company: Uuid, anna: Uuid, start: Date) {
    let mut conn = pool.acquire().await.unwrap();
    let (mut closed, mut numbers) = (false, Vec::new());
    for event in doris_eventstore::read_all(&mut conn, 0).await.unwrap() {
        if !event.stream_id.starts_with("ledger-") {
            continue;
        }
        match event.decode().unwrap() {
            LedgerEvent::VoucherRecorded { number, text, .. } => {
                assert!(!closed, "ver {number} ({text}) recorded in a closed year");
                numbers.push(number);
            }
            LedgerEvent::FiscalYearClosed { .. } => {
                assert!(!closed, "closed twice");
                closed = true;
            }
            LedgerEvent::FiscalYearReopened { .. } => {
                assert!(closed, "reopened an open year");
                closed = false;
            }
            LedgerEvent::OpeningBalancesSet { .. } => {}
        }
    }
    drop(conn);
    let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
    assert_eq!(numbers, expected);
    let vouchers = list_vouchers(pool, company, anna, start).await.unwrap();
    assert_eq!(vouchers.iter().map(|v| v.number).collect::<Vec<_>>(), numbers);
    let years = list_fiscal_years(pool, company, anna, d(TODAY)).await.unwrap();
    let status = years
        .iter()
        .find(|y| y.fiscal_year.start == start)
        .unwrap();
    assert_eq!(status.closed, closed);
    // A closed year's result is on equity, so its resultaträkning nets to 0.
    if closed {
        let rows = trial_balance(pool, company, anna, start).await.unwrap();
        let result: i64 = rows
            .iter()
            .filter(|r| r.account >= 3000)
            .map(|r| r.debit - r.credit)
            .sum();
        assert_eq!(result, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn no_voucher_lands_in_a_closed_year() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("closing.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let company = setup(&pool, anna).await;
    let start = d("2024-01-01");

    let tasks: Vec<_> = (0..16)
        .map(|task| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let today = d(TODAY);
                for op in 0..20 {
                    if task % 4 == 0 {
                        // Closers: close on even ops, reopen on odd ones.
                        let result = if op % 2 == 0 {
                            close_fiscal_year(&pool, company, anna, start, today)
                                .await
                                .map(|_| ())
                        } else {
                            reopen_fiscal_year(&pool, company, anna, start, "Stress", today)
                                .await
                                .map(|_| ())
                        };
                        match result {
                            Ok(())
                            | Err(Error::Domain(
                                DomainError::FiscalYearClosed | DomainError::FiscalYearOpen,
                            )) => {}
                            Err(other) => panic!("closer {task}/{op}: {other:?}"),
                        }
                    } else {
                        let cmd = voucher(d("2024-06-15"), 100 + op as i64, 3001);
                        match record_voucher(&pool, company, anna, cmd, today).await {
                            Ok(_) | Err(Error::Domain(DomainError::FiscalYearClosed)) => {}
                            Err(other) => panic!("writer {task}/{op}: {other:?}"),
                        }
                    }
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }

    assert_closing_consistent(&pool, company, anna, start).await;
    rebuild_projections(&pool).await.unwrap();
    assert_closing_consistent(&pool, company, anna, start).await;
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p doris-ledger --test stress`
Expected: both stress tests pass.

- [ ] **Step 3: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/ledger/tests/stress.rs
git commit -m "Stress-test closing a fiscal year against concurrent bookings"
```

---

### Task 6: Serve opening balances and closing over gRPC-Web

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`
- Modify: `crates/server/src/ledger.rs`
- Modify: `crates/web/src/fiscal_year.rs` (test helper only), `crates/web/src/pages/trial_balance.rs` (test helper only)
- Test: `crates/server/tests/ledger.rs`

**Interfaces:**
- Consumes: Tasks 3–4's functions.
- Produces (proto, package `doris.ledger.v1`):
  - `FiscalYear.closed` (bool, 3), `TrialBalanceRow.opening` (int64, 5), `GetAccountLedgerResponse.opening` (int64, 2).
  - RPCs `GetOpeningBalances`, `SetOpeningBalances`, `CloseFiscalYear`, `ReopenFiscalYear` with the messages below. `CloseFiscalYearResponse.result_voucher` is 0 when no voucher was booked.

- [ ] **Step 1: Write the failing tests**

In `crates/server/tests/ledger.rs`, replace `company` with a version that takes the first year, and keep the old name for existing callers:

```rust
/// Anna's company with the given first räkenskapsår.
async fn company_starting(server: &TestServer, session: &str, start: &str, end: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: start.into(),
                fiscal_year_end: end.into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
}

/// Anna's company, first räkenskapsår 2026.
async fn company(server: &TestServer, session: &str) -> String {
    company_starting(server, session, "2026-01-01", "2026-12-31").await
}
```

In `the_trial_balance_and_an_accounts_ledger_follow_the_vouchers`, add `opening: 0,` to both `pb::TrialBalanceRow` literals and replace `.into_inner().entries;` with `.into_inner();` plus `assert_eq!(ledger.opening, 0);` (rename the binding to `ledger` and compare `ledger.entries` to the existing `vec![…]`).

Append:

```rust
fn line(account: u32, debit: i64, credit: i64) -> pb::VoucherLine {
    pb::VoucherLine {
        account,
        debit,
        credit,
    }
}

fn close_of(company_id: &str, fiscal_year_start: &str) -> pb::CloseFiscalYearRequest {
    pb::CloseFiscalYearRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

fn reopen_of(company_id: &str, fiscal_year_start: &str, reason: &str) -> pb::ReopenFiscalYearRequest {
    pb::ReopenFiscalYearRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
        reason: reason.into(),
    }
}

#[tokio::test]
async fn opening_balances_and_a_closed_year_carry_into_the_next() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    // 2025 has ended by the time this runs.
    let id = company_starting(&server, &anna, "2025-01-01", "2025-12-31").await;
    let mut api = server.ledger();
    let ib = vec![line(1930, 10_000, 0), line(2081, 0, 10_000)];

    api.set_opening_balances(authed(
        pb::SetOpeningBalancesRequest {
            company_id: id.clone(),
            lines: ib.clone(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let saved = api
        .get_opening_balances(authed(
            pb::GetOpeningBalancesRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .lines;
    assert_eq!(saved, ib);
    let mut sale_2025 = sale(&id, 1_000);
    sale_2025.date = "2025-03-01".into();
    api.record_voucher(authed(sale_2025.clone(), &anna))
        .await
        .unwrap();

    let closed = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(closed.result_voucher, 2);

    let years = api
        .list_fiscal_years(authed(
            pb::ListFiscalYearsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .fiscal_years;
    let oldest = years.last().unwrap();
    assert_eq!((oldest.start.as_str(), oldest.closed), ("2025-01-01", true));
    assert!(!years[0].closed);

    let err = api
        .record_voucher(authed(sale_2025, &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "fiscal_year_closed".to_owned())
    );

    // 2026 opens with the balance sheet and the result on 2099.
    let rows = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner()
        .rows;
    assert_eq!(
        rows.iter().map(|r| (r.account, r.opening)).collect::<Vec<_>>(),
        vec![(1930, 11_000), (2081, -10_000), (2099, -1_000)]
    );
    let bank = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(bank.opening, 11_000);

    let err = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "company_not_found".to_owned())
    );
    api.reopen_fiscal_year(authed(reopen_of(&id, "2025-01-01", "Glömd faktura"), &anna))
        .await
        .unwrap();
}

#[tokio::test]
async fn closing_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    // 2024 and 2025 have both ended by the time this runs.
    let id = company_starting(&server, &anna, "2024-01-01", "2024-12-31").await;
    let mut api = server.ledger();
    let set = |lines: Vec<pb::VoucherLine>| {
        authed(
            pb::SetOpeningBalancesRequest {
                company_id: id.clone(),
                lines,
            },
            &anna,
        )
    };
    let expect = |err: tonic::Status, code: Code, message: &str| {
        assert_eq!(code_of(err), (code, message.to_owned()));
    };

    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(2081, 0, 99)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "opening_balances_unbalanced");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(3001, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "not_balance_sheet_account");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(1930, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "duplicate_account");

    let err = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "previous_fiscal_year_open");
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2024-01-01", "Fel"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_open");
    let err = api
        .close_fiscal_year(authed(close_of(&id, "2024-02-01"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::NotFound, "fiscal_year_not_found");
    let err = api
        .close_fiscal_year(authed(close_of(&id, "nonsense"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "invalid_date");

    api.close_fiscal_year(authed(close_of(&id, "2024-01-01"), &anna))
        .await
        .unwrap();
    api.close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap();
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2025-01-01", "  "), &anna))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "invalid_reason");
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2024-01-01", "Fel"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "later_fiscal_year_closed");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(2081, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_closed");

    // The newest listed year contains today, so it has never ended. Every
    // year between 2025 and it must be closed first, oldest first.
    let years = api
        .list_fiscal_years(authed(
            pb::ListFiscalYearsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .fiscal_years;
    let (current, ended) = years.split_first().unwrap();
    for year in ended.iter().rev().filter(|y| y.start.as_str() > "2025-01-01") {
        api.close_fiscal_year(authed(close_of(&id, &year.start), &anna))
            .await
            .unwrap();
    }
    let err = api
        .close_fiscal_year(authed(close_of(&id, &current.start), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_not_ended");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-server --test ledger`
Expected: compile errors: no field `opening` on `TrialBalanceRow`, no method `set_opening_balances` on the client.

- [ ] **Step 3: Extend the proto**

In `proto/doris/ledger/v1/ledger.proto`, add to `service LedgerService` after `GetAccountLedger`:

```proto
  // The first fiscal year's ingående balanser, by account.
  rpc GetOpeningBalances(GetOpeningBalancesRequest) returns (GetOpeningBalancesResponse);
  // Replaces them. Only while the first fiscal year is open.
  rpc SetOpeningBalances(SetOpeningBalancesRequest) returns (SetOpeningBalancesResponse);
  // Books the year's result (8999 against 2099 or 2019) and locks the year.
  // Years close oldest first, once they have ended.
  rpc CloseFiscalYear(CloseFiscalYearRequest) returns (CloseFiscalYearResponse);
  // Unlocks the newest closed year and reverses its result voucher.
  rpc ReopenFiscalYear(ReopenFiscalYearRequest) returns (ReopenFiscalYearResponse);
```

Change the existing messages:

```proto
message FiscalYear {
  string start = 1;
  string end = 2;
  bool closed = 3;
}
```

```proto
// The balance is debit - credit; positive is a debit balance. opening is the
// ingående balans; the utgående balans is opening + debit - credit.
message TrialBalanceRow {
  uint32 account = 1;
  string name = 2;
  int64 debit = 3;
  int64 credit = 4;
  int64 opening = 5;
}
```

```proto
// opening is the ingående balans the running balance starts from.
message GetAccountLedgerResponse {
  repeated LedgerEntry entries = 1;
  int64 opening = 2;
}
```

Append:

```proto
message GetOpeningBalancesRequest {
  string company_id = 1;
}

message GetOpeningBalancesResponse {
  repeated VoucherLine lines = 1;
}

message SetOpeningBalancesRequest {
  string company_id = 1;
  repeated VoucherLine lines = 2;
}

message SetOpeningBalancesResponse {}

message CloseFiscalYearRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}

// 0 when the result was already on equity and no voucher was booked.
message CloseFiscalYearResponse {
  uint32 result_voucher = 1;
}

message ReopenFiscalYearRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  string reason = 3;
}

message ReopenFiscalYearResponse {}
```

- [ ] **Step 4: Serve it**

In `crates/server/src/ledger.rs`, add two helpers after `voucher_message`, and use `line_message` inside `voucher_message` (`.lines.iter().map(line_message).collect()`):

```rust
fn line_message(l: &VoucherLine) -> pb::VoucherLine {
    pb::VoucherLine {
        account: l.account.get().into(),
        debit: l.debit,
        credit: l.credit,
    }
}

/// An out-of-range account is refused as `account_not_found`.
fn domain_lines(lines: &[pb::VoucherLine]) -> Result<Vec<VoucherLine>, Status> {
    lines
        .iter()
        .map(|l| VoucherLine::new(l.account, l.debit, l.credit))
        .collect::<Result<_, _>>()
        .map_err(domain_status)
}
```

In `record_voucher`, replace the `lines: req.lines.iter().map(…)…map_err(domain_status)?,` expression with `lines: domain_lines(&req.lines)?,`.

In `list_fiscal_years`, add `closed: y.closed,` to the `pb::FiscalYear`.
In `get_trial_balance`, add `opening: r.opening,` to the `pb::TrialBalanceRow`.
Replace `get_account_ledger`'s body after `let fiscal_year_start = …;` with:

```rust
        let ledger =
            doris_ledger::account_ledger(&self.pool, company, user, fiscal_year_start, req.account)
                .await
                .map_err(status)?;
        let entries = ledger
            .entries
            .into_iter()
            .map(|e| pb::LedgerEntry {
                date: e.date.to_string(),
                number: e.number,
                text: e.text,
                debit: e.debit,
                credit: e.credit,
                balance: e.balance,
            })
            .collect();
        Ok(Response::new(pb::GetAccountLedgerResponse {
            entries,
            opening: ledger.opening,
        }))
```

Add the four methods to `impl LedgerService for LedgerApi`:

```rust
    async fn get_opening_balances(
        &self,
        request: Request<pb::GetOpeningBalancesRequest>,
    ) -> Result<Response<pb::GetOpeningBalancesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let lines = doris_ledger::opening_balances(&self.pool, company, user)
            .await
            .map_err(status)?
            .iter()
            .map(line_message)
            .collect();
        Ok(Response::new(pb::GetOpeningBalancesResponse { lines }))
    }

    async fn set_opening_balances(
        &self,
        request: Request<pb::SetOpeningBalancesRequest>,
    ) -> Result<Response<pb::SetOpeningBalancesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let lines = domain_lines(&request.get_ref().lines)?;
        doris_ledger::set_opening_balances(&self.pool, company, user, lines)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetOpeningBalancesResponse {}))
    }

    async fn close_fiscal_year(
        &self,
        request: Request<pb::CloseFiscalYearRequest>,
    ) -> Result<Response<pb::CloseFiscalYearResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let start = date(&request.get_ref().fiscal_year_start)?;
        let result_voucher =
            doris_ledger::close_fiscal_year(&self.pool, company, user, start, today())
                .await
                .map_err(status)?;
        Ok(Response::new(pb::CloseFiscalYearResponse {
            result_voucher: result_voucher.unwrap_or(0),
        }))
    }

    async fn reopen_fiscal_year(
        &self,
        request: Request<pb::ReopenFiscalYearRequest>,
    ) -> Result<Response<pb::ReopenFiscalYearResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let start = date(&req.fiscal_year_start)?;
        doris_ledger::reopen_fiscal_year(&self.pool, company, user, start, &req.reason, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ReopenFiscalYearResponse {}))
    }
```

- [ ] **Step 5: Keep the web crate compiling**

The generated structs gained fields, so two test helpers in `doris-web` need them:
- `crates/web/src/fiscal_year.rs`, `tests::years`: add `closed: false,` to the `lpb::FiscalYear` literal.
- `crates/web/src/pages/trial_balance.rs`, `tests::row`: add `opening: 0,` to the `lpb::TrialBalanceRow` literal.

- [ ] **Step 6: Run the tests**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`

```bash
git add proto/doris/ledger/v1/ledger.proto crates/server/src/ledger.rs crates/server/tests/ledger.rs crates/web/src/fiscal_year.rs crates/web/src/pages/trial_balance.rs
git commit -m "Serve opening balances and closing fiscal years over gRPC-Web"
```

---

### Task 7: Web building blocks: error texts, year helpers, shared line editor

**Files:**
- Modify: `crates/web/src/errors.rs`
- Modify: `crates/web/src/fiscal_year.rs`
- Create: `crates/web/src/voucher_lines.rs`
- Modify: `crates/web/src/main.rs` (register the module)
- Modify: `crates/web/src/pages/new_voucher.rs`

**Interfaces:**
- Consumes: Task 6's generated `lpb` types (`FiscalYear.closed`).
- Produces:
  - `fiscal_year::opening_balances_preliminary(years: &[lpb::FiscalYear], start: &str) -> bool`
  - `fiscal_year::is_closed(years: &[lpb::FiscalYear], start: &str) -> bool`
  - `fiscal_year::closable(years: &[lpb::FiscalYear], today: &str) -> Option<String>`
  - `fiscal_year::reopenable(years: &[lpb::FiscalYear]) -> Option<String>`
  - `voucher_lines::Lines` (Copy) with `Lines::new()`, `.clear()`, `.fill(&[lpb::VoucherLine])`, `.request() -> Option<Vec<lpb::VoucherLine>>`
  - `voucher_lines::LineRows` component: props `lines: Lines`, `list: &'static str`
  - `voucher_lines::account_number(raw: &str) -> u32` (moved from `new_voucher.rs`)

- [ ] **Step 1: Write the failing tests**

In `crates/web/src/errors.rs`, add a test:

```rust
    #[test]
    fn closing_codes_have_swedish_messages() {
        for code in [
            "not_balance_sheet_account",
            "duplicate_account",
            "opening_balances_unbalanced",
            "invalid_reason",
            "fiscal_year_not_found",
            "fiscal_year_closed",
            "fiscal_year_open",
            "fiscal_year_not_ended",
            "previous_fiscal_year_open",
            "later_fiscal_year_closed",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("fiscal_year_closed"),
            "Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen."
        );
    }
```

In `crates/web/src/fiscal_year.rs`, add to `mod tests`:

```rust
    fn fy(start: &str, end: &str, closed: bool) -> lpb::FiscalYear {
        lpb::FiscalYear {
            start: start.into(),
            end: end.into(),
            closed,
        }
    }

    /// 2027 and 2026 open, 2025 (the first) closed; newest first.
    fn three_years() -> Vec<lpb::FiscalYear> {
        vec![
            fy("2027-01-01", "2027-12-31", false),
            fy("2026-01-01", "2026-12-31", false),
            fy("2025-01-01", "2025-12-31", true),
        ]
    }

    #[test]
    fn opening_balances_are_preliminary_while_the_year_before_is_open() {
        let ys = three_years();
        assert!(opening_balances_preliminary(&ys, "2027-01-01"));
        assert!(!opening_balances_preliminary(&ys, "2026-01-01"));
        // The first year has no year before it.
        assert!(!opening_balances_preliminary(&ys, "2025-01-01"));
        assert!(!opening_balances_preliminary(&ys, ""));
    }

    #[test]
    fn a_listed_year_is_closed_or_not() {
        let ys = three_years();
        assert!(is_closed(&ys, "2025-01-01"));
        assert!(!is_closed(&ys, "2026-01-01"));
        assert!(!is_closed(&ys, "2024-01-01"));
    }

    #[test]
    fn the_oldest_open_year_can_be_closed_once_it_has_ended() {
        let ys = three_years();
        assert_eq!(closable(&ys, "2027-03-01").as_deref(), Some("2026-01-01"));
        assert_eq!(closable(&ys, "2026-12-31"), None);
        let all_closed: Vec<_> = ys.iter().map(|y| fy(&y.start, &y.end, true)).collect();
        assert_eq!(closable(&all_closed, "2030-01-01"), None);
    }

    #[test]
    fn the_newest_closed_year_can_be_reopened() {
        let mut ys = three_years();
        assert_eq!(reopenable(&ys).as_deref(), Some("2025-01-01"));
        ys[1].closed = true;
        assert_eq!(reopenable(&ys).as_deref(), Some("2026-01-01"));
        assert_eq!(reopenable(&[fy("2025-01-01", "2025-12-31", false)]), None);
    }
```

Create `crates/web/src/voucher_lines.rs` with only its tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse_amount;

    #[test]
    fn account_number_takes_the_leading_digits() {
        assert_eq!(account_number("1930"), 1930);
        assert_eq!(
            account_number(" 1930 Företagskonto/checkkonto/affärskonto"),
            1930
        );
        assert_eq!(account_number("Företagskonto"), 0);
        assert_eq!(account_number(""), 0);
        assert_eq!(account_number("19x0"), 0);
    }

    #[test]
    fn an_empty_amount_field_is_zero_and_junk_is_unreadable() {
        assert_eq!(field_amount(" "), Some(0));
        assert_eq!(field_amount("1 250,50"), Some(125_050));
        assert_eq!(field_amount("1oo"), None);
    }

    #[test]
    fn saved_amounts_read_back_as_typed() {
        assert_eq!(side(0), "");
        for ore in [1, 125_050, 100_000_000_000] {
            assert_eq!(parse_amount(&side(ore)), Some(ore), "{ore}");
        }
    }
}
```

and register it in `crates/web/src/main.rs` with `mod voucher_lines;` after `mod ui;`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-web`
Expected: compile errors: cannot find `opening_balances_preliminary`, `is_closed`, `closable`, `reopenable`, `account_number`, `field_amount`, `side`.

- [ ] **Step 3: Add the error texts**

In `crates/web/src/errors.rs`, add before `"invalid_date" =>`:

```rust
        "not_balance_sheet_account" => {
            "Ingående balanser får bara finnas på balanskonton, 1000–2999."
        }
        "duplicate_account" => "Varje konto får bara förekomma en gång.",
        "opening_balances_unbalanced" => {
            "De ingående balanserna måste balansera: debet och kredit ska vara lika stora."
        }
        "invalid_reason" => "Anledningen måste vara 1–200 tecken.",
        "fiscal_year_not_found" => "Räkenskapsåret finns inte.",
        "fiscal_year_closed" => {
            "Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen."
        }
        "fiscal_year_open" => "Räkenskapsåret är redan öppet.",
        "fiscal_year_not_ended" => "Räkenskapsåret är inte slut än.",
        "previous_fiscal_year_open" => "Stäng föregående räkenskapsår först.",
        "later_fiscal_year_closed" => "Öppna det senare räkenskapsåret först.",
```

- [ ] **Step 4: Add the year helpers**

In `crates/web/src/fiscal_year.rs`, add after `opening_balances_missing` (which stays until Task 9):

```rust
/// Whether `start`'s opening balances are still preliminary: the year
/// before it (listed next, years are newest first) is open, so its result
/// is not on equity yet.
pub fn opening_balances_preliminary(years: &[lpb::FiscalYear], start: &str) -> bool {
    years
        .iter()
        .position(|y| y.start == start)
        .and_then(|i| years.get(i + 1))
        .is_some_and(|previous| !previous.closed)
}

/// Whether the listed year starting on `start` is closed.
pub fn is_closed(years: &[lpb::FiscalYear], start: &str) -> bool {
    years.iter().any(|y| y.start == start && y.closed)
}

/// The year that can be closed next: the oldest open one, once it has ended
/// (`end < today`, both `YYYY-MM-DD`).
pub fn closable(years: &[lpb::FiscalYear], today: &str) -> Option<String> {
    years
        .iter()
        .rev()
        .find(|y| !y.closed)
        .filter(|y| y.end.as_str() < today)
        .map(|y| y.start.clone())
}

/// The year that can be reopened: the newest closed one.
pub fn reopenable(years: &[lpb::FiscalYear]) -> Option<String> {
    years.iter().find(|y| y.closed).map(|y| y.start.clone())
}
```

- [ ] **Step 5: Move the line editor into `voucher_lines.rs`**

Put this above the tests in `crates/web/src/voucher_lines.rs`:

```rust
//! The rows of konteringar shared by Ny verifikation and Ingående balanser:
//! Konto, Debet and Kredit per row, a running total, and the request lines.

use crate::api::lpb;
use crate::format::{amount, parse_amount};
use crate::ui::{Button, TextInput, Variant};
use leptos::prelude::*;

/// The account number at the start of what was typed or picked from the
/// list ("1930 Företagskonto" → 1930). 0 when there is none; the server
/// refuses it as an unknown account.
pub fn account_number(raw: &str) -> u32 {
    raw.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// An empty amount field is 0; anything else must parse.
fn field_amount(raw: &str) -> Option<i64> {
    if raw.trim().is_empty() {
        Some(0)
    } else {
        parse_amount(raw)
    }
}

/// A saved amount as it goes back into a field: empty for 0, else
/// `1 234,50`, which `parse_amount` reads back exactly.
fn side(ore: i64) -> String {
    if ore == 0 { String::new() } else { amount(ore) }
}

#[derive(Clone, Copy)]
struct Line {
    id: usize,
    account: RwSignal<String>,
    debit: RwSignal<String>,
    credit: RwSignal<String>,
}

impl Line {
    fn is_blank(&self) -> bool {
        [self.account, self.debit, self.credit]
            .iter()
            .all(|s| s.get_untracked().trim().is_empty())
    }
}

/// The editable rows. `Copy`, so pages can hand it to closures freely.
#[derive(Clone, Copy)]
pub struct Lines {
    next_id: StoredValue<usize>,
    rows: RwSignal<Vec<Line>>,
}

impl Default for Lines {
    fn default() -> Self {
        Self::new()
    }
}

impl Lines {
    /// Two empty rows.
    pub fn new() -> Self {
        let lines = Self {
            next_id: StoredValue::new(0),
            rows: RwSignal::new(Vec::new()),
        };
        lines.clear();
        lines
    }

    /// Back to two empty rows. New ids, so the view rebuilds every row.
    pub fn clear(&self) {
        self.fill(&[]);
    }

    /// One row per saved line, topped up to at least two rows.
    pub fn fill(&self, saved: &[lpb::VoucherLine]) {
        let mut rows: Vec<Line> = saved
            .iter()
            .map(|l| self.line(l.account.to_string(), side(l.debit), side(l.credit)))
            .collect();
        while rows.len() < 2 {
            rows.push(self.line(String::new(), String::new(), String::new()));
        }
        self.rows.set(rows);
    }

    fn line(&self, account: String, debit: String, credit: String) -> Line {
        let id = self.next_id.get_value();
        self.next_id.set_value(id + 1);
        Line {
            id,
            account: RwSignal::new(account),
            debit: RwSignal::new(debit),
            credit: RwSignal::new(credit),
        }
    }

    fn add(&self) {
        let line = self.line(String::new(), String::new(), String::new());
        self.rows.update(|rows| rows.push(line));
    }

    /// Debit and credit totals of what is typed; unreadable amounts count
    /// as 0.
    fn totals(&self) -> (i64, i64) {
        self.rows.with(|rows| {
            rows.iter().fold((0_i64, 0_i64), |(d, c), line| {
                (
                    d + line.debit.with(|s| field_amount(s)).unwrap_or(0),
                    c + line.credit.with(|s| field_amount(s)).unwrap_or(0),
                )
            })
        })
    }

    /// The non-blank rows as request lines, or `None` if an amount can't be
    /// read.
    pub fn request(&self) -> Option<Vec<lpb::VoucherLine>> {
        self.rows
            .get_untracked()
            .iter()
            .filter(|l| !l.is_blank())
            .map(|line| {
                Some(lpb::VoucherLine {
                    account: account_number(&line.account.get_untracked()),
                    debit: field_amount(&line.debit.get_untracked())?,
                    credit: field_amount(&line.credit.get_untracked())?,
                })
            })
            .collect()
    }
}

/// The Konto, Debet and Kredit inputs of every row ("Konto, rad 1" …) with
/// "Ta bort", then "Lägg till rad" and the running "Debet · Kredit ·
/// Differens". `list` is the id of the page's account `<datalist>`.
#[component]
pub fn LineRows(lines: Lines, list: &'static str) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2 text-muted-foreground">
                <span>"Konto"</span>
                <span>"Debet"</span>
                <span>"Kredit"</span>
                <span></span>
            </div>
            <For each=move || { lines.rows.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, l)| (*i, l.id) let((index, line))>
                <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2">
                    <TextInput label=format!("Konto, rad {}", index + 1) value=line.account list=list />
                    <TextInput label=format!("Debet, rad {}", index + 1) value=line.debit inputmode="decimal" />
                    <TextInput label=format!("Kredit, rad {}", index + 1) value=line.credit inputmode="decimal" />
                    <Button
                        variant=Variant::Ghost
                        kind="button"
                        on:click=move |_| lines.rows.update(|rows| rows.retain(|other| other.id != line.id))
                    >
                        "Ta bort"
                    </Button>
                </div>
            </For>
            <div>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| lines.add()>"Lägg till rad"</Button>
            </div>
        </div>
        <p class="text-xs/relaxed text-muted-foreground">
            {move || {
                let (debit, credit) = lines.totals();
                format!("Debet {} · Kredit {} · Differens {}", amount(debit), amount(credit), amount(debit - credit))
            }}
        </p>
    }
}
```

- [ ] **Step 6: Make Ny verifikation use it**

In `crates/web/src/pages/new_voucher.rs`:
- Delete `account_number`, `field_amount`, `struct Line`, `impl Line`, the `add_line` and `totals` closures and the `#[cfg(test)] mod tests` (the test moved to `voucher_lines.rs`).
- Change the imports to:

```rust
use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::today;
use crate::ui::{Button, Card, ErrorAlert, Field};
use crate::voucher_lines::{LineRows, Lines};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
```

- Replace `let next_id = …;` and `let lines = RwSignal::new(vec![Line::new(0), Line::new(1)]);` with `let lines = Lines::new();`.
- Make `clear` only `text.set(String::new()); lines.clear();`.
- In `submit`, replace the `let mut request_lines = Vec::new(); for line in … { … }` block with:

```rust
        let Some(request_lines) = lines.request() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
```

- In the view, replace everything from `<div class="grid gap-2">` (the rows header) through the closing `</p>` of the "Debet · Kredit · Differens" paragraph with `<LineRows lines=lines list="accounts" />`. Keep the `<datalist id="accounts">` above it.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p doris-web`
Expected: all pass.

- [ ] **Step 8: Lint and commit**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`

```bash
git add crates/web/src/errors.rs crates/web/src/fiscal_year.rs crates/web/src/voucher_lines.rs crates/web/src/main.rs crates/web/src/pages/new_voucher.rs
git commit -m "Add closing error texts, fiscal year helpers and a shared line editor"
```

---

### Task 8: Räkenskapsår and Ingående balanser pages

**Files:**
- Create: `crates/web/src/pages/fiscal_years.rs`
- Create: `crates/web/src/pages/opening_balances.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`

**Interfaces:**
- Consumes: `use_fiscal_years`, `closable`, `reopenable`, `Lines`, `LineRows`, the Task 6 RPCs.
- Produces: routes `/fiscal-years` (`FiscalYears`) and `/opening-balances` (`OpeningBalances`), and the header link "Räkenskapsår".

The pages are exercised end to end in Task 10; their logic lives in the helpers tested in Task 7.

- [ ] **Step 1: Write the Räkenskapsår page**

Create `crates/web/src/pages/fiscal_years.rs`:

```rust
//! Räkenskapsår: the active company's fiscal years, closed and reopened
//! here. The server checks every rule; the buttons only show where they can
//! work.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{closable, reopenable, use_fiscal_years};
use crate::format::today;
use crate::ui::{
    Button, ErrorAlert, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table,
    TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn FiscalYears() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let error = RwSignal::new(None::<String>);
    let done = RwSignal::new(None::<String>);
    let (years, _) = use_fiscal_years(String::new(), error);
    // Never say "closed" about the previous company.
    Effect::new(move |_| {
        companies.active.track();
        done.set(None);
    });

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Räkenskapsår"</h1>
            <ErrorAlert message=error />
            {move || done.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Räkenskapsår"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || years.get() key=|y| (y.start.clone(), y.closed) let(fiscal_year)>
                        <FiscalYearRow fiscal_year=fiscal_year years=years error=error done=done />
                    </For>
                </tbody>
            </Table>
            <A href="/opening-balances" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ingående balanser"</A>
        </div>
    }
}

#[component]
fn FiscalYearRow(
    fiscal_year: lpb::FiscalYear,
    years: RwSignal<Vec<lpb::FiscalYear>>,
    error: RwSignal<Option<String>>,
    done: RwSignal<Option<String>>,
) -> impl IntoView {
    let companies = expect_context::<Companies>();
    let start = StoredValue::new(fiscal_year.start.clone());
    let confirming = RwSignal::new(false);
    let reopening = RwSignal::new(false);
    let reason = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let can_close = move || years.with(|ys| closable(ys, &today())) == Some(start.get_value());
    let can_reopen = move || years.with(|ys| reopenable(ys)) == Some(start.get_value());
    // Once the server has answered, the list says so; the row is rebuilt.
    let mark = move |closed: bool| {
        years.update(|ys| {
            for y in ys.iter_mut().filter(|y| y.start == start.get_value()) {
                y.closed = closed;
            }
        })
    };

    let close = move |_| {
        error.set(None);
        done.set(None);
        busy.set(true);
        let company_id = companies.active.get_untracked();
        spawn_local(async move {
            let result = ledger_api()
                .close_fiscal_year(lpb::CloseFiscalYearRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.get_value(),
                })
                .await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    done.set(Some(match response.into_inner().result_voucher {
                        0 => "Räkenskapsåret stängt.".to_owned(),
                        n => format!("Räkenskapsåret stängt. Resultatet bokfördes som ver {n}."),
                    }));
                    mark(true);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let reopen = move |_| {
        error.set(None);
        done.set(None);
        busy.set(true);
        let company_id = companies.active.get_untracked();
        spawn_local(async move {
            let result = ledger_api()
                .reopen_fiscal_year(lpb::ReopenFiscalYearRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.get_value(),
                    reason: reason.get_untracked(),
                })
                .await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => {
                    done.set(Some("Räkenskapsåret öppnat igen.".to_owned()));
                    mark(false);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{format!("{} – {}", fiscal_year.start, fiscal_year.end)}</td>
            <td class=TABLE_CELL>{if fiscal_year.closed { "Stängt" } else { "Öppet" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || can_close() && !confirming.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>"Stäng år"</Button>
                </Show>
                <Show when=move || confirming.get()>
                    <span class="inline-flex items-center gap-2 whitespace-normal">
                        <span class="text-xs/relaxed text-muted-foreground">"Årets resultat bokförs som en verifikation och året låses för bokföring."</span>
                        <Button kind="button" disabled=busy on:click=close>"Bekräfta stängning"</Button>
                    </span>
                </Show>
                <Show when=move || can_reopen() && !reopening.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| reopening.set(true)>"Öppna igen"</Button>
                </Show>
                <Show when=move || reopening.get()>
                    <span class="inline-flex items-center gap-2">
                        <TextInput label="Anledning" value=reason />
                        <Button kind="button" disabled=busy on:click=reopen>"Bekräfta"</Button>
                    </span>
                </Show>
            </td>
        </tr>
    }
}
```

- [ ] **Step 2: Write the Ingående balanser page**

Create `crates/web/src/pages/opening_balances.rs`:

```rust
//! Ingående balanser for the active company's first fiscal year, typed in
//! by a company that moves to Doris with history. Later years' are derived
//! by the server. Read-only once the first year is closed.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::use_fiscal_years;
use crate::format::amount;
use crate::ui::{
    Button, ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use crate::voucher_lines::{LineRows, Lines};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn OpeningBalances() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let error = RwSignal::new(None::<String>);
    let saved = RwSignal::new(None::<String>);
    let (years, _) = use_fiscal_years(String::new(), error);
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    // What the server holds; None until it has answered.
    let current = RwSignal::new(None::<Vec<lpb::VoucherLine>>);
    let lines = Lines::new();
    let busy = RwSignal::new(false);
    // The company this form was filled for; a save only ever goes there.
    let form_company = StoredValue::new(String::new());
    // Years are newest first, so the first year is the last one.
    let first = move || years.with(|ys| ys.last().cloned());

    Effect::new(move |_| {
        let company_id = companies.active.get();
        accounts.set(Vec::new());
        current.set(None);
        saved.set(None);
        lines.clear();
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = ledger_api();
            let balances = api
                .get_opening_balances(lpb::GetOpeningBalancesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let chart = api
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = chart {
                accounts.set(response.into_inner().accounts);
            }
            match balances {
                Ok(response) => {
                    let list = response.into_inner().lines;
                    lines.fill(&list);
                    current.set(Some(list));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        saved.set(None);
        let Some(request_lines) = lines.request() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        busy.set(true);
        let company_id = form_company.get_value();
        spawn_local(async move {
            let result = ledger_api()
                .set_opening_balances(lpb::SetOpeningBalancesRequest {
                    company_id: company_id.clone(),
                    lines: request_lines.clone(),
                })
                .await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => {
                    saved.set(Some("Ingående balanser sparade".into()));
                    current.set(Some(request_lines));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">
                {move || first().map(|y| format!("Ingående balanser {}", y.start)).unwrap_or_else(|| "Ingående balanser".into())}
            </h1>
            <ErrorAlert message=error />
            {move || saved.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Show
                when=move || first().is_some_and(|y| y.closed)
                fallback=move || view! {
                    <form class="grid gap-4" novalidate on:submit=submit>
                        <datalist id="balance_accounts">
                            {move || {
                                accounts
                                    .get()
                                    .into_iter()
                                    .filter(|a| a.number < 3000)
                                    .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                                    .collect_view()
                            }}
                        </datalist>
                        <LineRows lines=lines list="balance_accounts" />
                        <div>
                            <Button disabled=busy>"Spara"</Button>
                        </div>
                    </form>
                }
            >
                <p class="text-xs/relaxed text-muted-foreground">
                    "Räkenskapsåret är stängt, så de ingående balanserna kan inte ändras."
                </p>
                <Table>
                    <thead class=TABLE_HEAD>
                        <tr class=TABLE_ROW>
                            <th class=TABLE_HEADER_CELL>"Konto"</th>
                            <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                            <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        </tr>
                    </thead>
                    <tbody class=TABLE_BODY>
                        {move || {
                            current
                                .get()
                                .unwrap_or_default()
                                .into_iter()
                                .map(|l| view! {
                                    <tr class=TABLE_ROW>
                                        <td class=TABLE_CELL>{l.account}</td>
                                        <td class=TABLE_AMOUNT_CELL>{(l.debit != 0).then(|| amount(l.debit))}</td>
                                        <td class=TABLE_AMOUNT_CELL>{(l.credit != 0).then(|| amount(l.credit))}</td>
                                    </tr>
                                })
                                .collect_view()
                        }}
                    </tbody>
                </Table>
            </Show>
        </div>
    }
}
```

- [ ] **Step 3: Route them and link Räkenskapsår**

In `crates/web/src/pages/mod.rs`, add `mod fiscal_years;` and `mod opening_balances;` in alphabetical order, and `pub use fiscal_years::FiscalYears;` and `pub use opening_balances::OpeningBalances;`.

In `crates/web/src/app.rs`:
- Add `FiscalYears` and `OpeningBalances` to the `use crate::pages::{…}` list.
- After the `/trial-balance/:account` route, add:

```rust
                        <Route path=path!("/fiscal-years") view=|| view! { <SignedIn><FiscalYears /></SignedIn> } />
                        <Route path=path!("/opening-balances") view=|| view! { <SignedIn><OpeningBalances /></SignedIn> } />
```

- After the "Saldobalans" header link, add:

```rust
                        <A href="/fiscal-years" attr:class="text-muted-foreground hover:text-foreground">"Räkenskapsår"</A>
```

- [ ] **Step 4: Build and lint**

Run: `cargo test -p doris-web && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make web`
Expected: all pass, and the debug frontend builds.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/pages/fiscal_years.rs crates/web/src/pages/opening_balances.rs crates/web/src/pages/mod.rs crates/web/src/app.rs
git commit -m "Close and reopen fiscal years and type in opening balances in the web app"
```

---

### Task 9: Opening balances and closed years in the reports and the grundbok

**Files:**
- Modify: `crates/web/src/pages/trial_balance.rs`
- Modify: `crates/web/src/pages/account_ledger.rs`
- Modify: `crates/web/src/pages/vouchers.rs`
- Modify: `crates/web/src/fiscal_year.rs` (remove `opening_balances_missing` and its test)

**Interfaces:**
- Consumes: `opening_balances_preliminary`, `is_closed`, the Task 6 fields `TrialBalanceRow.opening`, `GetAccountLedgerResponse.opening`, `FiscalYear.closed`.
- Produces: `Part { rows, opening, debit, credit }` with `Part::closing()`, and `computed_result(&Part) -> i64` in `trial_balance.rs`.

- [ ] **Step 1: Write the failing tests**

In `crates/web/src/pages/trial_balance.rs`, `mod tests`, change `row` to take the opening balance, and replace both tests:

```rust
    fn row(account: u32, opening: i64, debit: i64, credit: i64) -> lpb::TrialBalanceRow {
        lpb::TrialBalanceRow {
            account,
            name: String::new(),
            opening,
            debit,
            credit,
        }
    }

    #[test]
    fn accounts_below_3000_belong_to_the_balance_sheet() {
        let (balance, income) = split(vec![
            row(1930, 500, 1000, 300),
            row(2999, -500, 0, 50),
            row(3000, 0, 0, 900),
            row(5010, 0, 250, 0),
        ]);

        assert_eq!(
            balance.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [1930, 2999]
        );
        assert_eq!(
            (balance.opening, balance.debit, balance.credit, balance.closing()),
            (0, 1000, 350, 650)
        );
        assert_eq!(
            income.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [3000, 5010]
        );
        assert_eq!(
            (income.opening, income.debit, income.credit, income.closing()),
            (0, 250, 900, -650)
        );
    }

    #[test]
    fn no_rows_give_two_empty_parts() {
        assert_eq!(split(Vec::new()), (Part::default(), Part::default()));
    }

    #[test]
    fn the_computed_result_leaves_out_8999_so_it_survives_closing() {
        let (_, income) = split(vec![
            row(3001, 0, 0, 1000),
            row(5010, 0, 300, 0),
            row(8999, 0, 700, 0),
        ]);
        assert_eq!(computed_result(&income), 700);
    }
```

In `crates/web/src/fiscal_year.rs`, delete the test `opening_balances_are_missing_after_the_first_year`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-web`
Expected: compile errors: no field `opening` on `Part`, no method `closing`, no function `computed_result`.

- [ ] **Step 3: Implement the saldobalans**

In `crates/web/src/pages/trial_balance.rs`:

Replace `Part`, its `impl` and `split`:

```rust
/// One part of the saldobalans and its totals.
#[derive(Debug, Default, PartialEq)]
pub struct Part {
    pub rows: Vec<lpb::TrialBalanceRow>,
    pub opening: i64,
    pub debit: i64,
    pub credit: i64,
}

impl Part {
    /// The utgående balans.
    pub fn closing(&self) -> i64 {
        self.opening + self.debit - self.credit
    }
}

/// Balansräkning (accounts 1000–2999) and resultaträkning (3000–8999).
// ponytail: i64 totals; a year's lines would need ~92 biljarder kronor to overflow.
pub fn split(rows: Vec<lpb::TrialBalanceRow>) -> (Part, Part) {
    let (mut balance, mut income) = (Part::default(), Part::default());
    for row in rows {
        let part = if row.account < 3000 {
            &mut balance
        } else {
            &mut income
        };
        part.opening += row.opening;
        part.debit += row.debit;
        part.credit += row.credit;
        part.rows.push(row);
    }
    (balance, income)
}

/// The year's result as a profit is positive, leaving out 8999: once the
/// year is closed, the result voucher on 8999 would cancel it to 0.
pub fn computed_result(income: &Part) -> i64 {
    -income
        .rows
        .iter()
        .filter(|r| r.account != 8999)
        .map(|r| r.debit - r.credit)
        .sum::<i64>()
}
```

Change the `crate::fiscal_year` import to `{FiscalYearSelect, is_closed, keep_year_in_url, opening_balances_preliminary, use_fiscal_years}`.

In `TrialBalance`:
- Replace `let missing = move || years.with(|ys| opening_balances_missing(ys, &year.get()));` with:

```rust
    let preliminary = move || years.with(|ys| opening_balances_preliminary(ys, &year.get()));
    let closed = move || years.with(|ys| is_closed(ys, &year.get()));
```

- Replace `<FiscalYearSelect years=years year=year />` and the `<Show when=missing>…</Show>` block with:

```rust
            <div class="flex items-end gap-4">
                <FiscalYearSelect years=years year=year />
                <Show when=closed>
                    <span class="pb-2 text-xs/relaxed text-muted-foreground">"Stängt"</span>
                </Show>
            </div>
            <Show when=preliminary>
                <p class="text-xs/relaxed text-muted-foreground">
                    "Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära."
                </p>
            </Show>
```

- In the rows closure, replace `let total = balance.balance() + income.balance();` and `let result = -income.balance();` with:

```rust
                    let total = balance.closing() + income.closing();
                    let result = computed_result(&income);
```

In `PartTable`:
- Replace `let (debit, credit, balance) = (part.debit, part.credit, part.balance());` with `let (opening, debit, credit, closing) = (part.opening, part.debit, part.credit, part.closing());`.
- Header cells become Konto, Namn, Ingående, Debet, Kredit, Utgående (the last four with `text-right`):

```rust
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Ingående"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Utgående"</th>
```

- Each row's amount cells become:

```rust
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.opening)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.opening + row.debit - row.credit)}</td>
```

- The "Summa" row becomes:

```rust
                    <tr class=TABLE_ROW>
                        <td class=format!("{TABLE_CELL} font-medium") colspan="2">"Summa"</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(opening)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(closing)}</td>
                    </tr>
```

In `crates/web/src/fiscal_year.rs`, delete `opening_balances_missing` (nothing uses it now).

- [ ] **Step 4: Implement the huvudbok**

In `crates/web/src/pages/account_ledger.rs`:
- Change `let entries = RwSignal::new(None::<Vec<lpb::LedgerEntry>>);` to hold the opening balance too, with the comment updated:

```rust
    // The opening balance and entries; None until the chosen year's arrive.
    let entries = RwSignal::new(None::<(i64, Vec<lpb::LedgerEntry>)>);
```

- In the fetch effect: `Ok(response) => { let r = response.into_inner(); entries.set(Some((r.opening, r.entries))) }`.
- In the view, replace `entries.get().map(|entries| {` with `entries.get().map(|(opening, entries)| {`, change the empty check to `if entries.is_empty() && opening == 0 {`, and the balance to `let balance = entries.last().map_or(opening, |e| e.balance);`.
- Insert the opening row as the first child of `<tbody class=TABLE_BODY>`:

```rust
                                {(opening != 0).then(|| view! {
                                    <tr class=TABLE_ROW>
                                        <td class=TABLE_CELL></td>
                                        <td class=TABLE_CELL></td>
                                        <td class=TABLE_CELL>"Ingående balans"</td>
                                        <td class=TABLE_AMOUNT_CELL></td>
                                        <td class=TABLE_AMOUNT_CELL></td>
                                        <td class=TABLE_AMOUNT_CELL>{amount(opening)}</td>
                                    </tr>
                                })}
```

- [ ] **Step 5: Implement the grundbok**

In `crates/web/src/pages/vouchers.rs`:
- Add `use crate::fiscal_year::is_closed;`.
- Wrap the `<div class="w-56"> <Select …> … </div>` year picker in `<div class="flex items-end gap-4">…</div>` and add after it, inside the flex:

```rust
                <Show when=move || years.with(|ys| is_closed(ys, &year.get()))>
                    <span class="pb-2 text-xs/relaxed text-muted-foreground">"Stängt"</span>
                </Show>
```

- In `VoucherRow`, before `let fiscal_year = StoredValue::new(fiscal_year);`, add `let closed = fiscal_year.as_ref().is_some_and(|y| y.closed);`, and change `can_correct` to:

```rust
    // A closed year takes no correction; the server refuses one anyway.
    let can_correct = voucher.corrects == 0 && voucher.corrected_by == 0 && !closed;
```

- [ ] **Step 6: Run the tests and lint**

Run: `cargo test -p doris-web && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/web/src/pages/trial_balance.rs crates/web/src/pages/account_ledger.rs crates/web/src/pages/vouchers.rs crates/web/src/fiscal_year.rs
git commit -m "Show opening balances and closed years in the reports and the grundbok"
```

---

### Task 10: End to end, docs and the wasm budget

**Files:**
- Modify: `e2e/tests/fixtures.ts` (`addCompany` takes a start)
- Create: `e2e/tests/fiscal_year.spec.ts`
- Modify: `e2e/tests/ledger.spec.ts` (two texts that Task 9 replaced)
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Let `addCompany` take the first year**

In `e2e/tests/fixtures.ts`, change `addCompany`:

```ts
/** Adds a company whose first räkenskapsår starts on `start` (a 1 January). */
export async function addCompany(page: Page, app: string, orgNr: string, name: string, start = "2026-01-01") {
  await page.goto(`${app}/companies`);
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
  await page.getByLabel("Organisationsnummer").fill(orgNr);
  await page.getByLabel("Företagsnamn").fill(name);
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Postort").fill("Stockholm");
  await page.getByLabel("Räkenskapsåret börjar").fill(start);
  await expect(page.getByText(`Räkenskapsåret slutar ${start.slice(0, 4)}-12-31.`)).toBeVisible();
  await page.getByLabel("Faktureringsmetoden").check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
}
```

- [ ] **Step 2: Update the texts Task 9 replaced**

In `e2e/tests/ledger.spec.ts`:
- In "the trial balance and an account's ledger show what was booked", replace `await expect(page.getByText(/Ingående balanser saknas/)).toHaveCount(0);` and its comment with:

```ts
  // The company's first year has no year before it to be preliminary about.
  await expect(page.getByText(/preliminära/)).toHaveCount(0);
```

- In "the chosen fiscal year stays in the URL and survives a reload", replace the company-creating block (from `await page.goto(\`${app}/companies\`);` to the `toBeVisible()` of the heading) with `await addCompany(page, app, "5560160680", "Exempel AB", "2025-01-01");`, and replace the two `getByText(/Ingående balanser saknas/)` assertions with `getByText(/preliminära/)` (the comment above the first becomes `// The newest year is the second, and the first is still open.`).

- [ ] **Step 3: Write the end-to-end tests**

Create `e2e/tests/fiscal_year.spec.ts`:

```ts
import type { Page } from "@playwright/test";
import { addCompany, expect, register, test } from "./fixtures";

// Last calendar year has always ended, so it can be closed.
const last = new Date().getFullYear() - 1;
const lastStart = `${last}-01-01`;
const nextStart = `${last + 1}-01-01`;

async function book(page: Page, app: string, date: string, kronor: string) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Försäljning");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
}

test("a year opens with balances, closes with its result and reopens", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);

  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await page.getByRole("link", { name: "Ingående balanser" }).click();
  await expect(page.getByRole("heading", { name: `Ingående balanser ${lastStart}` })).toBeVisible();
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("10000");
  await page.getByLabel("Konto, rad 2").fill("2081");
  await page.getByLabel("Kredit, rad 2").fill("10000");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("status")).toHaveText("Ingående balanser sparade");
  // They come back when the page is opened again.
  await page.reload();
  // amount() groups with a no-break space; \s matches it.
  await expect(page.getByLabel("Debet, rad 1")).toHaveValue(/^10\s000,00$/);

  await book(page, app, `${last}-06-01`, "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret stängt. Resultatet bokfördes som ver 2.");
  await expect(lastYear).toContainText("Stängt");

  await book(page, app, `${last}-06-02`, "100");
  await expect(page.getByRole("alert")).toHaveText("Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen.");

  await page.goto(`${app}/trial-balance?fy=${nextStart}`);
  await expect(page.getByRole("row", { name: /^1930 / })).toContainText("11 250,00");
  await expect(page.getByRole("row", { name: /^2099 / })).toContainText("-1 250,00");
  await expect(page.getByText(/preliminära/)).toHaveCount(0);
  // An account with only an opening balance shows it in its huvudbok.
  await page.getByRole("link", { name: "2081", exact: true }).click();
  await expect(page.getByRole("row", { name: /Ingående balans/ })).toContainText("-10 000,00");
  await expect(page.getByText("Inga transaktioner på kontot under räkenskapsåret.")).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await lastYear.getByRole("button", { name: "Öppna igen" }).click();
  await lastYear.getByLabel("Anledning").fill("Glömd faktura");
  await lastYear.getByRole("button", { name: "Bekräfta", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret öppnat igen.");
  await expect(lastYear).toContainText("Öppet");

  await page.goto(`${app}/trial-balance?fy=${nextStart}`);
  await expect(page.getByText("Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära.")).toBeVisible();

  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(lastStart);
  await expect(page.getByRole("row", { name: /^3 / })).toContainText("Rättelse av ver 2");
});

test("the fiscal years follow the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret stängt.");

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(lastYear).toHaveCount(0);
  await expect(page.getByRole("row", { name: /^2026-01-01/ })).toContainText("Öppet");
});
```

- [ ] **Step 4: Run them**

Run: `make e2e`
Expected: all Playwright tests pass, old and new. If "the grundbok and the chart fit without scrolling sideways" fails because of the extra header link, stop and report. Don't change the header's layout classes on your own.

- [ ] **Step 5: Document the rules**

In `AGENTS.md`:
- In the Layout block, change the `crates/ledger` line to `crates/ledger       doris-ledger: chart of accounts, vouchers, opening balances and year closing`.
- Replace the paragraph that starts "The saldobalans and huvudbok (`trial_balance`, `account_ledger`…" with:

```markdown
- The saldobalans and huvudbok (`trial_balance`, `account_ledger` in
  `crates/ledger/src/queries.rs`) are plain queries over the voucher
  projections, one fiscal year at a time. Ingående balanser are never
  stored for later years: they are the first year's typed-in
  `opening_balances` plus every earlier year's lines on accounts 1000–2999.
- Closing a year (`FiscalYearClosed`) first books "Årets resultat", 8999
  against 2099 (2019 for enskild firma, HB and KB), then locks the year:
  no voucher or rättelse goes into a closed year. Years close oldest first,
  once they have ended. Reopening (`FiscalYearReopened`, with a reason)
  comes before the reversal of that voucher and goes newest first, so no
  voucher is ever recorded while its year is closed.
```

- [ ] **Step 6: Check the wasm budget and run everything**

Run: `make test && make dist`
Expected: tests pass, and `make dist` prints the gzipped wasm size under 500000 bytes. Report the size. If it is over, stop and report.

- [ ] **Step 7: Verify and commit**

Run the `verify` skill against the release binary: open `/fiscal-years`, close an ended year, check the saldobalans of the next year, reopen it.

```bash
git add e2e/tests/fixtures.ts e2e/tests/fiscal_year.spec.ts e2e/tests/ledger.spec.ts AGENTS.md
git commit -m "Test closing and opening balances end to end and document the rules"
```
