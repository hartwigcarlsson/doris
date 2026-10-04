# Plan 11: Income Statement and Balance Sheet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Members can see one fiscal year's resultaträkning and balansräkning under ÅRL headings (K2's abbreviated forms), with the previous fiscal year as a comparison column.

**Architecture:**
- The step is read-only. It adds no events, no commands, no migration and no projection.
- A new pure module, `crates/ledger/src/statements.rs`, is the only place that maps a BAS account to an ÅRL post and lays out the two statements. `build` turns `TrialBalanceRow`s into finished lines.
- A new query, `financial_statements`, runs the existing `trial_balance` for the chosen year and the year before, then calls `build`.
- `LedgerService` gets one RPC, `GetFinancialStatements`. The web app gets one page, `/financial-statements`, which only draws the lines the server sends.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, tonic 0.14 gRPC-Web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-resultat-balansrakning-design.md`

## Global Constraints
- **TDD is mandatory.** Every behaviour starts as a failing test. Each task ends with a commit.
- **Amounts** are `i64` öre everywhere, including proto `int64`. Every sum uses checked arithmetic. An overflow is `doris_ledger::Error::Overflow`, which maps to `internal`. Code never panics and never wraps.
- **Sign convention:**
  - Resultaträkning: every post is −(debit − credit), so income is positive and costs are negative.
  - Balansräkning: assets are UB (`opening + debit − credit`), and equity, untaxed reserves, provisions and liabilities are −UB.
- **Årets resultat:**
  - The resultaträkning leaves out accounts 8990–8999.
  - The balansräkning's "Årets resultat" post holds the result account plus every account from 3000 to 8999, 8999 included. The result account is 2099, or 2019 for enskild firma, HB and KB, as given by `domain::result_account`.
- **Hidden posts:** an `Item` whose amount is 0 this year and 0 (or absent) the year before is left out. Headings and subtotals are always kept.
- **Previous year:** the fiscal year that ends the day before the chosen one starts. The first fiscal year has none.
- **Fiscal year lookup** uses the existing `fiscal_year_at`, so a date that does not start a fiscal year, a date before the first year and a year that starts after today all give `fiscal_year_not_found`. Membership is checked before that, and a non-member gets `company_not_found`.
- **Error codes:** none are new. Only `invalid_date`, `fiscal_year_not_found`, `company_not_found`, `not_signed_in` and `internal` are used, so `errors.rs` is unchanged.
- **No new dependencies.** The wasm budget (500 KB gzipped, checked by `make dist`) still applies.
- **Language:** code, identifiers, URLs and commits are in English. User-visible text is Swedish and must match exactly:
  - "Rapporter", "Resultat- och balansräkning", "Resultaträkning", "Balansräkning", "Post"
  - "Räkenskapsåret är stängt"
  - "Balansräkningen balanserar inte (differens {belopp}). Ett tidigare räkenskapsår är inte stängt, så dess resultat finns inte i eget kapital."
  - The post, heading and subtotal labels listed in Task 1 and Task 2.

## Review Focus
- **Previous year not closed.** Year 2 is viewed while year 1 is still open. The balansräkning must show a non-zero `difference` and the Swedish warning, not a silently balanced sheet. Task 2 (`difference`) and Task 3 (store test) pin this.
- **A closed year looks exactly like an open year.** Closing books 8999 against 2099 or 2019. Every resultaträkning and balansräkning line must stay the same, for AB and for enskild firma. Task 2 pins this.
- **A post that is 0 now but had an amount last year** must still be shown, or the comparison column loses information. Task 2 pins this.
- **The URL holds a year that is not a fiscal year start** (`?fy=2026-02-01`, a far future year or junk). The page must show the Swedish error and never panic. Task 3 and Task 4 pin `fiscal_year_not_found` and `invalid_date`. The page reuses `describe`.
- **The active company changes while a request is in flight.** A stale answer must not be drawn for the new company. Task 5 copies `/trial-balance`'s staleness check, and the e2e test switches company.

---

### Task 1: Map every BAS account to an ÅRL post

**Files:**
- Create: `crates/ledger/src/statements.rs`
- Modify: `crates/ledger/src/lib.rs:8-11` (module list)
- Modify: `crates/ledger/src/domain.rs:786` (`fn result_account` becomes `pub(crate) fn result_account`)
- Test: `crates/ledger/tests/statements.rs` (new)

**Interfaces:**
- Consumes: `domain::result_account(LegalForm) -> AccountNumber` and `AccountNumber::get(self) -> u16`, both existing.
- Produces:
  - `pub enum Post`, which is `Copy`, `Eq` and `Ord`, with the variants below.
  - `Post::label(self) -> &'static str`.
  - `pub fn income_post(account: u32) -> Option<Post>`.
  - `pub fn balance_post(account: u32, legal_form: LegalForm) -> Option<Post>`.
  - The private `Group` enum, with `Post::group(self) -> Group` and `Post::is_asset(self) -> bool`.

The ranges below are this plan's reading of BAS's SRU codes for INK2R. They are authoritative for this plan. If you find a range that disagrees with the official BAS SRU column, report it in your summary rather than changing it silently.

- [ ] **Step 1: Write the failing test**

Create `crates/ledger/tests/statements.rs`:

```rust
use doris_company::domain::LegalForm;
use doris_ledger::statements::{Post, balance_post, income_post};

const FORMS: [LegalForm; 8] = [
    LegalForm::Aktiebolag,
    LegalForm::Handelsbolag,
    LegalForm::Kommanditbolag,
    LegalForm::EnskildFirma,
    LegalForm::EkonomiskForening,
    LegalForm::IdeellForening,
    LegalForm::Stiftelse,
    LegalForm::Other,
];

#[test]
fn every_account_lands_in_exactly_one_post_of_each_statement() {
    for form in FORMS {
        for account in 1000..=8999 {
            let (income, balance) = (income_post(account), balance_post(account, form));
            match account {
                1000..=2999 => assert!(income.is_none() && balance.is_some(), "{account}"),
                3000..=8989 => assert!(
                    income.is_some() && balance == Some(Post::ResultForYear),
                    "{account}"
                ),
                // The result voucher's 8999 only moves the result to equity.
                _ => assert!(
                    income.is_none() && balance == Some(Post::ResultForYear),
                    "{account}"
                ),
            }
        }
    }
    assert_eq!(income_post(999), None);
    assert_eq!(balance_post(999, LegalForm::Aktiebolag), None);
}

#[test]
fn the_narrower_ranges_win_over_their_groups() {
    let ab = LegalForm::Aktiebolag;
    assert_eq!(income_post(4010), Some(Post::RawMaterials));
    assert_eq!(income_post(4950), Some(Post::InventoryChange));
    assert_eq!(income_post(4960), Some(Post::Goods));
    assert_eq!(income_post(7832), Some(Post::Depreciation));
    assert_eq!(income_post(7745), Some(Post::CurrentAssetWritedowns));
    assert_eq!(income_post(7790), Some(Post::CurrentAssetWritedowns));
    assert_eq!(income_post(8410), Some(Post::InterestExpenses));
    assert_eq!(income_post(8910), Some(Post::IncomeTax));
    assert_eq!(income_post(8980), Some(Post::OtherTaxes));
    assert_eq!(balance_post(1125, ab), Some(Post::LeaseholdImprovements));
    assert_eq!(balance_post(1185, ab), Some(Post::ConstructionInProgress));
    assert_eq!(balance_post(1285, ab), Some(Post::ConstructionInProgress));
    assert_eq!(balance_post(1110, ab), Some(Post::Buildings));
    assert_eq!(balance_post(1220, ab), Some(Post::Machinery));
    assert_eq!(balance_post(1930, ab), Some(Post::Cash));
    assert_eq!(balance_post(2440, ab), Some(Post::Payables));
    assert_eq!(balance_post(2510, ab), Some(Post::TaxLiabilities));
    assert_eq!(balance_post(2610, ab), Some(Post::OtherCurrentLiabilities));
    assert_eq!(balance_post(2990, ab), Some(Post::Accrued));
}

#[test]
fn equity_follows_the_legal_form() {
    let (ab, ef) = (LegalForm::Aktiebolag, LegalForm::EnskildFirma);
    assert_eq!(balance_post(2081, ab), Some(Post::RestrictedEquity));
    assert_eq!(balance_post(2091, ab), Some(Post::RetainedEarnings));
    assert_eq!(balance_post(2019, ab), Some(Post::RetainedEarnings));
    assert_eq!(balance_post(2099, ab), Some(Post::ResultForYear));
    assert_eq!(balance_post(2010, ef), Some(Post::OwnersEquity));
    assert_eq!(balance_post(2099, ef), Some(Post::OwnersEquity));
    assert_eq!(balance_post(2019, ef), Some(Post::ResultForYear));
    assert_eq!(balance_post(2019, LegalForm::Handelsbolag), Some(Post::ResultForYear));
    assert_eq!(balance_post(2081, LegalForm::Kommanditbolag), Some(Post::OwnersEquity));
}

#[test]
fn posts_carry_their_arl_labels() {
    assert_eq!(Post::NetSales.label(), "Nettoomsättning");
    assert_eq!(Post::Cash.label(), "Kassa och bank");
    assert_eq!(Post::ResultForYear.label(), "Årets resultat");
    assert_eq!(Post::OwnersEquity.label(), "Eget kapital");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p doris-ledger --test statements`
Expected: FAIL to compile with "could not find `statements` in `doris_ledger`".

- [ ] **Step 3: Write the minimal implementation**

In `crates/ledger/src/domain.rs`, change line 786 from `fn result_account(` to `pub(crate) fn result_account(`.

In `crates/ledger/src/lib.rs`, add the module after `mod queries;`:

```rust
mod queries;
pub mod statements;
```

Create `crates/ledger/src/statements.rs`:

```rust
//! Resultat- och balansräkning: the saldobalans set out under the headings
//! of ÅRL's abbreviated forms (bilaga 1 and 2) as K2 uses them. This is the
//! only place that maps a BAS account to a post; the ranges follow BAS's
//! SRU codes for INK2R.

use crate::domain::result_account;
use doris_company::domain::LegalForm;

/// One post of the resultaträkning or the balansräkning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Post {
    // Resultaträkning
    NetSales,
    InventoryChange,
    CapitalizedWork,
    OtherOperatingIncome,
    RawMaterials,
    Goods,
    OtherExternalExpenses,
    Personnel,
    Depreciation,
    CurrentAssetWritedowns,
    OtherOperatingExpenses,
    GroupShares,
    AssociateShares,
    OtherSecurities,
    InterestIncome,
    InterestExpenses,
    Appropriations,
    IncomeTax,
    OtherTaxes,
    // Balansräkning
    Intangible,
    Buildings,
    LeaseholdImprovements,
    Machinery,
    ConstructionInProgress,
    FinancialFixed,
    Inventory,
    Receivables,
    OtherReceivables,
    Prepaid,
    ShortTermInvestments,
    Cash,
    RestrictedEquity,
    RetainedEarnings,
    OwnersEquity,
    ResultForYear,
    UntaxedReserves,
    Provisions,
    LongTermLiabilities,
    Payables,
    TaxLiabilities,
    OtherCurrentLiabilities,
    Accrued,
}

/// The posts a subtotal adds up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    OperatingIncome,
    OperatingExpenses,
    Financial,
    Appropriations,
    Taxes,
    FixedAssets,
    CurrentAssets,
    Equity,
    UntaxedReserves,
    Provisions,
    LongTerm,
    ShortTerm,
}

impl Post {
    pub fn label(self) -> &'static str {
        match self {
            Post::NetSales => "Nettoomsättning",
            Post::InventoryChange => {
                "Förändring av lager av produkter i arbete, färdiga varor och pågående arbete för annans räkning"
            }
            Post::CapitalizedWork => "Aktiverat arbete för egen räkning",
            Post::OtherOperatingIncome => "Övriga rörelseintäkter",
            Post::RawMaterials => "Råvaror och förnödenheter",
            Post::Goods => "Handelsvaror",
            Post::OtherExternalExpenses => "Övriga externa kostnader",
            Post::Personnel => "Personalkostnader",
            Post::Depreciation => {
                "Av- och nedskrivningar av materiella och immateriella anläggningstillgångar"
            }
            Post::CurrentAssetWritedowns => {
                "Nedskrivningar av omsättningstillgångar utöver normala nedskrivningar"
            }
            Post::OtherOperatingExpenses => "Övriga rörelsekostnader",
            Post::GroupShares => "Resultat från andelar i koncernföretag",
            Post::AssociateShares => {
                "Resultat från andelar i intresseföretag och gemensamt styrda företag"
            }
            Post::OtherSecurities => {
                "Resultat från övriga värdepapper och fordringar som är anläggningstillgångar"
            }
            Post::InterestIncome => "Övriga ränteintäkter och liknande resultatposter",
            Post::InterestExpenses => "Räntekostnader och liknande resultatposter",
            Post::Appropriations => "Bokslutsdispositioner",
            Post::IncomeTax => "Skatt på årets resultat",
            Post::OtherTaxes => "Övriga skatter",
            Post::Intangible => "Immateriella anläggningstillgångar",
            Post::Buildings => "Byggnader och mark",
            Post::LeaseholdImprovements => "Förbättringsutgifter på annans fastighet",
            Post::Machinery => "Maskiner, inventarier och övriga materiella anläggningstillgångar",
            Post::ConstructionInProgress => {
                "Pågående nyanläggningar och förskott avseende materiella anläggningstillgångar"
            }
            Post::FinancialFixed => "Finansiella anläggningstillgångar",
            Post::Inventory => "Varulager m.m.",
            Post::Receivables => "Kundfordringar",
            Post::OtherReceivables => "Övriga fordringar",
            Post::Prepaid => "Förutbetalda kostnader och upplupna intäkter",
            Post::ShortTermInvestments => "Kortfristiga placeringar",
            Post::Cash => "Kassa och bank",
            Post::RestrictedEquity => "Bundet eget kapital",
            Post::RetainedEarnings => "Balanserat resultat",
            Post::OwnersEquity => "Eget kapital",
            Post::ResultForYear => "Årets resultat",
            Post::UntaxedReserves => "Obeskattade reserver",
            Post::Provisions => "Avsättningar",
            Post::LongTermLiabilities => "Långfristiga skulder",
            Post::Payables => "Leverantörsskulder",
            Post::TaxLiabilities => "Skatteskulder",
            Post::OtherCurrentLiabilities => "Övriga kortfristiga skulder",
            Post::Accrued => "Upplupna kostnader och förutbetalda intäkter",
        }
    }

    fn group(self) -> Group {
        match self {
            Post::NetSales
            | Post::InventoryChange
            | Post::CapitalizedWork
            | Post::OtherOperatingIncome => Group::OperatingIncome,
            Post::RawMaterials
            | Post::Goods
            | Post::OtherExternalExpenses
            | Post::Personnel
            | Post::Depreciation
            | Post::CurrentAssetWritedowns
            | Post::OtherOperatingExpenses => Group::OperatingExpenses,
            Post::GroupShares
            | Post::AssociateShares
            | Post::OtherSecurities
            | Post::InterestIncome
            | Post::InterestExpenses => Group::Financial,
            Post::Appropriations => Group::Appropriations,
            Post::IncomeTax | Post::OtherTaxes => Group::Taxes,
            Post::Intangible
            | Post::Buildings
            | Post::LeaseholdImprovements
            | Post::Machinery
            | Post::ConstructionInProgress
            | Post::FinancialFixed => Group::FixedAssets,
            Post::Inventory
            | Post::Receivables
            | Post::OtherReceivables
            | Post::Prepaid
            | Post::ShortTermInvestments
            | Post::Cash => Group::CurrentAssets,
            Post::RestrictedEquity
            | Post::RetainedEarnings
            | Post::OwnersEquity
            | Post::ResultForYear => Group::Equity,
            Post::UntaxedReserves => Group::UntaxedReserves,
            Post::Provisions => Group::Provisions,
            Post::LongTermLiabilities => Group::LongTerm,
            Post::Payables
            | Post::TaxLiabilities
            | Post::OtherCurrentLiabilities
            | Post::Accrued => Group::ShortTerm,
        }
    }

    /// Assets show a debit balance as positive; everything else in the
    /// balansräkning shows a credit balance as positive.
    fn is_asset(self) -> bool {
        matches!(self.group(), Group::FixedAssets | Group::CurrentAssets)
    }
}

/// The resultaträkning post of `account`. None for 8990–8999, where the
/// result voucher only moves the result to equity, and for 1000–2999.
// ponytail: 4000–4799 count as råvaror; a trading company may want them
// under handelsvaror, which needs a per-company choice.
pub fn income_post(account: u32) -> Option<Post> {
    Some(match account {
        3000..=3799 => Post::NetSales,
        3800..=3899 => Post::CapitalizedWork,
        3900..=3999 => Post::OtherOperatingIncome,
        4940..=4959 | 4970..=4979 => Post::InventoryChange,
        4960..=4969 => Post::Goods,
        4000..=4999 => Post::RawMaterials,
        5000..=6999 => Post::OtherExternalExpenses,
        7000..=7699 => Post::Personnel,
        7740..=7749 | 7790..=7799 => Post::CurrentAssetWritedowns,
        7700..=7899 => Post::Depreciation,
        7900..=7999 => Post::OtherOperatingExpenses,
        8000..=8099 => Post::GroupShares,
        8100..=8199 => Post::AssociateShares,
        8200..=8299 => Post::OtherSecurities,
        8300..=8399 => Post::InterestIncome,
        8400..=8799 => Post::InterestExpenses,
        8800..=8899 => Post::Appropriations,
        8900..=8979 => Post::IncomeTax,
        8980..=8989 => Post::OtherTaxes,
        _ => return None,
    })
}

/// The balansräkning post of `account`. Every resultaträkning account
/// (3000–8999, 8999 included) folds into "Årets resultat", so the year's
/// result shows there whether or not the year is closed.
pub fn balance_post(account: u32, legal_form: LegalForm) -> Option<Post> {
    let result = u32::from(result_account(legal_form).get());
    Some(match account {
        1000..=1099 => Post::Intangible,
        1120..=1129 => Post::LeaseholdImprovements,
        1180..=1189 | 1280..=1289 => Post::ConstructionInProgress,
        1100..=1199 => Post::Buildings,
        1200..=1299 => Post::Machinery,
        1300..=1399 => Post::FinancialFixed,
        1400..=1499 => Post::Inventory,
        1500..=1599 => Post::Receivables,
        1600..=1699 => Post::OtherReceivables,
        1700..=1799 => Post::Prepaid,
        1800..=1899 => Post::ShortTermInvestments,
        1900..=1999 => Post::Cash,
        a if a == result || (3000..=8999).contains(&a) => Post::ResultForYear,
        // Enskild firma, HB and KB: the owners' capital is one post.
        2000..=2099 if result == 2019 => Post::OwnersEquity,
        2080..=2089 => Post::RestrictedEquity,
        2000..=2099 => Post::RetainedEarnings,
        2100..=2199 => Post::UntaxedReserves,
        2200..=2299 => Post::Provisions,
        2300..=2399 => Post::LongTermLiabilities,
        2440..=2449 => Post::Payables,
        2500..=2599 => Post::TaxLiabilities,
        2900..=2999 => Post::Accrued,
        2400..=2899 => Post::OtherCurrentLiabilities,
        _ => return None,
    })
}
```

`Group`, `group` and `is_asset` are unused until Task 2. If `cargo clippy` flags them as dead code in this commit, add `#[allow(dead_code)]` on them now and remove it in Task 2.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-ledger --test statements`
Expected: PASS (4 tests).

- [ ] **Step 5: Lint and commit**

```bash
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger/src/statements.rs crates/ledger/src/lib.rs crates/ledger/src/domain.rs crates/ledger/tests/statements.rs
git commit -m "Map every BAS account to an ÅRL post"
```

---

### Task 2: Build the two statements from a saldobalans

**Files:**
- Modify: `crates/ledger/src/statements.rs`
- Test: `crates/ledger/tests/statements.rs`

**Interfaces:**
- Consumes:
  - `Post`, `Group`, `income_post`, `balance_post`, `Post::group` and `Post::is_asset` from Task 1.
  - `domain::TrialBalanceRow { account: u32, name: String, opening: i64, debit: i64, credit: i64 }`.
  - `crate::{Error, Result}`, where `Error::Overflow` exists.
- Produces:
  - `pub enum LineKind { Heading, Item, Subtotal }`.
  - `pub struct StatementLine { pub label: &'static str, pub kind: LineKind, pub amount: i64, pub previous: Option<i64> }`.
  - `pub struct FinancialStatements { pub income: Vec<StatementLine>, pub balance: Vec<StatementLine>, pub difference: i64, pub previous_difference: Option<i64>, pub previous_fiscal_year_start: Option<Date> }`.
  - `pub fn build(current: &[TrialBalanceRow], previous: Option<(Date, &[TrialBalanceRow])>, legal_form: LegalForm) -> Result<FinancialStatements>`.

Headings carry `amount: 0, previous: None`. `difference` is "Summa tillgångar" − "Summa eget kapital och skulder".

- [ ] **Step 1: Write the failing tests**

Change the `use` line at the top of `crates/ledger/tests/statements.rs` to:

```rust
use doris_company::domain::LegalForm;
use doris_ledger::Error;
use doris_ledger::domain::TrialBalanceRow;
use doris_ledger::statements::{
    FinancialStatements, LineKind, Post, StatementLine, balance_post, build, income_post,
};
use jiff::civil::Date;
```

Append to the file:

```rust
fn row(account: u32, opening: i64, debit: i64, credit: i64) -> TrialBalanceRow {
    TrialBalanceRow {
        account,
        name: String::new(),
        opening,
        debit,
        credit,
    }
}

fn line<'a>(lines: &'a [StatementLine], label: &str) -> &'a StatementLine {
    lines
        .iter()
        .find(|l| l.label == label)
        .unwrap_or_else(|| panic!("no line {label}"))
}

fn amount(lines: &[StatementLine], label: &str) -> i64 {
    line(lines, label).amount
}

fn shown(lines: &[StatementLine], label: &str) -> bool {
    lines.iter().any(|l| l.label == label)
}

/// An open year: 1 000 sold, 300 rent, paid through the bank.
fn trading() -> Vec<TrialBalanceRow> {
    vec![row(1930, 0, 1_000, 300), row(3001, 0, 0, 1_000), row(5010, 0, 300, 0)]
}

fn ab(rows: &[TrialBalanceRow]) -> FinancialStatements {
    build(rows, None, LegalForm::Aktiebolag).unwrap()
}

#[test]
fn the_income_statement_runs_down_to_the_years_result() {
    let s = ab(&trading());
    assert_eq!(amount(&s.income, "Nettoomsättning"), 1_000);
    assert_eq!(amount(&s.income, "Övriga externa kostnader"), -300);
    assert_eq!(amount(&s.income, "Summa rörelseintäkter, lagerförändringar m.m."), 1_000);
    assert_eq!(amount(&s.income, "Summa rörelsekostnader"), -300);
    assert_eq!(amount(&s.income, "Rörelseresultat"), 700);
    assert_eq!(amount(&s.income, "Resultat efter finansiella poster"), 700);
    assert_eq!(amount(&s.income, "Resultat före skatt"), 700);
    assert_eq!(amount(&s.income, "Årets resultat"), 700);
    assert_eq!(s.income[0].label, "Rörelseintäkter, lagerförändringar m.m.");
    assert_eq!(s.income[0].kind, LineKind::Heading);
    assert_eq!(line(&s.income, "Nettoomsättning").kind, LineKind::Item);
    assert_eq!(line(&s.income, "Rörelseresultat").kind, LineKind::Subtotal);
    assert_eq!(s.income.last().unwrap().label, "Årets resultat");
}

#[test]
fn the_balance_sheet_carries_the_result_into_equity() {
    let s = ab(&trading());
    assert_eq!(amount(&s.balance, "Kassa och bank"), 700);
    assert_eq!(amount(&s.balance, "Summa omsättningstillgångar"), 700);
    assert_eq!(amount(&s.balance, "Summa tillgångar"), 700);
    assert_eq!(amount(&s.balance, "Årets resultat"), 700);
    assert_eq!(amount(&s.balance, "Summa eget kapital"), 700);
    assert_eq!(amount(&s.balance, "Summa eget kapital och skulder"), 700);
    assert_eq!(s.difference, 0);
    assert_eq!(s.balance[0].label, "Tillgångar");
}

#[test]
fn a_closed_year_reads_exactly_like_the_open_one() {
    for (form, result_account) in [
        (LegalForm::Aktiebolag, 2099),
        (LegalForm::EnskildFirma, 2019),
        (LegalForm::Handelsbolag, 2019),
    ] {
        let open = build(&trading(), None, form).unwrap();
        // "Årets resultat": a 700 profit is debited to 8999.
        let mut rows = trading();
        rows.push(row(result_account, 0, 0, 700));
        rows.push(row(8999, 0, 700, 0));
        let closed = build(&rows, None, form).unwrap();
        assert_eq!(closed.income, open.income, "{form:?}");
        assert_eq!(closed.balance, open.balance, "{form:?}");
        assert_eq!(closed.difference, 0, "{form:?}");
        assert_eq!(amount(&closed.balance, "Årets resultat"), 700, "{form:?}");
    }
}

#[test]
fn equity_is_split_for_a_company_and_whole_for_an_owner() {
    let rows = [
        row(1930, 10_000, 1_000, 0),
        row(2010, -4_000, 0, 0),
        row(2081, -6_000, 0, 0),
        row(3001, 0, 0, 1_000),
    ];
    let company = build(&rows, None, LegalForm::Aktiebolag).unwrap();
    assert!(shown(&company.balance, "Fritt eget kapital"));
    assert_eq!(amount(&company.balance, "Bundet eget kapital"), 6_000);
    assert_eq!(amount(&company.balance, "Balanserat resultat"), 4_000);
    assert_eq!(amount(&company.balance, "Årets resultat"), 1_000);
    assert_eq!(company.difference, 0);

    let owner = build(&rows, None, LegalForm::EnskildFirma).unwrap();
    assert!(!shown(&owner.balance, "Bundet eget kapital"));
    assert!(!shown(&owner.balance, "Fritt eget kapital"));
    assert_eq!(amount(&owner.balance, "Eget kapital"), 10_000);
    assert_eq!(amount(&owner.balance, "Årets resultat"), 1_000);
    assert_eq!(amount(&owner.balance, "Summa eget kapital"), 11_000);
    assert_eq!(owner.difference, 0);
}

#[test]
fn posts_without_amounts_in_either_year_are_hidden() {
    let now = [row(1930, 0, 1_000, 0), row(3001, 0, 0, 1_000)];
    let before = trading();
    let start: Date = "2025-01-01".parse().unwrap();
    let s = build(&now, Some((start, &before)), LegalForm::Aktiebolag).unwrap();

    let rent = line(&s.income, "Övriga externa kostnader");
    assert_eq!((rent.amount, rent.previous), (0, Some(-300)));
    assert!(!shown(&s.income, "Personalkostnader"));
    // Headings and subtotals stay, even at 0.
    assert!(shown(&s.income, "Finansiella poster"));
    let financial = line(&s.income, "Summa finansiella poster");
    assert_eq!((financial.amount, financial.previous), (0, Some(0)));
    let net = line(&s.income, "Nettoomsättning");
    assert_eq!((net.amount, net.previous), (1_000, Some(1_000)));
    assert_eq!(s.previous_fiscal_year_start, Some(start));
    assert_eq!(s.previous_difference, Some(0));
}

#[test]
fn without_a_previous_year_nothing_has_a_comparison() {
    let s = ab(&trading());
    assert!(s.income.iter().chain(&s.balance).all(|l| l.previous.is_none()));
    assert_eq!(s.previous_difference, None);
    assert_eq!(s.previous_fiscal_year_start, None);
}

#[test]
fn an_earlier_open_year_leaves_a_difference() {
    // Year 2 while year 1 (a 700 profit) is open: the cash came in, but
    // the result never reached equity.
    let rows = [row(1930, 10_700, 0, 0), row(2081, -10_000, 0, 0)];
    let s = ab(&rows);
    assert_eq!(amount(&s.balance, "Summa tillgångar"), 10_700);
    assert_eq!(amount(&s.balance, "Summa eget kapital och skulder"), 10_000);
    assert_eq!(s.difference, 700);
}

#[test]
fn liabilities_show_credit_balances_as_positive() {
    let rows = [
        row(1930, 0, 500, 0),
        row(2440, 0, 0, 200),
        row(2350, 0, 0, 300),
    ];
    let s = ab(&rows);
    assert_eq!(amount(&s.balance, "Leverantörsskulder"), 200);
    assert_eq!(amount(&s.balance, "Summa kortfristiga skulder"), 200);
    assert_eq!(amount(&s.balance, "Långfristiga skulder"), 300);
    assert_eq!(s.difference, 0);
}

#[test]
fn an_overflow_is_an_error_not_a_panic() {
    let one_row = [row(1930, i64::MAX, 1, 0)];
    assert!(matches!(
        build(&one_row, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
    let two_rows = [row(1930, i64::MAX, 0, 0), row(1910, 1, 0, 0)];
    assert!(matches!(
        build(&two_rows, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
    let negated = [row(2081, i64::MIN, 0, 0)];
    assert!(matches!(
        build(&negated, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-ledger --test statements`
Expected: FAIL to compile with "unresolved imports `doris_ledger::statements::FinancialStatements`, `LineKind`, `StatementLine`, `build`".

- [ ] **Step 3: Write the minimal implementation**

Replace the `use` lines at the top of `crates/ledger/src/statements.rs` with:

```rust
use crate::domain::{TrialBalanceRow, result_account};
use crate::{Error, Result};
use doris_company::domain::LegalForm;
use jiff::civil::Date;
use std::collections::BTreeMap;
```

Remove any `#[allow(dead_code)]` added in Task 1. Append:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    Item,
    Subtotal,
}

/// One line of a statement, in öre with the sign the statement shows.
/// Headings carry no amounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementLine {
    pub label: &'static str,
    pub kind: LineKind,
    pub amount: i64,
    /// None when there is no previous year.
    pub previous: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialStatements {
    pub income: Vec<StatementLine>,
    pub balance: Vec<StatementLine>,
    /// Summa tillgångar − summa eget kapital och skulder: 0 unless an
    /// earlier year is open, so its result never reached equity.
    pub difference: i64,
    pub previous_difference: Option<i64>,
    pub previous_fiscal_year_start: Option<Date>,
}

#[derive(Clone, Copy)]
enum Entry {
    Heading(&'static str),
    Item(Post),
    Subtotal(&'static str, &'static [Group]),
}

const ASSET_GROUPS: &[Group] = &[Group::FixedAssets, Group::CurrentAssets];
const CLAIM_GROUPS: &[Group] = &[
    Group::Equity,
    Group::UntaxedReserves,
    Group::Provisions,
    Group::LongTerm,
    Group::ShortTerm,
];

/// Kostnadsslagsindelad resultaträkning, ÅRL bilaga 2.
const INCOME: &[Entry] = &[
    Entry::Heading("Rörelseintäkter, lagerförändringar m.m."),
    Entry::Item(Post::NetSales),
    Entry::Item(Post::InventoryChange),
    Entry::Item(Post::CapitalizedWork),
    Entry::Item(Post::OtherOperatingIncome),
    Entry::Subtotal(
        "Summa rörelseintäkter, lagerförändringar m.m.",
        &[Group::OperatingIncome],
    ),
    Entry::Heading("Rörelsekostnader"),
    Entry::Item(Post::RawMaterials),
    Entry::Item(Post::Goods),
    Entry::Item(Post::OtherExternalExpenses),
    Entry::Item(Post::Personnel),
    Entry::Item(Post::Depreciation),
    Entry::Item(Post::CurrentAssetWritedowns),
    Entry::Item(Post::OtherOperatingExpenses),
    Entry::Subtotal("Summa rörelsekostnader", &[Group::OperatingExpenses]),
    Entry::Subtotal(
        "Rörelseresultat",
        &[Group::OperatingIncome, Group::OperatingExpenses],
    ),
    Entry::Heading("Finansiella poster"),
    Entry::Item(Post::GroupShares),
    Entry::Item(Post::AssociateShares),
    Entry::Item(Post::OtherSecurities),
    Entry::Item(Post::InterestIncome),
    Entry::Item(Post::InterestExpenses),
    Entry::Subtotal("Summa finansiella poster", &[Group::Financial]),
    Entry::Subtotal(
        "Resultat efter finansiella poster",
        &[Group::OperatingIncome, Group::OperatingExpenses, Group::Financial],
    ),
    Entry::Item(Post::Appropriations),
    Entry::Subtotal(
        "Resultat före skatt",
        &[
            Group::OperatingIncome,
            Group::OperatingExpenses,
            Group::Financial,
            Group::Appropriations,
        ],
    ),
    Entry::Item(Post::IncomeTax),
    Entry::Item(Post::OtherTaxes),
    Entry::Subtotal(
        "Årets resultat",
        &[
            Group::OperatingIncome,
            Group::OperatingExpenses,
            Group::Financial,
            Group::Appropriations,
            Group::Taxes,
        ],
    ),
];

/// Balansräkning, ÅRL bilaga 1, up to equity.
const ASSETS: &[Entry] = &[
    Entry::Heading("Tillgångar"),
    Entry::Heading("Anläggningstillgångar"),
    Entry::Item(Post::Intangible),
    Entry::Item(Post::Buildings),
    Entry::Item(Post::LeaseholdImprovements),
    Entry::Item(Post::Machinery),
    Entry::Item(Post::ConstructionInProgress),
    Entry::Item(Post::FinancialFixed),
    Entry::Subtotal("Summa anläggningstillgångar", &[Group::FixedAssets]),
    Entry::Heading("Omsättningstillgångar"),
    Entry::Item(Post::Inventory),
    Entry::Item(Post::Receivables),
    Entry::Item(Post::OtherReceivables),
    Entry::Item(Post::Prepaid),
    Entry::Item(Post::ShortTermInvestments),
    Entry::Item(Post::Cash),
    Entry::Subtotal("Summa omsättningstillgångar", &[Group::CurrentAssets]),
    Entry::Subtotal("Summa tillgångar", ASSET_GROUPS),
    Entry::Heading("Eget kapital och skulder"),
];

/// Aktiebolag, ekonomisk förening and the other forms.
const COMPANY_EQUITY: &[Entry] = &[
    Entry::Heading("Eget kapital"),
    Entry::Item(Post::RestrictedEquity),
    Entry::Heading("Fritt eget kapital"),
    Entry::Item(Post::RetainedEarnings),
    Entry::Item(Post::ResultForYear),
    Entry::Subtotal("Summa eget kapital", &[Group::Equity]),
];

/// Enskild firma, HB and KB.
const OWNERS_EQUITY: &[Entry] = &[
    Entry::Item(Post::OwnersEquity),
    Entry::Item(Post::ResultForYear),
    Entry::Subtotal("Summa eget kapital", &[Group::Equity]),
];

const LIABILITIES: &[Entry] = &[
    Entry::Item(Post::UntaxedReserves),
    Entry::Item(Post::Provisions),
    Entry::Item(Post::LongTermLiabilities),
    Entry::Heading("Kortfristiga skulder"),
    Entry::Item(Post::Payables),
    Entry::Item(Post::TaxLiabilities),
    Entry::Item(Post::OtherCurrentLiabilities),
    Entry::Item(Post::Accrued),
    Entry::Subtotal("Summa kortfristiga skulder", &[Group::ShortTerm]),
    Entry::Subtotal("Summa eget kapital och skulder", CLAIM_GROUPS),
];

type Totals = BTreeMap<Post, i64>;

/// The resultaträkning and balansräkning of `current`, with `previous`
/// (its start and saldobalans) as the comparison year.
pub fn build(
    current: &[TrialBalanceRow],
    previous: Option<(Date, &[TrialBalanceRow])>,
    legal_form: LegalForm,
) -> Result<FinancialStatements> {
    let (income_now, balance_now) = totals(current, legal_form)?;
    let before = previous
        .map(|(_, rows)| totals(rows, legal_form))
        .transpose()?;
    // The owners of an enskild firma, HB or KB keep the result on 2019.
    let equity = if result_account(legal_form).get() == 2019 {
        OWNERS_EQUITY
    } else {
        COMPANY_EQUITY
    };
    let balance_layout = [ASSETS, equity, LIABILITIES].concat();
    Ok(FinancialStatements {
        income: lines(INCOME, &income_now, before.as_ref().map(|(i, _)| i))?,
        balance: lines(&balance_layout, &balance_now, before.as_ref().map(|(_, b)| b))?,
        difference: difference(&balance_now)?,
        previous_difference: before.as_ref().map(|(_, b)| difference(b)).transpose()?,
        previous_fiscal_year_start: previous.map(|(start, _)| start),
    })
}

/// Each post's amount, with the sign its statement shows.
fn totals(rows: &[TrialBalanceRow], legal_form: LegalForm) -> Result<(Totals, Totals)> {
    let (mut income, mut balance) = (Totals::new(), Totals::new());
    for row in rows {
        let closing = row
            .opening
            .checked_add(row.debit)
            .and_then(|b| b.checked_sub(row.credit))
            .ok_or(Error::Overflow)?;
        let credit = closing.checked_neg().ok_or(Error::Overflow)?;
        if let Some(post) = income_post(row.account) {
            add(&mut income, post, credit)?;
        }
        if let Some(post) = balance_post(row.account, legal_form) {
            add(&mut balance, post, if post.is_asset() { closing } else { credit })?;
        }
    }
    Ok((income, balance))
}

fn add(totals: &mut Totals, post: Post, amount: i64) -> Result<()> {
    let sum = totals.entry(post).or_insert(0);
    *sum = sum.checked_add(amount).ok_or(Error::Overflow)?;
    Ok(())
}

fn sum(totals: &Totals, groups: &[Group]) -> Result<i64> {
    totals
        .iter()
        .filter(|(post, _)| groups.contains(&post.group()))
        .try_fold(0i64, |acc, (_, amount)| {
            acc.checked_add(*amount).ok_or(Error::Overflow)
        })
}

fn difference(balance: &Totals) -> Result<i64> {
    sum(balance, ASSET_GROUPS)?
        .checked_sub(sum(balance, CLAIM_GROUPS)?)
        .ok_or(Error::Overflow)
}

fn lines(layout: &[Entry], now: &Totals, before: Option<&Totals>) -> Result<Vec<StatementLine>> {
    let mut out = Vec::new();
    for entry in layout {
        let (label, kind, amount, previous) = match *entry {
            Entry::Heading(label) => (label, LineKind::Heading, 0, None),
            Entry::Item(post) => {
                let amount = now.get(&post).copied().unwrap_or(0);
                let previous = before.map(|b| b.get(&post).copied().unwrap_or(0));
                if amount == 0 && previous.unwrap_or(0) == 0 {
                    continue;
                }
                (post.label(), LineKind::Item, amount, previous)
            }
            Entry::Subtotal(label, groups) => (
                label,
                LineKind::Subtotal,
                sum(now, groups)?,
                before.map(|b| sum(b, groups)).transpose()?,
            ),
        };
        out.push(StatementLine {
            label,
            kind,
            amount,
            previous,
        });
    }
    Ok(out)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-ledger --test statements`
Expected: PASS (13 tests).

- [ ] **Step 5: Lint and commit**

```bash
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger/src/statements.rs crates/ledger/tests/statements.rs
git commit -m "Build the resultaträkning and balansräkning from a saldobalans"
```

---

### Task 3: Read the statements for a fiscal year and the year before

**Files:**
- Modify: `crates/ledger/src/queries.rs` (append after `trial_balance`, around line 216)
- Modify: `crates/ledger/src/lib.rs:26-29` (`pub use queries::{…}`)
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes:
  - `statements::{build, FinancialStatements}` from Task 2.
  - `trial_balance(pool, company_id, user_id, fiscal_year_start) -> Result<Vec<TrialBalanceRow>>`.
  - `crate::fiscal_year_at(&Company, Date, Date) -> Option<FiscalYear>`, private in `lib.rs` but visible to child modules.
  - `doris_company::get_company`.
- Produces: `pub async fn financial_statements(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, fiscal_year_start: Date, today: Date) -> Result<FinancialStatements>`, re-exported as `doris_ledger::financial_statements`.

- [ ] **Step 1: Write the failing tests**

In `crates/ledger/tests/store.rs`, add `financial_statements` to the `use doris_ledger::{…}` list. Add `use doris_ledger::statements::StatementLine;` below it. Then append:

```rust
fn post(lines: &[StatementLine], label: &str) -> (i64, Option<i64>) {
    let line = lines.iter().find(|l| l.label == label).unwrap();
    (line.amount, line.previous)
}

#[tokio::test]
async fn the_statements_compare_with_the_year_before() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    for cmd in [sale("2025-03-01", 1_000), sale("2026-02-01", 50)] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    let first = financial_statements(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    assert_eq!(first.previous_fiscal_year_start, None);
    assert_eq!(post(&first.income, "Nettoomsättning"), (1_000, None));

    // 2025 is open, so 2026's balansräkning is short its result.
    let open = financial_statements(&pool, id, anna, d("2026-01-01"), today)
        .await
        .unwrap();
    assert_eq!(open.previous_fiscal_year_start, Some(d("2025-01-01")));
    assert_eq!(post(&open.income, "Nettoomsättning"), (50, Some(1_000)));
    assert_eq!(post(&open.balance, "Kassa och bank"), (1_050, Some(1_000)));
    assert_eq!((open.difference, open.previous_difference), (1_000, Some(0)));

    close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    let closed = financial_statements(&pool, id, anna, d("2026-01-01"), today)
        .await
        .unwrap();
    assert_eq!((closed.difference, closed.previous_difference), (0, Some(0)));
    // 2025's result stays on 2099 until it is moved to 2098 by hand.
    assert_eq!(post(&closed.balance, "Årets resultat"), (1_050, Some(1_000)));
}

#[tokio::test]
async fn the_statements_need_a_fiscal_year_start_and_membership() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    // Mid-year, before the first year, and after today.
    for start in ["2025-02-01", "2024-01-01", "2027-01-01", "9999-01-01"] {
        assert!(
            matches!(
                financial_statements(&pool, id, anna, d(start), today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "{start}"
        );
    }
    // Membership is checked before the year.
    assert!(matches!(
        financial_statements(&pool, id, bo, d("2025-02-01"), today).await,
        Err(Error::NotFound)
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-ledger --test store statements`
Expected: FAIL to compile with "no `financial_statements` in the root".

- [ ] **Step 3: Write the minimal implementation**

In `crates/ledger/src/queries.rs`, add `use crate::statements::{FinancialStatements, build};` to the imports. Append after `trial_balance`:

```rust
/// The resultaträkning and balansräkning for the fiscal year starting on
/// `fiscal_year_start`, with the year before it (if any) for comparison.
// ponytail: two saldobalans queries, so two snapshots; a voucher booked in
// between can show in one column only. Share a read transaction if that
// ever matters.
pub async fn financial_statements(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    today: Date,
) -> Result<FinancialStatements> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let fiscal_year = crate::fiscal_year_at(&company, fiscal_year_start, today)
        .ok_or(DomainError::FiscalYearNotFound)?;
    let current = trial_balance(pool, company_id, user_id, fiscal_year.start).await?;
    let previous = if fiscal_year == company.first_fiscal_year {
        None
    } else {
        let day_before = fiscal_year
            .start
            .yesterday()
            .expect("fiscal years are far from the date limits");
        let start = company.first_fiscal_year.containing(day_before).start;
        Some((start, trial_balance(pool, company_id, user_id, start).await?))
    };
    build(
        &current,
        previous.as_ref().map(|(start, rows)| (*start, rows.as_slice())),
        company.legal_form,
    )
}
```

In `crates/ledger/src/lib.rs`, extend the re-export:

```rust
pub use queries::{
    account_ledger, financial_statements, get_attachment, list_accounts, list_fiscal_years,
    list_vouchers, opening_balances, trial_balance,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-ledger`
Expected: PASS, including the two new store tests and `stress.rs`.

- [ ] **Step 5: Lint and commit**

```bash
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger/src/queries.rs crates/ledger/src/lib.rs crates/ledger/tests/store.rs
git commit -m "Read a fiscal year's statements with the year before"
```

---

### Task 4: Serve `GetFinancialStatements` over gRPC-Web

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto` (service block at lines 9-40, and messages after `GetTrialBalanceResponse`)
- Modify: `crates/server/src/ledger.rs` (handler after `get_trial_balance`, around line 251, plus a `statement_line` helper next to the other mapping helpers)
- Test: `crates/server/tests/ledger.rs`

**Interfaces:**
- Consumes: `doris_ledger::financial_statements` from Task 3, and `doris_ledger::statements::{LineKind, StatementLine}`.
- Produces: the proto types `pb::GetFinancialStatementsRequest`, `pb::GetFinancialStatementsResponse`, `pb::StatementLine` and `pb::StatementLineKind { Unspecified, Heading, Item, Subtotal }`, and the RPC `get_financial_statements`.

- [ ] **Step 1: Write the failing test**

Append to `crates/server/tests/ledger.rs`:

```rust
fn statements_of(company_id: &str, fiscal_year_start: &str) -> pb::GetFinancialStatementsRequest {
    pb::GetFinancialStatementsRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

#[tokio::test]
async fn the_financial_statements_follow_the_vouchers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.record_voucher(authed(sale(&id, 125_000), &anna))
        .await
        .unwrap();

    let s = api
        .get_financial_statements(authed(statements_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(s.previous_fiscal_year_start, "");
    assert_eq!((s.difference, s.previous_difference), (0, None));
    assert_eq!(
        s.income_statement[0],
        pb::StatementLine {
            label: "Rörelseintäkter, lagerförändringar m.m.".into(),
            kind: pb::StatementLineKind::Heading as i32,
            amount: 0,
            previous: None,
        }
    );
    assert!(s.income_statement.contains(&pb::StatementLine {
        label: "Nettoomsättning".into(),
        kind: pb::StatementLineKind::Item as i32,
        amount: 125_000,
        previous: None,
    }));
    assert!(s.income_statement.contains(&pb::StatementLine {
        label: "Rörelseresultat".into(),
        kind: pb::StatementLineKind::Subtotal as i32,
        amount: 125_000,
        previous: None,
    }));
    assert!(s.balance_sheet.contains(&pb::StatementLine {
        label: "Kassa och bank".into(),
        kind: pb::StatementLineKind::Item as i32,
        amount: 125_000,
        previous: None,
    }));
}

#[tokio::test]
async fn the_financial_statements_refuse_bad_input_and_non_members() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();

    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-13-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_date".to_owned()));
    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-02-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "fiscal_year_not_found".to_owned()));
    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".to_owned()));
    let err = api
        .get_financial_statements(statements_of(&id, "2026-01-01"))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".to_owned()));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p doris-server --test ledger financial_statements`
Expected: FAIL to compile with "cannot find struct `GetFinancialStatementsRequest` in module `pb`".

- [ ] **Step 3: Add the proto contract**

In `proto/doris/ledger/v1/ledger.proto`, add inside `service LedgerService`, after `GetAccountLedger`:

```proto
  // The resultaträkning and balansräkning for one fiscal year under ÅRL
  // headings, with the year before as comparison.
  rpc GetFinancialStatements(GetFinancialStatementsRequest) returns (GetFinancialStatementsResponse);
```

Add after `message GetTrialBalanceResponse { … }`:

```proto
message GetFinancialStatementsRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}

enum StatementLineKind {
  STATEMENT_LINE_KIND_UNSPECIFIED = 0;
  STATEMENT_LINE_KIND_HEADING = 1;   // no amounts
  STATEMENT_LINE_KIND_ITEM = 2;
  STATEMENT_LINE_KIND_SUBTOTAL = 3;
}

// Öre, with the sign the statement shows: income, equity and liabilities
// positive, costs negative.
message StatementLine {
  string label = 1;
  StatementLineKind kind = 2;
  int64 amount = 3;
  optional int64 previous = 4;  // unset when there is no previous year
}

message GetFinancialStatementsResponse {
  repeated StatementLine income_statement = 1;
  repeated StatementLine balance_sheet = 2;
  string previous_fiscal_year_start = 3;  // empty when there is none
  // Summa tillgångar − summa eget kapital och skulder; not 0 only while an
  // earlier year is open.
  int64 difference = 4;
  optional int64 previous_difference = 5;
}
```

- [ ] **Step 4: Write the handler**

In `crates/server/src/ledger.rs`, add `use doris_ledger::statements::{LineKind, StatementLine};` to the imports. Add this method to `impl LedgerService for LedgerApi`, after `get_trial_balance`:

```rust
    async fn get_financial_statements(
        &self,
        request: Request<pb::GetFinancialStatementsRequest>,
    ) -> Result<Response<pb::GetFinancialStatementsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let statements = doris_ledger::financial_statements(
            &self.pool,
            company,
            user,
            fiscal_year_start,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetFinancialStatementsResponse {
            income_statement: statements.income.into_iter().map(statement_line).collect(),
            balance_sheet: statements.balance.into_iter().map(statement_line).collect(),
            previous_fiscal_year_start: statements
                .previous_fiscal_year_start
                .map(|start| start.to_string())
                .unwrap_or_default(),
            difference: statements.difference,
            previous_difference: statements.previous_difference,
        }))
    }
```

Add this free function next to `fn date`:

```rust
fn statement_line(line: StatementLine) -> pb::StatementLine {
    let kind = match line.kind {
        LineKind::Heading => pb::StatementLineKind::Heading,
        LineKind::Item => pb::StatementLineKind::Item,
        LineKind::Subtotal => pb::StatementLineKind::Subtotal,
    };
    pb::StatementLine {
        label: line.label.into(),
        kind: kind as i32,
        amount: line.amount,
        previous: line.previous,
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p doris-server --test ledger`
Expected: PASS, including both new tests.

- [ ] **Step 6: Lint and commit**

```bash
cargo clippy --workspace --all-targets -- -D warnings
git add proto/doris/ledger/v1/ledger.proto crates/server/src/ledger.rs crates/server/tests/ledger.rs
git commit -m "Serve GetFinancialStatements over gRPC-Web"
```

---

### Task 5: The Rapporter page

**Files:**
- Modify: `crates/web/src/fiscal_year.rs` (new `period`, with a test in its `mod tests`)
- Create: `crates/web/src/pages/financial_statements.rs`
- Modify: `crates/web/src/pages/mod.rs` (module and re-export)
- Modify: `crates/web/src/app.rs:7` (import), `:77` (route), `:129` (nav link)
- Modify: `e2e/tests/ledger.spec.ts` (new test after "the trial balance follows the active company")
- Modify: `AGENTS.md` (API section, after the `AddAttachment` bullet, and the event-sourcing bullet about the saldobalans)

**Interfaces:**
- Consumes:
  - `lpb::GetFinancialStatementsRequest` and `lpb::StatementLineKind` from Task 4, plus the generated accessor `lpb::StatementLine::kind()`.
  - `use_fiscal_years`, `keep_year_in_url`, `FiscalYearSelect` and `is_closed` from `fiscal_year.rs`.
  - `amount` from `format.rs`.
  - `Table` and `TABLE_*` from `ui.rs`, and `describe` from `errors.rs`.
- Produces:
  - `pub fn period(years: &[lpb::FiscalYear], start: &str) -> String`, which returns `"{start} – {end}"`, or `start` alone when the year isn't listed.
  - The `FinancialStatements` page component.

- [ ] **Step 1: Write the failing unit test**

In `crates/web/src/fiscal_year.rs`, inside `mod tests`, append:

```rust
    #[test]
    fn a_period_reads_start_to_end() {
        let list = vec![lpb::FiscalYear {
            start: "2025-07-01".into(),
            end: "2026-12-31".into(),
            closed: false,
        }];
        assert_eq!(period(&list, "2025-07-01"), "2025-07-01 – 2026-12-31");
        assert_eq!(period(&list, "2024-07-01"), "2024-07-01");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p doris-web a_period_reads_start_to_end`
Expected: FAIL to compile with "cannot find function `period`".

- [ ] **Step 3: Implement `period` and use it in the select**

In `crates/web/src/fiscal_year.rs`, add after `is_closed`:

```rust
/// "start – end" for the listed year starting on `start`, or `start` alone.
pub fn period(years: &[lpb::FiscalYear], start: &str) -> String {
    years
        .iter()
        .find(|y| y.start == start)
        .map(|y| format!("{} – {}", y.start, y.end))
        .unwrap_or_else(|| start.to_owned())
}
```

Leave `FiscalYearSelect` unchanged. Its options already format each year the same way, and they have the year itself at hand, not the list.

Run: `cargo test -p doris-web a_period_reads_start_to_end`
Expected: PASS.

- [ ] **Step 4: Write the failing e2e test**

In `e2e/tests/ledger.spec.ts`, add after the test "the trial balance follows the active company":

```ts
test("the income statement and balance sheet show what was booked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers`);
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await page.getByRole("banner").getByRole("link", { name: "Rapporter" }).click();
  await expect(page.getByRole("heading", { name: "Resultat- och balansräkning" })).toBeVisible();
  await expect(page.getByRole("row", { name: /^Nettoomsättning/ })).toContainText("1 250,00");
  const result = page.getByRole("row", { name: /^Årets resultat/ });
  await expect(result).toHaveCount(2);
  await expect(result.first()).toContainText("1 250,00");
  await expect(result.last()).toContainText("1 250,00");
  await expect(page.getByRole("row", { name: /^Kassa och bank/ })).toContainText("1 250,00");
  await expect(page.getByText(/balanserar inte/)).toHaveCount(0);

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });
  await expect(page.getByRole("row", { name: /^Nettoomsättning/ })).toHaveCount(0);
  await expect(page.getByRole("row", { name: /^Rörelseresultat/ })).toContainText("0,00");
});
```

Run: `make e2e` (or, after `make web`, `cd e2e && npx playwright test ledger.spec.ts -g "income statement"`).
Expected: FAIL on the "Rapporter" link, because it does not exist yet.

- [ ] **Step 5: Write the page**

Create `crates/web/src/pages/financial_statements.rs`:

```rust
//! Resultat- och balansräkning for one fiscal year, with the year before.
//! The server sends finished lines; this page only draws them.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, is_closed, keep_year_in_url, period, use_fiscal_years};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_query_map;

#[component]
pub fn FinancialStatements() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    keep_year_in_url("/financial-statements".into(), year);
    // None until the chosen year's statements have arrived.
    let statements = RwSignal::new(None::<lpb::GetFinancialStatementsResponse>);

    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        statements.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_financial_statements(lpb::GetFinancialStatementsRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => statements.set(Some(response.into_inner())),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let closed = move || years.with(|ys| is_closed(ys, &year.get()));

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Resultat- och balansräkning"</h1>
            <ErrorAlert message=error />
            <div class="flex items-end gap-4">
                <FiscalYearSelect years=years year=year />
                <Show when=closed>
                    <span class="pb-2 text-xs/relaxed text-muted-foreground">"Räkenskapsåret är stängt"</span>
                </Show>
            </div>
            {move || {
                statements.get().map(|s| {
                    let current = years.with_untracked(|ys| period(ys, &year.get_untracked()));
                    let previous = (!s.previous_fiscal_year_start.is_empty())
                        .then(|| years.with_untracked(|ys| period(ys, &s.previous_fiscal_year_start)));
                    let differences: Vec<i64> = std::iter::once(s.difference)
                        .chain(s.previous_difference)
                        .filter(|d| *d != 0)
                        .collect();
                    view! {
                        <StatementTable title="Resultaträkning" lines=s.income_statement current=current.clone() previous=previous.clone() />
                        <StatementTable title="Balansräkning" lines=s.balance_sheet current=current previous=previous />
                        {differences
                            .into_iter()
                            .map(|d| view! {
                                <p class="text-xs/relaxed text-destructive">
                                    {format!(
                                        "Balansräkningen balanserar inte (differens {}). Ett tidigare räkenskapsår är inte stängt, så dess resultat finns inte i eget kapital.",
                                        amount(d)
                                    )}
                                </p>
                            })
                            .collect_view()}
                    }
                })
            }}
        </div>
    }
}

/// One statement: headings in bold without amounts, items indented,
/// subtotals in bold under a rule.
#[component]
fn StatementTable(
    title: &'static str,
    lines: Vec<lpb::StatementLine>,
    current: String,
    previous: Option<String>,
) -> impl IntoView {
    let has_previous = previous.is_some();
    view! {
        <section class="grid gap-2">
            <h2 class="text-sm font-medium">{title}</h2>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Post"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>{current}</th>
                        {previous.map(|p| view! { <th class=format!("{TABLE_HEADER_CELL} text-right")>{p}</th> })}
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {lines
                        .into_iter()
                        .map(|line| {
                            let kind = line.kind();
                            let previous = line.previous;
                            match kind {
                                lpb::StatementLineKind::Heading => view! {
                                    <tr class=TABLE_ROW>
                                        <td class=format!("{TABLE_CELL} font-medium") colspan=if has_previous { "3" } else { "2" }>{line.label}</td>
                                    </tr>
                                }
                                .into_any(),
                                _ => {
                                    let subtotal = kind == lpb::StatementLineKind::Subtotal;
                                    view! {
                                        <tr class=if subtotal { format!("{TABLE_ROW} border-t font-medium") } else { TABLE_ROW.to_owned() }>
                                            <td class=if subtotal { TABLE_CELL.to_owned() } else { format!("{TABLE_CELL} pl-6") }>{line.label}</td>
                                            <td class=TABLE_AMOUNT_CELL>{amount(line.amount)}</td>
                                            {has_previous.then(|| view! { <td class=TABLE_AMOUNT_CELL>{amount(previous.unwrap_or(0))}</td> })}
                                        </tr>
                                    }
                                    .into_any()
                                }
                            }
                        })
                        .collect_view()}
                </tbody>
            </Table>
        </section>
    }
}
```

In `crates/web/src/pages/mod.rs`, add `mod financial_statements;` (in alphabetical order, after `mod company;`) and `pub use financial_statements::FinancialStatements;` (after `pub use company::CompanyPage;`).

In `crates/web/src/app.rs`:
- Add `FinancialStatements` to the `use crate::pages::{…}` import on line 7.
- After the `/trial-balance/:account` route, add:
  ```rust
                          <Route path=path!("/financial-statements") view=|| view! { <SignedIn><FinancialStatements /></SignedIn> } />
  ```
- After the "Saldobalans" nav link, add:
  ```rust
                          <A href="/financial-statements" attr:class="text-muted-foreground hover:text-foreground">"Rapporter"</A>
  ```

- [ ] **Step 6: Run the e2e test and the lints**

Run:
```bash
cargo test -p doris-web
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
make e2e
```
Expected: all PASS, including the new Playwright test.

Then run `make dist`. Expected: it succeeds, which means the gzipped wasm is still within `WASM_BUDGET`.

- [ ] **Step 7: Document it in AGENTS.md**

In `AGENTS.md`, add this bullet under "## API", after the `AddAttachment` bullet:

```markdown
- `LedgerService` also has `GetFinancialStatements`: the resultaträkning
  and balansräkning for one fiscal year under ÅRL headings (K2's
  abbreviated forms), with the year before as comparison. The mapping from
  BAS account to post lives only in `crates/ledger/src/statements.rs`; the
  frontend draws the lines it gets. "Årets resultat" in the balansräkning
  is the result account plus every account 3000–8999, so it reads the same
  whether or not the year is closed.
```

- [ ] **Step 8: Commit**

```bash
git add crates/web/src/fiscal_year.rs crates/web/src/pages/financial_statements.rs crates/web/src/pages/mod.rs crates/web/src/app.rs e2e/tests/ledger.spec.ts AGENTS.md
git commit -m "Add the Rapporter page with the resultaträkning and balansräkning"
```
