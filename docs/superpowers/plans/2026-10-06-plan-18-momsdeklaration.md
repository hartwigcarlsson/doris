# Momsdeklaration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Doris tar fram momsdeklarationen per redovisningsperiod ur bokföringen, visar den som SKV 4700, skriver eSKD-filen och bokför momsavräkningen när perioden markeras inlämnad.

**Architecture:** Ledgern får en momsruta per konto (`AccountVatBoxSet` i kontoplanens ström, standard från BAS) och en query som summerar en periods konton med ruta. En ny crate `doris-vat` äger strömmen `vat-{company}` (redovisningsperiod per räkenskapsår och inlämningar), räknar rutor, avräkning, status och deklarationsdag i rena funktioner och bokför avräkningen med `doris_ledger::record_voucher_in` i samma transaktion som händelsen. En ny `VatService` (gRPC-Web) och två sidor (`/vat`, `/vat/{ÅÅÅÅMM}`) i Leptos.

**Tech Stack:** Rust 2024, jiff, serde, sqlx 0.9 (SQLite), sha2, tonic 0.14 + prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-06-momsdeklaration-design.md`

### Avsteg från specen (medvetna, enklare)
1. **Ingen kolumn `accounts.vat_box`.** Befintliga företags projektionsrader skulle behöva fyllas i med en SQL-kopia av standardkartan. I stället läses rutan ur kontoplanens händelser (`Chart`), både av `list_accounts` och `vat_box_totals_in`. Ingen ledger-migration.
2. **Inga projektioner `vat_periods`/`vat_returns`.** Allt `doris-vat` läser finns i den egna strömmen (`load`), även vem och när (händelsens metadata). Ingen migration alls.
3. Standardkartan ligger i `crates/ledger/src/vat_box.rs` bredvid `VatBox`, inte i `bas.rs`.
4. En inlämning utan något att bokföra (allt redan avräknat, eller noll moms) sparas utan verifikation (`voucher: None`).
5. Filen innehåller bara ASCII (taggar och siffror), så den är giltig ISO-8859-1 och skickas som `string`.
6. E2E bokför manuella verifikationer i förra årets fjärde kvartal (företaget börjar förra året), så testet inte beror på dagens datum.

## Global Constraints
- Kod, identifierare, URL:er, proto, händelsenamn och commits på engelska; bara synlig UI-text på svenska.
- TDD: inget utan ett test som först fallerar. Varje cykel slutar i en commit.
- Händelser är JSON med `schema_version` 1; `events` är append-only.
- Skrivningar i en `BEGIN IMMEDIATE` (`doris_eventstore::begin`); avräkningen och `VatReturnSubmitted` i samma transaktion.
- `doris-vat` läser inga ledger-tabeller själv; bara `doris_ledger::vat_box_totals_in`, `corrected_vouchers_in` och `record_voucher_in`.
- Belopp i öre internt; rutor i hela kronor, ören strukna mot noll.
- Felkoder snake_case, utan personuppgifter: `invalid_vat_box`, `invalid_vat_period`, `vat_period_not_ended`, `vat_period_locked`, `vat_return_outdated`, `vat_return_unchanged`, `vat_not_registered`.
- Ett personnummer (enskild firmas org nr) loggas aldrig.
- UI följer `docs/design/README.md`: `PageHeader`, `TableCard`, `Badge`, endast tokens, ljust/mörkt, 390 px. Sidor startar uppgifter med `crate::task::spawn_local`.
- Lint: `cargo clippy --workspace -- -D warnings` och `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.

## Review Focus
1. **Ett konto vars ruta byts efter en inlämning** (t.ex. 2614 från 30 till ingen): perioden ska bli Ändrad och nästa avräkning nolla tillbaka det som tidigare flyttats till 2650. Test i Task 7 (`an_account_that_lost_its_vat_box_is_settled_back`).
2. **Dubbelklick på "Bekräfta"**: den andra inlämningen får `vat_return_unchanged` och bokför ingen verifikation till. Test i Task 10 (`a_second_identical_submission_is_refused_and_books_nothing`), och knappen låses i Task 14.
3. **Ett stängt räkenskapsår**: inlämning nekas med `fiscal_year_closed` och lämnar ingen händelse. Test i Task 10.
4. **Brutet räkenskapsår med kvartal**: kvartalet april–juni hör till året där juni ligger, även om april är i året innan. Test i Task 5 (`quarters_follow_the_calendar_in_a_broken_year`).
5. **Negativa rutor** (kreditfakturor större än försäljningen): ören stryks mot noll och filen skriver `-` direkt före siffrorna. Test i Task 6 och Task 8.

---

### Task 1: `VatBox` och standardkartan (ledger, ren)

**Files:**
- Create: `crates/ledger/src/vat_box.rs`
- Modify: `crates/ledger/src/lib.rs` (lägg till `pub mod vat_box;`)
- Modify: `crates/ledger/src/domain.rs` (variant `InvalidVatBox` i `DomainError`)
- Test: `crates/ledger/tests/vat_box.rs`

**Interfaces:**
- Produces: `doris_ledger::vat_box::{VatBox, Side, BOXES, default_vat_box}`; `VatBox::parse(u32) -> Result<VatBox, DomainError>`, `VatBox::get(self) -> u8`, `VatBox::side(self) -> Side`, `VatBox::is_vat(self) -> bool`, `default_vat_box(account: u16) -> Option<VatBox>`; `DomainError::InvalidVatBox`.

- [ ] **Step 1: Write the failing test**

`crates/ledger/tests/vat_box.rs`:
```rust
use doris_ledger::domain::DomainError;
use doris_ledger::vat_box::{BOXES, Side, VatBox, default_vat_box};

fn b(n: u32) -> VatBox {
    VatBox::parse(n).unwrap()
}

#[test]
fn only_the_forms_boxes_parse_and_49_is_computed() {
    for n in [5, 8, 10, 24, 42, 48, 50, 62] {
        assert_eq!(b(n).get() as u32, n);
    }
    for n in [0, 1, 9, 13, 25, 49, 63, 300] {
        assert_eq!(VatBox::parse(n), Err(DomainError::InvalidVatBox), "{n}");
    }
    assert_eq!(BOXES.len(), 28);
}

#[test]
fn boxes_know_their_side_and_whether_they_carry_vat() {
    for n in [20, 21, 22, 23, 24, 48, 50] {
        assert_eq!(b(n).side(), Side::Debit, "{n}");
    }
    for n in [5, 6, 7, 8, 10, 11, 12, 30, 31, 32, 35, 39, 42, 60, 61, 62] {
        assert_eq!(b(n).side(), Side::Credit, "{n}");
    }
    let vat: Vec<u8> = BOXES.iter().copied().filter(|&n| b(n.into()).is_vat()).collect();
    assert_eq!(vat, [10, 11, 12, 30, 31, 32, 60, 61, 62, 48]);
}

#[test]
fn bas_accounts_have_their_boxes() {
    let cases = [
        (3001, Some(5)), (3002, Some(5)), (3003, Some(5)), (3106, Some(5)),
        (3004, Some(42)), (3108, Some(35)), (3305, Some(40)), (3308, Some(39)),
        (2611, Some(10)), (2621, Some(11)), (2631, Some(12)),
        (4515, Some(20)), (4535, Some(21)), (4531, Some(22)), (4415, Some(23)), (4425, Some(24)),
        (2614, Some(30)), (2624, Some(31)), (2634, Some(32)),
        (4545, Some(50)), (2615, Some(60)), (2625, Some(61)), (2635, Some(62)),
        (2640, Some(48)), (2641, Some(48)), (2645, Some(48)), (2647, Some(48)),
        (2650, None), (3740, None), (1930, None), (4010, None),
    ];
    for (account, expected) in cases {
        assert_eq!(default_vat_box(account).map(VatBox::get), expected, "{account}");
    }
}

#[test]
fn a_box_is_stored_as_its_number() {
    assert_eq!(serde_json::to_string(&b(10)).unwrap(), "10");
    assert_eq!(serde_json::from_str::<VatBox>("48").unwrap(), b(48));
    assert!(serde_json::from_str::<VatBox>("49").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p doris-ledger --test vat_box`
Expected: FAIL to compile (`vat_box` not found).

- [ ] **Step 3: Write minimal implementation**

In `crates/ledger/src/domain.rs`, `DomainError`, after `AttachmentNotFound`:
```rust
    #[error("no such box on the momsdeklaration, or not for this account")]
    InvalidVatBox,
```

`crates/ledger/src/vat_box.rs`:
```rust
//! The boxes (rutor) of Skatteverket's momsdeklaration (SKV 4700) and the
//! box each BAS account goes in by default. Pure.

use crate::domain::DomainError;
use serde::{Deserialize, Serialize};

/// Every box an account can be in, in the order of Skatteverket's file.
/// 49 is computed and is never an account's.
pub const BOXES: [u8; 28] = [
    5, 6, 7, 8, 20, 21, 22, 23, 24, 50, 35, 36, 37, 38, 39, 40, 41, 42, 10, 11, 12, 30, 31, 32,
    60, 61, 62, 48,
];

/// A box on the momsdeklaration, stored as its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct VatBox(u8);

/// The side of an account's saldo (debit − credit) a box counts as positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

impl VatBox {
    pub fn parse(n: u32) -> Result<Self, DomainError> {
        u8::try_from(n)
            .ok()
            .filter(|b| BOXES.contains(b))
            .map(Self)
            .ok_or(DomainError::InvalidVatBox)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// Purchases and input VAT are debits; sales and output VAT credits.
    pub fn side(self) -> Side {
        match self.0 {
            20..=24 | 48 | 50 => Side::Debit,
            _ => Side::Credit,
        }
    }

    /// The VAT itself: the settlement moves these accounts to 2650.
    pub fn is_vat(self) -> bool {
        matches!(self.0, 10..=12 | 30..=32 | 60..=62 | 48)
    }
}

impl TryFrom<u32> for VatBox {
    type Error = DomainError;
    fn try_from(n: u32) -> Result<Self, DomainError> {
        Self::parse(n)
    }
}

impl From<VatBox> for u32 {
    fn from(b: VatBox) -> u32 {
        b.0.into()
    }
}

/// The box BAS puts `account` in, looked up by number so it also holds for
/// accounts added later.
pub fn default_vat_box(account: u16) -> Option<VatBox> {
    let n = match account {
        3001..=3003 | 3106 => 5,
        3004 => 42,
        3108 => 35,
        3305 => 40,
        3308 => 39,
        2611 => 10,
        2621 => 11,
        2631 => 12,
        4515..=4517 => 20,
        4535..=4537 => 21,
        4531..=4533 => 22,
        4415..=4417 => 23,
        4425..=4427 => 24,
        2614 => 30,
        2624 => 31,
        2634 => 32,
        4545..=4547 => 50,
        2615 => 60,
        2625 => 61,
        2635 => 62,
        2640 | 2641 | 2645 | 2647 => 48,
        _ => return None,
    };
    Some(VatBox::parse(n).expect("default boxes are on the form"))
}
```

In `crates/ledger/src/lib.rs`, after `pub mod statements;`: `pub mod vat_box;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p doris-ledger --test vat_box`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/ledger/src/vat_box.rs crates/ledger/src/lib.rs crates/ledger/src/domain.rs crates/ledger/tests/vat_box.rs
git commit -m "Add the momsdeklaration's boxes and BAS's default box per account"
```

---

### Task 2: Momsruta per konto i kontoplanen

**Files:**
- Modify: `crates/ledger/src/domain.rs` (`ChartEvent::AccountVatBoxSet`, `Account.vat_box`, `Chart::apply`, `set_account_vat_box`)
- Modify: `crates/ledger/src/projections.rs` (`apply_chart`: ignore the new event)
- Modify: `crates/ledger/src/lib.rs` (`set_account_vat_box`, `chart_in`)
- Modify: `crates/ledger/src/queries.rs` (`list_accounts` builds from the chart's events)
- Test: `crates/ledger/tests/domain.rs`, `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: Task 1.
- Produces: `Account { number, name, active, vat_box: Option<VatBox> }`; `ChartEvent::AccountVatBoxSet { number: AccountNumber, vat_box: Option<VatBox> }`; `domain::set_account_vat_box(&Chart, AccountNumber, Option<VatBox>) -> Result<Vec<ChartEvent>, DomainError>`; `doris_ledger::set_account_vat_box(pool, company_id, actor, number: u32, vat_box: Option<u32>) -> Result<()>`; `pub(crate) async fn chart_in(conn, company_id) -> Result<Chart>` (no seeding append).

- [ ] **Step 1: Write the failing domain tests**

Append to `crates/ledger/tests/domain.rs` (use its existing imports; add `use doris_ledger::vat_box::VatBox;` and `set_account_vat_box` to the `domain::` import):
```rust
fn acct(n: u32) -> AccountNumber {
    AccountNumber::parse(n).unwrap()
}

#[test]
fn seeded_and_added_accounts_get_bas_boxes_and_a_set_box_wins() {
    let mut chart = Chart::from_events(&[seed_chart()]);
    assert_eq!(chart.get(acct(2611)).unwrap().vat_box, VatBox::parse(10).ok());
    assert_eq!(chart.get(acct(1930)).unwrap().vat_box, None);
    chart.apply(&ChartEvent::AccountAdded { number: acct(4535), name: AccountName::parse("Tjänst EU").unwrap() });
    assert_eq!(chart.get(acct(4535)).unwrap().vat_box, VatBox::parse(21).ok());
    chart.apply(&ChartEvent::AccountVatBoxSet { number: acct(2611), vat_box: None });
    assert_eq!(chart.get(acct(2611)).unwrap().vat_box, None);
}

#[test]
fn a_box_is_set_once_and_never_on_2650_or_3740() {
    let chart = Chart::from_events(&[seed_chart()]);
    let b42 = VatBox::parse(42).ok();
    assert_eq!(
        set_account_vat_box(&chart, acct(3004), VatBox::parse(5).ok()).unwrap(),
        [ChartEvent::AccountVatBoxSet { number: acct(3004), vat_box: VatBox::parse(5).ok() }]
    );
    assert_eq!(set_account_vat_box(&chart, acct(3004), b42).unwrap(), []);
    assert_eq!(set_account_vat_box(&chart, acct(2650), b42), Err(DomainError::InvalidVatBox));
    assert_eq!(set_account_vat_box(&chart, acct(3740), b42), Err(DomainError::InvalidVatBox));
    assert!(set_account_vat_box(&chart, acct(2650), None).unwrap().is_empty());
    // 1234 is not in the BAS selection.
    assert_eq!(set_account_vat_box(&chart, acct(1234), None), Err(DomainError::AccountNotFound));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p doris-ledger --test domain`
Expected: FAIL to compile (`vat_box` field, `AccountVatBoxSet`, `set_account_vat_box` missing).

- [ ] **Step 3: Implement the domain**

In `crates/ledger/src/domain.rs`:
- `use crate::vat_box::{VatBox, default_vat_box};`
- In `ChartEvent`, after `AccountReactivated`:
```rust
    /// The account's box on the momsdeklaration. None: no box, even where
    /// BAS has one.
    AccountVatBoxSet {
        number: AccountNumber,
        vat_box: Option<VatBox>,
    },
```
- `Account` gets `pub vat_box: Option<VatBox>,`.
- In `Chart::apply`, both `Account { … }` literals (seed and add) get `vat_box: default_vat_box(number.get()),`, and add the arm:
```rust
            ChartEvent::AccountVatBoxSet { number, vat_box } => {
                if let Some(account) = self.accounts.get_mut(&number) {
                    account.vat_box = vat_box;
                }
            }
```
- After `set_account_active`:
```rust
/// Setting the box it already has yields no events. 2650 and 3740 take
/// no box: the settlement itself books on them.
pub fn set_account_vat_box(
    chart: &Chart,
    number: AccountNumber,
    vat_box: Option<VatBox>,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    if vat_box.is_some() && matches!(number.get(), 2650 | 3740) {
        return Err(DomainError::InvalidVatBox);
    }
    if account.vat_box == vat_box {
        return Ok(vec![]);
    }
    Ok(vec![ChartEvent::AccountVatBoxSet { number, vat_box }])
}
```
In `crates/ledger/src/projections.rs`, `apply_chart`, add `ChartEvent::AccountVatBoxSet { .. } => {}` (the box is read from the chart's events, not projected).

- [ ] **Step 4: Run the domain tests**

Run: `cargo test -p doris-ledger --test domain`
Expected: PASS. Fix any other `Account { … }` literal the compiler reports (e.g. in `queries.rs`, handled next step).

- [ ] **Step 5: Write the failing store test**

Append to `crates/ledger/tests/store.rs` (add `set_account_vat_box` to the `doris_ledger::{…}` import and `use doris_ledger::vat_box::VatBox;`):
```rust
#[tokio::test]
async fn an_accounts_box_is_listed_set_and_cleared() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let boxed = |accounts: &[doris_ledger::domain::Account], n: u16| {
        accounts.iter().find(|a| a.number.get() == n).unwrap().vat_box.map(VatBox::get)
    };
    let before = list_accounts(&pool, id, anna).await.unwrap();
    assert_eq!(boxed(&before, 2611), Some(10));

    set_account_vat_box(&pool, id, anna, 3004, Some(5)).await.unwrap();
    set_account_vat_box(&pool, id, anna, 2611, None).await.unwrap();
    let after = list_accounts(&pool, id, anna).await.unwrap();
    assert_eq!((boxed(&after, 3004), boxed(&after, 2611)), (Some(5), None));
    assert_eq!(
        events_of(&pool, "accounts-").await,
        ["ChartSeeded", "AccountVatBoxSet", "AccountVatBoxSet"]
    );

    let refused = set_account_vat_box(&pool, id, anna, 2650, Some(48)).await;
    assert!(matches!(refused, Err(Error::Domain(DomainError::InvalidVatBox))));
    let refused = set_account_vat_box(&pool, id, anna, 2611, Some(49)).await;
    assert!(matches!(refused, Err(Error::Domain(DomainError::InvalidVatBox))));
    let stranger = set_account_vat_box(&pool, id, Uuid::new_v4(), 3004, None).await;
    assert!(matches!(stranger, Err(Error::NotFound)));
}
```
(Import `Error` from `doris_ledger` if the file doesn't already.)

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p doris-ledger --test store an_accounts_box`
Expected: FAIL to compile (`set_account_vat_box` not in `doris_ledger`).

- [ ] **Step 7: Implement the store side**

In `crates/ledger/src/lib.rs`:
```rust
use vat_box::VatBox;

pub async fn set_account_vat_box(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    vat_box: Option<u32>,
) -> Result<()> {
    let number = AccountNumber::parse(number)?;
    let vat_box = vat_box.map(VatBox::parse).transpose()?;
    change_chart(pool, company_id, actor, |chart| {
        domain::set_account_vat_box(chart, number, vat_box)
    })
    .await
}

/// The company's chart as its events say, or the built-in BAS selection
/// when it has none yet. Writes nothing.
pub(crate) async fn chart_in(conn: &mut SqliteConnection, company_id: Uuid) -> Result<Chart> {
    let recorded = doris_eventstore::load(conn, &accounts_stream(company_id)).await?;
    if recorded.is_empty() {
        return Ok(Chart::from_events(&[domain::seed_chart()]));
    }
    let events = recorded
        .iter()
        .map(|e| e.decode::<ChartEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Chart::from_events(&events))
}
```
In `crates/ledger/src/queries.rs`, replace the body of `list_accounts` after the membership check:
```rust
    doris_company::get_company(pool, company_id, user_id).await?;
    let mut conn = pool.acquire().await?;
    // The chart's events, not the projection: they also carry each
    // account's momsruta.
    Ok(crate::chart_in(&mut conn, company_id)
        .await?
        .accounts()
        .cloned()
        .collect())
```
Update its doc comment ("The company's chart, by number, read from its events…") and drop now-unused imports.

- [ ] **Step 8: Run the ledger tests**

Run: `cargo test -p doris-ledger`
Expected: PASS (including `the_chart_projection_rebuilds_from_the_events`).

- [ ] **Step 9: Commit**

```bash
git add crates/ledger
git commit -m "Give each account a momsruta, BAS's by default"
```

---

### Task 3: `vat_box_totals_in`

**Files:**
- Modify: `crates/ledger/src/queries.rs`, `crates/ledger/src/lib.rs` (re-export)
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: Task 2 (`chart_in`), `VoucherRef`.
- Produces:
```rust
pub struct VatAccountTotal { pub number: u16, pub name: String, pub vat_box: VatBox, pub saldo: i64 }
pub async fn vat_box_totals_in(conn: &mut SqliteConnection, company_id: Uuid, from: Date, to: Date, exclude: &[VoucherRef]) -> Result<Vec<VatAccountTotal>>
```
Sorted by account number; only accounts with a box and lines in `from..=to`; `saldo` = debit − credit in öre; vouchers in `exclude` and every voucher that corrects one of them are left out. No membership check (the caller's transaction has done it).

- [ ] **Step 1: Write the failing test**

Append to `crates/ledger/tests/store.rs` (import `vat_box_totals_in`, `VatAccountTotal`, `VoucherRef`, `correct_voucher`, `record_voucher` as needed):
```rust
fn voucher(date: &str, text: &str, lines: &[(u32, i64, i64)]) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: text.into(),
        lines: lines.iter().map(|&(a, dr, cr)| VoucherLine::new(a, dr, cr).unwrap()).collect(),
    }
}

#[tokio::test]
async fn vat_totals_sum_a_periods_boxed_accounts_and_leave_out_what_is_excluded() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    // A sale with 25 % VAT, a purchase with input VAT, and a sale outside the period.
    record_voucher(&pool, id, anna, voucher("2025-02-10", "Sale", &[(1930, 12_500, 0), (3001, 0, 10_000), (2611, 0, 2_500)]), today).await.unwrap();
    record_voucher(&pool, id, anna, voucher("2025-03-31", "Buy", &[(4010, 800, 0), (2640, 200, 0), (1930, 0, 1_000)]), today).await.unwrap();
    record_voucher(&pool, id, anna, voucher("2025-04-01", "Later", &[(1930, 125, 0), (3001, 0, 100), (2611, 0, 25)]), today).await.unwrap();
    // A settlement to exclude, and its correction.
    let settled = record_voucher(&pool, id, anna, voucher("2025-03-31", "Momsavräkning", &[(2611, 2_500, 0), (2640, 0, 200), (2650, 0, 2_300)]), today).await.unwrap();
    correct_voucher(&pool, id, anna, settled.fiscal_year_start, settled.number, d("2025-03-31"), today).await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let totals = vat_box_totals_in(&mut conn, id, d("2025-01-01"), d("2025-03-31"), &[settled]).await.unwrap();
    let got: Vec<(u16, u8, i64)> = totals.iter().map(|t| (t.number, t.vat_box.get(), t.saldo)).collect();
    assert_eq!(got, [(2611, 10, -2_500), (2640, 48, 200), (3001, 5, -10_000)]);
    assert_eq!(totals[0].name, "Utgående moms på försäljning inom Sverige, 25 %");

    // Without the exclusion the settlement and its correction cancel each
    // other on 2611 and 2640; 2650 has no box.
    let all = vat_box_totals_in(&mut conn, id, d("2025-01-01"), d("2025-03-31"), &[]).await.unwrap();
    assert_eq!(all.len(), 3);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-ledger --test store vat_totals`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

In `crates/ledger/src/queries.rs`:
```rust
/// One account with a momsruta and its saldo over a period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatAccountTotal {
    pub number: u16,
    pub name: String,
    pub vat_box: crate::vat_box::VatBox,
    /// Debit − credit, öre.
    pub saldo: i64,
}

/// Each account with a box and lines dated `from..=to`: its box and saldo.
/// Vouchers in `exclude`, and every voucher that corrects one of them,
/// are left out. For `doris-vat`, in its own transaction, which has
/// checked membership.
// ponytail: sums the period's lines in Rust; a GROUP BY when periods get big.
pub async fn vat_box_totals_in(
    conn: &mut sqlx::SqliteConnection,
    company_id: Uuid,
    from: Date,
    to: Date,
    exclude: &[crate::VoucherRef],
) -> Result<Vec<VatAccountTotal>> {
    let chart = crate::chart_in(conn, company_id).await?;
    let rows: Vec<(String, u32, Option<u32>, u16, i64)> = sqlx::query_as(
        "SELECT v.fiscal_year_start, v.number, v.corrects, l.account, l.debit - l.credit
         FROM voucher_lines l
         JOIN vouchers v ON v.company_id = l.company_id
             AND v.fiscal_year_start = l.fiscal_year_start AND v.number = l.number
         WHERE l.company_id = ? AND v.date BETWEEN ? AND ?",
    )
    .bind(company_id.to_string())
    .bind(from.to_string())
    .bind(to.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let excluded = |start: &str, number: u32| {
        exclude
            .iter()
            .any(|v| v.number == number && v.fiscal_year_start.to_string() == start)
    };
    let mut sums = std::collections::BTreeMap::<u16, i64>::new();
    for (start, number, corrects, account, saldo) in rows {
        if excluded(&start, number) || corrects.is_some_and(|c| excluded(&start, c)) {
            continue;
        }
        *sums.entry(account).or_default() += saldo;
    }
    Ok(sums
        .into_iter()
        .filter_map(|(number, saldo)| {
            let account = chart.get(AccountNumber::parse(number.into()).ok()?)?;
            Some(VatAccountTotal {
                number,
                name: account.name.as_str().to_owned(),
                vat_box: account.vat_box?,
                saldo,
            })
        })
        .collect())
}
```
In `crates/ledger/src/lib.rs`, add `vat_box_totals_in, VatAccountTotal` to `pub use queries::{…}`.

- [ ] **Step 4: Run the test**

Run: `cargo test -p doris-ledger --test store vat_totals`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger
git commit -m "Sum a period's accounts per momsruta, leaving out given vouchers"
```

---

### Task 4: `SetAccountVatBox` och `Account.vat_box` i API:t

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`
- Modify: `crates/server/src/ledger.rs` (handler, `account_message`, `InvalidVatBox` mapping)
- Test: `crates/server/tests/ledger.rs`

**Interfaces:**
- Consumes: Task 2.
- Produces: `rpc SetAccountVatBox(SetAccountVatBoxRequest) returns (SetAccountVatBoxResponse)`; `Account.vat_box` (field 4, `uint32`, 0 = none); code `invalid_vat_box`.

- [ ] **Step 1: Write the failing test**

Append to `crates/server/tests/ledger.rs` (it already has `company`, `code_of`, `authed`, `device` and `pb` = ledger):
```rust
async fn vat_boxes(api: &mut Ledger, session: &str, company_id: &str) -> std::collections::HashMap<u32, u32> {
    api.list_accounts(authed(pb::ListAccountsRequest { company_id: company_id.into() }, session))
        .await
        .unwrap()
        .into_inner()
        .accounts
        .into_iter()
        .map(|a| (a.number, a.vat_box))
        .collect()
}

fn set_box(company_id: &str, number: u32, vat_box: u32) -> pb::SetAccountVatBoxRequest {
    pb::SetAccountVatBoxRequest { company_id: company_id.into(), number, vat_box }
}

#[tokio::test]
async fn an_accounts_momsruta_is_listed_and_changed() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let before = vat_boxes(&mut api, &anna, &id).await;
    assert_eq!((before[&2611], before[&1930]), (10, 0));

    api.set_account_vat_box(authed(set_box(&id, 3004, 5), &anna)).await.unwrap();
    api.set_account_vat_box(authed(set_box(&id, 2611, 0), &anna)).await.unwrap();
    let after = vat_boxes(&mut api, &anna, &id).await;
    assert_eq!((after[&3004], after[&2611]), (5, 0));

    let err = api.set_account_vat_box(authed(set_box(&id, 2611, 49), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_vat_box".into()));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-server --test ledger momsruta`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`proto/doris/ledger/v1/ledger.proto`: after `rpc SetAccountActive…`:
```proto
  // An account's box on the momsdeklaration; 0 takes it away.
  rpc SetAccountVatBox(SetAccountVatBoxRequest) returns (SetAccountVatBoxResponse);
```
In `message Account`: `uint32 vat_box = 4; // box on the momsdeklaration, 0: none`. And:
```proto
message SetAccountVatBoxRequest {
  string company_id = 1;
  uint32 number = 2;
  uint32 vat_box = 3; // 0: none
}

message SetAccountVatBoxResponse {}
```
In `crates/server/src/ledger.rs`:
- wherever `pb::Account { number, name, active }` is built, add `vat_box: a.vat_box.map_or(0, |b| b.get().into()),`.
- new handler next to `set_account_active`, same shape:
```rust
    async fn set_account_vat_box(
        &self,
        request: Request<pb::SetAccountVatBoxRequest>,
    ) -> Result<Response<pb::SetAccountVatBoxResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let vat_box = (req.vat_box != 0).then_some(req.vat_box);
        doris_ledger::set_account_vat_box(&self.pool, company, user, req.number, vat_box)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetAccountVatBoxResponse {}))
    }
```
(This is the shape of `set_account_active` in the same file: it uses `self.caller`.)
- in `domain_status`: `InvalidVatBox => Status::invalid_argument("invalid_vat_box"),`.

In `crates/web/src/pages/accounts.rs`, the destructuring `let lpb::Account { number, name: current, active } = account;` must become `let lpb::Account { number, name: current, active, .. } = account;` so the web crate still compiles (the column comes in Task 13).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-server --test ledger && cargo build -p doris-web`
Expected: PASS / builds.

- [ ] **Step 5: Commit**

```bash
git add proto crates/server crates/web/src/pages/accounts.rs
git commit -m "Serve and change an account's momsruta"
```

---

### Task 5: `doris-vat`: perioder, etiketter och deklarationsdag

**Files:**
- Create: `crates/vat/Cargo.toml`, `crates/vat/src/lib.rs`, `crates/vat/src/period.rs`
- Modify: `Cargo.toml` (workspace members + `doris-vat` dependency)
- Test: `crates/vat/tests/period.rs`

**Interfaces:**
- Produces: `doris_vat::period::{VatPeriodKind, VatPeriod, periods, due_date}`; `VatPeriodKind::{Monthly, Quarterly (default), Yearly, NotRegistered}` (serde snake_case); `VatPeriod { start: Date, end: Date }` with `code() -> String` ("ÅÅÅÅMM") and `label() -> String` ("juli–september 2026"); `periods(FiscalYear, VatPeriodKind) -> Vec<VatPeriod>`; `due_date(VatPeriod, VatPeriodKind) -> Option<Date>`.

- [ ] **Step 1: Create the crate skeleton**

`Cargo.toml` (workspace): add `"crates/vat"` to `members` and `doris-vat = { path = "crates/vat" }` to `[workspace.dependencies]`.

`crates/vat/Cargo.toml`:
```toml
[package]
name = "doris-vat"
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
sha2.workspace = true
sqlx.workspace = true
thiserror.workspace = true
uuid.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio.workspace = true
```
`crates/vat/src/lib.rs`:
```rust
//! The momsdeklaration of a company: the redovisningsperiod per
//! räkenskapsår, each period's boxes from the ledger, the file for
//! Skatteverket, and the settlement voucher booked when a period is marked
//! submitted.

pub mod period;
```

- [ ] **Step 2: Write the failing test**

`crates/vat/tests/period.rs`:
```rust
use doris_company::domain::FiscalYear;
use doris_vat::period::{VatPeriod, VatPeriodKind::*, due_date, periods};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn year(start: &str, end: &str) -> FiscalYear {
    FiscalYear { start: d(start), end: d(end) }
}

fn spans(list: &[VatPeriod]) -> Vec<(String, String)> {
    list.iter().map(|p| (p.start.to_string(), p.end.to_string())).collect()
}

#[test]
fn a_calendar_year_has_twelve_months_four_quarters_or_one_year() {
    let y = year("2026-01-01", "2026-12-31");
    let months = periods(y, Monthly);
    assert_eq!(months.len(), 12);
    assert_eq!(spans(&months[1..2]), [("2026-02-01".into(), "2026-02-28".into())]);
    assert_eq!(
        spans(&periods(y, Quarterly)),
        [
            ("2026-01-01".into(), "2026-03-31".into()),
            ("2026-04-01".into(), "2026-06-30".into()),
            ("2026-07-01".into(), "2026-09-30".into()),
            ("2026-10-01".into(), "2026-12-31".into()),
        ]
    );
    assert_eq!(spans(&periods(y, Yearly)), [("2026-01-01".into(), "2026-12-31".into())]);
    assert!(periods(y, NotRegistered).is_empty());
}

#[test]
fn quarters_follow_the_calendar_in_a_broken_year() {
    // May 2026 – April 2027: the quarters ending June, September, December
    // and March. April–June 2027 belongs to the next year.
    let quarters = periods(year("2026-05-01", "2027-04-30"), Quarterly);
    assert_eq!(
        spans(&quarters),
        [
            ("2026-04-01".into(), "2026-06-30".into()),
            ("2026-07-01".into(), "2026-09-30".into()),
            ("2026-10-01".into(), "2026-12-31".into()),
            ("2027-01-01".into(), "2027-03-31".into()),
        ]
    );
    assert_eq!(periods(year("2026-05-01", "2027-04-30"), Yearly)[0].end, d("2027-04-30"));
}

#[test]
fn periods_have_a_code_and_a_swedish_label() {
    let q3 = VatPeriod { start: d("2026-07-01"), end: d("2026-09-30") };
    assert_eq!((q3.code(), q3.label()), ("202609".into(), "juli–september 2026".into()));
    let may = VatPeriod { start: d("2026-05-01"), end: d("2026-05-31") };
    assert_eq!(may.label(), "maj 2026");
    let broken = VatPeriod { start: d("2026-05-01"), end: d("2027-04-30") };
    assert_eq!(broken.label(), "maj 2026–april 2027");
}

fn due(start: &str, end: &str, kind: doris_vat::period::VatPeriodKind) -> Option<String> {
    due_date(VatPeriod { start: d(start), end: d(end) }, kind).map(|d| d.to_string())
}

#[test]
fn the_declaration_is_due_the_12th_of_the_second_month_or_the_17th_in_january_and_august() {
    assert_eq!(due("2026-07-01", "2026-09-30", Quarterly).as_deref(), Some("2026-11-12"));
    assert_eq!(due("2026-04-01", "2026-06-30", Quarterly).as_deref(), Some("2026-08-17"));
    assert_eq!(due("2026-03-01", "2026-03-31", Monthly).as_deref(), Some("2026-05-12"));
    // 17 January 2027 is a Sunday.
    assert_eq!(due("2026-11-01", "2026-11-30", Monthly).as_deref(), Some("2027-01-18"));
    // 12 April 2026 is a Sunday.
    assert_eq!(due("2026-02-01", "2026-02-28", Monthly).as_deref(), Some("2026-04-13"));
    assert_eq!(due("2026-01-01", "2026-12-31", Yearly), None);
}

#[test]
fn easter_holidays_push_the_date_to_the_next_workday() {
    // Annandag påsk 2004-04-12.
    assert_eq!(due("2004-02-01", "2004-02-29", Monthly).as_deref(), Some("2004-04-13"));
    // Långfredag 2047-04-12, then a weekend and annandag påsk on the 15th.
    assert_eq!(due("2047-02-01", "2047-02-28", Monthly).as_deref(), Some("2047-04-16"));
    // Kristi himmelsfärdsdag 2067-05-12.
    assert_eq!(due("2067-03-01", "2067-03-31", Monthly).as_deref(), Some("2067-05-13"));
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p doris-vat --test period`
Expected: FAIL to compile (`periods` etc. missing).

- [ ] **Step 4: Implement**

`crates/vat/src/period.rs`:
```rust
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
    "januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september",
    "oktober", "november", "december",
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
            format!("{} {}–{} {}", name(start), start.year(), name(end), end.year())
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
            .map(|m| VatPeriod { start: m, end: m.last_of_month() })
            .collect(),
        VatPeriodKind::Quarterly => months
            .into_iter()
            .filter(|m| m.month() % 3 == 0)
            .map(|m| VatPeriod { start: add_months(m, -2), end: m.last_of_month() })
            .collect(),
        VatPeriodKind::Yearly => vec![VatPeriod { start: fiscal_year.start, end: fiscal_year.end }],
        VatPeriodKind::NotRegistered => vec![],
    }
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
    let day = if matches!(month.month(), 1 | 8) { 17 } else { 12 };
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
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-vat --test period`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/vat
git commit -m "Add doris-vat with redovisningsperioder and deklarationsdagar"
```

---

### Task 6: Rutor och fingerprint

**Files:**
- Create: `crates/vat/src/domain.rs`
- Modify: `crates/vat/src/lib.rs` (`pub mod domain;`)
- Test: `crates/vat/tests/domain.rs`

**Interfaces:**
- Consumes: `doris_ledger::{VatAccountTotal, vat_box::{VatBox, Side}}`.
- Produces:
```rust
pub struct AccountSaldo { pub account: u16, pub vat_box: VatBox, pub saldo: i64 }   // Serialize, Deserialize, Eq
pub struct Boxes { pub amounts: Vec<(VatBox, i64)>, pub vat_due: i64 }             // kronor; Default
impl Boxes { pub fn get(&self, n: u8) -> i64 }
pub fn saldos(totals: &[VatAccountTotal]) -> Vec<AccountSaldo>
pub fn boxes(accounts: &[AccountSaldo]) -> Boxes
pub fn fingerprint(period_end: Date, accounts: &[AccountSaldo]) -> String
pub fn booked_vat(accounts: &[AccountSaldo]) -> i64   // öre, box 49 before rounding
```

- [ ] **Step 1: Write the failing test**

`crates/vat/tests/domain.rs`:
```rust
use doris_ledger::vat_box::VatBox;
use doris_vat::domain::{AccountSaldo, booked_vat, boxes, fingerprint};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(account: u16, vat_box: u32, saldo: i64) -> AccountSaldo {
    AccountSaldo { account, vat_box: VatBox::parse(vat_box).unwrap(), saldo }
}

#[test]
fn boxes_count_sales_as_credits_and_purchases_as_debits_in_whole_kronor() {
    let accounts = [
        a(2611, 10, -95_000_37),   // output VAT 25 %
        a(2640, 48, 24_610_99),    // input VAT
        a(3001, 5, -380_000_00),
        a(3002, 5, -32_300_40),
        a(4535, 21, 4_800_00),     // EU service purchase
        a(2614, 30, -1_200_00),    // its output VAT
        a(2645, 48, 1_200_00),     // and its input VAT
    ];
    let b = boxes(&accounts);
    assert_eq!(
        [b.get(5), b.get(10), b.get(21), b.get(30), b.get(48)],
        [412_300, 95_000, 4_800, 1_200, 25_810]
    );
    // 49 from the rounded boxes: 95 000 + 1 200 − 25 810.
    assert_eq!(b.vat_due, 70_390);
    assert_eq!(b.get(6), 0);
    assert!(b.amounts.iter().all(|(_, kr)| *kr != 0));
    // Exactly booked: 95 000,37 + 1 200,00 − 25 810,99 = 70 389,38.
    assert_eq!(booked_vat(&accounts), 70_389_38);
}

#[test]
fn ore_are_struck_off_toward_zero_also_for_negative_boxes() {
    // More credited than sold: a debit saldo on sales and output VAT.
    let b = boxes(&[a(3001, 5, 1_000_99), a(2611, 10, 250_75)]);
    assert_eq!((b.get(5), b.get(10), b.vat_due), (-1_000, -250, -250));
}

#[test]
fn the_fingerprint_follows_the_accounts_and_their_boxes() {
    let one = fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]);
    assert_eq!(one.len(), 64);
    assert_eq!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -101)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 11, -100)]));
    assert_ne!(one, fingerprint(d("2026-06-30"), &[a(2611, 10, -100)]));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-vat --test domain`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`crates/vat/src/domain.rs`:
```rust
//! Pure VAT rules: boxes, the settlement voucher, status and decisions.

use doris_ledger::VatAccountTotal;
use doris_ledger::vat_box::{Side, VatBox};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// An account's saldo (debit − credit, öre) over a period, with its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSaldo {
    pub account: u16,
    pub vat_box: VatBox,
    pub saldo: i64,
}

/// What is declared: whole kronor per box that is not zero, and box 49.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Boxes {
    pub amounts: Vec<(VatBox, i64)>,
    pub vat_due: i64,
}

impl Boxes {
    /// Kronor in box `n`, 0 when empty.
    pub fn get(&self, n: u8) -> i64 {
        self.amounts
            .iter()
            .find(|(b, _)| b.get() == n)
            .map_or(0, |(_, kr)| *kr)
    }
}

const OUTPUT_VAT: [u8; 9] = [10, 11, 12, 30, 31, 32, 60, 61, 62];

pub fn saldos(totals: &[VatAccountTotal]) -> Vec<AccountSaldo> {
    totals
        .iter()
        .map(|t| AccountSaldo { account: t.number, vat_box: t.vat_box, saldo: t.saldo })
        .collect()
}

/// An account's saldo as its box counts it: positive for sales and
/// output VAT on the credit side, purchases and input VAT on the debit side.
pub fn signed(a: &AccountSaldo) -> i64 {
    match a.vat_box.side() {
        Side::Debit => a.saldo,
        Side::Credit => -a.saldo,
    }
}

/// Each box summed in öre, then the öre struck off toward zero, as
/// Skatteverket asks. Box 49 comes from the rounded boxes, as Skatteverket
/// checks it.
pub fn boxes(accounts: &[AccountSaldo]) -> Boxes {
    let mut ore = BTreeMap::<VatBox, i64>::new();
    for a in accounts {
        *ore.entry(a.vat_box).or_default() += signed(a);
    }
    let amounts: Vec<(VatBox, i64)> = ore
        .into_iter()
        .map(|(b, o)| (b, o / 100))
        .filter(|(_, kr)| *kr != 0)
        .collect();
    let mut boxes = Boxes { amounts, vat_due: 0 };
    boxes.vat_due = OUTPUT_VAT.iter().map(|&n| boxes.get(n)).sum::<i64>() - boxes.get(48);
    boxes
}

/// Box 49 in öre before rounding: the VAT the books hold.
pub fn booked_vat(accounts: &[AccountSaldo]) -> i64 {
    accounts
        .iter()
        .filter(|a| a.vat_box.is_vat())
        .map(|a| -a.saldo)
        .sum()
}

/// Hex SHA-256 of what a submission would record, so marking a period
/// submitted is refused when the books changed after it was shown.
pub fn fingerprint(period_end: Date, accounts: &[AccountSaldo]) -> String {
    let json = serde_json::to_vec(&(period_end, accounts)).expect("plain data serializes");
    Sha256::digest(json).iter().map(|b| format!("{b:02x}")).collect()
}
```
In `crates/vat/src/lib.rs`: `pub mod domain;`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-vat --test domain`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vat
git commit -m "Compute the momsdeklaration's boxes from account saldos"
```

---

### Task 7: Avräkningen, händelser, tillstånd, status och beslut

**Files:**
- Modify: `crates/vat/src/domain.rs`
- Test: `crates/vat/tests/domain.rs`

**Interfaces:**
- Consumes: Tasks 5–6, `doris_ledger::{VoucherRef, domain::VoucherLine}`.
- Produces:
```rust
pub enum DomainError { InvalidVatPeriod, VatPeriodNotEnded, VatPeriodLocked, VatReturnOutdated, VatReturnUnchanged, VatNotRegistered }
pub struct Submission { pub period_end: Date, pub accounts: Vec<AccountSaldo>, pub boxes: Boxes, pub settled: Vec<(u16, i64)>, pub settled_vat_due: i64, pub voucher: Option<VoucherRef> }
pub enum VatEvent { VatPeriodSet { fiscal_year_start: Date, kind: VatPeriodKind }, VatReturnSubmitted(Submission) }   // #[serde(tag = "type")]
pub struct Vat { pub kinds: BTreeMap<Date, VatPeriodKind>, pub submissions: Vec<Submission> }
impl Vat { from_events(&[VatEvent]) -> Self; apply(&mut self, &VatEvent); kind(&self, Date) -> VatPeriodKind; vouchers(&self) -> Vec<VoucherRef>; submissions_for(&self, Date) -> Vec<&Submission> }
pub enum VatStatus { InProgress, ToSubmit, Submitted, Changed }
pub struct Settlement { pub lines: Vec<VoucherLine>, pub settled: Vec<(u16, i64)>, pub vat_due: i64 }
pub fn settlement(accounts: &[AccountSaldo], boxes: &Boxes, earlier: &[&Submission]) -> Settlement
pub fn standing<'a>(vat: &'a Vat, period_end: Date, corrected: &HashSet<VoucherRef>) -> Vec<&'a Submission>
pub fn status(vat: &Vat, period: VatPeriod, today: Date, accounts: &[AccountSaldo], corrected: &HashSet<VoucherRef>) -> VatStatus
pub fn is_locked(vat: &Vat, fiscal_year: FiscalYear) -> bool
pub fn set_vat_period(vat: &Vat, fiscal_year: FiscalYear, kind: VatPeriodKind) -> Result<Vec<VatEvent>, DomainError>
pub struct Prepared { pub lines: Vec<VoucherLine>, pub submission: Submission }
pub fn submit(vat: &Vat, period: VatPeriod, kind: VatPeriodKind, today: Date, accounts: Vec<AccountSaldo>, fingerprint: &str, corrected: &HashSet<VoucherRef>) -> Result<Prepared, DomainError>
```

- [ ] **Step 1: Write the failing tests**

Append to `crates/vat/tests/domain.rs`:
```rust
use doris_company::domain::FiscalYear;
use doris_ledger::VoucherRef;
use doris_ledger::domain::VoucherLine;
use doris_vat::domain::{DomainError, Submission, Vat, VatEvent, VatStatus, set_vat_period, settlement, status, submit};
use doris_vat::period::{VatPeriod, VatPeriodKind};
use std::collections::HashSet;

fn lines(list: &[VoucherLine]) -> Vec<(u16, i64, i64)> {
    list.iter().map(|l| (l.account.get(), l.debit, l.credit)).collect()
}

fn q3() -> VatPeriod {
    VatPeriod { start: d("2026-07-01"), end: d("2026-09-30") }
}

fn year26() -> FiscalYear {
    FiscalYear { start: d("2026-01-01"), end: d("2026-12-31") }
}

fn ver(n: u32) -> VoucherRef {
    VoucherRef { fiscal_year_start: d("2026-01-01"), number: n }
}

fn sales() -> Vec<AccountSaldo> {
    vec![a(2611, 10, -1_000_50), a(2640, 48, 400_30), a(3001, 5, -4_002_00)]
}

/// Submits `accounts` for Q3 on 2026-10-06 and records it with voucher `n`.
fn submitted(vat: &mut Vat, accounts: Vec<AccountSaldo>, n: u32) -> Vec<VoucherLine> {
    let print = fingerprint(q3().end, &accounts);
    let prepared = submit(vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), accounts, &print, &HashSet::new()).unwrap();
    let mut submission = prepared.submission;
    submission.voucher = Some(ver(n));
    vat.apply(&VatEvent::VatReturnSubmitted(submission));
    prepared.lines
}

#[test]
fn the_settlement_moves_the_vat_to_2650_and_the_ore_to_3740() {
    let accounts = sales();
    let s = settlement(&accounts, &boxes(&accounts), &[]);
    // 2611 debit 1 000,50; 2640 credit 400,30; 2650 credit box 49 = 600 kr;
    // 3740 takes the 0,20 left.
    assert_eq!(lines(&s.lines), [(2611, 1_000_50, 0), (2640, 0, 400_30), (2650, 0, 600_00), (3740, 0, 20)]);
    assert_eq!(s.settled, [(2611, -1_000_50), (2640, 400_30)]);
    assert_eq!(s.vat_due, 600_00);
}

#[test]
fn vat_to_get_back_is_a_debit_on_2650() {
    let accounts = [a(2611, 10, -100_00), a(2640, 48, 350_40)];
    let s = settlement(&accounts, &boxes(&accounts), &[]);
    assert_eq!(lines(&s.lines), [(2611, 100_00, 0), (2640, 0, 350_40), (2650, 250_00, 0), (3740, 40, 0)]);
}

#[test]
fn a_second_submission_settles_only_the_difference() {
    let mut vat = Vat::default();
    submitted(&mut vat, sales(), 7);
    // A late sale: 100 kr more output VAT.
    let more = vec![a(2611, 10, -1_100_50), a(2640, 48, 400_30), a(3001, 5, -4_402_00)];
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &more, &HashSet::new()), VatStatus::Changed);
    let lines_now = submitted(&mut vat, more, 9);
    assert_eq!(lines(&lines_now), [(2611, 100_00, 0), (2650, 0, 100_00)]);
}

#[test]
fn an_account_that_lost_its_vat_box_is_settled_back() {
    let mut vat = Vat::default();
    let with_reverse_charge = vec![a(2614, 30, -250_00), a(2645, 48, 250_00), a(4535, 21, 1_000_00)];
    submitted(&mut vat, with_reverse_charge, 7);
    // 2614 no longer has a box: it is no longer in the period's accounts.
    let now = vec![a(2645, 48, 250_00), a(4535, 21, 1_000_00)];
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &now, &HashSet::new()), VatStatus::Changed);
    let back = submitted(&mut vat, now, 9);
    // What was moved off 2614 goes back; 49 drops by 250 kr.
    assert_eq!(lines(&back), [(2614, 0, 250_00), (2650, 250_00, 0)]);
}

#[test]
fn a_corrected_settlement_counts_as_not_booked() {
    let mut vat = Vat::default();
    submitted(&mut vat, sales(), 7);
    let corrected = HashSet::from([ver(7)]);
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &sales(), &corrected), VatStatus::Changed);
    let print = fingerprint(q3().end, &sales());
    let again = submit(&vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), sales(), &print, &corrected).unwrap();
    assert_eq!(again.lines.len(), 4, "the whole settlement again");
}

#[test]
fn statuses_follow_the_date_and_the_latest_submission() {
    let mut vat = Vat::default();
    let none = HashSet::new();
    assert_eq!(status(&vat, q3(), d("2026-09-30"), &sales(), &none), VatStatus::InProgress);
    assert_eq!(status(&vat, q3(), d("2026-10-01"), &sales(), &none), VatStatus::ToSubmit);
    submitted(&mut vat, sales(), 7);
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &sales(), &none), VatStatus::Submitted);
}

#[test]
fn submitting_is_refused_when_not_due_outdated_unchanged_or_not_registered() {
    let mut vat = Vat::default();
    let print = fingerprint(q3().end, &sales());
    let none = HashSet::new();
    let try_on = |vat: &Vat, kind, today: &str, print: &str| {
        submit(vat, q3(), kind, d(today), sales(), print, &none).map(|_| ())
    };
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-09-30", &print), Err(DomainError::VatPeriodNotEnded));
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-10-06", "stale"), Err(DomainError::VatReturnOutdated));
    assert_eq!(try_on(&vat, VatPeriodKind::NotRegistered, "2026-10-06", &print), Err(DomainError::VatNotRegistered));
    submitted(&mut vat, sales(), 7);
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-10-06", &print), Err(DomainError::VatReturnUnchanged));
}

#[test]
fn nothing_to_settle_gives_no_lines() {
    let vat = Vat::default();
    let accounts = vec![a(3004, 42, -500_00)];
    let print = fingerprint(q3().end, &accounts);
    let prepared = submit(&vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), accounts, &print, &HashSet::new()).unwrap();
    assert!(prepared.lines.is_empty());
    assert_eq!(prepared.submission.boxes.get(42), 500);
}

#[test]
fn the_period_kind_is_quarterly_until_set_and_locked_once_the_year_has_a_submission() {
    let mut vat = Vat::default();
    assert_eq!(vat.kind(d("2026-01-01")), VatPeriodKind::Quarterly);
    assert_eq!(set_vat_period(&vat, year26(), VatPeriodKind::Quarterly).unwrap(), []);
    let events = set_vat_period(&vat, year26(), VatPeriodKind::Monthly).unwrap();
    assert_eq!(events, [VatEvent::VatPeriodSet { fiscal_year_start: d("2026-01-01"), kind: VatPeriodKind::Monthly }]);
    vat.apply(&events[0]);
    assert_eq!(vat.kind(d("2026-01-01")), VatPeriodKind::Monthly);
    vat.apply(&VatEvent::VatReturnSubmitted(Submission {
        period_end: d("2026-01-31"),
        accounts: vec![],
        boxes: Default::default(),
        settled: vec![],
        settled_vat_due: 0,
        voucher: None,
    }));
    assert_eq!(set_vat_period(&vat, year26(), VatPeriodKind::Yearly), Err(DomainError::VatPeriodLocked));
    // Another year is not locked.
    let next = FiscalYear { start: d("2027-01-01"), end: d("2027-12-31") };
    assert!(set_vat_period(&vat, next, VatPeriodKind::Yearly).is_ok());
}

#[test]
fn events_round_trip_as_tagged_json() {
    let event = VatEvent::VatPeriodSet { fiscal_year_start: d("2026-01-01"), kind: VatPeriodKind::NotRegistered };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "VatPeriodSet");
    assert_eq!(json["kind"], "not_registered");
    assert_eq!(serde_json::from_value::<VatEvent>(json).unwrap(), event);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p doris-vat --test domain`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Append to `crates/vat/src/domain.rs` (and extend its `use` lines with `crate::period::{VatPeriod, VatPeriodKind}`, `doris_company::domain::FiscalYear`, `doris_ledger::VoucherRef`, `doris_ledger::domain::VoucherLine`, `std::collections::HashSet`):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("no such redovisningsperiod")]
    InvalidVatPeriod,
    #[error("the period has not ended")]
    VatPeriodNotEnded,
    #[error("a period of the year is submitted")]
    VatPeriodLocked,
    #[error("the books changed since the declaration was shown")]
    VatReturnOutdated,
    #[error("the period is submitted and unchanged")]
    VatReturnUnchanged,
    #[error("the company is not VAT registered that year")]
    VatNotRegistered,
}

/// What was declared for a period, and what its settlement voucher booked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub period_end: Date,
    pub accounts: Vec<AccountSaldo>,
    pub boxes: Boxes,
    /// Per VAT account, the saldo the voucher moved to 2650 (öre).
    pub settled: Vec<(u16, i64)>,
    /// What the voucher booked on 2650 (öre); positive: to pay.
    pub settled_vat_due: i64,
    /// None when there was nothing to book.
    pub voucher: Option<VoucherRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VatEvent {
    /// How often the company declares VAT in the räkenskapsår.
    VatPeriodSet {
        fiscal_year_start: Date,
        kind: VatPeriodKind,
    },
    /// As the user confirms after uploading the file to Skatteverket.
    VatReturnSubmitted(Submission),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vat {
    pub kinds: BTreeMap<Date, VatPeriodKind>,
    /// In order; the last one per period is what Skatteverket has.
    pub submissions: Vec<Submission>,
}

impl Vat {
    pub fn from_events(events: &[VatEvent]) -> Self {
        let mut vat = Self::default();
        for event in events {
            vat.apply(event);
        }
        vat
    }

    pub fn apply(&mut self, event: &VatEvent) {
        match event {
            VatEvent::VatPeriodSet { fiscal_year_start, kind } => {
                self.kinds.insert(*fiscal_year_start, *kind);
            }
            VatEvent::VatReturnSubmitted(submission) => self.submissions.push(submission.clone()),
        }
    }

    /// Quarterly until set.
    pub fn kind(&self, fiscal_year_start: Date) -> VatPeriodKind {
        self.kinds.get(&fiscal_year_start).copied().unwrap_or_default()
    }

    /// Every settlement voucher Doris has booked.
    pub fn vouchers(&self) -> Vec<VoucherRef> {
        self.submissions.iter().filter_map(|s| s.voucher).collect()
    }

    pub fn submissions_for(&self, period_end: Date) -> Vec<&Submission> {
        self.submissions.iter().filter(|s| s.period_end == period_end).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VatStatus {
    InProgress,
    ToSubmit,
    Submitted,
    Changed,
}

/// The settlement voucher's lines and what they settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub lines: Vec<VoucherLine>,
    pub settled: Vec<(u16, i64)>,
    pub vat_due: i64,
}

/// `amount` öre on `account`: a debit when positive, a credit when negative.
fn line(account: u16, amount: i64) -> VoucherLine {
    let (debit, credit) = if amount > 0 { (amount, 0) } else { (0, -amount) };
    VoucherLine::new(account.into(), debit, credit).expect("settlement accounts are valid")
}

/// Moves each VAT account's saldo not yet settled by `earlier` to 2650,
/// books box 49 less what `earlier` booked there, and puts the öre left
/// on 3740 (öres- och kronutjämning). No lines when nothing is left.
pub fn settlement(accounts: &[AccountSaldo], boxes: &Boxes, earlier: &[&Submission]) -> Settlement {
    let mut open = BTreeMap::<u16, i64>::new();
    for a in accounts.iter().filter(|a| a.vat_box.is_vat()) {
        *open.entry(a.account).or_default() += a.saldo;
    }
    for s in earlier {
        for &(account, saldo) in &s.settled {
            *open.entry(account).or_default() -= saldo;
        }
    }
    open.retain(|_, saldo| *saldo != 0);
    let vat_due = boxes.vat_due * 100 - earlier.iter().map(|s| s.settled_vat_due).sum::<i64>();
    let mut lines: Vec<VoucherLine> = open.iter().map(|(&a, &saldo)| line(a, -saldo)).collect();
    if vat_due != 0 {
        lines.push(line(2650, -vat_due));
    }
    let rest: i64 = lines.iter().map(|l| l.debit - l.credit).sum();
    if rest != 0 {
        lines.push(line(3740, -rest));
    }
    Settlement { lines, settled: open.into_iter().collect(), vat_due }
}

/// The period's submissions whose voucher has not been corrected: what
/// the books still hold as settled.
pub fn standing<'a>(vat: &'a Vat, period_end: Date, corrected: &HashSet<VoucherRef>) -> Vec<&'a Submission> {
    vat.submissions_for(period_end)
        .into_iter()
        .filter(|s| !s.voucher.is_some_and(|v| corrected.contains(&v)))
        .collect()
}

/// Submitted while the latest submission declared these very accounts and
/// nothing is left to settle; Changed otherwise.
pub fn status(
    vat: &Vat,
    period: VatPeriod,
    today: Date,
    accounts: &[AccountSaldo],
    corrected: &HashSet<VoucherRef>,
) -> VatStatus {
    if period.end >= today {
        return VatStatus::InProgress;
    }
    let Some(latest) = vat.submissions_for(period.end).last().copied() else {
        return VatStatus::ToSubmit;
    };
    let left = settlement(accounts, &boxes(accounts), &standing(vat, period.end, corrected));
    if latest.accounts == accounts && left.lines.is_empty() {
        VatStatus::Submitted
    } else {
        VatStatus::Changed
    }
}

/// A period of `fiscal_year` has been submitted, so its kind stays.
pub fn is_locked(vat: &Vat, fiscal_year: FiscalYear) -> bool {
    vat.submissions
        .iter()
        .any(|s| fiscal_year.start <= s.period_end && s.period_end <= fiscal_year.end)
}

/// Setting the kind it already has yields no events. Once a period of the
/// year is submitted, its kind stays.
pub fn set_vat_period(vat: &Vat, fiscal_year: FiscalYear, kind: VatPeriodKind) -> Result<Vec<VatEvent>, DomainError> {
    if vat.kind(fiscal_year.start) == kind {
        return Ok(vec![]);
    }
    if is_locked(vat, fiscal_year) {
        return Err(DomainError::VatPeriodLocked);
    }
    Ok(vec![VatEvent::VatPeriodSet { fiscal_year_start: fiscal_year.start, kind }])
}

/// The settlement to book and the submission to record once it has its
/// voucher number.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub lines: Vec<VoucherLine>,
    pub submission: Submission,
}

pub fn submit(
    vat: &Vat,
    period: VatPeriod,
    kind: VatPeriodKind,
    today: Date,
    accounts: Vec<AccountSaldo>,
    fingerprint_seen: &str,
    corrected: &HashSet<VoucherRef>,
) -> Result<Prepared, DomainError> {
    if kind == VatPeriodKind::NotRegistered {
        return Err(DomainError::VatNotRegistered);
    }
    if period.end >= today {
        return Err(DomainError::VatPeriodNotEnded);
    }
    if fingerprint_seen != fingerprint(period.end, &accounts) {
        return Err(DomainError::VatReturnOutdated);
    }
    if status(vat, period, today, &accounts, corrected) == VatStatus::Submitted {
        return Err(DomainError::VatReturnUnchanged);
    }
    let boxes = boxes(&accounts);
    let Settlement { lines, settled, vat_due } = settlement(&accounts, &boxes, &standing(vat, period.end, corrected));
    Ok(Prepared {
        lines,
        submission: Submission {
            period_end: period.end,
            accounts,
            boxes,
            settled,
            settled_vat_due: vat_due,
            voucher: None,
        },
    })
}
```
Check against `a_second_submission_settles_only_the_difference`: earlier settled `2611: -1 000,50` and `vat_due 600_00`; now `2611: -1 100,50` → open −100,00 → debit 100,00; box 49 = 1 100 − 400 = 700 kr → 700_00 − 600_00 = 100_00 credit 2650; rest 0. ✓

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-vat`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vat
git commit -m "Decide VAT periods and submissions and the settlement voucher"
```

---

### Task 8: eSKD-filen och momsregistreringsnumret

**Files:**
- Create: `crates/vat/src/eskd.rs`
- Modify: `crates/vat/src/lib.rs` (`pub mod eskd;`)
- Test: `crates/vat/tests/eskd.rs`

**Interfaces:**
- Produces: `eskd::eskd_xml(org_nr: &OrgNr, period: VatPeriod, boxes: &Boxes) -> String`; `eskd::vat_number(org_nr: &OrgNr) -> String`; `eskd::file_name(org_nr: &OrgNr, period: VatPeriod) -> String`.

- [ ] **Step 1: Write the failing test**

`crates/vat/tests/eskd.rs`:
```rust
use doris_company::domain::OrgNr;
use doris_ledger::vat_box::VatBox;
use doris_vat::domain::Boxes;
use doris_vat::eskd::{eskd_xml, file_name, vat_number};
use doris_vat::period::VatPeriod;

fn q3() -> VatPeriod {
    VatPeriod { start: "2026-07-01".parse().unwrap(), end: "2026-09-30".parse().unwrap() }
}

fn org() -> OrgNr {
    OrgNr::parse("556016-0680").unwrap()
}

#[test]
fn the_file_lists_boxes_that_are_not_zero_in_skatteverkets_order_and_always_49() {
    let b = |n: u32| VatBox::parse(n).unwrap();
    let boxes = Boxes {
        amounts: vec![(b(5), 412_300), (b(10), 95_000), (b(21), 4_800), (b(30), 1_200), (b(48), 25_810), (b(42), -1_500)],
        vat_due: 70_390,
    };
    assert_eq!(
        eskd_xml(&org(), q3(), &boxes),
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n\
         <eSKDUpload Version=\"6.0\">\n\
         <OrgNr>556016-0680</OrgNr>\n\
         <Moms>\n\
         <Period>202609</Period>\n\
         <ForsMomsEjAnnan>412300</ForsMomsEjAnnan>\n\
         <InkopTjanstAnnatEg>4800</InkopTjanstAnnatEg>\n\
         <ForsOvrigt>-1500</ForsOvrigt>\n\
         <MomsUtgHog>95000</MomsUtgHog>\n\
         <MomsInkopUtgHog>1200</MomsInkopUtgHog>\n\
         <MomsIngAvdr>25810</MomsIngAvdr>\n\
         <MomsBetala>70390</MomsBetala>\n\
         </Moms>\n\
         </eSKDUpload>\n"
    );
    let empty = eskd_xml(&org(), q3(), &Boxes::default());
    assert!(empty.contains("<MomsBetala>0</MomsBetala>"));
    assert!(empty.is_ascii(), "ASCII is valid ISO-8859-1");
}

#[test]
fn the_vat_number_and_file_name_come_from_the_org_nr() {
    assert_eq!(vat_number(&org()), "SE556016068001");
    assert_eq!(file_name(&org(), q3()), "moms_5560160680_202609.xml");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-vat --test eskd`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`crates/vat/src/eskd.rs`:
```rust
//! The momsdeklaration as Skatteverket's eSKD file (eSKDUpload 6.0), which
//! the user uploads in Skatteverket's e-tjänst. Written as text: only tags
//! and digits, so ASCII and thus valid ISO-8859-1.

use crate::domain::Boxes;
use crate::period::VatPeriod;
use doris_company::domain::OrgNr;

/// Each box's element, in the file's order. 49 (`MomsBetala`) comes last.
const ELEMENTS: [(u8, &str); 28] = [
    (5, "ForsMomsEjAnnan"),
    (6, "UttagMoms"),
    (7, "UlagMargbesk"),
    (8, "HyrinkomstFriv"),
    (20, "InkopVaruAnnatEg"),
    (21, "InkopTjanstAnnatEg"),
    (22, "InkopTjanstUtomEg"),
    (23, "InkopVaruSverige"),
    (24, "InkopTjanstSverige"),
    (50, "MomsUlagImport"),
    (35, "ForsVaruAnnatEg"),
    (36, "ForsVaruUtomEg"),
    (37, "InkopVaruMellan3p"),
    (38, "ForsVaruMellan3p"),
    (39, "ForsTjSkskAnnatEg"),
    (40, "ForsTjOvrUtomEg"),
    (41, "ForsKopareSkskSverige"),
    (42, "ForsOvrigt"),
    (10, "MomsUtgHog"),
    (11, "MomsUtgMedel"),
    (12, "MomsUtgLag"),
    (30, "MomsInkopUtgHog"),
    (31, "MomsInkopUtgMedel"),
    (32, "MomsInkopUtgLag"),
    (60, "MomsImportUtgHog"),
    (61, "MomsImportUtgMedel"),
    (62, "MomsImportUtgLag"),
    (48, "MomsIngAvdr"),
];

pub fn eskd_xml(org_nr: &OrgNr, period: VatPeriod, boxes: &Boxes) -> String {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<eSKDUpload Version=\"6.0\">\n\
         <OrgNr>{}</OrgNr>\n<Moms>\n<Period>{}</Period>\n",
        org_nr.formatted(),
        period.code()
    );
    for (n, tag) in ELEMENTS {
        let kr = boxes.get(n);
        if kr != 0 {
            xml.push_str(&format!("<{tag}>{kr}</{tag}>\n"));
        }
    }
    xml.push_str(&format!(
        "<MomsBetala>{}</MomsBetala>\n</Moms>\n</eSKDUpload>\n",
        boxes.vat_due
    ));
    xml
}

/// SE, the ten digits, 01. For an enskild firma that is the owner's
/// personnummer, as Skatteverket assigns it.
pub fn vat_number(org_nr: &OrgNr) -> String {
    format!("SE{}01", org_nr.as_str())
}

pub fn file_name(org_nr: &OrgNr, period: VatPeriod) -> String {
    format!("moms_{}_{}.xml", org_nr.as_str(), period.code())
}
```
`crates/vat/src/lib.rs`: `pub mod eskd;`. If the test shows `OrgNr::as_str` holds the hyphen, use `org_nr.as_str().replace('-', "")` in both functions.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-vat --test eskd`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vat
git commit -m "Write the momsdeklaration as an eSKD file"
```

---

### Task 9: `doris-vat` mot databasen: period och läsningar

**Files:**
- Modify: `crates/vat/src/lib.rs`
- Test: `crates/vat/tests/store.rs`

**Interfaces:**
- Consumes: Tasks 3, 5–8; `doris_company::get_company_in`, `doris_ledger::{vat_box_totals_in, corrected_vouchers_in, VatAccountTotal}`.
- Produces:
```rust
pub enum Error { Domain(DomainError), NotFound, Ledger(doris_ledger::Error), Store(doris_eventstore::Error) }
pub async fn set_vat_period(pool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, kind: VatPeriodKind) -> Result<()>
pub struct PeriodSummary { pub period: VatPeriod, pub status: VatStatus, pub due_date: Option<Date>, pub vat_due: i64 }
pub struct VatYear { pub kind: VatPeriodKind, pub locked: bool, pub periods: Vec<PeriodSummary> }
pub async fn list_vat_returns(pool, company_id, actor, fiscal_year_start: Date, today: Date) -> Result<VatYear>
pub struct SubmissionRecord { pub submission: Submission, pub submitted_at: String, pub submitted_by: Option<Uuid>, pub corrected: bool }
pub struct VatReturnView { pub summary: PeriodSummary, pub kind: VatPeriodKind, pub org_nr: String, pub vat_number: String, pub totals: Vec<VatAccountTotal>, pub boxes: Boxes, pub booked_vat: i64, pub fingerprint: String, pub submissions: Vec<SubmissionRecord> }
pub async fn get_vat_return(pool, company_id, actor, period_end: Date, today: Date) -> Result<VatReturnView>
pub async fn export_vat_file(pool, company_id, actor, period_end: Date, today: Date) -> Result<(String, String, String)>  // name, xml, fingerprint
```

- [ ] **Step 1: Write the failing test**

`crates/vat/tests/store.rs`:
```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{RecordVoucher, VoucherLine};
use doris_vat::domain::{DomainError, VatStatus};
use doris_vat::period::VatPeriodKind;
use doris_vat::{Error, export_vat_file, get_vat_return, list_vat_returns, set_vat_period};
use jiff::civil::Date;
use sqlx::SqlitePool;
use uuid::Uuid;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

const TODAY: &str = "2026-10-06";

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// Exempel AB, first räkenskapsår 2026, faktureringsmetoden.
async fn company(pool: &SqlitePool, owner: Uuid, method: AccountingMethod) -> Uuid {
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
            fiscal_year_start: d("2026-01-01"),
            fiscal_year_end: d("2026-12-31"),
            accounting_method: method,
        },
    )
    .await
    .unwrap()
}

async fn book(pool: &SqlitePool, id: Uuid, user: Uuid, date: &str, lines: &[(u32, i64, i64)]) {
    let cmd = RecordVoucher {
        date: d(date),
        text: "Underlag".into(),
        lines: lines.iter().map(|&(a, dr, cr)| VoucherLine::new(a, dr, cr).unwrap()).collect(),
    };
    doris_ledger::record_voucher(pool, id, user, cmd, d(TODAY)).await.unwrap();
}

/// A sale of 10 000,40 + 25 % VAT and a purchase with 400,30 input VAT in Q3.
async fn q3_books(pool: &SqlitePool, id: Uuid, user: Uuid) {
    book(pool, id, user, "2026-08-10", &[(1510, 12_500_90, 0), (3001, 0, 10_000_40), (2611, 0, 2_500_50)]).await;
    book(pool, id, user, "2026-09-15", &[(4010, 1_601_20, 0), (2640, 400_30, 0), (2440, 0, 2_001_50)]).await;
}

#[tokio::test]
async fn a_year_lists_its_quarters_with_status_due_date_and_box_49() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert_eq!((year.kind, year.locked), (VatPeriodKind::Quarterly, false));
    let rows: Vec<(String, VatStatus, Option<String>, i64)> = year
        .periods
        .iter()
        .map(|p| (p.period.code(), p.status, p.due_date.map(|d| d.to_string()), p.vat_due))
        .collect();
    assert_eq!(
        rows,
        [
            ("202603".into(), VatStatus::ToSubmit, Some("2026-05-12".into()), 0),
            ("202606".into(), VatStatus::ToSubmit, Some("2026-08-17".into()), 0),
            ("202609".into(), VatStatus::ToSubmit, Some("2026-11-12".into()), 2_100),
            ("202612".into(), VatStatus::InProgress, Some("2027-02-12".into()), 0),
        ]
    );
}

#[tokio::test]
async fn a_period_shows_its_boxes_accounts_and_file() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let view = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!((view.boxes.get(5), view.boxes.get(10), view.boxes.get(48), view.boxes.vat_due), (10_000, 2_500, 400, 2_100));
    assert_eq!(view.booked_vat, 2_100_20);
    assert_eq!((view.org_nr.as_str(), view.vat_number.as_str()), ("556016-0680", "SE556016068001"));
    assert_eq!(view.totals.iter().map(|t| t.number).collect::<Vec<_>>(), [2611, 2640, 3001]);
    assert!(view.submissions.is_empty());

    let (name, xml, print) = export_vat_file(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!(name, "moms_5560160680_202609.xml");
    assert!(xml.contains("<MomsBetala>2100</MomsBetala>"));
    assert_eq!(print, view.fingerprint);

    let early = export_vat_file(&pool, id, anna, d("2026-12-31"), d(TODAY)).await;
    assert!(matches!(early, Err(Error::Domain(DomainError::VatPeriodNotEnded))));
    let no_such = get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await;
    assert!(matches!(no_such, Err(Error::Domain(DomainError::InvalidVatPeriod))));
    let stranger = get_vat_return(&pool, id, Uuid::new_v4(), d("2026-09-30"), d(TODAY)).await;
    assert!(matches!(stranger, Err(Error::NotFound)));
}

#[tokio::test]
async fn monthly_and_not_registered_change_the_periods() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    set_vat_period(&pool, id, anna, d("2026-01-01"), VatPeriodKind::Monthly).await.unwrap();
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert_eq!((year.kind, year.periods.len()), (VatPeriodKind::Monthly, 12));
    assert!(get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await.is_ok());

    set_vat_period(&pool, id, anna, d("2026-01-01"), VatPeriodKind::NotRegistered).await.unwrap();
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert!(year.periods.is_empty());
    let refused = get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await;
    assert!(matches!(refused, Err(Error::Domain(DomainError::VatNotRegistered))));

    let not_a_year = set_vat_period(&pool, id, anna, d("2026-02-01"), VatPeriodKind::Monthly).await;
    assert!(matches!(not_a_year, Err(Error::Ledger(_))));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-vat --test store`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`crates/vat/src/lib.rs` (keep the module lines):
```rust
use domain::{AccountSaldo, Boxes, DomainError, Submission, Vat, VatEvent, VatStatus};
use doris_company::domain::{Company, FiscalYear};
use doris_eventstore::{Metadata, NewEvent, RecordedEvent};
use doris_ledger::{VatAccountTotal, VoucherRef};
use jiff::civil::Date;
use period::{VatPeriod, VatPeriodKind};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

const VAT_STREAM: &str = "vat-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// The ledger refused (closed year, inactive account, no such year…).
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

fn vat_stream(company_id: Uuid) -> String {
    format!("{VAT_STREAM}{company_id}")
}

/// Membership, the company, its VAT state, the stream's version and its
/// recorded events (for who submitted what, and when).
async fn load(conn: &mut SqliteConnection, company_id: Uuid, actor: Uuid) -> Result<(Company, Vat, i64, Vec<RecordedEvent>)> {
    let company = doris_company::get_company_in(conn, company_id, actor).await?;
    let recorded = doris_eventstore::load(conn, &vat_stream(company_id)).await?;
    let events = recorded.iter().map(|e| e.decode::<VatEvent>()).collect::<Result<Vec<_>, _>>()?;
    let version = doris_eventstore::stream_version(conn, &vat_stream(company_id)).await?;
    Ok((company, Vat::from_events(&events), version, recorded))
}

/// The räkenskapsår that starts on `start`, or the ledger's
/// `fiscal_year_not_found`.
fn fiscal_year(company: &Company, start: Date) -> Result<FiscalYear> {
    let year = company.first_fiscal_year.containing(start);
    if year.start != start {
        return Err(doris_ledger::Error::Domain(doris_ledger::domain::DomainError::FiscalYearNotFound).into());
    }
    Ok(year)
}

/// The period ending `period_end`, of the year its last month is in.
fn resolve(company: &Company, vat: &Vat, period_end: Date) -> Result<(VatPeriodKind, VatPeriod)> {
    let year = company.first_fiscal_year.containing(period_end);
    let kind = vat.kind(year.start);
    if kind == VatPeriodKind::NotRegistered {
        return Err(DomainError::VatNotRegistered.into());
    }
    let period = period::periods(year, kind)
        .into_iter()
        .find(|p| p.end == period_end)
        .ok_or(DomainError::InvalidVatPeriod)?;
    Ok((kind, period))
}

async fn totals_in(conn: &mut SqliteConnection, company_id: Uuid, vat: &Vat, period: VatPeriod) -> Result<Vec<VatAccountTotal>> {
    Ok(doris_ledger::vat_box_totals_in(conn, company_id, period.start, period.end, &vat.vouchers()).await?)
}

async fn append(conn: &mut SqliteConnection, company_id: Uuid, expected_version: i64, events: &[VatEvent], actor: Uuid) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events.iter().map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION)).collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata { actor: Some(actor.to_string()) };
    doris_eventstore::append(conn, &vat_stream(company_id), expected_version, &new_events, &metadata).await?;
    Ok(())
}

pub async fn set_vat_period(pool: &SqlitePool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, kind: VatPeriodKind) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, vat, version, _) = load(&mut tx, company_id, actor).await?;
    let year = fiscal_year(&company, fiscal_year_start)?;
    let events = domain::set_vat_period(&vat, year, kind)?;
    append(&mut tx, company_id, version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeriodSummary {
    pub period: VatPeriod,
    pub status: VatStatus,
    pub due_date: Option<Date>,
    /// Box 49, kronor.
    pub vat_due: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VatYear {
    pub kind: VatPeriodKind,
    /// A period of the year is submitted, so the kind cannot change.
    pub locked: bool,
    pub periods: Vec<PeriodSummary>,
}

async fn summary(conn: &mut SqliteConnection, company_id: Uuid, vat: &Vat, kind: VatPeriodKind, period: VatPeriod, today: Date, corrected: &HashSet<VoucherRef>) -> Result<(PeriodSummary, Vec<VatAccountTotal>, Vec<AccountSaldo>)> {
    let totals = totals_in(conn, company_id, vat, period).await?;
    let accounts = domain::saldos(&totals);
    let summary = PeriodSummary {
        period,
        status: domain::status(vat, period, today, &accounts, corrected),
        due_date: period::due_date(period, kind),
        vat_due: domain::boxes(&accounts).vat_due,
    };
    Ok((summary, totals, accounts))
}

// ponytail: one ledger query per period (at most 13 a year); one grouped
// query when that shows.
pub async fn list_vat_returns(pool: &SqlitePool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, today: Date) -> Result<VatYear> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, _) = load(&mut conn, company_id, actor).await?;
    let year = fiscal_year(&company, fiscal_year_start)?;
    let kind = vat.kind(year.start);
    let corrected = doris_ledger::corrected_vouchers_in(&mut conn, company_id).await?;
    let mut periods = Vec::new();
    for period in period::periods(year, kind) {
        periods.push(summary(&mut conn, company_id, &vat, kind, period, today, &corrected).await?.0);
    }
    Ok(VatYear { kind, locked: domain::is_locked(&vat, year), periods })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionRecord {
    pub submission: Submission,
    pub submitted_at: String,
    pub submitted_by: Option<Uuid>,
    /// Its settlement voucher has been corrected.
    pub corrected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VatReturnView {
    pub summary: PeriodSummary,
    pub kind: VatPeriodKind,
    pub org_nr: String,
    pub vat_number: String,
    pub totals: Vec<VatAccountTotal>,
    pub boxes: Boxes,
    /// Box 49 in öre before rounding.
    pub booked_vat: i64,
    pub fingerprint: String,
    pub submissions: Vec<SubmissionRecord>,
}

pub async fn get_vat_return(pool: &SqlitePool, company_id: Uuid, actor: Uuid, period_end: Date, today: Date) -> Result<VatReturnView> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, recorded) = load(&mut conn, company_id, actor).await?;
    let (kind, period) = resolve(&company, &vat, period_end)?;
    let corrected = doris_ledger::corrected_vouchers_in(&mut conn, company_id).await?;
    let (summary, totals, accounts) = summary(&mut conn, company_id, &vat, kind, period, today, &corrected).await?;
    let mut submissions = Vec::new();
    for event in &recorded {
        if let VatEvent::VatReturnSubmitted(submission) = event.decode::<VatEvent>()?
            && submission.period_end == period_end
        {
            submissions.push(SubmissionRecord {
                corrected: submission.voucher.is_some_and(|v| corrected.contains(&v)),
                submitted_at: event.recorded_at.clone(),
                submitted_by: event.metadata.actor.as_deref().and_then(|a| a.parse().ok()),
                submission,
            });
        }
    }
    Ok(VatReturnView {
        summary,
        kind,
        org_nr: company.org_nr.formatted(),
        vat_number: eskd::vat_number(&company.org_nr),
        boxes: domain::boxes(&accounts),
        booked_vat: domain::booked_vat(&accounts),
        fingerprint: domain::fingerprint(period.end, &accounts),
        totals,
        submissions,
    })
}

/// The file to upload, its name, and the fingerprint of what it declares.
pub async fn export_vat_file(pool: &SqlitePool, company_id: Uuid, actor: Uuid, period_end: Date, today: Date) -> Result<(String, String, String)> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, _) = load(&mut conn, company_id, actor).await?;
    let (_, period) = resolve(&company, &vat, period_end)?;
    if period.end >= today {
        return Err(DomainError::VatPeriodNotEnded.into());
    }
    let accounts = domain::saldos(&totals_in(&mut conn, company_id, &vat, period).await?);
    let xml = eskd::eskd_xml(&company.org_nr, period, &domain::boxes(&accounts));
    Ok((eskd::file_name(&company.org_nr, period), xml, domain::fingerprint(period.end, &accounts)))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-vat`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vat
git commit -m "Read a company's VAT periods, returns and file"
```

---

### Task 10: Markera inlämnad och bokför avräkningen

**Files:**
- Modify: `crates/vat/src/lib.rs`
- Test: `crates/vat/tests/store.rs`

**Interfaces:**
- Produces: `pub async fn mark_vat_return_submitted(pool, company_id: Uuid, actor: Uuid, period_end: Date, fingerprint: &str, today: Date) -> Result<Option<VoucherRef>>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/vat/tests/store.rs` (import `mark_vat_return_submitted`):
```rust
async fn saldo(pool: &SqlitePool, id: Uuid, user: Uuid, account: u32) -> i64 {
    doris_ledger::trial_balance(pool, id, user, d("2026-01-01"))
        .await
        .unwrap()
        .iter()
        .find(|r| r.account == account)
        .map_or(0, |r| r.opening + r.debit - r.credit)
}

async fn mark(pool: &SqlitePool, id: Uuid, user: Uuid, end: &str) -> doris_vat::Result<Option<doris_ledger::VoucherRef>> {
    let print = get_vat_return(pool, id, user, d(end), d(TODAY)).await?.fingerprint;
    mark_vat_return_submitted(pool, id, user, d(end), &print, d(TODAY)).await
}

#[tokio::test]
async fn marking_submitted_books_the_settlement_and_zeroes_the_vat_accounts() {
    for method in [AccountingMethod::Invoice, AccountingMethod::Cash] {
        let pool = db().await;
        let anna = Uuid::new_v4();
        let id = company(&pool, anna, method).await;
        q3_books(&pool, id, anna).await;
        let voucher = mark(&pool, id, anna, "2026-09-30").await.unwrap().unwrap();
        assert_eq!(voucher.number, 3);
        assert_eq!((saldo(&pool, id, anna, 2611).await, saldo(&pool, id, anna, 2640).await), (0, 0));
        assert_eq!(saldo(&pool, id, anna, 2650).await, -2_100_00);
        assert_eq!(saldo(&pool, id, anna, 3740).await, -20);
        let view = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
        assert_eq!((view.summary.status, view.boxes.vat_due), (VatStatus::Submitted, 2_100), "the settlement is not counted");
        assert_eq!(view.submissions[0].submitted_by, Some(anna));
        let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2026-01-01")).await.unwrap();
        let settled = vouchers.iter().find(|v| v.number == 3).unwrap();
        assert_eq!((settled.date, settled.text.as_str()), (d("2026-09-30"), "Momsavräkning juli–september 2026"));
        let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
        assert!(year.locked);
    }
}

#[tokio::test]
async fn a_second_identical_submission_is_refused_and_books_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let print = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap().fingerprint;
    mark_vat_return_submitted(&pool, id, anna, d("2026-09-30"), &print, d(TODAY)).await.unwrap();
    let again = mark_vat_return_submitted(&pool, id, anna, d("2026-09-30"), &print, d(TODAY)).await;
    assert!(matches!(again, Err(Error::Domain(DomainError::VatReturnUnchanged))));
    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2026-01-01")).await.unwrap();
    assert_eq!(vouchers.len(), 3);
}

#[tokio::test]
async fn a_late_voucher_changes_the_period_and_the_next_submission_books_the_difference() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    mark(&pool, id, anna, "2026-09-30").await.unwrap();
    book(&pool, id, anna, "2026-09-20", &[(1510, 500_00, 0), (3001, 0, 400_00), (2611, 0, 100_00)]).await;
    let view = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!((view.summary.status, view.boxes.vat_due), (VatStatus::Changed, 2_200));
    let stale = mark_vat_return_submitted(&pool, id, anna, d("2026-09-30"), "stale", d(TODAY)).await;
    assert!(matches!(stale, Err(Error::Domain(DomainError::VatReturnOutdated))));
    mark(&pool, id, anna, "2026-09-30").await.unwrap();
    assert_eq!(saldo(&pool, id, anna, 2611).await, 0);
    assert_eq!(saldo(&pool, id, anna, 2650).await, -2_200_00);
}

#[tokio::test]
async fn a_corrected_settlement_is_booked_again() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let first = mark(&pool, id, anna, "2026-09-30").await.unwrap().unwrap();
    doris_ledger::correct_voucher(&pool, id, anna, first.fiscal_year_start, first.number, d(TODAY), d(TODAY)).await.unwrap();
    let view = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!(view.summary.status, VatStatus::Changed);
    assert!(view.submissions[0].corrected);
    mark(&pool, id, anna, "2026-09-30").await.unwrap();
    assert_eq!(saldo(&pool, id, anna, 2650).await, -2_100_00);
}

#[tokio::test]
async fn a_closed_year_refuses_the_submission_and_records_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    // First year 2025, so 2025 can be closed on 2026-10-06.
    let id = doris_company::register_company(
        &pool,
        anna,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: d("2025-01-01"),
            fiscal_year_end: d("2025-12-31"),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap();
    book(&pool, id, anna, "2025-12-10", &[(1930, 125_00, 0), (3001, 0, 100_00), (2611, 0, 25_00)]).await;
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), d(TODAY)).await.unwrap();
    let refused = mark(&pool, id, anna, "2025-12-31").await;
    assert!(matches!(refused, Err(Error::Ledger(_))));
    let view = get_vat_return(&pool, id, anna, d("2025-12-31"), d(TODAY)).await.unwrap();
    assert!(view.submissions.is_empty());
}

#[tokio::test]
async fn a_period_with_nothing_to_settle_is_recorded_without_a_voucher() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    assert_eq!(mark(&pool, id, anna, "2026-03-31").await.unwrap(), None);
    let view = get_vat_return(&pool, id, anna, d("2026-03-31"), d(TODAY)).await.unwrap();
    assert_eq!((view.summary.status, view.submissions.len()), (VatStatus::Submitted, 1));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p doris-vat --test store`
Expected: FAIL to compile (`mark_vat_return_submitted` missing).

- [ ] **Step 3: Implement**

Append to `crates/vat/src/lib.rs`:
```rust
/// Records the period as submitted and books its settlement voucher (dated
/// the period's last day) in one transaction: both, or nothing and no
/// voucher number used up. `fingerprint` is that of the file downloaded or
/// the declaration shown.
pub async fn mark_vat_return_submitted(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period_end: Date,
    fingerprint: &str,
    today: Date,
) -> Result<Option<VoucherRef>> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, vat, version, _) = load(&mut tx, company_id, actor).await?;
    let (kind, period) = resolve(&company, &vat, period_end)?;
    let accounts = domain::saldos(&totals_in(&mut tx, company_id, &vat, period).await?);
    let corrected = doris_ledger::corrected_vouchers_in(&mut tx, company_id).await?;
    let domain::Prepared { lines, mut submission } =
        domain::submit(&vat, period, kind, today, accounts, fingerprint, &corrected)?;
    if !lines.is_empty() {
        let cmd = doris_ledger::domain::RecordVoucher {
            date: period.end,
            text: format!("Momsavräkning {}", period.label()),
            lines,
        };
        submission.voucher = Some(doris_ledger::record_voucher_in(&mut tx, company_id, actor, cmd, today).await?);
    }
    let voucher = submission.voucher;
    append(&mut tx, company_id, version, &[VatEvent::VatReturnSubmitted(submission)], actor).await?;
    tx.commit().await?;
    Ok(voucher)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-vat && cargo clippy -p doris-vat -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/vat
git commit -m "Mark a VAT period submitted and book its settlement in one transaction"
```

---

### Task 11: `VatService` (proto och server)

**Files:**
- Create: `proto/doris/vat/v1/vat.proto`, `crates/server/src/vat.rs`
- Modify: `crates/proto/build.rs`, `crates/proto/src/lib.rs`, `crates/server/Cargo.toml`, `crates/server/src/lib.rs`, `crates/server/src/main.rs`, `crates/server/tests/common/mod.rs`
- Test: `crates/server/tests/vat.rs`

**Interfaces:**
- Consumes: Tasks 9–10.
- Produces: `doris_server::VatApi::new(pool)`; `router(…, vat: VatApi, …)` (new parameter after `invoicing`); proto package `doris.vat.v1` as below; `common::TestServer::vat() -> Vat` client.

- [ ] **Step 1: Write the proto**

`proto/doris/vat/v1/vat.proto`:
```proto
syntax = "proto3";

package doris.vat.v1;

// The momsdeklaration: periods per räkenskapsår, each period's boxes, the
// eSKD file and marking a period submitted (which books the settlement).
service VatService {
  rpc SetVatPeriod(SetVatPeriodRequest) returns (SetVatPeriodResponse);
  rpc ListVatReturns(ListVatReturnsRequest) returns (ListVatReturnsResponse);
  rpc GetVatReturn(VatReturnRef) returns (VatReturn);
  rpc ExportVatFile(VatReturnRef) returns (VatFile);
  rpc MarkVatReturnSubmitted(MarkVatReturnSubmittedRequest) returns (MarkVatReturnSubmittedResponse);
}

enum VatPeriodKind {
  VAT_PERIOD_KIND_UNSPECIFIED = 0;
  VAT_PERIOD_KIND_MONTHLY = 1;
  VAT_PERIOD_KIND_QUARTERLY = 2;
  VAT_PERIOD_KIND_YEARLY = 3;
  VAT_PERIOD_KIND_NOT_REGISTERED = 4;
}

enum VatStatus {
  VAT_STATUS_UNSPECIFIED = 0;
  VAT_STATUS_IN_PROGRESS = 1;
  VAT_STATUS_TO_SUBMIT = 2;
  VAT_STATUS_SUBMITTED = 3;
  VAT_STATUS_CHANGED = 4;
}

message SetVatPeriodRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  VatPeriodKind kind = 3;
}

message SetVatPeriodResponse {}

message ListVatReturnsRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}

message VatPeriodSummary {
  string period = 1;   // ÅÅÅÅMM of the last month
  string start = 2;
  string end = 3;
  string label = 4;    // "juli–september 2026"
  VatStatus status = 5;
  string due_date = 6; // empty for helår
  int64 vat_due = 7;   // box 49, kronor
}

message ListVatReturnsResponse {
  VatPeriodKind kind = 1;
  bool locked = 2;
  repeated VatPeriodSummary periods = 3;
}

message VatReturnRef {
  string company_id = 1;
  string period = 2; // ÅÅÅÅMM
}

message VatBoxAccount {
  uint32 number = 1;
  string name = 2;
  int64 amount = 3; // öre, as the box counts it
}

message VatBoxAmount {
  uint32 box = 1;
  int64 amount = 2; // kronor
  repeated VatBoxAccount accounts = 3;
}

message VatSubmission {
  string submitted_at = 1;
  string submitted_by_name = 2;
  string fiscal_year_start = 3;
  uint32 voucher_number = 4; // 0: nothing was booked
  bool corrected = 5;
}

message VatReturn {
  VatPeriodSummary summary = 1;
  VatPeriodKind kind = 2;
  string org_nr = 3;
  string vat_number = 4;
  repeated VatBoxAmount boxes = 5; // every box with an account behind it
  int64 booked_vat = 6;            // box 49 in öre before rounding
  string fingerprint = 7;
  repeated VatSubmission submissions = 8;
}

message VatFile {
  string file_name = 1;
  string content = 2;
  string fingerprint = 3;
}

message MarkVatReturnSubmittedRequest {
  string company_id = 1;
  string period = 2;
  string fingerprint = 3;
}

message MarkVatReturnSubmittedResponse {
  string fiscal_year_start = 1;
  uint32 voucher_number = 2; // 0: nothing was booked
}
```
`crates/proto/build.rs`: add `"../../proto/doris/vat/v1/vat.proto",`. `crates/proto/src/lib.rs`:
```rust
pub mod vat {
    pub mod v1 {
        tonic::include_proto!("doris.vat.v1");
    }
}
```

- [ ] **Step 2: Write the failing server test**

First add the client to `crates/server/tests/common/mod.rs`, next to `Payroll` (copy its type alias and `payroll()` method, changing the names):
```rust
pub type Vat = doris_proto::vat::v1::vat_service_client::VatServiceClient<Transport>;
// in impl TestServer:
    pub fn vat(&self) -> Vat {
        Vat::new(self.transport())
    }
```
(Use exactly the `Transport`/constructor shape `payroll()` uses.) In `launch`, pass `VatApi::new(pool.clone())` to `router` after the `InvoicingApi` argument, and import `VatApi`.

`crates/server/tests/vat.rs`:
```rust
mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::vat::v1 as pb;
use tonic::{Code, Request};

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company, first räkenskapsår 2025, so its periods have ended.
async fn company(server: &TestServer, session: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2025-01-01".into(),
                fiscal_year_end: "2025-12-31".into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
}

fn r(company_id: &str, period: &str) -> pb::VatReturnRef {
    pb::VatReturnRef { company_id: company_id.into(), period: period.into() }
}

fn mark(company_id: &str, period: &str, fingerprint: &str) -> pb::MarkVatReturnSubmittedRequest {
    pb::MarkVatReturnSubmittedRequest { company_id: company_id.into(), period: period.into(), fingerprint: fingerprint.into() }
}

fn line(account: u32, debit: i64, credit: i64) -> lpb::VoucherLine {
    lpb::VoucherLine { account, debit, credit }
}

#[tokio::test]
async fn a_member_declares_a_quarter_and_the_settlement_is_booked() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    server
        .ledger()
        .record_voucher(authed(
            lpb::RecordVoucherRequest {
                company_id: id.clone(),
                date: "2025-11-10".into(),
                text: "Försäljning".into(),
                lines: vec![line(1930, 125_000, 0), line(3001, 0, 100_000), line(2611, 0, 25_000)],
                attachments: vec![],
            },
            &anna,
        ))
        .await
        .unwrap();
    let mut api = server.vat();
    let year = api
        .list_vat_returns(authed(pb::ListVatReturnsRequest { company_id: id.clone(), fiscal_year_start: "2025-01-01".into() }, &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(year.kind(), pb::VatPeriodKind::Quarterly);
    let q4 = year.periods.last().unwrap();
    assert_eq!(
        (q4.period.as_str(), q4.label.as_str(), q4.status(), q4.vat_due, q4.due_date.as_str()),
        ("202512", "oktober–december 2025", pb::VatStatus::ToSubmit, 250, "2026-02-12")
    );

    let declaration = api.get_vat_return(authed(r(&id, "202512"), &anna)).await.unwrap().into_inner();
    let boxes: Vec<(u32, i64)> = declaration.boxes.iter().map(|b| (b.r#box, b.amount)).collect();
    assert_eq!(boxes, [(5, 1_000), (10, 250)]);
    assert_eq!(declaration.boxes[0].accounts[0].amount, 100_000);
    assert_eq!(declaration.vat_number, "SE556016068001");

    let file = api.export_vat_file(authed(r(&id, "202512"), &anna)).await.unwrap().into_inner();
    assert_eq!(file.file_name, "moms_5560160680_202512.xml");
    assert!(file.content.contains("<MomsUtgHog>250</MomsUtgHog>"));

    let marked = api.mark_vat_return_submitted(authed(mark(&id, "202512", &file.fingerprint), &anna)).await.unwrap().into_inner();
    assert_eq!((marked.fiscal_year_start.as_str(), marked.voucher_number), ("2025-01-01", 2));
    let again = api.mark_vat_return_submitted(authed(mark(&id, "202512", &file.fingerprint), &anna)).await.unwrap_err();
    assert_eq!(code_of(again), (Code::FailedPrecondition, "vat_return_unchanged".into()));

    let after = api.get_vat_return(authed(r(&id, "202512"), &anna)).await.unwrap().into_inner();
    assert_eq!(after.summary.unwrap().status(), pb::VatStatus::Submitted);
    assert_eq!((after.submissions[0].submitted_by_name.as_str(), after.submissions[0].voucher_number), ("Anna", 2));
}

#[tokio::test]
async fn vat_input_gets_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.vat();
    for period in ["2025", "202513", "202511", "abcdef"] {
        let err = api.get_vat_return(authed(r(&id, period), &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_vat_period".into()), "{period}");
    }
    let err = api.mark_vat_return_submitted(authed(mark(&id, "202503", "stale"), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "vat_return_outdated".into()));
    let fingerprint = api.get_vat_return(authed(r(&id, "202503"), &anna)).await.unwrap().into_inner().fingerprint;
    let marked = api.mark_vat_return_submitted(authed(mark(&id, "202503", &fingerprint), &anna)).await.unwrap().into_inner();
    assert_eq!(marked.voucher_number, 0, "nothing to book");

    let set = |kind: i32| pb::SetVatPeriodRequest { company_id: id.clone(), fiscal_year_start: "2025-01-01".into(), kind };
    let err = api.set_vat_period(authed(set(pb::VatPeriodKind::Monthly as i32), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "vat_period_locked".into()));
    let err = api.set_vat_period(authed(set(0), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_vat_period".into()));
}

#[tokio::test]
async fn strangers_and_signed_out_callers_find_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    // An invited user who is not a member of Anna's company.
    let bertil = server.invite(&anna, "bertil@example.se").await;
    let mut api = server.vat();
    let err = api.get_vat_return(authed(r(&id, "202503"), &bertil)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api.get_vat_return(Request::new(r(&id, "202503"))).await.unwrap_err();
    assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".into()));
}
```
`sign_up` registers the display name "Anna"; `invite` signs up an invited user and returns their session.

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p doris-server --test vat`
Expected: FAIL to compile (`VatApi` missing).

- [ ] **Step 4: Implement the service**

`crates/server/Cargo.toml`: `doris-vat.workspace = true` (and `doris-vat = { path = "crates/vat" }` is already in the workspace from Task 5).

`crates/server/src/vat.rs`:
```rust
//! `doris.vat.v1.VatService`: maps gRPC calls onto `doris_vat`. Every call
//! needs a session, and a company the caller isn't a member of looks
//! exactly like one that doesn't exist. An org nr can be a personnummer:
//! never logged.

use crate::grpc::{signed_in_user, today};
use doris_proto::vat::v1 as pb;
use doris_proto::vat::v1::vat_service_server::VatService;
use doris_vat::domain::{DomainError, VatStatus};
use doris_vat::period::VatPeriodKind;
use doris_vat::{Error, PeriodSummary};
use jiff::civil::{Date, date};
use sqlx::SqlitePool;
use std::collections::HashMap;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct VatApi {
    pool: SqlitePool,
}

impl VatApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn caller<T>(&self, request: &Request<T>, company_id: &str) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id.parse().map_err(|_| Status::not_found("company_not_found"))?;
        Ok((company, user.id))
    }
}

#[tonic::async_trait]
impl VatService for VatApi {
    async fn set_vat_period(&self, request: Request<pb::SetVatPeriodRequest>) -> Result<Response<pb::SetVatPeriodResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let kind = match pb::VatPeriodKind::try_from(req.kind) {
            Ok(pb::VatPeriodKind::Monthly) => VatPeriodKind::Monthly,
            Ok(pb::VatPeriodKind::Quarterly) => VatPeriodKind::Quarterly,
            Ok(pb::VatPeriodKind::Yearly) => VatPeriodKind::Yearly,
            Ok(pb::VatPeriodKind::NotRegistered) => VatPeriodKind::NotRegistered,
            _ => return Err(domain_status(DomainError::InvalidVatPeriod)),
        };
        doris_vat::set_vat_period(&self.pool, company, user, crate::ledger::date(&req.fiscal_year_start)?, kind)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetVatPeriodResponse {}))
    }

    async fn list_vat_returns(&self, request: Request<pb::ListVatReturnsRequest>) -> Result<Response<pb::ListVatReturnsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let start = crate::ledger::date(&request.get_ref().fiscal_year_start)?;
        let year = doris_vat::list_vat_returns(&self.pool, company, user, start, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ListVatReturnsResponse {
            kind: kind_message(year.kind) as i32,
            locked: year.locked,
            periods: year.periods.iter().map(summary_message).collect(),
        }))
    }

    async fn get_vat_return(&self, request: Request<pb::VatReturnRef>) -> Result<Response<pb::VatReturn>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let end = period_end(&request.get_ref().period)?;
        let view = doris_vat::get_vat_return(&self.pool, company, user, end, today())
            .await
            .map_err(status)?;
        // Identity knows the names; once per person.
        let mut names = HashMap::<Uuid, String>::new();
        for id in view.submissions.iter().filter_map(|s| s.submitted_by) {
            if !names.contains_key(&id) {
                let name = doris_identity::get_user(&self.pool, id)
                    .await
                    .map_err(|_| Status::internal("internal"))?
                    .map(|u| u.display_name)
                    .unwrap_or_default();
                names.insert(id, name);
            }
        }
        let mut boxes: Vec<pb::VatBoxAmount> = Vec::new();
        for t in &view.totals {
            let account = pb::VatBoxAccount {
                number: t.number.into(),
                name: t.name.clone(),
                amount: doris_vat::domain::signed(&doris_vat::domain::AccountSaldo { account: t.number, vat_box: t.vat_box, saldo: t.saldo }),
            };
            let n = u32::from(t.vat_box);
            match boxes.iter_mut().find(|b| b.r#box == n) {
                Some(b) => b.accounts.push(account),
                None => boxes.push(pb::VatBoxAmount { r#box: n, amount: view.boxes.get(t.vat_box.get()), accounts: vec![account] }),
            }
        }
        boxes.sort_by_key(|b| b.r#box);
        Ok(Response::new(pb::VatReturn {
            summary: Some(summary_message(&view.summary)),
            kind: kind_message(view.kind) as i32,
            org_nr: view.org_nr,
            vat_number: view.vat_number,
            boxes,
            booked_vat: view.booked_vat,
            fingerprint: view.fingerprint,
            submissions: view
                .submissions
                .iter()
                .map(|s| pb::VatSubmission {
                    submitted_at: s.submitted_at.clone(),
                    submitted_by_name: s.submitted_by.and_then(|id| names.get(&id).cloned()).unwrap_or_default(),
                    fiscal_year_start: s.submission.voucher.map(|v| v.fiscal_year_start.to_string()).unwrap_or_default(),
                    voucher_number: s.submission.voucher.map_or(0, |v| v.number),
                    corrected: s.corrected,
                })
                .collect(),
        }))
    }

    async fn export_vat_file(&self, request: Request<pb::VatReturnRef>) -> Result<Response<pb::VatFile>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let end = period_end(&request.get_ref().period)?;
        let (file_name, content, fingerprint) = doris_vat::export_vat_file(&self.pool, company, user, end, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::VatFile { file_name, content, fingerprint }))
    }

    async fn mark_vat_return_submitted(&self, request: Request<pb::MarkVatReturnSubmittedRequest>) -> Result<Response<pb::MarkVatReturnSubmittedResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let voucher = doris_vat::mark_vat_return_submitted(&self.pool, company, user, period_end(&req.period)?, &req.fingerprint, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::MarkVatReturnSubmittedResponse {
            fiscal_year_start: voucher.map(|v| v.fiscal_year_start.to_string()).unwrap_or_default(),
            voucher_number: voucher.map_or(0, |v| v.number),
        }))
    }
}

/// "ÅÅÅÅMM" → the month's last day.
fn period_end(raw: &str) -> Result<Date, Status> {
    let invalid = || domain_status(DomainError::InvalidVatPeriod);
    if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let (year, month): (i16, i8) = (raw[..4].parse().map_err(|_| invalid())?, raw[4..].parse().map_err(|_| invalid())?);
    if !(1..=12).contains(&month) || !(1900..=2999).contains(&year) {
        return Err(invalid());
    }
    Ok(date(year, month, 1).last_of_month())
}

fn kind_message(kind: VatPeriodKind) -> pb::VatPeriodKind {
    match kind {
        VatPeriodKind::Monthly => pb::VatPeriodKind::Monthly,
        VatPeriodKind::Quarterly => pb::VatPeriodKind::Quarterly,
        VatPeriodKind::Yearly => pb::VatPeriodKind::Yearly,
        VatPeriodKind::NotRegistered => pb::VatPeriodKind::NotRegistered,
    }
}

fn summary_message(s: &PeriodSummary) -> pb::VatPeriodSummary {
    pb::VatPeriodSummary {
        period: s.period.code(),
        start: s.period.start.to_string(),
        end: s.period.end.to_string(),
        label: s.period.label(),
        status: match s.status {
            VatStatus::InProgress => pb::VatStatus::InProgress,
            VatStatus::ToSubmit => pb::VatStatus::ToSubmit,
            VatStatus::Submitted => pb::VatStatus::Submitted,
            VatStatus::Changed => pb::VatStatus::Changed,
        } as i32,
        due_date: s.due_date.map(|d| d.to_string()).unwrap_or_default(),
        vat_due: s.vat_due,
    }
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidVatPeriod => Status::invalid_argument("invalid_vat_period"),
        VatPeriodNotEnded => Status::failed_precondition("vat_period_not_ended"),
        VatPeriodLocked => Status::failed_precondition("vat_period_locked"),
        VatReturnOutdated => Status::failed_precondition("vat_return_outdated"),
        VatReturnUnchanged => Status::failed_precondition("vat_return_unchanged"),
        VatNotRegistered => Status::failed_precondition("vat_not_registered"),
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
`crate::ledger::date` is the ledger's date parser (`invalid_date`). `doris-identity` is already a server dependency.

`crates/server/src/lib.rs`: `mod vat;`, `use doris_proto::vat::v1::vat_service_server::VatServiceServer;`, `pub use vat::VatApi;`, a `vat: VatApi` parameter after `invoicing`, and `.add_service(VatServiceServer::new(vat))` after the invoicing service. `crates/server/src/main.rs`: import `VatApi` and pass `VatApi::new(pool.clone())` before `InvoicingApi::new(pool)` moves the pool (reorder: `let vat = VatApi::new(pool.clone());` above the `router` call).

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-server`
Expected: PASS (all server tests, since `router` gained a parameter).

- [ ] **Step 6: Commit**

```bash
git add proto crates/proto crates/server Cargo.lock
git commit -m "Serve the momsdeklaration over gRPC-Web"
```

---

### Task 12: Webbklient, felkoder och blankettens struktur

**Files:**
- Modify: `crates/web/src/api.rs`, `crates/web/src/errors.rs`
- Create: `crates/web/src/vat_form.rs`
- Modify: `crates/web/src/main.rs` (`mod vat_form;`)
- Test: unit tests in `errors.rs` and `vat_form.rs`

**Interfaces:**
- Produces: `api::{vat_api, vpb, VatApi}`; `vat_form::{SECTIONS, Section, Row, box_label}`:
```rust
pub struct Row { pub vat_box: u32, pub label: &'static str }
pub struct Section { pub letter: char, pub title: &'static str, pub rows: &'static [Row], pub right: bool }
pub const SECTIONS: [Section; 9];
pub fn box_label(n: u32) -> Option<&'static str>
```

- [ ] **Step 1: Write the failing tests**

In `crates/web/src/errors.rs` tests module:
```rust
    #[test]
    fn vat_codes_have_swedish_messages() {
        for code in [
            "invalid_vat_box",
            "invalid_vat_period",
            "vat_period_not_ended",
            "vat_period_locked",
            "vat_return_outdated",
            "vat_return_unchanged",
            "vat_not_registered",
        ] {
            assert_ne!(message(code), message("something_else"), "{code}");
        }
    }
```
`crates/web/src/vat_form.rs` (tests at the bottom; the module with an empty `SECTIONS` won't compile yet, so write the test first in the new file and the declarations in step 3):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_has_every_box_once_left_and_right_as_skv_4700() {
        let mut boxes: Vec<u32> = SECTIONS.iter().flat_map(|s| s.rows.iter().map(|r| r.vat_box)).collect();
        boxes.sort_unstable();
        let mut expected = vec![5, 6, 7, 8, 10, 11, 12, 20, 21, 22, 23, 24, 30, 31, 32, 35, 36, 37, 38, 39, 40, 41, 42, 48, 49, 50, 60, 61, 62];
        expected.sort_unstable();
        assert_eq!(boxes, expected);
        let left: String = SECTIONS.iter().filter(|s| !s.right).map(|s| s.letter).collect();
        let right: String = SECTIONS.iter().filter(|s| s.right).map(|s| s.letter).collect();
        assert_eq!((left.as_str(), right.as_str()), ("ACHE", "BDIFG"));
        assert_eq!(box_label(48), Some("Ingående moms att dra av"));
        assert_eq!(box_label(49), Some("Moms att betala eller få tillbaka"));
        assert_eq!(box_label(13), None);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p doris-web vat_`
Expected: FAIL (module/fields missing; messages are the generic one).

- [ ] **Step 3: Implement**

`crates/web/src/api.rs`: `use doris_proto::vat::v1::vat_service_client::VatServiceClient;`, `pub use doris_proto::vat::v1 as vpb;`, `pub type VatApi = VatServiceClient<Client>;` and
```rust
pub fn vat_api() -> VatApi {
    VatServiceClient::new(client())
}
```
`crates/web/src/errors.rs`, in `message`:
```rust
        "invalid_vat_box" => "Välj en ruta som finns på momsdeklarationen.",
        "invalid_vat_period" => "Den redovisningsperioden finns inte.",
        "vat_period_not_ended" => "Perioden har inte tagit slut än.",
        "vat_period_locked" => "Redovisningsperioden kan inte ändras när en deklaration för året är inlämnad.",
        "vat_return_outdated" => "Bokföringen har ändrats sedan deklarationen visades. Ladda om sidan och kontrollera den igen.",
        "vat_return_unchanged" => "Perioden är redan inlämnad och har inte ändrats.",
        "vat_not_registered" => "Företaget är inte momsregistrerat det räkenskapsåret.",
```
`crates/web/src/vat_form.rs` (above the tests):
```rust
//! Skatteverket's momsdeklaration (SKV 4700) as the page draws it: its
//! sections, headings and row texts word for word, in its two columns.

pub struct Row {
    pub vat_box: u32,
    pub label: &'static str,
}

pub struct Section {
    pub letter: char,
    pub title: &'static str,
    pub rows: &'static [Row],
    /// In the form's right-hand column.
    pub right: bool,
}

const fn row(vat_box: u32, label: &'static str) -> Row {
    Row { vat_box, label }
}

pub const SECTIONS: [Section; 9] = [
    Section { letter: 'A', title: "Momspliktig försäljning eller uttag exklusive moms", right: false, rows: &[
        row(5, "Momspliktig försäljning som inte ingår i ruta 06, 07 eller 08"),
        row(6, "Momspliktiga uttag"),
        row(7, "Beskattningsunderlag vid vinstmarginalbeskattning"),
        row(8, "Hyresinkomster vid frivillig skattskyldighet"),
    ] },
    Section { letter: 'B', title: "Utgående moms på försäljning eller uttag i ruta 05–08", right: true, rows: &[
        row(10, "Utgående moms 25 %"),
        row(11, "Utgående moms 12 %"),
        row(12, "Utgående moms 6 %"),
    ] },
    Section { letter: 'C', title: "Momspliktiga inköp vid omvänd skattskyldighet", right: false, rows: &[
        row(20, "Inköp av varor från ett annat EU-land"),
        row(21, "Inköp av tjänster från ett annat EU-land enligt huvudregeln"),
        row(22, "Inköp av tjänster från ett land utanför EU"),
        row(23, "Inköp av varor i Sverige som köparen är skattskyldig för"),
        row(24, "Övriga inköp av tjänster i Sverige som köparen är skattskyldig för"),
    ] },
    Section { letter: 'D', title: "Utgående moms på inköp i ruta 20–24", right: true, rows: &[
        row(30, "Utgående moms 25 %"),
        row(31, "Utgående moms 12 %"),
        row(32, "Utgående moms 6 %"),
    ] },
    Section { letter: 'H', title: "Import", right: false, rows: &[
        row(50, "Beskattningsunderlag vid import"),
    ] },
    Section { letter: 'I', title: "Utgående moms på import i ruta 50", right: true, rows: &[
        row(60, "Utgående moms 25 %"),
        row(61, "Utgående moms 12 %"),
        row(62, "Utgående moms 6 %"),
    ] },
    Section { letter: 'E', title: "Försäljning m.m. som är undantagen från moms", right: false, rows: &[
        row(35, "Försäljning av varor till ett annat EU-land"),
        row(36, "Försäljning av varor utanför EU"),
        row(37, "Mellanmans inköp av varor vid trepartshandel"),
        row(38, "Mellanmans försäljning av varor vid trepartshandel"),
        row(39, "Försäljning av tjänster till näringsidkare i annat EU-land enligt huvudregeln"),
        row(40, "Övrig försäljning av tjänster omsatta utanför Sverige"),
        row(41, "Försäljning när köparen är skattskyldig i Sverige"),
        row(42, "Övrig försäljning m.m."),
    ] },
    Section { letter: 'F', title: "Ingående moms", right: true, rows: &[
        row(48, "Ingående moms att dra av"),
    ] },
    Section { letter: 'G', title: "Moms att betala eller få tillbaka", right: true, rows: &[
        row(49, "Moms att betala eller få tillbaka"),
    ] },
];

pub fn box_label(n: u32) -> Option<&'static str> {
    SECTIONS.iter().flat_map(|s| s.rows).find(|r| r.vat_box == n).map(|r| r.label)
}
```
Declare `mod vat_form;` in `crates/web/src/main.rs`, after `mod ui;`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-web`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Add the VAT client, its error messages and SKV 4700's layout"
```

---

### Task 13: Momsruta i kontoplanen

**Files:**
- Modify: `crates/web/src/pages/accounts.rs`

**Interfaces:**
- Consumes: Task 4 (`lpb::SetAccountVatBoxRequest`, `Account.vat_box`), Task 12 (`vat_form::box_label`, `SECTIONS`).

- [ ] **Step 1: Write the failing test**

Add to `crates/web/src/pages/accounts.rs`:
```rust
/// "05 Momspliktig försäljning…", or "–" for no box.
pub fn vat_box_text(n: u32) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_reads_as_its_number_and_the_forms_text() {
        assert_eq!(vat_box_text(5), "05 Momspliktig försäljning som inte ingår i ruta 06, 07 eller 08");
        assert_eq!(vat_box_text(48), "48 Ingående moms att dra av");
        assert_eq!(vat_box_text(0), "–");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-web a_box_reads`
Expected: FAIL (panics at `todo!()`).

- [ ] **Step 3: Implement**

```rust
pub fn vat_box_text(n: u32) -> String {
    match crate::vat_form::box_label(n) {
        Some(label) if n != 49 => format!("{n:02} {label}"),
        _ => "–".into(),
    }
}
```
In `Accounts`, add the header cell `<th class=TABLE_HEADER_CELL>"Momsruta"</th>` after "Namn", and include `a.vat_box` in the `For` key. In `AccountRow`, destructure `vat_box` too (`let lpb::Account { number, name: current, active, vat_box } = account;`), add a signal and a cell with a hidden-label `Select`:
```rust
    let chosen = RwSignal::new(vat_box.to_string());
    Effect::new(move |previous: Option<String>| {
        let value = chosen.get();
        // Only a change by the user, not the first run.
        if previous.is_some_and(|p| p != value) {
            error.set(None);
            let vat_box = value.parse().unwrap_or(0);
            spawn_local(async move {
                let request = lpb::SetAccountVatBoxRequest { company_id: company_id.get_value(), number, vat_box };
                match ledger_api().set_account_vat_box(request).await {
                    Ok(_) => changed.run(()),
                    Err(status) => error.set(Some(describe(&status))),
                }
            });
        }
        value
    });
```
and in the row, after the name cell, a native select (not `ui::Select`: its `id` is `&'static str` and would repeat on every row), named per account:
```rust
            <td class=TABLE_CELL>
                <div class="relative w-64 max-w-full">
                    <select
                        class=SELECT
                        aria-label=format!("Momsruta för {number}")
                        prop:value=move || chosen.get()
                        on:change=move |ev| chosen.set(event_target_value(&ev))
                    >
                        <option class=SELECT_OPTION value="0">"–"</option>
                        {crate::vat_form::SECTIONS.iter().flat_map(|s| s.rows).filter(|r| r.vat_box != 49).map(|r| view! {
                            <option class=SELECT_OPTION value=r.vat_box.to_string()>{vat_box_text(r.vat_box)}</option>
                        }).collect_view()}
                    </select>
                    <Icon name=IconName::ChevronDown class="pointer-events-none absolute top-1/2 right-1.5 size-3.5 -translate-y-1/2 text-muted-foreground select-none" />
                </div>
            </td>
```
Import `SELECT`, `SELECT_OPTION`, `Icon` and `IconName` from `crate::ui` (both constants are already `pub`).

- [ ] **Step 4: Run tests and lints**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Choose each account's momsruta in the kontoplan"
```

---

### Task 14: Sidorna Moms och Momsdeklaration

**Files:**
- Create: `crates/web/src/pages/vat.rs`, `crates/web/src/pages/vat_return.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs` (routes), `crates/web/src/nav.rs` (menu item, `section_of`)
- Modify: `crates/web/src/attachments.rs` only if `save_as` needs nothing new (it takes `&str`: fine)
- Modify: `e2e/tests/design.spec.ts`, `e2e/tests/leaving.spec.ts` (paths `"/vat"`, `"/vat/202603"`), `e2e/tests/fixtures.ts` (`MENU_OF`)

**Interfaces:**
- Consumes: Tasks 11–12.
- Produces: components `pages::vat::Vat` (route `/vat`), `pages::vat_return::VatReturnPage` (route `/vat/:period`); pure helpers `vat_status_label(vpb::VatStatus) -> &'static str`, `kind_label`, `box_amount(kr: i64, n: u32) -> String`.

- [ ] **Step 1: Write the failing unit tests**

In `crates/web/src/pages/vat.rs`:
```rust
pub fn vat_status_label(status: vpb::VatStatus) -> &'static str {
    todo!()
}

/// Kronor as the form shows them: "412 300", "−24 610" for box 48, "–" for empty.
pub fn box_amount(kr: i64, vat_box: u32) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_amounts_read_as_on_the_form() {
        assert_eq!(vat_status_label(vpb::VatStatus::InProgress), "Pågår");
        assert_eq!(vat_status_label(vpb::VatStatus::ToSubmit), "Att lämna");
        assert_eq!(vat_status_label(vpb::VatStatus::Submitted), "Inlämnad");
        assert_eq!(vat_status_label(vpb::VatStatus::Changed), "Ändrad");
        assert_eq!(box_amount(412_300, 5), "412\u{a0}300");
        assert_eq!(box_amount(24_610, 48), "−24\u{a0}610");
        assert_eq!(box_amount(0, 10), "–");
        assert_eq!(box_amount(-1_500, 42), "-1\u{a0}500");
    }
}
```


- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p doris-web statuses_and_amounts`
Expected: FAIL (`todo!()`; module not declared yet → declare `pub mod vat;` in `pages/mod.rs` first).

- [ ] **Step 3: Implement the helpers**

```rust
pub fn vat_status_label(status: vpb::VatStatus) -> &'static str {
    match status {
        vpb::VatStatus::InProgress => "Pågår",
        vpb::VatStatus::ToSubmit => "Att lämna",
        vpb::VatStatus::Submitted => "Inlämnad",
        vpb::VatStatus::Changed => "Ändrad",
        vpb::VatStatus::Unspecified => "",
    }
}

pub fn box_amount(kr: i64, vat_box: u32) -> String {
    // Whole kronor, grouped as everywhere else.
    let grouped = amount(kr * 100).trim_end_matches(",00").to_owned();
    match (kr, vat_box) {
        (0, _) => "–".into(),
        // The form prints box 48 with a minus: it is deducted.
        (_, 48) => format!("−{grouped}"),
        _ => grouped,
    }
}
```
(`crate::format::amount` groups with U+00A0 and ends in ",00" for whole kronor.)

Run: `cargo test -p doris-web statuses_and_amounts` → PASS.

- [ ] **Step 4: Build the list page `/vat`**

`crates/web/src/pages/vat.rs` component (structure like `trial_balance.rs` for the year, `agi.rs` for loading):
```rust
//! Moms: the chosen räkenskapsår's redovisningsperiod and its periods,
//! each with status, deklarationsdag and box 49.

use crate::active_company::Companies;
use crate::api::{vat_api, vpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url, use_fiscal_years};
use crate::format::amount;
use crate::overview::whole_kronor;
use crate::task::spawn_local;
use crate::ui::{Badge, BadgeVariant, Card, ErrorAlert, PageHeader, SELECT_OPTION, Select, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, TableCard};
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

fn kind_value(kind: vpb::VatPeriodKind) -> &'static str {
    match kind {
        vpb::VatPeriodKind::Monthly => "monthly",
        vpb::VatPeriodKind::Yearly => "yearly",
        vpb::VatPeriodKind::NotRegistered => "not_registered",
        _ => "quarterly",
    }
}

fn kind_of(value: &str) -> vpb::VatPeriodKind {
    match value {
        "monthly" => vpb::VatPeriodKind::Monthly,
        "yearly" => vpb::VatPeriodKind::Yearly,
        "not_registered" => vpb::VatPeriodKind::NotRegistered,
        _ => vpb::VatPeriodKind::Quarterly,
    }
}

#[component]
pub fn Vat() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    keep_year_in_url("/vat".into(), year);
    // The year's answer and the (company, year) it is for.
    let loaded = RwSignal::new(None::<(String, String, vpb::ListVatReturnsResponse)>);
    let kind = RwSignal::new(String::new());

    let load = move || {
        let (company_id, start) = (companies.active.get_untracked(), year.get_untracked());
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = vat_api()
                .list_vat_returns(vpb::ListVatReturnsRequest { company_id: company_id.clone(), fiscal_year_start: start.clone() })
                .await;
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    let response = response.into_inner();
                    kind.set(kind_value(response.kind()).into());
                    loaded.set(Some((company_id, start, response)));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        year.track();
        loaded.set(None);
        load();
    });
    // A change of the select (not the value loaded into it) is saved.
    Effect::new(move |_| {
        let chosen = kind.get();
        let Some((company_id, start, response)) = loaded.get_untracked() else { return };
        if chosen.is_empty() || chosen == kind_value(response.kind()) {
            return;
        }
        error.set(None);
        spawn_local(async move {
            let request = vpb::SetVatPeriodRequest { company_id, fiscal_year_start: start, kind: kind_of(&chosen) as i32 };
            match vat_api().set_vat_period(request).await {
                Ok(_) => load(),
                Err(status) => {
                    error.set(Some(describe(&status)));
                    load();
                }
            }
        });
    });

    view! {
        <div class="grid gap-6">
            <PageHeader title="Moms">
                <FiscalYearSelect years=years year=year />
            </PageHeader>
            <ErrorAlert message=error />
            <Card title="Redovisningsperiod" narrow=true>
                <Select label="Redovisningsperiod" id="vat_period_kind" hide_label=true value=kind>
                    <option class=SELECT_OPTION value="monthly">"Månad"</option>
                    <option class=SELECT_OPTION value="quarterly">"Kvartal"</option>
                    <option class=SELECT_OPTION value="yearly">"Helår"</option>
                    <option class=SELECT_OPTION value="not_registered">"Ej momsregistrerad"</option>
                </Select>
                {move || loaded.get().filter(|(_, _, r)| r.locked).map(|_| view! {
                    <p class="mt-2 text-xs/relaxed text-muted-foreground">"Perioden kan inte ändras när en deklaration för året är inlämnad."</p>
                })}
            </Card>
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Period"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL>"Deklarationsdag"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Att betala/få tillbaka"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {move || loaded.get().map(|(_, _, r)| r.periods.into_iter().map(|p| {
                        let status = p.status();
                        let variant = if status == vpb::VatStatus::Changed { BadgeVariant::Destructive } else if status == vpb::VatStatus::Submitted { BadgeVariant::Secondary } else { BadgeVariant::Outline };
                        view! {
                            <tr class=TABLE_ROW>
                                <td class=TABLE_CELL><a class="underline-offset-4 hover:underline" href=format!("/vat/{}", p.period)>{p.label.clone()}</a></td>
                                <td class=TABLE_CELL><Badge variant=variant>{vat_status_label(status)}</Badge></td>
                                <td class=TABLE_CELL>{if p.due_date.is_empty() { "Se Skatteverket".to_owned() } else { p.due_date.clone() }}</td>
                                <td class=TABLE_AMOUNT_CELL>{whole_kronor(p.vat_due * 100)}</td>
                            </tr>
                        }
                    }).collect_view())}
                </tbody>
            </Table></TableCard>
        </div>
    }
}
```
The select is disabled while the year is locked. Give `ui::Select` an optional prop for it:
```rust
// in `Select`'s props:
    #[prop(optional, into)] disabled: Signal<bool>,
// on its <select>:
                    prop:disabled=move || disabled.get()
```
and pass `disabled=Signal::derive(move || loaded.get().is_some_and(|(_, _, r)| r.locked))` from the Moms page. Existing callers are unchanged (the prop defaults to `false`).

- [ ] **Step 5: Build the declaration page `/vat/:period`**

`crates/web/src/pages/vat_return.rs`:
```rust
//! Momsdeklaration for one period, drawn as SKV 4700: the boxes with the
//! accounts behind them, the file for Skatteverket and marking it submitted.

use crate::active_company::Companies;
use crate::api::{vat_api, vpb};
use crate::attachments::save_as;
use crate::errors::describe;
use crate::format::amount;
use crate::pages::vat::{box_amount, vat_status_label};
use crate::task::spawn_local;
use crate::ui::{Badge, Button, ErrorAlert, PageHeader, Variant};
use crate::vat_form::{SECTIONS, Section};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn VatReturnPage() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let period = use_params_map().read_untracked().get("period").unwrap_or_default();
    let period = StoredValue::new(period);
    let error = RwSignal::new(None::<String>);
    let declaration = RwSignal::new(None::<(String, vpb::VatReturn)>);
    let downloaded = RwSignal::new(None::<String>);
    let confirming = RwSignal::new(false);
    let marking = RwSignal::new(false);
    let booked = RwSignal::new(None::<String>);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = vat_api().get_vat_return(vpb::VatReturnRef { company_id: company_id.clone(), period: period.get_value() }).await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(r) => declaration.set(Some((company_id, r.into_inner()))),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        declaration.set(None);
        error.set(None);
        load();
    });

    let download = move |_| {
        error.set(None);
        let Some((company_id, _)) = declaration.get_untracked() else { return };
        spawn_local(async move {
            match vat_api().export_vat_file(vpb::VatReturnRef { company_id, period: period.get_value() }).await {
                Ok(file) => {
                    let file = file.into_inner();
                    save_as(&file.file_name, "application/xml", &file.content);
                    downloaded.set(Some(file.fingerprint));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let submit = move |_| {
        if marking.get_untracked() {
            return;
        }
        let Some((company_id, shown)) = declaration.get_untracked() else { return };
        marking.set(true);
        error.set(None);
        let fingerprint = downloaded.get_untracked().unwrap_or(shown.fingerprint);
        spawn_local(async move {
            let result = vat_api()
                .mark_vat_return_submitted(vpb::MarkVatReturnSubmittedRequest { company_id, period: period.get_value(), fingerprint })
                .await;
            marking.set(false);
            match result {
                Ok(r) => {
                    let r = r.into_inner();
                    confirming.set(false);
                    booked.set(Some(if r.voucher_number == 0 {
                        "Inlämnad. Det fanns inget att bokföra.".into()
                    } else {
                        format!("Inlämnad. Momsavräkningen bokfördes som verifikation {}.", r.voucher_number)
                    }));
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <div class="grid gap-6">
            {move || {
                let title = declaration.get().and_then(|(_, d)| d.summary).map_or("Momsdeklaration".to_owned(), |s| format!("Momsdeklaration {}", s.label));
                view! {
                    <PageHeader title=title>
                        <Button variant=Variant::Outline kind="button" on:click=download>"Ladda ner fil"</Button>
                        <Button kind="button" on:click=move |_| confirming.set(true)>"Markera inlämnad…"</Button>
                    </PageHeader>
                }
            }}
            <ErrorAlert message=error />
            {move || booked.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Show when=move || confirming.get()>
                {move || declaration.get().and_then(|(_, d)| d.summary).map(|s| view! {
                    <div class="flex flex-wrap items-center gap-2 text-xs/relaxed">
                        <span>{format!("Markera som inlämnad när filen är uppladdad hos Skatteverket. Doris bokför momsavräkningen daterad {}.", s.end)}</span>
                        <Button kind="button" disabled=Signal::derive(move || marking.get()) on:click=submit>"Bekräfta"</Button>
                        <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(false)>"Avbryt"</Button>
                    </div>
                })}
            </Show>
            {move || declaration.get().map(|(_, d)| view! { <Declaration declaration=d /> })}
        </div>
    }
}

#[component]
fn Declaration(declaration: vpb::VatReturn) -> impl IntoView {
    let summary = declaration.summary.clone().unwrap_or_default();
    let status = summary.status();
    let rounding = summary.vat_due * 100 - declaration.booked_vat;
    let boxes = StoredValue::new(declaration.boxes.clone());
    let column = move |right: bool| {
        SECTIONS.iter().filter(move |s| s.right == right).map(move |s| view! {
            <FormSection section=s boxes=boxes vat_due=summary.vat_due />
        }).collect_view()
    };
    view! {
        <div class="grid gap-3">
            <div class="flex flex-wrap items-center gap-2 text-xs/relaxed">
                <Badge>{vat_status_label(status)}</Badge>
                <span class="text-muted-foreground">{format!("Organisationsnummer {} · Momsregistreringsnummer {} · Deklarationsdag {} · Period i filen {}",
                    declaration.org_nr, declaration.vat_number,
                    if summary.due_date.is_empty() { "se Skatteverket".to_owned() } else { summary.due_date.clone() },
                    summary.period)}</span>
            </div>
            <p class="text-xs/relaxed text-muted-foreground">"Ange endast kronor, ej ören."</p>
            <div class="grid gap-3 md:grid-cols-2">
                <div class="grid content-start gap-3">{column(false)}</div>
                <div class="grid content-start gap-3">
                    {column(true)}
                    <p class="text-xs/relaxed text-muted-foreground">
                        {format!("Bokförd moms {} · Öresavrundning (3740) {}", amount(declaration.booked_vat), amount(rounding))}
                    </p>
                </div>
            </div>
            {(!declaration.submissions.is_empty()).then(|| view! {
                <section class="grid gap-1 text-xs/relaxed">
                    <h2 class="text-sm font-medium">"Inlämningar"</h2>
                    {declaration.submissions.iter().map(|s| {
                        let voucher = if s.voucher_number == 0 { "ingen verifikation".to_owned() } else { format!("ver. {}", s.voucher_number) };
                        let corrected = if s.corrected { " · Rättad" } else { "" };
                        view! { <p>{format!("{} · {} · {voucher}{corrected}", s.submitted_at, s.submitted_by_name)}</p> }
                    }).collect_view()}
                </section>
            })}
        </div>
    }
}

#[component]
fn FormSection(section: &'static Section, boxes: StoredValue<Vec<vpb::VatBoxAmount>>, vat_due: i64) -> impl IntoView {
    view! {
        <section class="overflow-hidden rounded-lg bg-card ring-1 ring-foreground/10">
            <h2 class="border-b bg-muted px-3 py-2 text-xs font-medium">{format!("{}. {}", section.letter, section.title)}</h2>
            {section.rows.iter().map(|row| {
                let n = row.vat_box;
                let found = boxes.with_value(|b| b.iter().find(|b| b.r#box == n).cloned());
                let kr = if n == 49 { vat_due } else { found.as_ref().map_or(0, |b| b.amount) };
                let open = RwSignal::new(false);
                let accounts = found.map(|b| b.accounts).unwrap_or_default();
                let has_accounts = !accounts.is_empty();
                let value_class = if n == 49 { "rounded-md px-2 py-0.5 text-right tabular-nums ring-1 ring-primary" } else { "rounded-md px-2 py-0.5 text-right tabular-nums ring-1 ring-input" };
                view! {
                    <div class="border-t first:border-t-0">
                        <button type="button" class="grid w-full grid-cols-[1fr_2rem_7rem] items-center gap-2 px-3 py-1.5 text-left text-xs/relaxed aria-expanded:bg-accent"
                            aria-expanded=move || open.get().to_string()
                            disabled=!has_accounts
                            on:click=move |_| open.update(|o| *o = !*o)>
                            <span>{row.label}</span>
                            <span class="text-right font-medium text-muted-foreground">{format!("{n:02}")}</span>
                            <span class=value_class>{box_amount(kr, n)}</span>
                        </button>
                        <Show when=move || open.get()>
                            <div class="grid gap-0.5 bg-accent px-6 pb-2 text-xs/relaxed text-muted-foreground">
                                {accounts.iter().map(|a| view! {
                                    <div class="flex justify-between gap-4"><span>{format!("{} {}", a.number, a.name)}</span><span class="tabular-nums">{amount(a.amount)}</span></div>
                                }).collect_view()}
                            </div>
                        </Show>
                    </div>
                }
            }).collect_view()}
        </section>
    }
}
```
Only tokens are used (`bg-card`, `bg-muted`, `bg-accent`, `ring-primary`, `ring-input`, `text-muted-foreground`), so light and dark follow `input.css`.

The two actions only make sense for a period to submit or a changed one. In `VatReturnPage`, replace the two buttons inside `PageHeader` with:
```rust
                        <Show when=move || declaration.get().and_then(|(_, d)| d.summary).is_some_and(|s| matches!(s.status(), vpb::VatStatus::ToSubmit | vpb::VatStatus::Changed))>
                            <Button variant=Variant::Outline kind="button" on:click=download>"Ladda ner fil"</Button>
                            <Button kind="button" on:click=move |_| confirming.set(true)>"Markera inlämnad…"</Button>
                        </Show>
```

- [ ] **Step 6: Routes, menu and route tests**

`crates/web/src/pages/mod.rs`: `pub mod vat; pub mod vat_return;` (and the re-exports the file uses for other pages). `crates/web/src/app.rs`, after the `/fiscal-years` route:
```rust
                        <Route path=path!("/vat") view=|| view! { <SignedIn><Vat /></SignedIn> } />
                        <Route path=path!("/vat/:period") view=|| view! { <SignedIn><VatReturnPage /></SignedIn> } />
```
`crates/web/src/nav.rs`: in `SECTIONS`, `("/vat", Section::Bookkeeping),`; in the Bokföring menu after Räkenskapsår: `<NavItem href="/vat" icon=IconName::Landmark label="Moms" />`.
`e2e/tests/design.spec.ts` (`const paths = [`) and `e2e/tests/leaving.spec.ts` (`const pages = [`): add `"/vat", "/vat/202603",`. In `e2e/tests/fixtures.ts`, add `Moms: "Bokföring",` to `MENU_OF` so `goTo(page, "Moms")` works.

- [ ] **Step 7: Run tests and lints**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS (the route tests in `app.rs` see both new routes in both specs and in a menu group).

- [ ] **Step 8: Commit**

```bash
git add crates/web e2e/tests/design.spec.ts e2e/tests/leaving.spec.ts e2e/tests/fixtures.ts
git commit -m "Add the Moms pages: periods per year and the declaration as SKV 4700"
```

---

### Task 15: Att göra och Räkenskapsår

**Files:**
- Modify: `crates/web/src/overview.rs` (`TodoInput.vat_periods`, rule), `crates/web/src/pages/home.rs` (load `ListVatReturns` for the shown year), `crates/web/src/pages/fiscal_years.rs` (warning)
- Test: `crates/web/src/overview.rs` tests

**Interfaces:**
- Consumes: `vpb::VatPeriodSummary`.
- Produces: `TodoInput { …, vat_periods: &'a [vpb::VatPeriodSummary] }`.

- [ ] **Step 1: Write the failing tests**

In `overview.rs` tests: add `vat_periods: &[],` to `empty()`, and:
```rust
    fn vat(period: &str, label: &str, status: vpb::VatStatus, due: &str) -> vpb::VatPeriodSummary {
        vpb::VatPeriodSummary { period: period.into(), label: label.into(), status: status as i32, due_date: due.into(), ..Default::default() }
    }

    #[test]
    fn vat_periods_to_submit_and_changed_ones_are_listed_with_their_due_date() {
        let periods = [
            vat("202606", "april–juni 2026", vpb::VatStatus::Submitted, "2026-08-17"),
            vat("202609", "juli–september 2026", vpb::VatStatus::ToSubmit, "2026-11-12"),
            vat("202603", "januari–mars 2026", vpb::VatStatus::Changed, "2026-05-12"),
            vat("202612", "oktober–december 2026", vpb::VatStatus::InProgress, "2027-02-12"),
        ];
        let list = todos(TodoInput { vat_periods: &periods, ..empty() });
        assert_eq!(
            list,
            [
                Todo {
                    urgent: true,
                    title: "Momsdeklarationen för januari–mars 2026 är ändrad".into(),
                    detail: "Lämna en ny deklaration för perioden".into(),
                    action: "Öppna deklarationen",
                    href: "/vat/202603".into(),
                },
                Todo {
                    urgent: false,
                    title: "Momsdeklarationen för juli–september 2026 ska lämnas".into(),
                    detail: "Senast 2026-11-12".into(),
                    action: "Öppna deklarationen",
                    href: "/vat/202609".into(),
                },
            ]
        );
    }

    #[test]
    fn an_overdue_vat_period_is_urgent() {
        let periods = [vat("202606", "april–juni 2026", vpb::VatStatus::ToSubmit, "2026-08-17")];
        let list = todos(TodoInput { vat_periods: &periods, ..empty() });
        assert!(list[0].urgent);
    }
```
(`todos` uses the module's `TODAY`, "2026-10-04": after 2026-08-17, before 2026-11-12.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p doris-web vat_periods`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`TodoInput` gains `pub vat_periods: &'a [vpb::VatPeriodSummary],` (import `vpb` from `crate::api`). At the end of `todo_list`, before `list`:
```rust
    // A changed declaration first, then each period to submit, oldest first.
    let mut vat: Vec<&vpb::VatPeriodSummary> = input
        .vat_periods
        .iter()
        .filter(|p| matches!(p.status(), vpb::VatStatus::ToSubmit | vpb::VatStatus::Changed))
        .collect();
    vat.sort_by_key(|p| (p.status() != vpb::VatStatus::Changed, p.period.clone()));
    for p in vat {
        let changed = p.status() == vpb::VatStatus::Changed;
        let late = !p.due_date.is_empty() && p.due_date.as_str() < today;
        list.push(Todo {
            urgent: changed || late,
            title: if changed {
                format!("Momsdeklarationen för {} är ändrad", p.label)
            } else {
                format!("Momsdeklarationen för {} ska lämnas", p.label)
            },
            detail: match (changed, p.due_date.is_empty()) {
                (true, _) => "Lämna en ny deklaration för perioden".into(),
                (false, true) => "Se Skatteverket för deklarationsdagen".into(),
                (false, false) => format!("Senast {}", p.due_date),
            },
            action: "Öppna deklarationen",
            href: format!("/vat/{}", p.period),
        });
    }
```
In `pages/home.rs` (import `vat_api`, `vpb` from `crate::api`):
- next to `agi_months`: `let vat_periods: Loaded<Vec<vpb::VatPeriodSummary>> = RwSignal::new(None);`, and `vat_periods.set(None);` in the per-company effect's reset list;
- in the per-year effect, next to `balance.set(None); vouchers.set(None);` add `vat_periods.set(None);`, and before the last `spawn_local` (the one that moves `company_id` and `start`) add:
```rust
        spawn_local({
            let (company_id, start, stale) = (company_id.clone(), start.clone(), stale.clone());
            async move {
                let listed = vat_api()
                    .list_vat_returns(vpb::ListVatReturnsRequest { company_id, fiscal_year_start: start })
                    .await;
                if !stale() {
                    vat_periods.set(Some(
                        listed.map(|r| r.into_inner().periods).map_err(|s| describe(&s)),
                    ));
                }
            }
        });
```
- in `todos`, read it with the others:
```rust
        let (s, c, r, m, v) = (
            supplier_invoices.get()?,
            customer_invoices.get()?,
            payroll_runs.get()?,
            agi_months.get()?,
            vat_periods.get()?,
        );
        Some((|| {
            let (s, c, r, m, v) = (s?, c?, r?, m?, v?);
            Ok::<_, String>(todo_list(
                &TodoInput {
                    supplier_invoices: &s,
                    customer_invoices: &c,
                    payroll_runs: &r,
                    agi_months: &m,
                    vat_periods: &v,
                },
                &today(),
            ))
        })())
```
Update the comment above it ("needs all five lists").

In `pages/fiscal_years.rs` (import `vat_api`, `vpb`), next to `unpaid_under_cash`:
```rust
    // Declarations to submit, or changed, in any listed year.
    let vat_open = RwSignal::new(false);
    Effect::new(move |_| {
        let list = years.get();
        let company_id = companies.active.get_untracked();
        vat_open.set(false);
        if company_id.is_empty() || list.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut open = false;
            for year in &list {
                let request = vpb::ListVatReturnsRequest { company_id: company_id.clone(), fiscal_year_start: year.start.clone() };
                if let Ok(response) = vat_api().list_vat_returns(request).await {
                    open |= response.into_inner().periods.iter().any(|p| {
                        matches!(p.status(), vpb::VatStatus::ToSubmit | vpb::VatStatus::Changed)
                    });
                }
            }
            if company_id == companies.active.get_untracked() {
                vat_open.set(open);
            }
        });
    });
```
and under the kontantmetoden note:
```rust
            {move || vat_open.get().then(|| view! {
                <p class="text-xs/relaxed text-muted-foreground">
                    "Det finns momsdeklarationer som inte är inlämnade eller som har ändrats. Lämna dem innan räkenskapsåret stängs."
                </p>
            })}
```

- [ ] **Step 4: Run tests and lints**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Show VAT declarations to submit under Att göra and on Räkenskapsår"
```

---

### Task 16: E2E och dokumentation

**Files:**
- Create: `e2e/tests/vat.spec.ts`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write the e2e test**

`e2e/tests/vat.spec.ts`:
```ts
import type { Page } from "@playwright/test";
import { addCompany, expect, goTo, register, test } from "./fixtures";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Last year, so the fourth quarter has ended whatever day the test runs.
const last = new Date().getFullYear() - 1;

async function voucher(page: Page, app: string, date: string, text: string, lines: [string, string, string][]) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill(text);
  // The editor starts with two rows.
  for (let i = 2; i < lines.length; i++) await page.getByRole("button", { name: "Lägg till rad" }).click();
  for (const [i, [account, debit, credit]] of lines.entries()) {
    await page.getByLabel(`Konto, rad ${i + 1}`).fill(account);
    if (debit) await page.getByLabel(`Debet, rad ${i + 1}`).fill(debit);
    if (credit) await page.getByLabel(`Kredit, rad ${i + 1}`).fill(credit);
  }
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("a quarter is declared, downloaded, submitted and changed", async ({ page, app }, testInfo) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${last}-01-01`);
  await voucher(page, app, `${last}-11-10`, "Försäljning", [["1930", "1250", ""], ["3001", "", "1000"], ["2611", "", "250"]]);
  await voucher(page, app, `${last}-12-05`, "Inköp", [["4010", "320", ""], ["2640", "80", ""], ["1930", "", "400"]]);

  await goTo(page, "Moms");
  await expect(page.getByRole("heading", { level: 1, name: "Moms" })).toBeVisible();
  await page.goto(`${app}/vat?fy=${last}-01-01`);
  const q4 = page.getByRole("row", { name: new RegExp(`oktober–december ${last}`) });
  await expect(q4).toContainText("Att lämna");
  await expect(q4).toContainText("170");
  await q4.getByRole("link").click();
  await expect(page.getByRole("heading", { level: 1, name: `Momsdeklaration oktober–december ${last}` })).toBeVisible();
  await expect(page.getByRole("button", { name: /Momspliktig försäljning som inte ingår.*05.*1\s000/ })).toBeVisible();
  await page.getByRole("button", { name: /Ingående moms att dra av/ }).click();
  await expect(page.getByText("2640 Ingående moms")).toBeVisible();

  const [file] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "Ladda ner fil" }).click()]);
  expect(file.suggestedFilename()).toBe(`moms_5560160680_${last}12.xml`);
  const path = join(testInfo.outputDir, file.suggestedFilename());
  await file.saveAs(path);
  const xml = readFileSync(path, "utf8");
  expect(xml).toContain("<MomsUtgHog>250</MomsUtgHog>");
  expect(xml).toContain("<MomsIngAvdr>80</MomsIngAvdr>");
  expect(xml).toContain("<MomsBetala>170</MomsBetala>");

  await page.getByRole("button", { name: "Markera inlämnad…" }).click();
  await page.getByRole("button", { name: "Bekräfta" }).dblclick();
  await expect(page.getByRole("status")).toContainText("verifikation 3");
  await expect(page.getByText("Perioden är redan inlämnad och har inte ändrats.")).toHaveCount(0);

  await page.goto(`${app}/vouchers?fy=${last}-01-01`);
  await expect(page.getByText(`Momsavräkning oktober–december ${last}`)).toBeVisible();

  await voucher(page, app, `${last}-12-20`, "Sen försäljning", [["1930", "125", ""], ["3001", "", "100"], ["2611", "", "25"]]);
  await page.goto(`${app}/vat?fy=${last}-01-01`);
  await expect(page.getByRole("row", { name: new RegExp(`oktober–december ${last}`) })).toContainText("Ändrad");
});

test("an account's momsruta changes the declaration", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${last}-01-01`);
  await voucher(page, app, `${last}-11-10`, "Momsfri försäljning", [["1930", "500", ""], ["3004", "", "500"]]);
  await goTo(page, "Kontoplan");
  await page.getByLabel("Momsruta för 3004").selectOption("5");
  await page.goto(`${app}/vat/${last}12`);
  await expect(page.getByRole("button", { name: /Momspliktig försäljning som inte ingår.*05.*500/ })).toBeVisible();
});
```
The labels ("Datum", "Text", "Konto, rad N", "Debet, rad N", "Kredit, rad N", "Lägg till rad", "Bokför") are those of `pages/new_voucher.rs` and `voucher_lines.rs`, as `e2e/tests/ledger.spec.ts` uses them; Ny verifikation shows a `role="status"` line once booked.

- [ ] **Step 2: Run the e2e suite**

Run: `make e2e`
Expected: PASS, including `design.spec.ts` and `leaving.spec.ts` for `/vat` and `/vat/202603`. Fix any selector that doesn't match the real markup.

- [ ] **Step 3: Update AGENTS.md**

- Layout: `crates/vat           doris-vat: redovisningsperiod, momsdeklaration and the momsavräkning`.
- Event sourcing rules, a new bullet:
  "Moms (`vat-{company_id}`, crate `doris-vat`): `VatPeriodSet` per räkenskapsår (månad, kvartal (default), helår, ej momsregistrerad; calendar months and quarters, a period belongs to the year its last month is in) and `VatReturnSubmitted` with what was declared. An account's box is `AccountVatBoxSet` in the chart (BAS default in `crates/ledger/src/vat_box.rs`, read from the chart's events). The boxes come from `doris_ledger::vat_box_totals_in`, leaving out Doris' own settlement vouchers and their corrections; öre are struck off per box and box 49 is computed from the rounded boxes. Marking a period submitted books the momsavräkning (VAT accounts to 2650, öre to 3740, dated the period's last day) with `record_voucher_in` in the same transaction; a later submission books only the difference, and a corrected settlement counts as not booked. `doris-vat` has no projections: it reads its own stream."
- API, a new bullet: "`VatService` (`proto/doris/vat/v1/vat.proto`; codes mapped in `crates/server/src/vat.rs`): `SetVatPeriod`, `ListVatReturns`, `GetVatReturn`, `ExportVatFile` (eSKD 6.0, ISO-8859-1) and `MarkVatReturnSubmitted` (with the fingerprint). Codes: `invalid_vat_period`, `vat_period_not_ended`, `vat_period_locked`, `vat_return_outdated`, `vat_return_unchanged` and `vat_not_registered`; `LedgerService.SetAccountVatBox` answers `invalid_vat_box`. Ledger refusals keep their codes."
- Update the proto list in the API section to include `proto/doris/vat/v1/vat.proto` (and fix the duplicated line there).
- Frontend: "`src/vat_form.rs` holds SKV 4700's sections and row texts; `/vat` and `/vat/{ÅÅÅÅMM}` draw them."
- Menu list in `src/nav.rs` bullet: Bokföring now also has Moms.

- [ ] **Step 4: Full check**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add e2e/tests/vat.spec.ts AGENTS.md
git commit -m "Test the momsdeklaration end to end and document it"
```
