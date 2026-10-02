# Plan 7: Chart of Accounts and Vouchers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every company gets a chart of accounts (a BAS selection, seeded lazily), and its members record balanced vouchers. Each voucher gets a gap-free number per fiscal year and is corrected only by a reversing voucher. A stress test proves the numbering holds under concurrency and rollbacks.

**Architecture:**
- A new crate, `crates/ledger` (`doris-ledger`), follows the `doris-company` layout:
  - `domain.rs` holds the pure rules.
  - `lib.rs` holds the commands, each running in one `BEGIN IMMEDIATE` transaction.
  - `projections.rs` and `queries.rs` hold the read side.
- There are two stream kinds. `accounts-{company_id}` holds the chart, and `ledger-{company_id}-{fy_start}` holds one fiscal year's vouchers.
- The voucher number is `last_number + 1`, decided inside the write transaction. A `vouchers` projection with a primary key plus a gap-refusing trigger backs the domain logic up.
- The server exposes `doris.ledger.v1.LedgerService`. The web app gets three pages, `/accounts`, `/vouchers` and `/vouchers/new`, and follows active-company changes made in other tabs.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, tonic 0.14 gRPC-Web, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-02-kontoplan-verifikationer-design.md`

## Global Constraints
- Amounts are `i64` öre everywhere: domain, event JSON, SQL and proto (`int64`). One line holds at most 10¹³ öre.
- Account numbers are 1000–8999. Account names are 1–100 chars after trim. Voucher text is 1–200 chars after trim. A voucher has 2–100 lines.
- The client never sends a voucher number. The server decides it inside the write transaction.
- Events are never updated or deleted. A correction is a new `VoucherRecorded` with `corrects: Some(n)` and the text `Rättelse av ver {n}`.
- The `ChartSeeded` payload carries the full account list, not a reference to a BAS version.
- `ledger-` and `accounts-` projections have **no** foreign key to `companies`, because each crate rebuilds its own tables independently (see `migrations/0005_companies.sql`).
- Error codes are exactly the spec's list, plus `invalid_date` for an unparseable date string:
  - `invalid_account_number`, `invalid_account_name`, `account_exists`, `account_not_found`, `account_inactive`
  - `invalid_voucher_text`, `invalid_voucher_lines`, `invalid_amount`, `voucher_unbalanced`
  - `voucher_date_in_future`, `voucher_date_before_first_fiscal_year`, `correction_date_outside_fiscal_year`
  - `voucher_not_found`, `already_corrected`, `cannot_correct_correction`
  - `company_not_found`, `invalid_date`
- "Today" on the server is the UTC date, as `GetCompany` already does (the existing `ponytail:` note applies). Domain and library functions take `today: Date` as a parameter. They never read the clock.
- User-visible text is Swedish and must match exactly:
  - "Verifikationer", "Kontoplan", "Ny verifikation", "Lägg till konto", "Byt namn", "Spara", "Inaktivera", "Aktivera", "Visa inaktiva"
  - "Räkenskapsår", "Rätta", "Bekräfta rättelse", "Rättad av ver {n}", "Rättelse av ver {n}", "Verifikation {n} bokförd"
  - "Lägg till rad", "Ta bort", "Bokför", "Datum", "Text", "Konto", "Debet", "Kredit", "Differens", "Nummer", "Namn", "Status", "Aktivt", "Inaktivt", "Nr", "Belopp"
- The UI follows shadcn preset b1Gdz9bFY. Table classes are copied from shadcn's generated `table` component (Task 7). Invent no classes except layout utilities.
- No new dependencies, except `tempfile` (already in the workspace) as a dev-dependency of `doris-ledger` and the `StorageEvent` feature of `web-sys`.
- Lints: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- The wasm budget rises from 800 000 to **900 000 bytes** in Task 7 (decided by the user on 2026-10-02: it was 786 184 bytes before this plan; code splitting needs cargo-leptos and is a separate step). Measure before Task 7 and after Tasks 8 and 9. If it goes over 900 000, stop and report; don't raise it further.
- TDD: a failing test first for every behavior, and a commit per task. Commit messages are English and end with the attribution lines from the session.

## Review Focus
1. **Amounts typed the Swedish way**, for example `1 234,50`, `1 234,50` with a no-break space, `1234.5`, `12`, or a stray `-` or three decimals. These must become the right öre or be refused, never silently become something else. *Test: Task 7, `parse_amount_*` tests.*
2. **A voucher dated on a fiscal-year boundary** (31 Dec / 1 Jan) must land in the right year and stream, and a date after today must be refused even inside the current year. *Test: Task 2, `vouchers_on_fiscal_year_boundaries_land_in_the_right_year`.*
3. **Correcting a voucher from a past fiscal year:** its last day is allowed and the next day is refused. *Test: Task 2, `a_correction_must_be_dated_inside_the_original_fiscal_year`.*
4. **Two tabs, different active company:** after switching in one tab, the other must follow before anything is booked. *Test: Task 8 e2e `another tab follows the active company`.*
5. **An account typed as `1930 Företagskonto` (the datalist value), `1930`, or junk.** The first two must book on 1930, and junk must reach the server as an unknown account and show its Swedish error. *Test: Task 9, `account_number_takes_the_leading_digits`.*

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Adds the workspace member `crates/ledger` and the dependency `doris-ledger`. |
| `crates/ledger/Cargo.toml` | The new crate's manifest. |
| `crates/ledger/src/bas.rs` | The BAS account selection, as a constant list. |
| `crates/ledger/src/domain.rs` | Pure rules: account and voucher value types, events, `Chart`, `Ledger`, the decide functions. |
| `crates/ledger/src/lib.rs` | Errors, stream names, transactional commands (`*_in` variants take the caller's connection). |
| `crates/ledger/src/projections.rs` | Projects events into `accounts`, `vouchers` and `voucher_lines`, and rebuilds them. |
| `crates/ledger/src/queries.rs` | `list_accounts`, `list_fiscal_years`, `list_vouchers`. |
| `crates/ledger/tests/{domain,store,stress}.rs` | Domain, store and stress tests. |
| `migrations/0006_ledger.sql` | Projection tables and the gap-refusing trigger. |
| `crates/company/src/lib.rs` | Adds `get_company_in(conn, …)`. |
| `proto/doris/ledger/v1/ledger.proto` | `LedgerService`. |
| `crates/proto/{build.rs,src/lib.rs}` | Generates and exposes `doris_proto::ledger::v1`. |
| `crates/server/src/ledger.rs` | `LedgerApi` and the error mapping. |
| `crates/server/src/{lib.rs,main.rs,grpc.rs,company.rs}` | Router wiring and a shared `today()`. |
| `crates/server/tests/{ledger.rs,common/mod.rs}` | Integration tests and the ledger client. |
| `crates/web/src/{api.rs,errors.rs,format.rs,ui.rs}` | Ledger client, error texts, amounts, `Table`. |
| `crates/web/src/active_company.rs` | Follows the active company when another tab changes it. |
| `crates/web/src/pages/{accounts,vouchers,new_voucher}.rs` | The three pages. |
| `crates/web/src/{app.rs,pages/mod.rs}` | Routes and header links. |
| `e2e/tests/{fixtures.ts,companies.spec.ts,ledger.spec.ts}` | A shared `addCompany` and the ledger e2e. |
| `AGENTS.md` | Layout, API and the numbering rule. |

---

### Task 1: The `doris-ledger` crate and the chart domain

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/ledger/Cargo.toml`, `crates/ledger/src/lib.rs`, `crates/ledger/src/domain.rs`, `crates/ledger/src/bas.rs`
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces, all in `doris_ledger::domain`:
  - `DomainError`, with the 15 variants below.
  - `AccountNumber::parse(u32) -> Result<AccountNumber, DomainError>` and `.get() -> u16`.
  - `AccountName::parse(&str) -> Result<AccountName, DomainError>` and `.as_str()`.
  - `ChartAccount { number, name }`.
  - `ChartEvent` (`ChartSeeded`, `AccountAdded`, `AccountRenamed`, `AccountDeactivated`, `AccountReactivated`).
  - `Account { number, name, active }`.
  - `Chart::from_events(&[ChartEvent])`, `Chart::apply(&mut self, &ChartEvent)`, `Chart::get(AccountNumber) -> Option<&Account>`, `Chart::accounts() -> impl Iterator<Item = &Account>`.
  - `seed_chart() -> ChartEvent`.
  - `add_account(&Chart, AccountNumber, AccountName)`, `rename_account(&Chart, AccountNumber, AccountName)` and `set_account_active(&Chart, AccountNumber, bool)`, all returning `Result<Vec<ChartEvent>, DomainError>`.

- [ ] **Step 1: Create the crate skeleton**

In the root `Cargo.toml`, add `"crates/ledger"` to `members` and a workspace dependency:

```toml
members = ["crates/eventstore", "crates/identity", "crates/proto", "crates/server", "crates/web", "crates/company", "crates/ledger"]
```
```toml
doris-ledger = { path = "crates/ledger" }
```

`crates/ledger/Cargo.toml`:

```toml
[package]
name = "doris-ledger"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
doris-company.workspace = true
doris-eventstore.workspace = true
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

`crates/ledger/src/lib.rs` (grows in Task 3):

```rust
//! The chart of accounts and the vouchers (verifikationer) of a company,
//! event-sourced into SQLite.

mod bas;
pub mod domain;
```

- [ ] **Step 2: Write the failing chart tests**

`crates/ledger/tests/domain.rs`:

```rust
use doris_ledger::domain::*;

fn n(number: u32) -> AccountNumber {
    AccountNumber::parse(number).unwrap()
}

fn name(raw: &str) -> AccountName {
    AccountName::parse(raw).unwrap()
}

fn seeded() -> Chart {
    Chart::from_events(&[seed_chart()])
}

#[test]
fn account_numbers_are_four_digits_from_1000_to_8999() {
    assert_eq!(n(1000).get(), 1000);
    assert_eq!(n(8999).get(), 8999);
    for bad in [0, 999, 9000, 19300] {
        assert_eq!(AccountNumber::parse(bad), Err(DomainError::InvalidAccountNumber), "{bad}");
    }
}

#[test]
fn account_names_are_trimmed_and_1_to_100_characters() {
    assert_eq!(name("  Kassa ").as_str(), "Kassa");
    assert_eq!(name(&"å".repeat(100)).as_str().chars().count(), 100);
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(AccountName::parse(bad), Err(DomainError::InvalidAccountName));
    }
}

#[test]
fn the_seed_is_a_bas_selection_with_every_account_active() {
    let chart = seeded();
    let count = chart.accounts().count();
    assert!((150..=250).contains(&count), "{count} accounts");
    let bank = chart.get(n(1930)).unwrap();
    assert_eq!(bank.name.as_str(), "Företagskonto/checkkonto/affärskonto");
    assert!(chart.accounts().all(|a| a.active));
    for number in [1510, 2440, 2611, 2641, 2650, 3001, 4010, 5010, 6570, 7510, 8999] {
        assert!(chart.get(n(number)).is_some(), "{number} missing");
    }
}

#[test]
fn an_account_is_added_once() {
    let chart = seeded();

    let events = add_account(&chart, n(1931), name("Sparkonto")).unwrap();

    assert_eq!(
        events,
        vec![ChartEvent::AccountAdded { number: n(1931), name: name("Sparkonto") }]
    );
    assert_eq!(
        add_account(&chart, n(1930), name("Bank")),
        Err(DomainError::AccountExists)
    );
}

#[test]
fn renaming_needs_an_existing_account_and_a_new_name() {
    let chart = seeded();

    assert_eq!(
        rename_account(&chart, n(1930), name("Bank")).unwrap(),
        vec![ChartEvent::AccountRenamed { number: n(1930), name: name("Bank") }]
    );
    assert_eq!(
        rename_account(&chart, n(1930), name("Företagskonto/checkkonto/affärskonto")).unwrap(),
        vec![]
    );
    assert_eq!(
        rename_account(&chart, n(1999), name("Bank")),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn deactivating_and_reactivating_are_idempotent() {
    let mut chart = seeded();

    let off = set_account_active(&chart, n(1930), false).unwrap();
    assert_eq!(off, vec![ChartEvent::AccountDeactivated { number: n(1930) }]);
    chart.apply(&off[0]);
    assert!(!chart.get(n(1930)).unwrap().active);
    assert_eq!(set_account_active(&chart, n(1930), false).unwrap(), vec![]);

    let on = set_account_active(&chart, n(1930), true).unwrap();
    assert_eq!(on, vec![ChartEvent::AccountReactivated { number: n(1930) }]);
    chart.apply(&on[0]);
    assert!(chart.get(n(1930)).unwrap().active);
    assert_eq!(
        set_account_active(&chart, n(1999), true),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn chart_events_are_tagged_json_with_plain_numbers() {
    let json = serde_json::to_value(ChartEvent::AccountAdded {
        number: n(1931),
        name: name("Sparkonto"),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({"type": "AccountAdded", "number": 1931, "name": "Sparkonto"})
    );
}
```

Add `serde_json.workspace = true` under `[dev-dependencies]` too. It is already a normal dependency, so that is only for clarity, and you may skip it.

- [ ] **Step 2b: Run them to see them fail**

Run: `cargo test -p doris-ledger --test domain`
Expected: compile errors (`AccountNumber`, `seed_chart`, … not found).

- [ ] **Step 3: Write `bas.rs`**

`crates/ledger/src/bas.rs`. The names follow the BAS chart. BAS-kontogruppen publishes the chart for free use; the reviewer checks that the names match the current BAS chart:

```rust
//! The BAS accounts every company starts with: the ones a small Swedish
//! company commonly uses. More can be added per company.

pub(crate) const ACCOUNTS: &[(u16, &str)] = &[
    (1010, "Utvecklingsutgifter"),
    (1030, "Patent"),
    (1050, "Goodwill"),
    (1110, "Byggnader"),
    (1119, "Ackumulerade avskrivningar på byggnader"),
    (1130, "Mark"),
    (1150, "Markanläggningar"),
    (1210, "Maskiner och andra tekniska anläggningar"),
    (1219, "Ackumulerade avskrivningar på maskiner och andra tekniska anläggningar"),
    (1220, "Inventarier och verktyg"),
    (1229, "Ackumulerade avskrivningar på inventarier och verktyg"),
    (1240, "Bilar och andra transportmedel"),
    (1249, "Ackumulerade avskrivningar på bilar och andra transportmedel"),
    (1250, "Datorer"),
    (1259, "Ackumulerade avskrivningar på datorer"),
    (1310, "Andelar i koncernföretag"),
    (1350, "Andelar och värdepapper i andra företag"),
    (1380, "Andra långfristiga fordringar"),
    (1385, "Värde av kapitalförsäkring"),
    (1410, "Lager av råvaror"),
    (1460, "Lager av handelsvaror"),
    (1470, "Pågående arbeten"),
    (1480, "Förskott för varor och tjänster"),
    (1510, "Kundfordringar"),
    (1515, "Osäkra kundfordringar"),
    (1610, "Kortfristiga fordringar hos anställda"),
    (1630, "Avräkning för skatter och avgifter (skattekonto)"),
    (1640, "Skattefordringar"),
    (1650, "Momsfordran"),
    (1680, "Andra kortfristiga fordringar"),
    (1710, "Förutbetalda hyreskostnader"),
    (1720, "Förutbetalda leasingavgifter"),
    (1730, "Förutbetalda försäkringspremier"),
    (1790, "Övriga förutbetalda kostnader och upplupna intäkter"),
    (1910, "Kassa"),
    (1920, "PlusGiro"),
    (1930, "Företagskonto/checkkonto/affärskonto"),
    (1940, "Övriga bankkonton"),
    (2010, "Eget kapital"),
    (2013, "Övriga egna uttag"),
    (2017, "Årets kapitaltillskott"),
    (2018, "Övriga egna insättningar"),
    (2019, "Årets resultat"),
    (2081, "Aktiekapital"),
    (2085, "Uppskrivningsfond"),
    (2086, "Reservfond"),
    (2091, "Balanserad vinst eller förlust"),
    (2093, "Erhållna aktieägartillskott"),
    (2098, "Vinst eller förlust från föregående år"),
    (2099, "Årets resultat"),
    (2123, "Periodiseringsfond 2023"),
    (2124, "Periodiseringsfond 2024"),
    (2125, "Periodiseringsfond 2025"),
    (2126, "Periodiseringsfond 2026"),
    (2150, "Ackumulerade överavskrivningar"),
    (2220, "Avsättningar för garantier"),
    (2350, "Andra långfristiga skulder till kreditinstitut"),
    (2390, "Övriga långfristiga skulder"),
    (2393, "Lån från närstående personer, långfristig del"),
    (2410, "Andra kortfristiga låneskulder till kreditinstitut"),
    (2420, "Förskott från kunder"),
    (2440, "Leverantörsskulder"),
    (2510, "Skatteskulder"),
    (2512, "Beräknad inkomstskatt"),
    (2514, "Beräknad särskild löneskatt på pensionskostnader"),
    (2518, "Betald F-skatt"),
    (2610, "Utgående moms, 25 %"),
    (2611, "Utgående moms på försäljning inom Sverige, 25 %"),
    (2614, "Utgående moms omvänd skattskyldighet, 25 %"),
    (2615, "Utgående moms import av varor, 25 %"),
    (2620, "Utgående moms, 12 %"),
    (2621, "Utgående moms på försäljning inom Sverige, 12 %"),
    (2630, "Utgående moms, 6 %"),
    (2631, "Utgående moms på försäljning inom Sverige, 6 %"),
    (2640, "Ingående moms"),
    (2641, "Debiterad ingående moms"),
    (2645, "Beräknad ingående moms på förvärv från utlandet"),
    (2650, "Redovisningskonto för moms"),
    (2710, "Personalskatt"),
    (2730, "Lagstadgade sociala avgifter och särskild löneskatt"),
    (2731, "Avräkning lagstadgade sociala avgifter"),
    (2732, "Avräkning särskild löneskatt"),
    (2790, "Övriga löneavdrag"),
    (2820, "Kortfristiga skulder till anställda"),
    (2890, "Övriga kortfristiga skulder"),
    (2893, "Skulder till närstående personer, kortfristig del"),
    (2898, "Outtagen vinstutdelning"),
    (2910, "Upplupna löner"),
    (2920, "Upplupna semesterlöner"),
    (2940, "Upplupna lagstadgade sociala och andra avgifter"),
    (2990, "Övriga upplupna kostnader och förutbetalda intäkter"),
    (3001, "Försäljning inom Sverige, 25 % moms"),
    (3002, "Försäljning inom Sverige, 12 % moms"),
    (3003, "Försäljning inom Sverige, 6 % moms"),
    (3004, "Försäljning inom Sverige, momsfri"),
    (3105, "Försäljning varor till land utanför EU"),
    (3106, "Försäljning varor till annat EU-land, momspliktig"),
    (3108, "Försäljning varor till annat EU-land, momsfri"),
    (3305, "Försäljning tjänster till land utanför EU"),
    (3308, "Försäljning tjänster till annat EU-land"),
    (3540, "Faktureringsavgifter"),
    (3730, "Lämnade rabatter"),
    (3740, "Öres- och kronutjämning"),
    (3911, "Hyresintäkter"),
    (3960, "Valutakursvinster på fordringar och skulder av rörelsekaraktär"),
    (3970, "Vinst vid avyttring av immateriella och materiella anläggningstillgångar"),
    (3990, "Övriga ersättningar och intäkter"),
    (4010, "Inköp material och varor"),
    (4415, "Inköpta varor i Sverige, omvänd skattskyldighet, 25 % moms"),
    (4425, "Inköpta tjänster i Sverige, omvänd skattskyldighet, 25 %"),
    (4515, "Inköp av varor från annat EU-land, 25 %"),
    (4535, "Inköp av tjänster från annat EU-land, 25 %"),
    (4545, "Import av varor, 25 % moms"),
    (4598, "Justering, omvänd moms"),
    (4600, "Legoarbeten och underentreprenader"),
    (4900, "Förändring av lager"),
    (5010, "Lokalhyra"),
    (5020, "El för belysning"),
    (5060, "Städning och renhållning"),
    (5090, "Övriga lokalkostnader"),
    (5410, "Förbrukningsinventarier"),
    (5420, "Programvaror"),
    (5460, "Förbrukningsmaterial"),
    (5500, "Reparation och underhåll"),
    (5611, "Drivmedel för personbilar"),
    (5612, "Försäkring och skatt för personbilar"),
    (5613, "Reparation och underhåll av personbilar"),
    (5615, "Leasing av personbilar"),
    (5800, "Resekostnader"),
    (5810, "Biljetter"),
    (5831, "Kost och logi i Sverige"),
    (5832, "Kost och logi i utlandet"),
    (5910, "Annonsering"),
    (5930, "Reklamtrycksaker och direktreklam"),
    (6071, "Representation, avdragsgill"),
    (6072, "Representation, ej avdragsgill"),
    (6110, "Kontorsmateriel"),
    (6211, "Fast telefoni"),
    (6212, "Mobiltelefon"),
    (6230, "Datakommunikation"),
    (6250, "Postbefordran"),
    (6310, "Företagsförsäkringar"),
    (6420, "Ersättningar till revisor"),
    (6530, "Redovisningstjänster"),
    (6540, "IT-tjänster"),
    (6550, "Konsultarvoden"),
    (6570, "Bankkostnader"),
    (6590, "Övriga externa tjänster"),
    (6970, "Tidningar, tidskrifter och facklitteratur"),
    (6981, "Föreningsavgifter, avdragsgilla"),
    (6982, "Föreningsavgifter, ej avdragsgilla"),
    (6991, "Övriga externa kostnader, avdragsgilla"),
    (6992, "Övriga externa kostnader, ej avdragsgilla"),
    (7010, "Löner till kollektivanställda"),
    (7210, "Löner till tjänstemän"),
    (7220, "Löner till företagsledare"),
    (7290, "Förändring av semesterlöneskuld"),
    (7331, "Skattefria bilersättningar"),
    (7332, "Skattepliktiga bilersättningar"),
    (7385, "Kostnader för fri bil"),
    (7410, "Pensionsförsäkringspremier"),
    (7510, "Arbetsgivaravgifter 31,42 %"),
    (7519, "Sociala avgifter för semester- och löneskulder"),
    (7530, "Särskild löneskatt"),
    (7533, "Särskild löneskatt för pensionskostnader"),
    (7570, "Premier för arbetsmarknadsförsäkringar"),
    (7610, "Utbildning"),
    (7631, "Personalrepresentation, avdragsgill"),
    (7632, "Personalrepresentation, ej avdragsgill"),
    (7690, "Övriga personalkostnader"),
    (7810, "Avskrivningar på immateriella anläggningstillgångar"),
    (7820, "Avskrivningar på byggnader och markanläggningar"),
    (7832, "Avskrivningar på maskiner och andra tekniska anläggningar"),
    (7834, "Avskrivningar på bilar och andra transportmedel"),
    (7835, "Avskrivningar på datorer"),
    (7960, "Valutakursförluster på fordringar och skulder av rörelsekaraktär"),
    (7970, "Förlust vid avyttring av immateriella och materiella anläggningstillgångar"),
    (8310, "Ränteintäkter från omsättningstillgångar"),
    (8314, "Skattefria ränteintäkter"),
    (8410, "Räntekostnader för långfristiga skulder"),
    (8420, "Räntekostnader för kortfristiga skulder"),
    (8423, "Räntekostnader för skatter och avgifter"),
    (8811, "Avsättning till periodiseringsfond"),
    (8819, "Återföring från periodiseringsfond"),
    (8850, "Förändring av överavskrivningar"),
    (8910, "Skatt som belastar årets resultat"),
    (8999, "Årets resultat"),
];
```

That is 190 accounts.

- [ ] **Step 4: Write the chart half of `domain.rs`**

`crates/ledger/src/domain.rs`:

```rust
//! Pure ledger rules: the chart of accounts and vouchers. No I/O, no clock.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("account number must be 1000-8999")]
    InvalidAccountNumber,
    #[error("account name must be 1-100 characters")]
    InvalidAccountName,
    #[error("account already exists")]
    AccountExists,
    #[error("no such account")]
    AccountNotFound,
    #[error("account is inactive")]
    AccountInactive,
    #[error("voucher text must be 1-200 characters")]
    InvalidVoucherText,
    #[error("a voucher has 2-100 lines")]
    InvalidVoucherLines,
    #[error("each line has exactly one of debit and credit, at most 10^13 öre")]
    InvalidAmount,
    #[error("debit and credit differ")]
    VoucherUnbalanced,
    #[error("voucher date is in the future")]
    VoucherDateInFuture,
    #[error("voucher date is before the first fiscal year")]
    VoucherDateBeforeFirstFiscalYear,
    #[error("correction date is outside the voucher's fiscal year")]
    CorrectionDateOutsideFiscalYear,
    #[error("no such voucher")]
    VoucherNotFound,
    #[error("voucher is already corrected")]
    AlreadyCorrected,
    #[error("a correction cannot be corrected")]
    CannotCorrectCorrection,
}

/// A BAS account number: four digits, 1000-8999.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountNumber(u16);

impl AccountNumber {
    pub fn parse(raw: u32) -> Result<Self, DomainError> {
        match raw {
            1000..=8999 => Ok(Self(raw as u16)),
            _ => Err(DomainError::InvalidAccountNumber),
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountName(String);

impl AccountName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if (1..=100).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidAccountName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartAccount {
    pub number: AccountNumber,
    pub name: AccountName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChartEvent {
    /// The whole starting chart, spelled out so the history reads the same
    /// even after the built-in selection changes.
    ChartSeeded { accounts: Vec<ChartAccount> },
    AccountAdded { number: AccountNumber, name: AccountName },
    AccountRenamed { number: AccountNumber, name: AccountName },
    AccountDeactivated { number: AccountNumber },
    AccountReactivated { number: AccountNumber },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub number: AccountNumber,
    pub name: AccountName,
    pub active: bool,
}

/// A company's chart of accounts. Accounts are never removed, only
/// deactivated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    accounts: BTreeMap<AccountNumber, Account>,
}

impl Chart {
    pub fn from_events(events: &[ChartEvent]) -> Self {
        let mut chart = Self::default();
        for event in events {
            chart.apply(event);
        }
        chart
    }

    pub fn apply(&mut self, event: &ChartEvent) {
        match event.clone() {
            ChartEvent::ChartSeeded { accounts } => {
                for ChartAccount { number, name } in accounts {
                    self.accounts.insert(number, Account { number, name, active: true });
                }
            }
            ChartEvent::AccountAdded { number, name } => {
                self.accounts.insert(number, Account { number, name, active: true });
            }
            ChartEvent::AccountRenamed { number, name } => {
                if let Some(account) = self.accounts.get_mut(&number) {
                    account.name = name;
                }
            }
            ChartEvent::AccountDeactivated { number } => self.set_active(number, false),
            ChartEvent::AccountReactivated { number } => self.set_active(number, true),
        }
    }

    fn set_active(&mut self, number: AccountNumber, active: bool) {
        if let Some(account) = self.accounts.get_mut(&number) {
            account.active = active;
        }
    }

    pub fn get(&self, number: AccountNumber) -> Option<&Account> {
        self.accounts.get(&number)
    }

    /// By number.
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }
}

/// The built-in BAS selection every company starts with.
pub fn seed_chart() -> ChartEvent {
    ChartEvent::ChartSeeded {
        accounts: crate::bas::ACCOUNTS
            .iter()
            .map(|&(number, name)| ChartAccount {
                number: AccountNumber::parse(number.into()).expect("BAS numbers are valid"),
                name: AccountName::parse(name).expect("BAS names are valid"),
            })
            .collect(),
    }
}

pub fn add_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    if chart.get(number).is_some() {
        return Err(DomainError::AccountExists);
    }
    Ok(vec![ChartEvent::AccountAdded { number, name }])
}

/// Renaming to the current name yields no events.
pub fn rename_account(
    chart: &Chart,
    number: AccountNumber,
    name: AccountName,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    if account.name == name {
        return Ok(vec![]);
    }
    Ok(vec![ChartEvent::AccountRenamed { number, name }])
}

/// Idempotent: an account already in the wanted state yields no events.
pub fn set_account_active(
    chart: &Chart,
    number: AccountNumber,
    active: bool,
) -> Result<Vec<ChartEvent>, DomainError> {
    let account = chart.get(number).ok_or(DomainError::AccountNotFound)?;
    Ok(match (account.active, active) {
        (true, false) => vec![ChartEvent::AccountDeactivated { number }],
        (false, true) => vec![ChartEvent::AccountReactivated { number }],
        _ => vec![],
    })
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-ledger --test domain`
Expected: all 7 pass.

- [ ] **Step 6: Lint and commit**

```bash
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add Cargo.toml Cargo.lock crates/ledger
git commit -m "Add doris-ledger with the chart of accounts domain"
```

---

### Task 2: The voucher domain

**Files:**
- Modify: `crates/ledger/src/domain.rs`
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Consumes: `doris_company::domain::FiscalYear` (`start`, `end`, `next()`, `containing(Date)`) and the Task 1 types.
- Produces, in `doris_ledger::domain`:
  - `MAX_AMOUNT: i64 = 10_000_000_000_000`.
  - `VoucherLine { account: AccountNumber, debit: i64, credit: i64 }` and `VoucherLine::new(account: u32, debit: i64, credit: i64) -> Result<VoucherLine, DomainError>`. An invalid number gives `AccountNotFound`.
  - `LedgerEvent::VoucherRecorded { number: u32, date: Date, text: String, lines: Vec<VoucherLine>, corrects: Option<u32> }`.
  - `Voucher { number, date, text, lines, corrects: Option<u32>, corrected_by: Option<u32> }`.
  - `Ledger::new(FiscalYear)`, `Ledger::from_events(FiscalYear, &[LedgerEvent])`, `.apply(&LedgerEvent)`, `.fiscal_year`, `.last_number() -> u32`, `.voucher(u32) -> Option<&Voucher>`, `.vouchers() -> &[Voucher]`.
  - `fiscal_year_for(first: FiscalYear, date: Date, today: Date) -> Result<FiscalYear, DomainError>`.
  - `RecordVoucher { date: Date, text: String, lines: Vec<VoucherLine> }`.
  - `record_voucher(&Ledger, &Chart, RecordVoucher) -> Result<LedgerEvent, DomainError>`.
  - `correct_voucher(&Ledger, number: u32, date: Date, today: Date) -> Result<LedgerEvent, DomainError>`.

- [ ] **Step 1: Write the failing voucher tests**

Append to `crates/ledger/tests/domain.rs`:

```rust
use doris_company::domain::{FiscalYear, LegalForm};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn first_year() -> FiscalYear {
    FiscalYear::first(d("2025-01-01"), d("2025-12-31"), LegalForm::Aktiebolag).unwrap()
}

fn line(account: u32, debit: i64, credit: i64) -> VoucherLine {
    VoucherLine::new(account, debit, credit).unwrap()
}

fn sale(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Försäljning".into(),
        lines: vec![line(1930, ore, 0), line(3001, 0, ore)],
    }
}

/// Given these vouchers in the 2025 ledger, the decision on `cmd`.
fn record(given: &[LedgerEvent], chart: &Chart, cmd: RecordVoucher) -> Result<LedgerEvent, DomainError> {
    record_voucher(&Ledger::from_events(first_year(), given), chart, cmd)
}

fn number_of(event: &LedgerEvent) -> u32 {
    let LedgerEvent::VoucherRecorded { number, .. } = event;
    *number
}

#[test]
fn the_first_voucher_is_number_1_and_the_next_follows_the_last() {
    let chart = seeded();
    let first = record(&[], &chart, sale("2025-03-01", 10_000)).unwrap();
    assert_eq!(
        first,
        LedgerEvent::VoucherRecorded {
            number: 1,
            date: d("2025-03-01"),
            text: "Försäljning".into(),
            lines: vec![line(1930, 10_000, 0), line(3001, 0, 10_000)],
            corrects: None,
        }
    );

    let mut given = vec![first];
    for _ in 0..2 {
        let next = record(&given, &chart, sale("2025-03-02", 500)).unwrap();
        given.push(next);
    }
    assert_eq!(given.iter().map(number_of).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(number_of(&record(&given, &chart, sale("2025-01-01", 1)).unwrap()), 4);
}

#[test]
fn voucher_text_is_trimmed_and_1_to_200_characters() {
    let chart = seeded();
    let mut cmd = sale("2025-03-01", 100);
    cmd.text = "  Hyra mars ".into();
    let LedgerEvent::VoucherRecorded { text, .. } = record(&[], &chart, cmd).unwrap();
    assert_eq!(text, "Hyra mars");
    for bad in ["", "  ", &"x".repeat(201)] {
        let mut cmd = sale("2025-03-01", 100);
        cmd.text = bad.into();
        assert_eq!(record(&[], &chart, cmd), Err(DomainError::InvalidVoucherText));
    }
}

#[test]
fn a_voucher_has_2_to_100_lines() {
    let chart = seeded();
    let one = RecordVoucher { lines: vec![line(1930, 100, 0)], ..sale("2025-03-01", 100) };
    assert_eq!(record(&[], &chart, one), Err(DomainError::InvalidVoucherLines));

    let mut many = vec![line(1930, 99, 0)];
    many.extend((0..99).map(|_| line(3001, 0, 1)));
    let hundred = RecordVoucher { lines: many.clone(), ..sale("2025-03-01", 1) };
    assert!(record(&[], &chart, hundred).is_ok());
    many.push(line(3001, 0, 1));
    let too_many = RecordVoucher { lines: many, ..sale("2025-03-01", 1) };
    assert_eq!(record(&[], &chart, too_many), Err(DomainError::InvalidVoucherLines));
}

#[test]
fn each_line_has_exactly_one_positive_amount_within_the_limit() {
    let chart = seeded();
    for bad in [
        (0, 0),
        (100, 100),
        (-100, 0),
        (0, -100),
        (MAX_AMOUNT + 1, 0),
    ] {
        let cmd = RecordVoucher {
            lines: vec![line(1930, bad.0, bad.1), line(3001, 0, 100)],
            ..sale("2025-03-01", 100)
        };
        assert_eq!(record(&[], &chart, cmd), Err(DomainError::InvalidAmount), "{bad:?}");
    }
    let max = RecordVoucher {
        lines: vec![line(1930, MAX_AMOUNT, 0), line(3001, 0, MAX_AMOUNT)],
        ..sale("2025-03-01", 1)
    };
    assert!(record(&[], &chart, max).is_ok());
}

#[test]
fn debit_must_equal_credit() {
    let cmd = RecordVoucher {
        lines: vec![line(1930, 10_000, 0), line(3001, 0, 9_999)],
        ..sale("2025-03-01", 1)
    };
    assert_eq!(record(&[], &seeded(), cmd), Err(DomainError::VoucherUnbalanced));
}

#[test]
fn every_account_must_exist_and_be_active() {
    let mut chart = seeded();
    let unknown = RecordVoucher {
        lines: vec![line(1999, 100, 0), line(3001, 0, 100)],
        ..sale("2025-03-01", 1)
    };
    assert_eq!(record(&[], &chart, unknown), Err(DomainError::AccountNotFound));
    assert_eq!(VoucherLine::new(19300, 100, 0), Err(DomainError::AccountNotFound));

    chart.apply(&ChartEvent::AccountDeactivated { number: n(3001) });
    assert_eq!(
        record(&[], &chart, sale("2025-03-01", 100)),
        Err(DomainError::AccountInactive)
    );
}

#[test]
fn vouchers_on_fiscal_year_boundaries_land_in_the_right_year() {
    let first = first_year();
    let today = d("2026-06-15");
    assert_eq!(fiscal_year_for(first, d("2025-01-01"), today).unwrap(), first);
    assert_eq!(fiscal_year_for(first, d("2025-12-31"), today).unwrap(), first);
    assert_eq!(fiscal_year_for(first, d("2026-01-01"), today).unwrap().start, d("2026-01-01"));
    assert_eq!(fiscal_year_for(first, today, today).unwrap().end, d("2026-12-31"));
    assert_eq!(
        fiscal_year_for(first, d("2026-06-16"), today),
        Err(DomainError::VoucherDateInFuture)
    );
    assert_eq!(
        fiscal_year_for(first, d("2024-12-31"), today),
        Err(DomainError::VoucherDateBeforeFirstFiscalYear)
    );
}

#[test]
fn a_correction_reverses_every_line_and_points_at_the_original() {
    let chart = seeded();
    let original = record(&[], &chart, sale("2025-03-01", 10_000)).unwrap();
    let ledger = Ledger::from_events(first_year(), &[original]);

    let correction = correct_voucher(&ledger, 1, d("2025-03-05"), d("2026-01-10")).unwrap();

    assert_eq!(
        correction,
        LedgerEvent::VoucherRecorded {
            number: 2,
            date: d("2025-03-05"),
            text: "Rättelse av ver 1".into(),
            lines: vec![line(1930, 0, 10_000), line(3001, 10_000, 0)],
            corrects: Some(1),
        }
    );
    let after = Ledger::from_events(first_year(), &[record(&[], &chart, sale("2025-03-01", 10_000)).unwrap(), correction]);
    assert_eq!(after.voucher(1).unwrap().corrected_by, Some(2));
    assert_eq!(after.voucher(2).unwrap().corrects, Some(1));
}

#[test]
fn a_voucher_is_corrected_once_and_a_correction_never() {
    let chart = seeded();
    let original = record(&[], &chart, sale("2025-03-01", 100)).unwrap();
    let mut ledger = Ledger::from_events(first_year(), &[original]);
    let today = d("2025-06-01");
    ledger.apply(&correct_voucher(&ledger, 1, d("2025-03-02"), today).unwrap());

    assert_eq!(correct_voucher(&ledger, 1, d("2025-03-02"), today), Err(DomainError::AlreadyCorrected));
    assert_eq!(correct_voucher(&ledger, 2, d("2025-03-02"), today), Err(DomainError::CannotCorrectCorrection));
    assert_eq!(correct_voucher(&ledger, 3, d("2025-03-02"), today), Err(DomainError::VoucherNotFound));
    assert_eq!(correct_voucher(&ledger, 0, d("2025-03-02"), today), Err(DomainError::VoucherNotFound));
}

#[test]
fn a_correction_must_be_dated_inside_the_original_fiscal_year() {
    let chart = seeded();
    let ledger = Ledger::from_events(first_year(), &[record(&[], &chart, sale("2025-03-01", 100)).unwrap()]);
    let today = d("2026-02-01");

    assert!(correct_voucher(&ledger, 1, d("2025-12-31"), today).is_ok());
    assert_eq!(
        correct_voucher(&ledger, 1, d("2026-01-01"), today),
        Err(DomainError::CorrectionDateOutsideFiscalYear)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2024-12-31"), today),
        Err(DomainError::CorrectionDateOutsideFiscalYear)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2025-06-02"), d("2025-06-01")),
        Err(DomainError::VoucherDateInFuture)
    );
}

#[test]
fn a_correction_works_even_when_an_account_has_since_been_deactivated() {
    let mut chart = seeded();
    let ledger = Ledger::from_events(first_year(), &[record(&[], &chart, sale("2025-03-01", 100)).unwrap()]);
    chart.apply(&ChartEvent::AccountDeactivated { number: n(3001) });

    // correct_voucher never looks at the chart.
    assert!(correct_voucher(&ledger, 1, d("2025-03-02"), d("2025-06-01")).is_ok());
}

#[test]
fn voucher_events_are_readable_json() {
    let event = record(&[], &seeded(), sale("2025-03-01", 12_550)).unwrap();
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({
            "type": "VoucherRecorded",
            "number": 1,
            "date": "2025-03-01",
            "text": "Försäljning",
            "lines": [
                {"account": 1930, "debit": 12550, "credit": 0},
                {"account": 3001, "debit": 0, "credit": 12550}
            ],
            "corrects": null
        })
    );
}
```

Add `doris-company.workspace = true` and `jiff.workspace = true` to `[dev-dependencies]` if the test does not compile without them. They are normal dependencies already, so it should.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p doris-ledger --test domain`
Expected: compile errors (`VoucherLine`, `Ledger`, `record_voucher`, … not found).

- [ ] **Step 3: Implement the voucher half of `domain.rs`**

Add to the imports at the top of `domain.rs`:

```rust
use doris_company::domain::FiscalYear;
use jiff::civil::Date;
```

Append:

```rust
/// The most one line may carry: 100 miljarder kronor. With at most 100
/// lines, sums stay far below `i64::MAX`, so they cannot overflow.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

/// One kontering. Exactly one of `debit` and `credit` is positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoucherLine {
    pub account: AccountNumber,
    pub debit: i64,
    pub credit: i64,
}

impl VoucherLine {
    /// An out-of-range account number cannot be in any chart, so it is
    /// reported as an unknown account.
    pub fn new(account: u32, debit: i64, credit: i64) -> Result<Self, DomainError> {
        let account = AccountNumber::parse(account).map_err(|_| DomainError::AccountNotFound)?;
        Ok(Self { account, debit, credit })
    }

    fn is_valid(&self) -> bool {
        let in_range = |amount: i64| (0..=MAX_AMOUNT).contains(&amount);
        in_range(self.debit) && in_range(self.credit) && (self.debit == 0) != (self.credit == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LedgerEvent {
    /// Who recorded it, and when, is in the event metadata (BFL 5 kap. 11 §).
    VoucherRecorded {
        number: u32,
        date: Date,
        text: String,
        lines: Vec<VoucherLine>,
        corrects: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Voucher {
    pub number: u32,
    pub date: Date,
    pub text: String,
    pub lines: Vec<VoucherLine>,
    pub corrects: Option<u32>,
    pub corrected_by: Option<u32>,
}

/// One fiscal year's vouchers, in number order.
#[derive(Debug, Clone, PartialEq)]
pub struct Ledger {
    pub fiscal_year: FiscalYear,
    vouchers: Vec<Voucher>,
}

impl Ledger {
    pub fn new(fiscal_year: FiscalYear) -> Self {
        Self { fiscal_year, vouchers: Vec::new() }
    }

    pub fn from_events(fiscal_year: FiscalYear, events: &[LedgerEvent]) -> Self {
        let mut ledger = Self::new(fiscal_year);
        for event in events {
            ledger.apply(event);
        }
        ledger
    }

    pub fn apply(&mut self, event: &LedgerEvent) {
        let LedgerEvent::VoucherRecorded { number, date, text, lines, corrects } = event.clone();
        if let Some(original) = corrects.and_then(|n| self.vouchers.iter_mut().find(|v| v.number == n)) {
            original.corrected_by = Some(number);
        }
        self.vouchers.push(Voucher { number, date, text, lines, corrects, corrected_by: None });
    }

    /// 0 before the first voucher.
    pub fn last_number(&self) -> u32 {
        self.vouchers.last().map_or(0, |v| v.number)
    }

    pub fn voucher(&self, number: u32) -> Option<&Voucher> {
        self.vouchers.iter().find(|v| v.number == number)
    }

    pub fn vouchers(&self) -> &[Voucher] {
        &self.vouchers
    }
}

/// The räkenskapsår a voucher dated `date` belongs to. A voucher records
/// something that happened, so it cannot be dated after `today`.
pub fn fiscal_year_for(first: FiscalYear, date: Date, today: Date) -> Result<FiscalYear, DomainError> {
    if date > today {
        return Err(DomainError::VoucherDateInFuture);
    }
    if date < first.start {
        return Err(DomainError::VoucherDateBeforeFirstFiscalYear);
    }
    Ok(first.containing(date))
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordVoucher {
    pub date: Date,
    pub text: String,
    pub lines: Vec<VoucherLine>,
}

/// Decides a new voucher in `ledger`, the fiscal year `cmd.date` falls in
/// (see [`fiscal_year_for`]). Its number is the next one in that year.
pub fn record_voucher(ledger: &Ledger, chart: &Chart, cmd: RecordVoucher) -> Result<LedgerEvent, DomainError> {
    let text = cmd.text.trim();
    if !(1..=200).contains(&text.chars().count()) {
        return Err(DomainError::InvalidVoucherText);
    }
    if !(2..=100).contains(&cmd.lines.len()) {
        return Err(DomainError::InvalidVoucherLines);
    }
    let (mut debit, mut credit) = (0_i64, 0_i64);
    for line in &cmd.lines {
        if !line.is_valid() {
            return Err(DomainError::InvalidAmount);
        }
        let account = chart.get(line.account).ok_or(DomainError::AccountNotFound)?;
        if !account.active {
            return Err(DomainError::AccountInactive);
        }
        // Cannot overflow: at most 100 lines of at most MAX_AMOUNT.
        debit += line.debit;
        credit += line.credit;
    }
    if debit != credit {
        return Err(DomainError::VoucherUnbalanced);
    }
    Ok(LedgerEvent::VoucherRecorded {
        number: ledger.last_number() + 1,
        date: cmd.date,
        text: text.to_owned(),
        lines: cmd.lines,
        corrects: None,
    })
}

/// A rättelse (BFL 5 kap. 5 §): a new voucher in the same fiscal year with
/// debit and credit swapped on every line. It ignores whether the accounts
/// are still active, so a voucher can always be reversed.
pub fn correct_voucher(ledger: &Ledger, number: u32, date: Date, today: Date) -> Result<LedgerEvent, DomainError> {
    let original = ledger.voucher(number).ok_or(DomainError::VoucherNotFound)?;
    if original.corrects.is_some() {
        return Err(DomainError::CannotCorrectCorrection);
    }
    if original.corrected_by.is_some() {
        return Err(DomainError::AlreadyCorrected);
    }
    if date > today {
        return Err(DomainError::VoucherDateInFuture);
    }
    if date < ledger.fiscal_year.start || date > ledger.fiscal_year.end {
        return Err(DomainError::CorrectionDateOutsideFiscalYear);
    }
    Ok(LedgerEvent::VoucherRecorded {
        number: ledger.last_number() + 1,
        date,
        text: format!("Rättelse av ver {number}"),
        lines: original
            .lines
            .iter()
            .map(|l| VoucherLine { account: l.account, debit: l.credit, credit: l.debit })
            .collect(),
        corrects: Some(number),
    })
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p doris-ledger --test domain`
Expected: all pass.

- [ ] **Step 5: Lint, format and commit**

```bash
cargo fmt -p doris-ledger
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger
git commit -m "Decide vouchers and corrections with gap-free numbers"
```

---

### Task 3: Storage for the chart: migration, projections, lazy seed

**Files:**
- Create: `migrations/0006_ledger.sql`, `crates/ledger/src/projections.rs`, `crates/ledger/src/queries.rs`
- Modify: `crates/ledger/src/lib.rs`, `crates/company/src/lib.rs`
- Test: `crates/ledger/tests/store.rs`, `crates/company/tests/store.rs`

**Interfaces:**
- Consumes: Task 1 domain. From `doris-eventstore`: `begin`, `load`, `append`, `read_all`, `Metadata`, `NewEvent::from_tagged`.
- Produces:
  - `doris_company::get_company_in(conn: &mut SqliteConnection, company_id: Uuid, user_id: Uuid) -> doris_company::Result<Company>`.
  - `doris_ledger::Error` (`Domain(DomainError)`, `NotFound`, `Store(doris_eventstore::Error)`) and `doris_ledger::Result<T>`.
  - `doris_ledger::add_account(pool, company_id: Uuid, actor: Uuid, number: u32, name: &str) -> Result<()>`.
  - `doris_ledger::rename_account(pool, company_id, actor, number: u32, name: &str) -> Result<()>`.
  - `doris_ledger::set_account_active(pool, company_id, actor, number: u32, active: bool) -> Result<()>`.
  - `doris_ledger::list_accounts(pool, company_id, user_id) -> Result<Vec<domain::Account>>`.
  - `doris_ledger::rebuild_projections(pool) -> Result<()>`.
  - Internal, for Task 4: `seeded_chart(conn, company_id, actor) -> Result<Chart>`, `append(conn, stream, expected, &[E], actor)`, `member_company(conn, company_id, user) -> Result<Company>`, `ACCOUNTS_STREAM`, `LEDGER_STREAM`.

- [ ] **Step 1: Write the failing test for `get_company_in`**

Append to `crates/company/tests/store.rs`, and add `get_company_in` to its `use doris_company::{…}` list:

```rust
#[tokio::test]
async fn get_company_in_works_inside_a_write_transaction() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB"))
        .await
        .unwrap();

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let company = get_company_in(&mut tx, id, anna).await.unwrap();
    let outsider = get_company_in(&mut tx, id, bo).await;
    tx.rollback().await.unwrap();

    assert_eq!(company.id, id);
    assert!(matches!(outsider, Err(Error::NotFound)));
}
```

Run: `cargo test -p doris-company --test store get_company_in`
Expected: FAIL, `get_company_in` not found.

- [ ] **Step 2: Implement `get_company_in`**

In `crates/company/src/lib.rs`, replace `get_company` with:

```rust
/// The company, if `user_id` is a member of it.
pub async fn get_company(pool: &SqlitePool, company_id: Uuid, user_id: Uuid) -> Result<Company> {
    let mut conn = pool.acquire().await?;
    get_company_in(&mut conn, company_id, user_id).await
}

/// [`get_company`] on the caller's connection, e.g. inside another crate's
/// write transaction.
pub async fn get_company_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Company> {
    match load_company(conn, company_id).await? {
        Some((company, _)) if company.is_member(user_id) => Ok(company),
        _ => Err(Error::NotFound),
    }
}
```

Run: `cargo test -p doris-company`
Expected: all pass. Then commit:

```bash
git add crates/company
git commit -m "Load a member's company on the caller's connection"
```

- [ ] **Step 3: Write the failing chart store tests**

`crates/ledger/tests/store.rs`:

```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::DomainError;
use doris_ledger::{Error, add_account, list_accounts, rebuild_projections, rename_account, set_account_active};
use sqlx::SqlitePool;
use uuid::Uuid;

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

async fn events_of(pool: &SqlitePool, prefix: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM events WHERE stream_id LIKE ? ORDER BY global_position")
        .bind(format!("{prefix}%"))
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn table(pool: &SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn a_new_company_lists_the_bas_selection_without_writing_anything() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let accounts = list_accounts(&pool, id, anna).await.unwrap();

    assert!(accounts.len() >= 150);
    assert!(accounts.iter().any(|a| a.number.get() == 1930 && a.active));
    assert!(events_of(&pool, "accounts-").await.is_empty());
}

#[tokio::test]
async fn the_first_change_seeds_the_chart_in_the_same_transaction() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    add_account(&pool, id, anna, 1931, "Sparkonto").await.unwrap();
    rename_account(&pool, id, anna, 1931, "Sparkonto Handelsbanken").await.unwrap();
    set_account_active(&pool, id, anna, 1910, false).await.unwrap();
    set_account_active(&pool, id, anna, 1910, false).await.unwrap();

    assert_eq!(
        events_of(&pool, "accounts-").await,
        ["ChartSeeded", "AccountAdded", "AccountRenamed", "AccountDeactivated"]
    );
    let accounts = list_accounts(&pool, id, anna).await.unwrap();
    let get = |n: u16| accounts.iter().find(|a| a.number.get() == n).unwrap();
    assert_eq!(get(1931).name.as_str(), "Sparkonto Handelsbanken");
    assert!(!get(1910).active);
}

#[tokio::test]
async fn a_rejected_change_writes_nothing_not_even_the_seed() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let result = add_account(&pool, id, anna, 1930, "Bank").await;

    assert!(matches!(result, Err(Error::Domain(DomainError::AccountExists))));
    assert!(events_of(&pool, "accounts-").await.is_empty());
}

#[tokio::test]
async fn invalid_input_is_refused() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    for (result, expected) in [
        (add_account(&pool, id, anna, 999, "X").await, DomainError::InvalidAccountNumber),
        (add_account(&pool, id, anna, 1931, " ").await, DomainError::InvalidAccountName),
        (rename_account(&pool, id, anna, 1999, "X").await, DomainError::AccountNotFound),
        (set_account_active(&pool, id, anna, 1999, false).await, DomainError::AccountNotFound),
    ] {
        assert!(matches!(result, Err(Error::Domain(e)) if e == expected), "{expected:?}");
    }
}

#[tokio::test]
async fn non_members_get_not_found() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;

    assert!(matches!(list_accounts(&pool, id, bo).await, Err(Error::NotFound)));
    assert!(matches!(add_account(&pool, id, bo, 1931, "X").await, Err(Error::NotFound)));
    assert!(matches!(list_accounts(&pool, Uuid::new_v4(), anna).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn the_chart_projection_rebuilds_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    add_account(&pool, id, anna, 1931, "Sparkonto").await.unwrap();
    set_account_active(&pool, id, anna, 1910, false).await.unwrap();
    let sql = "SELECT company_id || number || name || active FROM accounts ORDER BY company_id, number";
    let before = table(&pool, sql).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(table(&pool, sql).await, before);
    assert!(!before.is_empty());
}
```

Run: `cargo test -p doris-ledger --test store`
Expected: compile errors (`add_account`, `list_accounts`, … not found).

- [ ] **Step 4: Write the migration**

`migrations/0006_ledger.sql`:

```sql
-- Projections of the accounts-* and ledger-* streams. Rebuildable from events.
-- No foreign key to companies: each crate rebuilds its own tables
-- independently.

CREATE TABLE accounts (
    company_id TEXT    NOT NULL,
    number     INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);

CREATE TABLE vouchers (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    date              TEXT    NOT NULL,
    text              TEXT    NOT NULL,
    corrects          INTEGER,
    corrected_by      INTEGER,
    recorded_at       TEXT    NOT NULL,
    recorded_by       TEXT    NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number)
);

CREATE TABLE voucher_lines (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    line_no           INTEGER NOT NULL,
    account           INTEGER NOT NULL,
    debit             INTEGER NOT NULL,
    credit            INTEGER NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number, line_no),
    FOREIGN KEY (company_id, fiscal_year_start, number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);

-- Voucher numbers run 1, 2, 3… per company and fiscal year (BFL 5 kap. 7 §).
-- The primary key refuses a duplicate; this refuses a gap. Both back up the
-- domain logic, which decides the number inside the write transaction.
CREATE TRIGGER vouchers_numbered_without_gaps BEFORE INSERT ON vouchers
WHEN NEW.number IS NOT (
    SELECT COALESCE(MAX(number), 0) + 1 FROM vouchers
    WHERE company_id = NEW.company_id AND fiscal_year_start = NEW.fiscal_year_start
)
BEGIN SELECT RAISE(ABORT, 'voucher numbers must run without gaps'); END;
```

- [ ] **Step 5: Write `lib.rs`, `projections.rs` and `queries.rs` for the chart**

`crates/ledger/src/lib.rs`:

```rust
//! The chart of accounts and the vouchers (verifikationer) of a company,
//! event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, load
//! state, decide, append, project. A voucher's number is decided inside that
//! transaction, so a write that fails or rolls back uses up no number.

mod bas;
pub mod domain;
mod projections;
mod queries;

use doris_company::domain::Company;
use doris_eventstore::{Metadata, NewEvent};
use domain::{AccountName, AccountNumber, Chart, ChartEvent, DomainError};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::list_accounts;

const ACCOUNTS_STREAM: &str = "accounts-";
const LEDGER_STREAM: &str = "ledger-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
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

pub async fn add_account(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, name: &str) -> Result<()> {
    let (number, name) = (AccountNumber::parse(number)?, AccountName::parse(name)?);
    change_chart(pool, company_id, actor, |chart| domain::add_account(chart, number, name)).await
}

pub async fn rename_account(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, name: &str) -> Result<()> {
    let (number, name) = (AccountNumber::parse(number)?, AccountName::parse(name)?);
    change_chart(pool, company_id, actor, |chart| domain::rename_account(chart, number, name)).await
}

pub async fn set_account_active(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, active: bool) -> Result<()> {
    let number = AccountNumber::parse(number)?;
    change_chart(pool, company_id, actor, |chart| domain::set_account_active(chart, number, active)).await
}

async fn change_chart(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Chart) -> Result<Vec<ChartEvent>, DomainError>,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    member_company(&mut tx, company_id, actor).await?;
    let chart = seeded_chart(&mut tx, company_id, actor).await?;
    let version = doris_eventstore::stream_version(&mut tx, &accounts_stream(company_id)).await?;
    let events = decide(&chart)?;
    append(&mut tx, &accounts_stream(company_id), version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

async fn member_company(conn: &mut SqliteConnection, company_id: Uuid, user_id: Uuid) -> Result<Company> {
    Ok(doris_company::get_company_in(conn, company_id, user_id).await?)
}

fn accounts_stream(company_id: Uuid) -> String {
    format!("{ACCOUNTS_STREAM}{company_id}")
}

/// The company's chart. A company that has none yet gets the built-in BAS
/// selection appended first, in the caller's transaction.
async fn seeded_chart(conn: &mut SqliteConnection, company_id: Uuid, actor: Uuid) -> Result<Chart> {
    let stream = accounts_stream(company_id);
    let recorded = doris_eventstore::load(conn, &stream).await?;
    if recorded.is_empty() {
        let seed = domain::seed_chart();
        append(conn, &stream, 0, std::slice::from_ref(&seed), actor).await?;
        return Ok(Chart::from_events(&[seed]));
    }
    let events = recorded
        .iter()
        .map(|e| e.decode::<ChartEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Chart::from_events(&events))
}

/// Appends events and updates projections within the caller's transaction.
async fn append<E: Serialize>(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[E],
    actor: Uuid,
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata { actor: Some(actor.to_string()) };
    let recorded = doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
```

`crates/ledger/src/projections.rs`:

```rust
//! Read models for the chart and the vouchers. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::{ChartAccount, ChartEvent};
use crate::{ACCOUNTS_STREAM, LEDGER_STREAM};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(company_id) = event.stream_id.strip_prefix(ACCOUNTS_STREAM) {
        return apply_chart(conn, company_id, event.decode()?).await;
    }
    if event.stream_id.starts_with(LEDGER_STREAM) {
        return Ok(()); // Task 4
    }
    Ok(())
}

async fn apply_chart(conn: &mut SqliteConnection, company_id: &str, event: ChartEvent) -> crate::Result<()> {
    match event {
        ChartEvent::ChartSeeded { accounts } => {
            for ChartAccount { number, name } in accounts {
                insert_account(conn, company_id, number.get(), name.as_str()).await?;
            }
        }
        ChartEvent::AccountAdded { number, name } => {
            insert_account(conn, company_id, number.get(), name.as_str()).await?;
        }
        ChartEvent::AccountRenamed { number, name } => {
            sqlx::query("UPDATE accounts SET name = ? WHERE company_id = ? AND number = ?")
                .bind(name.as_str())
                .bind(company_id)
                .bind(number.get())
                .execute(&mut *conn)
                .await?;
        }
        ChartEvent::AccountDeactivated { number } => set_active(conn, company_id, number.get(), false).await?,
        ChartEvent::AccountReactivated { number } => set_active(conn, company_id, number.get(), true).await?,
    }
    Ok(())
}

async fn insert_account(conn: &mut SqliteConnection, company_id: &str, number: u16, name: &str) -> crate::Result<()> {
    sqlx::query("INSERT INTO accounts (company_id, number, name, active) VALUES (?, ?, ?, 1)")
        .bind(company_id)
        .bind(number)
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn set_active(conn: &mut SqliteConnection, company_id: &str, number: u16, active: bool) -> crate::Result<()> {
    sqlx::query("UPDATE accounts SET active = ? WHERE company_id = ? AND number = ?")
        .bind(active)
        .bind(company_id)
        .bind(number)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Empties the ledger projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in ["DELETE FROM voucher_lines", "DELETE FROM vouchers", "DELETE FROM accounts"] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

`crates/ledger/src/queries.rs`:

```rust
//! Read-only views over the ledger projections. Each checks membership first.

use crate::Result;
use crate::domain::{Account, AccountName, AccountNumber, Chart};
use sqlx::SqlitePool;
use uuid::Uuid;

/// The company's chart, by number. Before its first change that is the
/// built-in BAS selection, which is not stored until then.
pub async fn list_accounts(pool: &SqlitePool, company_id: Uuid, user_id: Uuid) -> Result<Vec<Account>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(i64, String, bool)> =
        sqlx::query_as("SELECT number, name, active FROM accounts WHERE company_id = ? ORDER BY number")
            .bind(company_id.to_string())
            .fetch_all(pool)
            .await?;
    if rows.is_empty() {
        return Ok(Chart::from_events(&[crate::domain::seed_chart()]).accounts().cloned().collect());
    }
    Ok(rows
        .into_iter()
        .map(|(number, name, active)| Account {
            number: AccountNumber::parse(number as u32).expect("projected numbers are valid"),
            name: AccountName::parse(&name).expect("projected names are valid"),
            active,
        })
        .collect())
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-ledger`
Expected: all pass, including the Task 1 and 2 domain tests.

- [ ] **Step 7: Lint, format and commit**

```bash
cargo fmt -p doris-ledger
cargo clippy -p doris-ledger -p doris-company --all-targets -- -D warnings
git add migrations/0006_ledger.sql crates/ledger
git commit -m "Store the chart of accounts, seeded on first change"
```

---

### Task 4: Storage for vouchers: record, correct, list, rebuild, trigger

**Files:**
- Modify: `crates/ledger/src/lib.rs`, `crates/ledger/src/projections.rs`, `crates/ledger/src/queries.rs`
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: Task 2 domain and Task 3 internals (`member_company`, `seeded_chart`, `append`, `LEDGER_STREAM`).
- Produces:
  - `doris_ledger::VoucherRef { fiscal_year_start: Date, number: u32 }`, deriving `Debug, Clone, Copy, PartialEq, Eq`.
  - `record_voucher(pool, company_id, actor, cmd: RecordVoucher, today: Date) -> Result<VoucherRef>`.
  - `record_voucher_in(conn: &mut SqliteConnection, company_id, actor, cmd, today) -> Result<VoucherRef>`.
  - `correct_voucher(pool, company_id, actor, fiscal_year_start: Date, number: u32, date: Date, today: Date) -> Result<VoucherRef>`.
  - `correct_voucher_in(conn, company_id, actor, fiscal_year_start, number, date, today) -> Result<VoucherRef>`.
  - `list_fiscal_years(pool, company_id, user_id, today: Date) -> Result<Vec<FiscalYear>>`, newest first.
  - `list_vouchers(pool, company_id, user_id, fiscal_year_start: Date) -> Result<Vec<domain::Voucher>>`, by number.

- [ ] **Step 1: Write the failing voucher store tests**

Append to `crates/ledger/tests/store.rs`, extending the imports:

```rust
use doris_ledger::domain::{RecordVoucher, VoucherLine};
use doris_ledger::{VoucherRef, correct_voucher, list_fiscal_years, list_vouchers, record_voucher, record_voucher_in};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

const TODAY: &str = "2026-10-02";

fn sale(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Försäljning".into(),
        lines: vec![
            VoucherLine::new(1930, ore, 0).unwrap(),
            VoucherLine::new(3001, 0, ore).unwrap(),
        ],
    }
}

#[tokio::test]
async fn vouchers_are_numbered_per_fiscal_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    let a = record_voucher(&pool, id, anna, sale("2025-12-31", 100), today).await.unwrap();
    let b = record_voucher(&pool, id, anna, sale("2026-01-01", 100), today).await.unwrap();
    let c = record_voucher(&pool, id, anna, sale("2025-02-01", 100), today).await.unwrap();

    assert_eq!(a, VoucherRef { fiscal_year_start: d("2025-01-01"), number: 1 });
    assert_eq!(b, VoucherRef { fiscal_year_start: d("2026-01-01"), number: 1 });
    assert_eq!(c, VoucherRef { fiscal_year_start: d("2025-01-01"), number: 2 });
    let in_2025 = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!(in_2025.iter().map(|v| v.number).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(in_2025[0].date, d("2025-12-31"));
    assert_eq!(in_2025[0].lines, sale("2025-12-31", 100).lines);
}

#[tokio::test]
async fn a_rejected_voucher_uses_no_number_and_writes_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let mut unbalanced = sale("2025-03-01", 100);
    unbalanced.lines[1].credit = 99;

    let result = record_voucher(&pool, id, anna, unbalanced, today).await;
    let next = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();

    assert!(matches!(result, Err(Error::Domain(DomainError::VoucherUnbalanced))));
    assert_eq!(next.number, 1);
    assert_eq!(events_of(&pool, "ledger-").await, ["VoucherRecorded"]);
}

#[tokio::test]
async fn a_rolled_back_transaction_uses_no_number() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let abandoned = record_voucher_in(&mut tx, id, anna, sale("2025-03-01", 100), today).await.unwrap();
    tx.rollback().await.unwrap();
    let kept = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();

    assert_eq!(abandoned.number, 1);
    assert_eq!(kept.number, 1);
    assert!(events_of(&pool, "accounts-").await == ["ChartSeeded"]);
}

#[tokio::test]
async fn the_first_voucher_seeds_the_chart_it_is_checked_against() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY)).await.unwrap();

    assert_eq!(events_of(&pool, "accounts-").await, ["ChartSeeded"]);
}

#[tokio::test]
async fn a_correction_is_listed_with_both_links() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let original = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();

    let correction = correct_voucher(&pool, id, anna, original.fiscal_year_start, 1, d("2025-12-31"), today)
        .await
        .unwrap();
    let again = correct_voucher(&pool, id, anna, original.fiscal_year_start, 1, d("2025-12-31"), today).await;

    assert_eq!(correction, VoucherRef { fiscal_year_start: d("2025-01-01"), number: 2 });
    assert!(matches!(again, Err(Error::Domain(DomainError::AlreadyCorrected))));
    let listed = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!((listed[0].corrects, listed[0].corrected_by), (None, Some(2)));
    assert_eq!((listed[1].corrects, listed[1].corrected_by), (Some(1), None));
    assert_eq!(listed[1].text, "Rättelse av ver 1");
}

#[tokio::test]
async fn correcting_in_a_year_that_is_not_a_fiscal_year_start_finds_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();

    for start in ["2025-02-01", "2024-01-01", "2027-01-01"] {
        let result = correct_voucher(&pool, id, anna, d(start), 1, d("2025-03-02"), today).await;
        assert!(matches!(result, Err(Error::Domain(DomainError::VoucherNotFound))), "{start}");
    }
}

#[tokio::test]
async fn fiscal_years_run_from_the_first_to_the_current_newest_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let years = list_fiscal_years(&pool, id, anna, d(TODAY)).await.unwrap();
    let before_start = list_fiscal_years(&pool, id, anna, d("2024-06-01")).await.unwrap();

    assert_eq!(years.iter().map(|y| y.start).collect::<Vec<_>>(), [d("2026-01-01"), d("2025-01-01")]);
    assert_eq!(before_start.iter().map(|y| y.start).collect::<Vec<_>>(), [d("2025-01-01")]);
}

#[tokio::test]
async fn non_members_cannot_book_correct_or_read() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();

    assert!(matches!(record_voucher(&pool, id, bo, sale("2025-03-01", 1), today).await, Err(Error::NotFound)));
    assert!(matches!(correct_voucher(&pool, id, bo, d("2025-01-01"), 1, d("2025-03-02"), today).await, Err(Error::NotFound)));
    assert!(matches!(list_vouchers(&pool, id, bo, d("2025-01-01")).await, Err(Error::NotFound)));
    assert!(matches!(list_fiscal_years(&pool, id, bo, today).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn the_database_refuses_a_gap_or_a_duplicate_in_the_projection() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY)).await.unwrap();
    let insert = |number: i64| {
        sqlx::query(
            "INSERT INTO vouchers (company_id, fiscal_year_start, number, date, text, recorded_at, recorded_by)
             VALUES (?, '2025-01-01', ?, '2025-03-01', 'x', 'now', 'test')",
        )
        .bind(id.to_string())
        .bind(number)
        .execute(&pool)
    };

    assert!(insert(3).await.is_err(), "a gap");
    assert!(insert(1).await.is_err(), "a duplicate");
    assert!(insert(2).await.is_ok(), "the next number");
}

#[tokio::test]
async fn the_voucher_projections_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();
    record_voucher(&pool, id, anna, sale("2026-03-01", 250), today).await.unwrap();
    correct_voucher(&pool, id, anna, d("2025-01-01"), 1, d("2025-03-02"), today).await.unwrap();
    let before_2025 = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    let before_2026 = list_vouchers(&pool, id, anna, d("2026-01-01")).await.unwrap();
    let lines_sql = "SELECT company_id || fiscal_year_start || number || line_no || account || debit || credit FROM voucher_lines ORDER BY 1";
    let audit_sql = "SELECT number || recorded_at || recorded_by FROM vouchers ORDER BY 1";
    let (lines, audit) = (table(&pool, lines_sql).await, table(&pool, audit_sql).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap(), before_2025);
    assert_eq!(list_vouchers(&pool, id, anna, d("2026-01-01")).await.unwrap(), before_2026);
    assert_eq!(table(&pool, lines_sql).await, lines);
    assert_eq!(table(&pool, audit_sql).await, audit);
    assert!(audit.iter().all(|row| row.ends_with(&anna.to_string())));
}
```

Run: `cargo test -p doris-ledger --test store`
Expected: compile errors (`record_voucher`, `VoucherRef`, … not found).

- [ ] **Step 2: Add the voucher commands to `lib.rs`**

Extend the imports:

```rust
use doris_company::domain::FiscalYear;
use domain::{Ledger, LedgerEvent, RecordVoucher};
use jiff::civil::Date;
```

Change the re-export to `pub use queries::{list_accounts, list_fiscal_years, list_vouchers};` and append:

```rust
/// Where a voucher landed: its fiscal year and its number in that year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoucherRef {
    pub fiscal_year_start: Date,
    pub number: u32,
}

pub async fn record_voucher(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    today: Date,
) -> Result<VoucherRef> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = record_voucher_in(&mut tx, company_id, actor, cmd, today).await?;
    tx.commit().await?;
    Ok(voucher)
}

/// [`record_voucher`] in the caller's transaction, which must be IMMEDIATE
/// (see [`doris_eventstore::begin`]). Nothing is kept, and no number is used
/// up, unless the caller commits.
pub async fn record_voucher_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    today: Date,
) -> Result<VoucherRef> {
    let company = member_company(conn, company_id, actor).await?;
    let fiscal_year = domain::fiscal_year_for(company.first_fiscal_year, cmd.date, today)?;
    let chart = seeded_chart(conn, company_id, actor).await?;
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let event = domain::record_voucher(&ledger, &chart, cmd)?;
    commit_voucher(conn, company_id, fiscal_year, version, event, actor).await
}

pub async fn correct_voucher(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    date: Date,
    today: Date,
) -> Result<VoucherRef> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = correct_voucher_in(&mut tx, company_id, actor, fiscal_year_start, number, date, today).await?;
    tx.commit().await?;
    Ok(voucher)
}

/// [`correct_voucher`] in the caller's IMMEDIATE transaction.
pub async fn correct_voucher_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    date: Date,
    today: Date,
) -> Result<VoucherRef> {
    let company = member_company(conn, company_id, actor).await?;
    let fiscal_year = company.first_fiscal_year.containing(fiscal_year_start);
    if fiscal_year.start != fiscal_year_start {
        return Err(DomainError::VoucherNotFound.into());
    }
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let event = domain::correct_voucher(&ledger, number, date, today)?;
    commit_voucher(conn, company_id, fiscal_year, version, event, actor).await
}

async fn commit_voucher(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    fiscal_year: FiscalYear,
    version: i64,
    event: LedgerEvent,
    actor: Uuid,
) -> Result<VoucherRef> {
    let LedgerEvent::VoucherRecorded { number, .. } = event;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(conn, &stream, version, &[event], actor).await?;
    Ok(VoucherRef { fiscal_year_start: fiscal_year.start, number })
}

fn ledger_stream(company_id: Uuid, fiscal_year_start: Date) -> String {
    format!("{LEDGER_STREAM}{company_id}-{fiscal_year_start}")
}

/// The ledger stream id's parts: company id and fiscal year start.
fn parse_ledger_stream(stream_id: &str) -> Option<(&str, &str)> {
    let rest = stream_id.strip_prefix(LEDGER_STREAM)?;
    // A uuid is 36 characters, followed by '-' and the YYYY-MM-DD start.
    Some((rest.get(..36)?, rest.get(37..)?))
}

async fn load_ledger(conn: &mut SqliteConnection, company_id: Uuid, fiscal_year: FiscalYear) -> Result<(Ledger, i64)> {
    let recorded = doris_eventstore::load(conn, &ledger_stream(company_id, fiscal_year.start)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<LedgerEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((Ledger::from_events(fiscal_year, &events), version))
}
```

`commit_voucher` destructures `event` before moving it into `append`. If the borrow checker objects, copy `number` first with `let number = match &event { LedgerEvent::VoucherRecorded { number, .. } => *number };`.

- [ ] **Step 3: Project vouchers**

In `projections.rs`, replace the `LEDGER_STREAM` placeholder branch in `apply`:

```rust
    if let Some((company_id, fiscal_year_start)) = crate::parse_ledger_stream(&event.stream_id) {
        return apply_ledger(conn, company_id, fiscal_year_start, event).await;
    }
```

Remove the now unused `LEDGER_STREAM` import, add `use crate::domain::LedgerEvent;`, and append:

```rust
async fn apply_ledger(
    conn: &mut SqliteConnection,
    company_id: &str,
    fiscal_year_start: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    let LedgerEvent::VoucherRecorded { number, date, text, lines, corrects } = event.decode()?;
    sqlx::query(
        "INSERT INTO vouchers (company_id, fiscal_year_start, number, date, text, corrects,
             recorded_at, recorded_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(company_id)
    .bind(fiscal_year_start)
    .bind(number)
    .bind(date.to_string())
    .bind(&text)
    .bind(corrects)
    .bind(&event.recorded_at)
    .bind(event.metadata.actor.as_deref().unwrap_or_default())
    .execute(&mut *conn)
    .await?;
    for (line_no, line) in (1_i64..).zip(&lines) {
        sqlx::query(
            "INSERT INTO voucher_lines (company_id, fiscal_year_start, number, line_no, account,
                 debit, credit)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(company_id)
        .bind(fiscal_year_start)
        .bind(number)
        .bind(line_no)
        .bind(line.account.get())
        .bind(line.debit)
        .bind(line.credit)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(original) = corrects {
        sqlx::query(
            "UPDATE vouchers SET corrected_by = ?
             WHERE company_id = ? AND fiscal_year_start = ? AND number = ?",
        )
        .bind(number)
        .bind(company_id)
        .bind(fiscal_year_start)
        .bind(original)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
```

- [ ] **Step 4: Add the voucher queries**

Append to `queries.rs`, extending its imports with `use crate::domain::{Voucher, VoucherLine};`, `use doris_company::domain::FiscalYear;` and `use jiff::civil::Date;`:

```rust
/// The company's räkenskapsår from the first up to the one containing
/// `today`, newest first.
pub async fn list_fiscal_years(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, today: Date) -> Result<Vec<FiscalYear>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let mut years = vec![company.first_fiscal_year];
    while let Some(last) = years.last().copied()
        && last.end < today
    {
        years.push(last.next());
    }
    years.reverse();
    Ok(years)
}

/// The grundbok for one fiscal year: every voucher with its lines, by number.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn list_vouchers(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, fiscal_year_start: Date) -> Result<Vec<Voucher>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let (company_id, fiscal_year_start) = (company_id.to_string(), fiscal_year_start.to_string());
    let heads: Vec<(u32, String, String, Option<u32>, Option<u32>)> = sqlx::query_as(
        "SELECT number, date, text, corrects, corrected_by FROM vouchers
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(pool)
    .await?;
    let lines: Vec<(u32, u32, i64, i64)> = sqlx::query_as(
        "SELECT number, account, debit, credit FROM voucher_lines
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number, line_no",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(pool)
    .await?;
    let mut vouchers: Vec<Voucher> = heads
        .into_iter()
        .map(|(number, date, text, corrects, corrected_by)| Voucher {
            number,
            date: date.parse().expect("projected dates are valid"),
            text,
            lines: Vec::new(),
            corrects,
            corrected_by,
        })
        .collect();
    for (number, account, debit, credit) in lines {
        // Numbers run 1..=n (the trigger guarantees it), so number - 1 is the index.
        vouchers[number as usize - 1]
            .lines
            .push(VoucherLine::new(account, debit, credit).expect("projected accounts are valid"));
    }
    Ok(vouchers)
}
```

`let … && …` chains need edition 2024, which the workspace uses. If sqlx refuses `u32` for SQLite columns, select `i64` and convert with `as u32`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p doris-ledger`
Expected: all pass.

- [ ] **Step 6: Lint, format and commit**

```bash
cargo fmt -p doris-ledger
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger
git commit -m "Record, correct and list vouchers per fiscal year"
```

---

### Task 5: The stress test

**Files:**
- Create: `crates/ledger/tests/stress.rs`

**Interfaces:**
- Consumes: `record_voucher`, `record_voucher_in`, `correct_voucher`, `set_account_active`, `list_vouchers`, `rebuild_projections`, `VoucherRef`, `domain::{RecordVoucher, VoucherLine, LedgerEvent, DomainError}`, `doris_eventstore::{open, begin, read_all}`.
- Produces: nothing for later tasks.

This task writes a test against code that already exists. It "fails first" in the sense that it must be able to fail. Step 2 proves that by breaking the numbering on purpose.

- [ ] **Step 1: Write the stress test**

`crates/ledger/tests/stress.rs`:

```rust
//! Many concurrent writers on a real database file. However the writes
//! interleave, fail or roll back, every fiscal year's voucher numbers must
//! run 1..=n with no gap and no duplicate, in the events and in the
//! projection, before and after a rebuild.

use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{DomainError, LedgerEvent, RecordVoucher, VoucherLine, Voucher};
use doris_ledger::{
    Error, correct_voucher, list_vouchers, rebuild_projections, record_voucher, record_voucher_in,
    set_account_active,
};
use jiff::civil::Date;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const TASKS: usize = 32;
const OPS_PER_TASK: usize = 50;
const TODAY: &str = "2026-10-02";
/// One date in each of the three fiscal years 2024, 2025 and 2026.
const DATES: [&str; 3] = ["2024-06-15", "2025-03-01", "2026-09-30"];
/// Vouchers booked before the stress, per year, for the tasks to correct.
const TARGETS_PER_YEAR: u32 = 3;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn year_start(date: Date) -> Date {
    Date::new(date.year(), 1, 1).unwrap()
}

fn voucher(date: Date, ore: i64, credit_account: u32) -> RecordVoucher {
    RecordVoucher {
        date,
        text: "Stress".into(),
        lines: vec![
            VoucherLine::new(1930, ore, 0).unwrap(),
            VoucherLine::new(credit_account, 0, ore).unwrap(),
        ],
    }
}

/// Successful commits per fiscal year start, as counted by the writers.
type Committed = Arc<Mutex<BTreeMap<Date, u32>>>;

fn count(committed: &Committed, fiscal_year_start: Date) {
    *committed.lock().unwrap().entry(fiscal_year_start).or_default() += 1;
}

async fn setup(pool: &SqlitePool, anna: Uuid) -> Uuid {
    let id = doris_company::register_company(
        pool,
        anna,
        NewCompany {
            org_nr: "556016-0680",
            name: "Stress AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: d("2024-01-01"),
            fiscal_year_end: d("2024-12-31"),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap();
    // 1910 Kassa is inactive, so bookings on it must fail.
    set_account_active(pool, id, anna, 1910, false).await.unwrap();
    id
}

/// One writer: `OPS_PER_TASK` operations, chosen by their global index `k`.
async fn writer(pool: SqlitePool, company: Uuid, anna: Uuid, task: usize, committed: Committed, corrections: Arc<Mutex<BTreeMap<(Date, u32), u32>>>) {
    let today = d(TODAY);
    for op in 0..OPS_PER_TASK {
        let k = task * OPS_PER_TASK + op;
        let date = d(DATES[k % 3]);
        if op == 0 || k % 10 == 3 {
            // Many tasks race to correct the same few vouchers.
            let target_date = if op == 0 { d(DATES[2]) } else { date };
            let number = if op == 0 { 1 } else { (k / 10) as u32 % TARGETS_PER_YEAR + 1 };
            let start = year_start(target_date);
            match correct_voucher(&pool, company, anna, start, number, target_date, today).await {
                Ok(r) => {
                    count(&committed, r.fiscal_year_start);
                    *corrections.lock().unwrap().entry((start, number)).or_default() += 1;
                }
                Err(Error::Domain(DomainError::AlreadyCorrected)) => {}
                Err(other) => panic!("correction {k}: {other:?}"),
            }
        } else if k % 7 == 0 {
            let cmd = if k % 2 == 0 {
                let mut unbalanced = voucher(date, 100, 3001);
                unbalanced.lines[1].credit = 99;
                unbalanced
            } else {
                voucher(date, 100, 1910)
            };
            match record_voucher(&pool, company, anna, cmd, today).await {
                Err(Error::Domain(DomainError::VoucherUnbalanced | DomainError::AccountInactive)) => {}
                other => panic!("invalid voucher {k} was not refused: {other:?}"),
            }
        } else if k % 5 == 0 {
            // A number is decided, then the transaction is abandoned.
            let mut tx = doris_eventstore::begin(&pool).await.unwrap();
            let abandoned = record_voucher_in(&mut tx, company, anna, voucher(date, 100, 3001), today)
                .await
                .unwrap_or_else(|e| panic!("abandoned {k}: {e:?}"));
            assert!(abandoned.number >= 1);
            drop(tx);
        } else {
            let r = record_voucher(&pool, company, anna, voucher(date, 100 + k as i64, 3001), today)
                .await
                .unwrap_or_else(|e| panic!("voucher {k}: {e:?}"));
            count(&committed, r.fiscal_year_start);
        }
    }
}

async fn assert_consistent(pool: &SqlitePool, company: Uuid, anna: Uuid, committed: &BTreeMap<Date, u32>, corrections: &BTreeMap<(Date, u32), u32>) {
    // 1. The events: every year's numbers are exactly 1..=n.
    let mut conn = pool.acquire().await.unwrap();
    let mut numbers: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for event in doris_eventstore::read_all(&mut conn, 0).await.unwrap() {
        if event.stream_id.starts_with("ledger-") {
            let LedgerEvent::VoucherRecorded { number, .. } = event.decode().unwrap();
            numbers.entry(event.stream_id.clone()).or_default().push(number);
        }
    }
    drop(conn);
    assert_eq!(numbers.len(), 3, "one stream per fiscal year");
    for (stream, numbers) in &numbers {
        let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
        assert_eq!(numbers, &expected, "{stream}");
    }

    for (&start, &n) in committed {
        // 2. As many vouchers as the writers saw commit.
        let vouchers: Vec<Voucher> = list_vouchers(pool, company, anna, start).await.unwrap();
        assert_eq!(vouchers.len() as u32, n, "{start}");
        // 3. The projection matches the events and every voucher balances.
        let stream = numbers.iter().find(|(s, _)| s.ends_with(&start.to_string())).unwrap().1;
        assert_eq!(&vouchers.iter().map(|v| v.number).collect::<Vec<_>>(), stream);
        for v in &vouchers {
            let debit: i64 = v.lines.iter().map(|l| l.debit).sum();
            let credit: i64 = v.lines.iter().map(|l| l.credit).sum();
            assert_eq!(debit, credit, "ver {} in {start}", v.number);
        }
        // 4. Every raced-for voucher was corrected exactly once.
        for target in 1..=TARGETS_PER_YEAR {
            let by: Vec<_> = vouchers.iter().filter(|v| v.corrects == Some(target)).collect();
            let expected = corrections.get(&(start, target)).copied().unwrap_or(0);
            assert!(expected <= 1, "{start} ver {target} corrected {expected} times");
            assert_eq!(by.len() as u32, expected, "{start} ver {target}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn voucher_numbers_never_gap_or_repeat_under_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("stress.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let company = setup(&pool, anna).await;
    let committed: Committed = Arc::default();
    for date in DATES {
        for _ in 0..TARGETS_PER_YEAR {
            let r = record_voucher(&pool, company, anna, voucher(d(date), 1_000, 3001), d(TODAY)).await.unwrap();
            count(&committed, r.fiscal_year_start);
        }
    }
    let corrections = Arc::default();

    let writers: Vec<_> = (0..TASKS)
        .map(|task| tokio::spawn(writer(pool.clone(), company, anna, task, committed.clone(), Arc::clone(&corrections))))
        .collect();
    for w in writers {
        w.await.unwrap();
    }

    let committed = committed.lock().unwrap().clone();
    let corrections = corrections.lock().unwrap().clone();
    assert_eq!(committed.len(), 3);
    assert!(corrections.contains_key(&(d("2026-01-01"), 1)), "the op-0 race was won by someone");
    assert_consistent(&pool, company, anna, &committed, &corrections).await;
    rebuild_projections(&pool).await.unwrap();
    assert_consistent(&pool, company, anna, &committed, &corrections).await;
}
```

Notes for the implementer:
- An `SQLITE_BUSY` ("database is locked") error reaches the `panic!` arms as `Error::Store` and fails the test. That is intended (see the spec). If it happens, **stop and report it with the output**. Don't raise timeouts or pool sizes to hide it; that is a decision for the user.
- `drop(tx)` makes sqlx queue a rollback on that connection, which runs before the connection is used again. That is what we want to exercise: an abandoned write.

- [ ] **Step 2: Prove the test can fail**

Temporarily break the numbering in `domain::record_voucher`: change `number: ledger.last_number() + 1` to `number: ledger.last_number() + 2`.
Run: `cargo test -p doris-ledger --test stress`
Expected: FAIL. The trigger refuses the gapped insert, so the first valid write panics.

Then also try removing the trigger. Comment it out in `0006_ledger.sql`. A migration checksum change only matters for existing databases; tests use fresh ones.
Run again. Expected: FAIL at check 1 (`numbers` is `[2, 4, 6, …]`).

Revert both changes (`git checkout migrations/0006_ledger.sql crates/ledger/src/domain.rs`).

- [ ] **Step 3: Run it for real, three times**

Run: `for i in 1 2 3; do cargo test -p doris-ledger --test stress -- --nocapture || break; done`
Expected: PASS all three times. Record the run time. It should be a few seconds. If one run takes more than 60 s, report it.

- [ ] **Step 4: Lint and commit**

```bash
cargo fmt -p doris-ledger
cargo clippy -p doris-ledger --all-targets -- -D warnings
git add crates/ledger/tests/stress.rs
git commit -m "Stress-test voucher numbering under concurrency and rollbacks"
```

---

### Task 6: The ledger gRPC API

**Files:**
- Create: `proto/doris/ledger/v1/ledger.proto`, `crates/server/src/ledger.rs`, `crates/server/tests/ledger.rs`
- Modify: `crates/proto/build.rs`, `crates/proto/src/lib.rs`, `crates/server/Cargo.toml`, `crates/server/src/lib.rs`, `crates/server/src/main.rs`, `crates/server/src/grpc.rs`, `crates/server/src/company.rs`, `crates/server/tests/common/mod.rs`

**Interfaces:**
- Consumes: the whole `doris_ledger` public API from Tasks 3 and 4.
- Produces:
  - `doris_proto::ledger::v1` with `LedgerService` and the messages below.
  - `doris_server::LedgerApi::new(SqlitePool)`.
  - `router(api, companies, ledger: LedgerApi, cors_origins, serve_frontend)`, which gains the new third parameter.
  - `grpc::today() -> Date`.
  - The test helper `TestServer::ledger() -> Ledger`, with `pub type Ledger = LedgerServiceClient<Transport>`.

- [ ] **Step 1: Write the proto**

`proto/doris/ledger/v1/ledger.proto`:

```proto
syntax = "proto3";

package doris.ledger.v1;

// A company's chart of accounts and its vouchers (verifikationer). Every RPC
// needs a session; a company the caller isn't a member of answers NOT_FOUND
// "company_not_found". Dates are YYYY-MM-DD; amounts are öre.
service LedgerService {
  rpc ListAccounts(ListAccountsRequest) returns (ListAccountsResponse);
  rpc AddAccount(AddAccountRequest) returns (AddAccountResponse);
  rpc RenameAccount(RenameAccountRequest) returns (RenameAccountResponse);
  rpc SetAccountActive(SetAccountActiveRequest) returns (SetAccountActiveResponse);
  // From the first räkenskapsår up to the current one, newest first.
  rpc ListFiscalYears(ListFiscalYearsRequest) returns (ListFiscalYearsResponse);
  // The server decides the voucher's number.
  rpc RecordVoucher(RecordVoucherRequest) returns (RecordVoucherResponse);
  // Books a reversing voucher in the original's fiscal year.
  rpc CorrectVoucher(CorrectVoucherRequest) returns (CorrectVoucherResponse);
  // The grundbok for one fiscal year, by number.
  rpc ListVouchers(ListVouchersRequest) returns (ListVouchersResponse);
}

message Account {
  uint32 number = 1;
  string name = 2;
  bool active = 3;
}

message ListAccountsRequest {
  string company_id = 1;
}

message ListAccountsResponse {
  repeated Account accounts = 1;
}

message AddAccountRequest {
  string company_id = 1;
  uint32 number = 2;
  string name = 3;
}

message AddAccountResponse {}

message RenameAccountRequest {
  string company_id = 1;
  uint32 number = 2;
  string name = 3;
}

message RenameAccountResponse {}

message SetAccountActiveRequest {
  string company_id = 1;
  uint32 number = 2;
  bool active = 3;
}

message SetAccountActiveResponse {}

message FiscalYear {
  string start = 1;
  string end = 2;
}

message ListFiscalYearsRequest {
  string company_id = 1;
}

message ListFiscalYearsResponse {
  repeated FiscalYear fiscal_years = 1;
}

// Exactly one of debit and credit is positive.
message VoucherLine {
  uint32 account = 1;
  int64 debit = 2;
  int64 credit = 3;
}

message RecordVoucherRequest {
  string company_id = 1;
  string date = 2;
  string text = 3;
  repeated VoucherLine lines = 4;
}

message RecordVoucherResponse {
  string fiscal_year_start = 1;
  uint32 number = 2;
}

message CorrectVoucherRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 number = 3;
  string date = 4;
}

message CorrectVoucherResponse {
  uint32 number = 1;
}

message ListVouchersRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
}

// corrects and corrected_by are voucher numbers in the same fiscal year;
// 0 means none.
message Voucher {
  uint32 number = 1;
  string date = 2;
  string text = 3;
  repeated VoucherLine lines = 4;
  uint32 corrects = 5;
  uint32 corrected_by = 6;
}

message ListVouchersResponse {
  repeated Voucher vouchers = 1;
}
```

In `crates/proto/build.rs`, add `"../../proto/doris/ledger/v1/ledger.proto",` to the list. In `crates/proto/src/lib.rs`, add:

```rust
pub mod ledger {
    pub mod v1 {
        tonic::include_proto!("doris.ledger.v1");
    }
}
```

Run: `cargo build -p doris-proto --features server`
Expected: builds.

- [ ] **Step 2: Write the failing integration tests**

In `crates/server/tests/common/mod.rs`:
- Add `use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;`.
- Add `use doris_server::LedgerApi;` next to the other `doris_server` imports.
- Add `pub type Ledger = LedgerServiceClient<Transport>;`.
- In `launch`, pass `LedgerApi::new(pool.clone()),` as the third argument to `router`.
- Add:

```rust
    pub fn ledger(&self) -> Ledger {
        LedgerServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }
```

`crates/server/tests/ledger.rs`:

```rust
mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as pb;
use tonic::Code;

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

fn sale(company_id: &str, ore: i64) -> pb::RecordVoucherRequest {
    pb::RecordVoucherRequest {
        company_id: company_id.into(),
        date: "2026-01-15".into(),
        text: "Försäljning".into(),
        lines: vec![
            pb::VoucherLine { account: 1930, debit: ore, credit: 0 },
            pb::VoucherLine { account: 3001, debit: 0, credit: ore },
        ],
    }
}

#[tokio::test]
async fn a_member_keeps_the_chart_and_books_and_corrects_vouchers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();

    let accounts = api.list_accounts(authed(pb::ListAccountsRequest { company_id: id.clone() }, &anna)).await.unwrap().into_inner().accounts;
    assert!(accounts.iter().any(|a| a.number == 1930 && a.active));
    api.add_account(authed(pb::AddAccountRequest { company_id: id.clone(), number: 1931, name: "Sparkonto".into() }, &anna)).await.unwrap();
    api.rename_account(authed(pb::RenameAccountRequest { company_id: id.clone(), number: 1931, name: "Sparkonto SEB".into() }, &anna)).await.unwrap();
    api.set_account_active(authed(pb::SetAccountActiveRequest { company_id: id.clone(), number: 1910, active: false }, &anna)).await.unwrap();
    let accounts = api.list_accounts(authed(pb::ListAccountsRequest { company_id: id.clone() }, &anna)).await.unwrap().into_inner().accounts;
    assert!(accounts.contains(&pb::Account { number: 1931, name: "Sparkonto SEB".into(), active: true }));
    assert!(accounts.contains(&pb::Account { number: 1910, name: "Kassa".into(), active: false }));

    let years = api.list_fiscal_years(authed(pb::ListFiscalYearsRequest { company_id: id.clone() }, &anna)).await.unwrap().into_inner().fiscal_years;
    assert_eq!(years.last().unwrap(), &pb::FiscalYear { start: "2026-01-01".into(), end: "2026-12-31".into() });

    let booked = api.record_voucher(authed(sale(&id, 12_500), &anna)).await.unwrap().into_inner();
    assert_eq!(booked, pb::RecordVoucherResponse { fiscal_year_start: "2026-01-01".into(), number: 1 });
    let corrected = api
        .correct_voucher(authed(pb::CorrectVoucherRequest { company_id: id.clone(), fiscal_year_start: "2026-01-01".into(), number: 1, date: "2026-01-16".into() }, &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(corrected.number, 2);

    let vouchers = api.list_vouchers(authed(pb::ListVouchersRequest { company_id: id, fiscal_year_start: "2026-01-01".into() }, &anna)).await.unwrap().into_inner().vouchers;
    assert_eq!(
        vouchers[0],
        pb::Voucher {
            number: 1,
            date: "2026-01-15".into(),
            text: "Försäljning".into(),
            lines: sale("", 12_500).lines,
            corrects: 0,
            corrected_by: 2,
        }
    );
    assert_eq!((vouchers[1].corrects, vouchers[1].text.as_str()), (1, "Rättelse av ver 1"));
}

#[tokio::test]
async fn ledger_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.set_account_active(authed(pb::SetAccountActiveRequest { company_id: id.clone(), number: 1910, active: false }, &anna)).await.unwrap();
    api.record_voucher(authed(sale(&id, 100), &anna)).await.unwrap();
    api.correct_voucher(authed(pb::CorrectVoucherRequest { company_id: id.clone(), fiscal_year_start: "2026-01-01".into(), number: 1, date: "2026-01-15".into() }, &anna)).await.unwrap();

    let mut unbalanced = sale(&id, 100);
    unbalanced.lines[1].credit = 99;
    let mut inactive = sale(&id, 100);
    inactive.lines[0].account = 1910;
    let mut bad_date = sale(&id, 100);
    bad_date.date = "15/1".into();
    let mut future = sale(&id, 100);
    future.date = "2999-01-01".into();
    let mut before = sale(&id, 100);
    before.date = "2025-12-31".into();
    let mut no_text = sale(&id, 100);
    no_text.text = " ".into();
    let mut one_line = sale(&id, 100);
    one_line.lines.pop();
    let mut negative = sale(&id, 100);
    negative.lines[0].debit = -100;
    let mut unknown = sale(&id, 100);
    unknown.lines[0].account = 1999;

    for (request, expected) in [
        (unbalanced, (Code::InvalidArgument, "voucher_unbalanced")),
        (inactive, (Code::FailedPrecondition, "account_inactive")),
        (bad_date, (Code::InvalidArgument, "invalid_date")),
        (future, (Code::InvalidArgument, "voucher_date_in_future")),
        (before, (Code::InvalidArgument, "voucher_date_before_first_fiscal_year")),
        (no_text, (Code::InvalidArgument, "invalid_voucher_text")),
        (one_line, (Code::InvalidArgument, "invalid_voucher_lines")),
        (negative, (Code::InvalidArgument, "invalid_amount")),
        (unknown, (Code::NotFound, "account_not_found")),
    ] {
        let err = api.record_voucher(authed(request, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (expected.0, expected.1.to_owned()));
    }

    let correct = |number: u32, date: &str| pb::CorrectVoucherRequest { company_id: id.clone(), fiscal_year_start: "2026-01-01".into(), number, date: date.into() };
    for (request, expected) in [
        (correct(1, "2026-01-15"), (Code::FailedPrecondition, "already_corrected")),
        (correct(2, "2026-01-15"), (Code::FailedPrecondition, "cannot_correct_correction")),
        (correct(9, "2026-01-15"), (Code::NotFound, "voucher_not_found")),
    ] {
        let err = api.correct_voucher(authed(request, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (expected.0, expected.1.to_owned()));
    }

    for (result, expected) in [
        (api.add_account(authed(pb::AddAccountRequest { company_id: id.clone(), number: 1930, name: "X".into() }, &anna)).await.map(drop), (Code::AlreadyExists, "account_exists")),
        (api.add_account(authed(pb::AddAccountRequest { company_id: id.clone(), number: 99, name: "X".into() }, &anna)).await.map(drop), (Code::InvalidArgument, "invalid_account_number")),
        (api.add_account(authed(pb::AddAccountRequest { company_id: id.clone(), number: 1931, name: "".into() }, &anna)).await.map(drop), (Code::InvalidArgument, "invalid_account_name")),
        (api.rename_account(authed(pb::RenameAccountRequest { company_id: id.clone(), number: 1999, name: "X".into() }, &anna)).await.map(drop), (Code::NotFound, "account_not_found")),
    ] {
        assert_eq!(code_of(result.unwrap_err()), (expected.0, expected.1.to_owned()));
    }
}

#[tokio::test]
async fn others_get_company_not_found_and_strangers_not_signed_in() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    for company_id in [id.clone(), "not-a-uuid".into()] {
        let err = api.list_accounts(authed(pb::ListAccountsRequest { company_id: company_id.clone() }, &bo)).await.unwrap_err();
        assert_eq!(code_of(err), not_found);
        let err = api.record_voucher(authed(sale(&company_id, 100), &bo)).await.unwrap_err();
        assert_eq!(code_of(err), not_found);
    }

    let unauthenticated = (Code::Unauthenticated, "not_signed_in".to_owned());
    let results = [
        api.list_accounts(pb::ListAccountsRequest { company_id: id.clone() }).await.map(drop),
        api.add_account(pb::AddAccountRequest { company_id: id.clone(), number: 1931, name: "X".into() }).await.map(drop),
        api.rename_account(pb::RenameAccountRequest { company_id: id.clone(), number: 1930, name: "X".into() }).await.map(drop),
        api.set_account_active(pb::SetAccountActiveRequest { company_id: id.clone(), number: 1930, active: false }).await.map(drop),
        api.list_fiscal_years(pb::ListFiscalYearsRequest { company_id: id.clone() }).await.map(drop),
        api.record_voucher(sale(&id, 100)).await.map(drop),
        api.correct_voucher(pb::CorrectVoucherRequest { company_id: id.clone(), fiscal_year_start: "2026-01-01".into(), number: 1, date: "2026-01-15".into() }).await.map(drop),
        api.list_vouchers(pb::ListVouchersRequest { company_id: id, fiscal_year_start: "2026-01-01".into() }).await.map(drop),
    ];
    for result in results {
        assert_eq!(code_of(result.unwrap_err()), unauthenticated);
    }
}
```

Run: `cargo test -p doris-server --test ledger`
Expected: compile errors (`LedgerApi` not found).

- [ ] **Step 3: Share `today()`**

In `crates/server/src/grpc.rs`, add `use jiff::Timestamp;`, `use jiff::civil::Date;` and `use jiff::tz::TimeZone;` (skip any already imported), and:

```rust
/// Today's date for date rules.
// ponytail: "today" in UTC, so the date flips up to 2 hours late in Sweden;
// use Europe/Stockholm once the image ships tzdata.
pub(crate) fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::UTC).date()
}
```

In `crates/server/src/company.rs` `get_company`, replace the two lines (the `ponytail` comment and `let today = …`) with `let today = grpc::today();`, and drop the now-unused `Timestamp`/`TimeZone` imports.

- [ ] **Step 4: Write `LedgerApi`**

Add `doris-ledger.workspace = true` to `crates/server/Cargo.toml` `[dependencies]`.

`crates/server/src/ledger.rs`:

```rust
//! `doris.ledger.v1.LedgerService`: maps gRPC calls onto `doris_ledger`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist.

use crate::grpc::{signed_in_user, today};
use doris_ledger::domain::{DomainError, RecordVoucher, Voucher, VoucherLine};
use doris_ledger::Error;
use doris_proto::ledger::v1 as pb;
use doris_proto::ledger::v1::ledger_service_server::LedgerService;
use jiff::civil::Date;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct LedgerApi {
    pool: SqlitePool,
}

impl LedgerApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_ledger` itself.
    async fn caller<T>(&self, request: &Request<T>, company_id: &str) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id.parse().map_err(|_| company_not_found())?;
        Ok((company, user.id))
    }
}

#[tonic::async_trait]
impl LedgerService for LedgerApi {
    async fn list_accounts(&self, request: Request<pb::ListAccountsRequest>) -> Result<Response<pb::ListAccountsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let accounts = doris_ledger::list_accounts(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(|a| pb::Account { number: a.number.get().into(), name: a.name.as_str().to_owned(), active: a.active })
            .collect();
        Ok(Response::new(pb::ListAccountsResponse { accounts }))
    }

    async fn add_account(&self, request: Request<pb::AddAccountRequest>) -> Result<Response<pb::AddAccountResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::add_account(&self.pool, company, user, req.number, &req.name).await.map_err(status)?;
        Ok(Response::new(pb::AddAccountResponse {}))
    }

    async fn rename_account(&self, request: Request<pb::RenameAccountRequest>) -> Result<Response<pb::RenameAccountResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::rename_account(&self.pool, company, user, req.number, &req.name).await.map_err(status)?;
        Ok(Response::new(pb::RenameAccountResponse {}))
    }

    async fn set_account_active(&self, request: Request<pb::SetAccountActiveRequest>) -> Result<Response<pb::SetAccountActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_ledger::set_account_active(&self.pool, company, user, req.number, req.active).await.map_err(status)?;
        Ok(Response::new(pb::SetAccountActiveResponse {}))
    }

    async fn list_fiscal_years(&self, request: Request<pb::ListFiscalYearsRequest>) -> Result<Response<pb::ListFiscalYearsResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_years = doris_ledger::list_fiscal_years(&self.pool, company, user, today())
            .await
            .map_err(status)?
            .into_iter()
            .map(|y| pb::FiscalYear { start: y.start.to_string(), end: y.end.to_string() })
            .collect();
        Ok(Response::new(pb::ListFiscalYearsResponse { fiscal_years }))
    }

    async fn record_voucher(&self, request: Request<pb::RecordVoucherRequest>) -> Result<Response<pb::RecordVoucherResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let cmd = RecordVoucher {
            date: date(&req.date)?,
            text: req.text,
            lines: req
                .lines
                .iter()
                .map(|l| VoucherLine::new(l.account, l.debit, l.credit))
                .collect::<Result<_, _>>()
                .map_err(domain_status)?,
        };
        let booked = doris_ledger::record_voucher(&self.pool, company, user, cmd, today()).await.map_err(status)?;
        Ok(Response::new(pb::RecordVoucherResponse {
            fiscal_year_start: booked.fiscal_year_start.to_string(),
            number: booked.number,
        }))
    }

    async fn correct_voucher(&self, request: Request<pb::CorrectVoucherRequest>) -> Result<Response<pb::CorrectVoucherResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let booked = doris_ledger::correct_voucher(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            date(&req.date)?,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CorrectVoucherResponse { number: booked.number }))
    }

    async fn list_vouchers(&self, request: Request<pb::ListVouchersRequest>) -> Result<Response<pb::ListVouchersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let fiscal_year_start = date(&request.get_ref().fiscal_year_start)?;
        let vouchers = doris_ledger::list_vouchers(&self.pool, company, user, fiscal_year_start)
            .await
            .map_err(status)?
            .into_iter()
            .map(voucher_message)
            .collect();
        Ok(Response::new(pb::ListVouchersResponse { vouchers }))
    }
}

fn voucher_message(v: Voucher) -> pb::Voucher {
    pb::Voucher {
        number: v.number,
        date: v.date.to_string(),
        text: v.text,
        lines: v
            .lines
            .iter()
            .map(|l| pb::VoucherLine { account: l.account.get().into(), debit: l.debit, credit: l.credit })
            .collect(),
        corrects: v.corrects.unwrap_or(0),
        corrected_by: v.corrected_by.unwrap_or(0),
    }
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse().map_err(|_| Status::invalid_argument("invalid_date"))
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidAccountNumber => Status::invalid_argument("invalid_account_number"),
        InvalidAccountName => Status::invalid_argument("invalid_account_name"),
        AccountExists => Status::already_exists("account_exists"),
        AccountNotFound => Status::not_found("account_not_found"),
        AccountInactive => Status::failed_precondition("account_inactive"),
        InvalidVoucherText => Status::invalid_argument("invalid_voucher_text"),
        InvalidVoucherLines => Status::invalid_argument("invalid_voucher_lines"),
        InvalidAmount => Status::invalid_argument("invalid_amount"),
        VoucherUnbalanced => Status::invalid_argument("voucher_unbalanced"),
        VoucherDateInFuture => Status::invalid_argument("voucher_date_in_future"),
        VoucherDateBeforeFirstFiscalYear => Status::invalid_argument("voucher_date_before_first_fiscal_year"),
        CorrectionDateOutsideFiscalYear => Status::invalid_argument("correction_date_outside_fiscal_year"),
        VoucherNotFound => Status::not_found("voucher_not_found"),
        AlreadyCorrected => Status::failed_precondition("already_corrected"),
        CannotCorrectCorrection => Status::failed_precondition("cannot_correct_correction"),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::NotFound => company_not_found(),
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}
```

Note the order in `record_voucher`: the session and company id are checked first. Then the date and the account numbers are parsed. Membership is checked inside `doris_ledger`, so a non-member sending a bad date gets `invalid_date`. The same goes for `doris_company`, where input is validated before membership. Neither reveals anything about the company.

If `signed_in_user` is not `pub(crate)` in `grpc.rs`, make it so. `company.rs` already uses it, so it is.

- [ ] **Step 5: Wire the router**

In `crates/server/src/lib.rs`:
- Add `mod ledger;` and `pub use ledger::LedgerApi;`.
- Add `use doris_proto::ledger::v1::ledger_service_server::LedgerServiceServer;`.
- Give `router` a new parameter `ledger: LedgerApi` after `companies`.
- Add `.add_service(LedgerServiceServer::new(ledger))` after the company service.

In `crates/server/src/main.rs`, import `LedgerApi` and pass `LedgerApi::new(pool.clone())`. The existing `CompanyApi::new(pool, …)` call moves `pool`, so clone it there instead, or put the `LedgerApi` argument first.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p doris-server`
Expected: all pass, the new ones and the existing ones.

- [ ] **Step 7: Lint, format and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add proto crates/proto crates/server
git commit -m "Serve the ledger over gRPC-Web"
```

---

### Task 7: Web foundations: client, error texts, amounts, Table

**Files:**
- Modify: `crates/web/src/api.rs`, `crates/web/src/errors.rs`, `crates/web/src/format.rs`, `crates/web/src/ui.rs`

**Interfaces:**
- Consumes: `doris_proto::ledger::v1`.
- Produces:
  - In `api.rs`: `lpb` (an alias for `doris_proto::ledger::v1`) and `ledger_api() -> LedgerApi`.
  - In `format.rs`: `parse_amount(&str) -> Option<i64>`, `amount(i64) -> String`, `today() -> String` (`YYYY-MM-DD`, browser local date).
  - In `ui.rs`:
    - the components `Table` (children: thead/tbody) and `TextInput`, a bare input bound to a `RwSignal<String>` with a required `label` used as `aria-label`
    - `pub const`s `TABLE_HEAD`, `TABLE_BODY`, `TABLE_ROW`, `TABLE_HEADER_CELL`, `TABLE_CELL`.

- [ ] **Step 0: Raise the budget and measure the wasm before**

In `Makefile`, set `WASM_BUDGET := 900000`. In `AGENTS.md`, change "fails if it grows past `WASM_BUDGET` (800 KB uncompressed)" to "(900 KB uncompressed)". Run `make dist` and note the size printed by `wc -c crates/web/dist/*_bg.wasm`; expected about 786 000. Put the number in this task's commit message, for example "wasm before: 786 184 bytes", and add `Makefile` and `AGENTS.md` to the commit.

- [ ] **Step 1: Write the failing tests**

In `crates/web/src/format.rs` `mod tests`, add:

```rust
    #[test]
    fn parse_amount_reads_swedish_and_plain_spellings() {
        assert_eq!(parse_amount("1 234,50"), Some(123_450));
        assert_eq!(parse_amount("1\u{a0}234,50"), Some(123_450));
        assert_eq!(parse_amount("1\u{202f}234,5"), Some(123_450));
        assert_eq!(parse_amount("1234.5"), Some(123_450));
        assert_eq!(parse_amount("12"), Some(1_200));
        assert_eq!(parse_amount(" 0,05 "), Some(5));
        assert_eq!(parse_amount("12,"), Some(1_200));
    }

    #[test]
    fn parse_amount_refuses_anything_else() {
        for bad in ["", " ", "-5", "+5", "1,234", "1,2,3", "1.2.3", "abc", "12 kr", ",50", "99999999999999999999"] {
            assert_eq!(parse_amount(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn amount_formats_ore_as_kronor_with_grouped_thousands() {
        assert_eq!(amount(0), "0,00");
        assert_eq!(amount(5), "0,05");
        assert_eq!(amount(123_450), "1\u{a0}234,50");
        assert_eq!(amount(100_000_000), "1\u{a0}000\u{a0}000,00");
        assert_eq!(amount(-123_450), "-1\u{a0}234,50");
    }
```

`"1,234"` is refused because it has three decimals. A Swede never writes thousands with a comma, so a three-digit fraction is a typo.

In `crates/web/src/errors.rs` `mod tests`, add:

```rust
    #[test]
    fn ledger_codes_have_swedish_messages() {
        for code in [
            "invalid_account_number",
            "invalid_account_name",
            "account_exists",
            "account_not_found",
            "account_inactive",
            "invalid_voucher_text",
            "invalid_voucher_lines",
            "invalid_amount",
            "voucher_unbalanced",
            "voucher_date_in_future",
            "voucher_date_before_first_fiscal_year",
            "correction_date_outside_fiscal_year",
            "voucher_not_found",
            "already_corrected",
            "cannot_correct_correction",
            "invalid_date",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }
```

Run: `cargo test -p doris-web`
Expected: compile errors (`parse_amount`, `amount` not found), and the errors test fails.

- [ ] **Step 2: Implement `format.rs` additions**

```rust
/// Kronor as typed in Sweden ("1 234,50", "1234.5", "12") to öre. Spaces,
/// no-break spaces and narrow no-break spaces group thousands; comma or
/// point marks the decimals, of which there are at most two. Anything else,
/// including a sign or an empty field, is `None`.
pub fn parse_amount(raw: &str) -> Option<i64> {
    let digits: String = raw
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}'))
        .collect();
    let (kronor, ore) = digits.split_once([',', '.']).unwrap_or((&digits, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if kronor.is_empty() || ore.len() > 2 || !all_digits(kronor) || !all_digits(ore) {
        return None;
    }
    let ore: i64 = format!("{ore:0<2}").parse().ok()?;
    kronor.parse::<i64>().ok()?.checked_mul(100)?.checked_add(ore)
}

/// Öre as kronor, thousands grouped with no-break spaces: 123450 → "1 234,50".
pub fn amount(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    let kronor = (ore / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in kronor.chars().enumerate() {
        if i > 0 && (kronor.len() - i) % 3 == 0 {
            grouped.push('\u{a0}');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped},{:02}", ore % 100)
}

/// Today by the browser's clock and time zone, as `YYYY-MM-DD`.
pub fn today() -> String {
    let now = js_sys::Date::new_0();
    format!("{:04}-{:02}-{:02}", now.get_full_year(), now.get_month() + 1, now.get_date())
}
```

`"99999999999999999999"` fails in `parse::<i64>()` and gives `None`.

- [ ] **Step 3: Add the error texts**

In `errors.rs` `message`, before the `_` arm:

```rust
        "invalid_account_number" => "Kontonumret ska vara fyra siffror, 1000–8999.",
        "invalid_account_name" => "Kontonamnet måste vara 1–100 tecken.",
        "account_exists" => "Kontot finns redan i kontoplanen.",
        "account_not_found" => "Kontot finns inte i kontoplanen.",
        "account_inactive" => "Kontot är inaktivt. Aktivera det i kontoplanen eller välj ett annat.",
        "invalid_voucher_text" => "Texten måste vara 1–200 tecken.",
        "invalid_voucher_lines" => "En verifikation har 2–100 rader.",
        "invalid_amount" => "Varje rad ska ha ett belopp i antingen debet eller kredit.",
        "voucher_unbalanced" => "Debet och kredit måste vara lika stora.",
        "voucher_date_in_future" => "Datumet kan inte vara i framtiden.",
        "voucher_date_before_first_fiscal_year" => "Datumet ligger före företagets första räkenskapsår.",
        "correction_date_outside_fiscal_year" => "Rättelsen ska dateras inom samma räkenskapsår som verifikationen.",
        "voucher_not_found" => "Verifikationen finns inte.",
        "already_corrected" => "Verifikationen är redan rättad.",
        "cannot_correct_correction" => "En rättelse kan inte rättas. Bokför en ny verifikation i stället.",
        "invalid_date" => "Ange ett giltigt datum.",
```

- [ ] **Step 4: Add the client**

In `api.rs`:

```rust
use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;

pub use doris_proto::ledger::v1 as lpb;

pub type LedgerApi = LedgerServiceClient<Client>;

pub fn ledger_api() -> LedgerApi {
    LedgerServiceClient::new(client())
}
```

- [ ] **Step 5: Add `Table` and `TextInput` with the preset's classes**

Generate the reference classes in a scratch directory, not in the repo:

```bash
cd "$(mktemp -d)" && npx shadcn@latest init -t vite -b radix -p b1Gdz9bFY -y && npx shadcn@latest add table -y && cat src/components/ui/table.tsx
```

Copy the `className` strings of `Table` (the container `div` and the `table`), `TableHeader`, `TableBody`, `TableRow`, `TableHead` and `TableCell` **exactly** into these constants in `ui.rs`. The values below are shadcn's usual ones and serve only as the expected shape. The generated output wins.

```rust
const TABLE_CONTAINER: &str = "relative w-full overflow-x-auto";
const TABLE: &str = "w-full caption-bottom text-xs/relaxed";
pub const TABLE_HEAD: &str = "[&_tr]:border-b";
pub const TABLE_BODY: &str = "[&_tr:last-child]:border-0";
pub const TABLE_ROW: &str = "border-b transition-colors hover:bg-muted/50 has-aria-expanded:bg-muted/50 data-[state=selected]:bg-muted";
pub const TABLE_HEADER_CELL: &str = "h-10 px-2 text-left align-middle font-medium whitespace-nowrap text-foreground [&:has([role=checkbox])]:pr-0";
pub const TABLE_CELL: &str = "p-2 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0";

/// A table in the preset's style. Children are `<thead class=TABLE_HEAD>`
/// and `<tbody class=TABLE_BODY>` with `TABLE_ROW` rows and
/// `TABLE_HEADER_CELL`/`TABLE_CELL` cells.
#[component]
pub fn Table(children: Children) -> impl IntoView {
    view! {
        <div class=TABLE_CONTAINER>
            <table class=TABLE>{children()}</table>
        </div>
    }
}

/// An input without a visible label, for tables and line editors. `label`
/// is its accessible name.
#[component]
pub fn TextInput(
    #[prop(into)] label: String,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
    #[prop(optional)] inputmode: &'static str,
    #[prop(optional)] list: &'static str,
) -> impl IntoView {
    view! {
        <input
            type=kind
            class=INPUT
            aria-label=label
            inputmode=(!inputmode.is_empty()).then_some(inputmode)
            list=(!list.is_empty()).then_some(list)
            bind:value=value
        />
    }
}
```

- [ ] **Step 6: Run the tests and the wasm lint**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: tests pass. Clippy may warn that `ledger_api`, `Table` and the rest are unused until Task 8. If it does, put `#[allow(dead_code)]` on them only for this commit, and remove it in Task 8.

- [ ] **Step 7: Commit**

```bash
git add crates/web
git commit -m "Add the ledger client, error texts, amounts and a table to the web app"
```

---

### Task 8: Follow other tabs, header links, the chart of accounts page

**Files:**
- Modify: `crates/web/Cargo.toml`, `crates/web/src/active_company.rs`, `crates/web/src/app.rs`, `crates/web/src/pages/mod.rs`
- Create: `crates/web/src/pages/accounts.rs`, `e2e/tests/ledger.spec.ts`
- Modify: `e2e/tests/fixtures.ts`, `e2e/tests/companies.spec.ts`

**Interfaces:**
- Consumes: Task 7 (`ledger_api`, `lpb`, `Table`, `TextInput`, table consts) and `Companies` (`active`, `list`).
- Produces:
  - `Companies::follow_other_tabs(self)`, called once from `Companies::new`.
  - The route `/accounts` with the page `Accounts`.
  - Header links "Verifikationer" (`/vouchers`) and "Kontoplan" (`/accounts`).
  - An `addCompany(page, app, orgNr, name)` exported from `e2e/tests/fixtures.ts`.

- [ ] **Step 1: Move `addCompany` into the fixtures**

Cut `addCompany` from `e2e/tests/companies.spec.ts` and paste it into `e2e/tests/fixtures.ts` as `export async function addCompany(…)`. Use the same body, and import `Page` there if needed. In `companies.spec.ts`, import it: `import { addCompany, expect, register, test } from "./fixtures";`.

- [ ] **Step 2: Write the failing e2e tests**

`e2e/tests/ledger.spec.ts`:

```ts
import { addCompany, expect, register, test } from "./fixtures";

test("the chart of accounts starts from BAS and can be extended", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Kontoplan" }).click();

  const bank = page.getByRole("row", { name: /^1930 Företagskonto/ });
  await expect(bank).toContainText("Aktivt");

  await page.getByLabel("Nummer").fill("1931");
  await page.getByLabel("Namn").fill("Sparkonto");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto/ })).toBeVisible();

  await page.getByLabel("Nummer").fill("1930");
  await page.getByLabel("Namn").fill("Bank");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("alert")).toHaveText("Kontot finns redan i kontoplanen.");

  const sparkonto = page.getByRole("row", { name: /^1931 / });
  await sparkonto.getByRole("button", { name: "Byt namn" }).click();
  await sparkonto.getByLabel("Nytt namn för 1931").fill("Sparkonto SEB");
  await sparkonto.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto SEB/ })).toBeVisible();

  await page.getByRole("row", { name: /^1910 Kassa/ }).getByRole("button", { name: "Inaktivera" }).click();
  await expect(page.getByRole("row", { name: /^1910 Kassa/ })).toHaveCount(0);
  await page.getByLabel("Visa inaktiva").check();
  await expect(page.getByRole("row", { name: /^1910 Kassa/ })).toContainText("Inaktivt");
});

test("another tab follows the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  const other = await page.context().newPage();
  await other.goto(app);
  const active = (p: typeof page) => p.getByLabel("Aktivt företag").locator("option:checked");
  await expect(active(other)).toHaveText("Exempel AB");

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(active(other)).toHaveText("Bolaget AB");
});
```

Run: `make e2e` (or, if the binary and frontend are already built, `cd e2e && npx playwright test ledger.spec.ts`).
Expected: both fail. There is no "Kontoplan" link, and the other tab stays on "Exempel AB".

- [ ] **Step 3: Follow other tabs**

In `crates/web/Cargo.toml`, add `"StorageEvent"` to the `web-sys` features.

In `active_company.rs`, at the end of `Companies::new` before returning, call `companies.follow_other_tabs();`. Then add to `impl Companies`:

```rust
    /// Another tab of this browser chose another company: switch too, so
    /// this tab never books in a company the user has just left. A company
    /// this tab doesn't know yet (added in the other tab) reloads the list.
    fn follow_other_tabs(self) {
        // The handle is dropped on purpose: the listener lives as long as the app.
        let _ = window_event_listener(leptos::ev::storage, move |event| {
            let Some(user_id) = self.user_id.get_untracked() else {
                return;
            };
            if event.key().as_deref() != Some(storage_key(&user_id).as_str()) {
                return;
            }
            let Some(company_id) = event.new_value().filter(|id| !id.is_empty()) else {
                return;
            };
            let known = self.list.with_untracked(|list| list.iter().any(|c| c.id == company_id));
            self.active.set(company_id);
            if !known {
                self.reload();
            }
        });
    }
```

The `storage` event only fires in *other* tabs, so this tab's own `remember` never loops back.

- [ ] **Step 4: Header links and route**

In `app.rs`'s `Header`, right after `<ActiveCompanySelect />`:

```rust
                    <Show when=move || !companies.active.get().is_empty()>
                        <A href="/vouchers" attr:class="text-muted-foreground hover:text-foreground">"Verifikationer"</A>
                        <A href="/accounts" attr:class="text-muted-foreground hover:text-foreground">"Kontoplan"</A>
                    </Show>
```

Add `let companies = expect_context::<Companies>();` at the top of `Header`. The `Companies` *page* import collides with the context type's name. Check how `app.rs` already handles this, since both are imported, and follow it. If needed, import the page as `pages::Companies as CompaniesPage`.

Add the route: `<Route path=path!("/accounts") view=|| view! { <SignedIn><Accounts /></SignedIn> } />`. In `pages/mod.rs`, add `mod accounts;` and `pub use accounts::Accounts;`.

- [ ] **Step 5: Write the accounts page**

`crates/web/src/pages/accounts.rs`:

```rust
//! The active company's chart of accounts: add, rename, (de)activate.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::ui::{
    Button, Card, Checkbox, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, TextInput, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn Accounts() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let show_inactive = RwSignal::new(false);
    let number = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            match ledger_api().list_accounts(lpb::ListAccountsRequest { company_id }).await {
                Ok(response) => accounts.set(response.into_inner().accounts),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        load();
    });
    let changed = Callback::new(move |()| load());

    let add = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = lpb::AddAccountRequest {
                company_id: companies.active.get_untracked(),
                // Not a number: 0, which the server refuses with its own message.
                number: number.get_untracked().trim().parse().unwrap_or(0),
                name: name.get_untracked(),
            };
            match ledger_api().add_account(request).await {
                Ok(_) => {
                    number.set(String::new());
                    name.set(String::new());
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <h1 class="text-sm font-medium">"Kontoplan"</h1>
            <ErrorAlert message=error />
            <Card title="Lägg till konto">
                <form class="grid grid-cols-[8rem_1fr_auto] items-end gap-4" novalidate on:submit=add>
                    <Field label="Nummer" id="account_number" value=number />
                    <Field label="Namn" id="account_name" value=name />
                    <Button disabled=busy>"Lägg till konto"</Button>
                </form>
            </Card>
            <Checkbox label="Visa inaktiva" id="show_inactive" checked=show_inactive />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            accounts
                                .get()
                                .into_iter()
                                .filter(|a| a.active || show_inactive.get())
                                .collect::<Vec<_>>()
                        }
                        key=|a| (a.number, a.name.clone(), a.active)
                        let(account)
                    >
                        <AccountRow account=account changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn AccountRow(account: lpb::Account, changed: Callback<()>, error: RwSignal<Option<String>>) -> impl IntoView {
    let companies = expect_context::<Companies>();
    let lpb::Account { number, name: current, active } = account;
    let editing = RwSignal::new(false);
    let name = RwSignal::new(current.clone());

    let rename = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        spawn_local(async move {
            let request = lpb::RenameAccountRequest {
                company_id: companies.active.get_untracked(),
                number,
                name: name.get_untracked(),
            };
            match ledger_api().rename_account(request).await {
                Ok(_) => {
                    editing.set(false);
                    changed.run(());
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = lpb::SetAccountActiveRequest {
                company_id: companies.active.get_untracked(),
                number,
                active: !active,
            };
            match ledger_api().set_account_active(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>
                <Show
                    when=move || editing.get()
                    fallback={
                        let current = current.clone();
                        move || current.clone()
                    }
                >
                    <form class="flex gap-2" novalidate on:submit=rename>
                        <TextInput label=format!("Nytt namn för {number}") value=name />
                        <Button>"Spara"</Button>
                    </form>
                </Show>
            </td>
            <td class=TABLE_CELL>{if active { "Aktivt" } else { "Inaktivt" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| editing.set(true)>
                    "Byt namn"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}
```

The e2e matches rows by accessible name, for example `/^1930 Företagskonto/`. A row's name is its cells' text joined with spaces. If Playwright doesn't produce that name, match with `page.getByRole("row").filter({ hasText: "1930" })` instead, and change the e2e accordingly.

- [ ] **Step 6: Run the e2e and the unit tests**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: everything passes, including the existing company specs that now import `addCompany` from the fixtures.

- [ ] **Step 7: Commit**

```bash
git add crates/web e2e
git commit -m "Keep the chart of accounts in the web app and follow other tabs"
```

---

### Task 9: The voucher pages

**Files:**
- Create: `crates/web/src/pages/vouchers.rs`, `crates/web/src/pages/new_voucher.rs`
- Modify: `crates/web/src/app.rs`, `crates/web/src/pages/mod.rs`, `e2e/tests/ledger.spec.ts`

**Interfaces:**
- Consumes: Tasks 7–8.
- Produces: the routes `/vouchers` (`Vouchers`) and `/vouchers/new` (`NewVoucher`), and `new_voucher::account_number(&str) -> u32`.

- [ ] **Step 1: Write the failing tests**

Append to `e2e/tests/ledger.spec.ts`:

```ts
async function bookSale(page: import("@playwright/test").Page, text: string, kronor: string) {
  await page.getByRole("link", { name: "Ny verifikation" }).click();
  await page.getByLabel("Text").fill(text);
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001 Försäljning inom Sverige, 25 % moms");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
}

test("a voucher is booked, listed and corrected", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();

  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await expect(page.getByLabel("Text")).toHaveValue("");

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  const first = page.getByRole("row", { name: /^1 / });
  await expect(first).toContainText("Försäljning kassa");
  await expect(first).toContainText("1 250,00");
  await first.getByRole("button", { name: "1" }).click();
  await expect(page.getByText("3001 Försäljning inom Sverige, 25 % moms")).toBeVisible();

  await first.getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(page.getByRole("row", { name: /^1 / })).toContainText("Rättad av ver 2");
  await expect(page.getByRole("row", { name: /^2 / })).toContainText("Rättelse av ver 1");
  await expect(page.getByRole("row", { name: /^1 / }).getByRole("button", { name: "Rätta" })).toHaveCount(0);
});

test("an unbalanced voucher is refused in Swedish and nothing is booked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers/new`);

  await page.getByLabel("Text").fill("Fel");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("100");
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill("99");
  await expect(page.getByText("Differens 1,00")).toBeVisible();
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("alert")).toHaveText("Debet och kredit måste vara lika stora.");

  await page.getByLabel("Kredit, rad 2").fill("1oo");
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("alert")).toHaveText("Skriv beloppen som 1 234,50.");

  await page.goto(`${app}/vouchers`);
  await expect(page.getByRole("row", { name: /^1 / })).toHaveCount(0);
});

test("an added account can be used in a voucher", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/accounts`);
  await page.getByLabel("Nummer").fill("1931");
  await page.getByLabel("Namn").fill("Sparkonto");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto/ })).toBeVisible();

  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Text").fill("Överföring");
  await page.getByLabel("Konto, rad 1").fill("1931 Sparkonto");
  await page.getByLabel("Debet, rad 1").fill("500");
  await page.getByLabel("Konto, rad 2").fill("1930");
  await page.getByLabel("Kredit, rad 2").fill("500");
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
});
```

Add a unit test module at the bottom of `new_voucher.rs`. It will not compile until Step 3, which is the "red" step:

```rust
#[cfg(test)]
mod tests {
    use super::account_number;

    #[test]
    fn account_number_takes_the_leading_digits() {
        assert_eq!(account_number("1930"), 1930);
        assert_eq!(account_number(" 1930 Företagskonto/checkkonto/affärskonto"), 1930);
        assert_eq!(account_number("Företagskonto"), 0);
        assert_eq!(account_number(""), 0);
        assert_eq!(account_number("19x0"), 0);
    }
}
```

Run: `cargo test -p doris-web` (expected: does not compile, `account_number` missing) and the e2e (expected: the three new tests fail).

- [ ] **Step 2: Routes**

In `pages/mod.rs`, add `mod new_voucher;`, `mod vouchers;`, `pub use new_voucher::NewVoucher;` and `pub use vouchers::Vouchers;`. In `app.rs`, import them and add:

```rust
                        <Route path=path!("/vouchers") view=|| view! { <SignedIn><Vouchers /></SignedIn> } />
                        <Route path=path!("/vouchers/new") view=|| view! { <SignedIn><NewVoucher /></SignedIn> } />
```

- [ ] **Step 3: The new-voucher page**

`crates/web/src/pages/new_voucher.rs`:

```rust
//! Book a voucher in the active company. The server decides its number.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::{amount, parse_amount, today};
use crate::ui::{Button, Card, ErrorAlert, Field, TextInput, Variant};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The account number at the start of what was typed or picked from the
/// list ("1930 Företagskonto" → 1930). 0 when there is none; the server
/// refuses it as an unknown account.
pub fn account_number(raw: &str) -> u32 {
    raw.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// An empty amount field is 0; anything else must parse.
fn field_amount(raw: &str) -> Option<i64> {
    if raw.trim().is_empty() { Some(0) } else { parse_amount(raw) }
}

#[derive(Clone, Copy)]
struct Line {
    id: usize,
    account: RwSignal<String>,
    debit: RwSignal<String>,
    credit: RwSignal<String>,
}

impl Line {
    fn new(id: usize) -> Self {
        Self { id, account: RwSignal::new(String::new()), debit: RwSignal::new(String::new()), credit: RwSignal::new(String::new()) }
    }

    fn is_blank(&self) -> bool {
        [self.account, self.debit, self.credit].iter().all(|s| s.get_untracked().trim().is_empty())
    }
}

#[component]
pub fn NewVoucher() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let date = RwSignal::new(today());
    let text = RwSignal::new(String::new());
    let next_id = StoredValue::new(2_usize);
    let lines = RwSignal::new(vec![Line::new(0), Line::new(1)]);
    let error = RwSignal::new(None::<String>);
    let booked = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    Effect::new(move |_| {
        let company_id = companies.active.get();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            if let Ok(response) = ledger_api().list_accounts(lpb::ListAccountsRequest { company_id }).await {
                accounts.set(response.into_inner().accounts);
            }
        });
    });

    let add_line = move |_| {
        let id = next_id.get_value();
        next_id.set_value(id + 1);
        lines.update(|l| l.push(Line::new(id)));
    };
    let totals = move || {
        lines.with(|l| {
            l.iter().fold((0_i64, 0_i64), |(d, c), line| {
                (
                    d + line.debit.with(|s| field_amount(s)).unwrap_or(0),
                    c + line.credit.with(|s| field_amount(s)).unwrap_or(0),
                )
            })
        })
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        booked.set(None);
        let mut request_lines = Vec::new();
        for line in lines.get_untracked().iter().filter(|l| !l.is_blank()) {
            let (Some(debit), Some(credit)) =
                (field_amount(&line.debit.get_untracked()), field_amount(&line.credit.get_untracked()))
            else {
                return error.set(Some("Skriv beloppen som 1 234,50.".into()));
            };
            request_lines.push(lpb::VoucherLine { account: account_number(&line.account.get_untracked()), debit, credit });
        }
        busy.set(true);
        spawn_local(async move {
            let request = lpb::RecordVoucherRequest {
                company_id: companies.active.get_untracked(),
                date: date.get_untracked(),
                text: text.get_untracked(),
                lines: request_lines,
            };
            match ledger_api().record_voucher(request).await {
                Ok(response) => {
                    booked.set(Some(format!("Verifikation {} bokförd", response.into_inner().number)));
                    text.set(String::new());
                    let id = next_id.get_value();
                    next_id.set_value(id + 2);
                    lines.set(vec![Line::new(id), Line::new(id + 1)]);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <Card title="Ny verifikation">
            <form class="grid gap-4" novalidate on:submit=submit>
                <ErrorAlert message=error />
                {move || booked.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
                <div class="grid grid-cols-[10rem_1fr] gap-4">
                    <Field label="Datum" id="voucher_date" kind="date" value=date />
                    <Field label="Text" id="voucher_text" value=text />
                </div>
                <datalist id="accounts">
                    {move || {
                        accounts
                            .get()
                            .into_iter()
                            .filter(|a| a.active)
                            .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                            .collect_view()
                    }}
                </datalist>
                <div class="grid gap-2">
                    <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2 text-muted-foreground">
                        <span>"Konto"</span>
                        <span>"Debet"</span>
                        <span>"Kredit"</span>
                        <span></span>
                    </div>
                    <For each=move || lines.get().into_iter().enumerate().collect::<Vec<_>>() key=|(_, l)| l.id let((index, line))>
                        <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2">
                            <TextInput label=format!("Konto, rad {}", index + 1) value=line.account list="accounts" />
                            <TextInput label=format!("Debet, rad {}", index + 1) value=line.debit inputmode="decimal" />
                            <TextInput label=format!("Kredit, rad {}", index + 1) value=line.credit inputmode="decimal" />
                            <Button
                                variant=Variant::Ghost
                                kind="button"
                                on:click=move |_| lines.update(|l| l.retain(|other| other.id != line.id))
                            >
                                "Ta bort"
                            </Button>
                        </div>
                    </For>
                    <div>
                        <Button variant=Variant::Ghost kind="button" on:click=add_line>"Lägg till rad"</Button>
                    </div>
                </div>
                <p class="text-xs/relaxed text-muted-foreground">
                    {move || {
                        let (debit, credit) = totals();
                        format!("Debet {} · Kredit {} · Differens {}", amount(debit), amount(credit), amount(debit - credit))
                    }}
                </p>
                <Button disabled=busy>"Bokför"</Button>
            </form>
        </Card>
    }
}
```

The row labels are positional ("rad 1", "rad 2"). After a middle row is removed they renumber, which matches what a screen-reader user sees.

- [ ] **Step 4: The grundbok page**

`crates/web/src/pages/vouchers.rs`:

```rust
//! The grundbok: the active company's vouchers for one fiscal year.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::{amount, today};
use crate::ui::{
    Button, ErrorAlert, SELECT_OPTION, Select, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use std::collections::HashMap;

#[component]
pub fn Vouchers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let years = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    let year = RwSignal::new(String::new());
    let vouchers = RwSignal::new(Vec::<lpb::Voucher>::new());
    let names = RwSignal::new(HashMap::<u32, String>::new());
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let company_id = companies.active.get();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = ledger_api();
            match api.list_fiscal_years(lpb::ListFiscalYearsRequest { company_id: company_id.clone() }).await {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    year.set(list.first().map(|y| y.start.clone()).unwrap_or_default());
                    years.set(list);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            if let Ok(response) = api.list_accounts(lpb::ListAccountsRequest { company_id }).await {
                names.set(response.into_inner().accounts.into_iter().map(|a| (a.number, a.name)).collect());
            }
        });
    });
    let load = move || {
        let (company_id, fiscal_year_start) = (companies.active.get_untracked(), year.get_untracked());
        if company_id.is_empty() || fiscal_year_start.is_empty() {
            return;
        }
        spawn_local(async move {
            match ledger_api().list_vouchers(lpb::ListVouchersRequest { company_id, fiscal_year_start }).await {
                Ok(response) => vouchers.set(response.into_inner().vouchers),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        year.track();
        load();
    });
    let changed = Callback::new(move |()| load());
    let fiscal_year = Signal::derive(move || years.get().into_iter().find(|y| y.start == year.get()));

    view! {
        <div class="grid gap-6">
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">"Verifikationer"</h1>
                <A href="/vouchers/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ny verifikation"</A>
            </div>
            <ErrorAlert message=error />
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
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Datum"</th>
                        <th class=TABLE_HEADER_CELL>"Text"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Belopp"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || vouchers.get() key=|v| (v.number, v.corrected_by) let(voucher)>
                        <VoucherRow voucher=voucher fiscal_year=fiscal_year names=names changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn VoucherRow(
    voucher: lpb::Voucher,
    fiscal_year: Signal<Option<lpb::FiscalYear>>,
    names: RwSignal<HashMap<u32, String>>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let companies = expect_context::<Companies>();
    let expanded = RwSignal::new(false);
    let correcting = RwSignal::new(false);
    let date = RwSignal::new(String::new());
    let number = voucher.number;
    let total: i64 = voucher.lines.iter().map(|l| l.debit).sum();
    let status = match (voucher.corrects, voucher.corrected_by) {
        (_, by) if by != 0 => format!("Rättad av ver {by}"),
        (of, _) if of != 0 => format!("Rättelse av ver {of}"),
        _ => String::new(),
    };
    let can_correct = voucher.corrects == 0 && voucher.corrected_by == 0;
    let lines = voucher.lines.clone();

    let start_correction = move |_| {
        // Today, or the year's last day once the year is over.
        let end = fiscal_year.get_untracked().map(|y| y.end).unwrap_or_default();
        let today = today();
        date.set(if !end.is_empty() && today > end { end } else { today });
        correcting.set(true);
    };
    let confirm = move |_| {
        error.set(None);
        let Some(year) = fiscal_year.get_untracked() else { return };
        spawn_local(async move {
            let request = lpb::CorrectVoucherRequest {
                company_id: companies.active.get_untracked(),
                fiscal_year_start: year.start,
                number,
                date: date.get_untracked(),
            };
            match ledger_api().correct_voucher(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>
                <button type="button" aria-expanded=move || expanded.get().to_string() on:click=move |_| expanded.update(|e| *e = !*e)>
                    {number}
                </button>
            </td>
            <td class=TABLE_CELL>{voucher.date.clone()}</td>
            <td class=TABLE_CELL>{voucher.text.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(total)}</td>
            <td class=TABLE_CELL>{status}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || can_correct && !correcting.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=start_correction>"Rätta"</Button>
                </Show>
                <Show when=move || correcting.get()>
                    <span class="inline-flex items-center gap-2">
                        <TextInput label=format!("Datum för rättelse av ver {number}") value=date kind="date" />
                        <Button kind="button" on:click=confirm>"Bekräfta rättelse"</Button>
                    </span>
                </Show>
            </td>
        </tr>
        <Show when=move || expanded.get()>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL></td>
                <td class=TABLE_CELL colspan="5">
                    <ul class="grid gap-1">
                        {lines
                            .iter()
                            .map(|l| {
                                let name = names.with(|n| n.get(&l.account).cloned().unwrap_or_default());
                                let side = if l.debit > 0 {
                                    format!("Debet {}", amount(l.debit))
                                } else {
                                    format!("Kredit {}", amount(l.credit))
                                };
                                view! {
                                    <li class="flex justify-between gap-4">
                                        <span>{format!("{} {}", l.account, name)}</span>
                                        <span class="tabular-nums">{side}</span>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                </td>
            </tr>
        </Show>
    }
}
```

The e2e matches `getByRole("row", { name: /^1 / })`. The row's accessible name begins with the button's text, "1". If Playwright computes it differently, use `page.getByRole("row").filter({ has: page.getByRole("button", { name: "1", exact: true }) })` and change the e2e accordingly.

- [ ] **Step 5: Run everything for the web**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: everything passes.

- [ ] **Step 6: Check the wasm budget**

Run: `make dist`
Expected: passes the `WASM_BUDGET` check (900 000). Report the size and how much it grew since Task 7 Step 0. If it fails, stop and report. Don't raise the budget further.

- [ ] **Step 7: Commit**

```bash
git add crates/web e2e
git commit -m "Book, list and correct vouchers in the web app"
```

---

### Task 10: Docs and the full check

**Files:**
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: everything.
- Produces: nothing.

- [ ] **Step 1: Update `AGENTS.md`**

- **Layout:** add `crates/ledger     doris-ledger: chart of accounts and vouchers (verifikationer)` after `crates/identity`.
- **Under "Event sourcing rules"**, add:

```markdown
- Voucher numbers run 1..=n per company and fiscal year without gaps (BFL
  5 kap.). The number is decided inside the write transaction
  (`last_number + 1`), never by the client and never ahead of time; the
  `vouchers` projection's primary key and the `vouchers_numbered_without_gaps`
  trigger back that up. `crates/ledger/tests/stress.rs` must keep passing.
- A voucher is never changed or removed. A rättelse is a new voucher with
  every line reversed and `corrects` pointing at the original.
```

- **Under "API":** the contract list becomes `proto/doris/auth/v1/auth.proto`, `proto/doris/company/v1/company.proto` and `proto/doris/ledger/v1/ledger.proto`. After the sentence about company codes, add: "Ledger codes are mapped in `crates/server/src/ledger.rs` (`status`, `domain_status`)."
- **Under "Frontend",** in the active company bullet, add: "Other tabs follow a change through the `storage` event, so a stale tab never books in the wrong company."

- [ ] **Step 2: Run the full check**

```bash
make test
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
make dist
make e2e
```

Expected: all green. `make dist` stays under the 900 000-byte budget.

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "Document the ledger and its numbering rule"
```
