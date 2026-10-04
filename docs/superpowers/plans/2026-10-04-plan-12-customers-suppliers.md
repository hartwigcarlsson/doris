# Plan 12: Customers and Suppliers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Members keep a customer register (kunder) and a supplier register (leverantörer) per company, numbered 1..n, editable, deactivatable and never removed.

**Architecture:**
- A new crate, `crates/invoicing` (`doris-invoicing`), holds pure domain logic in `domain.rs`, store functions in `lib.rs` and projections in `projections.rs`. Fakturor will land here in later steps.
- The logic is written once, generic over the details type (`Register<D>`, `Change<D>`). `CustomerEvent` and `SupplierEvent` are only the stored shapes, so the event log reads `CustomerAdded`, `SupplierUpdated` and so on.
- There is one stream per company and register: `customers-{company_id}` and `suppliers-{company_id}`. The projections `customers` and `suppliers` keep the details as JSON next to `(company_id, number, active)`.
- A new `InvoicingService` (gRPC-Web) is served from `crates/server/src/invoicing.rs`. Two Leptos pages, `/customers` and `/suppliers`, use it.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), serde_json, tonic 0.14 gRPC-Web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-kunder-leverantorer-design.md`

## Global Constraints
- **TDD is mandatory.** Every behaviour starts as a failing test, and each task ends with a commit.
- **Commit messages** are in English, and each ends with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01RBPdfcwGSBXA9x2g5jEGxd
  ```
- **Language:** code, identifiers, URLs, proto and event names are in English. Only text the user sees in the UI is Swedish.
- **Event sourcing rules:**
  - The `events` table is append-only.
  - Payloads carry `schema_version` 1.
  - Projections are updated in the same `BEGIN IMMEDIATE` transaction as the append (`doris_eventstore::begin`).
  - A projection must be rebuildable from `read_all`.
- **Numbers** are decided inside the write transaction as `max(number) + 1`, or 1 for an empty register. The client never decides them, and the projection's primary key `(company_id, number)` backs that up.
- **Personal data:** an org.nr can be a personnummer, and an email address is personal data. Neither is ever logged. `OrgNr` and `Email` have redacted `Debug` impls.
- **Membership:** any member of the company may read and write. A non-member, an unknown company and a company id that is not a UUID all give `company_not_found`.
- **No new dependencies** beyond the workspace ones listed in Task 1. The wasm budget (500 KB gzipped, checked by `make dist`) still applies.
- **Lint:** `cargo clippy --workspace -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings` stay clean.

## Review Focus
1. **Blank optional fields:** a field holding only spaces must be stored as "none", never as an empty value or a validation error. Pinned in Task 2 (`blank_optional_fields_are_none`).
2. **Pasted, formatted values:** `se45 5000 0000 0583 9825 7466`, `essesess`, `SE 556016-0680 01` and `5050-1055` are accepted and normalised. Non-ASCII input such as `SÉ45…` is refused without a panic. Pinned in Task 1 (`values_are_normalised_and_non_ascii_never_panics`).
3. **Concurrent adds:** two members adding customers at the same moment never get the same number and leave no gap. Pinned in Task 3 (`concurrent_adds_get_numbers_1_to_n`).
4. **Cross-company numbers:** updating customer 1 under company B must not touch company A's customer 1, and gives `customer_not_found` when B has none. Pinned in Task 3 (`numbers_are_per_company`).
5. **Non-numeric payment terms in the form:** `trettio` must give "Betalningsvillkoret ska vara 0–365 dagar.", never be saved silently as 0. Pinned in Task 5 (e2e).

---

### Task 1: The crate and its value types

**Files:**
- Modify: `Cargo.toml` (workspace members and dependencies)
- Modify: `crates/company/src/domain.rs` (make `luhn` public and count from the right)
- Modify: `crates/company/tests/domain.rs`
- Create: `crates/invoicing/Cargo.toml`
- Create: `crates/invoicing/src/lib.rs`
- Create: `crates/invoicing/src/domain.rs`
- Test: `crates/invoicing/tests/domain.rs`

**Interfaces:**
- Produces:
  - `doris_company::domain::luhn(digits: &str) -> bool`, with weights 1,2,1,2… from the right. It is identical to today's result for ten digits.
  - In `doris_invoicing::domain`: `DomainError`, plus the value types `PartyName`, `Email`, `VatNumber`, `PaymentTerms`, `Bankgiro`, `Plusgiro`, `Iban` and `Bic`.
  - Every type has `parse(&str) -> Result<Self, DomainError>` except `PaymentTerms`, which has `new(u32)`.
  - Every type has `as_str()` except `PaymentTerms`, which has `get() -> u32`.
  - `Bankgiro`, `Plusgiro` and `Iban` also have `formatted() -> String`.

- [ ] **Step 1: Write the failing Luhn test in `crates/company/tests/domain.rs`**

```rust
#[test]
fn luhn_counts_from_the_right_for_any_length() {
    assert!(luhn("5560160680"));
    assert!(!luhn("5560160681"));
    // Seven and two digits: bankgiro 123-4566 and plusgiro 1-8.
    assert!(luhn("1234566"));
    assert!(luhn("18"));
    assert!(!luhn("1234567"));
}
```

- [ ] **Step 2: Run it and check that it fails**

Run: `cargo test -p doris-company --test domain luhn`
Expected: compile error, because `luhn` is private.

- [ ] **Step 3: Make `luhn` public and count from the right**

In `crates/company/src/domain.rs`, replace the `luhn` function:

```rust
/// The Luhn check over all digits, weights 1,2,1,2… from the right. Used
/// for organisationsnummer, bankgiro and plusgiro.
pub fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 2 };
            if d > 9 { d - 9 } else { d }
        })
        .sum();
    sum.is_multiple_of(10)
}
```

Run: `cargo test -p doris-company`
Expected: PASS. The org.nr tests still pass, because left-first and right-first weights agree for ten digits.

- [ ] **Step 4: Scaffold the crate**

In the root `Cargo.toml`, add `"crates/invoicing"` to `members`, and add `doris-invoicing = { path = "crates/invoicing" }` under `[workspace.dependencies]`.

`crates/invoicing/Cargo.toml`:
```toml
[package]
name = "doris-invoicing"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
doris-company.workspace = true
doris-eventstore.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
uuid.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio.workspace = true
```

`crates/invoicing/src/lib.rs`:
```rust
//! Customers and suppliers (kunder och leverantörer) of a company,
//! event-sourced into SQLite. Fakturor will join them here.

pub mod domain;
```

- [ ] **Step 5: Write the failing value tests in `crates/invoicing/tests/domain.rs`**

```rust
use doris_invoicing::domain::*;

#[test]
fn names_are_trimmed_and_1_to_200_characters() {
    assert_eq!(PartyName::parse("  Kund AB ").unwrap().as_str(), "Kund AB");
    assert!(PartyName::parse(&"å".repeat(200)).is_ok());
    for bad in ["", "  ", &"å".repeat(201)] {
        assert_eq!(PartyName::parse(bad), Err(DomainError::InvalidName));
    }
}

#[test]
fn emails_are_lowercased_and_need_a_local_part_and_a_domain() {
    assert_eq!(Email::parse(" Ekonomi@Kund.SE ").unwrap().as_str(), "ekonomi@kund.se");
    for bad in ["kund.se", "@kund.se", "a@b@kund.se", "a@kund", "a b@kund.se"] {
        assert_eq!(Email::parse(bad), Err(DomainError::InvalidEmail), "{bad}");
    }
    assert_eq!(
        format!("{:?}", Email::parse("a@kund.se").unwrap()),
        "Email(<redacted>)"
    );
}

#[test]
fn vat_numbers_are_two_letters_and_2_to_12_characters_and_swedish_ones_end_in_01() {
    assert_eq!(VatNumber::parse("SE556016068001").unwrap().as_str(), "SE556016068001");
    assert_eq!(VatNumber::parse("de 123456789").unwrap().as_str(), "DE123456789");
    for bad in [
        "SE5560160680",   // no 01
        "SE556016068101", // bad Luhn
        "SE55601606800",  // too short
        "S1234",
        "DE1",
        "DE1234567890123",
        "DE12_34",
    ] {
        assert_eq!(VatNumber::parse(bad), Err(DomainError::InvalidVatNumber), "{bad}");
    }
}

#[test]
fn payment_terms_are_0_to_365_days() {
    assert_eq!(PaymentTerms::new(0).unwrap().get(), 0);
    assert_eq!(PaymentTerms::new(365).unwrap().get(), 365);
    assert_eq!(PaymentTerms::new(366), Err(DomainError::InvalidPaymentTerms));
}

#[test]
fn bankgiro_is_7_or_8_digits_with_a_luhn_check_digit() {
    let bg = Bankgiro::parse("5050-1055").unwrap();
    assert_eq!((bg.as_str(), bg.formatted().as_str()), ("50501055", "5050-1055"));
    assert_eq!(Bankgiro::parse("1234566").unwrap().formatted(), "123-4566");
    for bad in ["5050-1056", "123456", "123456789", "12a4566"] {
        assert_eq!(Bankgiro::parse(bad), Err(DomainError::InvalidBankgiro), "{bad}");
    }
}

#[test]
fn plusgiro_is_2_to_8_digits_with_a_luhn_check_digit() {
    assert_eq!(Plusgiro::parse("1-8").unwrap().formatted(), "1-8");
    assert_eq!(Plusgiro::parse("12 34 56 7-4").unwrap().formatted(), "1234567-4");
    for bad in ["1-9", "8", "123456789"] {
        assert_eq!(Plusgiro::parse(bad), Err(DomainError::InvalidPlusgiro), "{bad}");
    }
}

#[test]
fn iban_passes_the_mod_97_check() {
    let iban = Iban::parse("SE45 5000 0000 0583 9825 7466").unwrap();
    assert_eq!(iban.as_str(), "SE4550000000058398257466");
    assert_eq!(iban.formatted(), "SE45 5000 0000 0583 9825 7466");
    for bad in ["SE46 5000 0000 0583 9825 7466", "SE45", "1145 5000 0000 0583 9825 7466"] {
        assert_eq!(Iban::parse(bad), Err(DomainError::InvalidIban), "{bad}");
    }
}

#[test]
fn bic_is_8_or_11_characters() {
    assert_eq!(Bic::parse("ESSESESS").unwrap().as_str(), "ESSESESS");
    assert_eq!(Bic::parse("ESSESESSXXX").unwrap().as_str(), "ESSESESSXXX");
    for bad in ["ESSESES", "ESSESESSX", "1SSESESS", "ESSE1ESS"] {
        assert_eq!(Bic::parse(bad), Err(DomainError::InvalidBic), "{bad}");
    }
}

#[test]
fn values_are_normalised_and_non_ascii_never_panics() {
    assert!(Iban::parse("se45 5000 0000 0583 9825 7466").is_ok());
    assert_eq!(Bic::parse(" essesess ").unwrap().as_str(), "ESSESESS");
    assert_eq!(VatNumber::parse("SE 556016-0680 01").unwrap().as_str(), "SE556016068001");
    assert_eq!(Iban::parse("SÉ45 5000 0000 0583 9825 7466"), Err(DomainError::InvalidIban));
    assert_eq!(Bic::parse("ÉSSESESS"), Err(DomainError::InvalidBic));
    assert_eq!(VatNumber::parse("É1234"), Err(DomainError::InvalidVatNumber));
    assert_eq!(Bankgiro::parse("5050-105å"), Err(DomainError::InvalidBankgiro));
}
```

- [ ] **Step 6: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test domain`
Expected: compile errors, because none of the types exist yet.

- [ ] **Step 7: Write the value types in `crates/invoicing/src/domain.rs`**

```rust
//! Pure rules for the customer and supplier registers. No I/O.

use doris_company::domain::{OrgNr, luhn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("name must be 1-200 characters")]
    InvalidName,
    #[error("invalid organisationsnummer")]
    InvalidOrgNr,
    #[error("address fields must be at most 200 characters")]
    InvalidAddress,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("invalid momsregistreringsnummer")]
    InvalidVatNumber,
    #[error("payment terms must be 0-365 days")]
    InvalidPaymentTerms,
    #[error("invalid bankgiro number")]
    InvalidBankgiro,
    #[error("invalid plusgiro number")]
    InvalidPlusgiro,
    #[error("invalid IBAN")]
    InvalidIban,
    #[error("invalid BIC")]
    InvalidBic,
    #[error("customer not found")]
    CustomerNotFound,
    #[error("supplier not found")]
    SupplierNotFound,
}

/// Spaces and hyphens removed, letters upper case: how numbers are pasted.
fn compact(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .collect::<String>()
        .to_ascii_uppercase()
}

fn upper_alnum(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn upper_letters(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_uppercase())
}

fn digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PartyName(String);

impl PartyName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if (1..=200).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Personal data: never logged, so `Debug` is redacted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(String);

impl std::fmt::Debug for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Email(<redacted>)")
    }
}

impl Email {
    /// The same rule as `doris_identity::domain::Email`, which this crate
    /// does not depend on.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let email = raw.trim().to_lowercase();
        let well_formed = match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }
            None => false,
        };
        if well_formed && email.len() <= 254 && !email.contains(char::is_whitespace) {
            Ok(Self(email))
        } else {
            Err(DomainError::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Momsregistreringsnummer. The format is checked, but the number is not
/// looked up in VIES.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VatNumber(String);

impl VatNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let vat = compact(raw);
        // is_ascii first: slicing by byte index must not split a char.
        let valid = vat.is_ascii()
            && vat.len() >= 4
            && vat.len() <= 14
            && upper_letters(&vat[..2])
            && upper_alnum(&vat[2..])
            && (&vat[..2] != "SE"
                || (vat.len() == 14
                    && vat.ends_with("01")
                    && digits(&vat[2..12])
                    && OrgNr::parse(&vat[2..12]).is_ok()));
        if valid { Ok(Self(vat)) } else { Err(DomainError::InvalidVatNumber) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentTerms(u16);

impl PaymentTerms {
    pub fn new(days: u32) -> Result<Self, DomainError> {
        match u16::try_from(days) {
            Ok(days) if days <= 365 => Ok(Self(days)),
            _ => Err(DomainError::InvalidPaymentTerms),
        }
    }

    pub fn get(self) -> u32 {
        self.0.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bankgiro(String);

impl Bankgiro {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let bg = compact(raw);
        if (7..=8).contains(&bg.len()) && digits(&bg) && luhn(&bg) {
            Ok(Self(bg))
        } else {
            Err(DomainError::InvalidBankgiro)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `NNN-NNNN` or `NNNN-NNNN`.
    pub fn formatted(&self) -> String {
        let (head, tail) = self.0.split_at(self.0.len() - 4);
        format!("{head}-{tail}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Plusgiro(String);

impl Plusgiro {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let pg = compact(raw);
        if (2..=8).contains(&pg.len()) && digits(&pg) && luhn(&pg) {
            Ok(Self(pg))
        } else {
            Err(DomainError::InvalidPlusgiro)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A hyphen before the check digit.
    pub fn formatted(&self) -> String {
        let (head, tail) = self.0.split_at(self.0.len() - 1);
        format!("{head}-{tail}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Iban(String);

impl Iban {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let iban = compact(raw);
        let valid = iban.is_ascii()
            && (15..=34).contains(&iban.len())
            && upper_letters(&iban[..2])
            && digits(&iban[2..4])
            && upper_alnum(&iban[4..])
            && mod_97(&iban) == Some(1);
        if valid { Ok(Self(iban)) } else { Err(DomainError::InvalidIban) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Groups of four.
    pub fn formatted(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        chars
            .chunks(4)
            .map(|group| group.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// ISO 13616: the first four characters moved to the end, letters as
/// 10..=35, the whole number mod 97, digit by digit so nothing overflows.
fn mod_97(iban: &str) -> Option<u32> {
    iban[4..].chars().chain(iban[..4].chars()).try_fold(0u32, |acc, c| {
        let d = c.to_digit(36)?;
        Some(if d < 10 { (acc * 10 + d) % 97 } else { (acc * 100 + d) % 97 })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bic(String);

impl Bic {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let bic = compact(raw);
        let valid = bic.is_ascii()
            && matches!(bic.len(), 8 | 11)
            && upper_letters(&bic[..6])
            && upper_alnum(&bic[6..]);
        if valid { Ok(Self(bic)) } else { Err(DomainError::InvalidBic) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

- [ ] **Step 8: Run the tests and check that they pass**

Run: `cargo test -p doris-invoicing --test domain && cargo clippy -p doris-invoicing -p doris-company -- -D warnings`
Expected: PASS, with no warnings.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/company crates/invoicing
git commit -m "Add doris-invoicing with the value types for customers and suppliers"
```
(End the message with the two trailer lines from Global Constraints.)

---

### Task 2: Details, events and the register rules

**Files:**
- Modify: `crates/invoicing/src/domain.rs`
- Test: `crates/invoicing/tests/domain.rs`

**Interfaces:**
- Consumes: the value types from Task 1, and `doris_company::domain::{Address, OrgNr}`.
- Produces:
  - `CustomerForm<'a>` and `SupplierForm<'a>`: raw `&str` fields plus `payment_terms: u32` on the customer form.
  - `CustomerDetails::parse(&CustomerForm) -> Result<CustomerDetails, DomainError>` and the matching `SupplierDetails::parse`.
  - `trait PartyDetails: Clone + PartialEq { const NOT_FOUND: DomainError; }`
  - `enum Change<D> { Added { number: u32, details: D }, Updated { number: u32, details: D }, Deactivated { number: u32 }, Reactivated { number: u32 } }`, with `fn number(&self) -> u32`.
  - The stored shapes `CustomerEvent` and `SupplierEvent`, with `From<Change<_>>` in both directions.
  - `struct Party<D> { pub number: u32, pub details: D, pub active: bool }`.
  - `struct Register<D>` with `from_changes`, `apply`, `get` and `parties`.
  - The decisions `add`, `update` and `set_active`.

- [ ] **Step 1: Write the failing tests (append to `crates/invoicing/tests/domain.rs`)**

```rust
fn customer_form(name: &str) -> CustomerForm<'_> {
    CustomerForm {
        name,
        org_nr: "556016-0680",
        vat_number: "",
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        email: "",
        payment_terms: 30,
    }
}

fn customer(name: &str) -> CustomerDetails {
    CustomerDetails::parse(&customer_form(name)).unwrap()
}

#[test]
fn customer_details_parse_every_field() {
    let details = CustomerDetails::parse(&CustomerForm {
        email: "Ekonomi@Kund.se",
        vat_number: "SE556016068001",
        ..customer_form(" Kund AB ")
    })
    .unwrap();
    assert_eq!(details.name.as_str(), "Kund AB");
    assert_eq!(details.org_nr.unwrap().as_str(), "5560160680");
    assert_eq!(details.vat_number.unwrap().as_str(), "SE556016068001");
    assert_eq!(details.address.city.as_deref(), Some("Stockholm"));
    assert_eq!(details.email.unwrap().as_str(), "ekonomi@kund.se");
    assert_eq!(details.payment_terms.get(), 30);
}

#[test]
fn blank_optional_fields_are_none() {
    let details = CustomerDetails::parse(&CustomerForm {
        org_nr: "  ",
        vat_number: " ",
        street: " ",
        postal_code: "",
        city: "",
        email: "  ",
        ..customer_form("Privatperson")
    })
    .unwrap();
    assert_eq!(details.org_nr, None);
    assert_eq!(details.vat_number, None);
    assert_eq!(details.address.street, None);
    assert_eq!(details.email, None);

    let supplier = SupplierDetails::parse(&SupplierForm {
        name: "Leverantör AB",
        org_nr: "",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: " ",
        plusgiro: "",
        iban: "",
        bic: "",
    })
    .unwrap();
    assert_eq!((supplier.bankgiro, supplier.iban), (None, None));
}

#[test]
fn each_bad_field_gives_its_own_error() {
    let bad = |form: CustomerForm| CustomerDetails::parse(&form).unwrap_err();
    assert_eq!(bad(customer_form("")), DomainError::InvalidName);
    assert_eq!(bad(CustomerForm { org_nr: "556016-0681", ..customer_form("K") }), DomainError::InvalidOrgNr);
    assert_eq!(bad(CustomerForm { vat_number: "SE1", ..customer_form("K") }), DomainError::InvalidVatNumber);
    let long = "å".repeat(201);
    assert_eq!(bad(CustomerForm { city: &long, ..customer_form("K") }), DomainError::InvalidAddress);
    assert_eq!(bad(CustomerForm { email: "kund", ..customer_form("K") }), DomainError::InvalidEmail);
    assert_eq!(bad(CustomerForm { payment_terms: 366, ..customer_form("K") }), DomainError::InvalidPaymentTerms);

    let supplier = |bankgiro, plusgiro, iban, bic| {
        SupplierDetails::parse(&SupplierForm {
            name: "L",
            org_nr: "",
            vat_number: "",
            street: "",
            postal_code: "",
            city: "",
            email: "",
            bankgiro,
            plusgiro,
            iban,
            bic,
        })
        .unwrap_err()
    };
    assert_eq!(supplier("1", "", "", ""), DomainError::InvalidBankgiro);
    assert_eq!(supplier("", "1", "", ""), DomainError::InvalidPlusgiro);
    assert_eq!(supplier("", "", "SE1", ""), DomainError::InvalidIban);
    assert_eq!(supplier("", "", "", "X"), DomainError::InvalidBic);
}

#[test]
fn numbers_run_1_to_n() {
    let mut register = Register::default();
    for expected in 1..=3 {
        let changes = add(&register, customer("Kund")).unwrap();
        assert_eq!(changes[0].number(), expected);
        changes.into_iter().for_each(|c| register.apply(c));
    }
    assert_eq!(register.parties().map(|p| p.number).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn an_update_replaces_every_detail_and_the_same_details_are_a_no_op() {
    let register = Register::from_changes([Change::Added { number: 1, details: customer("Gamla AB") }]);
    assert_eq!(
        update(&register, 1, customer("Nya AB")).unwrap(),
        [Change::Updated { number: 1, details: customer("Nya AB") }]
    );
    assert_eq!(update(&register, 1, customer("Gamla AB")).unwrap(), []);
}

#[test]
fn deactivating_and_reactivating_are_idempotent() {
    let mut register = Register::from_changes([Change::Added { number: 1, details: customer("K") }]);
    assert_eq!(set_active(&register, 1, true).unwrap(), []);
    let off = set_active(&register, 1, false).unwrap();
    assert_eq!(off, [Change::Deactivated { number: 1 }]);
    off.into_iter().for_each(|c| register.apply(c));
    assert!(!register.get(1).unwrap().active);
    assert_eq!(set_active(&register, 1, false).unwrap(), []);
    assert_eq!(set_active(&register, 1, true).unwrap(), [Change::Reactivated { number: 1 }]);
}

#[test]
fn unknown_numbers_are_not_found_per_register() {
    let customers: Register<CustomerDetails> = Register::default();
    assert_eq!(update(&customers, 7, customer("K")), Err(DomainError::CustomerNotFound));
    assert_eq!(set_active(&customers, 7, false), Err(DomainError::CustomerNotFound));
    let suppliers: Register<SupplierDetails> = Register::default();
    assert_eq!(set_active(&suppliers, 7, false), Err(DomainError::SupplierNotFound));
}

#[test]
fn stored_events_read_as_customer_and_supplier_events() {
    let event = CustomerEvent::from(Change::Added { number: 1, details: customer("K") });
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "CustomerAdded");
    assert_eq!(json["number"], 1);
    assert_eq!(json["details"]["org_nr"], "5560160680");
    let back: Change<CustomerDetails> = serde_json::from_value::<CustomerEvent>(json).unwrap().into();
    assert_eq!(back, Change::Added { number: 1, details: customer("K") });
    let event = SupplierEvent::from(Change::<SupplierDetails>::Deactivated { number: 2 });
    assert_eq!(serde_json::to_value(&event).unwrap()["type"], "SupplierDeactivated");
}
```

Add `serde_json.workspace = true` under `[dev-dependencies]` in `crates/invoicing/Cargo.toml`, because the last test uses it directly.

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test domain`
Expected: compile errors for `CustomerForm`, `Register` and the rest.

- [ ] **Step 3: Implement (append to `crates/invoicing/src/domain.rs`)**

Extend the import at the top to `use doris_company::domain::{Address, OrgNr, luhn};` and add `use std::collections::BTreeMap;`. Then append:

```rust
/// A customer as typed into the form; blank optional fields become `None`.
/// No `Debug`: the org.nr may be a personnummer.
#[derive(Clone, Copy)]
pub struct CustomerForm<'a> {
    pub name: &'a str,
    pub org_nr: &'a str,
    pub vat_number: &'a str,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub email: &'a str,
    pub payment_terms: u32,
}

#[derive(Clone, Copy)]
pub struct SupplierForm<'a> {
    pub name: &'a str,
    pub org_nr: &'a str,
    pub vat_number: &'a str,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub email: &'a str,
    pub bankgiro: &'a str,
    pub plusgiro: &'a str,
    pub iban: &'a str,
    pub bic: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub payment_terms: PaymentTerms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierDetails {
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
    pub bankgiro: Option<Bankgiro>,
    pub plusgiro: Option<Plusgiro>,
    pub iban: Option<Iban>,
    pub bic: Option<Bic>,
}

fn optional<T>(
    raw: &str,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<Option<T>, DomainError> {
    if raw.trim().is_empty() { Ok(None) } else { parse(raw).map(Some) }
}

/// Org.nr or personnummer: a private customer has the latter.
fn org_nr(raw: &str) -> Result<Option<OrgNr>, DomainError> {
    optional(raw, |raw| OrgNr::parse(raw).map_err(|_| DomainError::InvalidOrgNr))
}

fn address(street: &str, postal_code: &str, city: &str) -> Result<Address, DomainError> {
    Address::parse(street, postal_code, city).map_err(|_| DomainError::InvalidAddress)
}

impl CustomerDetails {
    pub fn parse(form: &CustomerForm) -> Result<Self, DomainError> {
        Ok(Self {
            name: PartyName::parse(form.name)?,
            org_nr: org_nr(form.org_nr)?,
            vat_number: optional(form.vat_number, VatNumber::parse)?,
            address: address(form.street, form.postal_code, form.city)?,
            email: optional(form.email, Email::parse)?,
            payment_terms: PaymentTerms::new(form.payment_terms)?,
        })
    }
}

impl SupplierDetails {
    pub fn parse(form: &SupplierForm) -> Result<Self, DomainError> {
        Ok(Self {
            name: PartyName::parse(form.name)?,
            org_nr: org_nr(form.org_nr)?,
            vat_number: optional(form.vat_number, VatNumber::parse)?,
            address: address(form.street, form.postal_code, form.city)?,
            email: optional(form.email, Email::parse)?,
            bankgiro: optional(form.bankgiro, Bankgiro::parse)?,
            plusgiro: optional(form.plusgiro, Plusgiro::parse)?,
            iban: optional(form.iban, Iban::parse)?,
            bic: optional(form.bic, Bic::parse)?,
        })
    }
}

/// What the register logic needs to know about one register's details.
pub trait PartyDetails: Clone + PartialEq {
    const NOT_FOUND: DomainError;
}

impl PartyDetails for CustomerDetails {
    const NOT_FOUND: DomainError = DomainError::CustomerNotFound;
}

impl PartyDetails for SupplierDetails {
    const NOT_FOUND: DomainError = DomainError::SupplierNotFound;
}

/// A change to a register. Stored as a [`CustomerEvent`] or a
/// [`SupplierEvent`], so the log names the register.
#[derive(Debug, Clone, PartialEq)]
pub enum Change<D> {
    Added { number: u32, details: D },
    /// The full new set of details, not a diff.
    Updated { number: u32, details: D },
    Deactivated { number: u32 },
    Reactivated { number: u32 },
}

impl<D> Change<D> {
    pub fn number(&self) -> u32 {
        match self {
            Change::Added { number, .. }
            | Change::Updated { number, .. }
            | Change::Deactivated { number }
            | Change::Reactivated { number } => *number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustomerEvent {
    CustomerAdded { number: u32, details: CustomerDetails },
    CustomerUpdated { number: u32, details: CustomerDetails },
    CustomerDeactivated { number: u32 },
    CustomerReactivated { number: u32 },
}

impl From<Change<CustomerDetails>> for CustomerEvent {
    fn from(change: Change<CustomerDetails>) -> Self {
        match change {
            Change::Added { number, details } => Self::CustomerAdded { number, details },
            Change::Updated { number, details } => Self::CustomerUpdated { number, details },
            Change::Deactivated { number } => Self::CustomerDeactivated { number },
            Change::Reactivated { number } => Self::CustomerReactivated { number },
        }
    }
}

impl From<CustomerEvent> for Change<CustomerDetails> {
    fn from(event: CustomerEvent) -> Self {
        match event {
            CustomerEvent::CustomerAdded { number, details } => Self::Added { number, details },
            CustomerEvent::CustomerUpdated { number, details } => Self::Updated { number, details },
            CustomerEvent::CustomerDeactivated { number } => Self::Deactivated { number },
            CustomerEvent::CustomerReactivated { number } => Self::Reactivated { number },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupplierEvent {
    SupplierAdded { number: u32, details: SupplierDetails },
    SupplierUpdated { number: u32, details: SupplierDetails },
    SupplierDeactivated { number: u32 },
    SupplierReactivated { number: u32 },
}

impl From<Change<SupplierDetails>> for SupplierEvent {
    fn from(change: Change<SupplierDetails>) -> Self {
        match change {
            Change::Added { number, details } => Self::SupplierAdded { number, details },
            Change::Updated { number, details } => Self::SupplierUpdated { number, details },
            Change::Deactivated { number } => Self::SupplierDeactivated { number },
            Change::Reactivated { number } => Self::SupplierReactivated { number },
        }
    }
}

impl From<SupplierEvent> for Change<SupplierDetails> {
    fn from(event: SupplierEvent) -> Self {
        match event {
            SupplierEvent::SupplierAdded { number, details } => Self::Added { number, details },
            SupplierEvent::SupplierUpdated { number, details } => Self::Updated { number, details },
            SupplierEvent::SupplierDeactivated { number } => Self::Deactivated { number },
            SupplierEvent::SupplierReactivated { number } => Self::Reactivated { number },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Party<D> {
    pub number: u32,
    pub details: D,
    pub active: bool,
}

/// One company's customers or suppliers. Never removed, only deactivated.
#[derive(Debug, Clone, PartialEq)]
pub struct Register<D> {
    parties: BTreeMap<u32, Party<D>>,
}

impl<D> Default for Register<D> {
    fn default() -> Self {
        Self { parties: BTreeMap::new() }
    }
}

impl<D> Register<D> {
    pub fn from_changes(changes: impl IntoIterator<Item = Change<D>>) -> Self {
        let mut register = Self::default();
        changes.into_iter().for_each(|c| register.apply(c));
        register
    }

    pub fn apply(&mut self, change: Change<D>) {
        match change {
            Change::Added { number, details } => {
                self.parties.insert(number, Party { number, details, active: true });
            }
            Change::Updated { number, details } => {
                if let Some(party) = self.parties.get_mut(&number) {
                    party.details = details;
                }
            }
            Change::Deactivated { number } => self.set_active(number, false),
            Change::Reactivated { number } => self.set_active(number, true),
        }
    }

    fn set_active(&mut self, number: u32, active: bool) {
        if let Some(party) = self.parties.get_mut(&number) {
            party.active = active;
        }
    }

    pub fn get(&self, number: u32) -> Option<&Party<D>> {
        self.parties.get(&number)
    }

    /// By number.
    pub fn parties(&self) -> impl Iterator<Item = &Party<D>> {
        self.parties.values()
    }
}

/// The next number: one more than the highest, so 1..=n without gaps.
pub fn add<D: PartyDetails>(register: &Register<D>, details: D) -> Result<Vec<Change<D>>, DomainError> {
    let number = register.parties.keys().next_back().map_or(1, |n| n + 1);
    Ok(vec![Change::Added { number, details }])
}

/// The same details yield no events.
pub fn update<D: PartyDetails>(
    register: &Register<D>,
    number: u32,
    details: D,
) -> Result<Vec<Change<D>>, DomainError> {
    let party = register.get(number).ok_or(D::NOT_FOUND)?;
    if party.details == details {
        return Ok(vec![]);
    }
    Ok(vec![Change::Updated { number, details }])
}

/// Idempotent: a party already in the wanted state yields no events.
pub fn set_active<D: PartyDetails>(
    register: &Register<D>,
    number: u32,
    active: bool,
) -> Result<Vec<Change<D>>, DomainError> {
    let party = register.get(number).ok_or(D::NOT_FOUND)?;
    Ok(match (party.active, active) {
        (true, false) => vec![Change::Deactivated { number }],
        (false, true) => vec![Change::Reactivated { number }],
        _ => vec![],
    })
}
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p doris-invoicing --test domain && cargo clippy -p doris-invoicing --all-targets -- -D warnings`
Expected: PASS. Run `cargo fmt` if clippy or fmt complain about the long test lines.

- [ ] **Step 5: Commit**

```bash
git add crates/invoicing
git commit -m "Decide and evolve the customer and supplier registers"
```

---

### Task 3: Storage — migration, store functions and projections

**Files:**
- Create: `migrations/0009_invoicing.sql`
- Create: `crates/invoicing/src/projections.rs`
- Modify: `crates/invoicing/src/lib.rs`
- Test: `crates/invoicing/tests/store.rs`

**Interfaces:**
- Consumes: everything in `domain` from Tasks 1–2, plus `doris_company::{get_company, get_company_in}` and `doris_eventstore::{begin, load, stream_version, append, read_all, NewEvent, Metadata}`.
- Produces (in `doris_invoicing`):
  ```rust
  pub enum Error { Domain(DomainError), NotFound, Store(doris_eventstore::Error) }
  pub type Customer = Party<CustomerDetails>;
  pub type Supplier = Party<SupplierDetails>;
  pub async fn list_customers(pool: &SqlitePool, company_id: Uuid, actor: Uuid) -> Result<Vec<Customer>>
  pub async fn add_customer(pool: &SqlitePool, company_id: Uuid, actor: Uuid, form: &CustomerForm<'_>) -> Result<u32>
  pub async fn update_customer(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, form: &CustomerForm<'_>) -> Result<()>
  pub async fn set_customer_active(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, active: bool) -> Result<()>
  // list_suppliers, add_supplier, update_supplier and set_supplier_active take SupplierForm and the same arguments
  pub async fn rebuild_projections(pool: &SqlitePool) -> Result<()>
  ```

- [ ] **Step 1: Write the failing store tests in `crates/invoicing/tests/store.rs`**

```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::domain::{CustomerForm, DomainError, SupplierForm};
use doris_invoicing::{
    Error, add_customer, add_supplier, list_customers, list_suppliers, rebuild_projections,
    set_customer_active, set_supplier_active, update_customer, update_supplier,
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

async fn company(pool: &SqlitePool, owner: Uuid, org_nr: &str) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr,
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

fn customer(name: &str) -> CustomerForm<'_> {
    CustomerForm {
        name,
        org_nr: "556016-0680",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "Stockholm",
        email: "ekonomi@kund.se",
        payment_terms: 30,
    }
}

fn supplier(name: &str) -> SupplierForm<'_> {
    SupplierForm {
        name,
        org_nr: "",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: "5050-1055",
        plusgiro: "",
        iban: "",
        bic: "",
    }
}

async fn event_types(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM events WHERE stream_id LIKE 'customers-%' OR stream_id LIKE 'suppliers-%' ORDER BY global_position")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn customers_are_added_updated_and_deactivated() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    assert_eq!(add_customer(&pool, id, anna, &customer("Kund AB")).await.unwrap(), 1);
    assert_eq!(add_customer(&pool, id, anna, &customer("Annan AB")).await.unwrap(), 2);
    update_customer(&pool, id, anna, 1, &CustomerForm { payment_terms: 10, ..customer("Kund i Stockholm AB") })
        .await
        .unwrap();
    set_customer_active(&pool, id, anna, 2, false).await.unwrap();

    let customers = list_customers(&pool, id, anna).await.unwrap();
    assert_eq!(customers.len(), 2);
    assert_eq!(customers[0].details.name.as_str(), "Kund i Stockholm AB");
    assert_eq!(customers[0].details.payment_terms.get(), 10);
    assert_eq!(customers[0].details.email.as_ref().unwrap().as_str(), "ekonomi@kund.se");
    assert!(customers[0].active);
    assert!(!customers[1].active);
    assert_eq!(
        event_types(&pool).await,
        ["CustomerAdded", "CustomerAdded", "CustomerUpdated", "CustomerDeactivated"]
    );
}

#[tokio::test]
async fn suppliers_are_added_updated_and_deactivated() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    assert_eq!(add_supplier(&pool, id, anna, &supplier("Lev AB")).await.unwrap(), 1);
    update_supplier(&pool, id, anna, 1, &SupplierForm { iban: "SE45 5000 0000 0583 9825 7466", bic: "ESSESESS", ..supplier("Lev AB") })
        .await
        .unwrap();
    set_supplier_active(&pool, id, anna, 1, false).await.unwrap();
    set_supplier_active(&pool, id, anna, 1, true).await.unwrap();

    let suppliers = list_suppliers(&pool, id, anna).await.unwrap();
    assert_eq!(suppliers[0].details.bankgiro.as_ref().unwrap().formatted(), "5050-1055");
    assert_eq!(suppliers[0].details.iban.as_ref().unwrap().as_str(), "SE4550000000058398257466");
    assert!(suppliers[0].active);
    assert_eq!(
        event_types(&pool).await,
        ["SupplierAdded", "SupplierUpdated", "SupplierDeactivated", "SupplierReactivated"]
    );
}

#[tokio::test]
async fn invalid_details_and_unknown_numbers_write_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    let err = add_customer(&pool, id, anna, &customer("")).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvalidName)));
    let err = update_supplier(&pool, id, anna, 1, &supplier("L")).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierNotFound)));
    assert!(event_types(&pool).await.is_empty());
}

#[tokio::test]
async fn a_non_member_gets_not_found() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna, "556016-0680").await;

    assert!(matches!(list_customers(&pool, id, bo).await, Err(Error::NotFound)));
    assert!(matches!(add_customer(&pool, id, bo, &customer("K")).await, Err(Error::NotFound)));
    assert!(matches!(list_suppliers(&pool, Uuid::new_v4(), anna).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn numbers_are_per_company() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let a = company(&pool, anna, "556016-0680").await;
    let b = company(&pool, anna, "556036-0793").await;

    add_customer(&pool, a, anna, &customer("A:s kund")).await.unwrap();
    let err = update_customer(&pool, b, anna, 1, &customer("Fel")).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerNotFound)));
    assert_eq!(add_customer(&pool, b, anna, &customer("B:s kund")).await.unwrap(), 1);
    assert_eq!(add_supplier(&pool, a, anna, &supplier("Lev")).await.unwrap(), 1);

    assert_eq!(list_customers(&pool, a, anna).await.unwrap()[0].details.name.as_str(), "A:s kund");
}

#[tokio::test]
async fn the_projections_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;
    add_customer(&pool, id, anna, &customer("K")).await.unwrap();
    update_customer(&pool, id, anna, 1, &customer("K2")).await.unwrap();
    add_supplier(&pool, id, anna, &supplier("L")).await.unwrap();
    set_supplier_active(&pool, id, anna, 1, false).await.unwrap();
    let rows = |sql: &'static str| {
        let pool = pool.clone();
        async move { sqlx::query_scalar::<_, String>(sql).fetch_all(&pool).await.unwrap() }
    };
    let customers = "SELECT company_id || number || details || active FROM customers ORDER BY company_id, number";
    let suppliers = "SELECT company_id || number || details || active FROM suppliers ORDER BY company_id, number";
    let before = (rows(customers).await, rows(suppliers).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!((rows(customers).await, rows(suppliers).await), before);
    assert_eq!(before.0.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_adds_get_numbers_1_to_n() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("parties.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { add_customer(&pool, id, anna, &customer("K")).await.unwrap() })
        })
        .collect();
    let mut numbers = Vec::new();
    for task in tasks {
        numbers.push(task.await.unwrap());
    }
    numbers.sort();

    assert_eq!(numbers, (1..=20).collect::<Vec<u32>>());
    let listed: Vec<u32> = list_customers(&pool, id, anna).await.unwrap().iter().map(|c| c.number).collect();
    assert_eq!(listed, numbers);
}
```

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test store`
Expected: compile errors, because `add_customer` and the rest don't exist.

- [ ] **Step 3: Write the migration `migrations/0009_invoicing.sql`**

```sql
-- Customer and supplier registers: projections of the customers-{company}
-- and suppliers-{company} streams. `details` is the event's details as JSON.
CREATE TABLE customers (
    company_id TEXT    NOT NULL REFERENCES companies(company_id),
    number     INTEGER NOT NULL,
    details    TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);

CREATE TABLE suppliers (
    company_id TEXT    NOT NULL REFERENCES companies(company_id),
    number     INTEGER NOT NULL,
    details    TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);
```

`doris_eventstore::open` runs all migrations in `migrations/`, so nothing else needs registering. Check with `grep -n migrate crates/eventstore/src/lib.rs`.

- [ ] **Step 4: Write `crates/invoicing/src/projections.rs`**

```rust
//! Read models for the registers. Updated in the same transaction as the
//! append; rebuildable from the event log.

use crate::domain::{Change, CustomerEvent, SupplierEvent};
use crate::{CUSTOMERS_STREAM, SUPPLIERS_STREAM};
use doris_eventstore::RecordedEvent;
use serde::Serialize;
use sqlx::SqliteConnection;

/// One projection table's statements. Static SQL, since sqlx only takes
/// static strings.
pub(crate) struct Table {
    insert: &'static str,
    update: &'static str,
    set_active: &'static str,
    pub(crate) list: &'static str,
    clear: &'static str,
}

pub(crate) const CUSTOMERS: Table = Table {
    insert: "INSERT INTO customers (company_id, number, details, active) VALUES (?, ?, ?, 1)",
    update: "UPDATE customers SET details = ? WHERE company_id = ? AND number = ?",
    set_active: "UPDATE customers SET active = ? WHERE company_id = ? AND number = ?",
    list: "SELECT number, details, active FROM customers WHERE company_id = ? ORDER BY number",
    clear: "DELETE FROM customers",
};

pub(crate) const SUPPLIERS: Table = Table {
    insert: "INSERT INTO suppliers (company_id, number, details, active) VALUES (?, ?, ?, 1)",
    update: "UPDATE suppliers SET details = ? WHERE company_id = ? AND number = ?",
    set_active: "UPDATE suppliers SET active = ? WHERE company_id = ? AND number = ?",
    list: "SELECT number, details, active FROM suppliers WHERE company_id = ? ORDER BY number",
    clear: "DELETE FROM suppliers",
};

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(company_id) = event.stream_id.strip_prefix(CUSTOMERS_STREAM) {
        let change: Change<_> = event.decode::<CustomerEvent>()?.into();
        return apply_change(conn, &CUSTOMERS, company_id, change).await;
    }
    if let Some(company_id) = event.stream_id.strip_prefix(SUPPLIERS_STREAM) {
        let change: Change<_> = event.decode::<SupplierEvent>()?.into();
        return apply_change(conn, &SUPPLIERS, company_id, change).await;
    }
    Ok(())
}

async fn apply_change<D: Serialize>(
    conn: &mut SqliteConnection,
    table: &Table,
    company_id: &str,
    change: Change<D>,
) -> crate::Result<()> {
    let query = match change {
        Change::Added { number, details } => sqlx::query(table.insert)
            .bind(company_id)
            .bind(number)
            .bind(serde_json::to_string(&details)?),
        Change::Updated { number, details } => sqlx::query(table.update)
            .bind(serde_json::to_string(&details)?)
            .bind(company_id)
            .bind(number),
        Change::Deactivated { number } => sqlx::query(table.set_active)
            .bind(false)
            .bind(company_id)
            .bind(number),
        Change::Reactivated { number } => sqlx::query(table.set_active)
            .bind(true)
            .bind(company_id)
            .bind(number),
    };
    query.execute(&mut *conn).await?;
    Ok(())
}

pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for table in [&CUSTOMERS, &SUPPLIERS] {
        sqlx::query(table.clear).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 5: Write the store functions in `crates/invoicing/src/lib.rs`**

Every dependency used here is already listed from Task 1. Replace the file:

```rust
//! Customers and suppliers (kunder och leverantörer) of a company,
//! event-sourced into SQLite. Fakturor will join them here.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, load
//! the register, decide, append, project. A number is decided inside that
//! transaction, so concurrent writers never share one.

pub mod domain;
mod projections;

use domain::{
    Change, CustomerDetails, CustomerEvent, CustomerForm, DomainError, PartyDetails, Party,
    Register, SupplierDetails, SupplierEvent, SupplierForm,
};
use doris_eventstore::{Metadata, NewEvent};
use projections::{CUSTOMERS, SUPPLIERS, Table};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;

const CUSTOMERS_STREAM: &str = "customers-";
const SUPPLIERS_STREAM: &str = "suppliers-";
const SCHEMA_VERSION: i64 = 1;

pub type Customer = Party<CustomerDetails>;
pub type Supplier = Party<SupplierDetails>;

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

pub async fn list_customers(pool: &SqlitePool, company_id: Uuid, actor: Uuid) -> Result<Vec<Customer>> {
    list(pool, &CUSTOMERS, company_id, actor).await
}

pub async fn add_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &CustomerForm<'_>,
) -> Result<u32> {
    let details = CustomerDetails::parse(form)?;
    let changes = change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &CustomerForm<'_>,
) -> Result<()> {
    let details = CustomerDetails::parse(form)?;
    change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_customer_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<CustomerDetails, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

pub async fn list_suppliers(pool: &SqlitePool, company_id: Uuid, actor: Uuid) -> Result<Vec<Supplier>> {
    list(pool, &SUPPLIERS, company_id, actor).await
}

pub async fn add_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &SupplierForm<'_>,
) -> Result<u32> {
    let details = SupplierDetails::parse(form)?;
    let changes = change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &SupplierForm<'_>,
) -> Result<()> {
    let details = SupplierDetails::parse(form)?;
    change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_supplier_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<SupplierDetails, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of parties.
async fn list<D: DeserializeOwned>(
    pool: &SqlitePool,
    table: &Table,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Party<D>>> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(table.list)
        .bind(company_id.to_string())
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|(number, details, active)| {
            Ok(Party {
                number: u32::try_from(number).expect("projected numbers are u32"),
                details: serde_json::from_str(&details)?,
                active,
            })
        })
        .collect()
}

/// Loads one register's stream, decides, appends and projects, all in one
/// IMMEDIATE transaction. Returns the changes that were recorded.
async fn change<D, E>(
    pool: &SqlitePool,
    stream_prefix: &str,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Register<D>) -> Result<Vec<Change<D>>, DomainError>,
) -> Result<Vec<Change<D>>>
where
    D: PartyDetails,
    E: Serialize + DeserializeOwned + From<Change<D>> + Into<Change<D>>,
{
    let mut tx = doris_eventstore::begin(pool).await?;
    doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let stream = format!("{stream_prefix}{company_id}");
    let version = doris_eventstore::stream_version(&mut tx, &stream).await?;
    let history = doris_eventstore::load(&mut tx, &stream)
        .await?
        .iter()
        .map(|e| e.decode::<E>().map(Into::into))
        .collect::<Result<Vec<Change<D>>, _>>()?;
    let changes = decide(&Register::from_changes(history))?;
    let events: Vec<E> = changes.iter().cloned().map(E::from).collect();
    append(&mut tx, &stream, version, &events, actor).await?;
    tx.commit().await?;
    Ok(changes)
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
    let metadata = Metadata {
        actor: Some(actor.to_string()),
    };
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
```

`Change<D>` needs `Clone` for `changes.iter().cloned()`. It derives it, and `PartyDetails: Clone` makes the bound hold.

- [ ] **Step 6: Run the tests and check that they pass**

Run: `cargo test -p doris-invoicing && cargo clippy -p doris-invoicing --all-targets -- -D warnings`
Expected: PASS. If `concurrent_adds_get_numbers_1_to_n` fails with `Conflict` or `database is locked`, check that `change` opens its transaction with `doris_eventstore::begin` (IMMEDIATE). Never retry around it.

- [ ] **Step 7: Make sure the ledger stress test still passes**

Run: `cargo test -p doris-ledger --test stress`
Expected: PASS. The ledger's `rebuild_projections` ignores the new streams, because `apply` only matches its own prefixes.

- [ ] **Step 8: Commit**

```bash
git add migrations/0009_invoicing.sql crates/invoicing
git commit -m "Store the customer and supplier registers with projections"
```

---

### Task 4: InvoicingService over gRPC-Web

**Files:**
- Create: `proto/doris/invoicing/v1/invoicing.proto`
- Modify: `crates/proto/build.rs`, `crates/proto/src/lib.rs`
- Create: `crates/server/src/invoicing.rs`
- Modify: `crates/server/src/lib.rs` (module, re-export, `router` parameter)
- Modify: `crates/server/src/main.rs`
- Modify: `crates/server/Cargo.toml`
- Modify: `crates/server/tests/common/mod.rs`
- Test: `crates/server/tests/invoicing.rs`

**Interfaces:**
- Consumes: the `doris_invoicing` functions and types from Task 3, plus `crate::grpc::signed_in_user`.
- Produces:
  - `doris_proto::invoicing::v1` (client always; server with the `server` feature).
  - `doris_server::InvoicingApi::new(pool)`.
  - `router(api, companies, ledger, invoicing, cors_origins, serve_frontend)`, where `invoicing` is new and comes after `ledger`.
  - In the test harness: `TestServer::invoicing() -> Invoicing`.

- [ ] **Step 1: Write the proto file `proto/doris/invoicing/v1/invoicing.proto`**

```proto
syntax = "proto3";

package doris.invoicing.v1;

// Customers and suppliers of a company. Every call needs a session and
// membership of the company; numbers are decided by the server.
service InvoicingService {
  rpc ListCustomers(ListCustomersRequest) returns (ListCustomersResponse);
  rpc AddCustomer(AddCustomerRequest) returns (AddCustomerResponse);
  rpc UpdateCustomer(UpdateCustomerRequest) returns (UpdateCustomerResponse);
  rpc SetCustomerActive(SetCustomerActiveRequest) returns (SetCustomerActiveResponse);
  rpc ListSuppliers(ListSuppliersRequest) returns (ListSuppliersResponse);
  rpc AddSupplier(AddSupplierRequest) returns (AddSupplierResponse);
  rpc UpdateSupplier(UpdateSupplierRequest) returns (UpdateSupplierResponse);
  rpc SetSupplierActive(SetSupplierActiveRequest) returns (SetSupplierActiveResponse);
}

// "" means none for every optional text field. Answers carry the values in
// their formatted form (556016-0680, 5050-1055, SE45 5000 …).
message CustomerDetails {
  string name = 1;
  string org_nr = 2;
  string vat_number = 3;
  string street = 4;
  string postal_code = 5;
  string city = 6;
  string email = 7;
  uint32 payment_terms = 8;
}

message Customer {
  uint32 number = 1;
  CustomerDetails details = 2;
  bool active = 3;
}

message SupplierDetails {
  string name = 1;
  string org_nr = 2;
  string vat_number = 3;
  string street = 4;
  string postal_code = 5;
  string city = 6;
  string email = 7;
  string bankgiro = 8;
  string plusgiro = 9;
  string iban = 10;
  string bic = 11;
}

message Supplier {
  uint32 number = 1;
  SupplierDetails details = 2;
  bool active = 3;
}

message ListCustomersRequest { string company_id = 1; }
message ListCustomersResponse { repeated Customer customers = 1; }
message AddCustomerRequest { string company_id = 1; CustomerDetails details = 2; }
message AddCustomerResponse { uint32 number = 1; }
message UpdateCustomerRequest { string company_id = 1; uint32 number = 2; CustomerDetails details = 3; }
message UpdateCustomerResponse {}
message SetCustomerActiveRequest { string company_id = 1; uint32 number = 2; bool active = 3; }
message SetCustomerActiveResponse {}

message ListSuppliersRequest { string company_id = 1; }
message ListSuppliersResponse { repeated Supplier suppliers = 1; }
message AddSupplierRequest { string company_id = 1; SupplierDetails details = 2; }
message AddSupplierResponse { uint32 number = 1; }
message UpdateSupplierRequest { string company_id = 1; uint32 number = 2; SupplierDetails details = 3; }
message UpdateSupplierResponse {}
message SetSupplierActiveRequest { string company_id = 1; uint32 number = 2; bool active = 3; }
message SetSupplierActiveResponse {}
```

Add `"../../proto/doris/invoicing/v1/invoicing.proto",` to the list in `crates/proto/build.rs`, and append this to `crates/proto/src/lib.rs`:
```rust

pub mod invoicing {
    pub mod v1 {
        tonic::include_proto!("doris.invoicing.v1");
    }
}
```

Run: `cargo build -p doris-proto --features server`
Expected: it builds.

- [ ] **Step 2: Add the test client to `crates/server/tests/common/mod.rs`**

```rust
use doris_proto::invoicing::v1::invoicing_service_client::InvoicingServiceClient;
// next to the other aliases:
pub type Invoicing = InvoicingServiceClient<Transport>;
// in impl TestServer:
    pub fn invoicing(&self) -> Invoicing {
        InvoicingServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }
```
In `launch`, pass `InvoicingApi::new(pool.clone()),` after `LedgerApi::new(pool.clone()),`, and import `InvoicingApi` from `doris_server`.

- [ ] **Step 3: Write the failing server tests in `crates/server/tests/invoicing.rs`**

```rust
mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as pb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

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

fn customer(name: &str) -> pb::CustomerDetails {
    pb::CustomerDetails {
        name: name.into(),
        org_nr: "5560160680".into(),
        city: "Stockholm".into(),
        payment_terms: 30,
        ..Default::default()
    }
}

fn supplier(name: &str) -> pb::SupplierDetails {
    pb::SupplierDetails {
        name: name.into(),
        bankgiro: "50501055".into(),
        iban: "se4550000000058398257466".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_member_keeps_customers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();

    let number = api
        .add_customer(authed(pb::AddCustomerRequest { company_id: id.clone(), details: Some(customer("Kund AB")) }, &anna))
        .await
        .unwrap()
        .into_inner()
        .number;
    assert_eq!(number, 1);
    api.update_customer(authed(
        pb::UpdateCustomerRequest { company_id: id.clone(), number, details: Some(customer("Kund i Sthlm AB")) },
        &anna,
    ))
    .await
    .unwrap();
    api.set_customer_active(authed(pb::SetCustomerActiveRequest { company_id: id.clone(), number, active: false }, &anna))
        .await
        .unwrap();

    let customers = api
        .list_customers(authed(pb::ListCustomersRequest { company_id: id.clone() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .customers;
    assert_eq!(customers.len(), 1);
    assert!(!customers[0].active);
    let details = customers[0].details.clone().unwrap();
    assert_eq!(details.name, "Kund i Sthlm AB");
    assert_eq!(details.org_nr, "556016-0680");
    assert_eq!(details.vat_number, "");
    assert_eq!(details.payment_terms, 30);
}

#[tokio::test]
async fn a_member_keeps_suppliers_and_sees_formatted_numbers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();

    api.add_supplier(authed(pb::AddSupplierRequest { company_id: id.clone(), details: Some(supplier("Lev AB")) }, &anna))
        .await
        .unwrap();
    api.update_supplier(authed(
        pb::UpdateSupplierRequest { company_id: id.clone(), number: 1, details: Some(pb::SupplierDetails { bic: "essesess".into(), ..supplier("Lev AB") }) },
        &anna,
    ))
    .await
    .unwrap();
    api.set_supplier_active(authed(pb::SetSupplierActiveRequest { company_id: id.clone(), number: 1, active: false }, &anna))
        .await
        .unwrap();

    let suppliers = api
        .list_suppliers(authed(pb::ListSuppliersRequest { company_id: id.clone() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .suppliers;
    let details = suppliers[0].details.clone().unwrap();
    assert_eq!(details.bankgiro, "5050-1055");
    assert_eq!(details.iban, "SE45 5000 0000 0583 9825 7466");
    assert_eq!(details.bic, "ESSESESS");
    assert!(!suppliers[0].active);
}

#[tokio::test]
async fn bad_details_and_unknown_numbers_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();
    let add_customer = |details: pb::CustomerDetails| pb::AddCustomerRequest { company_id: id.clone(), details: Some(details) };
    let add_supplier = |details: pb::SupplierDetails| pb::AddSupplierRequest { company_id: id.clone(), details: Some(details) };
    let invalid = |code: &str| (Code::InvalidArgument, code.to_owned());

    for (details, code) in [
        (customer(""), "invalid_name"),
        (pb::CustomerDetails { org_nr: "556016-0681".into(), ..customer("K") }, "invalid_org_nr"),
        (pb::CustomerDetails { vat_number: "SE1".into(), ..customer("K") }, "invalid_vat_number"),
        (pb::CustomerDetails { city: "å".repeat(201), ..customer("K") }, "invalid_address"),
        (pb::CustomerDetails { email: "kund".into(), ..customer("K") }, "invalid_email"),
        (pb::CustomerDetails { payment_terms: 366, ..customer("K") }, "invalid_payment_terms"),
    ] {
        let err = api.add_customer(authed(add_customer(details), &anna)).await.unwrap_err();
        assert_eq!(code_of(err), invalid(code));
    }
    for (details, code) in [
        (pb::SupplierDetails { bankgiro: "1".into(), ..supplier("L") }, "invalid_bankgiro"),
        (pb::SupplierDetails { plusgiro: "1".into(), ..supplier("L") }, "invalid_plusgiro"),
        (pb::SupplierDetails { iban: "SE1".into(), ..supplier("L") }, "invalid_iban"),
        (pb::SupplierDetails { bic: "X".into(), ..supplier("L") }, "invalid_bic"),
    ] {
        let err = api.add_supplier(authed(add_supplier(details), &anna)).await.unwrap_err();
        assert_eq!(code_of(err), invalid(code));
    }

    let err = api
        .update_customer(authed(pb::UpdateCustomerRequest { company_id: id.clone(), number: 9, details: Some(customer("K")) }, &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "customer_not_found".into()));
    let err = api
        .set_supplier_active(authed(pb::SetSupplierActiveRequest { company_id: id.clone(), number: 9, active: false }, &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "supplier_not_found".into()));
}

#[tokio::test]
async fn others_get_company_not_found_and_strangers_not_signed_in() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();

    for company_id in [id.clone(), "not-a-uuid".into()] {
        let err = api
            .list_customers(authed(pb::ListCustomersRequest { company_id: company_id.clone() }, &bo))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
        let err = api
            .add_supplier(authed(pb::AddSupplierRequest { company_id, details: Some(supplier("L")) }, &bo))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    }

    let err = api
        .list_suppliers(pb::ListSuppliersRequest { company_id: id.clone() })
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".into()));
}
```

- [ ] **Step 4: Run them and check that they fail**

Run: `cargo test -p doris-server --test invoicing`
Expected: compile error, because `InvoicingApi` doesn't exist.

- [ ] **Step 5: Write `crates/server/src/invoicing.rs`**

Add `doris-invoicing.workspace = true` to `[dependencies]` in `crates/server/Cargo.toml`.

```rust
//! `doris.invoicing.v1.InvoicingService`: maps gRPC calls onto
//! `doris_invoicing`. Every call needs a session, and a company the caller
//! isn't a member of looks exactly like one that doesn't exist.

use crate::grpc::signed_in_user;
use doris_invoicing::domain::{CustomerForm, DomainError, SupplierForm};
use doris_invoicing::{Customer, Error, Supplier};
use doris_proto::invoicing::v1 as pb;
use doris_proto::invoicing::v1::invoicing_service_server::InvoicingService;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub struct InvoicingApi {
    pool: SqlitePool,
}

impl InvoicingApi {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The signed-in user and the company id they asked about. Membership
    /// is checked by `doris_invoicing` itself.
    async fn caller<T>(&self, request: &Request<T>, company_id: &str) -> Result<(Uuid, Uuid), Status> {
        let user = signed_in_user(&self.pool, request).await?;
        let company: Uuid = company_id.parse().map_err(|_| company_not_found())?;
        Ok((company, user.id))
    }
}

#[tonic::async_trait]
impl InvoicingService for InvoicingApi {
    async fn list_customers(
        &self,
        request: Request<pb::ListCustomersRequest>,
    ) -> Result<Response<pb::ListCustomersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let customers = doris_invoicing::list_customers(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(customer_pb)
            .collect();
        Ok(Response::new(pb::ListCustomersResponse { customers }))
    }

    async fn add_customer(
        &self,
        request: Request<pb::AddCustomerRequest>,
    ) -> Result<Response<pb::AddCustomerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let details = request.into_inner().details.unwrap_or_default();
        let number = doris_invoicing::add_customer(&self.pool, company, user, &customer_form(&details))
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddCustomerResponse { number }))
    }

    async fn update_customer(
        &self,
        request: Request<pb::UpdateCustomerRequest>,
    ) -> Result<Response<pb::UpdateCustomerResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let details = req.details.unwrap_or_default();
        doris_invoicing::update_customer(&self.pool, company, user, req.number, &customer_form(&details))
            .await
            .map_err(status)?;
        Ok(Response::new(pb::UpdateCustomerResponse {}))
    }

    async fn set_customer_active(
        &self,
        request: Request<pb::SetCustomerActiveRequest>,
    ) -> Result<Response<pb::SetCustomerActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::set_customer_active(&self.pool, company, user, req.number, req.active)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetCustomerActiveResponse {}))
    }

    async fn list_suppliers(
        &self,
        request: Request<pb::ListSuppliersRequest>,
    ) -> Result<Response<pb::ListSuppliersResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let suppliers = doris_invoicing::list_suppliers(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(supplier_pb)
            .collect();
        Ok(Response::new(pb::ListSuppliersResponse { suppliers }))
    }

    async fn add_supplier(
        &self,
        request: Request<pb::AddSupplierRequest>,
    ) -> Result<Response<pb::AddSupplierResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let details = request.into_inner().details.unwrap_or_default();
        let number = doris_invoicing::add_supplier(&self.pool, company, user, &supplier_form(&details))
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddSupplierResponse { number }))
    }

    async fn update_supplier(
        &self,
        request: Request<pb::UpdateSupplierRequest>,
    ) -> Result<Response<pb::UpdateSupplierResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let details = req.details.unwrap_or_default();
        doris_invoicing::update_supplier(&self.pool, company, user, req.number, &supplier_form(&details))
            .await
            .map_err(status)?;
        Ok(Response::new(pb::UpdateSupplierResponse {}))
    }

    async fn set_supplier_active(
        &self,
        request: Request<pb::SetSupplierActiveRequest>,
    ) -> Result<Response<pb::SetSupplierActiveResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::set_supplier_active(&self.pool, company, user, req.number, req.active)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::SetSupplierActiveResponse {}))
    }
}

fn customer_form(d: &pb::CustomerDetails) -> CustomerForm<'_> {
    CustomerForm {
        name: &d.name,
        org_nr: &d.org_nr,
        vat_number: &d.vat_number,
        street: &d.street,
        postal_code: &d.postal_code,
        city: &d.city,
        email: &d.email,
        payment_terms: d.payment_terms,
    }
}

fn supplier_form(d: &pb::SupplierDetails) -> SupplierForm<'_> {
    SupplierForm {
        name: &d.name,
        org_nr: &d.org_nr,
        vat_number: &d.vat_number,
        street: &d.street,
        postal_code: &d.postal_code,
        city: &d.city,
        email: &d.email,
        bankgiro: &d.bankgiro,
        plusgiro: &d.plusgiro,
        iban: &d.iban,
        bic: &d.bic,
    }
}

fn customer_pb(c: Customer) -> pb::Customer {
    let d = c.details;
    pb::Customer {
        number: c.number,
        active: c.active,
        details: Some(pb::CustomerDetails {
            name: d.name.as_str().to_owned(),
            org_nr: d.org_nr.map(|o| o.formatted()).unwrap_or_default(),
            vat_number: d.vat_number.map(|v| v.as_str().to_owned()).unwrap_or_default(),
            street: d.address.street.unwrap_or_default(),
            postal_code: d.address.postal_code.unwrap_or_default(),
            city: d.address.city.unwrap_or_default(),
            email: d.email.map(|e| e.as_str().to_owned()).unwrap_or_default(),
            payment_terms: d.payment_terms.get(),
        }),
    }
}

fn supplier_pb(s: Supplier) -> pb::Supplier {
    let d = s.details;
    pb::Supplier {
        number: s.number,
        active: s.active,
        details: Some(pb::SupplierDetails {
            name: d.name.as_str().to_owned(),
            org_nr: d.org_nr.map(|o| o.formatted()).unwrap_or_default(),
            vat_number: d.vat_number.map(|v| v.as_str().to_owned()).unwrap_or_default(),
            street: d.address.street.unwrap_or_default(),
            postal_code: d.address.postal_code.unwrap_or_default(),
            city: d.address.city.unwrap_or_default(),
            email: d.email.map(|e| e.as_str().to_owned()).unwrap_or_default(),
            bankgiro: d.bankgiro.map(|b| b.formatted()).unwrap_or_default(),
            plusgiro: d.plusgiro.map(|p| p.formatted()).unwrap_or_default(),
            iban: d.iban.map(|i| i.formatted()).unwrap_or_default(),
            bic: d.bic.map(|b| b.as_str().to_owned()).unwrap_or_default(),
        }),
    }
}

fn company_not_found() -> Status {
    Status::not_found("company_not_found")
}

fn domain_status(err: DomainError) -> Status {
    use DomainError::*;
    match err {
        InvalidName => Status::invalid_argument("invalid_name"),
        InvalidOrgNr => Status::invalid_argument("invalid_org_nr"),
        InvalidAddress => Status::invalid_argument("invalid_address"),
        InvalidEmail => Status::invalid_argument("invalid_email"),
        InvalidVatNumber => Status::invalid_argument("invalid_vat_number"),
        InvalidPaymentTerms => Status::invalid_argument("invalid_payment_terms"),
        InvalidBankgiro => Status::invalid_argument("invalid_bankgiro"),
        InvalidPlusgiro => Status::invalid_argument("invalid_plusgiro"),
        InvalidIban => Status::invalid_argument("invalid_iban"),
        InvalidBic => Status::invalid_argument("invalid_bic"),
        CustomerNotFound => Status::not_found("customer_not_found"),
        SupplierNotFound => Status::not_found("supplier_not_found"),
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

- [ ] **Step 6: Wire it into the router**

In `crates/server/src/lib.rs`:
- Add `mod invoicing;` next to `mod ledger;`.
- Add `pub use invoicing::InvoicingApi;`.
- Add `use doris_proto::invoicing::v1::invoicing_service_server::InvoicingServiceServer;`.
- Add the parameter `invoicing: InvoicingApi,` after `ledger: LedgerApi,` in `router`.
- Add `.add_service(InvoicingServiceServer::new(invoicing))` after the ledger `.add_service(…)`. It keeps tonic's default 4 MiB limits, and `session_gate` stays ledger-only.

In `crates/server/src/main.rs`, import `InvoicingApi`. Change `LedgerApi::new(pool)` to `LedgerApi::new(pool.clone())`, and add `InvoicingApi::new(pool),` after it.

- [ ] **Step 7: Run the server tests and check that they pass**

Run: `cargo test -p doris-server && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, including the existing `ledger`, `companies` and `grpc` tests.

- [ ] **Step 8: Commit**

```bash
git add proto crates/proto crates/server Cargo.lock
git commit -m "Serve InvoicingService for customers and suppliers over gRPC-Web"
```

---

### Task 5: The Kunder page

**Files:**
- Modify: `crates/web/src/api.rs`
- Modify: `crates/web/src/errors.rs`
- Create: `crates/web/src/pages/customers.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs` (route and nav link)
- Test: `crates/web/src/errors.rs` (unit), `e2e/tests/invoicing.spec.ts`

**Interfaces:**
- Consumes: `doris_proto::invoicing::v1` from Task 4, plus `ui::{Button, Card, ErrorAlert, Field, Table, TABLE_*, Variant}` and `active_company::Companies`.
- Produces:
  - `api::invoicing_api() -> InvoicingApi`, and `pub use doris_proto::invoicing::v1 as ipb;`.
  - `pages::Customers` at `/customers`.

- [ ] **Step 1: Write the failing errors test (add to `mod tests` in `crates/web/src/errors.rs`)**

```rust
    #[test]
    fn invoicing_codes_have_swedish_messages() {
        for code in [
            "invalid_name",
            "invalid_vat_number",
            "invalid_payment_terms",
            "invalid_bankgiro",
            "invalid_plusgiro",
            "invalid_iban",
            "invalid_bic",
            "customer_not_found",
            "supplier_not_found",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("invalid_payment_terms"),
            "Betalningsvillkoret ska vara 0–365 dagar."
        );
    }
```

Run: `cargo test -p doris-web invoicing_codes`
Expected: FAIL.

- [ ] **Step 2: Add the messages to `message` in `crates/web/src/errors.rs`, before the `_` arm**

```rust
        "invalid_name" => "Namnet måste vara 1–200 tecken.",
        "invalid_vat_number" => {
            "Ange ett giltigt momsregistreringsnummer, till exempel SE556016068001."
        }
        "invalid_payment_terms" => "Betalningsvillkoret ska vara 0–365 dagar.",
        "invalid_bankgiro" => "Ange ett giltigt bankgironummer (7–8 siffror).",
        "invalid_plusgiro" => "Ange ett giltigt plusgironummer (2–8 siffror).",
        "invalid_iban" => "Ange ett giltigt IBAN-nummer.",
        "invalid_bic" => "Ange en giltig BIC (8 eller 11 tecken).",
        "customer_not_found" => "Kunden finns inte.",
        "supplier_not_found" => "Leverantören finns inte.",
```

Run: `cargo test -p doris-web invoicing_codes`
Expected: PASS.

- [ ] **Step 3: Write the failing e2e test `e2e/tests/invoicing.spec.ts`**

```ts
import { addCompany, expect, register, test } from "./fixtures";
import type { Locator } from "@playwright/test";

// The Status cell: the row's buttons also say "Aktivera"/"Inaktivera".
const status = (row: Locator) => row.getByRole("cell").nth(4);

test("customers are added, edited and deactivated", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Kunder" }).click();

  await page.getByRole("button", { name: "Ny kund" }).click();
  await expect(page.getByLabel("Betalningsvillkor (dagar)")).toHaveValue("30");
  await page.getByLabel("Namn", { exact: true }).fill("Kund AB");
  await page.getByLabel("Org.nr/personnr").fill("5560360793");
  await page.getByLabel("Ort", { exact: true }).fill("Stockholm");
  await page.getByLabel("Betalningsvillkor (dagar)").fill("trettio");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("alert")).toHaveText("Betalningsvillkoret ska vara 0–365 dagar.");

  await page.getByLabel("Betalningsvillkor (dagar)").fill("10");
  await page.getByRole("button", { name: "Spara" }).click();
  const row = page.getByRole("row", { name: /^1 Kund AB 556036-0793 Stockholm/ });
  await expect(status(row)).toHaveText("Aktiv");

  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Betalningsvillkor (dagar)")).toHaveValue("10");
  await page.getByLabel("Namn", { exact: true }).fill("Kund i Stockholm AB");
  await page.getByRole("button", { name: "Spara" }).click();
  const renamed = page.getByRole("row", { name: /^1 Kund i Stockholm AB/ });
  await expect(renamed).toBeVisible();

  await renamed.getByRole("button", { name: "Inaktivera" }).click();
  await expect(status(renamed)).toHaveText("Inaktiv");
  await renamed.getByRole("button", { name: "Aktivera" }).click();
  await expect(status(renamed)).toHaveText("Aktiv");
});
```

Run: `make e2e` (or `cd e2e && npx playwright test invoicing` after `make web`)
Expected: FAIL, because there is no "Kunder" link.

- [ ] **Step 4: Add the client in `crates/web/src/api.rs`**

```rust
use doris_proto::invoicing::v1::invoicing_service_client::InvoicingServiceClient;
pub use doris_proto::invoicing::v1 as ipb;
pub type InvoicingApi = InvoicingServiceClient<Client>;

pub fn invoicing_api() -> InvoicingApi {
    InvoicingServiceClient::new(client())
}
```

- [ ] **Step 5: Write `crates/web/src/pages/customers.rs`**

```rust
//! The active company's customers: add, edit, (de)activate.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::errors::describe;
use crate::ui::{
    Button, Card, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The form's fields, one signal each.
#[derive(Clone, Copy)]
struct Form {
    name: RwSignal<String>,
    org_nr: RwSignal<String>,
    vat_number: RwSignal<String>,
    street: RwSignal<String>,
    postal_code: RwSignal<String>,
    city: RwSignal<String>,
    email: RwSignal<String>,
    payment_terms: RwSignal<String>,
}

impl Form {
    fn new() -> Self {
        let text = || RwSignal::new(String::new());
        let form = Self {
            name: text(),
            org_nr: text(),
            vat_number: text(),
            street: text(),
            postal_code: text(),
            city: text(),
            email: text(),
            payment_terms: text(),
        };
        form.clear();
        form
    }

    fn fill(&self, d: &ipb::CustomerDetails) {
        self.name.set(d.name.clone());
        self.org_nr.set(d.org_nr.clone());
        self.vat_number.set(d.vat_number.clone());
        self.street.set(d.street.clone());
        self.postal_code.set(d.postal_code.clone());
        self.city.set(d.city.clone());
        self.email.set(d.email.clone());
        self.payment_terms.set(d.payment_terms.to_string());
    }

    /// Empty, with the usual 30 days.
    fn clear(&self) {
        self.fill(&ipb::CustomerDetails {
            payment_terms: 30,
            ..Default::default()
        });
    }

    fn details(&self) -> ipb::CustomerDetails {
        ipb::CustomerDetails {
            name: self.name.get_untracked(),
            org_nr: self.org_nr.get_untracked(),
            vat_number: self.vat_number.get_untracked(),
            street: self.street.get_untracked(),
            postal_code: self.postal_code.get_untracked(),
            city: self.city.get_untracked(),
            email: self.email.get_untracked(),
            // Not a number: out of range, so the server refuses it with its own message.
            payment_terms: self.payment_terms.get_untracked().trim().parse().unwrap_or(u32::MAX),
        }
    }
}

#[component]
pub fn Customers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The customers and the company they were loaded for, set together.
    let customers = RwSignal::new((String::new(), Vec::<ipb::Customer>::new()));
    let form = Form::new();
    // None: closed. Some(None): a new customer. Some(Some(n)): editing customer n.
    let open = RwSignal::new(None::<Option<u32>>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_customers(ipb::ListCustomersRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => customers.set((company_id, response.into_inner().customers)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or form) on screen.
        customers.set((String::new(), Vec::new()));
        error.set(None);
        open.set(None);
        form.clear();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |customer: ipb::Customer| {
        error.set(None);
        form.fill(&customer.details.unwrap_or_default());
        open.set(Some(Some(customer.number)));
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose customers are on screen, not whatever is active now.
        let company_id = customers.with_untracked(|(id, _)| id.clone());
        let details = Some(form.details());
        let editing = open.get_untracked().flatten();
        spawn_local(async move {
            let result = match editing {
                Some(number) => invoicing_api()
                    .update_customer(ipb::UpdateCustomerRequest {
                        company_id,
                        number,
                        details,
                    })
                    .await
                    .map(|_| ()),
                None => invoicing_api()
                    .add_customer(ipb::AddCustomerRequest {
                        company_id,
                        details,
                    })
                    .await
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    open.set(None);
                    form.clear();
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-center justify-between">
                <h1 class="text-sm font-medium">"Kunder"</h1>
                <Button
                    kind="button"
                    on:click=move |_| {
                        error.set(None);
                        form.clear();
                        open.set(Some(None));
                    }
                >
                    "Ny kund"
                </Button>
            </div>
            <ErrorAlert message=error />
            <Show when=move || open.get().is_some()>
                <Card title="Kunduppgifter">
                    <form class="grid grid-cols-2 gap-4" novalidate on:submit=save>
                        <Field label="Namn" id="customer_name" value=form.name />
                        <Field label="Org.nr/personnr" id="customer_org_nr" value=form.org_nr />
                        <Field label="Momsreg.nr" id="customer_vat_number" value=form.vat_number />
                        <Field label="E-post" id="customer_email" value=form.email />
                        <Field label="Gatuadress" id="customer_street" value=form.street />
                        <Field label="Postnummer" id="customer_postal_code" value=form.postal_code />
                        <Field label="Ort" id="customer_city" value=form.city />
                        <Field
                            label="Betalningsvillkor (dagar)"
                            id="customer_payment_terms"
                            value=form.payment_terms
                        />
                        <div class="col-span-2 flex gap-2">
                            <Button disabled=busy>"Spara"</Button>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| open.set(None)>
                                "Avbryt"
                            </Button>
                        </div>
                    </form>
                </Card>
            </Show>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Org.nr"</th>
                        <th class=TABLE_HEADER_CELL>"Ort"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = customers.get();
                            list.into_iter().map(|c| (company_id.clone(), c)).collect::<Vec<_>>()
                        }
                        key=|(company_id, c)| (company_id.clone(), c.number, c.active, format!("{:?}", c.details))
                        let((company_id, customer))
                    >
                        <CustomerRow company_id=company_id customer=customer edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn CustomerRow(
    company_id: String,
    customer: ipb::Customer,
    edit: Callback<ipb::Customer>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let (number, active) = (customer.number, customer.active);
    let details = customer.details.clone().unwrap_or_default();
    let customer = StoredValue::new(customer);

    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ipb::SetCustomerActiveRequest {
                company_id: company_id.get_value(),
                number,
                active: !active,
            };
            match invoicing_api().set_customer_active(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>{details.name}</td>
            <td class=TABLE_CELL>{details.org_nr}</td>
            <td class=TABLE_CELL>{details.city}</td>
            <td class=TABLE_CELL>{if active { "Aktiv" } else { "Inaktiv" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(customer.get_value())>
                    "Redigera"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}
```

- [ ] **Step 6: Register the page, the route and the nav link**

`crates/web/src/pages/mod.rs`: add `mod customers;` and `pub use customers::Customers;` in alphabetical order.

`crates/web/src/app.rs`:
- Next to the other routes, add:
  ```rust
  <Route path=path!("/customers") view=|| view! { <SignedIn><Customers /></SignedIn> } />
  ```
- Import `Customers` with the other pages.
- In the "Bokföring" nav, add this after the "Verifikationer" link:
  ```rust
  <A href="/customers" attr:class="text-muted-foreground hover:text-foreground">"Kunder"</A>
  ```

- [ ] **Step 7: Run all checks**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: PASS, including the new `invoicing.spec.ts` test and all the existing e2e tests.

- [ ] **Step 8: Commit**

```bash
git add crates/web e2e/tests/invoicing.spec.ts
git commit -m "Add the Kunder page"
```

---

### Task 6: The Leverantörer page

**Files:**
- Create: `crates/web/src/pages/suppliers.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`
- Test: `e2e/tests/invoicing.spec.ts`

**Interfaces:**
- Consumes: `api::{invoicing_api, ipb}` from Task 5, plus the same `ui` items.
- Produces: `pages::Suppliers` at `/suppliers`.

- [ ] **Step 1: Write the failing e2e test (append to `e2e/tests/invoicing.spec.ts`)**

```ts
test("suppliers are added with payment details, edited and deactivated", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Leverantörer" }).click();

  await page.getByRole("button", { name: "Ny leverantör" }).click();
  await page.getByLabel("Namn", { exact: true }).fill("Lev AB");
  await page.getByLabel("Bankgiro").fill("5050-1056");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange ett giltigt bankgironummer (7–8 siffror).");

  await page.getByLabel("Bankgiro").fill("50501055");
  await page.getByLabel("IBAN").fill("se45 5000 0000 0583 9825 7466");
  await page.getByLabel("BIC").fill("essesess");
  await page.getByRole("button", { name: "Spara" }).click();
  const row = page.getByRole("row", { name: /^1 Lev AB/ });
  await expect(row).toContainText("5050-1055");

  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("IBAN")).toHaveValue("SE45 5000 0000 0583 9825 7466");
  await expect(page.getByLabel("BIC")).toHaveValue("ESSESESS");
  await page.getByRole("button", { name: "Avbryt" }).click();

  await row.getByRole("button", { name: "Inaktivera" }).click();
  await expect(status(row)).toHaveText("Inaktiv");
});
```

Run: `make e2e`
Expected: FAIL, because there is no "Leverantörer" link.

- [ ] **Step 2: Write `crates/web/src/pages/suppliers.rs`**

The list shows the bankgiro instead of the city, because that is what you look for when paying.

```rust
//! The active company's suppliers: add, edit, (de)activate.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::errors::describe;
use crate::ui::{
    Button, Card, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The form's fields, one signal each.
#[derive(Clone, Copy)]
struct Form {
    name: RwSignal<String>,
    org_nr: RwSignal<String>,
    vat_number: RwSignal<String>,
    street: RwSignal<String>,
    postal_code: RwSignal<String>,
    city: RwSignal<String>,
    email: RwSignal<String>,
    bankgiro: RwSignal<String>,
    plusgiro: RwSignal<String>,
    iban: RwSignal<String>,
    bic: RwSignal<String>,
}

impl Form {
    fn new() -> Self {
        let text = || RwSignal::new(String::new());
        Self {
            name: text(),
            org_nr: text(),
            vat_number: text(),
            street: text(),
            postal_code: text(),
            city: text(),
            email: text(),
            bankgiro: text(),
            plusgiro: text(),
            iban: text(),
            bic: text(),
        }
    }

    fn fill(&self, d: &ipb::SupplierDetails) {
        self.name.set(d.name.clone());
        self.org_nr.set(d.org_nr.clone());
        self.vat_number.set(d.vat_number.clone());
        self.street.set(d.street.clone());
        self.postal_code.set(d.postal_code.clone());
        self.city.set(d.city.clone());
        self.email.set(d.email.clone());
        self.bankgiro.set(d.bankgiro.clone());
        self.plusgiro.set(d.plusgiro.clone());
        self.iban.set(d.iban.clone());
        self.bic.set(d.bic.clone());
    }

    fn clear(&self) {
        self.fill(&ipb::SupplierDetails::default());
    }

    fn details(&self) -> ipb::SupplierDetails {
        ipb::SupplierDetails {
            name: self.name.get_untracked(),
            org_nr: self.org_nr.get_untracked(),
            vat_number: self.vat_number.get_untracked(),
            street: self.street.get_untracked(),
            postal_code: self.postal_code.get_untracked(),
            city: self.city.get_untracked(),
            email: self.email.get_untracked(),
            bankgiro: self.bankgiro.get_untracked(),
            plusgiro: self.plusgiro.get_untracked(),
            iban: self.iban.get_untracked(),
            bic: self.bic.get_untracked(),
        }
    }
}

#[component]
pub fn Suppliers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The suppliers and the company they were loaded for, set together.
    let suppliers = RwSignal::new((String::new(), Vec::<ipb::Supplier>::new()));
    let form = Form::new();
    // None: closed. Some(None): a new supplier. Some(Some(n)): editing supplier n.
    let open = RwSignal::new(None::<Option<u32>>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_suppliers(ipb::ListSuppliersRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => suppliers.set((company_id, response.into_inner().suppliers)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or form) on screen.
        suppliers.set((String::new(), Vec::new()));
        error.set(None);
        open.set(None);
        form.clear();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |supplier: ipb::Supplier| {
        error.set(None);
        form.fill(&supplier.details.unwrap_or_default());
        open.set(Some(Some(supplier.number)));
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose suppliers are on screen, not whatever is active now.
        let company_id = suppliers.with_untracked(|(id, _)| id.clone());
        let details = Some(form.details());
        let editing = open.get_untracked().flatten();
        spawn_local(async move {
            let result = match editing {
                Some(number) => invoicing_api()
                    .update_supplier(ipb::UpdateSupplierRequest {
                        company_id,
                        number,
                        details,
                    })
                    .await
                    .map(|_| ()),
                None => invoicing_api()
                    .add_supplier(ipb::AddSupplierRequest {
                        company_id,
                        details,
                    })
                    .await
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    open.set(None);
                    form.clear();
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-center justify-between">
                <h1 class="text-sm font-medium">"Leverantörer"</h1>
                <Button
                    kind="button"
                    on:click=move |_| {
                        error.set(None);
                        form.clear();
                        open.set(Some(None));
                    }
                >
                    "Ny leverantör"
                </Button>
            </div>
            <ErrorAlert message=error />
            <Show when=move || open.get().is_some()>
                <Card title="Leverantörsuppgifter">
                    <form class="grid grid-cols-2 gap-4" novalidate on:submit=save>
                        <Field label="Namn" id="supplier_name" value=form.name />
                        <Field label="Org.nr" id="supplier_org_nr" value=form.org_nr />
                        <Field label="Momsreg.nr" id="supplier_vat_number" value=form.vat_number />
                        <Field label="E-post" id="supplier_email" value=form.email />
                        <Field label="Gatuadress" id="supplier_street" value=form.street />
                        <Field label="Postnummer" id="supplier_postal_code" value=form.postal_code />
                        <Field label="Ort" id="supplier_city" value=form.city />
                        <Field label="Bankgiro" id="supplier_bankgiro" value=form.bankgiro />
                        <Field label="Plusgiro" id="supplier_plusgiro" value=form.plusgiro />
                        <Field label="IBAN" id="supplier_iban" value=form.iban />
                        <Field label="BIC" id="supplier_bic" value=form.bic />
                        <div class="col-span-2 flex gap-2">
                            <Button disabled=busy>"Spara"</Button>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| open.set(None)>
                                "Avbryt"
                            </Button>
                        </div>
                    </form>
                </Card>
            </Show>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Org.nr"</th>
                        <th class=TABLE_HEADER_CELL>"Bankgiro"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = suppliers.get();
                            list.into_iter().map(|s| (company_id.clone(), s)).collect::<Vec<_>>()
                        }
                        key=|(company_id, s)| (company_id.clone(), s.number, s.active, format!("{:?}", s.details))
                        let((company_id, supplier))
                    >
                        <SupplierRow company_id=company_id supplier=supplier edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn SupplierRow(
    company_id: String,
    supplier: ipb::Supplier,
    edit: Callback<ipb::Supplier>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let (number, active) = (supplier.number, supplier.active);
    let details = supplier.details.clone().unwrap_or_default();
    let supplier = StoredValue::new(supplier);

    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ipb::SetSupplierActiveRequest {
                company_id: company_id.get_value(),
                number,
                active: !active,
            };
            match invoicing_api().set_supplier_active(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>{details.name}</td>
            <td class=TABLE_CELL>{details.org_nr}</td>
            <td class=TABLE_CELL>{details.bankgiro}</td>
            <td class=TABLE_CELL>{if active { "Aktiv" } else { "Inaktiv" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(supplier.get_value())>
                    "Redigera"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}
```

- [ ] **Step 3: Register the page, the route and the nav link**

`crates/web/src/pages/mod.rs`: add `mod suppliers;` and `pub use suppliers::Suppliers;`.

`crates/web/src/app.rs`:
- Add the route:
  ```rust
  <Route path=path!("/suppliers") view=|| view! { <SignedIn><Suppliers /></SignedIn> } />
  ```
- Import `Suppliers`.
- Add the nav link right after "Kunder":
  ```rust
  <A href="/suppliers" attr:class="text-muted-foreground hover:text-foreground">"Leverantörer"</A>
  ```

- [ ] **Step 4: Run all checks**

Run: `cargo test --workspace && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests/invoicing.spec.ts
git commit -m "Add the Leverantörer page"
```

---

### Task 7: Budget check and AGENTS.md

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Check the wasm budget**

Run: `make dist`
Expected: `dist: … bytes gzipped (budget 500000)`, and the target succeeds. If it fails, look at the two pages first: the `Form` structs and `For` keys are the only new code paths in the wasm. Shrink the code; don't raise the budget.

- [ ] **Step 2: Update `AGENTS.md`**

- Under **Layout**, add this line after `crates/company`:
  ```
  crates/invoicing    doris-invoicing: customers and suppliers (fakturor later)
  ```
- Under **API**, in the first bullet's list of contracts, add `proto/doris/invoicing/v1/invoicing.proto`. In the error-code bullet, add: "Invoicing codes are mapped in `crates/server/src/invoicing.rs` (`status`, `domain_status`)."
- Add a new bullet under **API**:
  ```
  - `InvoicingService` keeps a customer and a supplier register per
    company (`customers-{company}` and `suppliers-{company}` streams; the
    `customers`/`suppliers` projections hold the details as JSON).
    Numbers run 1..=n per company and register, decided in the write
    transaction. A party is never removed, only deactivated. Codes:
    `invalid_name`, `invalid_vat_number`, `invalid_payment_terms`,
    `invalid_bankgiro`, `invalid_plusgiro`, `invalid_iban`, `invalid_bic`,
    `customer_not_found` and `supplier_not_found` (plus `invalid_org_nr`,
    `invalid_address` and `invalid_email`). The VAT number's format is
    checked, never looked up in VIES.
  ```

- [ ] **Step 3: Run everything one last time**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Document the customer and supplier registers"
```
