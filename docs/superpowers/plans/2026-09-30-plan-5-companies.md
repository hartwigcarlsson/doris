# Plan 5: Companies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A signed-in user adds the companies they keep the books for, and can pre-fill the details from Bolagsverket by organisationsnummer. Each company records the räkenskapsår and bokföringsmetod. A user sees only the companies they are a member of and can add other users as members.

**Architecture:**
- A new crate, `crates/company` (`doris-company`), has the same shape as `doris-identity`:
  - `domain.rs`: pure value types, events, state and decisions
  - `projections.rs`: the `companies` and `company_members` read models
  - `queries.rs`
  - `lib.rs`: load → decide → append → project, in one `BEGIN IMMEDIATE` transaction
- `proto/doris/company/v1/company.proto` defines `CompanyService`. The server implements it in `crates/server/src/company.rs` and mounts it next to `AuthService` on the same `Routes`.
- `crates/server/src/bolagsverket.rs` is the only outbound HTTP client. It uses reqwest with native-tls, which on Linux means the OpenSSL that webauthn-rs already links.
- The frontend gets three pages: `/companies`, `/companies/new` and `/companies/:id`.

**Tech Stack:** Rust 2024, sqlx 0.9 (SQLite), jiff 0.2 (`civil::Date`), tonic/tonic-web 0.14, prost 0.14, reqwest 0.13 (`native-tls`, `json`, `form`), Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-09-30-foretag-design.md`

**Deviations from the spec, decided while planning:**
1. **TLS:** reqwest uses `native-tls`, not rustls. The system OpenSSL is already a build and runtime requirement, whereas rustls would pull in aws-lc-rs and its C build. Task 7 updates the spec.
2. **Bolagsverket URLs:** there are two variables, `DORIS_BOLAGSVERKET_TOKEN_URL` and `DORIS_BOLAGSVERKET_API_URL`, instead of one `_URL`. The token lives on `portal.api.bolagsverket.se` and the API on `gw.api.bolagsverket.se`.
3. **Personnummer are never sent:** an organisationsnummer that is a personnummer (enskild firma, third digit < 2) is never sent to Bolagsverket. The server returns `lookup_personal_number` and the user fills in the details by hand.
4. **Extra error codes:** `invalid_address`, `invalid_legal_form` and `invalid_accounting_method` are added for the proto boundary.

**Bolagsverket API (verified 2026-09-30 from a working integration, GreveHertig/SparkUF#16):**
- Token: `POST https://portal.api.bolagsverket.se/oauth2/token`, form fields `grant_type=client_credentials`, `client_id`, `client_secret`, `scope=vardefulla-datamangder:read`. The response is `{ access_token, scope, token_type: "Bearer", expires_in: 3600 }`.
- Base URL: `https://gw.api.bolagsverket.se/vardefulla-datamangder/v1`. The lookup is `POST /organisationer` with body `{ "identitetsbeteckning": "5560160680" }`, answered with `200 { "organisationer": [ … ] }`.
- Fields in each organisation:
  - `organisationsnamn.organisationsnamnLista[].{namn, organisationsnamntyp.kod}`, where `"FORETAGSNAMN"` is the registered name
  - `organisationsform.kod`, for example `"AB"`
  - `postadressOrganisation.postadress.{utdelningsadress, postnummer, postort}`, where any of them may be `null`
- Errors: a bad check digit gives `400` in Problem Details form. Whether an unknown number gives `404` or an empty list is unverified, so both are treated as not found.

## Global Constraints
- **TDD:** every behavior starts from a failing test that was run and seen to fail. Each red → green cycle ends in a commit.
- **Events:** the `events` table is append-only. Payloads are JSON with `schema_version = 1`. Projections are written in the same transaction as the append and can be rebuilt from `read_all`, which a test covers.
- **Naming:** code, identifiers, proto, event names and commits are in English. Only user-visible text is Swedish.
- **Error statuses** carry stable snake_case codes as the message, and every code gets a line in `crates/web/src/errors.rs`:

  | Code | gRPC | When |
  |---|---|---|
  | `invalid_org_nr` | InvalidArgument | not 10 digits after normalization, or a bad Luhn check digit |
  | `invalid_company_name` | InvalidArgument | empty after trim, or over 200 characters |
  | `invalid_address` | InvalidArgument | an address field over 200 characters |
  | `invalid_legal_form` | InvalidArgument | legal form unspecified |
  | `invalid_accounting_method` | InvalidArgument | method unspecified |
  | `invalid_fiscal_year` | InvalidArgument | an unparseable date, or a breach of the 3 kap. rules |
  | `company_exists` | AlreadyExists | org nr already registered (UNIQUE) |
  | `company_not_found` | NotFound | no such company, the caller isn't a member, or a malformed id |
  | `user_not_found` | NotFound | AddMember: no user with that email |
  | `lookup_unavailable` | FailedPrecondition | Bolagsverket credentials not configured |
  | `lookup_personal_number` | FailedPrecondition | the org nr is a personnummer (enskild firma) |
  | `lookup_not_found` | NotFound | Bolagsverket doesn't know the number |
  | `lookup_failed` | Unavailable | network error or an error response from Bolagsverket |
- **Personal data:** org nr (possibly a personnummer) and email are never logged. Bolagsverket failure reasons are built from HTTP status codes and `reqwest::Error::without_url()` only.
- **Access:** a non-member gets `company_not_found` for every company RPC, exactly as for a nonexistent id.
- **Wasm:** it must stay under `WASM_BUDGET` (800 000 bytes, checked by `make dist`). The only new wasm dependency is `js-sys`, which is already in the lock file.
- **Lints:** `cargo clippy --workspace -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings` pass after every task.

## Review Focus
1. **Org nr typed any way people type it** (`556016-0680`, `5560160680`, `556016 0680`, `16556016-0680`, `19121212-1212`) normalizes to the same 10 digits, and a typo in the check digit is `invalid_org_nr`. *Test: Task 1 `org_nr_accepts_common_spellings`.*
2. **Two users registering the same org nr at the same moment** means one wins and the other gets `company_exists` with nothing half-written. *Test: Task 4 `the_same_org_nr_cannot_be_registered_twice`, which also checks the event count is unchanged.*
3. **A member of company A guessing company B's id, or sending a garbage id,** gets `company_not_found` from Get, ListMembers and AddMember, and AddMember doesn't reveal whether the email exists. *Test: Task 6 `non_members_and_bad_ids_get_company_not_found`.*
4. **Bolagsverket returns `null` for parts of the address or the name list** (it does for Volvo and Ericsson): the lookup still succeeds with the missing fields empty. *Test: Task 7 `parses_a_response_with_nulls`.*
5. **The cached token is revoked or expires early:** the first lookup fails with `lookup_failed`, the cache is cleared, and the next lookup fetches a new token instead of failing forever. *Test: Task 7 `a_401_clears_the_cached_token`.*

---

## File Structure

```
crates/company/Cargo.toml                 new crate doris-company
crates/company/src/lib.rs                 app layer: register_company, add_member, get_company, Error
crates/company/src/domain.rs              OrgNr, CompanyName, Address, LegalForm, AccountingMethod,
                                          FiscalYear, CompanyEvent, Company, decisions
crates/company/src/projections.rs         apply, rebuild_projections
crates/company/src/queries.rs             list_companies, CompanySummary
crates/company/tests/domain.rs            given/when/then, no DB
crates/company/tests/store.rs             SQLite in memory
migrations/0005_companies.sql             companies, company_members
proto/doris/company/v1/company.proto      CompanyService
crates/proto/build.rs, src/lib.rs         compile + expose doris.company.v1
crates/server/src/company.rs              CompanyApi (gRPC → doris_company)
crates/server/src/bolagsverket.rs         OAuth2 + POST /organisationer, response mapping
crates/server/src/grpc.rs                 extract signed_in_user(), make status() pub(crate)
crates/server/src/lib.rs, main.rs         mount CompanyService, Bolagsverket config
crates/server/tests/common/mod.rs         companies() client, start_with_bolagsverket, invite helper
crates/server/tests/companies.rs          integration tests incl. fake Bolagsverket
crates/web/src/api.rs                     company_api()
crates/web/src/errors.rs                  new codes
crates/web/src/format.rs                  org_nr(), legal_form_label(), method label
crates/web/src/ui.rs                      Select, Radio (classes from shadcn preset)
crates/web/src/pages/{companies,new_company,company}.rs
crates/web/src/app.rs                     routes + "Företag" nav link
e2e/tests/companies.spec.ts
Dockerfile                                ca-certificates in the runtime image
AGENTS.md, docs/superpowers/specs/2026-09-30-foretag-design.md
```

---

### Task 1: `doris-company` crate with `OrgNr`

**Files:**
- Create: `crates/company/Cargo.toml`, `crates/company/src/lib.rs`, `crates/company/src/domain.rs`, `crates/company/tests/domain.rs`
- Modify: `Cargo.toml` (workspace members + dependency)

**Interfaces:**
- Produces:
  - `doris_company::domain::DomainError`, with variants `InvalidOrgNr`, `InvalidCompanyName`, `InvalidAddress`, `InvalidFiscalYear` and `NotMember`
  - `OrgNr::parse(&str) -> Result<OrgNr, DomainError>`
  - `OrgNr::as_str(&self) -> &str`, which gives the 10 digits
  - `OrgNr::formatted(&self) -> String`, which gives `NNNNNN-NNNN`
  - `OrgNr::is_personal_identity_number(&self) -> bool`

- [ ] **Step 1: Create the crate and register it**

`Cargo.toml` (workspace): add `"crates/company"` to `members` and `doris-company = { path = "crates/company" }` to `[workspace.dependencies]`.

`crates/company/Cargo.toml`:
```toml
[package]
name = "doris-company"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
doris-eventstore.workspace = true
jiff.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
uuid.workspace = true

[dev-dependencies]
tokio.workspace = true
```

`crates/company/src/lib.rs`:
```rust
//! Companies a user keeps the books for, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
```

`crates/company/src/domain.rs`:
```rust
//! Pure company rules: value types, events, state and decisions. No I/O.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid organisationsnummer")]
    InvalidOrgNr,
    #[error("company name must be 1-200 characters")]
    InvalidCompanyName,
    #[error("address fields must be at most 200 characters")]
    InvalidAddress,
    #[error("fiscal year breaks BFL 3 kap.")]
    InvalidFiscalYear,
    #[error("not a member of the company")]
    NotMember,
}
```

- [ ] **Step 2: Write the failing tests**

`crates/company/tests/domain.rs`:
```rust
use doris_company::domain::*;

#[test]
fn org_nr_accepts_common_spellings() {
    for raw in ["556016-0680", "5560160680", " 556016 0680 ", "16556016-0680", "165560160680"] {
        let org_nr = OrgNr::parse(raw).unwrap();
        assert_eq!(org_nr.as_str(), "5560160680", "{raw:?}");
        assert_eq!(org_nr.formatted(), "556016-0680");
    }
}

#[test]
fn org_nr_accepts_a_personnummer_for_enskild_firma() {
    let org_nr = OrgNr::parse("19121212-1212").unwrap();
    assert_eq!(org_nr.as_str(), "1212121212");
    assert!(org_nr.is_personal_identity_number());
    assert!(!OrgNr::parse("556016-0680").unwrap().is_personal_identity_number());
}

#[test]
fn org_nr_rejects_bad_check_digits_and_formats() {
    for raw in ["", "556016-0681", "5599999999", "55601606", "55601606800", "556016-068O", "185560160680"] {
        assert_eq!(OrgNr::parse(raw), Err(DomainError::InvalidOrgNr), "{raw:?}");
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p doris-company --test domain`
Expected: compile error `cannot find type OrgNr`

- [ ] **Step 4: Implement `OrgNr`** (append to `domain.rs`)

```rust
/// Organisationsnummer: 10 digits, no hyphen. For an enskild firma it is the
/// owner's personnummer, so it is personal data and must never be logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgNr(String);

impl OrgNr {
    /// Accepts `NNNNNN-NNNN`, spaces, and the 12-digit form with a century
    /// (`16`, `19` or `20`) in front.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let digits: String = raw.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
        // is_ascii first: slicing by byte index must not split a char.
        let digits = match digits.len() {
            12 if digits.is_ascii() && ["16", "19", "20"].contains(&&digits[..2]) => {
                digits[2..].to_owned()
            }
            _ => digits,
        };
        if digits.len() == 10 && digits.bytes().all(|b| b.is_ascii_digit()) && luhn(&digits) {
            Ok(Self(digits))
        } else {
            Err(DomainError::InvalidOrgNr)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn formatted(&self) -> String {
        format!("{}-{}", &self.0[..6], &self.0[6..])
    }

    /// Organisationsnummer have 2 or more as their third digit; a lower
    /// digit is a birth month, so this is a person's personnummer.
    pub fn is_personal_identity_number(&self) -> bool {
        self.0.as_bytes()[2] < b'2'
    }
}

/// The Luhn check (weights 2,1,2,1…) over all ten digits.
fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0') * if i % 2 == 0 { 2 } else { 1 };
            if d > 9 { d - 9 } else { d }
        })
        .sum();
    sum % 10 == 0
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p doris-company --test domain`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/company
git commit -m "Add doris-company crate with organisationsnummer validation"
```

---

### Task 2: Company value types and BFL fiscal-year rules

**Files:**
- Modify: `crates/company/src/domain.rs`
- Test: `crates/company/tests/domain.rs`

**Interfaces:**
- Produces:
  - `CompanyName::parse(&str) -> Result<CompanyName, DomainError>`, `.as_str()`
  - `Address { street, postal_code, city: Option<String> }` (Default), and `Address::parse(street, postal_code, city: &str) -> Result<Address, DomainError>`
  - `LegalForm`, with variants `Aktiebolag`, `Handelsbolag`, `Kommanditbolag`, `EnskildFirma`, `EkonomiskForening`, `IdeellForening`, `Stiftelse` and `Other`; `.as_str()` returns snake_case
  - `AccountingMethod`, with variants `Cash` and `Invoice`; `.as_str()` returns `"cash"` or `"invoice"`
  - `FiscalYear { start: Date, end: Date }`
  - `FiscalYear::first(start, end, LegalForm) -> Result<FiscalYear, DomainError>`
  - `FiscalYear::next(&self) -> FiscalYear`
  - `FiscalYear::containing(&self, day: Date) -> FiscalYear`

- [ ] **Step 1: Write the failing tests** (append to `tests/domain.rs`)

```rust
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

#[test]
fn company_name_is_trimmed_and_1_to_200_characters() {
    assert_eq!(CompanyName::parse("  Exempel AB ").unwrap().as_str(), "Exempel AB");
    assert!(CompanyName::parse(&"å".repeat(200)).is_ok());
    assert_eq!(CompanyName::parse("  "), Err(DomainError::InvalidCompanyName));
    assert_eq!(CompanyName::parse(&"å".repeat(201)), Err(DomainError::InvalidCompanyName));
}

#[test]
fn address_fields_are_trimmed_and_empty_ones_dropped() {
    let address = Address::parse(" Storgatan 1 ", "", " Stockholm").unwrap();
    assert_eq!(address.street.as_deref(), Some("Storgatan 1"));
    assert_eq!(address.postal_code, None);
    assert_eq!(address.city.as_deref(), Some("Stockholm"));
    assert_eq!(Address::parse(&"a".repeat(201), "", ""), Err(DomainError::InvalidAddress));
}

#[test]
fn a_calendar_year_is_a_valid_first_fiscal_year() {
    let year = FiscalYear::first(d("2026-01-01"), d("2026-12-31"), LegalForm::Aktiebolag).unwrap();
    assert_eq!((year.start, year.end), (d("2026-01-01"), d("2026-12-31")));
}

#[test]
fn a_first_fiscal_year_may_be_short_or_extended_up_to_18_months() {
    for (start, end) in [
        ("2026-10-01", "2026-10-31"), // 1 month
        ("2026-07-01", "2027-12-31"), // 18 months
        ("2026-05-01", "2027-04-30"), // broken year
        ("2027-03-01", "2028-02-29"), // ends on a leap day
    ] {
        assert!(FiscalYear::first(d(start), d(end), LegalForm::Aktiebolag).is_ok(), "{start}–{end}");
    }
}

#[test]
fn a_first_fiscal_year_must_follow_bfl_3_kap() {
    for (start, end) in [
        ("2026-01-02", "2026-12-31"), // not the first of a month
        ("2026-01-01", "2026-12-30"), // not the last of a month
        ("2026-07-01", "2028-01-31"), // 19 months
        ("2026-12-01", "2026-11-30"), // ends before it starts
        ("2027-03-01", "2028-02-28"), // 2028 is a leap year: not month end
    ] {
        assert_eq!(
            FiscalYear::first(d(start), d(end), LegalForm::Aktiebolag),
            Err(DomainError::InvalidFiscalYear),
            "{start}–{end}"
        );
    }
}

#[test]
fn enskild_firma_and_handelsbolag_must_use_the_calendar_year() {
    for form in [LegalForm::EnskildFirma, LegalForm::Handelsbolag, LegalForm::Kommanditbolag] {
        assert_eq!(
            FiscalYear::first(d("2026-05-01"), d("2027-04-30"), form),
            Err(DomainError::InvalidFiscalYear),
            "{form:?}"
        );
        // Starting mid-year is fine as long as the year ends 31 December.
        assert!(FiscalYear::first(d("2026-06-01"), d("2026-12-31"), form).is_ok());
    }
}

#[test]
fn later_fiscal_years_are_12_months_ending_in_the_same_month() {
    let first = FiscalYear::first(d("2026-07-01"), d("2027-12-31"), LegalForm::Aktiebolag).unwrap();
    let broken = FiscalYear::first(d("2026-05-01"), d("2027-04-30"), LegalForm::Aktiebolag).unwrap();

    assert_eq!(first.next(), FiscalYear { start: d("2028-01-01"), end: d("2028-12-31") });
    assert_eq!(broken.next(), FiscalYear { start: d("2027-05-01"), end: d("2028-04-30") });
    assert_eq!(first.containing(d("2026-09-30")), first);
    assert_eq!(first.containing(d("2026-01-15")), first); // before the company existed
    assert_eq!(broken.containing(d("2029-02-28")).start, d("2028-05-01"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-company --test domain`
Expected: compile errors for `CompanyName`, `Address`, `FiscalYear` and `LegalForm`

- [ ] **Step 3: Implement** (append to `domain.rs`; add `use jiff::civil::Date; use jiff::Span;` at the top)

```rust
fn bounded_text(raw: &str, max_chars: usize) -> Option<String> {
    let text = raw.trim();
    (!text.is_empty() && text.chars().count() <= max_chars).then(|| text.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompanyName(String);

impl CompanyName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded_text(raw, 200).map(Self).ok_or(DomainError::InvalidCompanyName)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Postal address. Every part is optional: Bolagsverket often has only
/// postnummer and postort.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub street: Option<String>,
    pub postal_code: Option<String>,
    pub city: Option<String>,
}

impl Address {
    pub fn parse(street: &str, postal_code: &str, city: &str) -> Result<Self, DomainError> {
        let part = |raw: &str| {
            let text = raw.trim();
            match text.chars().count() {
                0 => Ok(None),
                1..=200 => Ok(Some(text.to_owned())),
                _ => Err(DomainError::InvalidAddress),
            }
        };
        Ok(Self { street: part(street)?, postal_code: part(postal_code)?, city: part(city)? })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalForm {
    Aktiebolag,
    Handelsbolag,
    Kommanditbolag,
    EnskildFirma,
    EkonomiskForening,
    IdeellForening,
    Stiftelse,
    Other,
}

impl LegalForm {
    pub fn as_str(self) -> &'static str {
        match self {
            LegalForm::Aktiebolag => "aktiebolag",
            LegalForm::Handelsbolag => "handelsbolag",
            LegalForm::Kommanditbolag => "kommanditbolag",
            LegalForm::EnskildFirma => "enskild_firma",
            LegalForm::EkonomiskForening => "ekonomisk_forening",
            LegalForm::IdeellForening => "ideell_forening",
            LegalForm::Stiftelse => "stiftelse",
            LegalForm::Other => "other",
        }
    }

    /// BFL 3 kap. 1 §: a natural person and a handelsbolag must use the
    /// calendar year. (The exception for handelsbolag owned by legal persons
    /// is not supported yet.)
    fn requires_calendar_year(self) -> bool {
        matches!(self, LegalForm::EnskildFirma | LegalForm::Handelsbolag | LegalForm::Kommanditbolag)
    }
}

/// Kontantmetoden (BFL 5 kap. 2 §, net sales up to 3 MSEK) or faktureringsmetoden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountingMethod {
    Cash,
    Invoice,
}

impl AccountingMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountingMethod::Cash => "cash",
            AccountingMethod::Invoice => "invoice",
        }
    }
}

/// A räkenskapsår: from the first day of a month to the last day of a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiscalYear {
    pub start: Date,
    pub end: Date,
}

impl FiscalYear {
    /// The company's first räkenskapsår (BFL 3 kap. 1 och 3 §§): 1-18
    /// months, whole months, and ending 31 December where the legal form
    /// requires the calendar year.
    pub fn first(start: Date, end: Date, legal_form: LegalForm) -> Result<Self, DomainError> {
        let months = (i32::from(end.year()) * 12 + i32::from(end.month()))
            - (i32::from(start.year()) * 12 + i32::from(start.month()))
            + 1;
        let valid = start == start.first_of_month()
            && end == end.last_of_month()
            && (1..=18).contains(&months)
            && (!legal_form.requires_calendar_year() || end.month() == 12);
        if valid { Ok(Self { start, end }) } else { Err(DomainError::InvalidFiscalYear) }
    }

    /// The following räkenskapsår: 12 months, ending in the same month.
    pub fn next(&self) -> Self {
        let start = self.end.tomorrow().expect("fiscal years are far from the date limits");
        let end = start
            .checked_add(Span::new().months(11))
            .expect("fiscal years are far from the date limits")
            .last_of_month();
        Self { start, end }
    }

    /// The räkenskapsår `day` falls in, counting from this (first) one.
    /// Days before the first year give the first year.
    pub fn containing(&self, day: Date) -> Self {
        let mut year = *self;
        while day > year.end {
            year = year.next();
        }
        year
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-company --test domain`
Expected: all passed

- [ ] **Step 5: Commit**

```bash
git add crates/company
git commit -m "Add company value types and BFL fiscal-year rules"
```

---

### Task 3: Company events, state and decisions

**Files:**
- Modify: `crates/company/src/domain.rs`
- Test: `crates/company/tests/domain.rs`

**Interfaces:**
- Produces:
  - `CompanyEvent` (serde `tag = "type"`), with variants:
    - `CompanyRegistered { company_id: Uuid, org_nr: OrgNr, name: CompanyName, legal_form: LegalForm, address: Address, first_fiscal_year: FiscalYear, accounting_method: AccountingMethod, created_by: Uuid }`
    - `MemberAdded { user_id: Uuid, added_by: Uuid }`
  - `Company { id, org_nr, name, legal_form, address, first_fiscal_year, accounting_method, members: Vec<Uuid> }`
  - `Company::from_events(&[CompanyEvent]) -> Option<Company>`, `Company::is_member(&self, Uuid) -> bool`
  - `RegisterCompany { company_id, org_nr, name, legal_form, address, first_fiscal_year, accounting_method }`
  - `register_company(cmd: RegisterCompany, created_by: Uuid) -> Vec<CompanyEvent>`
  - `add_member(company: &Company, actor: Uuid, user_id: Uuid) -> Result<Vec<CompanyEvent>, DomainError>`

- [ ] **Step 1: Write the failing tests** (append; add `use uuid::Uuid;`)

```rust
fn registered(creator: Uuid) -> (Company, Vec<CompanyEvent>) {
    let events = register_company(
        RegisterCompany {
            company_id: Uuid::new_v4(),
            org_nr: OrgNr::parse("556016-0680").unwrap(),
            name: CompanyName::parse("Exempel AB").unwrap(),
            legal_form: LegalForm::Aktiebolag,
            address: Address::default(),
            first_fiscal_year: FiscalYear::first(d("2026-01-01"), d("2026-12-31"), LegalForm::Aktiebolag).unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
        creator,
    );
    (Company::from_events(&events).unwrap(), events)
}

#[test]
fn registering_a_company_makes_the_creator_its_first_member() {
    let anna = Uuid::new_v4();
    let (company, events) = registered(anna);

    assert!(matches!(events[0], CompanyEvent::CompanyRegistered { created_by, .. } if created_by == anna));
    assert_eq!(events[1], CompanyEvent::MemberAdded { user_id: anna, added_by: anna });
    assert_eq!(company.members, vec![anna]);
    assert_eq!(company.name.as_str(), "Exempel AB");
}

#[test]
fn a_member_adds_another_user_once() {
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let (company, mut events) = registered(anna);

    let added = add_member(&company, anna, bo).unwrap();
    events.extend(added.clone());
    let company = Company::from_events(&events).unwrap();

    assert_eq!(added, vec![CompanyEvent::MemberAdded { user_id: bo, added_by: anna }]);
    assert!(company.is_member(bo));
    assert_eq!(add_member(&company, anna, bo).unwrap(), vec![]);
}

#[test]
fn a_non_member_cannot_add_members() {
    let (company, _) = registered(Uuid::new_v4());
    let stranger = Uuid::new_v4();
    assert_eq!(add_member(&company, stranger, stranger), Err(DomainError::NotMember));
}

#[test]
fn events_round_trip_through_json_with_readable_dates() {
    let (_, events) = registered(Uuid::new_v4());
    let json = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(json["type"], "CompanyRegistered");
    assert_eq!(json["org_nr"], "5560160680");
    assert_eq!(json["first_fiscal_year"]["start"], "2026-01-01");
    assert_eq!(json["accounting_method"], "invoice");
    assert_eq!(serde_json::from_value::<CompanyEvent>(json).unwrap(), events[0]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-company --test domain`
Expected: compile errors for `register_company` and `CompanyEvent`

- [ ] **Step 3: Implement** (append to `domain.rs`; add `use uuid::Uuid;`)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompanyEvent {
    CompanyRegistered {
        company_id: Uuid,
        org_nr: OrgNr,
        name: CompanyName,
        legal_form: LegalForm,
        address: Address,
        first_fiscal_year: FiscalYear,
        accounting_method: AccountingMethod,
        created_by: Uuid,
    },
    MemberAdded {
        user_id: Uuid,
        added_by: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Company {
    pub id: Uuid,
    pub org_nr: OrgNr,
    pub name: CompanyName,
    pub legal_form: LegalForm,
    pub address: Address,
    pub first_fiscal_year: FiscalYear,
    pub accounting_method: AccountingMethod,
    pub members: Vec<Uuid>,
}

impl Company {
    pub fn from_events(events: &[CompanyEvent]) -> Option<Self> {
        let mut company = None;
        for event in events {
            match event.clone() {
                CompanyEvent::CompanyRegistered {
                    company_id, org_nr, name, legal_form, address,
                    first_fiscal_year, accounting_method, ..
                } => {
                    company = Some(Company {
                        id: company_id, org_nr, name, legal_form, address,
                        first_fiscal_year, accounting_method, members: vec![],
                    });
                }
                CompanyEvent::MemberAdded { user_id, .. } => {
                    if let Some(c) = company.as_mut() {
                        c.members.push(user_id);
                    }
                }
            }
        }
        company
    }

    pub fn is_member(&self, user_id: Uuid) -> bool {
        self.members.contains(&user_id)
    }
}

#[derive(Debug, Clone)]
pub struct RegisterCompany {
    pub company_id: Uuid,
    pub org_nr: OrgNr,
    pub name: CompanyName,
    pub legal_form: LegalForm,
    pub address: Address,
    pub first_fiscal_year: FiscalYear,
    pub accounting_method: AccountingMethod,
}

/// The creator becomes the first member, so membership has one source.
pub fn register_company(cmd: RegisterCompany, created_by: Uuid) -> Vec<CompanyEvent> {
    vec![
        CompanyEvent::CompanyRegistered {
            company_id: cmd.company_id,
            org_nr: cmd.org_nr,
            name: cmd.name,
            legal_form: cmd.legal_form,
            address: cmd.address,
            first_fiscal_year: cmd.first_fiscal_year,
            accounting_method: cmd.accounting_method,
            created_by,
        },
        CompanyEvent::MemberAdded { user_id: created_by, added_by: created_by },
    ]
}

/// Idempotent: adding an existing member yields no events.
pub fn add_member(company: &Company, actor: Uuid, user_id: Uuid) -> Result<Vec<CompanyEvent>, DomainError> {
    if !company.is_member(actor) {
        return Err(DomainError::NotMember);
    }
    if company.is_member(user_id) {
        return Ok(vec![]);
    }
    Ok(vec![CompanyEvent::MemberAdded { user_id, added_by: actor }])
}
```

Run `cargo fmt -p doris-company` afterwards; the compact field lists above get reformatted.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-company --test domain`
Expected: all passed

- [ ] **Step 5: Commit**

```bash
git add crates/company
git commit -m "Add company events and membership decisions"
```

---

### Task 4: Store: migration, projections, app layer, queries

**Files:**
- Create: `migrations/0005_companies.sql`, `crates/company/src/projections.rs`, `crates/company/src/queries.rs`, `crates/company/tests/store.rs`
- Modify: `crates/company/src/lib.rs`

**Interfaces:**
- Consumes (from `doris_eventstore`):
  - `begin(&SqlitePool)`
  - `append(conn, stream, expected_version, &[NewEvent], &Metadata)`
  - `load(conn, stream)`
  - `read_all(conn, 0)`
  - `NewEvent::from_tagged(&T, i64)`
  - `RecordedEvent::decode`
- Produces:
  - `doris_company::Error`, with variants `Domain(DomainError)`, `AlreadyExists`, `NotFound` and `Store(doris_eventstore::Error)`; `Result<T>`
  - `NewCompany<'a> { org_nr: &'a str, name: &'a str, legal_form: LegalForm, street: &'a str, postal_code: &'a str, city: &'a str, fiscal_year_start: Date, fiscal_year_end: Date, accounting_method: AccountingMethod }`
  - `register_company(pool, created_by: Uuid, input: NewCompany<'_>) -> Result<Uuid>`
  - `add_member(pool, company_id: Uuid, actor: Uuid, user_id: Uuid) -> Result<()>`
  - `get_company(pool, company_id: Uuid, user_id: Uuid) -> Result<Company>`, which is `NotFound` for non-members
  - `list_companies(pool, user_id: Uuid) -> Result<Vec<CompanySummary>>`, with `CompanySummary { id: Uuid, org_nr: String, name: String }` sorted by name
  - `rebuild_projections(pool) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

`crates/company/tests/store.rs`:
```rust
use doris_company::domain::{AccountingMethod, DomainError, LegalForm};
use doris_company::{
    Error, NewCompany, add_member, get_company, list_companies, rebuild_projections,
    register_company,
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

fn input<'a>(org_nr: &'a str, name: &'a str) -> NewCompany<'a> {
    NewCompany {
        org_nr,
        name,
        legal_form: LegalForm::Aktiebolag,
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        fiscal_year_start: "2026-01-01".parse().unwrap(),
        fiscal_year_end: "2026-12-31".parse().unwrap(),
        accounting_method: AccountingMethod::Invoice,
    }
}

async fn event_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events").fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn a_registered_company_is_listed_for_its_creator_only() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());

    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB")).await.unwrap();

    let listed = list_companies(&pool, anna).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id, listed[0].org_nr.as_str(), listed[0].name.as_str()), (id, "5560160680", "Exempel AB"));
    assert!(list_companies(&pool, bo).await.unwrap().is_empty());
    let company = get_company(&pool, id, anna).await.unwrap();
    assert_eq!(company.address.city.as_deref(), Some("Stockholm"));
    assert!(matches!(get_company(&pool, id, bo).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn the_same_org_nr_cannot_be_registered_twice() {
    let pool = db().await;
    register_company(&pool, Uuid::new_v4(), input("556016-0680", "Exempel AB")).await.unwrap();
    let before = event_count(&pool).await;

    let again = register_company(&pool, Uuid::new_v4(), input("5560160680", "Annat AB")).await;

    assert!(matches!(again, Err(Error::AlreadyExists)));
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn invalid_input_is_rejected_before_anything_is_written() {
    let pool = db().await;
    let mut bad_year = input("556016-0680", "Exempel AB");
    bad_year.fiscal_year_end = "2026-12-30".parse().unwrap();

    for (cmd, expected) in [
        (input("556016-0681", "Exempel AB"), DomainError::InvalidOrgNr),
        (input("556016-0680", " "), DomainError::InvalidCompanyName),
        (bad_year, DomainError::InvalidFiscalYear),
    ] {
        let err = register_company(&pool, Uuid::new_v4(), cmd).await.unwrap_err();
        assert!(matches!(err, Error::Domain(e) if e == expected), "{expected:?}");
    }
    assert_eq!(event_count(&pool).await, 0);
}

#[tokio::test]
async fn a_member_adds_another_user_who_then_sees_the_company() {
    let pool = db().await;
    let (anna, bo, stranger) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB")).await.unwrap();

    add_member(&pool, id, anna, bo).await.unwrap();
    add_member(&pool, id, anna, bo).await.unwrap();

    assert_eq!(list_companies(&pool, bo).await.unwrap().len(), 1);
    assert_eq!(get_company(&pool, id, bo).await.unwrap().members, vec![anna, bo]);
    assert!(matches!(
        add_member(&pool, id, stranger, stranger).await,
        Err(Error::Domain(DomainError::NotMember))
    ));
    assert!(matches!(add_member(&pool, Uuid::new_v4(), anna, bo).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn projections_rebuild_from_the_event_log() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB")).await.unwrap();
    register_company(&pool, bo, input("556036-0793", "Bolaget AB")).await.unwrap();
    add_member(&pool, id, anna, bo).await.unwrap();
    let dump = |pool: SqlitePool| async move {
        let companies: Vec<(String, String, String, String, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT company_id, org_nr, name, legal_form, city, first_fiscal_year_start,
                        first_fiscal_year_end, accounting_method FROM companies ORDER BY company_id",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
        let members: Vec<(String, String)> =
            sqlx::query_as("SELECT company_id, user_id FROM company_members ORDER BY company_id, user_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        (companies, members)
    };
    let before = dump(pool.clone()).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(dump(pool.clone()).await, before);
    assert_eq!(before.1.len(), 3);
}
```

`556036-0793` is a second valid number (its Luhn sum is 40); Task 7 reuses it as a number Bolagsverket doesn't know.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-company --test store`
Expected: compile errors for the unresolved imports `register_company`, `Error`, …

- [ ] **Step 3: Write the migration** `migrations/0005_companies.sql`

```sql
-- Projections of the company-* streams. Rebuildable from events.
-- user_id has no foreign key: users live in another crate's projection,
-- and each crate rebuilds its own tables independently.

CREATE TABLE companies (
    company_id              TEXT PRIMARY KEY,
    org_nr                  TEXT NOT NULL UNIQUE,
    name                    TEXT NOT NULL,
    legal_form              TEXT NOT NULL,
    street                  TEXT,
    postal_code             TEXT,
    city                    TEXT,
    first_fiscal_year_start TEXT NOT NULL,
    first_fiscal_year_end   TEXT NOT NULL,
    accounting_method       TEXT NOT NULL,
    registered_at           TEXT NOT NULL
);

CREATE TABLE company_members (
    company_id TEXT NOT NULL REFERENCES companies (company_id),
    user_id    TEXT NOT NULL,
    added_at   TEXT NOT NULL,
    PRIMARY KEY (company_id, user_id)
);

CREATE INDEX company_members_user ON company_members (user_id);
```

- [ ] **Step 4: Write `projections.rs`**

```rust
//! Read models for companies and their members. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::CompanyEvent;
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    let Some(company_id) = event.stream_id.strip_prefix(crate::COMPANY_STREAM) else {
        return Ok(());
    };
    let at = &event.recorded_at;
    match event.decode::<CompanyEvent>()? {
        CompanyEvent::CompanyRegistered {
            org_nr, name, legal_form, address, first_fiscal_year, accounting_method, ..
        } => {
            sqlx::query(
                "INSERT INTO companies (company_id, org_nr, name, legal_form, street, postal_code,
                     city, first_fiscal_year_start, first_fiscal_year_end, accounting_method,
                     registered_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(org_nr.as_str())
            .bind(name.as_str())
            .bind(legal_form.as_str())
            .bind(address.street)
            .bind(address.postal_code)
            .bind(address.city)
            .bind(first_fiscal_year.start.to_string())
            .bind(first_fiscal_year.end.to_string())
            .bind(accounting_method.as_str())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        CompanyEvent::MemberAdded { user_id, .. } => {
            sqlx::query("INSERT INTO company_members (company_id, user_id, added_at) VALUES (?, ?, ?)")
                .bind(company_id)
                .bind(user_id.to_string())
                .bind(at)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Empties the company projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in ["DELETE FROM company_members", "DELETE FROM companies"] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 5: Write `queries.rs`**

```rust
//! Read-only views over the company projections.

use crate::Result;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct CompanySummary {
    pub id: Uuid,
    /// 10 digits, no hyphen.
    pub org_nr: String,
    pub name: String,
}

/// The companies `user_id` is a member of, by name.
pub async fn list_companies(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<CompanySummary>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.company_id, c.org_nr, c.name
         FROM companies c JOIN company_members m ON m.company_id = c.company_id
         WHERE m.user_id = ? ORDER BY c.name, c.org_nr",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, org_nr, name)| CompanySummary {
            id: id.parse().expect("company_id is a uuid"),
            org_nr,
            name,
        })
        .collect())
}
```

- [ ] **Step 6: Write the app layer in `lib.rs`** (replace the file)

```rust
//! Companies a user keeps the books for, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
mod projections;
mod queries;

use domain::{
    AccountingMethod, Address, Company, CompanyEvent, CompanyName, DomainError, FiscalYear,
    LegalForm, OrgNr, RegisterCompany,
};
use doris_eventstore::{Metadata, NewEvent};
use jiff::civil::Date;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::{CompanySummary, list_companies};

const COMPANY_STREAM: &str = "company-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("already exists")]
    AlreadyExists,
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    #[error(transparent)]
    Store(doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<doris_eventstore::Error> for Error {
    fn from(err: doris_eventstore::Error) -> Self {
        match err {
            doris_eventstore::Error::Db(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Error::AlreadyExists
            }
            other => Error::Store(other),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        doris_eventstore::Error::from(err).into()
    }
}

/// Unvalidated input for a new company, as it arrives from the API.
#[derive(Debug, Clone)]
pub struct NewCompany<'a> {
    pub org_nr: &'a str,
    pub name: &'a str,
    pub legal_form: LegalForm,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub fiscal_year_start: Date,
    pub fiscal_year_end: Date,
    pub accounting_method: AccountingMethod,
}

/// Registers a company with `created_by` as its first member.
pub async fn register_company(
    pool: &SqlitePool,
    created_by: Uuid,
    input: NewCompany<'_>,
) -> Result<Uuid> {
    let cmd = RegisterCompany {
        company_id: Uuid::new_v4(),
        org_nr: OrgNr::parse(input.org_nr)?,
        name: CompanyName::parse(input.name)?,
        legal_form: input.legal_form,
        address: Address::parse(input.street, input.postal_code, input.city)?,
        first_fiscal_year: FiscalYear::first(
            input.fiscal_year_start,
            input.fiscal_year_end,
            input.legal_form,
        )?,
        accounting_method: input.accounting_method,
    };
    let id = cmd.company_id;
    let events = domain::register_company(cmd, created_by);
    let mut tx = doris_eventstore::begin(pool).await?;
    commit(&mut tx, &company_stream(id), 0, &events, created_by).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn add_member(pool: &SqlitePool, company_id: Uuid, actor: Uuid, user_id: Uuid) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, version) = load_company(&mut tx, company_id).await?.ok_or(Error::NotFound)?;
    let events = domain::add_member(&company, actor, user_id)?;
    commit(&mut tx, &company_stream(company_id), version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

/// The company, if `user_id` is a member of it.
pub async fn get_company(pool: &SqlitePool, company_id: Uuid, user_id: Uuid) -> Result<Company> {
    let mut conn = pool.acquire().await?;
    match load_company(&mut conn, company_id).await? {
        Some((company, _)) if company.is_member(user_id) => Ok(company),
        _ => Err(Error::NotFound),
    }
}

fn company_stream(id: Uuid) -> String {
    format!("{COMPANY_STREAM}{id}")
}

async fn load_company(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<(Company, i64)>> {
    let recorded = doris_eventstore::load(conn, &company_stream(id)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<CompanyEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Company::from_events(&events).map(|company| (company, version)))
}

/// Appends events and updates projections within the caller's transaction.
async fn commit(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[CompanyEvent],
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
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
```

`NewEvent::from_tagged` and `RecordedEvent::decode` return `doris_eventstore::Error`, so `?` goes through `From<doris_eventstore::Error>`. The `From<serde_json::Error>` impl mirrors identity's and is harmless.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p doris-company && cargo test -p doris-identity`
Expected: all passed. Identity is included because the new migration runs for every crate's tests.

- [ ] **Step 8: Commit**

```bash
git add migrations/0005_companies.sql crates/company
git commit -m "Store companies as events with rebuildable projections"
```

---

### Task 5: `CompanyService` proto

**Files:**
- Create: `proto/doris/company/v1/company.proto`
- Modify: `crates/proto/build.rs`, `crates/proto/src/lib.rs`

**Interfaces:**
- Produces:
  - `doris_proto::company::v1::{company_service_client::CompanyServiceClient, company_service_server::{CompanyService, CompanyServiceServer}}` (the server module only with feature `server`)
  - Messages: `LookupCompanyRequest{org_nr}`, `LookupCompanyResponse{org_nr,name,legal_form,address}`, `CreateCompanyRequest{…}`, `CreateCompanyResponse{company_id}`, `ListCompaniesRequest{}`, `ListCompaniesResponse{companies: Vec<CompanySummary{id,org_nr,name}>}`, `GetCompanyRequest{company_id}`, `Company{…}`, `AddMemberRequest{company_id,email}`, `AddMemberResponse{}`, `ListMembersRequest{company_id}`, `ListMembersResponse{members: Vec<Member{display_name,email}>}`, `Address{street,postal_code,city}`
  - Enums: `LegalForm::{Unspecified, Aktiebolag, Handelsbolag, Kommanditbolag, EnskildFirma, EkonomiskForening, IdeellForening, Stiftelse, Other}` and `AccountingMethod::{Unspecified, Cash, Invoice}`

This task is contract-only and has no behavior, so the build is the test.

- [ ] **Step 1: Write the proto**

```proto
syntax = "proto3";

package doris.company.v1;

// Companies a user keeps the books for. Every RPC needs a session; a company
// the caller isn't a member of answers NOT_FOUND "company_not_found".
service CompanyService {
  // Details from Bolagsverket to pre-fill the form. Nothing is stored.
  rpc LookupCompany(LookupCompanyRequest) returns (LookupCompanyResponse);
  rpc CreateCompany(CreateCompanyRequest) returns (CreateCompanyResponse);
  rpc ListCompanies(ListCompaniesRequest) returns (ListCompaniesResponse);
  rpc GetCompany(GetCompanyRequest) returns (Company);
  // Adds an existing user, by email, as a member.
  rpc AddMember(AddMemberRequest) returns (AddMemberResponse);
  rpc ListMembers(ListMembersRequest) returns (ListMembersResponse);
}

enum LegalForm {
  LEGAL_FORM_UNSPECIFIED = 0;
  LEGAL_FORM_AKTIEBOLAG = 1;
  LEGAL_FORM_HANDELSBOLAG = 2;
  LEGAL_FORM_KOMMANDITBOLAG = 3;
  LEGAL_FORM_ENSKILD_FIRMA = 4;
  LEGAL_FORM_EKONOMISK_FORENING = 5;
  LEGAL_FORM_IDEELL_FORENING = 6;
  LEGAL_FORM_STIFTELSE = 7;
  LEGAL_FORM_OTHER = 8;
}

enum AccountingMethod {
  ACCOUNTING_METHOD_UNSPECIFIED = 0;
  ACCOUNTING_METHOD_CASH = 1;     // kontantmetoden
  ACCOUNTING_METHOD_INVOICE = 2;  // faktureringsmetoden
}

// Empty strings mean "not known".
message Address {
  string street = 1;
  string postal_code = 2;
  string city = 3;
}

message LookupCompanyRequest {
  string org_nr = 1;
}

message LookupCompanyResponse {
  string org_nr = 1;  // NNNNNN-NNNN
  string name = 2;
  LegalForm legal_form = 3;
  Address address = 4;
}

message CreateCompanyRequest {
  string org_nr = 1;
  string name = 2;
  LegalForm legal_form = 3;
  Address address = 4;
  string fiscal_year_start = 5;  // YYYY-MM-DD, first räkenskapsår
  string fiscal_year_end = 6;    // YYYY-MM-DD
  AccountingMethod accounting_method = 7;
}

message CreateCompanyResponse {
  string company_id = 1;
}

message ListCompaniesRequest {}

message CompanySummary {
  string id = 1;
  string org_nr = 2;  // NNNNNN-NNNN
  string name = 3;
}

message ListCompaniesResponse {
  repeated CompanySummary companies = 1;
}

message GetCompanyRequest {
  string company_id = 1;
}

message Company {
  string id = 1;
  string org_nr = 2;  // NNNNNN-NNNN
  string name = 3;
  LegalForm legal_form = 4;
  Address address = 5;
  AccountingMethod accounting_method = 6;
  // The räkenskapsår that contains today, YYYY-MM-DD.
  string fiscal_year_start = 7;
  string fiscal_year_end = 8;
}

message AddMemberRequest {
  string company_id = 1;
  string email = 2;
}

message AddMemberResponse {}

message ListMembersRequest {
  string company_id = 1;
}

message Member {
  string display_name = 1;
  string email = 2;
}

message ListMembersResponse {
  repeated Member members = 1;
}
```

- [ ] **Step 2: Compile it**

In `crates/proto/build.rs`, change `compile_protos(&["../../proto/doris/auth/v1/auth.proto"], …)` to
`compile_protos(&["../../proto/doris/auth/v1/auth.proto", "../../proto/doris/company/v1/company.proto"], &["../../proto"])`.

In `crates/proto/src/lib.rs`, add:
```rust
pub mod company {
    pub mod v1 {
        tonic::include_proto!("doris.company.v1");
    }
}
```

- [ ] **Step 3: Verify that it builds for the host and for wasm**

Run: `cargo build -p doris-proto --features server && cargo build -p doris-proto --target wasm32-unknown-unknown`
Expected: both succeed

- [ ] **Step 4: Commit**

```bash
git add proto/doris/company crates/proto
git commit -m "Add the CompanyService gRPC contract"
```

---

### Task 6: Server: `CompanyApi` (create, list, get, members)

**Files:**
- Create: `crates/server/src/company.rs`, `crates/server/tests/companies.rs`
- Modify: `crates/server/src/grpc.rs`, `crates/server/src/lib.rs`, `crates/server/src/main.rs`, `crates/server/Cargo.toml`, `crates/server/tests/common/mod.rs`

**Interfaces:**
- Consumes: `doris_company::{register_company, add_member, get_company, list_companies, NewCompany, Error}` (Task 4), the proto types (Task 5), `doris_identity::{session_user, get_user, find_user_by_email}`
- Produces:
  - `doris_server::CompanyApi::new(pool: SqlitePool, bolagsverket: Option<Bolagsverket>)`. The `Bolagsverket` type arrives in Task 7; this task uses `CompanyApi::new(pool)`, and Task 7 adds the second parameter.
  - `router<E>(api: AuthApi, companies: CompanyApi, cors_origins, serve_frontend)`
  - `grpc::signed_in_user(pool, &Request<T>) -> Result<User, Status>` and `grpc::status(identity::Error) -> Status` (both `pub(crate)`)
  - Test harness: `TestServer::companies() -> Companies`, `TestServer::invite(&self, admin_session, email) -> String` (the new user's session)

- [ ] **Step 1: Extend the test harness** (`crates/server/tests/common/mod.rs`)

```rust
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
use doris_server::CompanyApi;

type Transport = GrpcWebClientService<Client<HttpConnector, GrpcWebCall<tonic::body::Body>>>;
pub type Grpc = AuthServiceClient<Transport>;
pub type Companies = CompanyServiceClient<Transport>;
```

Replace the `router` call in `start_with` with:
```rust
let app = doris_server::router::<TestDist>(
    AuthApi::new(pool.clone(), auth),
    CompanyApi::new(pool.clone()),
    cors_origins,
    serve_frontend,
);
```

Add these methods to `impl TestServer`, and make `grpc()` share the transport:
```rust
fn transport(&self) -> Transport {
    let client = Client::builder(TokioExecutor::new()).build_http();
    tower::ServiceBuilder::new().layer(GrpcWebClientLayer::new()).service(client)
}

pub fn grpc(&self) -> Grpc {
    AuthServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
}

pub fn companies(&self) -> Companies {
    CompanyServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
}

/// An admin invites `email`, who registers; returns the new user's session.
pub async fn invite(&self, admin: &str, email: &str) -> String {
    let invite = self
        .grpc()
        .create_invitation(authed(pb::CreateInvitationRequest { email: email.into() }, admin))
        .await
        .unwrap()
        .into_inner();
    self.sign_up(&mut device(), email, Some(&invite.token)).await
}
```

- [ ] **Step 2: Write the failing integration tests** (`crates/server/tests/companies.rs`)

```rust
mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as pb;
use tonic::Code;

fn create(org_nr: &str, name: &str) -> pb::CreateCompanyRequest {
    pb::CreateCompanyRequest {
        org_nr: org_nr.into(),
        name: name.into(),
        legal_form: pb::LegalForm::Aktiebolag as i32,
        address: Some(pb::Address { street: "".into(), postal_code: "111 22".into(), city: "Stockholm".into() }),
        fiscal_year_start: "2026-01-01".into(),
        fiscal_year_end: "2026-12-31".into(),
        accounting_method: pb::AccountingMethod::Invoice as i32,
    }
}

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

#[tokio::test]
async fn a_user_creates_lists_and_opens_a_company() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();

    let id = api.create_company(authed(create("556016-0680", "Exempel AB"), &anna)).await.unwrap().into_inner().company_id;
    let list = api.list_companies(authed(pb::ListCompaniesRequest {}, &anna)).await.unwrap().into_inner();
    let company = api.get_company(authed(pb::GetCompanyRequest { company_id: id.clone() }, &anna)).await.unwrap().into_inner();

    assert_eq!(list.companies, vec![pb::CompanySummary { id: id.clone(), org_nr: "556016-0680".into(), name: "Exempel AB".into() }]);
    assert_eq!(company.org_nr, "556016-0680");
    assert_eq!(company.legal_form(), pb::LegalForm::Aktiebolag);
    assert_eq!(company.accounting_method(), pb::AccountingMethod::Invoice);
    assert_eq!(company.address.unwrap().city, "Stockholm");
    // The current räkenskapsår is a whole calendar year at or after the first.
    assert!(company.fiscal_year_start.ends_with("-01-01") && company.fiscal_year_end.ends_with("-12-31"));
    assert!(company.fiscal_year_start.as_str() >= "2026-01-01");
}

#[tokio::test]
async fn create_rejects_invalid_input_with_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();
    let mut unparseable = create("556016-0680", "Exempel AB");
    unparseable.fiscal_year_end = "31/12".into();
    let mut no_form = create("556016-0680", "Exempel AB");
    no_form.legal_form = 0;
    let mut no_method = create("556016-0680", "Exempel AB");
    no_method.accounting_method = 0;
    let mut broken_hb = create("556016-0680", "Exempel HB");
    broken_hb.legal_form = pb::LegalForm::Handelsbolag as i32;
    broken_hb.fiscal_year_start = "2026-05-01".into();
    broken_hb.fiscal_year_end = "2027-04-30".into();

    for (request, code) in [
        (create("556016-0681", "Exempel AB"), "invalid_org_nr"),
        (create("556016-0680", ""), "invalid_company_name"),
        (unparseable, "invalid_fiscal_year"),
        (broken_hb, "invalid_fiscal_year"),
        (no_form, "invalid_legal_form"),
        (no_method, "invalid_accounting_method"),
    ] {
        let err = api.create_company(authed(request, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (Code::InvalidArgument, code.into()));
    }
    api.create_company(authed(create("556016-0680", "Exempel AB"), &anna)).await.unwrap();
    let dup = api.create_company(authed(create("5560160680", "Igen AB"), &anna)).await.unwrap_err();
    assert_eq!(code_of(dup), (Code::AlreadyExists, "company_exists".into()));
}

#[tokio::test]
async fn every_company_rpc_needs_a_session() {
    let server = TestServer::start().await;
    let err = server.companies().list_companies(pb::ListCompaniesRequest {}).await.unwrap_err();
    assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".into()));
}

#[tokio::test]
async fn a_member_adds_a_colleague_who_then_sees_the_company() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.companies();
    let id = api.create_company(authed(create("556016-0680", "Exempel AB"), &anna)).await.unwrap().into_inner().company_id;

    let before = api.list_companies(authed(pb::ListCompaniesRequest {}, &bo)).await.unwrap().into_inner();
    api.add_member(authed(pb::AddMemberRequest { company_id: id.clone(), email: "Bo@Example.se".into() }, &anna)).await.unwrap();
    let after = api.list_companies(authed(pb::ListCompaniesRequest {}, &bo)).await.unwrap().into_inner();
    let members = api.list_members(authed(pb::ListMembersRequest { company_id: id.clone() }, &bo)).await.unwrap().into_inner();
    let unknown = api.add_member(authed(pb::AddMemberRequest { company_id: id, email: "nobody@example.se".into() }, &anna)).await.unwrap_err();

    assert!(before.companies.is_empty());
    assert_eq!(after.companies.len(), 1);
    let emails: Vec<_> = members.members.iter().map(|m| m.email.as_str()).collect();
    assert_eq!(emails, ["anna@example.se", "bo@example.se"]);
    assert_eq!(code_of(unknown), (Code::NotFound, "user_not_found".into()));
}

#[tokio::test]
async fn non_members_and_bad_ids_get_company_not_found() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.companies();
    let id = api.create_company(authed(create("556016-0680", "Exempel AB"), &anna)).await.unwrap().into_inner().company_id;
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    for company_id in [id.clone(), "not-a-uuid".into(), uuid::Uuid::new_v4().to_string()] {
        let get = api.get_company(authed(pb::GetCompanyRequest { company_id: company_id.clone() }, &bo)).await.unwrap_err();
        let members = api.list_members(authed(pb::ListMembersRequest { company_id: company_id.clone() }, &bo)).await.unwrap_err();
        // Probing an unknown email must not reveal that it is unknown.
        let add = api.add_member(authed(pb::AddMemberRequest { company_id, email: "nobody@example.se".into() }, &bo)).await.unwrap_err();
        assert_eq!(code_of(get), not_found);
        assert_eq!(code_of(members), not_found);
        assert_eq!(code_of(add), not_found);
    }
}
```

`uuid` is already a normal dependency of `doris-server`, so the tests can use it.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-server --test companies`
Expected: compile error `unresolved import doris_server::CompanyApi`

- [ ] **Step 4: Share session lookup and status mapping in `grpc.rs`**

Replace the body of `AuthApi::user` with a call to a new free function, and make `status` `pub(crate)`:
```rust
/// The signed-in user, or `Unauthenticated`. Shared by every service.
pub(crate) async fn signed_in_user<T>(pool: &SqlitePool, request: &Request<T>) -> Result<User, Status> {
    let token = session_token(request).ok_or_else(not_signed_in)?;
    doris_identity::session_user(pool, &token, Timestamp::now())
        .await
        .map_err(status)?
        .ok_or_else(not_signed_in)
}
```
`AuthApi::user` becomes `signed_in_user(&self.pool, request).await`. Change `fn status(err: Error)` to `pub(crate) fn status(err: Error)`.

- [ ] **Step 5: Write `crates/server/src/company.rs`**

Add `doris-company.workspace = true` to `crates/server/Cargo.toml` `[dependencies]`.

```rust
//! `doris.company.v1.CompanyService`: maps gRPC calls onto `doris_company`.
//! Every call needs a session, and a company the caller isn't a member of
//! looks exactly like one that doesn't exist.

use crate::grpc::{self, signed_in_user};
use doris_company::domain::{AccountingMethod, Address, Company, DomainError, LegalForm};
use doris_company::{Error, NewCompany};
use doris_proto::company::v1 as pb;
use doris_proto::company::v1::company_service_server::CompanyService;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct CompanyApi {
    pool: SqlitePool,
}

impl CompanyApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The company, if the signed-in user is a member of it.
    async fn member_company<T>(&self, request: &Request<T>, company_id: &str) -> Result<Company, Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let id: Uuid = company_id.parse().map_err(|_| company_not_found())?;
        doris_company::get_company(&self.pool, id, user.id).await.map_err(status)
    }
}

#[tonic::async_trait]
impl CompanyService for CompanyApi {
    async fn lookup_company(
        &self,
        request: Request<pb::LookupCompanyRequest>,
    ) -> Result<Response<pb::LookupCompanyResponse>, Status> {
        signed_in_user(&self.pool, &request).await?;
        Err(Status::failed_precondition("lookup_unavailable"))
    }

    async fn create_company(
        &self,
        request: Request<pb::CreateCompanyRequest>,
    ) -> Result<Response<pb::CreateCompanyResponse>, Status> {
        let user = signed_in_user(&self.pool, &request).await?;
        let req = request.into_inner();
        let address = req.address.clone().unwrap_or_default();
        let input = NewCompany {
            org_nr: &req.org_nr,
            name: &req.name,
            legal_form: legal_form_from(req.legal_form())?,
            street: &address.street,
            postal_code: &address.postal_code,
            city: &address.city,
            fiscal_year_start: date(&req.fiscal_year_start)?,
            fiscal_year_end: date(&req.fiscal_year_end)?,
            accounting_method: accounting_method_from(req.accounting_method())?,
        };
        let id = doris_company::register_company(&self.pool, user.id, input)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CreateCompanyResponse { company_id: id.to_string() }))
    }

    async fn list_companies(
        &self,
        request: Request<pb::ListCompaniesRequest>,
    ) -> Result<Response<pb::ListCompaniesResponse>, Status> {
        let user = signed_in_user(&self.pool, &request).await?;
        let companies = doris_company::list_companies(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|c| pb::CompanySummary {
                id: c.id.to_string(),
                org_nr: format!("{}-{}", &c.org_nr[..6], &c.org_nr[6..]),
                name: c.name,
            })
            .collect();
        Ok(Response::new(pb::ListCompaniesResponse { companies }))
    }

    async fn get_company(
        &self,
        request: Request<pb::GetCompanyRequest>,
    ) -> Result<Response<pb::Company>, Status> {
        let company = self.member_company(&request, &request.get_ref().company_id).await?;
        // ponytail: "today" in UTC, so the fiscal year flips up to 2 hours
        // late at New Year in Sweden; use Europe/Stockholm once the image ships tzdata.
        let today = Timestamp::now().to_zoned(TimeZone::UTC).date();
        let year = company.first_fiscal_year.containing(today);
        Ok(Response::new(pb::Company {
            id: company.id.to_string(),
            org_nr: company.org_nr.formatted(),
            name: company.name.as_str().to_owned(),
            legal_form: legal_form_message(company.legal_form) as i32,
            address: Some(address_message(&company.address)),
            accounting_method: match company.accounting_method {
                AccountingMethod::Cash => pb::AccountingMethod::Cash,
                AccountingMethod::Invoice => pb::AccountingMethod::Invoice,
            } as i32,
            fiscal_year_start: year.start.to_string(),
            fiscal_year_end: year.end.to_string(),
        }))
    }

    async fn add_member(
        &self,
        request: Request<pb::AddMemberRequest>,
    ) -> Result<Response<pb::AddMemberResponse>, Status> {
        // Access first, so a non-member can't probe which emails exist.
        let company = self.member_company(&request, &request.get_ref().company_id).await?;
        let actor = signed_in_user(&self.pool, &request).await?;
        let member = doris_identity::find_user_by_email(&self.pool, &request.get_ref().email)
            .await
            .map_err(grpc::status)?
            .ok_or_else(|| Status::not_found("user_not_found"))?;
        doris_company::add_member(&self.pool, company.id, actor.id, member.id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddMemberResponse {}))
    }

    async fn list_members(
        &self,
        request: Request<pb::ListMembersRequest>,
    ) -> Result<Response<pb::ListMembersResponse>, Status> {
        let company = self.member_company(&request, &request.get_ref().company_id).await?;
        let mut members = Vec::with_capacity(company.members.len());
        for id in company.members {
            if let Some(user) = doris_identity::get_user(&self.pool, id).await.map_err(grpc::status)? {
                members.push(pb::Member {
                    display_name: user.display_name.as_str().to_owned(),
                    email: user.email.as_str().to_owned(),
                });
            }
        }
        Ok(Response::new(pb::ListMembersResponse { members }))
    }
}

pub(crate) fn legal_form_message(form: LegalForm) -> pb::LegalForm {
    match form {
        LegalForm::Aktiebolag => pb::LegalForm::Aktiebolag,
        LegalForm::Handelsbolag => pb::LegalForm::Handelsbolag,
        LegalForm::Kommanditbolag => pb::LegalForm::Kommanditbolag,
        LegalForm::EnskildFirma => pb::LegalForm::EnskildFirma,
        LegalForm::EkonomiskForening => pb::LegalForm::EkonomiskForening,
        LegalForm::IdeellForening => pb::LegalForm::IdeellForening,
        LegalForm::Stiftelse => pb::LegalForm::Stiftelse,
        LegalForm::Other => pb::LegalForm::Other,
    }
}

fn legal_form_from(form: pb::LegalForm) -> Result<LegalForm, Status> {
    Ok(match form {
        pb::LegalForm::Unspecified => return Err(Status::invalid_argument("invalid_legal_form")),
        pb::LegalForm::Aktiebolag => LegalForm::Aktiebolag,
        pb::LegalForm::Handelsbolag => LegalForm::Handelsbolag,
        pb::LegalForm::Kommanditbolag => LegalForm::Kommanditbolag,
        pb::LegalForm::EnskildFirma => LegalForm::EnskildFirma,
        pb::LegalForm::EkonomiskForening => LegalForm::EkonomiskForening,
        pb::LegalForm::IdeellForening => LegalForm::IdeellForening,
        pb::LegalForm::Stiftelse => LegalForm::Stiftelse,
        pb::LegalForm::Other => LegalForm::Other,
    })
}

fn accounting_method_from(method: pb::AccountingMethod) -> Result<AccountingMethod, Status> {
    match method {
        pb::AccountingMethod::Unspecified => Err(Status::invalid_argument("invalid_accounting_method")),
        pb::AccountingMethod::Cash => Ok(AccountingMethod::Cash),
        pb::AccountingMethod::Invoice => Ok(AccountingMethod::Invoice),
    }
}

pub(crate) fn address_message(address: &Address) -> pb::Address {
    pb::Address {
        street: address.street.clone().unwrap_or_default(),
        postal_code: address.postal_code.clone().unwrap_or_default(),
        city: address.city.clone().unwrap_or_default(),
    }
}

fn date(raw: &str) -> Result<Date, Status> {
    raw.parse().map_err(|_| Status::invalid_argument("invalid_fiscal_year"))
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

pub(crate) fn domain_status(err: DomainError) -> Status {
    match err {
        DomainError::InvalidOrgNr => Status::invalid_argument("invalid_org_nr"),
        DomainError::InvalidCompanyName => Status::invalid_argument("invalid_company_name"),
        DomainError::InvalidAddress => Status::invalid_argument("invalid_address"),
        DomainError::InvalidFiscalYear => Status::invalid_argument("invalid_fiscal_year"),
        DomainError::NotMember => company_not_found(),
    }
}

fn status(err: Error) -> Status {
    match err {
        Error::Domain(err) => domain_status(err),
        Error::AlreadyExists => Status::already_exists("company_exists"),
        Error::NotFound => company_not_found(),
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}
```

- [ ] **Step 6: Mount the service** (`crates/server/src/lib.rs`)

```rust
mod company;
use doris_proto::company::v1::company_service_server::CompanyServiceServer;
pub use company::CompanyApi;
```

Change the signature to `pub fn router<E: RustEmbed + Send + Sync + 'static>(api: AuthApi, companies: CompanyApi, cors_origins: Vec<HeaderValue>, serve_frontend: bool) -> Router`, and change its first line to:
```rust
let mut app = Routes::new(AuthServiceServer::new(api))
    .add_service(CompanyServiceServer::new(companies))
    .into_axum_router()
```

In `main.rs`, pass `CompanyApi::new(pool.clone())` as the second argument (the pool is moved into `AuthApi::new`, so clone it first) and add `CompanyApi` to the `use doris_server::…` line.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p doris-server`
Expected: all passed, including the existing `grpc`, `http`, `cli` and `web_dist` tests

- [ ] **Step 8: Lint and commit**

```bash
cargo clippy --workspace -- -D warnings
git add crates/server
git commit -m "Serve CompanyService: create, list, get and members"
```

---

### Task 7: Bolagsverket lookup

**Files:**
- Create: `crates/server/src/bolagsverket.rs`
- Modify: `Cargo.toml` (workspace dep), `crates/server/Cargo.toml`, `crates/server/src/company.rs`, `crates/server/src/lib.rs`, `crates/server/src/main.rs`, `crates/server/tests/common/mod.rs`, `crates/server/tests/companies.rs`, `crates/server/tests/cli.rs` (only if it snapshots `--help`), `Dockerfile`, `docs/superpowers/specs/2026-09-30-foretag-design.md`

**Interfaces:**
- Produces:
  - `doris_server::bolagsverket::Bolagsverket::new(token_url: &str, api_url: &str, client_id: String, client_secret: String) -> Bolagsverket`
  - `Bolagsverket::lookup(&self, &OrgNr) -> Result<Found, LookupError>`
  - `Found { name: String, legal_form: LegalForm, address: Address }`
  - `LookupError`, with variants `NotFound` and `Failed(String)`
  - `CompanyApi::new(pool, bolagsverket: Option<Bolagsverket>)`
  - Harness: `TestServer::start_with_bolagsverket(Bolagsverket)`

- [ ] **Step 1: Add the dependency**

In the workspace `Cargo.toml` `[workspace.dependencies]`:
```toml
reqwest = { version = "0.13", default-features = false, features = ["native-tls", "json", "form"] }
```
In `crates/server/Cargo.toml` `[dependencies]`, add `reqwest.workspace = true`.

Run `cargo tree -p doris-server -e normal -i openssl-sys` and confirm that both webauthn-rs and reqwest (via native-tls) use the same `openssl-sys`, so no second TLS stack is added.

- [ ] **Step 2: Write the failing unit tests for response mapping** (at the bottom of the new `crates/server/src/bolagsverket.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ERICSSON: &str = r#"{ "organisationer": [ {
        "avregistreradOrganisation": null,
        "organisationsform": { "kod": "AB", "klartext": "Aktiebolag", "dataproducent": "Bolagsverket", "fel": null },
        "organisationsidentitet": { "identitetsbeteckning": "5560160680", "typ": { "kod": "ORGNR", "klartext": "Organisationsnummer" } },
        "organisationsnamn": { "dataproducent": "Bolagsverket", "fel": null, "organisationsnamnLista": [
            { "namn": "Ericsson", "organisationsnamntyp": { "kod": "BIFIRMA", "klartext": "Bifirma" } },
            { "namn": "Telefonaktiebolaget LM Ericsson", "organisationsnamntyp": { "kod": "FORETAGSNAMN", "klartext": "Företagsnamn" } }
        ] },
        "postadressOrganisation": {
            "postadress": { "postnummer": "16483", "coAdress": null, "land": null, "postort": "STOCKHOLM", "utdelningsadress": null },
            "dataproducent": "Bolagsverket", "fel": null
        }
    } ] }"#;

    #[test]
    fn maps_name_legal_form_and_address() {
        let found = first(serde_json::from_str(ERICSSON).unwrap()).unwrap();
        assert_eq!(found.name, "Telefonaktiebolaget LM Ericsson");
        assert_eq!(found.legal_form, LegalForm::Aktiebolag);
        assert_eq!(found.address.street, None);
        assert_eq!(found.address.postal_code.as_deref(), Some("16483"));
        assert_eq!(found.address.city.as_deref(), Some("STOCKHOLM"));
    }

    #[test]
    fn parses_a_response_with_nulls() {
        let json = r#"{ "organisationer": [ { "organisationsnamn": null,
            "organisationsform": null, "postadressOrganisation": { "postadress": null } } ] }"#;
        let found = first(serde_json::from_str(json).unwrap()).unwrap();
        assert_eq!(found, Found { name: String::new(), legal_form: LegalForm::Other, address: Address::default() });
    }

    #[test]
    fn an_empty_list_is_not_found() {
        assert!(matches!(first(serde_json::from_str(r#"{ "organisationer": [] }"#).unwrap()), Err(LookupError::NotFound)));
    }

    #[test]
    fn organisationsform_codes_map_to_legal_forms() {
        for (code, form) in [
            ("AB", LegalForm::Aktiebolag),
            ("HB", LegalForm::Handelsbolag),
            ("KB", LegalForm::Kommanditbolag),
            ("E", LegalForm::EnskildFirma),
            ("EK", LegalForm::EkonomiskForening),
            ("I", LegalForm::IdeellForening),
            ("S", LegalForm::Stiftelse),
            ("BRF", LegalForm::Other),
        ] {
            assert_eq!(legal_form(code), form, "{code}");
        }
    }
}
```

Only `AB` has been seen in a real response. The other codes follow Bolagsverket's organisationsform list; anything unknown maps to `Other`, and the user can correct it in the form.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-server --lib bolagsverket`
Expected: compile errors, because `first`, `Found` and `legal_form` don't exist. Add `pub mod bolagsverket;` to `lib.rs` so that the module is compiled.

- [ ] **Step 4: Implement the client** (top of `bolagsverket.rs`)

```rust
//! Bolagsverket's API for värdefulla datamängder: free company details by
//! organisationsnummer, used to pre-fill the company form. OAuth2 client
//! credentials; the token is cached until a minute before it expires.
//!
//! The org nr may be a personnummer: it goes only in the POST body, and
//! failure reasons never include it (`reqwest::Error::without_url`).

use doris_company::domain::{Address, LegalForm, OrgNr};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SCOPE: &str = "vardefulla-datamangder:read";
pub const TOKEN_URL: &str = "https://portal.api.bolagsverket.se/oauth2/token";
pub const API_URL: &str = "https://gw.api.bolagsverket.se/vardefulla-datamangder/v1";

pub struct Bolagsverket {
    http: reqwest::Client,
    token_url: String,
    api_url: String,
    client_id: String,
    client_secret: String,
    token: Mutex<Option<(String, Instant)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: String,
    pub legal_form: LegalForm,
    pub address: Address,
}

#[derive(Debug)]
pub enum LookupError {
    NotFound,
    /// For the log. Never contains the org nr.
    Failed(String),
}

impl Bolagsverket {
    pub fn new(token_url: &str, api_url: &str, client_id: String, client_secret: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("the system TLS library loads");
        Self {
            http,
            token_url: token_url.to_owned(),
            api_url: api_url.trim_end_matches('/').to_owned(),
            client_id,
            client_secret,
            token: Mutex::new(None),
        }
    }

    pub async fn lookup(&self, org_nr: &OrgNr) -> Result<Found, LookupError> {
        let token = self.token().await?;
        let response = self
            .http
            .post(format!("{}/organisationer", self.api_url))
            .bearer_auth(token)
            .json(&serde_json::json!({ "identitetsbeteckning": org_nr.as_str() }))
            .send()
            .await
            .map_err(failed)?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND => return Err(LookupError::NotFound),
            reqwest::StatusCode::UNAUTHORIZED => {
                // Revoked or expired early: fetch a new token next time.
                *self.token.lock().expect("token lock") = None;
                return Err(LookupError::Failed("organisationer: HTTP 401".into()));
            }
            s if !s.is_success() => return Err(LookupError::Failed(format!("organisationer: HTTP {s}"))),
            _ => {}
        }
        first(response.json().await.map_err(failed)?)
    }

    async fn token(&self) -> Result<String, LookupError> {
        // Copy out and drop the guard at once: a std MutexGuard held across
        // an await would make the future !Send.
        let cached = self.token.lock().expect("token lock").clone();
        if let Some((token, valid_until)) = cached {
            if Instant::now() < valid_until {
                return Ok(token);
            }
        }
        let response = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("scope", SCOPE),
            ])
            .send()
            .await
            .map_err(failed)?;
        if !response.status().is_success() {
            return Err(LookupError::Failed(format!("token: HTTP {}", response.status())));
        }
        let token: Token = response.json().await.map_err(failed)?;
        let valid_until = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(60));
        *self.token.lock().expect("token lock") = Some((token.access_token.clone(), valid_until));
        Ok(token.access_token)
    }
}

fn failed(err: reqwest::Error) -> LookupError {
    LookupError::Failed(err.without_url().to_string())
}

/// Bolagsverket's organisationsform code. Unknown codes become `Other`.
fn legal_form(code: &str) -> LegalForm {
    match code {
        "AB" => LegalForm::Aktiebolag,
        "HB" => LegalForm::Handelsbolag,
        "KB" => LegalForm::Kommanditbolag,
        "E" => LegalForm::EnskildFirma,
        "EK" => LegalForm::EkonomiskForening,
        "I" => LegalForm::IdeellForening,
        "S" => LegalForm::Stiftelse,
        _ => LegalForm::Other,
    }
}

fn first(response: Organisationer) -> Result<Found, LookupError> {
    let org = response.organisationer.into_iter().next().ok_or(LookupError::NotFound)?;
    let names = org.organisationsnamn.and_then(|n| n.organisationsnamn_lista).unwrap_or_default();
    let name = names
        .iter()
        .find(|n| n.organisationsnamntyp.as_ref().is_some_and(|t| t.kod == "FORETAGSNAMN"))
        .or(names.first())
        .map(|n| n.namn.clone())
        .unwrap_or_default();
    let postal = org.postadress_organisation.and_then(|p| p.postadress).unwrap_or_default();
    // Over-long fields from the registry are dropped; the user can type them.
    let address = Address::parse(
        postal.utdelningsadress.as_deref().unwrap_or(""),
        postal.postnummer.as_deref().unwrap_or(""),
        postal.postort.as_deref().unwrap_or(""),
    )
    .unwrap_or_default();
    Ok(Found {
        name,
        legal_form: org.organisationsform.map_or(LegalForm::Other, |f| legal_form(&f.kod)),
        address,
    })
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Organisationer {
    organisationer: Vec<Organisation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Organisation {
    organisationsnamn: Option<Names>,
    organisationsform: Option<Code>,
    postadress_organisation: Option<PostalWrapper>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Names {
    organisationsnamn_lista: Option<Vec<Name>>,
}

#[derive(Deserialize)]
struct Name {
    namn: String,
    organisationsnamntyp: Option<Code>,
}

#[derive(Deserialize)]
struct Code {
    kod: String,
}

#[derive(Deserialize)]
struct PostalWrapper {
    postadress: Option<PostalAddress>,
}

#[derive(Default, Deserialize)]
struct PostalAddress {
    utdelningsadress: Option<String>,
    postnummer: Option<String>,
    postort: Option<String>,
}
```

- [ ] **Step 5: Run the unit tests to verify they pass**

Run: `cargo test -p doris-server --lib bolagsverket`
Expected: 4 passed

- [ ] **Step 6: Write the failing integration tests** (append to `tests/companies.rs`, plus a harness constructor)

In `tests/common/mod.rs`, make `start_with` delegate to a private `launch(cors_origins, serve_frontend, bolagsverket: Option<Bolagsverket>)` that passes `CompanyApi::new(pool.clone(), bolagsverket)`, and add:
```rust
pub async fn start_with_bolagsverket(bolagsverket: doris_server::bolagsverket::Bolagsverket) -> Self {
    Self::launch(vec![], true, Some(bolagsverket)).await
}
```

In `tests/companies.rs`:
```rust
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use doris_server::bolagsverket::Bolagsverket;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A stand-in for Bolagsverket that knows one company. Tokens are "t1", "t2", …
/// in the order they are issued; `revoked` tokens get 401.
struct FakeBolagsverket {
    base: String,
    tokens_issued: Arc<AtomicUsize>,
}

async fn fake_bolagsverket(revoked: &'static [&'static str]) -> FakeBolagsverket {
    let tokens_issued = Arc::new(AtomicUsize::new(0));
    let issued = tokens_issued.clone();
    let app = Router::new()
        .route(
            "/oauth2/token",
            post(move |body: String| {
                let issued = issued.clone();
                async move {
                    assert!(body.contains("grant_type=client_credentials"), "{body}");
                    assert!(body.contains("scope=vardefulla-datamangder%3Aread"), "{body}");
                    let n = issued.fetch_add(1, Ordering::SeqCst) + 1;
                    Json(json!({ "access_token": format!("t{n}"), "token_type": "Bearer", "expires_in": 3600 }))
                }
            }),
        )
        .route(
            "/v1/organisationer",
            post(move |headers: HeaderMap, Json(body): Json<Value>| async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                if !auth.starts_with("Bearer t") || revoked.iter().any(|t| auth == format!("Bearer {t}")) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                match body["identitetsbeteckning"].as_str() {
                    Some("5560160680") => Json(json!({ "organisationer": [ {
                        "organisationsform": { "kod": "AB" },
                        "organisationsnamn": { "organisationsnamnLista": [
                            { "namn": "Exempel AB", "organisationsnamntyp": { "kod": "FORETAGSNAMN" } } ] },
                        "postadressOrganisation": { "postadress": {
                            "utdelningsadress": "Storgatan 1", "postnummer": "11122", "postort": "STOCKHOLM" } }
                    } ] }))
                    .into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeBolagsverket { base, tokens_issued }
}

fn client_for(fake: &FakeBolagsverket) -> Bolagsverket {
    Bolagsverket::new(&format!("{}/oauth2/token", fake.base), &format!("{}/v1", fake.base), "id".into(), "secret".into())
}

fn lookup(org_nr: &str) -> pb::LookupCompanyRequest {
    pb::LookupCompanyRequest { org_nr: org_nr.into() }
}

#[tokio::test]
async fn lookup_prefills_from_bolagsverket_and_reuses_the_token() {
    let fake = fake_bolagsverket(&[]).await;
    let server = TestServer::start_with_bolagsverket(client_for(&fake)).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();

    let found = api.lookup_company(authed(lookup("556016-0680"), &anna)).await.unwrap().into_inner();
    let missing = api.lookup_company(authed(lookup("556036-0793"), &anna)).await.unwrap_err();

    assert_eq!(found.org_nr, "556016-0680");
    assert_eq!(found.name, "Exempel AB");
    assert_eq!(found.legal_form(), pb::LegalForm::Aktiebolag);
    assert_eq!(found.address.unwrap(), pb::Address { street: "Storgatan 1".into(), postal_code: "11122".into(), city: "STOCKHOLM".into() });
    assert_eq!(code_of(missing), (Code::NotFound, "lookup_not_found".into()));
    assert_eq!(fake.tokens_issued.load(Ordering::SeqCst), 1);
    // Nothing was stored.
    assert!(api.list_companies(authed(pb::ListCompaniesRequest {}, &anna)).await.unwrap().into_inner().companies.is_empty());
}

#[tokio::test]
async fn a_401_clears_the_cached_token() {
    let fake = fake_bolagsverket(&["t1"]).await;
    let server = TestServer::start_with_bolagsverket(client_for(&fake)).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();

    let first = api.lookup_company(authed(lookup("556016-0680"), &anna)).await.unwrap_err();
    let second = api.lookup_company(authed(lookup("556016-0680"), &anna)).await.unwrap();

    assert_eq!(code_of(first), (Code::Unavailable, "lookup_failed".into()));
    assert_eq!(second.into_inner().name, "Exempel AB");
    assert_eq!(fake.tokens_issued.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lookup_without_configuration_or_for_a_personnummer_is_refused() {
    let fake = fake_bolagsverket(&[]).await;
    let configured = TestServer::start_with_bolagsverket(client_for(&fake)).await;
    let unconfigured = TestServer::start().await;
    let anna = configured.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = unconfigured.sign_up(&mut device(), "bo@example.se", None).await;

    let personal = configured.companies().lookup_company(authed(lookup("19121212-1212"), &anna)).await.unwrap_err();
    let invalid = configured.companies().lookup_company(authed(lookup("556016-0681"), &anna)).await.unwrap_err();
    let off = unconfigured.companies().lookup_company(authed(lookup("556016-0680"), &bo)).await.unwrap_err();
    let anonymous = configured.companies().lookup_company(lookup("556016-0680")).await.unwrap_err();

    assert_eq!(code_of(personal), (Code::FailedPrecondition, "lookup_personal_number".into()));
    assert_eq!(code_of(invalid), (Code::InvalidArgument, "invalid_org_nr".into()));
    assert_eq!(code_of(off), (Code::FailedPrecondition, "lookup_unavailable".into()));
    assert_eq!(code_of(anonymous), (Code::Unauthenticated, "not_signed_in".into()));
    assert_eq!(fake.tokens_issued.load(Ordering::SeqCst), 0); // the personnummer never left
}
```

- [ ] **Step 7: Run the tests to verify they fail**

Run: `cargo test -p doris-server --test companies`
Expected: compile error, because `CompanyApi::new` takes 1 argument while the harness passes 2

- [ ] **Step 8: Wire the lookup into `CompanyApi`** (`company.rs`)

```rust
use crate::bolagsverket::{Bolagsverket, LookupError};
use doris_company::domain::OrgNr;

pub struct CompanyApi {
    pool: SqlitePool,
    /// `None` when no Bolagsverket credentials are configured.
    bolagsverket: Option<Bolagsverket>,
}

impl CompanyApi {
    pub fn new(pool: SqlitePool, bolagsverket: Option<Bolagsverket>) -> Self {
        Self { pool, bolagsverket }
    }
    // member_company unchanged
}
```

Replace `lookup_company`:
```rust
async fn lookup_company(
    &self,
    request: Request<pb::LookupCompanyRequest>,
) -> Result<Response<pb::LookupCompanyResponse>, Status> {
    signed_in_user(&self.pool, &request).await?;
    let org_nr = OrgNr::parse(&request.get_ref().org_nr).map_err(domain_status)?;
    if org_nr.is_personal_identity_number() {
        return Err(Status::failed_precondition("lookup_personal_number"));
    }
    let bolagsverket = self
        .bolagsverket
        .as_ref()
        .ok_or_else(|| Status::failed_precondition("lookup_unavailable"))?;
    let found = bolagsverket.lookup(&org_nr).await.map_err(|err| match err {
        LookupError::NotFound => Status::not_found("lookup_not_found"),
        LookupError::Failed(reason) => {
            tracing::warn!("bolagsverket: {reason}");
            Status::unavailable("lookup_failed")
        }
    })?;
    Ok(Response::new(pb::LookupCompanyResponse {
        org_nr: org_nr.formatted(),
        name: found.name,
        legal_form: legal_form_message(found.legal_form) as i32,
        address: Some(address_message(&found.address)),
    }))
}
```

Update the `start_with`/`launch` callers in the harness to pass `None` by default.

- [ ] **Step 9: Configure it in `main.rs`**

Add to `Config`:
```rust
/// Bolagsverket API client (värdefulla datamängder). Without it, company
/// details are entered by hand.
#[arg(long, env = "DORIS_BOLAGSVERKET_CLIENT_ID")]
bolagsverket_client_id: Option<String>,
#[arg(long, env = "DORIS_BOLAGSVERKET_CLIENT_SECRET", hide_env_values = true)]
bolagsverket_client_secret: Option<String>,
#[arg(long, env = "DORIS_BOLAGSVERKET_TOKEN_URL", default_value = doris_server::bolagsverket::TOKEN_URL)]
bolagsverket_token_url: String,
#[arg(long, env = "DORIS_BOLAGSVERKET_API_URL", default_value = doris_server::bolagsverket::API_URL)]
bolagsverket_api_url: String,
```
In `run`, before building the router:
```rust
let bolagsverket = match (config.bolagsverket_client_id, config.bolagsverket_client_secret) {
    (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Some(Bolagsverket::new(
        &config.bolagsverket_token_url,
        &config.bolagsverket_api_url,
        id,
        secret,
    )),
    _ => {
        tracing::info!("Bolagsverket lookup off: DORIS_BOLAGSVERKET_CLIENT_ID/_SECRET not set");
        None
    }
};
```
Pass `CompanyApi::new(pool.clone(), bolagsverket)`.

- [ ] **Step 10: Add CA certificates to the runtime image** (`Dockerfile`)

Change the runtime `apt-get install` line to `libssl3 ca-certificates`. Without them, TLS to Bolagsverket fails in the container.

- [ ] **Step 11: Record the deviations in the spec**

In `docs/superpowers/specs/2026-09-30-foretag-design.md`, section "Bolagsverket-klienten":
- Change `reqwest` med `rustls-tls` to `reqwest` med `native-tls` (samma system-OpenSSL som webauthn-rs redan länkar).
- Change the variables to `DORIS_BOLAGSVERKET_TOKEN_URL` and `DORIS_BOLAGSVERKET_API_URL`.
- Add: "Ett organisationsnummer som är ett personnummer skickas aldrig till Bolagsverket (`lookup_personal_number`)."
- Add the error codes `lookup_personal_number`, `invalid_address`, `invalid_legal_form` and `invalid_accounting_method` to the code table.

- [ ] **Step 12: Run everything and commit**

Run: `cargo test -p doris-server && cargo clippy --workspace -- -D warnings`
Expected: all passed. If `tests/cli.rs` asserts on `--help` output, update its expectation for the new flags.

```bash
git add Cargo.toml Cargo.lock crates/server Dockerfile docs/superpowers/specs/2026-09-30-foretag-design.md
git commit -m "Look up company details from Bolagsverket"
```

---

### Task 8: Frontend plumbing: API client, error texts, formatting, form controls

**Files:**
- Modify: `crates/web/src/api.rs`, `crates/web/src/errors.rs`, `crates/web/src/format.rs`, `crates/web/src/ui.rs`, `crates/web/Cargo.toml`

**Interfaces:**
- Produces:
  - `api::company_api() -> CompanyApi` and `api::cpb` (= `doris_proto::company::v1`)
  - `format::legal_form_label(cpb::LegalForm) -> &'static str`, `format::LEGAL_FORMS: [cpb::LegalForm; 8]`
  - `format::current_year() -> i32` (wasm only)
  - `ui::Select { label, id, value: RwSignal<String>, children }` and `ui::Radio { label, name, checked: Signal<bool>, on_select }`

- [ ] **Step 1: Write the failing tests**

In `errors.rs`, add to the existing `tests` module:
```rust
#[test]
fn company_codes_have_swedish_messages() {
    for code in [
        "invalid_org_nr", "invalid_company_name", "invalid_address", "invalid_legal_form",
        "invalid_accounting_method", "invalid_fiscal_year", "company_exists", "company_not_found",
        "user_not_found", "lookup_unavailable", "lookup_personal_number", "lookup_not_found",
        "lookup_failed",
    ] {
        assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
    }
}
```

In `format.rs`, add:
```rust
#[test]
fn every_legal_form_has_a_swedish_label() {
    assert_eq!(legal_form_label(cpb::LegalForm::Aktiebolag), "Aktiebolag");
    assert_eq!(legal_form_label(cpb::LegalForm::EnskildFirma), "Enskild firma");
    for form in LEGAL_FORMS {
        assert!(!legal_form_label(form).is_empty());
    }
    assert!(!LEGAL_FORMS.contains(&cpb::LegalForm::Unspecified));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-web`
Expected: compile error for `legal_form_label`; the error-code test fails once it compiles

- [ ] **Step 3: Implement**

`api.rs`:
```rust
//! gRPC-Web clients for the Doris API.

use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
// … existing imports …

pub use doris_proto::auth::v1 as pb;
pub use doris_proto::company::v1 as cpb;

pub type Api = AuthServiceClient<Client>;
pub type CompanyApi = CompanyServiceClient<Client>;

pub fn api() -> Api {
    AuthServiceClient::new(client())
}

pub fn company_api() -> CompanyApi {
    CompanyServiceClient::new(client())
}

/// Cookies are always sent, so the session also works when the frontend is
/// served from another origin (CDN) on the same site.
fn client() -> Client {
    let options = FetchOptions::new().credentials(Credentials::Include);
    Client::new_with_options(base_url(), options)
}
```

Add these to `message()` in `errors.rs`:
```rust
"invalid_org_nr" => "Ange ett giltigt organisationsnummer (10 siffror).",
"invalid_company_name" => "Företagsnamnet måste vara 1–200 tecken.",
"invalid_address" => "Adressfälten får vara högst 200 tecken.",
"invalid_legal_form" => "Välj juridisk form.",
"invalid_accounting_method" => "Välj bokföringsmetod.",
"invalid_fiscal_year" => "Räkenskapsåret ska börja den 1:a och sluta sista dagen i en månad, vara högst 18 månader, och för enskild firma och handelsbolag sluta 31 december.",
"company_exists" => "Företaget finns redan i Doris. Be någon som har tillgång att lägga till dig.",
"company_not_found" => "Företaget finns inte eller så saknar du tillgång.",
"user_not_found" => "Det finns ingen användare med den e-postadressen.",
"lookup_unavailable" => "Hämtning från Bolagsverket är inte konfigurerad. Fyll i uppgifterna själv.",
"lookup_personal_number" => "Enskilda firmor hämtas inte från Bolagsverket. Fyll i uppgifterna själv.",
"lookup_not_found" => "Bolagsverket hittade inget företag med det numret.",
"lookup_failed" => "Bolagsverket svarade inte. Försök igen eller fyll i uppgifterna själv.",
```

`format.rs` (add):
```rust
use crate::api::cpb;

/// Legal forms in the order the form offers them.
pub const LEGAL_FORMS: [cpb::LegalForm; 8] = [
    cpb::LegalForm::Aktiebolag,
    cpb::LegalForm::EnskildFirma,
    cpb::LegalForm::Handelsbolag,
    cpb::LegalForm::Kommanditbolag,
    cpb::LegalForm::EkonomiskForening,
    cpb::LegalForm::IdeellForening,
    cpb::LegalForm::Stiftelse,
    cpb::LegalForm::Other,
];

pub fn legal_form_label(form: cpb::LegalForm) -> &'static str {
    match form {
        cpb::LegalForm::Unspecified => "Välj…",
        cpb::LegalForm::Aktiebolag => "Aktiebolag",
        cpb::LegalForm::Handelsbolag => "Handelsbolag",
        cpb::LegalForm::Kommanditbolag => "Kommanditbolag",
        cpb::LegalForm::EnskildFirma => "Enskild firma",
        cpb::LegalForm::EkonomiskForening => "Ekonomisk förening",
        cpb::LegalForm::IdeellForening => "Ideell förening",
        cpb::LegalForm::Stiftelse => "Stiftelse",
        cpb::LegalForm::Other => "Annan",
    }
}

pub fn accounting_method_label(method: cpb::AccountingMethod) -> &'static str {
    match method {
        cpb::AccountingMethod::Cash => "Kontantmetoden",
        cpb::AccountingMethod::Invoice => "Faktureringsmetoden",
        cpb::AccountingMethod::Unspecified => "",
    }
}

/// This year by the browser's clock, for the default räkenskapsår.
pub fn current_year() -> i32 {
    js_sys::Date::new_0().get_full_year() as i32
}
```

Add `js-sys = "0.3"` to `crates/web/Cargo.toml`. It is already in the lock file through wasm-bindgen, and `js_sys::Date` also compiles on the host; it is only called in the browser.

`ui.rs`: generate the preset's classes first.
```bash
cd "$(mktemp -d)" && npx shadcn@latest init -t vite -b radix -p b1Gdz9bFY -y && npx shadcn@latest add native-select radio-group -y
```
Copy the `<select>` className from `src/components/ui/native-select.tsx` into `SELECT`, and the radio item's className from `radio-group.tsx` into `RADIO`. Radix renders a button there; keep only the visual classes and apply them to a native `<input type="radio" class="appearance-none …">`. Then add:
```rust
const SELECT: &str = "…copied from native-select.tsx…";
const RADIO: &str = "…copied from radio-group.tsx RadioGroupItem…";

/// A labelled native select bound to `value` (the option's `value`).
#[component]
pub fn Select(label: &'static str, id: &'static str, value: RwSignal<String>, children: Children) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <label for=id class=LABEL>{label}</label>
            <select id=id name=id class=SELECT prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))>
                {children()}
            </select>
        </div>
    }
}

/// One labelled radio button; `on_select` runs when it is chosen.
#[component]
pub fn Radio(
    label: &'static str,
    name: &'static str,
    #[prop(into)] checked: Signal<bool>,
    on_select: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <label class=LABEL>
            <input type="radio" name=name class=RADIO prop:checked=checked on:change=move |_| on_select() />
            {label}
        </label>
    }
}
```

- [ ] **Step 4: Run the tests and the wasm lint**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: passes. `Select`/`Radio` may warn as unused until Task 9. If clippy fails with `dead_code`, move the commit to the end of Task 9 instead of adding `allow`s.

- [ ] **Step 5: Commit**

```bash
git add crates/web Cargo.lock
git commit -m "Add company API client, Swedish error texts and form controls"
```

---

### Task 9: Frontend pages and routes

**Files:**
- Create: `crates/web/src/pages/companies.rs`, `crates/web/src/pages/new_company.rs`, `crates/web/src/pages/company.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`

**Interfaces:**
- Consumes: `company_api()`, `cpb`, `describe`, `LEGAL_FORMS`, `legal_form_label`, `accounting_method_label`, `current_year`, `Select`, `Radio`, `Field`, `Button`, `Card`, `ErrorAlert`
- Produces: the routes `/companies`, `/companies/new` and `/companies/:id`, and a "Företag" nav link. The Playwright tests in Task 10 rely on these labels:
  - `Organisationsnummer`
  - the button `Hämta från Bolagsverket`
  - `Företagsnamn`, `Juridisk form`, `Utdelningsadress`, `Postnummer`, `Postort`
  - `Räkenskapsåret börjar`, `Räkenskapsåret slutar`
  - the radios `Kontantmetoden` and `Faktureringsmetoden`
  - the button `Spara företag`
  - the link `Lägg till företag`
  - on the company page, the email field `E-post` and the button `Lägg till medlem`

The UI is tested end to end in Task 10; the pure logic it uses was unit-tested in Task 8.

- [ ] **Step 1: Write the list page** (`pages/companies.rs`)

```rust
use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::ui::{Card, ErrorAlert};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn Companies() -> impl IntoView {
    let companies = RwSignal::new(None::<Vec<cpb::CompanySummary>>);
    let error = RwSignal::new(None::<String>);
    spawn_local(async move {
        match company_api().list_companies(cpb::ListCompaniesRequest {}).await {
            Ok(list) => companies.set(Some(list.into_inner().companies)),
            Err(status) => error.set(Some(describe(&status))),
        }
    });

    view! {
        <Card title="Företag" description="Företagen du sköter bokföringen åt.">
            <div class="grid gap-4">
                <ErrorAlert message=error />
                {move || companies.get().map(|list| if list.is_empty() {
                    view! { <p class="text-muted-foreground">"Inga företag än."</p> }.into_any()
                } else {
                    view! {
                        <ul class="grid gap-2">
                            {list.into_iter().map(|c| view! {
                                <li class="flex justify-between gap-2">
                                    <A href=format!("/companies/{}", c.id) attr:class="font-medium hover:underline">{c.name}</A>
                                    <span class="text-muted-foreground">{c.org_nr}</span>
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                })}
                <A href="/companies/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Lägg till företag"</A>
            </div>
        </Card>
    }
}
```

- [ ] **Step 2: Write the new-company form** (`pages/new_company.rs`)

```rust
use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::format::{LEGAL_FORMS, current_year, legal_form_label};
use crate::ui::{Button, Card, ErrorAlert, Field, Radio, Select, Variant};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

#[component]
pub fn NewCompany() -> impl IntoView {
    let year = current_year();
    let org_nr = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let legal_form = RwSignal::new(String::new()); // the enum's i32, as text
    let street = RwSignal::new(String::new());
    let postal_code = RwSignal::new(String::new());
    let city = RwSignal::new(String::new());
    let start = RwSignal::new(format!("{year}-01-01"));
    let end = RwSignal::new(format!("{year}-12-31"));
    let method = RwSignal::new(cpb::AccountingMethod::Unspecified);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let navigate = use_navigate();

    let fetch = move |_| {
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = cpb::LookupCompanyRequest { org_nr: org_nr.get_untracked() };
            match company_api().lookup_company(request).await {
                Ok(found) => {
                    let found = found.into_inner();
                    let address = found.address.clone().unwrap_or_default();
                    org_nr.set(found.org_nr.clone());
                    name.set(found.name.clone());
                    legal_form.set(found.legal_form.to_string());
                    street.set(address.street);
                    postal_code.set(address.postal_code);
                    city.set(address.city);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = cpb::CreateCompanyRequest {
                org_nr: org_nr.get_untracked(),
                name: name.get_untracked(),
                legal_form: legal_form.get_untracked().parse().unwrap_or(0),
                address: Some(cpb::Address {
                    street: street.get_untracked(),
                    postal_code: postal_code.get_untracked(),
                    city: city.get_untracked(),
                }),
                fiscal_year_start: start.get_untracked(),
                fiscal_year_end: end.get_untracked(),
                accounting_method: method.get_untracked() as i32,
            };
            match company_api().create_company(request).await {
                Ok(created) => navigate(&format!("/companies/{}", created.into_inner().company_id), Default::default()),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <Card title="Lägg till företag" description="Hämta uppgifterna från Bolagsverket eller fyll i dem själv.">
            <form class="grid gap-4" novalidate on:submit=submit>
                <Field label="Organisationsnummer" id="org_nr" value=org_nr placeholder="556016-0680" />
                <Button variant=Variant::Ghost kind="button" disabled=busy on:click=fetch>"Hämta från Bolagsverket"</Button>
                <Field label="Företagsnamn" id="name" value=name autocomplete="organization" />
                <Select label="Juridisk form" id="legal_form" value=legal_form>
                    <option value="">{legal_form_label(cpb::LegalForm::Unspecified)}</option>
                    {LEGAL_FORMS.map(|f| view! { <option value=(f as i32).to_string()>{legal_form_label(f)}</option> }).collect_view()}
                </Select>
                <Field label="Utdelningsadress" id="street" value=street />
                <Field label="Postnummer" id="postal_code" value=postal_code />
                <Field label="Postort" id="city" value=city />
                <Field label="Räkenskapsåret börjar" id="fiscal_year_start" kind="date" value=start />
                <Field label="Räkenskapsåret slutar" id="fiscal_year_end" kind="date" value=end
                    hint=Signal::derive(|| Some("Första räkenskapsåret får vara 1–18 månader. Enskild firma och handelsbolag följer kalenderåret."))
                />
                <fieldset class="grid gap-2">
                    <legend class="text-xs/relaxed font-medium">"Bokföringsmetod"</legend>
                    <Radio label="Faktureringsmetoden" name="method"
                        checked=Signal::derive(move || method.get() == cpb::AccountingMethod::Invoice)
                        on_select=move || method.set(cpb::AccountingMethod::Invoice) />
                    <Radio label="Kontantmetoden" name="method"
                        checked=Signal::derive(move || method.get() == cpb::AccountingMethod::Cash)
                        on_select=move || method.set(cpb::AccountingMethod::Cash) />
                    <p class="text-xs/relaxed text-muted-foreground">"Kontantmetoden får bara användas om nettoomsättningen normalt är högst 3 miljoner kronor per år."</p>
                </fieldset>
                <ErrorAlert message=error />
                <Button disabled=busy>"Spara företag"</Button>
            </form>
        </Card>
    }
}
```

Check that `Field`'s `hint` prop accepts `Signal<Option<&'static str>>` this way; it is declared `#[prop(optional, into)]`. If `on:click` doesn't forward through the `Button` component, it goes directly on the rendered `<button>`: Leptos 0.8 forwards `on:` handlers on components to their root element, which is how `app.rs` already uses `on:click` on `Button`.

- [ ] **Step 3: Write the company page** (`pages/company.rs`)

```rust
use crate::api::{company_api, cpb};
use crate::errors::describe;
use crate::format::{accounting_method_label, legal_form_label};
use crate::ui::{Button, Card, ErrorAlert, Field};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_params_map;

#[component]
pub fn CompanyPage() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();
    let company = RwSignal::new(None::<cpb::Company>);
    let members = RwSignal::new(Vec::<cpb::Member>::new());
    let email = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = id();
        spawn_local(async move {
            let mut api = company_api();
            match api.get_company(cpb::GetCompanyRequest { company_id: company_id.clone() }).await {
                Ok(c) => company.set(Some(c.into_inner())),
                Err(status) => return error.set(Some(describe(&status))),
            }
            match api.list_members(cpb::ListMembersRequest { company_id }).await {
                Ok(list) => members.set(list.into_inner().members),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    load();

    let add = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let request = cpb::AddMemberRequest { company_id: id(), email: email.get_untracked() };
            match company_api().add_member(request).await {
                Ok(_) => {
                    email.set(String::new());
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <ErrorAlert message=error />
            {move || company.get().map(|c| {
                let address = c.address.clone().unwrap_or_default();
                let postal = format!("{} {}", address.postal_code, address.city).trim().to_owned();
                view! {
                    <section class="grid gap-2 text-xs/relaxed">
                        <h1 class="text-sm font-medium">{c.name.clone()}</h1>
                        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
                            <dt class="text-muted-foreground">"Organisationsnummer"</dt><dd>{c.org_nr.clone()}</dd>
                            <dt class="text-muted-foreground">"Juridisk form"</dt><dd>{legal_form_label(c.legal_form())}</dd>
                            <dt class="text-muted-foreground">"Adress"</dt><dd>{address.street.clone()}" "{postal}</dd>
                            <dt class="text-muted-foreground">"Räkenskapsår"</dt><dd>{format!("{} – {}", c.fiscal_year_start, c.fiscal_year_end)}</dd>
                            <dt class="text-muted-foreground">"Bokföringsmetod"</dt><dd>{accounting_method_label(c.accounting_method())}</dd>
                        </dl>
                    </section>
                }
            })}
            <Show when=move || company.get().is_some()>
                <Card title="Medlemmar" description="De som har tillgång till företaget.">
                    <ul class="mb-4 grid gap-2">
                        <For each=move || members.get() key=|m| m.email.clone() let(member)>
                            <li class="flex justify-between gap-2">
                                <span>{member.display_name}</span>
                                <span class="text-muted-foreground">{member.email}</span>
                            </li>
                        </For>
                    </ul>
                    <form class="grid gap-4" novalidate on:submit=add>
                        <Field label="E-post" id="member_email" kind="email" value=email />
                        <Button disabled=busy>"Lägg till medlem"</Button>
                    </form>
                </Card>
            </Show>
        </div>
    }
}
```

- [ ] **Step 4: Register the pages and routes**

In `pages/mod.rs`, add `mod companies; mod company; mod new_company;` and `pub use companies::Companies; pub use company::CompanyPage; pub use new_company::NewCompany;`.

In `app.rs`:
- Import `Companies, CompanyPage, NewCompany`.
- Add the routes. `/companies/new` must come before `/companies/:id`; leptos_router ranks static segments first anyway, but keep the order for readability.
```rust
<Route path=path!("/companies") view=|| view! { <SignedIn><Companies /></SignedIn> } />
<Route path=path!("/companies/new") view=|| view! { <SignedIn><NewCompany /></SignedIn> } />
<Route path=path!("/companies/:id") view=|| view! { <SignedIn><CompanyPage /></SignedIn> } />
```
- In `Header`, add `<A href="/companies" attr:class="text-muted-foreground hover:text-foreground">"Företag"</A>` as the first link inside the signed-in `<Show>`.

- [ ] **Step 5: Build and lint**

Run:
```bash
cargo test -p doris-web
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
make web
```
Expected: all succeed

- [ ] **Step 6: Commit**

```bash
git add crates/web
git commit -m "Add company list, form and detail pages"
```

---

### Task 10: End-to-end tests

**Files:**
- Create: `e2e/tests/companies.spec.ts`

**Interfaces:**
- Consumes: `register`, `newPerson`, `app` from `e2e/tests/fixtures.ts`, and the UI labels listed in Task 9. The server runs without Bolagsverket credentials.

- [ ] **Step 1: Write the tests**

```ts
import { expect, register, test } from "./fixtures";
import type { Page } from "@playwright/test";

async function addCompany(page: Page, app: string, orgNr: string, name: string) {
  await page.goto(`${app}/companies`);
  await page.getByRole("link", { name: "Lägg till företag" }).click();
  await page.getByLabel("Organisationsnummer").fill(orgNr);
  await page.getByLabel("Företagsnamn").fill(name);
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Postort").fill("Stockholm");
  await page.getByLabel("Räkenskapsåret börjar").fill("2026-01-01");
  await page.getByLabel("Räkenskapsåret slutar").fill("2026-12-31");
  await page.getByLabel("Faktureringsmetoden").check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
}

test("a user adds a company by hand and finds it in the list", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.getByRole("link", { name: "Företag" }).click();
  await expect(page.getByText("Inga företag än.")).toBeVisible();

  await addCompany(page, app, "5560160680", "Exempel AB");

  await expect(page.getByText("556016-0680")).toBeVisible();
  await expect(page.getByText("Faktureringsmetoden")).toBeVisible();
  await expect(page.getByText(/^\d{4}-01-01 – \d{4}-12-31$/)).toBeVisible();
  await page.getByRole("link", { name: "Företag" }).click();
  await expect(page.getByRole("link", { name: "Exempel AB" })).toBeVisible();
});

test("the form explains invalid input and a missing Bolagsverket setup in Swedish", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.goto(`${app}/companies/new`);

  await page.getByLabel("Organisationsnummer").fill("556016-0680");
  await page.getByRole("button", { name: "Hämta från Bolagsverket" }).click();
  await expect(page.getByRole("alert")).toHaveText("Hämtning från Bolagsverket är inte konfigurerad. Fyll i uppgifterna själv.");

  // Everything else valid, so the org nr is the error reported (the server
  // checks legal form and method before the org nr).
  await page.getByLabel("Organisationsnummer").fill("556016-0681");
  await page.getByLabel("Företagsnamn").fill("Exempel AB");
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Faktureringsmetoden").check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange ett giltigt organisationsnummer (10 siffror).");
});

test("a colleague sees a company only after being added as a member", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.getByRole("link", { name: "Inbjudningar" }).click();
  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  const link = await page.getByLabel("Inbjudningslänk").inputValue();
  const bo = await newPerson();
  await register(bo, app, { email: "bo@example.se", name: "Bo", invitationLink: link });

  await addCompany(page, app, "5560160680", "Exempel AB");
  const companyUrl = page.url();
  await bo.goto(companyUrl);
  await expect(bo.getByRole("alert")).toHaveText("Företaget finns inte eller så saknar du tillgång.");

  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Lägg till medlem" }).click();
  await expect(page.getByText("bo@example.se")).toBeVisible();

  await bo.goto(`${app}/companies`);
  await bo.getByRole("link", { name: "Exempel AB" }).click();
  await expect(bo.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
});
```

The `Räkenskapsår` assertion accepts any calendar year, because the page shows the year that contains today. Anna's "E-post" field on the company page is the member form; the invitations page, where the label is the same, is a different page.

- [ ] **Step 2: Run the tests and see them pass**

Run: `make e2e`
Expected: all specs pass, including the existing `auth`, `assets` and `design`. If a new test fails, the cause is in Tasks 8–9: fix it there, not in the test, unless a label in the test is wrong.

- [ ] **Step 3: Commit**

```bash
git add e2e/tests/companies.spec.ts
git commit -m "Cover adding companies and members end to end"
```

---

### Task 11: Documentation, size budget and final verification

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Update `AGENTS.md`**
- **Layout:** add `crates/company   doris-company: companies, members, fiscal year and accounting method`.
- **Layout:** add `proto/doris/company/v1/company.proto` in the API section, next to `auth.proto`, and say that the mapping for company codes is in `crates/server/src/company.rs` (`status`, `domain_status`).
- **Commands:** add `DORIS_BOLAGSVERKET_CLIENT_ID`, `DORIS_BOLAGSVERKET_CLIENT_SECRET`, `DORIS_BOLAGSVERKET_TOKEN_URL` and `DORIS_BOLAGSVERKET_API_URL` to the server configuration list.
- **API:** add a paragraph, in English:
  > Company lookup uses Bolagsverket's free "värdefulla datamängder" API (OAuth2 client credentials, register at portal.api.bolagsverket.se). Without credentials the lookup answers `lookup_unavailable` and details are typed in. An org nr can be a personnummer (enskild firma): never log it, and never send a personnummer to Bolagsverket.
- **Stack/Principles:** note that the server's only outbound HTTP is `reqwest` (native-tls, the same OpenSSL as webauthn-rs).

- [ ] **Step 2: Run the full verification**

```bash
make test
cargo clippy --workspace -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
make dist          # fails if the wasm is over WASM_BUDGET
make e2e-dist
```
Expected: everything passes. Note the wasm size from `make dist` in the commit message. If it is over budget, look at `js-sys` usage first; the pages themselves are small.

- [ ] **Step 3: Verify at runtime with the `verify` skill**

Run `/verify`, which builds the release binary and drives the browser plus gRPC-Web. Check that you can add a company by hand, that it shows up in the list and on its page, and that Hämta shows the "inte konfigurerad" message without credentials.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Document companies and the Bolagsverket lookup"
```
