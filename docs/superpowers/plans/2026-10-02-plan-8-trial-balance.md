# Plan 8: General Ledger and Trial Balance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Members can see a saldobalans (debit, credit and balance per account) and one account's huvudbok (its lines by date, with a running balance) for one fiscal year at a time.

**Architecture:**
- The step is read-only. It adds no events, no commands, no migration and no projection.
- `doris-ledger` gets two queries, `trial_balance` and `account_ledger`, which read the existing `voucher_lines`, `vouchers` and `accounts` projections. A pure `running_balance` in `domain.rs` computes the running balance.
- `LedgerService` gets two RPCs, `GetTrialBalance` and `GetAccountLedger`.
- The web app gets two pages, `/trial-balance` and `/trial-balance/:account`, plus a shared fiscal-year picker in `fiscal_year.rs`.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, tonic 0.14 gRPC-Web, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-02-huvudbok-saldobalans-design.md`

## Global Constraints
- Amounts are `i64` öre everywhere, including proto `int64`. A balance is `debit - credit`, and a positive balance is a debit balance.
- One fiscal year per request, chosen by `fiscal_year_start` (`YYYY-MM-DD`). There are no date ranges within a year.
- Corrected vouchers and their corrections are both counted. They cancel out.
- There are no opening balances (ingående balanser). A year's figures are that year's movements only.
- Membership is checked first in both queries (`doris_company::get_company`). A non-member gets `company_not_found`.
- No new error codes. The only ones used are `invalid_date`, `invalid_account_number`, `company_not_found` and `internal`.
- An `i64` overflow in a running balance is `doris_ledger::Error::Overflow` and maps to `internal`. It never panics and never wraps.
- User-visible text is Swedish and must match exactly:
  - "Saldobalans", "Balansräkning", "Resultaträkning", "Räkenskapsår"
  - "Konto", "Namn", "Debet", "Kredit", "Saldo", "Summa", "Datum", "Ver", "Text"
  - "Beräknat resultat", "Summa saldo", "Tillbaka till saldobalansen"
  - "Inga verifikationer under räkenskapsåret."
  - "Inga transaktioner på kontot under räkenskapsåret."
  - "Ingående balanser saknas än, så saldon för balansräkningens konton visar bara årets rörelser."
- URLs and identifiers are English: `/trial-balance`, `/trial-balance/:account`, query param `fy`.
- The UI follows shadcn preset b1Gdz9bFY. Use the existing `Table`, `TABLE_*`, `Select`, `ErrorAlert` and `amount`. Invent no classes except layout utilities.
- No new dependencies. The wasm budget is `WASM_BUDGET := 900000` in the `Makefile`. If `make dist` goes over it, stop and report; don't raise it.
- Lints: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- TDD: a failing test first for every behavior, and a commit per task. Commit messages are English and end with the session's attribution lines.

## Review Focus
1. **A voucher with a higher number but an earlier date.** The huvudbok must list it first (date order, then number), and the running balance must follow that order. *Test: Task 2, `an_accounts_ledger_runs_in_date_order_with_its_balance`.*
2. **A corrected voucher.** Both the original and the correction count, so they cancel, and the saldobalans still sums to 0. *Test: Task 2, `the_trial_balance_sums_each_account_in_one_fiscal_year`.*
3. **A hand-edited URL:** `/trial-balance/abc` or `?fy=nonsense`. A junk account must show the Swedish `invalid_account_number` text, and a junk `fy` must fall back to the newest year. *Tests: Task 4, `pick_year_*`; Task 6 e2e `a junk account in the URL shows a Swedish error`.*
4. **Switching the active company on the saldobalans page.** The previous company's rows must disappear, and a late answer for the old company must never be shown. *Test: Task 6 e2e `the trial balance follows the active company`.*
5. **The notice about missing opening balances** must show for every year except the oldest and never for a company's first year. *Tests: Task 4, `opening_balances_are_missing_after_the_first_year`; Task 6 e2e asserts it is absent in the first year.*

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/ledger/src/domain.rs` | Adds `TrialBalanceRow`, `LedgerEntry` and the pure `running_balance`. |
| `crates/ledger/src/lib.rs` | Adds `Error::Overflow` and re-exports the two new queries. |
| `crates/ledger/src/queries.rs` | Adds `trial_balance` and `account_ledger`. |
| `crates/ledger/tests/domain.rs` | Tests for `running_balance`. |
| `crates/ledger/tests/store.rs` | Tests for both queries against SQLite. |
| `proto/doris/ledger/v1/ledger.proto` | Adds `GetTrialBalance` and `GetAccountLedger`. |
| `crates/server/src/ledger.rs` | Serves both RPCs and maps `Error::Overflow`. |
| `crates/server/tests/ledger.rs` | gRPC-Web tests for both RPCs. |
| `crates/web/src/fiscal_year.rs` | Adds `pick_year`, `opening_balances_missing`, `use_fiscal_years` and `FiscalYearSelect`. |
| `crates/web/src/pages/trial_balance.rs` | The `/trial-balance` page plus the pure `split`. |
| `crates/web/src/pages/account_ledger.rs` | The `/trial-balance/:account` page. |
| `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs` | Routes and the header link "Saldobalans". |
| `crates/web/src/ui.rs` | Adds `TABLE_AMOUNT_CELL`. |
| `e2e/tests/ledger.spec.ts` | End-to-end tests for both pages. |
| `AGENTS.md` | One line on the reports. |

---

### Task 1: Running balance (pure)

**Files:**
- Modify: `crates/ledger/src/domain.rs` (append after `Voucher`)
- Test: `crates/ledger/tests/domain.rs` (append)

**Interfaces:**
- Produces:
  - `pub struct TrialBalanceRow { pub account: u32, pub name: String, pub debit: i64, pub credit: i64 }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub struct LedgerEntry { pub date: Date, pub number: u32, pub text: String, pub debit: i64, pub credit: i64, pub balance: i64 }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub fn running_balance(lines: Vec<(Date, u32, String, i64, i64)>) -> Option<Vec<LedgerEntry>>`. The tuple is `(date, number, text, debit, credit)`. It returns `None` on overflow.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ledger/tests/domain.rs`:

```rust
fn day(raw: &str) -> jiff::civil::Date {
    raw.parse().unwrap()
}

#[test]
fn the_running_balance_adds_debit_and_subtracts_credit_in_the_given_order() {
    let entries = running_balance(vec![
        (day("2026-01-05"), 1, "Försäljning".into(), 1000, 0),
        (day("2026-01-09"), 3, "Hyra".into(), 0, 1500),
        (day("2026-01-20"), 2, "Insättning".into(), 200, 0),
    ])
    .unwrap();

    assert_eq!(
        entries.iter().map(|e| e.balance).collect::<Vec<_>>(),
        [1000, -500, -300]
    );
    assert_eq!(
        entries[1],
        LedgerEntry {
            date: day("2026-01-09"),
            number: 3,
            text: "Hyra".into(),
            debit: 0,
            credit: 1500,
            balance: -500,
        }
    );
}

#[test]
fn no_lines_give_no_entries() {
    assert_eq!(running_balance(Vec::new()), Some(Vec::new()));
}

#[test]
fn an_overflowing_running_balance_is_none_not_a_panic() {
    let d = day("2026-01-01");
    assert_eq!(
        running_balance(vec![(d, 1, "a".into(), i64::MAX, 0), (d, 2, "b".into(), 1, 0)]),
        None
    );
    assert_eq!(
        running_balance(vec![(d, 1, "a".into(), 0, i64::MAX), (d, 2, "b".into(), 0, 2)]),
        None
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger --test domain running_balance`
Expected: compile error, `cannot find function running_balance`.

- [ ] **Step 3: Implement**

Append to `crates/ledger/src/domain.rs`, after the `Voucher` struct:

```rust
/// One account's totals in a fiscal year (saldobalans). Its balance is
/// `debit - credit`; positive is a debit balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBalanceRow {
    pub account: u32,
    pub name: String,
    pub debit: i64,
    pub credit: i64,
}

/// One line in an account's huvudbok, with the balance after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub date: Date,
    pub number: u32,
    pub text: String,
    pub debit: i64,
    pub credit: i64,
    pub balance: i64,
}

/// Adds the running balance to `(date, number, text, debit, credit)` lines,
/// in the order given. `None` if it outgrows `i64`, which no real ledger
/// reaches; a wrong figure would be worse than an error.
pub fn running_balance(lines: Vec<(Date, u32, String, i64, i64)>) -> Option<Vec<LedgerEntry>> {
    let mut balance = 0i64;
    lines
        .into_iter()
        .map(|(date, number, text, debit, credit)| {
            balance = balance.checked_add(debit)?.checked_sub(credit)?;
            Some(LedgerEntry {
                date,
                number,
                text,
                debit,
                credit,
                balance,
            })
        })
        .collect()
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-ledger --test domain`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger/src/domain.rs crates/ledger/tests/domain.rs
git commit -m "Compute an account's running balance without overflow"
```

---

### Task 2: Trial balance and account ledger queries

**Files:**
- Modify: `crates/ledger/src/lib.rs` (the `Error` enum and the `pub use queries::…` line)
- Modify: `crates/ledger/src/queries.rs` (imports, two new functions after `list_vouchers`)
- Modify: `crates/server/src/ledger.rs:226-236` (`status` must stay exhaustive)
- Test: `crates/ledger/tests/store.rs` (imports at the top; append tests)

**Interfaces:**
- Consumes: `TrialBalanceRow`, `LedgerEntry` and `running_balance` from Task 1.
- Produces:
  - `pub async fn trial_balance(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, fiscal_year_start: Date) -> Result<Vec<TrialBalanceRow>>`
  - `pub async fn account_ledger(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, fiscal_year_start: Date, account: u32) -> Result<Vec<LedgerEntry>>`
  - `doris_ledger::Error::Overflow`

- [ ] **Step 1: Write the failing tests**

In `crates/ledger/tests/store.rs`, change the imports at the top to:

```rust
use doris_ledger::domain::{DomainError, RecordVoucher, TrialBalanceRow, VoucherLine};
use doris_ledger::{
    Error, VoucherRef, account_ledger, add_account, correct_voucher, list_accounts,
    list_fiscal_years, list_vouchers, rebuild_projections, record_voucher, record_voucher_in,
    rename_account, set_account_active, trial_balance,
};
```

Append:

```rust
/// Another company of `owner`'s, also with first räkenskapsår 2025.
async fn second_company(pool: &SqlitePool, owner: Uuid) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556036-0793",
            name: "Bolaget AB",
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

fn booking(date: &str, text: &str, lines: &[(u32, i64, i64)]) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: text.into(),
        lines: lines
            .iter()
            .map(|&(account, debit, credit)| VoucherLine::new(account, debit, credit).unwrap())
            .collect(),
    }
}

#[tokio::test]
async fn the_trial_balance_sums_each_account_in_one_fiscal_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let other = second_company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-04-01", 250), today)
        .await
        .unwrap();
    record_voucher(
        &pool,
        id,
        anna,
        booking("2025-05-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        today,
    )
    .await
    .unwrap();
    // Reverses voucher 1: both it and the correction are counted.
    correct_voucher(&pool, id, anna, d("2025-01-01"), 1, d("2025-06-01"), today)
        .await
        .unwrap();
    // Another fiscal year, and another company: neither is counted.
    record_voucher(&pool, id, anna, sale("2026-01-10", 999), today)
        .await
        .unwrap();
    record_voucher(&pool, other, anna, sale("2025-03-01", 777), today)
        .await
        .unwrap();

    let rows = trial_balance(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![
            TrialBalanceRow {
                account: 1930,
                name: "Företagskonto/checkkonto/affärskonto".into(),
                debit: 350,
                credit: 400,
            },
            TrialBalanceRow {
                account: 3001,
                name: "Försäljning inom Sverige, 25 % moms".into(),
                debit: 100,
                credit: 350,
            },
            TrialBalanceRow {
                account: 5010,
                name: "Lokalhyra".into(),
                debit: 300,
                credit: 0,
            },
        ]
    );
    assert_eq!(rows.iter().map(|r| r.debit - r.credit).sum::<i64>(), 0);
    assert!(
        trial_balance(&pool, id, anna, d("2024-01-01"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_accounts_ledger_runs_in_date_order_with_its_balance() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    // Voucher 1 is dated after vouchers 2 and 3.
    for cmd in [
        sale("2025-04-01", 250),
        sale("2025-03-01", 100),
        booking("2025-03-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        booking("2025-05-01", "Omföring", &[(1930, 40, 0), (1930, 0, 40)]),
    ] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    let entries = account_ledger(&pool, id, anna, d("2025-01-01"), 1930)
        .await
        .unwrap();

    assert_eq!(
        entries
            .iter()
            .map(|e| (e.date, e.number, e.debit, e.credit, e.balance))
            .collect::<Vec<_>>(),
        vec![
            (d("2025-03-01"), 2, 100, 0, 100),
            (d("2025-03-01"), 3, 0, 300, -200),
            (d("2025-04-01"), 1, 250, 0, 50),
            (d("2025-05-01"), 4, 40, 0, 90),
            (d("2025-05-01"), 4, 0, 40, 50),
        ]
    );
    assert_eq!(entries[1].text, "Hyra");
    assert!(
        account_ledger(&pool, id, anna, d("2025-01-01"), 1931)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        account_ledger(&pool, id, anna, d("2025-01-01"), 999).await,
        Err(Error::Domain(DomainError::InvalidAccountNumber))
    ));
}

#[tokio::test]
async fn non_members_cannot_read_the_reports() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY))
        .await
        .unwrap();

    assert!(matches!(
        trial_balance(&pool, id, bo, d("2025-01-01")).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        account_ledger(&pool, id, bo, d("2025-01-01"), 1930).await,
        Err(Error::NotFound)
    ));
    // Membership is checked before the account number.
    assert!(matches!(
        account_ledger(&pool, id, bo, d("2025-01-01"), 999).await,
        Err(Error::NotFound)
    ));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-ledger --test store`
Expected: compile error, `unresolved imports doris_ledger::account_ledger, doris_ledger::trial_balance`.

- [ ] **Step 3: Implement**

In `crates/ledger/src/lib.rs`, add a variant to `Error` after `NotFound`:

```rust
    /// A sum outgrew `i64`; no real ledger gets there.
    #[error("amount overflow")]
    Overflow,
```

and change the re-export to:

```rust
pub use queries::{account_ledger, list_accounts, list_fiscal_years, list_vouchers, trial_balance};
```

In `crates/ledger/src/queries.rs`, change the imports to:

```rust
use crate::domain::{
    Account, AccountName, AccountNumber, Chart, LedgerEntry, TrialBalanceRow, Voucher, VoucherLine,
    running_balance,
};
use crate::{Error, Result};
```

and append before `fn attach_lines`:

```rust
/// The saldobalans for one fiscal year: every account with lines in it,
/// by number, with its debit and credit totals.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn trial_balance(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
) -> Result<Vec<TrialBalanceRow>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    // The chart is always seeded once there are vouchers; LEFT JOIN keeps an
    // account in the saldobalans even if it somehow weren't. SQLite's SUM
    // fails on integer overflow rather than wrapping.
    let rows: Vec<(u32, String, i64, i64)> = sqlx::query_as(
        "SELECT l.account, COALESCE(a.name, ''), SUM(l.debit), SUM(l.credit)
         FROM voucher_lines l
         LEFT JOIN accounts a ON a.company_id = l.company_id AND a.number = l.account
         WHERE l.company_id = ? AND l.fiscal_year_start = ?
         GROUP BY l.account
         ORDER BY l.account",
    )
    .bind(company_id.to_string())
    .bind(fiscal_year_start.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(account, name, debit, credit)| TrialBalanceRow {
            account,
            name,
            debit,
            credit,
        })
        .collect())
}

/// One account's huvudbok for one fiscal year: its lines by date, then
/// voucher number, then line, each with the balance after it.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn account_ledger(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    account: u32,
) -> Result<Vec<LedgerEntry>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let account = AccountNumber::parse(account)?;
    let lines: Vec<(String, u32, String, i64, i64)> = sqlx::query_as(
        "SELECT v.date, v.number, v.text, l.debit, l.credit
         FROM voucher_lines l
         JOIN vouchers v ON v.company_id = l.company_id
             AND v.fiscal_year_start = l.fiscal_year_start AND v.number = l.number
         WHERE l.company_id = ? AND l.fiscal_year_start = ? AND l.account = ?
         ORDER BY v.date, v.number, l.line_no",
    )
    .bind(company_id.to_string())
    .bind(fiscal_year_start.to_string())
    .bind(i64::from(account.get()))
    .fetch_all(pool)
    .await?;
    running_balance(
        lines
            .into_iter()
            .map(|(date, number, text, debit, credit)| {
                let date = date.parse().expect("projected dates are valid");
                (date, number, text, debit, credit)
            })
            .collect(),
    )
    .ok_or(Error::Overflow)
}
```

In `crates/server/src/ledger.rs`, add an arm to `fn status` before `Error::Store(err)`:

```rust
        Error::Overflow => {
            tracing::error!("ledger: amount overflow");
            Status::internal("internal")
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-ledger && cargo build -p doris-server`
Expected: all pass, and the server builds.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger crates/server/src/ledger.rs
git commit -m "Read a fiscal year's trial balance and an account's ledger"
```

---

### Task 3: GetTrialBalance and GetAccountLedger over gRPC-Web

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`
- Modify: `crates/server/src/ledger.rs` (two trait methods after `list_vouchers`)
- Test: `crates/server/tests/ledger.rs` (append)

**Interfaces:**
- Consumes: `doris_ledger::{trial_balance, account_ledger}` from Task 2.
- Produces: the proto messages `GetTrialBalanceRequest { company_id, fiscal_year_start }`, `TrialBalanceRow { account, name, debit, credit }`, `GetTrialBalanceResponse { rows }`, `GetAccountLedgerRequest { company_id, fiscal_year_start, account }`, `LedgerEntry { date, number, text, debit, credit, balance }` and `GetAccountLedgerResponse { entries }`. Rust: `lpb::GetTrialBalanceRequest` etc., and client methods `get_trial_balance` and `get_account_ledger`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/server/tests/ledger.rs`:

```rust
fn trial_balance_of(company_id: &str, fiscal_year_start: &str) -> pb::GetTrialBalanceRequest {
    pb::GetTrialBalanceRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

fn ledger_of(company_id: &str, fiscal_year_start: &str, account: u32) -> pb::GetAccountLedgerRequest {
    pb::GetAccountLedgerRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
        account,
    }
}

#[tokio::test]
async fn the_trial_balance_and_an_accounts_ledger_follow_the_vouchers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    for ore in [125_000, 5_000] {
        api.record_voucher(authed(sale(&id, ore), &anna))
            .await
            .unwrap();
    }

    let rows = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner()
        .rows;
    let entries = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &anna))
        .await
        .unwrap()
        .into_inner()
        .entries;

    assert_eq!(
        rows,
        vec![
            pb::TrialBalanceRow {
                account: 1930,
                name: "Företagskonto/checkkonto/affärskonto".into(),
                debit: 130_000,
                credit: 0,
            },
            pb::TrialBalanceRow {
                account: 3001,
                name: "Försäljning inom Sverige, 25 % moms".into(),
                debit: 0,
                credit: 130_000,
            },
        ]
    );
    assert_eq!(
        entries,
        vec![
            pb::LedgerEntry {
                date: "2026-01-15".into(),
                number: 1,
                text: "Försäljning".into(),
                debit: 125_000,
                credit: 0,
                balance: 125_000,
            },
            pb::LedgerEntry {
                date: "2026-01-15".into(),
                number: 2,
                text: "Försäljning".into(),
                debit: 5_000,
                credit: 0,
                balance: 130_000,
            },
        ]
    );
}

#[tokio::test]
async fn the_reports_refuse_bad_input_and_non_members() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let invalid_date = (Code::InvalidArgument, "invalid_date".to_owned());
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    let err = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-13-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), invalid_date);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "nonsense", 1930), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), invalid_date);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 99), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_account_number".to_owned())
    );

    let err = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), not_found);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), not_found);
    let err = api
        .get_trial_balance(trial_balance_of(&id, "2026-01-01"))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".to_owned())
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-server --test ledger reports`
Expected: compile error, `cannot find struct GetTrialBalanceRequest in module pb`.

- [ ] **Step 3: Add the contract**

In `proto/doris/ledger/v1/ledger.proto`, add to `service LedgerService` after `ListVouchers`:

```proto
  // The saldobalans for one fiscal year: debit and credit per account with
  // lines in it, by account number.
  rpc GetTrialBalance(GetTrialBalanceRequest) returns (GetTrialBalanceResponse);
  // One account's huvudbok for one fiscal year: its lines by date, then
  // voucher number, with the balance after each.
  rpc GetAccountLedger(GetAccountLedgerRequest) returns (GetAccountLedgerResponse);
```

Append the messages to the end of the file:

```proto
message GetTrialBalanceRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}

// The balance is debit - credit; positive is a debit balance.
message TrialBalanceRow {
  uint32 account = 1;
  string name = 2;
  int64 debit = 3;
  int64 credit = 4;
}

message GetTrialBalanceResponse {
  repeated TrialBalanceRow rows = 1;
}

message GetAccountLedgerRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 account = 3;
}

// balance is the account's balance after this line.
message LedgerEntry {
  string date = 1;
  uint32 number = 2;
  string text = 3;
  int64 debit = 4;
  int64 credit = 5;
  int64 balance = 6;
}

message GetAccountLedgerResponse {
  repeated LedgerEntry entries = 1;
}
```

- [ ] **Step 4: Serve it**

In `crates/server/src/ledger.rs`, add to `impl LedgerService for LedgerApi` after `list_vouchers`:

```rust
    async fn get_trial_balance(
        &self,
        request: Request<pb::GetTrialBalanceRequest>,
    ) -> Result<Response<pb::GetTrialBalanceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let rows = doris_ledger::trial_balance(&self.pool, company, user, fiscal_year_start)
            .await
            .map_err(status)?
            .into_iter()
            .map(|r| pb::TrialBalanceRow {
                account: r.account,
                name: r.name,
                debit: r.debit,
                credit: r.credit,
            })
            .collect();
        Ok(Response::new(pb::GetTrialBalanceResponse { rows }))
    }

    async fn get_account_ledger(
        &self,
        request: Request<pb::GetAccountLedgerRequest>,
    ) -> Result<Response<pb::GetAccountLedgerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let fiscal_year_start = date(&req.fiscal_year_start)?;
        let entries =
            doris_ledger::account_ledger(&self.pool, company, user, fiscal_year_start, req.account)
                .await
                .map_err(status)?
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
        Ok(Response::new(pb::GetAccountLedgerResponse { entries }))
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-server --test ledger && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass, with no warnings.

- [ ] **Step 6: Commit**

```bash
git add proto/doris/ledger/v1/ledger.proto crates/server
git commit -m "Serve the trial balance and an account's ledger over gRPC-Web"
```

---

### Task 4: Saldobalans page

**Files:**
- Modify: `crates/web/src/fiscal_year.rs` (module doc, new functions and a component, plus tests in the existing `mod tests`)
- Modify: `crates/web/src/ui.rs` (add `TABLE_AMOUNT_CELL` next to `TABLE_CELL`)
- Create: `crates/web/src/pages/trial_balance.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`

**Interfaces:**
- Consumes: `lpb::GetTrialBalanceRequest` and `lpb::TrialBalanceRow` from Task 3.
- Produces (used by Task 5):
  - `fiscal_year::pick_year(years: &[lpb::FiscalYear], preferred: &str) -> String`
  - `fiscal_year::opening_balances_missing(years: &[lpb::FiscalYear], start: &str) -> bool`
  - `fiscal_year::use_fiscal_years(preferred: String, error: RwSignal<Option<String>>) -> (RwSignal<Vec<lpb::FiscalYear>>, RwSignal<String>)`
  - `#[component] fiscal_year::FiscalYearSelect(years: RwSignal<Vec<lpb::FiscalYear>>, year: RwSignal<String>)`
  - `ui::TABLE_AMOUNT_CELL: &str`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` in `crates/web/src/fiscal_year.rs`:

```rust
    use crate::api::lpb;

    fn years(starts: &[&str]) -> Vec<lpb::FiscalYear> {
        starts
            .iter()
            .map(|s| lpb::FiscalYear {
                start: (*s).into(),
                end: String::new(),
            })
            .collect()
    }

    #[test]
    fn pick_year_keeps_a_listed_preference() {
        let ys = years(&["2027-01-01", "2026-01-01"]);
        assert_eq!(pick_year(&ys, "2026-01-01"), "2026-01-01");
    }

    #[test]
    fn pick_year_falls_back_to_the_newest() {
        let ys = years(&["2027-01-01", "2026-01-01"]);
        assert_eq!(pick_year(&ys, ""), "2027-01-01");
        assert_eq!(pick_year(&ys, "nonsense"), "2027-01-01");
        assert_eq!(pick_year(&[], "2026-01-01"), "");
    }

    #[test]
    fn opening_balances_are_missing_after_the_first_year() {
        let ys = years(&["2027-01-01", "2026-01-01"]);
        assert!(opening_balances_missing(&ys, "2027-01-01"));
        assert!(!opening_balances_missing(&ys, "2026-01-01"));
        assert!(!opening_balances_missing(&ys, ""));
        assert!(!opening_balances_missing(&[], "2026-01-01"));
    }
```

Create `crates/web/src/pages/trial_balance.rs` with only the pure part and its tests first:

```rust
//! Saldobalans: every account with lines in one fiscal year, split into
//! balansräkning and resultaträkning.

use crate::api::lpb;

/// One part of the saldobalans and its totals.
#[derive(Debug, Default, PartialEq)]
pub struct Part {
    pub rows: Vec<lpb::TrialBalanceRow>,
    pub debit: i64,
    pub credit: i64,
}

impl Part {
    pub fn balance(&self) -> i64 {
        self.debit - self.credit
    }
}

/// Balansräkning (accounts 1000–2999) and resultaträkning (3000–8999).
pub fn split(rows: Vec<lpb::TrialBalanceRow>) -> (Part, Part) {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(account: u32, debit: i64, credit: i64) -> lpb::TrialBalanceRow {
        lpb::TrialBalanceRow {
            account,
            name: String::new(),
            debit,
            credit,
        }
    }

    #[test]
    fn accounts_below_3000_belong_to_the_balance_sheet() {
        let (balance, income) = split(vec![
            row(1930, 1000, 300),
            row(2999, 0, 50),
            row(3000, 0, 900),
            row(5010, 250, 0),
        ]);

        assert_eq!(
            balance.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [1930, 2999]
        );
        assert_eq!((balance.debit, balance.credit, balance.balance()), (1000, 350, 650));
        assert_eq!(
            income.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [3000, 5010]
        );
        assert_eq!((income.debit, income.credit, income.balance()), (250, 900, -650));
    }

    #[test]
    fn no_rows_give_two_empty_parts() {
        assert_eq!(split(Vec::new()), (Part::default(), Part::default()));
    }
}
```

and register it in `crates/web/src/pages/mod.rs` (`mod trial_balance;` with the others).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p doris-web`
Expected: compile errors for `pick_year` and `opening_balances_missing`. If those are stubbed, the `split` tests panic at `todo!()`.

- [ ] **Step 3: Implement the fiscal-year helpers**

In `crates/web/src/fiscal_year.rs`, extend the module doc with a second paragraph:

```rust
//!
//! It also picks the räkenskapsår a report page shows, and loads the active
//! company's years for the pages that need them.
```

Change the imports to:

```rust
use crate::active_company::Companies;
use crate::api::{cpb, ledger_api, lpb};
use crate::errors::describe;
use crate::ui::{SELECT_OPTION, Select};
use leptos::prelude::*;
use leptos::task::spawn_local;
```

Add after `default_end` (before `fn days_in_month`):

```rust
/// `preferred` if `years` lists it, otherwise the newest (first) year, or
/// "" when there are none.
pub fn pick_year(years: &[lpb::FiscalYear], preferred: &str) -> String {
    years
        .iter()
        .find(|y| y.start == preferred)
        .or(years.first())
        .map(|y| y.start.clone())
        .unwrap_or_default()
}

/// Whether `start` is a later year than the company's first (the oldest,
/// listed last). Opening balances don't exist yet, so from the second year
/// on the balance-sheet accounts show only that year's movements.
pub fn opening_balances_missing(years: &[lpb::FiscalYear], start: &str) -> bool {
    !start.is_empty() && years.last().is_some_and(|first| first.start != start)
}

/// The active company's räkenskapsår (newest first) and the chosen one's
/// start, which is `preferred` if listed, else the newest. Both are cleared
/// and reloaded whenever the active company changes.
pub fn use_fiscal_years(
    preferred: String,
    error: RwSignal<Option<String>>,
) -> (RwSignal<Vec<lpb::FiscalYear>>, RwSignal<String>) {
    let companies = expect_context::<Companies>();
    let years = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    let year = RwSignal::new(String::new());
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's years on screen.
        years.set(Vec::new());
        year.set(String::new());
        error.set(None);
        if company_id.is_empty() {
            return;
        }
        let preferred = preferred.clone();
        spawn_local(async move {
            let result = ledger_api()
                .list_fiscal_years(lpb::ListFiscalYearsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    let chosen = pick_year(&list, &preferred);
                    years.set(list);
                    year.set(chosen);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    (years, year)
}

/// The "Räkenskapsår" select over `years`, bound to `year`.
#[component]
pub fn FiscalYearSelect(
    years: RwSignal<Vec<lpb::FiscalYear>>,
    year: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class="w-56">
            <Select label="Räkenskapsår" id="fiscal_year" value=year>
                {move || {
                    years
                        .get()
                        .into_iter()
                        .map(|y| view! { <option class=SELECT_OPTION value=y.start.clone()>{format!("{} – {}", y.start, y.end)}</option> })
                        .collect_view()
                }}
            </Select>
        </div>
    }
}
```

In `crates/web/src/ui.rs`, add after `TABLE_CELL`:

```rust
/// `TABLE_CELL` for amounts: right-aligned with tabular digits.
pub const TABLE_AMOUNT_CELL: &str =
    "p-2 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0 text-right tabular-nums";
```

- [ ] **Step 4: Implement `split` and the page**

Replace the `todo!()` body of `split` with:

```rust
// ponytail: i64 totals; a year's lines would need ~92 biljarder kronor to overflow.
pub fn split(rows: Vec<lpb::TrialBalanceRow>) -> (Part, Part) {
    let (mut balance, mut income) = (Part::default(), Part::default());
    for row in rows {
        let part = if row.account < 3000 { &mut balance } else { &mut income };
        part.debit += row.debit;
        part.credit += row.credit;
        part.rows.push(row);
    }
    (balance, income)
}
```

Change the imports at the top of `trial_balance.rs` to:

```rust
use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, opening_balances_missing, use_fiscal_years};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;
```

Add the page and the part table after `split`:

```rust
#[component]
pub fn TrialBalance() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    // None until the chosen year's rows have arrived.
    let rows = RwSignal::new(None::<Vec<lpb::TrialBalanceRow>>);

    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        rows.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_trial_balance(lpb::GetTrialBalanceRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => rows.set(Some(response.into_inner().rows)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let missing = move || years.with(|ys| opening_balances_missing(ys, &year.get()));

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Saldobalans"</h1>
            <ErrorAlert message=error />
            <FiscalYearSelect years=years year=year />
            <Show when=missing>
                <p class="text-xs/relaxed text-muted-foreground">
                    "Ingående balanser saknas än, så saldon för balansräkningens konton visar bara årets rörelser."
                </p>
            </Show>
            {move || {
                rows.get().map(|rows| {
                    if rows.is_empty() {
                        return view! {
                            <p class="text-xs/relaxed text-muted-foreground">"Inga verifikationer under räkenskapsåret."</p>
                        }
                        .into_any();
                    }
                    let start = year.get_untracked();
                    let (balance, income) = split(rows);
                    let total = balance.balance() + income.balance();
                    let result = -income.balance();
                    view! {
                        <PartTable title="Balansräkning" part=balance year=start.clone() />
                        <PartTable title="Resultaträkning" part=income year=start />
                        <div class="grid gap-1 text-xs/relaxed tabular-nums">
                            <p>"Beräknat resultat " {amount(result)}</p>
                            <p class=if total == 0 { "" } else { "text-destructive" }>"Summa saldo " {amount(total)}</p>
                        </div>
                    }
                    .into_any()
                })
            }}
        </div>
    }
}

/// One part of the saldobalans. Each account links to its huvudbok for `year`.
#[component]
fn PartTable(title: &'static str, part: Part, year: String) -> impl IntoView {
    let (debit, credit, balance) = (part.debit, part.credit, part.balance());
    view! {
        <section class="grid gap-2">
            <h2 class="text-sm font-medium">{title}</h2>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Saldo"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {part
                        .rows
                        .into_iter()
                        .map(|row| {
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>
                                        <A href=format!("/trial-balance/{}?fy={year}", row.account) attr:class="underline-offset-4 hover:underline">
                                            {row.account}
                                        </A>
                                    </td>
                                    <td class=TABLE_CELL>{row.name.clone()}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit - row.credit)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                    <tr class=TABLE_ROW>
                        <td class=format!("{TABLE_CELL} font-medium") colspan="2">"Summa"</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(balance)}</td>
                    </tr>
                </tbody>
            </Table>
        </section>
    }
}
```

In `crates/web/src/pages/mod.rs`, add `pub use trial_balance::TrialBalance;`.

In `crates/web/src/app.rs`:
- add `TrialBalance` to the `use crate::pages::{…}` list
- add the route after `/vouchers/new`:

```rust
                        <Route path=path!("/trial-balance") view=|| view! { <SignedIn><TrialBalance /></SignedIn> } />
```

- add the header link after "Verifikationer":

```rust
                        <A href="/trial-balance" attr:class="text-muted-foreground hover:text-foreground">"Saldobalans"</A>
```

- [ ] **Step 5: Run the tests and lints**

Run:
```
cargo test -p doris-web
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
```
Expected: all pass, with no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/web
git commit -m "Show the trial balance for a fiscal year in the web app"
```

---

### Task 5: Huvudbok page for one account

**Files:**
- Create: `crates/web/src/pages/account_ledger.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`

**Interfaces:**
- Consumes: `use_fiscal_years` and `FiscalYearSelect` (Task 4), `TABLE_AMOUNT_CELL` (Task 4), `lpb::GetAccountLedgerRequest` and `lpb::LedgerEntry` (Task 3).
- Produces: `#[component] pub fn AccountLedger()`, routed at `/trial-balance/:account`.

This task has no pure logic of its own. Its behavior is pinned by the e2e tests in Task 6, which are written first in that task. Here the deliverable is a page that compiles, lints clean and is reachable.

- [ ] **Step 1: Create the page**

`crates/web/src/pages/account_ledger.rs`:

```rust
//! Huvudbok for one account: its lines in one fiscal year, by date, with
//! the running balance.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, use_fiscal_years};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_params_map, use_query_map};

#[component]
pub fn AccountLedger() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // Junk in the URL becomes 0, which the server refuses as
    // invalid_account_number, shown in Swedish.
    let account = use_params_map()
        .read_untracked()
        .get("account")
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(0);
    let error = RwSignal::new(None::<String>);
    let preferred = use_query_map().read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    let name = RwSignal::new(String::new());
    // None until the chosen year's entries have arrived.
    let entries = RwSignal::new(None::<Vec<lpb::LedgerEntry>>);

    Effect::new(move |_| {
        let company_id = companies.active.get();
        name.set(String::new());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = result {
                let found = response.into_inner().accounts.into_iter().find(|a| a.number == account);
                name.set(found.map(|a| a.name).unwrap_or_default());
            }
        });
    });
    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        entries.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_account_ledger(lpb::GetAccountLedgerRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                    account,
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => entries.set(Some(response.into_inner().entries)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">{move || format!("{account} {}", name.get()).trim_end().to_owned()}</h1>
                <A href=move || format!("/trial-balance?fy={}", year.get()) attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">
                    "Tillbaka till saldobalansen"
                </A>
            </div>
            <ErrorAlert message=error />
            <FiscalYearSelect years=years year=year />
            {move || {
                entries.get().map(|entries| {
                    if entries.is_empty() {
                        return view! {
                            <p class="text-xs/relaxed text-muted-foreground">"Inga transaktioner på kontot under räkenskapsåret."</p>
                        }
                        .into_any();
                    }
                    let debit: i64 = entries.iter().map(|e| e.debit).sum();
                    let credit: i64 = entries.iter().map(|e| e.credit).sum();
                    let balance = entries.last().map(|e| e.balance).unwrap_or_default();
                    view! {
                        <Table>
                            <thead class=TABLE_HEAD>
                                <tr class=TABLE_ROW>
                                    <th class=TABLE_HEADER_CELL>"Datum"</th>
                                    <th class=TABLE_HEADER_CELL>"Ver"</th>
                                    <th class=TABLE_HEADER_CELL>"Text"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Saldo"</th>
                                </tr>
                            </thead>
                            <tbody class=TABLE_BODY>
                                {entries
                                    .into_iter()
                                    .map(|e| {
                                        view! {
                                            <tr class=TABLE_ROW>
                                                <td class=TABLE_CELL>{e.date}</td>
                                                <td class=TABLE_CELL>{e.number}</td>
                                                <td class=TABLE_CELL>{e.text}</td>
                                                <td class=TABLE_AMOUNT_CELL>{(e.debit != 0).then(|| amount(e.debit))}</td>
                                                <td class=TABLE_AMOUNT_CELL>{(e.credit != 0).then(|| amount(e.credit))}</td>
                                                <td class=TABLE_AMOUNT_CELL>{amount(e.balance)}</td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                                <tr class=TABLE_ROW>
                                    <td class=format!("{TABLE_CELL} font-medium") colspan="3">"Summa"</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(balance)}</td>
                                </tr>
                            </tbody>
                        </Table>
                    }
                    .into_any()
                })
            }}
        </div>
    }
}
```

In `crates/web/src/pages/mod.rs`, add `mod account_ledger;` and `pub use account_ledger::AccountLedger;`.

In `crates/web/src/app.rs`, add `AccountLedger` to the `use crate::pages::{…}` list and add the route after `/trial-balance`:

```rust
                        <Route path=path!("/trial-balance/:account") view=|| view! { <SignedIn><AccountLedger /></SignedIn> } />
```

- [ ] **Step 2: Build and lint**

Run:
```
cargo test -p doris-web
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
```
Expected: all pass, with no warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/web
git commit -m "Show one account's ledger with its running balance"
```

---

### Task 6: End to end, wasm budget and docs

**Files:**
- Modify: `e2e/tests/ledger.spec.ts` (append; `bookSale` is already defined in the file)
- Modify: `AGENTS.md` (the `## Event sourcing rules` list, after the line about rättelser)

**Interfaces:**
- Consumes: the pages from Tasks 4 and 5, and the fixtures `register` and `addCompany`. `addCompany` creates a company whose first räkenskapsår is 2026.

- [ ] **Step 1: Write the e2e tests**

Append to `e2e/tests/ledger.spec.ts`:

```ts
test("the trial balance and an account's ledger show what was booked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers`);
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await page.getByRole("banner").getByRole("link", { name: "Saldobalans" }).click();
  await expect(page.getByRole("heading", { name: "Saldobalans" })).toBeVisible();
  await expect(page.getByRole("row", { name: /^1930 Företagskonto/ })).toContainText("1 250,00");
  await expect(page.getByRole("row", { name: /^3001 / })).toContainText("-1 250,00");
  await expect(page.getByText("Beräknat resultat 1 250,00")).toBeVisible();
  await expect(page.getByText("Summa saldo 0,00")).toBeVisible();
  // The company's first year has no opening balances to miss.
  await expect(page.getByText(/Ingående balanser saknas/)).toHaveCount(0);

  await page.getByRole("link", { name: "1930", exact: true }).click();
  await expect(page.getByRole("heading", { name: "1930 Företagskonto/checkkonto/affärskonto" })).toBeVisible();
  await expect(page.getByRole("row", { name: /Försäljning kassa/ })).toContainText("1 250,00");

  await page.getByRole("link", { name: "Tillbaka till saldobalansen" }).click();
  await expect(page.getByRole("heading", { name: "Saldobalans" })).toBeVisible();
});

test("the trial balance follows the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers`);
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await page.getByRole("banner").getByRole("link", { name: "Saldobalans" }).click();
  await expect(page.getByRole("row", { name: /^1930 / })).toBeVisible();

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByText("Inga verifikationer under räkenskapsåret.")).toBeVisible();
  await expect(page.getByRole("row", { name: /^1930 / })).toHaveCount(0);
});

test("a junk account in the URL shows a Swedish error", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await page.goto(`${app}/trial-balance/abc?fy=nonsense`);

  await expect(page.getByRole("alert")).toHaveText("Kontonumret ska vara fyra siffror, 1000–8999.");
});
```

- [ ] **Step 2: Run them**

Run: `make e2e`
Expected: every test passes, including the earlier ones. If a new test fails, fix the page and not the test, unless the test contradicts the spec.

To prove the stale-answer guard is load-bearing, temporarily delete the line `if company_id != companies.active.get_untracked() || start != year.get_untracked() { return; }` in `trial_balance.rs`. Run `npx playwright test -g "follows the active company" --repeat-each 10` from `e2e/`. Note whether it fails, then restore the line. If it never fails, leave the guard in anyway: it matches `/vouchers`, and the e2e still pins the visible behavior.

- [ ] **Step 3: Check the wasm budget**

Run: `make dist`
Expected: it succeeds and prints the size. Record the size in the commit message. If it is over 900 000 bytes, stop and report.

- [ ] **Step 4: Document**

In `AGENTS.md`, after the bullet that starts "A voucher is never changed or removed.", add:

```markdown
- The saldobalans and huvudbok (`trial_balance`, `account_ledger` in
  `crates/ledger/src/queries.rs`) are plain queries over the voucher
  projections, one fiscal year at a time. There are no opening balances
  yet, so from the second year on balance-sheet accounts show only that
  year's movements.
```

- [ ] **Step 5: Run everything and commit**

Run:
```
make test
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
```
Expected: all pass.

```bash
git add e2e/tests/ledger.spec.ts AGENTS.md
git commit -m "Cover the trial balance and account ledger end to end"
```
