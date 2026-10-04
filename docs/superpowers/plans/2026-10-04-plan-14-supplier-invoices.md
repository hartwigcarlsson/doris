# Plan 14: Supplier Invoices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Members register supplier invoices (leverantörsfakturor) with lines, VAT and underlag. Doris books them for faktureringsmetoden or kontantmetoden, records the payment, and can cancel an invoice or reverse a payment.

**Architecture:**
- Two pure modules in `crates/invoicing`:
  - `vat.rs`: invoice lines and VAT. Customer invoices will share it.
  - `supplier_invoices.rs`: values, the supplier snapshot, events, state, decisions and voucher lines.
- Store functions in `lib.rs` run one IMMEDIATE transaction per write: membership check, load the stream, decide, book through `doris_ledger::{record_voucher_in, correct_voucher_in, link_attachment_in, check_accounts_in}`, append to `supplier-invoices-{company}`, then project.
- `InvoicingService` gets six RPCs. The web app gets a list page, a form, and a warning on Räkenskapsår.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, serde_json, tonic 0.14 gRPC-Web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-leverantorsfakturor-design.md`

## Global Constraints
- **TDD is mandatory.** Each task ends with a commit, and every commit message ends with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01RBPdfcwGSBXA9x2g5jEGxd
  ```
- **Language:** code, identifiers, URLs, proto and event names are English. Only user-visible UI text is Swedish.
- **Amounts:** `i64` öre. A line's net is 1..=10^13 öre, and an invoice has 1–50 lines, so sums never overflow `i64`.
- **VAT:**
  - Computed per rate on that rate's summed net, rounded half up to whole öre: `(net * pct + 50) / 100`.
  - A given VAT amount must be ≥ 0 and within ±100 öre of the computed one.
- **Event sourcing:**
  - The stream is `supplier-invoices-{company_id}`, with `schema_version` 1.
  - Each write is one `doris_eventstore::begin` (IMMEDIATE) transaction.
  - Projections update in the same transaction and are rebuildable from `read_all`.
  - Vouchers are never changed. Corrections go through `correct_voucher_in`.
- **Numbers** run 1..=n per company and are decided in the transaction. A rejected registration uses up no invoice number and no voucher number.
- **Accounting method** comes from `Company::accounting_method`:
  - `Invoice` (faktureringsmetoden) books at registration, against 2440.
  - `Cash` (kontantmetoden) books nothing until the invoice is paid.
- **Underlag** are read only through the company's own voucher or the company's own supplier invoice, never by hash alone.
- **Personal data:** org.nr, email and file names are never logged.
- **No new dependencies.** Workspace crates `doris-ledger` and `jiff` are added to `doris-invoicing`. The wasm budget (500 KB gzipped, `make dist`) still applies.
- **Lint:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings` stay clean.

## Spec amendments made by this plan (commit them with Task 8)
- `SupplierInvoiceRegistered` nests the registered fields as `invoice: Registration` instead of listing them flat.
- The `supplier_invoice_attachments` table is dropped. The invoice's own JSON already lists its attachments, so a file is read by first loading the company's own invoice, finding the hash in it, and then reading `attachment_files`. That keeps the "never by hash alone" rule.
- `invalid_reason` and `voucher_date_in_future` come from invoicing's own `DomainError::InvalidReason` and `InvoiceDateInFuture`, mapped to the same codes.
- An account number outside 1000–8999 on a line gives `invalid_invoice_account`.
- `ListSupplierInvoicesResponse` carries `bool cash_method`, which the Räkenskapsår warning needs.
- A voucher is shown as "Ver N (YYYY-MM-DD)" with a link to Verifikationen (the grundbok has no deep links).

## Review Focus
1. **Cancelling an invoice whose voucher was already corrected by hand in the grundbok:** the user must get `already_corrected` and the invoice stays unpaid, never half-cancelled. Pinned in Task 4 (`a_hand_corrected_registration_cannot_be_cancelled`).
2. **The same file attached twice in one registration:** this must give `duplicate_attachment`, never `internal`. Pinned in Task 4 (`the_same_file_twice_is_refused`).
3. **A very long supplier name with a long invoice number:** the voucher text is cut to 200 characters, so the booking never fails with `invalid_voucher_text`. Pinned in Task 3 (`the_voucher_text_names_the_invoice_and_fits_200_characters`).
4. **Paying an invoice for a company using kontantmetoden after a reversed payment:** the underlag is linked to the new payment voucher too, without `duplicate_attachment`. Pinned in Task 4 (`cash_method_links_the_underlag_to_each_payment`).
5. **Cancelling or reversing after the fiscal year has ended but while it is still open:** the correction is dated the year's last day, not today, which would fall outside the year. Pinned in Task 3 (`a_correction_is_dated_today_or_the_fiscal_years_last_day`) and Task 4 (`a_correction_after_the_year_ended_is_dated_its_last_day`).

---

### Task 1: Ledger hooks for invoicing

**Files:**
- Modify: `crates/ledger/src/lib.rs`
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Produces (in `doris_ledger`):
  - `VoucherRef` now also derives `Serialize` and `Deserialize` (JSON: `{"fiscal_year_start":"2026-01-01","number":3}`).
  - `pub async fn store_attachment_in(conn: &mut SqliteConnection, new: NewAttachment) -> Result<Attachment>`
  - `pub async fn link_attachment_in(conn: &mut SqliteConnection, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, number: u32, attachment: Attachment, today: Date) -> Result<()>`
  - `pub async fn check_accounts_in(conn: &mut SqliteConnection, company_id: Uuid, actor: Uuid, accounts: &[AccountNumber]) -> Result<()>`

- [ ] **Step 1: Write the failing tests (append to `crates/ledger/tests/store.rs`)**

Add `link_attachment_in`, `store_attachment_in` and `check_accounts_in` to the `use doris_ledger::{…}` list, and add `use doris_ledger::domain::AccountNumber;`.

```rust
#[test]
fn a_voucher_ref_is_stored_as_json() {
    let voucher = VoucherRef {
        fiscal_year_start: d("2026-01-01"),
        number: 3,
    };
    let json = serde_json::to_string(&voucher).unwrap();
    assert_eq!(json, r#"{"fiscal_year_start":"2026-01-01","number":3}"#);
    assert_eq!(serde_json::from_str::<VoucherRef>(&json).unwrap(), voucher);
}

#[tokio::test]
async fn an_underlag_is_stored_first_and_linked_to_a_voucher_later() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let booked = record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY))
        .await
        .unwrap();

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let stored = store_attachment_in(&mut tx, png("faktura.png")).await.unwrap();
    link_attachment_in(
        &mut tx,
        id,
        anna,
        booked.fiscal_year_start,
        booked.number,
        stored.clone(),
        d(TODAY),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let vouchers = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!(vouchers[0].attachments, [stored.clone()]);
    let (_, data) = get_attachment(&pool, id, anna, d("2025-01-01"), 1, &stored.sha256)
        .await
        .unwrap();
    assert_eq!(data, png("faktura.png").data);

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let err = store_attachment_in(&mut tx, NewAttachment { file_name: "a.txt".into(), data: b"text".to_vec() })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::UnsupportedAttachmentType)));
}

#[tokio::test]
async fn accounts_are_checked_against_the_chart() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    set_account_active(&pool, id, anna, 1910, false).await.unwrap();
    let n = |number| AccountNumber::parse(number).unwrap();

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    check_accounts_in(&mut tx, id, anna, &[n(1930), n(5410)]).await.unwrap();
    let inactive = check_accounts_in(&mut tx, id, anna, &[n(1930), n(1910)]).await.unwrap_err();
    assert!(matches!(inactive, Error::Domain(DomainError::AccountInactive)));
    let missing = check_accounts_in(&mut tx, id, anna, &[n(1931)]).await.unwrap_err();
    assert!(matches!(missing, Error::Domain(DomainError::AccountNotFound)));
    let stranger = check_accounts_in(&mut tx, id, Uuid::new_v4(), &[n(1930)]).await.unwrap_err();
    assert!(matches!(stranger, Error::NotFound));
}
```


- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-ledger --test store -- voucher_ref underlag_is_stored accounts_are_checked`
Expected: compile errors for `store_attachment_in`, `link_attachment_in`, `check_accounts_in` and the missing `Serialize` on `VoucherRef`.

- [ ] **Step 3: Implement in `crates/ledger/src/lib.rs`**

Change the import to `use serde::{Deserialize, Serialize};` and the derive on `VoucherRef` to:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoucherRef {
```

Replace `add_attachment_in` and add the three new functions next to it:
```rust
/// [`add_attachment`] in the caller's IMMEDIATE transaction. The file goes
/// in before the event, whose projection refers to it.
async fn add_attachment_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    new: NewAttachment,
    today: Date,
) -> Result<Attachment> {
    let company = member_company(conn, company_id, actor).await?;
    let fiscal_year =
        fiscal_year_at(&company, fiscal_year_start, today).ok_or(DomainError::VoucherNotFound)?;
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let attachment = Attachment::new(&new.file_name, &new.data, sha256_hex(&new.data))?;
    let event = domain::add_attachment(&ledger, number, attachment.clone())?;
    insert_file(conn, &attachment.sha256, &new.data).await?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(conn, &stream, version, &[event], actor).await?;
    Ok(attachment)
}

/// Checks an underlag and stores its bytes in the caller's transaction,
/// linked to no voucher yet (see [`link_attachment_in`]). Used where the
/// voucher comes later, as for a supplier invoice under kontantmetoden.
pub async fn store_attachment_in(
    conn: &mut SqliteConnection,
    new: NewAttachment,
) -> Result<Attachment> {
    let attachment = Attachment::new(&new.file_name, &new.data, sha256_hex(&new.data))?;
    insert_file(conn, &attachment.sha256, &new.data).await?;
    Ok(attachment)
}

/// Links an underlag already stored by [`store_attachment_in`] to voucher
/// `number` of the fiscal year starting on `fiscal_year_start`.
pub async fn link_attachment_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    attachment: Attachment,
    today: Date,
) -> Result<()> {
    let company = member_company(conn, company_id, actor).await?;
    let fiscal_year =
        fiscal_year_at(&company, fiscal_year_start, today).ok_or(DomainError::VoucherNotFound)?;
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let event = domain::add_attachment(&ledger, number, attachment)?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(conn, &stream, version, &[event], actor).await
}

/// Each account must be in the company's chart and active. For callers
/// that need the check without booking a voucher.
pub async fn check_accounts_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    accounts: &[AccountNumber],
) -> Result<()> {
    member_company(conn, company_id, actor).await?;
    let chart = seeded_chart(conn, company_id, actor).await?;
    for &number in accounts {
        match chart.get(number) {
            None => return Err(DomainError::AccountNotFound.into()),
            Some(account) if !account.active => return Err(DomainError::AccountInactive.into()),
            Some(_) => {}
        }
    }
    Ok(())
}

/// Bytes are stored once per SHA-256; `attachment_files` is append-only.
async fn insert_file(conn: &mut SqliteConnection, sha256: &str, data: &[u8]) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO attachment_files (sha256, size, data) VALUES (?, ?, ?)")
        .bind(sha256)
        .bind(data.len() as i64)
        .bind(data)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Run the ledger tests and check that they pass**

Run: `cargo test -p doris-ledger && cargo clippy -p doris-ledger --all-targets -- -D warnings`
Expected: PASS, including `stress.rs` and every existing attachment test.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger
git commit -m "Let other crates store and link underlag and check accounts in a transaction"
```

---

### Task 2: Invoice lines and VAT

**Files:**
- Modify: `crates/invoicing/Cargo.toml` (add `doris-ledger.workspace = true` and `jiff.workspace = true` to `[dependencies]`)
- Modify: `crates/invoicing/src/domain.rs` (new `DomainError` variants; make `optional` `pub(crate)`)
- Create: `crates/invoicing/src/vat.rs`
- Modify: `crates/invoicing/src/lib.rs` (`pub mod vat;`)
- Test: `crates/invoicing/tests/vat.rs`

**Interfaces:**
- Produces:
  - `doris_invoicing::vat::{VatRate, InvoiceLine, MAX_LINE_NET, check_lines, net, computed, check}`:
    - `VatRate::parse(u32) -> Result<VatRate, DomainError>`
    - `VatRate::percent(self) -> u32`
    - `InvoiceLine { pub account: AccountNumber, pub net: i64, pub vat_rate: VatRate }`
    - `InvoiceLine::new(account: u32, net: i64, vat_rate: u32) -> Result<InvoiceLine, DomainError>`
    - `check_lines(&[InvoiceLine]) -> Result<(), DomainError>`
    - `net(&[InvoiceLine]) -> i64`
    - `computed(&[InvoiceLine]) -> i64`
    - `check(&[InvoiceLine], Option<i64>) -> Result<i64, DomainError>`
  - New `DomainError` variants: `InvalidInvoiceNumber`, `DuplicateSupplierInvoice`, `InvalidDueDate`, `InvalidReference`, `InvalidInvoiceLines`, `InvalidVatRate`, `InvalidVatAmount`, `InvalidInvoiceAccount`, `InvalidPaymentAccount`, `SupplierInactive`, `SupplierInvoiceNotFound`, `SupplierInvoicePaid`, `SupplierInvoiceNotPaid`, `SupplierInvoiceCancelled`, `InvalidReason`, `InvoiceDateInFuture`.

- [ ] **Step 1: Write the failing tests in `crates/invoicing/tests/vat.rs`**

```rust
use doris_invoicing::domain::DomainError;
use doris_invoicing::vat::{self, InvoiceLine, VatRate};

fn line(account: u32, net: i64, rate: u32) -> InvoiceLine {
    InvoiceLine::new(account, net, rate).unwrap()
}

#[test]
fn vat_rates_are_25_12_6_or_0_percent() {
    for percent in [25, 12, 6, 0] {
        assert_eq!(VatRate::parse(percent).unwrap().percent(), percent);
    }
    assert_eq!(VatRate::parse(20), Err(DomainError::InvalidVatRate));
}

#[test]
fn a_line_needs_a_positive_amount_and_a_cost_account() {
    assert_eq!(line(5410, 1, 25).net, 1);
    for net in [0, -100, vat::MAX_LINE_NET + 1] {
        assert_eq!(
            InvoiceLine::new(5410, net, 25),
            Err(DomainError::InvalidInvoiceLines),
            "{net}"
        );
    }
    for account in [2440, 2640, 2611, 2600, 2699, 999, 9000] {
        assert_eq!(
            InvoiceLine::new(account, 100, 25),
            Err(DomainError::InvalidInvoiceAccount),
            "{account}"
        );
    }
    assert_eq!(InvoiceLine::new(5410, 100, 20), Err(DomainError::InvalidVatRate));
}

#[test]
fn an_invoice_has_1_to_50_lines() {
    assert_eq!(vat::check_lines(&[]), Err(DomainError::InvalidInvoiceLines));
    assert_eq!(vat::check_lines(&vec![line(5410, 1, 25); 50]), Ok(()));
    assert_eq!(
        vat::check_lines(&vec![line(5410, 1, 25); 51]),
        Err(DomainError::InvalidInvoiceLines)
    );
}

#[test]
fn vat_is_rounded_per_rate_not_per_line() {
    // 99 öre at 25 % is 24,75 öre: 25 öre, where per-line rounding gives 24.
    let lines = [line(5410, 33, 25), line(5410, 33, 25), line(5410, 33, 25)];
    assert_eq!(vat::computed(&lines), 25);
    assert_eq!(vat::net(&lines), 99);
    // 12 % of 10 kr, 6 % of 50 öre (3 öre), nothing on 0 %.
    let mixed = [line(5410, 1000, 12), line(4010, 50, 6), line(6110, 700, 0)];
    assert_eq!(vat::computed(&mixed), 123);
}

#[test]
fn a_given_vat_may_differ_by_at_most_one_krona() {
    let lines = [line(5410, 80_000, 25)];
    assert_eq!(vat::check(&lines, None), Ok(20_000));
    assert_eq!(vat::check(&lines, Some(20_100)), Ok(20_100));
    assert_eq!(vat::check(&lines, Some(19_900)), Ok(19_900));
    for bad in [20_101, 19_899] {
        assert_eq!(vat::check(&lines, Some(bad)), Err(DomainError::InvalidVatAmount));
    }
    assert_eq!(
        vat::check(&[line(6110, 100, 0)], Some(-1)),
        Err(DomainError::InvalidVatAmount)
    );
}
```

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test vat`
Expected: compile error, because `doris_invoicing::vat` doesn't exist.

- [ ] **Step 3: Add the error variants**

In `crates/invoicing/src/domain.rs`, add these to `DomainError` after `SupplierNotFound`:
```rust
    #[error("invoice number must be 1-50 characters")]
    InvalidInvoiceNumber,
    #[error("this supplier's invoice is already registered")]
    DuplicateSupplierInvoice,
    #[error("due date is before the invoice date")]
    InvalidDueDate,
    #[error("payment reference must be at most 50 characters")]
    InvalidReference,
    #[error("an invoice has 1-50 lines, each above 0")]
    InvalidInvoiceLines,
    #[error("VAT rate must be 25, 12, 6 or 0 %")]
    InvalidVatRate,
    #[error("VAT may differ from the computed amount by at most 1 krona")]
    InvalidVatAmount,
    #[error("an invoice line cannot book 2440 or a VAT account")]
    InvalidInvoiceAccount,
    #[error("payment account must be 1900-1999")]
    InvalidPaymentAccount,
    #[error("supplier is inactive")]
    SupplierInactive,
    #[error("no such supplier invoice")]
    SupplierInvoiceNotFound,
    #[error("supplier invoice is paid")]
    SupplierInvoicePaid,
    #[error("supplier invoice is not paid")]
    SupplierInvoiceNotPaid,
    #[error("supplier invoice is cancelled")]
    SupplierInvoiceCancelled,
    #[error("reason must be 1-200 characters")]
    InvalidReason,
    #[error("invoice date is in the future")]
    InvoiceDateInFuture,
```
Change `fn optional<T>(` to `pub(crate) fn optional<T>(`.

In `crates/server/src/invoicing.rs`, add the new arms to `domain_status` now so the workspace keeps compiling:
```rust
        InvalidInvoiceNumber => Status::invalid_argument("invalid_invoice_number"),
        DuplicateSupplierInvoice => Status::already_exists("duplicate_supplier_invoice"),
        InvalidDueDate => Status::invalid_argument("invalid_due_date"),
        InvalidReference => Status::invalid_argument("invalid_reference"),
        InvalidInvoiceLines => Status::invalid_argument("invalid_invoice_lines"),
        InvalidVatRate => Status::invalid_argument("invalid_vat_rate"),
        InvalidVatAmount => Status::invalid_argument("invalid_vat_amount"),
        InvalidInvoiceAccount => Status::invalid_argument("invalid_invoice_account"),
        InvalidPaymentAccount => Status::invalid_argument("invalid_payment_account"),
        SupplierInactive => Status::failed_precondition("supplier_inactive"),
        SupplierInvoiceNotFound => Status::not_found("supplier_invoice_not_found"),
        SupplierInvoicePaid => Status::failed_precondition("supplier_invoice_paid"),
        SupplierInvoiceNotPaid => Status::failed_precondition("supplier_invoice_not_paid"),
        SupplierInvoiceCancelled => Status::failed_precondition("supplier_invoice_cancelled"),
        InvalidReason => Status::invalid_argument("invalid_reason"),
        InvoiceDateInFuture => Status::invalid_argument("voucher_date_in_future"),
```

- [ ] **Step 4: Write `crates/invoicing/src/vat.rs`**

```rust
//! Invoice lines and their VAT (moms). Shared by supplier invoices and,
//! later, customer invoices. Pure.

use crate::domain::DomainError;
use doris_ledger::domain::AccountNumber;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most one line may be: 10^13 öre, as for a voucher line. With at
/// most 50 lines no sum gets near `i64::MAX`.
pub const MAX_LINE_NET: i64 = 10_000_000_000_000;
const MAX_LINES: usize = 50;

/// Swedish VAT rates. Stored as the percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum VatRate {
    Rate25,
    Rate12,
    Rate6,
    Rate0,
}

impl VatRate {
    pub fn parse(percent: u32) -> Result<Self, DomainError> {
        match percent {
            25 => Ok(Self::Rate25),
            12 => Ok(Self::Rate12),
            6 => Ok(Self::Rate6),
            0 => Ok(Self::Rate0),
            _ => Err(DomainError::InvalidVatRate),
        }
    }

    pub fn percent(self) -> u32 {
        match self {
            Self::Rate25 => 25,
            Self::Rate12 => 12,
            Self::Rate6 => 6,
            Self::Rate0 => 0,
        }
    }
}

impl TryFrom<u32> for VatRate {
    type Error = DomainError;
    fn try_from(percent: u32) -> Result<Self, DomainError> {
        Self::parse(percent)
    }
}

impl From<VatRate> for u32 {
    fn from(rate: VatRate) -> u32 {
        rate.percent()
    }
}

/// One cost on an invoice: the account, the amount without VAT, the rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub account: AccountNumber,
    pub net: i64,
    pub vat_rate: VatRate,
}

impl InvoiceLine {
    /// 2440 and the VAT accounts 2600–2699 are booked by Doris itself, and
    /// a number outside 1000–8999 is no account at all.
    pub fn new(account: u32, net: i64, vat_rate: u32) -> Result<Self, DomainError> {
        let account =
            AccountNumber::parse(account).map_err(|_| DomainError::InvalidInvoiceAccount)?;
        if account.get() == 2440 || (2600..=2699).contains(&account.get()) {
            return Err(DomainError::InvalidInvoiceAccount);
        }
        if !(1..=MAX_LINE_NET).contains(&net) {
            return Err(DomainError::InvalidInvoiceLines);
        }
        Ok(Self {
            account,
            net,
            vat_rate: VatRate::parse(vat_rate)?,
        })
    }
}

pub fn check_lines(lines: &[InvoiceLine]) -> Result<(), DomainError> {
    if (1..=MAX_LINES).contains(&lines.len()) {
        Ok(())
    } else {
        Err(DomainError::InvalidInvoiceLines)
    }
}

/// The sum without VAT.
pub fn net(lines: &[InvoiceLine]) -> i64 {
    lines.iter().map(|l| l.net).sum()
}

/// VAT per rate on that rate's summed net, rounded half up to whole öre.
pub fn computed(lines: &[InvoiceLine]) -> i64 {
    let mut by_rate = BTreeMap::<VatRate, i64>::new();
    for line in lines {
        *by_rate.entry(line.vat_rate).or_default() += line.net;
    }
    by_rate
        .into_iter()
        .map(|(rate, net)| (net * i64::from(rate.percent()) + 50) / 100)
        .sum()
}

/// The VAT to book: the computed amount, or the given one if it is ≥ 0 and
/// within 1 krona of it (an invoice may round differently).
pub fn check(lines: &[InvoiceLine], given: Option<i64>) -> Result<i64, DomainError> {
    let computed = computed(lines);
    match given {
        None => Ok(computed),
        Some(vat) if vat >= 0 && (vat - computed).abs() <= 100 => Ok(vat),
        Some(_) => Err(DomainError::InvalidVatAmount),
    }
}
```

Add `pub mod vat;` to `crates/invoicing/src/lib.rs` after `pub mod domain;`.

- [ ] **Step 5: Run the tests and check that they pass**

Run: `cargo test -p doris-invoicing && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/invoicing crates/server/src/invoicing.rs Cargo.lock
git commit -m "Add invoice lines with VAT per rate"
```

---

### Task 3: The supplier invoice domain

**Files:**
- Create: `crates/invoicing/src/supplier_invoices.rs`
- Modify: `crates/invoicing/src/lib.rs` (`pub mod supplier_invoices;`)
- Test: `crates/invoicing/tests/supplier_invoices_domain.rs`

**Interfaces:**
- Consumes: `vat::*` from Task 2; `domain::{DomainError, Party, SupplierDetails, PartyName, Bankgiro, Plusgiro, Iban, Bic, optional}`; `doris_ledger::{VoucherRef, domain::{AccountNumber, Attachment, VoucherLine}}`; `doris_company::domain::{AccountingMethod, OrgNr}`.
- Produces (in `doris_invoicing::supplier_invoices`):
  - `InvoiceNumber::parse(&str)` and `PaymentReference::parse(&str)`, each with `as_str()`
  - `payment_account(u32) -> Result<AccountNumber, DomainError>`
  - `reason(&str) -> Result<String, DomainError>`
  - `SupplierSnapshot { number, name, org_nr, bankgiro, plusgiro, iban, bic }`, with `SupplierSnapshot::of(&Party<SupplierDetails>) -> Result<Self, DomainError>`
  - `NewSupplierInvoice<'a> { invoice_number: &'a str, invoice_date: Date, due_date: Date, reference: &'a str, lines: Vec<InvoiceLine>, vat: Option<i64> }`
  - `Registration { supplier, invoice_number, invoice_date, due_date, reference, lines, vat, total }`, with `Registration::new(SupplierSnapshot, &NewSupplierInvoice) -> Result<Self, DomainError>`
  - `SupplierInvoiceEvent` with the four variants, plus `number()`
  - `Status { Unpaid, Paid { date, account, voucher }, Cancelled }`
  - `SupplierInvoice { number, invoice, attachments, status, vouchers, registration_voucher }`, with `registered(…)`, `apply(&event)` and `status_code()`
  - `SupplierInvoices`, with `from_events`, `apply`, `get` and `next_number`
  - The decisions `register(&SupplierInvoices, &Registration) -> Result<u32, DomainError>`, `unpaid(&SupplierInvoices, u32) -> Result<&SupplierInvoice, DomainError>` and `paid(&SupplierInvoices, u32) -> Result<(&SupplierInvoice, VoucherRef), DomainError>`
  - The voucher helpers `text(u32, &Registration) -> String`, `registration_lines(&Registration) -> Vec<VoucherLine>`, `payment_lines(&Registration, AccountingMethod, AccountNumber) -> Vec<VoucherLine>` and `correction_date(fiscal_year_end: Date, today: Date) -> Date`

- [ ] **Step 1: Write the failing tests in `crates/invoicing/tests/supplier_invoices_domain.rs`**

```rust
use doris_company::domain::AccountingMethod;
use doris_invoicing::domain::{DomainError, Party, PartyName, SupplierDetails, SupplierForm};
use doris_invoicing::supplier_invoices::*;
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(number: u32) -> AccountNumber {
    AccountNumber::parse(number).unwrap()
}

fn debit(account: u32, ore: i64) -> VoucherLine {
    VoucherLine { account: a(account), debit: ore, credit: 0 }
}

fn credit(account: u32, ore: i64) -> VoucherLine {
    VoucherLine { account: a(account), debit: 0, credit: ore }
}

fn line(account: u32, net: i64, rate: u32) -> InvoiceLine {
    InvoiceLine::new(account, net, rate).unwrap()
}

fn supplier(active: bool) -> Party<SupplierDetails> {
    let details = SupplierDetails::parse(&SupplierForm {
        name: "Lev AB",
        org_nr: "556036-0793",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: "5050-1055",
        plusgiro: "",
        iban: "",
        bic: "",
    })
    .unwrap();
    Party { number: 3, details, active }
}

fn new_invoice(lines: Vec<InvoiceLine>) -> NewSupplierInvoice<'static> {
    NewSupplierInvoice {
        invoice_number: "F-4711",
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines,
        vat: None,
    }
}

fn snapshot() -> SupplierSnapshot {
    SupplierSnapshot::of(&supplier(true)).unwrap()
}

fn registration() -> Registration {
    Registration::new(snapshot(), &new_invoice(vec![line(5410, 80_000, 25)])).unwrap()
}

fn voucher(number: u32) -> VoucherRef {
    VoucherRef { fiscal_year_start: d("2026-01-01"), number }
}

fn registered(number: u32, invoice: Registration) -> SupplierInvoiceEvent {
    SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments: vec![],
        voucher: Some(voucher(number)),
    }
}

#[test]
fn invoice_numbers_and_references_are_trimmed_and_bounded() {
    assert_eq!(InvoiceNumber::parse(" F-4711 ").unwrap().as_str(), "F-4711");
    for bad in ["", "  ", &"x".repeat(51)] {
        assert_eq!(InvoiceNumber::parse(bad), Err(DomainError::InvalidInvoiceNumber));
    }
    assert_eq!(PaymentReference::parse(" 12345 ").unwrap().as_str(), "12345");
    assert_eq!(
        PaymentReference::parse(&"1".repeat(51)),
        Err(DomainError::InvalidReference)
    );
}

#[test]
fn payment_accounts_are_19xx() {
    assert_eq!(payment_account(1930).unwrap().get(), 1930);
    for bad in [1899, 2000, 2440, 5] {
        assert_eq!(payment_account(bad), Err(DomainError::InvalidPaymentAccount), "{bad}");
    }
}

#[test]
fn reasons_are_1_to_200_characters() {
    assert_eq!(reason(" Dubbel "), Ok("Dubbel".to_owned()));
    for bad in ["", " ", &"å".repeat(201)] {
        assert_eq!(reason(bad), Err(DomainError::InvalidReason));
    }
}

#[test]
fn a_registration_works_out_vat_and_total_and_copies_the_supplier() {
    let r = registration();
    assert_eq!((r.vat, r.total), (20_000, 100_000));
    assert_eq!(r.supplier.number, 3);
    assert_eq!(r.supplier.name.as_str(), "Lev AB");
    assert_eq!(r.supplier.bankgiro.as_ref().unwrap().formatted(), "5050-1055");
    assert_eq!(r.reference, None);
    let with_vat = Registration::new(
        snapshot(),
        &NewSupplierInvoice { vat: Some(20_050), reference: "OCR 123", ..new_invoice(vec![line(5410, 80_000, 25)]) },
    )
    .unwrap();
    assert_eq!((with_vat.vat, with_vat.total), (20_050, 100_050));
    assert_eq!(with_vat.reference.unwrap().as_str(), "OCR 123");
}

#[test]
fn an_inactive_supplier_cannot_be_invoiced() {
    assert_eq!(SupplierSnapshot::of(&supplier(false)), Err(DomainError::SupplierInactive));
}

#[test]
fn each_bad_registration_field_gives_its_own_error() {
    let bad = |new: NewSupplierInvoice| Registration::new(snapshot(), &new).unwrap_err();
    let ok = || new_invoice(vec![line(5410, 80_000, 25)]);
    assert_eq!(bad(NewSupplierInvoice { invoice_number: "", ..ok() }), DomainError::InvalidInvoiceNumber);
    assert_eq!(bad(NewSupplierInvoice { due_date: d("2026-02-28"), ..ok() }), DomainError::InvalidDueDate);
    let long = "1".repeat(51);
    assert_eq!(bad(NewSupplierInvoice { reference: &long, ..ok() }), DomainError::InvalidReference);
    assert_eq!(bad(new_invoice(vec![])), DomainError::InvalidInvoiceLines);
    assert_eq!(bad(NewSupplierInvoice { vat: Some(25_000), ..ok() }), DomainError::InvalidVatAmount);
    assert!(Registration::new(snapshot(), &NewSupplierInvoice { due_date: d("2026-03-01"), ..ok() }).is_ok());
}

#[test]
fn numbers_run_1_to_n_and_duplicates_are_refused_until_cancelled() {
    let mut state = SupplierInvoices::default();
    assert_eq!(register(&state, &registration()), Ok(1));
    state.apply(registered(1, registration()));
    assert_eq!(register(&state, &registration()), Err(DomainError::DuplicateSupplierInvoice));
    let mut other_supplier = registration();
    other_supplier.supplier.number = 4;
    assert_eq!(register(&state, &other_supplier), Ok(2));
    state.apply(SupplierInvoiceEvent::SupplierInvoiceCancelled {
        number: 1,
        reason: "Dubbel".into(),
        voucher: Some(voucher(2)),
    });
    assert_eq!(register(&state, &registration()), Ok(2));
}

#[test]
fn an_invoice_is_paid_once_and_a_reversed_payment_makes_it_unpaid() {
    let mut state = SupplierInvoices::from_events([registered(1, registration())]);
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(paid(&state, 1).unwrap_err(), DomainError::SupplierInvoiceNotPaid);
    state.apply(SupplierInvoiceEvent::SupplierInvoicePaid {
        number: 1,
        date: d("2026-03-20"),
        account: a(1930),
        voucher: voucher(2),
    });
    assert_eq!(unpaid(&state, 1).unwrap_err(), DomainError::SupplierInvoicePaid);
    assert_eq!(paid(&state, 1).unwrap().1, voucher(2));
    assert_eq!(state.get(1).unwrap().status_code(), "paid");
    state.apply(SupplierInvoiceEvent::SupplierInvoicePaymentReversed {
        number: 1,
        reason: "Fel".into(),
        voucher: voucher(3),
    });
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(state.get(1).unwrap().vouchers, [voucher(1), voucher(2), voucher(3)]);
    assert_eq!(state.get(1).unwrap().registration_voucher, Some(voucher(1)));
}

#[test]
fn a_cancelled_invoice_can_be_neither_paid_nor_reversed_and_unknown_ones_are_not_found() {
    let mut state = SupplierInvoices::from_events([registered(1, registration())]);
    state.apply(SupplierInvoiceEvent::SupplierInvoiceCancelled {
        number: 1,
        reason: "Dubbel".into(),
        voucher: None,
    });
    assert_eq!(unpaid(&state, 1).unwrap_err(), DomainError::SupplierInvoiceCancelled);
    assert_eq!(paid(&state, 1).unwrap_err(), DomainError::SupplierInvoiceCancelled);
    assert_eq!(state.get(1).unwrap().status_code(), "cancelled");
    assert_eq!(unpaid(&state, 9).unwrap_err(), DomainError::SupplierInvoiceNotFound);
    assert_eq!(paid(&state, 9).unwrap_err(), DomainError::SupplierInvoiceNotFound);
}

#[test]
fn registration_books_cost_and_vat_against_2440() {
    let r = Registration::new(
        snapshot(),
        &new_invoice(vec![line(5410, 80_000, 25), line(6110, 10_000, 0)]),
    )
    .unwrap();
    assert_eq!(
        registration_lines(&r),
        [debit(5410, 80_000), debit(6110, 10_000), debit(2640, 20_000), credit(2440, 110_000)]
    );
}

#[test]
fn an_invoice_without_vat_has_no_2640_line() {
    let r = Registration::new(snapshot(), &new_invoice(vec![line(6110, 10_000, 0)])).unwrap();
    assert_eq!(registration_lines(&r), [debit(6110, 10_000), credit(2440, 10_000)]);
}

#[test]
fn payment_books_2440_under_faktureringsmetoden_and_the_cost_under_kontantmetoden() {
    let r = registration();
    assert_eq!(
        payment_lines(&r, AccountingMethod::Invoice, a(1930)),
        [debit(2440, 100_000), credit(1930, 100_000)]
    );
    assert_eq!(
        payment_lines(&r, AccountingMethod::Cash, a(1930)),
        [debit(5410, 80_000), debit(2640, 20_000), credit(1930, 100_000)]
    );
}

#[test]
fn the_voucher_text_names_the_invoice_and_fits_200_characters() {
    assert_eq!(text(12, &registration()), "Leverantörsfaktura 12, Lev AB (F-4711)");
    let mut long = registration();
    long.supplier.name = PartyName::parse(&"å".repeat(200)).unwrap();
    assert_eq!(text(12, &long).chars().count(), 200);
}

#[test]
fn a_correction_is_dated_today_or_the_fiscal_years_last_day() {
    assert_eq!(correction_date(d("2026-12-31"), d("2026-05-01")), d("2026-05-01"));
    assert_eq!(correction_date(d("2026-12-31"), d("2027-01-10")), d("2026-12-31"));
}

#[test]
fn stored_events_name_the_supplier_invoice() {
    let event = registered(1, registration());
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "SupplierInvoiceRegistered");
    assert_eq!(json["invoice"]["lines"][0]["vat_rate"], 25);
    assert_eq!(json["voucher"]["number"], 1);
    assert_eq!(serde_json::from_value::<SupplierInvoiceEvent>(json).unwrap(), event);
}
```


- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test supplier_invoices_domain`
Expected: compile error, because `supplier_invoices` doesn't exist.

- [ ] **Step 3: Write `crates/invoicing/src/supplier_invoices.rs`**

```rust
//! Pure rules for supplier invoices (leverantörsfakturor): the values, the
//! events, the state and the vouchers Doris books for them. No I/O.

use crate::domain::{
    Bankgiro, Bic, DomainError, Iban, Party, PartyName, Plusgiro, SupplierDetails, optional,
};
use crate::vat::{self, InvoiceLine};
use doris_company::domain::{AccountingMethod, OrgNr};
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, Attachment, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACCOUNTS_PAYABLE: u32 = 2440;
const INPUT_VAT: u32 = 2640;
const MAX_VOUCHER_TEXT: usize = 200;

fn account(number: u32) -> AccountNumber {
    AccountNumber::parse(number).expect("a BAS account")
}

fn bounded(raw: &str, max: usize) -> Option<String> {
    let text = raw.trim();
    (1..=max)
        .contains(&text.chars().count())
        .then(|| text.to_owned())
}

/// The supplier's own invoice number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvoiceNumber(String);

impl InvoiceNumber {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded(raw, 50)
            .map(Self)
            .ok_or(DomainError::InvalidInvoiceNumber)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// OCR number or message to send with the payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentReference(String);

impl PaymentReference {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded(raw, 50)
            .map(Self)
            .ok_or(DomainError::InvalidReference)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Payments go out from an account in 1900–1999 (kassa och bank).
pub fn payment_account(number: u32) -> Result<AccountNumber, DomainError> {
    match number {
        1900..=1999 => Ok(account(number)),
        _ => Err(DomainError::InvalidPaymentAccount),
    }
}

/// Why an invoice is cancelled or a payment reversed: 1–200 characters.
pub fn reason(raw: &str) -> Result<String, DomainError> {
    bounded(raw, 200).ok_or(DomainError::InvalidReason)
}

/// The supplier as it was when the invoice was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierSnapshot {
    pub number: u32,
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub bankgiro: Option<Bankgiro>,
    pub plusgiro: Option<Plusgiro>,
    pub iban: Option<Iban>,
    pub bic: Option<Bic>,
}

impl SupplierSnapshot {
    pub fn of(supplier: &Party<SupplierDetails>) -> Result<Self, DomainError> {
        if !supplier.active {
            return Err(DomainError::SupplierInactive);
        }
        let d = &supplier.details;
        Ok(Self {
            number: supplier.number,
            name: d.name.clone(),
            org_nr: d.org_nr.clone(),
            bankgiro: d.bankgiro.clone(),
            plusgiro: d.plusgiro.clone(),
            iban: d.iban.clone(),
            bic: d.bic.clone(),
        })
    }
}

/// A supplier invoice as typed in, before it has a number.
pub struct NewSupplierInvoice<'a> {
    pub invoice_number: &'a str,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: &'a str,
    pub lines: Vec<InvoiceLine>,
    /// `None`: the computed VAT.
    pub vat: Option<i64>,
}

/// What a registered invoice says. Never changes after registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub supplier: SupplierSnapshot,
    pub invoice_number: InvoiceNumber,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: Option<PaymentReference>,
    pub lines: Vec<InvoiceLine>,
    pub vat: i64,
    pub total: i64,
}

impl Registration {
    pub fn new(supplier: SupplierSnapshot, new: &NewSupplierInvoice) -> Result<Self, DomainError> {
        let invoice_number = InvoiceNumber::parse(new.invoice_number)?;
        if new.due_date < new.invoice_date {
            return Err(DomainError::InvalidDueDate);
        }
        let reference = optional(new.reference, PaymentReference::parse)?;
        vat::check_lines(&new.lines)?;
        let vat = vat::check(&new.lines, new.vat)?;
        Ok(Self {
            supplier,
            invoice_number,
            invoice_date: new.invoice_date,
            due_date: new.due_date,
            reference,
            total: vat::net(&new.lines) + vat,
            lines: new.lines.clone(),
            vat,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupplierInvoiceEvent {
    SupplierInvoiceRegistered {
        number: u32,
        invoice: Registration,
        attachments: Vec<Attachment>,
        /// `None` under kontantmetoden: nothing is booked until payment.
        voucher: Option<VoucherRef>,
    },
    SupplierInvoicePaid {
        number: u32,
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    SupplierInvoiceCancelled {
        number: u32,
        reason: String,
        /// The correction of the registration voucher, if there was one.
        voucher: Option<VoucherRef>,
    },
    SupplierInvoicePaymentReversed {
        number: u32,
        reason: String,
        /// The correction of the payment voucher.
        voucher: VoucherRef,
    },
}

impl SupplierInvoiceEvent {
    pub fn number(&self) -> u32 {
        match self {
            Self::SupplierInvoiceRegistered { number, .. }
            | Self::SupplierInvoicePaid { number, .. }
            | Self::SupplierInvoiceCancelled { number, .. }
            | Self::SupplierInvoicePaymentReversed { number, .. } => *number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Unpaid,
    Paid {
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    Cancelled,
}

/// A supplier invoice and what has happened to it. Also the projection's
/// `details` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupplierInvoice {
    pub number: u32,
    pub invoice: Registration,
    pub attachments: Vec<Attachment>,
    pub status: Status,
    /// Every voucher booked for it, in order.
    pub vouchers: Vec<VoucherRef>,
    pub registration_voucher: Option<VoucherRef>,
}

impl SupplierInvoice {
    pub fn registered(
        number: u32,
        invoice: Registration,
        attachments: Vec<Attachment>,
        voucher: Option<VoucherRef>,
    ) -> Self {
        Self {
            number,
            invoice,
            attachments,
            status: Status::Unpaid,
            vouchers: voucher.into_iter().collect(),
            registration_voucher: voucher,
        }
    }

    /// Applies a later event: paid, cancelled or payment reversed.
    pub fn apply(&mut self, event: &SupplierInvoiceEvent) {
        match event {
            SupplierInvoiceEvent::SupplierInvoiceRegistered { .. } => {}
            SupplierInvoiceEvent::SupplierInvoicePaid {
                date,
                account,
                voucher,
                ..
            } => {
                self.status = Status::Paid {
                    date: *date,
                    account: *account,
                    voucher: *voucher,
                };
                self.vouchers.push(*voucher);
            }
            SupplierInvoiceEvent::SupplierInvoiceCancelled { voucher, .. } => {
                self.status = Status::Cancelled;
                self.vouchers.extend(*voucher);
            }
            SupplierInvoiceEvent::SupplierInvoicePaymentReversed { voucher, .. } => {
                self.status = Status::Unpaid;
                self.vouchers.push(*voucher);
            }
        }
    }

    pub fn status_code(&self) -> &'static str {
        match self.status {
            Status::Unpaid => "unpaid",
            Status::Paid { .. } => "paid",
            Status::Cancelled => "cancelled",
        }
    }
}

/// One company's supplier invoices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SupplierInvoices {
    invoices: BTreeMap<u32, SupplierInvoice>,
}

impl SupplierInvoices {
    pub fn from_events(events: impl IntoIterator<Item = SupplierInvoiceEvent>) -> Self {
        let mut state = Self::default();
        events.into_iter().for_each(|e| state.apply(e));
        state
    }

    pub fn apply(&mut self, event: SupplierInvoiceEvent) {
        match event {
            SupplierInvoiceEvent::SupplierInvoiceRegistered {
                number,
                invoice,
                attachments,
                voucher,
            } => {
                self.invoices.insert(
                    number,
                    SupplierInvoice::registered(number, invoice, attachments, voucher),
                );
            }
            other => {
                if let Some(invoice) = self.invoices.get_mut(&other.number()) {
                    invoice.apply(&other);
                }
            }
        }
    }

    pub fn get(&self, number: u32) -> Option<&SupplierInvoice> {
        self.invoices.get(&number)
    }

    pub fn next_number(&self) -> u32 {
        self.invoices.keys().next_back().map_or(1, |n| n + 1)
    }
}

/// The new invoice's number. The same supplier's invoice number may not be
/// registered twice unless the earlier one is cancelled.
pub fn register(state: &SupplierInvoices, invoice: &Registration) -> Result<u32, DomainError> {
    let duplicate = state.invoices.values().any(|existing| {
        existing.status != Status::Cancelled
            && existing.invoice.supplier.number == invoice.supplier.number
            && existing.invoice.invoice_number == invoice.invoice_number
    });
    if duplicate {
        return Err(DomainError::DuplicateSupplierInvoice);
    }
    Ok(state.next_number())
}

/// An unpaid invoice, the only kind that can be paid or cancelled.
pub fn unpaid(state: &SupplierInvoices, number: u32) -> Result<&SupplierInvoice, DomainError> {
    let invoice = state.get(number).ok_or(DomainError::SupplierInvoiceNotFound)?;
    match invoice.status {
        Status::Unpaid => Ok(invoice),
        Status::Paid { .. } => Err(DomainError::SupplierInvoicePaid),
        Status::Cancelled => Err(DomainError::SupplierInvoiceCancelled),
    }
}

/// A paid invoice and its payment voucher, for reversing the payment.
pub fn paid(
    state: &SupplierInvoices,
    number: u32,
) -> Result<(&SupplierInvoice, VoucherRef), DomainError> {
    let invoice = state.get(number).ok_or(DomainError::SupplierInvoiceNotFound)?;
    match invoice.status {
        Status::Paid { voucher, .. } => Ok((invoice, voucher)),
        Status::Unpaid => Err(DomainError::SupplierInvoiceNotPaid),
        Status::Cancelled => Err(DomainError::SupplierInvoiceCancelled),
    }
}

/// "Leverantörsfaktura 12, Lev AB (F-4711)", cut to a voucher text's 200
/// characters.
pub fn text(number: u32, invoice: &Registration) -> String {
    format!(
        "Leverantörsfaktura {number}, {} ({})",
        invoice.supplier.name.as_str(),
        invoice.invoice_number.as_str()
    )
    .chars()
    .take(MAX_VOUCHER_TEXT)
    .collect()
}

/// Each line's net and the VAT, debited: the cost side, the same under
/// both methods.
fn cost_side(invoice: &Registration) -> Vec<VoucherLine> {
    let mut lines: Vec<VoucherLine> = invoice
        .lines
        .iter()
        .map(|l| VoucherLine { account: l.account, debit: l.net, credit: 0 })
        .collect();
    if invoice.vat > 0 {
        lines.push(VoucherLine { account: account(INPUT_VAT), debit: invoice.vat, credit: 0 });
    }
    lines
}

/// Faktureringsmetoden, at registration: the cost against 2440.
pub fn registration_lines(invoice: &Registration) -> Vec<VoucherLine> {
    let mut lines = cost_side(invoice);
    lines.push(VoucherLine {
        account: account(ACCOUNTS_PAYABLE),
        debit: 0,
        credit: invoice.total,
    });
    lines
}

/// At payment: 2440 against the bank under faktureringsmetoden, the whole
/// cost against the bank under kontantmetoden.
pub fn payment_lines(
    invoice: &Registration,
    method: AccountingMethod,
    paid_from: AccountNumber,
) -> Vec<VoucherLine> {
    let mut lines = match method {
        AccountingMethod::Invoice => vec![VoucherLine {
            account: account(ACCOUNTS_PAYABLE),
            debit: invoice.total,
            credit: 0,
        }],
        AccountingMethod::Cash => cost_side(invoice),
    };
    lines.push(VoucherLine { account: paid_from, debit: 0, credit: invoice.total });
    lines
}

/// A correction is dated today, or the last day of the corrected voucher's
/// fiscal year once today is past it (it must stay in that year).
pub fn correction_date(fiscal_year_end: Date, today: Date) -> Date {
    today.min(fiscal_year_end)
}
```

Add `pub mod supplier_invoices;` to `crates/invoicing/src/lib.rs`.

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo fmt -p doris-invoicing && cargo test -p doris-invoicing && cargo clippy -p doris-invoicing --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/invoicing Cargo.lock
git commit -m "Decide supplier invoices and the vouchers they book"
```

---

### Task 4: Storing supplier invoices

**Files:**
- Create: `migrations/0012_supplier_invoices.sql`
- Modify: `crates/invoicing/src/projections.rs`
- Modify: `crates/invoicing/src/lib.rs`
- Test: `crates/invoicing/tests/supplier_invoices.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces (in `doris_invoicing`):
  ```rust
  pub enum Error { Domain(DomainError), NotFound, Ledger(doris_ledger::Error), Store(doris_eventstore::Error) }
  pub async fn register_supplier_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, supplier: u32, new: NewSupplierInvoice<'_>, attachments: Vec<doris_ledger::NewAttachment>, today: Date) -> Result<u32>
  pub async fn pay_supplier_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, date: Date, account: u32, today: Date) -> Result<()>
  pub async fn cancel_supplier_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, reason: &str, today: Date) -> Result<()>
  pub async fn reverse_supplier_invoice_payment(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, reason: &str, today: Date) -> Result<()>
  pub async fn list_supplier_invoices(pool: &SqlitePool, company_id: Uuid, actor: Uuid) -> Result<Vec<SupplierInvoice>>   // newest first
  pub async fn supplier_invoice_attachment(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, sha256: &str) -> Result<(Attachment, Vec<u8>)>
  ```

- [ ] **Step 1: Write the failing store tests in `crates/invoicing/tests/supplier_invoices.rs`**

```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::domain::{DomainError, SupplierForm};
use doris_invoicing::supplier_invoices::{NewSupplierInvoice, Status};
use doris_invoicing::vat::InvoiceLine;
use doris_invoicing::{
    Error, add_supplier, cancel_supplier_invoice, list_supplier_invoices, pay_supplier_invoice,
    rebuild_projections, register_supplier_invoice, reverse_supplier_invoice_payment,
    set_supplier_active, supplier_invoice_attachment,
};
use doris_ledger::NewAttachment;
use doris_ledger::domain::DomainError as LedgerError;
use jiff::civil::Date;
use sqlx::SqlitePool;
use uuid::Uuid;

const TODAY: &str = "2026-10-02";

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

async fn company(pool: &SqlitePool, owner: Uuid, start: &str, method: AccountingMethod) -> Uuid {
    let year = &start[..4];
    let id = doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: start.parse().unwrap(),
            fiscal_year_end: format!("{year}-12-31").parse().unwrap(),
            accounting_method: method,
        },
    )
    .await
    .unwrap();
    add_supplier(
        pool,
        id,
        owner,
        &SupplierForm {
            name: "Lev AB",
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
        },
    )
    .await
    .unwrap();
    id
}

async fn setup(method: AccountingMethod) -> (SqlitePool, Uuid, Uuid) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", method).await;
    (pool, anna, id)
}

fn invoice(number: &str) -> NewSupplierInvoice<'_> {
    NewSupplierInvoice {
        invoice_number: number,
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines: vec![InvoiceLine::new(5410, 80_000, 25).unwrap()],
        vat: None,
    }
}

fn pdf(name: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: format!("%PDF-1.7\n{name}").into_bytes(),
    }
}

async fn register(pool: &SqlitePool, id: Uuid, anna: Uuid, number: &str) -> Result<u32, Error> {
    register_supplier_invoice(pool, id, anna, 1, invoice(number), vec![pdf("faktura.pdf")], d(TODAY)).await
}

async fn pay(pool: &SqlitePool, id: Uuid, anna: Uuid, number: u32) -> Result<(), Error> {
    pay_supplier_invoice(pool, id, anna, number, d("2026-03-20"), 1930, d(TODAY)).await
}

/// Utgående balans, debit − credit, of `account` in 2026.
async fn balance(pool: &SqlitePool, id: Uuid, anna: Uuid, account: u32) -> i64 {
    doris_ledger::trial_balance(pool, id, anna, d("2026-01-01"))
        .await
        .unwrap()
        .iter()
        .find(|r| r.account == account)
        .map_or(0, |r| r.opening + r.debit - r.credit)
}

async fn vouchers(pool: &SqlitePool, id: Uuid, anna: Uuid) -> Vec<doris_ledger::domain::Voucher> {
    doris_ledger::list_vouchers(pool, id, anna, d("2026-01-01")).await.unwrap()
}

fn ledger_error(err: Error) -> LedgerError {
    match err {
        Error::Ledger(doris_ledger::Error::Domain(e)) => e,
        other => panic!("not a ledger domain error: {other:?}"),
    }
}

#[tokio::test]
async fn faktureringsmetoden_books_the_registration_and_the_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;

    assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 1);
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].text, "Leverantörsfaktura 1, Lev AB (F-4711)");
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 2440).await, -100_000);
    assert_eq!(balance(&pool, id, anna, 2640).await, 20_000);

    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
    assert_eq!(balance(&pool, id, anna, 1930).await, -100_000);
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert!(matches!(listed[0].status, Status::Paid { .. }));
    assert_eq!(listed[0].vouchers.len(), 2);
}

#[tokio::test]
async fn kontantmetoden_books_only_the_payment_with_the_underlag() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;

    register(&pool, id, anna, "F-4711").await.unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].registration_voucher, None);

    pay(&pool, id, anna, 1).await.unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].date, d("2026-03-20"));
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 5410).await, 80_000);
    assert_eq!(balance(&pool, id, anna, 2640).await, 20_000);
    assert_eq!(balance(&pool, id, anna, 1930).await, -100_000);
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
}

#[tokio::test]
async fn cash_method_links_the_underlag_to_each_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    reverse_supplier_invoice_payment(&pool, id, anna, 1, "Fel datum", d(TODAY)).await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();

    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 3);
    assert_eq!(booked[2].attachments.len(), 1);
}

#[tokio::test]
async fn cancelling_and_reversing_book_corrections() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();

    cancel_supplier_invoice(&pool, id, anna, 1, "Dubbelregistrerad", d(TODAY)).await.unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked[1].corrects, Some(1));
    assert_eq!(booked[1].date, d(TODAY));
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
    let err = pay(&pool, id, anna, 1).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInvoiceCancelled)));

    assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 2);
    pay(&pool, id, anna, 2).await.unwrap();
    reverse_supplier_invoice_payment(&pool, id, anna, 2, "Fel konto", d(TODAY)).await.unwrap();
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].number, 2);
    assert_eq!(listed[0].status, Status::Unpaid);
    assert_eq!(balance(&pool, id, anna, 1930).await, 0);
    pay(&pool, id, anna, 2).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
}

#[tokio::test]
async fn cancelling_under_kontantmetoden_books_nothing() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    cancel_supplier_invoice(&pool, id, anna, 1, "Fel leverantör", d(TODAY)).await.unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());
    assert_eq!(list_supplier_invoices(&pool, id, anna).await.unwrap()[0].status, Status::Cancelled);
}

#[tokio::test]
async fn wrong_transitions_and_reasons_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    let err = reverse_supplier_invoice_payment(&pool, id, anna, 1, "Fel", d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInvoiceNotPaid)));
    let err = cancel_supplier_invoice(&pool, id, anna, 1, " ", d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvalidReason)));
    pay(&pool, id, anna, 1).await.unwrap();
    let err = cancel_supplier_invoice(&pool, id, anna, 1, "Fel", d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInvoicePaid)));
    let err = pay(&pool, id, anna, 9).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInvoiceNotFound)));
    let err = pay_supplier_invoice(&pool, id, anna, 1, d("2026-03-20"), 2440, d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvalidPaymentAccount)));
}

#[tokio::test]
async fn a_rejected_registration_leaves_no_invoice_and_uses_up_no_number() {
    for method in [AccountingMethod::Invoice, AccountingMethod::Cash] {
        let (pool, anna, id) = setup(method).await;
        doris_ledger::set_account_active(&pool, id, anna, 5410, false).await.unwrap();

        let err = register(&pool, id, anna, "F-4711").await.unwrap_err();
        assert_eq!(ledger_error(err), LedgerError::AccountInactive, "{method:?}");
        assert!(list_supplier_invoices(&pool, id, anna).await.unwrap().is_empty());

        doris_ledger::set_account_active(&pool, id, anna, 5410, true).await.unwrap();
        assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 1);
        if method == AccountingMethod::Invoice {
            assert_eq!(vouchers(&pool, id, anna).await[0].number, 1);
        }
    }
}

#[tokio::test]
async fn the_supplier_must_exist_and_be_active_and_the_date_not_in_the_future() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    let err = register_supplier_invoice(&pool, id, anna, 9, invoice("F-1"), vec![], d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierNotFound)));
    let future = NewSupplierInvoice { invoice_date: d("2026-10-03"), due_date: d("2026-11-03"), ..invoice("F-2") };
    let err = register_supplier_invoice(&pool, id, anna, 1, future, vec![], d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvoiceDateInFuture)));
    set_supplier_active(&pool, id, anna, 1, false).await.unwrap();
    let err = register(&pool, id, anna, "F-3").await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInactive)));
}

#[tokio::test]
async fn the_same_file_twice_is_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    let err = register_supplier_invoice(
        &pool, id, anna, 1, invoice("F-4711"), vec![pdf("a.pdf"), pdf("a.pdf")], d(TODAY),
    )
    .await
    .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::DuplicateAttachment);
}

#[tokio::test]
async fn a_closed_year_takes_no_invoice() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), d(TODAY)).await.unwrap();
    let old = NewSupplierInvoice { invoice_date: d("2025-06-01"), due_date: d("2025-07-01"), ..invoice("F-1") };
    let err = register_supplier_invoice(&pool, id, anna, 1, old, vec![], d(TODAY)).await.unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::FiscalYearClosed);
}

#[tokio::test]
async fn a_correction_after_the_year_ended_is_dated_its_last_day() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    let old = NewSupplierInvoice { invoice_date: d("2025-06-01"), due_date: d("2025-07-01"), ..invoice("F-1") };
    register_supplier_invoice(&pool, id, anna, 1, old, vec![], d(TODAY)).await.unwrap();

    cancel_supplier_invoice(&pool, id, anna, 1, "Dubbel", d(TODAY)).await.unwrap();

    let booked = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    assert_eq!(booked[1].date, d("2025-12-31"));
}

#[tokio::test]
async fn a_hand_corrected_registration_cannot_be_cancelled() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    doris_ledger::correct_voucher(&pool, id, anna, d("2026-01-01"), 1, d(TODAY), d(TODAY)).await.unwrap();

    let err = cancel_supplier_invoice(&pool, id, anna, 1, "Dubbel", d(TODAY)).await.unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AlreadyCorrected);
    assert_eq!(list_supplier_invoices(&pool, id, anna).await.unwrap()[0].status, Status::Unpaid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_registrations_of_the_same_invoice_give_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("invoices.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", AccountingMethod::Invoice).await;

    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { register(&pool, id, anna, "F-4711").await })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(Error::Domain(DomainError::DuplicateSupplierInvoice)) => {}
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(list_supplier_invoices(&pool, id, anna).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_projection_rebuilds_from_the_events() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    register(&pool, id, anna, "F-2").await.unwrap();
    cancel_supplier_invoice(&pool, id, anna, 2, "Fel", d(TODAY)).await.unwrap();
    let sql = "SELECT company_id || number || supplier_number || invoice_number || status || details FROM supplier_invoices ORDER BY number";
    let rows = || async { sqlx::query_scalar::<_, String>(sql).fetch_all(&pool).await.unwrap() };
    let before = rows().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(rows().await, before);
    assert_eq!(before.len(), 2);
}

#[tokio::test]
async fn an_underlag_is_read_only_through_the_companys_own_invoice() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    let sha = list_supplier_invoices(&pool, id, anna).await.unwrap()[0].attachments[0].sha256.clone();

    let (attachment, data) = supplier_invoice_attachment(&pool, id, anna, 1, &sha).await.unwrap();
    assert_eq!(attachment.file_name.as_str(), "faktura.pdf");
    assert_eq!(data, pdf("faktura.pdf").data);

    let err = supplier_invoice_attachment(&pool, id, anna, 2, &sha).await.unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AttachmentNotFound);
    let bo = Uuid::new_v4();
    let err = supplier_invoice_attachment(&pool, id, bo, 1, &sha).await.unwrap_err();
    assert!(matches!(err, Error::NotFound));
}

#[tokio::test]
async fn a_non_member_gets_not_found() {
    let (pool, _anna, id) = setup(AccountingMethod::Invoice).await;
    let bo = Uuid::new_v4();
    assert!(matches!(list_supplier_invoices(&pool, id, bo).await, Err(Error::NotFound)));
    assert!(matches!(register(&pool, id, bo, "F-1").await, Err(Error::NotFound)));
}
```


- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test supplier_invoices`
Expected: compile errors, because `register_supplier_invoice` and the rest don't exist.

- [ ] **Step 3: Write the migration `migrations/0012_supplier_invoices.sql`**

```sql
-- Supplier invoices: the projection of the supplier-invoices-{company}
-- streams. `details` is the invoice as listed (JSON).
CREATE TABLE supplier_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    supplier_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    details         TEXT    NOT NULL,
    PRIMARY KEY (company_id, number)
);

-- One live invoice per supplier and invoice number (dubbelregistrering).
CREATE UNIQUE INDEX supplier_invoices_no_duplicates
    ON supplier_invoices (company_id, supplier_number, invoice_number)
    WHERE status <> 'cancelled';
```

- [ ] **Step 4: Project supplier invoices in `crates/invoicing/src/projections.rs`**

Change the imports to:
```rust
use crate::domain::{Change, CustomerEvent, SupplierEvent};
use crate::supplier_invoices::{SupplierInvoice, SupplierInvoiceEvent};
use crate::{CUSTOMERS_STREAM, SUPPLIER_INVOICES_STREAM, SUPPLIERS_STREAM};
```

In `apply`, add this before the final `Ok(())`:
```rust
    if let Some(company_id) = event.stream_id.strip_prefix(SUPPLIER_INVOICES_STREAM) {
        return apply_supplier_invoice(conn, company_id, event.decode()?).await;
    }
```

Add:
```rust
async fn apply_supplier_invoice(
    conn: &mut SqliteConnection,
    company_id: &str,
    event: SupplierInvoiceEvent,
) -> crate::Result<()> {
    if let SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments,
        voucher,
    } = event
    {
        let invoice = SupplierInvoice::registered(number, invoice, attachments, voucher);
        sqlx::query(
            "INSERT INTO supplier_invoices
             (company_id, number, supplier_number, invoice_number, status, details)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(company_id)
        .bind(number)
        .bind(invoice.invoice.supplier.number)
        .bind(invoice.invoice.invoice_number.as_str())
        .bind(invoice.status_code())
        .bind(serde_json::to_string(&invoice)?)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    let details: String =
        sqlx::query_scalar("SELECT details FROM supplier_invoices WHERE company_id = ? AND number = ?")
            .bind(company_id)
            .bind(event.number())
            .fetch_one(&mut *conn)
            .await?;
    let mut invoice: SupplierInvoice = serde_json::from_str(&details)?;
    invoice.apply(&event);
    sqlx::query(
        "UPDATE supplier_invoices SET status = ?, details = ? WHERE company_id = ? AND number = ?",
    )
    .bind(invoice.status_code())
    .bind(serde_json::to_string(&invoice)?)
    .bind(company_id)
    .bind(event.number())
    .execute(&mut *conn)
    .await?;
    Ok(())
}
```

In `rebuild_projections`, add `sqlx::query("DELETE FROM supplier_invoices").execute(&mut *tx).await?;` after the loop over `[&CUSTOMERS, &SUPPLIERS]`.

- [ ] **Step 5: Write the store functions in `crates/invoicing/src/lib.rs`**

Add `pub mod supplier_invoices;` (if Task 3 hasn't already) and these imports:
```rust
use doris_company::domain::{AccountingMethod, Company};
use doris_ledger::domain::{Attachment, DomainError as LedgerError, RecordVoucher};
use doris_ledger::{NewAttachment, VoucherRef};
use jiff::civil::Date;
use supplier_invoices::{
    NewSupplierInvoice, Registration, SupplierInvoice, SupplierInvoiceEvent, SupplierInvoices,
    SupplierSnapshot,
};
```
Add `const SUPPLIER_INVOICES_STREAM: &str = "supplier-invoices-";`.

Add the `Ledger` variant to `Error`:
```rust
    /// A booking the ledger refused (inactive account, closed year, …).
    #[error(transparent)]
    Ledger(#[from] doris_ledger::Error),
```
and a helper for ledger domain errors raised here:
```rust
fn ledger(err: LedgerError) -> Error {
    Error::Ledger(doris_ledger::Error::Domain(err))
}
```

Refactor stream loading into one helper, and use it in `change`:
```rust
/// A stream's events and its version, in the caller's transaction.
async fn load<E: DeserializeOwned>(conn: &mut SqliteConnection, stream: &str) -> Result<(Vec<E>, i64)> {
    let version = doris_eventstore::stream_version(conn, stream).await?;
    let events = doris_eventstore::load(conn, stream)
        .await?
        .iter()
        .map(|e| e.decode::<E>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((events, version))
}
```
In `change`, replace the `version`/`history` lines with:
```rust
    let (history, version) = load::<E>(&mut tx, &stream).await?;
    let changes = decide(&Register::from_changes(history.into_iter().map(Into::into)))?;
```

Then append:
```rust
fn invoices_stream(company_id: Uuid) -> String {
    format!("{SUPPLIER_INVOICES_STREAM}{company_id}")
}

async fn load_invoices(
    conn: &mut SqliteConnection,
    company_id: Uuid,
) -> Result<(SupplierInvoices, i64)> {
    let (events, version) = load::<SupplierInvoiceEvent>(conn, &invoices_stream(company_id)).await?;
    Ok((SupplierInvoices::from_events(events), version))
}

/// Registers a supplier invoice and, under faktureringsmetoden, books it
/// with its underlag, all in one transaction.
pub async fn register_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    supplier: u32,
    new: NewSupplierInvoice<'_>,
    attachments: Vec<NewAttachment>,
    today: Date,
) -> Result<u32> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (supplier_events, _) =
        load::<SupplierEvent>(&mut tx, &format!("{SUPPLIERS_STREAM}{company_id}")).await?;
    let suppliers = Register::from_changes(supplier_events.into_iter().map(Into::into));
    let supplier = suppliers.get(supplier).ok_or(DomainError::SupplierNotFound)?;
    let invoice = Registration::new(SupplierSnapshot::of(supplier)?, &new)?;
    if invoice.invoice_date > today {
        return Err(DomainError::InvoiceDateInFuture.into());
    }
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let number = supplier_invoices::register(&state, &invoice)?;
    let mut stored: Vec<Attachment> = Vec::new();
    for new in attachments {
        let attachment = doris_ledger::store_attachment_in(&mut tx, new).await?;
        if stored.iter().any(|s| s.sha256 == attachment.sha256) {
            return Err(ledger(LedgerError::DuplicateAttachment));
        }
        stored.push(attachment);
    }
    let voucher = match company.accounting_method {
        AccountingMethod::Invoice => {
            let booked = doris_ledger::record_voucher_in(
                &mut tx,
                company_id,
                actor,
                RecordVoucher {
                    date: invoice.invoice_date,
                    text: supplier_invoices::text(number, &invoice),
                    lines: supplier_invoices::registration_lines(&invoice),
                },
                today,
            )
            .await?;
            link_all(&mut tx, company_id, actor, booked, &stored, today).await?;
            Some(booked)
        }
        AccountingMethod::Cash => {
            let accounts: Vec<_> = invoice.lines.iter().map(|l| l.account).collect();
            doris_ledger::check_accounts_in(&mut tx, company_id, actor, &accounts).await?;
            None
        }
    };
    let event = SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments: stored,
        voucher,
    };
    append(&mut tx, &invoices_stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(number)
}

/// Books the payment of an unpaid invoice. Under kontantmetoden that is
/// the whole cost, and the underlag go on the payment voucher.
pub async fn pay_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    date: Date,
    account: u32,
    today: Date,
) -> Result<()> {
    let account = supplier_invoices::payment_account(account)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = supplier_invoices::unpaid(&state, number)?;
    let lines = supplier_invoices::payment_lines(&invoice.invoice, company.accounting_method, account);
    let voucher = doris_ledger::record_voucher_in(
        &mut tx,
        company_id,
        actor,
        RecordVoucher {
            date,
            text: supplier_invoices::text(number, &invoice.invoice),
            lines,
        },
        today,
    )
    .await?;
    if company.accounting_method == AccountingMethod::Cash {
        link_all(&mut tx, company_id, actor, voucher, &invoice.attachments, today).await?;
    }
    let event = SupplierInvoiceEvent::SupplierInvoicePaid { number, date, account, voucher };
    append(&mut tx, &invoices_stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Cancels an unpaid invoice; under faktureringsmetoden its registration
/// voucher is corrected.
pub async fn cancel_supplier_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason: &str,
    today: Date,
) -> Result<()> {
    let reason = supplier_invoices::reason(reason)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = supplier_invoices::unpaid(&state, number)?;
    let voucher = match invoice.registration_voucher {
        Some(registered) => Some(correct(&mut tx, &company, actor, registered, today).await?),
        None => None,
    };
    let event = SupplierInvoiceEvent::SupplierInvoiceCancelled { number, reason, voucher };
    append(&mut tx, &invoices_stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Corrects the payment voucher of a paid invoice, which is unpaid again.
pub async fn reverse_supplier_invoice_payment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason: &str,
    today: Date,
) -> Result<()> {
    let reason = supplier_invoices::reason(reason)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let (_, payment) = supplier_invoices::paid(&state, number)?;
    let voucher = correct(&mut tx, &company, actor, payment, today).await?;
    let event = SupplierInvoiceEvent::SupplierInvoicePaymentReversed { number, reason, voucher };
    append(&mut tx, &invoices_stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of invoices.
/// The company's supplier invoices, newest first.
pub async fn list_supplier_invoices(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<SupplierInvoice>> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT details FROM supplier_invoices WHERE company_id = ? ORDER BY number DESC",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|details| Ok(serde_json::from_str(details)?))
        .collect()
}

/// An underlag of the company's own invoice `number`: found in that
/// invoice first, never by its hash alone.
pub async fn supplier_invoice_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let details: Option<String> = sqlx::query_scalar(
        "SELECT details FROM supplier_invoices WHERE company_id = ? AND number = ?",
    )
    .bind(company_id.to_string())
    .bind(number)
    .fetch_optional(pool)
    .await?;
    let invoice: Option<SupplierInvoice> = details.map(|d| serde_json::from_str(&d)).transpose()?;
    let attachment = invoice
        .and_then(|i| i.attachments.into_iter().find(|a| a.sha256 == sha256))
        .ok_or_else(|| ledger(LedgerError::AttachmentNotFound))?;
    let data: Vec<u8> = sqlx::query_scalar("SELECT data FROM attachment_files WHERE sha256 = ?")
        .bind(&attachment.sha256)
        .fetch_one(pool)
        .await?;
    Ok((attachment, data))
}

async fn link_all(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    voucher: VoucherRef,
    attachments: &[Attachment],
    today: Date,
) -> Result<()> {
    for attachment in attachments {
        doris_ledger::link_attachment_in(
            conn,
            company_id,
            actor,
            voucher.fiscal_year_start,
            voucher.number,
            attachment.clone(),
            today,
        )
        .await?;
    }
    Ok(())
}

/// Corrects `voucher`, dated today or its fiscal year's last day.
async fn correct(
    conn: &mut SqliteConnection,
    company: &Company,
    actor: Uuid,
    voucher: VoucherRef,
    today: Date,
) -> Result<VoucherRef> {
    let end = company.first_fiscal_year.containing(voucher.fiscal_year_start).end;
    Ok(doris_ledger::correct_voucher_in(
        conn,
        company.id,
        actor,
        voucher.fiscal_year_start,
        voucher.number,
        supplier_invoices::correction_date(end, today),
        today,
    )
    .await?)
}
```


- [ ] **Step 6: Run the tests and check that they pass**

Run: `cargo fmt -p doris-invoicing && cargo test -p doris-invoicing && cargo test -p doris-ledger --test stress && cargo clippy -p doris-invoicing --all-targets -- -D warnings`
Expected: PASS. If the server crate stops compiling because `Error` gained a variant, add `Error::Ledger(err) => crate::ledger::status(err),` to `status` in `crates/server/src/invoicing.rs` and make `fn status` in `crates/server/src/ledger.rs` `pub(crate)`.

- [ ] **Step 7: Commit**

```bash
git add migrations/0012_supplier_invoices.sql crates/invoicing crates/server Cargo.lock
git commit -m "Register, pay, cancel and reverse supplier invoices with their vouchers"
```

---

### Task 5: Supplier invoices over gRPC-Web

**Files:**
- Modify: `proto/doris/invoicing/v1/invoicing.proto`
- Modify: `crates/server/src/invoicing.rs`, `crates/server/src/ledger.rs`, `crates/server/src/lib.rs`
- Test: `crates/server/tests/supplier_invoices.rs`

**Interfaces:**
- Consumes: Task 4's store functions.
- Produces: six RPCs on `InvoicingService`. The request and response messages are as in the spec, plus `ListSupplierInvoicesResponse { repeated SupplierInvoice invoices = 1; bool cash_method = 2; }`. From `crates/server/src/ledger.rs`: `pub(crate) fn status`, `pub(crate) fn date`, `pub(crate) fn new_attachments` and `pub(crate) fn attachment_message`.

- [ ] **Step 1: Extend the proto**

At the top of `proto/doris/invoicing/v1/invoicing.proto`, after `package …;`, add `import "doris/ledger/v1/ledger.proto";`. Add to the service:
```proto
  rpc ListSupplierInvoices(ListSupplierInvoicesRequest) returns (ListSupplierInvoicesResponse);
  rpc RegisterSupplierInvoice(RegisterSupplierInvoiceRequest) returns (RegisterSupplierInvoiceResponse);
  rpc PaySupplierInvoice(PaySupplierInvoiceRequest) returns (PaySupplierInvoiceResponse);
  rpc CancelSupplierInvoice(CancelSupplierInvoiceRequest) returns (CancelSupplierInvoiceResponse);
  rpc ReverseSupplierInvoicePayment(ReverseSupplierInvoicePaymentRequest) returns (ReverseSupplierInvoicePaymentResponse);
  rpc GetSupplierInvoiceAttachment(GetSupplierInvoiceAttachmentRequest) returns (GetSupplierInvoiceAttachmentResponse);
```
and append the messages:
```proto
// An invoice line: account, amount without VAT in öre, VAT rate in percent
// (25, 12, 6 or 0).
message InvoiceLine {
  uint32 account = 1;
  int64 net = 2;
  uint32 vat_rate = 3;
}

message VoucherRef {
  string fiscal_year_start = 1;
  uint32 number = 2;
}

message SupplierInvoice {
  uint32 number = 1;
  uint32 supplier_number = 2;
  string supplier_name = 3;
  string invoice_number = 4;
  string invoice_date = 5;
  string due_date = 6;
  string reference = 7;
  repeated InvoiceLine lines = 8;
  int64 vat = 9;
  int64 total = 10;
  string status = 11;                 // "unpaid", "paid" or "cancelled"
  string paid_date = 12;              // "" unless paid
  repeated VoucherRef vouchers = 13;  // registration, payments, corrections, in order
  repeated doris.ledger.v1.Attachment attachments = 14;
  string bankgiro = 15;               // from the supplier as it was, for paying
  string plusgiro = 16;
  string iban = 17;
}

message ListSupplierInvoicesRequest { string company_id = 1; }
message ListSupplierInvoicesResponse {
  repeated SupplierInvoice invoices = 1;  // newest first
  bool cash_method = 2;                   // the company uses kontantmetoden
}
message RegisterSupplierInvoiceRequest {
  string company_id = 1;
  uint32 supplier_number = 2;
  string invoice_number = 3;
  string invoice_date = 4;
  string due_date = 5;
  string reference = 6;
  repeated InvoiceLine lines = 7;
  optional int64 vat = 8;             // unset: computed
  repeated doris.ledger.v1.NewAttachment attachments = 9;
}
message RegisterSupplierInvoiceResponse { uint32 number = 1; }
message PaySupplierInvoiceRequest { string company_id = 1; uint32 number = 2; string date = 3; uint32 account = 4; }
message PaySupplierInvoiceResponse {}
message CancelSupplierInvoiceRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message CancelSupplierInvoiceResponse {}
message ReverseSupplierInvoicePaymentRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message ReverseSupplierInvoicePaymentResponse {}
message GetSupplierInvoiceAttachmentRequest { string company_id = 1; uint32 number = 2; string sha256 = 3; }
message GetSupplierInvoiceAttachmentResponse { doris.ledger.v1.Attachment attachment = 1; bytes data = 2; }
```

Run: `cargo build -p doris-proto --features server`
Expected: it builds (prost resolves `doris.ledger.v1` because both files compile together). Then `cargo build -p doris-server` fails, because the six trait methods are missing. That is expected.

- [ ] **Step 2: Write the failing server tests in `crates/server/tests/supplier_invoices.rs`**

Also add `pub fn invoicing()`'s decode limit: in `crates/server/tests/common/mod.rs`, change `invoicing()` to
```rust
    pub fn invoicing(&self) -> Invoicing {
        // Room for a 10 MiB underlag coming back from GetSupplierInvoiceAttachment.
        InvoicingServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
            .max_decoding_message_size(11 << 20)
    }
```

```rust
mod common;

use common::{Invoicing, TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as pb;
use doris_proto::ledger::v1 as lpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company (first year 2026) with supplier 1, "Lev AB".
async fn company(server: &TestServer, session: &str, method: cpb::AccountingMethod) -> String {
    let id = server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2026-01-01".into(),
                fiscal_year_end: "2026-12-31".into(),
                accounting_method: method as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id;
    server
        .invoicing()
        .add_supplier(authed(
            pb::AddSupplierRequest {
                company_id: id.clone(),
                details: Some(pb::SupplierDetails {
                    name: "Lev AB".into(),
                    bankgiro: "50501055".into(),
                    ..Default::default()
                }),
            },
            session,
        ))
        .await
        .unwrap();
    id
}

fn request(company_id: &str, invoice_number: &str) -> pb::RegisterSupplierInvoiceRequest {
    pb::RegisterSupplierInvoiceRequest {
        company_id: company_id.into(),
        supplier_number: 1,
        invoice_number: invoice_number.into(),
        invoice_date: "2026-01-15".into(),
        due_date: "2026-02-14".into(),
        reference: "".into(),
        lines: vec![pb::InvoiceLine { account: 5410, net: 80_000, vat_rate: 25 }],
        vat: None,
        attachments: vec![lpb::NewAttachment {
            file_name: "faktura.pdf".into(),
            data: b"%PDF-1.7\nfaktura".to_vec(),
        }],
    }
}

async fn list(api: &mut Invoicing, id: &str, session: &str) -> pb::ListSupplierInvoicesResponse {
    api.list_supplier_invoices(authed(
        pb::ListSupplierInvoicesRequest { company_id: id.into() },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
}

#[tokio::test]
async fn an_invoice_is_registered_paid_reversed_and_listed() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();

    let number = api
        .register_supplier_invoice(authed(request(&id, "F-4711"), &anna))
        .await
        .unwrap()
        .into_inner()
        .number;
    assert_eq!(number, 1);
    api.pay_supplier_invoice(authed(
        pb::PaySupplierInvoiceRequest { company_id: id.clone(), number, date: "2026-01-20".into(), account: 1930 },
        &anna,
    ))
    .await
    .unwrap();

    let listed = list(&mut api, &id, &anna).await;
    assert!(!listed.cash_method);
    let invoice = &listed.invoices[0];
    assert_eq!(invoice.supplier_name, "Lev AB");
    assert_eq!(invoice.bankgiro, "5050-1055");
    assert_eq!((invoice.vat, invoice.total), (20_000, 100_000));
    assert_eq!(invoice.status, "paid");
    assert_eq!(invoice.paid_date, "2026-01-20");
    assert_eq!(invoice.lines[0].vat_rate, 25);
    assert_eq!(invoice.vouchers.len(), 2);
    assert_eq!(invoice.vouchers[0].fiscal_year_start, "2026-01-01");
    assert_eq!(invoice.attachments[0].file_name, "faktura.pdf");

    api.reverse_supplier_invoice_payment(authed(
        pb::ReverseSupplierInvoicePaymentRequest { company_id: id.clone(), number, reason: "Fel konto".into() },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(list(&mut api, &id, &anna).await.invoices[0].status, "unpaid");

    let sha = invoice.attachments[0].id.clone();
    let file = api
        .get_supplier_invoice_attachment(authed(
            pb::GetSupplierInvoiceAttachmentRequest { company_id: id.clone(), number, sha256: sha },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.data, b"%PDF-1.7\nfaktura");
    assert_eq!(file.attachment.unwrap().content_type, "application/pdf");
}

#[tokio::test]
async fn kontantmetoden_is_reported_and_cancelling_works() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Cash).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna)).await.unwrap();

    let listed = list(&mut api, &id, &anna).await;
    assert!(listed.cash_method);
    assert!(listed.invoices[0].vouchers.is_empty());
    api.cancel_supplier_invoice(authed(
        pb::CancelSupplierInvoiceRequest { company_id: id.clone(), number: 1, reason: "Dubbel".into() },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(list(&mut api, &id, &anna).await.invoices[0].status, "cancelled");
}

#[tokio::test]
async fn bad_requests_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    let ok = || request(&id, "F-1");
    let line = |account, net, vat_rate| pb::InvoiceLine { account, net, vat_rate };

    for (req, code, expected) in [
        (pb::RegisterSupplierInvoiceRequest { invoice_number: "".into(), ..ok() }, Code::InvalidArgument, "invalid_invoice_number"),
        (pb::RegisterSupplierInvoiceRequest { due_date: "2026-01-01".into(), ..ok() }, Code::InvalidArgument, "invalid_due_date"),
        (pb::RegisterSupplierInvoiceRequest { reference: "1".repeat(51), ..ok() }, Code::InvalidArgument, "invalid_reference"),
        (pb::RegisterSupplierInvoiceRequest { lines: vec![], ..ok() }, Code::InvalidArgument, "invalid_invoice_lines"),
        (pb::RegisterSupplierInvoiceRequest { lines: vec![line(5410, 100, 20)], ..ok() }, Code::InvalidArgument, "invalid_vat_rate"),
        (pb::RegisterSupplierInvoiceRequest { lines: vec![line(2440, 100, 25)], ..ok() }, Code::InvalidArgument, "invalid_invoice_account"),
        (pb::RegisterSupplierInvoiceRequest { vat: Some(25_000), ..ok() }, Code::InvalidArgument, "invalid_vat_amount"),
        (pb::RegisterSupplierInvoiceRequest { invoice_date: "2026-13-01".into(), ..ok() }, Code::InvalidArgument, "invalid_date"),
        (pb::RegisterSupplierInvoiceRequest { invoice_date: "2099-01-01".into(), due_date: "2099-02-01".into(), ..ok() }, Code::InvalidArgument, "voucher_date_in_future"),
        (pb::RegisterSupplierInvoiceRequest { supplier_number: 9, ..ok() }, Code::NotFound, "supplier_not_found"),
        (pb::RegisterSupplierInvoiceRequest { lines: vec![line(1931, 100, 25)], ..ok() }, Code::NotFound, "account_not_found"),
    ] {
        let err = api.register_supplier_invoice(authed(req, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (code, expected.to_owned()));
    }

    api.register_supplier_invoice(authed(ok(), &anna)).await.unwrap();
    let err = api.register_supplier_invoice(authed(ok(), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::AlreadyExists, "duplicate_supplier_invoice".into()));

    let pay = |number, account| pb::PaySupplierInvoiceRequest { company_id: id.clone(), number, date: "2026-01-20".into(), account };
    let err = api.pay_supplier_invoice(authed(pay(1, 2440), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_payment_account".into()));
    let err = api.pay_supplier_invoice(authed(pay(9, 1930), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "supplier_invoice_not_found".into()));
    let reverse = |reason: &str| pb::ReverseSupplierInvoicePaymentRequest { company_id: id.clone(), number: 1, reason: reason.into() };
    let err = api.reverse_supplier_invoice_payment(authed(reverse("Fel"), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "supplier_invoice_not_paid".into()));
    let cancel = |reason: &str| pb::CancelSupplierInvoiceRequest { company_id: id.clone(), number: 1, reason: reason.into() };
    let err = api.cancel_supplier_invoice(authed(cancel(""), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_reason".into()));
    api.pay_supplier_invoice(authed(pay(1, 1930), &anna)).await.unwrap();
    let err = api.cancel_supplier_invoice(authed(cancel("Fel"), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "supplier_invoice_paid".into()));

    api.set_supplier_active(authed(pb::SetSupplierActiveRequest { company_id: id.clone(), number: 1, active: false }, &anna))
        .await
        .unwrap();
    let err = api.register_supplier_invoice(authed(request(&id, "F-2"), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "supplier_inactive".into()));
}

#[tokio::test]
async fn a_cancelled_invoice_cannot_be_paid() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna)).await.unwrap();
    api.cancel_supplier_invoice(authed(
        pb::CancelSupplierInvoiceRequest { company_id: id.clone(), number: 1, reason: "Dubbel".into() },
        &anna,
    ))
    .await
    .unwrap();
    let err = api
        .pay_supplier_invoice(authed(
            pb::PaySupplierInvoiceRequest { company_id: id.clone(), number: 1, date: "2026-01-20".into(), account: 1930 },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "supplier_invoice_cancelled".into()));
}

#[tokio::test]
async fn others_cannot_see_the_invoices_or_their_underlag() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna)).await.unwrap();
    let sha = list(&mut api, &id, &anna).await.invoices[0].attachments[0].id.clone();

    let err = api
        .list_supplier_invoices(authed(pb::ListSupplierInvoicesRequest { company_id: id.clone() }, &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api
        .get_supplier_invoice_attachment(authed(
            pb::GetSupplierInvoiceAttachmentRequest { company_id: id.clone(), number: 2, sha256: sha },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "attachment_not_found".into()));
}

#[tokio::test]
async fn a_huge_invoicing_frame_without_a_session_is_refused_before_its_body_is_read() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = TestServer::start().await;
    let claimed: u32 = 21 << 20;
    let mut header = vec![0u8];
    header.extend_from_slice(&claimed.to_be_bytes());
    let mut stream = tokio::net::TcpStream::connect(server.base.trim_start_matches("http://"))
        .await
        .unwrap();
    let request = format!(
        "POST /doris.invoicing.v1.InvoicingService/RegisterSupplierInvoice HTTP/1.1\r\n\
         host: localhost\r\n\
         content-type: application/grpc-web+proto\r\n\
         x-grpc-web: 1\r\n\
         content-length: {}\r\n\r\n",
        claimed as usize + header.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(&header).await.unwrap();

    let mut response = Vec::new();
    let mut buf = [0u8; 1024];
    let head = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !response.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "connection closed without an answer");
            response.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&response).to_lowercase()
    })
    .await
    .expect("the server waited for the body instead of answering");
    assert!(head.contains("grpc-message: not_signed_in"), "{head}");
}
```

- [ ] **Step 3: Run them and check that they fail**

Run: `cargo test -p doris-server --test supplier_invoices`
Expected: compile errors, because the `InvoicingService` trait methods are missing.

- [ ] **Step 4: Open up the ledger helpers in `crates/server/src/ledger.rs`**

Change `fn status`, `fn date`, `fn new_attachments` and `fn attachment_message` to `pub(crate) fn`.

- [ ] **Step 5: Implement the RPCs in `crates/server/src/invoicing.rs`**

Extend the imports:
```rust
use crate::grpc::{signed_in_user, today};
use crate::ledger::{attachment_message, date, new_attachments};
use doris_company::domain::AccountingMethod;
use doris_invoicing::supplier_invoices::{NewSupplierInvoice, Status, SupplierInvoice};
use doris_invoicing::vat::InvoiceLine;
```
Add `Error::Ledger(err) => crate::ledger::status(err),` to `status` (if Task 4 hasn't already).

Add to `impl InvoicingService for InvoicingApi`:
```rust
    async fn list_supplier_invoices(
        &self,
        request: Request<pb::ListSupplierInvoicesRequest>,
    ) -> Result<Response<pb::ListSupplierInvoicesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let invoices = doris_invoicing::list_supplier_invoices(&self.pool, company, user)
            .await
            .map_err(status)?
            .into_iter()
            .map(supplier_invoice_pb)
            .collect();
        let method = doris_company::get_company(&self.pool, company, user)
            .await
            .map_err(|err| status(err.into()))?
            .accounting_method;
        Ok(Response::new(pb::ListSupplierInvoicesResponse {
            invoices,
            cash_method: method == AccountingMethod::Cash,
        }))
    }

    async fn register_supplier_invoice(
        &self,
        request: Request<pb::RegisterSupplierInvoiceRequest>,
    ) -> Result<Response<pb::RegisterSupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let lines = req
            .lines
            .iter()
            .map(|l| InvoiceLine::new(l.account, l.net, l.vat_rate))
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain_status)?;
        let new = NewSupplierInvoice {
            invoice_number: &req.invoice_number,
            invoice_date: date(&req.invoice_date)?,
            due_date: date(&req.due_date)?,
            reference: &req.reference,
            lines,
            vat: req.vat,
        };
        let attachments = new_attachments(req.attachments.clone())?;
        let number = doris_invoicing::register_supplier_invoice(
            &self.pool,
            company,
            user,
            req.supplier_number,
            new,
            attachments,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::RegisterSupplierInvoiceResponse { number }))
    }

    async fn pay_supplier_invoice(
        &self,
        request: Request<pb::PaySupplierInvoiceRequest>,
    ) -> Result<Response<pb::PaySupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::pay_supplier_invoice(
            &self.pool,
            company,
            user,
            req.number,
            date(&req.date)?,
            req.account,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::PaySupplierInvoiceResponse {}))
    }

    async fn cancel_supplier_invoice(
        &self,
        request: Request<pb::CancelSupplierInvoiceRequest>,
    ) -> Result<Response<pb::CancelSupplierInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::cancel_supplier_invoice(&self.pool, company, user, req.number, &req.reason, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CancelSupplierInvoiceResponse {}))
    }

    async fn reverse_supplier_invoice_payment(
        &self,
        request: Request<pb::ReverseSupplierInvoicePaymentRequest>,
    ) -> Result<Response<pb::ReverseSupplierInvoicePaymentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::reverse_supplier_invoice_payment(
            &self.pool,
            company,
            user,
            req.number,
            &req.reason,
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::ReverseSupplierInvoicePaymentResponse {}))
    }

    async fn get_supplier_invoice_attachment(
        &self,
        request: Request<pb::GetSupplierInvoiceAttachmentRequest>,
    ) -> Result<Response<pb::GetSupplierInvoiceAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let (attachment, data) = doris_invoicing::supplier_invoice_attachment(
            &self.pool,
            company,
            user,
            req.number,
            &req.sha256,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetSupplierInvoiceAttachmentResponse {
            attachment: Some(attachment_message(&attachment)),
            data,
        }))
    }
```
Add the conversion:
```rust
fn supplier_invoice_pb(i: SupplierInvoice) -> pb::SupplierInvoice {
    let status_code = i.status_code().to_owned();
    let paid_date = match &i.status {
        Status::Paid { date, .. } => date.to_string(),
        _ => String::new(),
    };
    let r = i.invoice;
    pb::SupplierInvoice {
        number: i.number,
        supplier_number: r.supplier.number,
        supplier_name: r.supplier.name.as_str().to_owned(),
        invoice_number: r.invoice_number.as_str().to_owned(),
        invoice_date: r.invoice_date.to_string(),
        due_date: r.due_date.to_string(),
        reference: r.reference.map(|x| x.as_str().to_owned()).unwrap_or_default(),
        lines: r
            .lines
            .iter()
            .map(|l| pb::InvoiceLine {
                account: l.account.get().into(),
                net: l.net,
                vat_rate: l.vat_rate.percent(),
            })
            .collect(),
        vat: r.vat,
        total: r.total,
        status: status_code,
        paid_date,
        vouchers: i
            .vouchers
            .iter()
            .map(|v| pb::VoucherRef {
                fiscal_year_start: v.fiscal_year_start.to_string(),
                number: v.number,
            })
            .collect(),
        attachments: i.attachments.iter().map(attachment_message).collect(),
        bankgiro: r.supplier.bankgiro.map(|b| b.formatted()).unwrap_or_default(),
        plusgiro: r.supplier.plusgiro.map(|p| p.formatted()).unwrap_or_default(),
        iban: r.supplier.iban.map(|x| x.formatted()).unwrap_or_default(),
    }
}
```
`pb::VoucherRef` here is the invoicing proto's `VoucherRef`, not ledger's.

- [ ] **Step 6: Raise the limits and gate the service in `crates/server/src/lib.rs`**

Replace `.add_service(InvoicingServiceServer::new(invoicing))` with:
```rust
        .add_service(
            // Underlag ride along with supplier invoices, as with vouchers.
            InvoicingServiceServer::new(invoicing)
                .max_decoding_message_size(ledger::MAX_REQUEST)
                .max_encoding_message_size(ledger::MAX_RESPONSE),
        )
```
In `session_gate`, replace the `ledger` binding and its use with:
```rust
    let path = request.uri().path();
    let large = path.starts_with("/doris.ledger.v1.LedgerService/")
        || path.starts_with("/doris.invoicing.v1.InvoicingService/");
    if large && let Err(status) = grpc::session_user(&pool, request.headers()).await {
```
Update its doc comment so it names both services.

- [ ] **Step 7: Run all server tests and check that they pass**

Run: `cargo fmt --all && cargo test -p doris-server && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, including the existing `invoicing.rs` and `ledger.rs` tests.

- [ ] **Step 8: Commit**

```bash
git add proto crates/server Cargo.lock
git commit -m "Serve supplier invoices over gRPC-Web"
```

---

### Task 6: The Leverantörsfakturor pages

**Files:**
- Modify: `crates/web/src/api.rs`, `crates/web/src/errors.rs`, `crates/web/src/format.rs`, `crates/web/src/ui.rs` (make `SELECT` `pub`)
- Create: `crates/web/src/pages/supplier_invoices.rs`, `crates/web/src/pages/new_supplier_invoice.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`
- Modify: `e2e/tests/fixtures.ts` (add `addSupplier`)
- Test: unit tests in `format.rs`, `errors.rs` and the two pages; `e2e/tests/supplier_invoices.spec.ts`

**Interfaces:**
- Consumes: `ipb::*` from Task 5.
- Produces: the routes `/supplier-invoices` and `/supplier-invoices/new`; `format::plus_days(&str, i64) -> Option<String>`; and in e2e, `addSupplier(page, app, name)`.

- [ ] **Step 1: Write the failing unit tests**

In `crates/web/src/format.rs`, inside its `mod tests` (create `#[cfg(test)] mod tests { use super::*; … }` if there is none):
```rust
    #[test]
    fn plus_days_crosses_months_years_and_leap_days() {
        assert_eq!(plus_days("2026-01-31", 30).as_deref(), Some("2026-03-02"));
        assert_eq!(plus_days("2026-12-15", 30).as_deref(), Some("2027-01-14"));
        assert_eq!(plus_days("2024-02-01", 29).as_deref(), Some("2024-03-01"));
        assert_eq!(plus_days("2026-03-01", 0).as_deref(), Some("2026-03-01"));
        assert_eq!(plus_days("idag", 30), None);
        assert_eq!(plus_days("2026-13-01", 30), None);
    }
```
In `crates/web/src/errors.rs` `mod tests`:
```rust
    #[test]
    fn supplier_invoice_codes_have_swedish_messages() {
        for code in [
            "supplier_invoice_not_found",
            "supplier_inactive",
            "invalid_invoice_number",
            "duplicate_supplier_invoice",
            "invalid_due_date",
            "invalid_reference",
            "invalid_invoice_lines",
            "invalid_vat_rate",
            "invalid_vat_amount",
            "invalid_invoice_account",
            "invalid_payment_account",
            "supplier_invoice_paid",
            "supplier_invoice_not_paid",
            "supplier_invoice_cancelled",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }
```

Run: `cargo test -p doris-web plus_days supplier_invoice_codes`
Expected: FAIL. `plus_days` doesn't compile, and the messages are missing.

- [ ] **Step 2: Add `plus_days` and the messages**

In `crates/web/src/format.rs`:
```rust
/// `date` (`YYYY-MM-DD`) plus `days`, or `None` if it isn't a date. Howard
/// Hinnant's civil-day arithmetic, so no date library goes into the wasm.
pub fn plus_days(date: &str, days: i64) -> Option<String> {
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
    let z = era * 146_097 + doe + days;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}
```
In `crates/web/src/errors.rs`, before the `_` arm:
```rust
        "supplier_invoice_not_found" => "Leverantörsfakturan finns inte.",
        "supplier_inactive" => "Leverantören är inaktiv. Aktivera den eller välj en annan.",
        "invalid_invoice_number" => "Fakturanumret måste vara 1–50 tecken.",
        "duplicate_supplier_invoice" => {
            "Den här fakturan från leverantören är redan registrerad."
        }
        "invalid_due_date" => "Förfallodatumet kan inte vara före fakturadatumet.",
        "invalid_reference" => "OCR/meddelande får vara högst 50 tecken.",
        "invalid_invoice_lines" => "En faktura ska ha 1–50 rader med belopp över noll.",
        "invalid_vat_rate" => "Momssatsen ska vara 25, 12, 6 eller 0 %.",
        "invalid_vat_amount" => "Momsen får skilja högst 1 kr från den uträknade.",
        "invalid_invoice_account" => {
            "Raderna kan inte bokföras på 2440 eller ett momskonto. Doris gör det själv."
        }
        "invalid_payment_account" => "Betalkontot ska vara ett konto i 1900–1999.",
        "supplier_invoice_paid" => "Fakturan är redan betald.",
        "supplier_invoice_not_paid" => "Fakturan är inte betald.",
        "supplier_invoice_cancelled" => "Fakturan är makulerad.",
```
Run: `cargo test -p doris-web plus_days supplier_invoice_codes`
Expected: PASS.

- [ ] **Step 3: Write the failing e2e test `e2e/tests/supplier_invoices.spec.ts`**

First add to `e2e/tests/fixtures.ts`:
```ts
export async function addSupplier(page: Page, app: string, name: string) {
  await page.goto(`${app}/suppliers`);
  await page.getByRole("button", { name: "Ny leverantör" }).click();
  await page.getByLabel("Namn", { exact: true }).fill(name);
  await page.getByLabel("Bankgiro").fill("5050-1055");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^1 ${name}`) })).toBeVisible();
}
```

```ts
import type { Locator, Page } from "@playwright/test";
import { addCompany, addSupplier, expect, register, test } from "./fixtures";

// The Status cell; the row's buttons have words of their own.
const status = (row: Locator) => row.getByRole("cell").nth(6);

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function registerInvoice(page: Page, app: string, invoiceNumber: string) {
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Lev AB" });
  await page.getByLabel("Fakturanummer").fill(invoiceNumber);
  await page.getByLabel("Konto, rad 1").fill("5410");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await expect(page.getByLabel("Moms", { exact: true })).toHaveValue("200,00");
  await expect(page.getByText(/Att betala 1\s000,00/)).toBeVisible();
  await page.getByLabel("Underlag").setInputFiles([pdf("faktura.pdf")]);
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Leverantörsfakturor" })).toBeVisible();
}

test("a supplier invoice is registered with its underlag, paid, reversed and paid again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await registerInvoice(page, app, "F-4711");

  const row = page.getByRole("row", { name: /^1 Lev AB F-4711/ });
  await expect(status(row)).toHaveText("Obetald");
  await row.getByRole("button", { name: "Detaljer" }).click();
  await expect(page.getByText("faktura.pdf")).toBeVisible();
  await expect(page.getByText("Bankgiro 5050-1055")).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura 1, Lev AB \(F-4711\)/ })).toContainText("1 underlag");

  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await page.getByLabel("Visa betalda och makulerade").check();
  await row.getByRole("button", { name: "Betala" }).click();
  await expect(page.getByLabel("Betalkonto")).toHaveValue("1930");
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(status(row)).toHaveText("Betald");

  await row.getByRole("button", { name: "Ångra betalning" }).click();
  await page.getByLabel("Anledning").fill("Fel konto");
  await page.getByRole("button", { name: "Bekräfta ångring" }).click();
  await expect(status(row)).toHaveText("Obetald");

  await row.getByRole("button", { name: "Betala" }).click();
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(status(row)).toHaveText("Betald");
});

test("a supplier invoice is cancelled and the same number registered again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-1");

  const first = page.getByRole("row", { name: /^1 Lev AB F-1/ });
  await first.getByRole("button", { name: "Makulera" }).click();
  await page.getByLabel("Anledning").fill("Dubbelregistrerad");
  await page.getByRole("button", { name: "Bekräfta makulering" }).click();
  await expect(first).toHaveCount(0);
  await page.getByLabel("Visa betalda och makulerade").check();
  await expect(status(first)).toHaveText("Makulerad");

  await registerInvoice(page, app, "F-1");
  await expect(status(page.getByRole("row", { name: /^2 Lev AB F-1/ }))).toHaveText("Obetald");
});

test("a duplicate invoice number shows a Swedish error", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-1");
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Lev AB" });
  await page.getByLabel("Fakturanummer").fill("F-1");
  await page.getByLabel("Konto, rad 1").fill("5410");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("alert")).toHaveText("Den här fakturan från leverantören är redan registrerad.");
});
```

Run: `make web && cargo build -p doris-server && (cd e2e && npx playwright test supplier_invoices)`
Expected: FAIL on the missing "Leverantörsfakturor" link.

- [ ] **Step 4: Plumbing**

`crates/web/src/api.rs`: change `invoicing_api` to
```rust
pub fn invoicing_api() -> InvoicingApi {
    // Room for a 10 MiB underlag coming back from GetSupplierInvoiceAttachment.
    InvoicingServiceClient::new(client()).max_decoding_message_size(11 << 20)
}
```
`crates/web/src/ui.rs`: change `const SELECT` to `pub const SELECT`.

- [ ] **Step 5: Write `crates/web/src/pages/new_supplier_invoice.rs`**

```rust
//! Register a supplier invoice in the active company. The server books it
//! and decides its number.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::attachments::{check_sizes, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::format::{amount, parse_amount, plus_days, today};
use crate::ui::{
    Button, Card, ErrorAlert, Field, FileInput, SELECT, SELECT_OPTION, Select, TextInput, Variant,
};
use crate::voucher_lines::account_number;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

/// VAT per rate on that rate's summed net, rounded half up: the server's
/// rule, shown while typing. The server decides.
fn preview_vat(lines: &[(i64, u32)]) -> i64 {
    let mut by_rate = std::collections::BTreeMap::<u32, i64>::new();
    for &(net, rate) in lines {
        *by_rate.entry(rate).or_default() += net;
    }
    by_rate
        .into_iter()
        .map(|(rate, net)| (net * i64::from(rate) + 50) / 100)
        .sum()
}

#[derive(Clone, Copy)]
struct Row {
    id: u32,
    account: RwSignal<String>,
    net: RwSignal<String>,
    rate: RwSignal<String>,
}

impl Row {
    fn new(id: u32) -> Self {
        Self {
            id,
            account: RwSignal::new(String::new()),
            net: RwSignal::new(String::new()),
            rate: RwSignal::new("25".into()),
        }
    }

    /// (net in öre, rate) as typed; an unreadable amount counts as 0.
    fn preview(&self) -> (i64, u32) {
        (
            parse_amount(&self.net.get()).unwrap_or(0),
            self.rate.get().parse().unwrap_or(25),
        )
    }

    fn request(&self) -> Option<ipb::InvoiceLine> {
        Some(ipb::InvoiceLine {
            account: account_number(&self.account.get_untracked()),
            net: parse_amount(&self.net.get_untracked())?,
            vat_rate: self.rate.get_untracked().parse().unwrap_or(25),
        })
    }
}

#[component]
pub fn NewSupplierInvoice() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let suppliers = RwSignal::new(Vec::<ipb::Supplier>::new());
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let supplier = RwSignal::new(String::new());
    let invoice_number = RwSignal::new(String::new());
    let invoice_date = RwSignal::new(today());
    let due_date = RwSignal::new(plus_days(&today(), 30).unwrap_or_default());
    let reference = RwSignal::new(String::new());
    let next_id = StoredValue::new(1u32);
    let rows = RwSignal::new(vec![Row::new(0)]);
    // The VAT field, and the computed amount it last showed: while they are
    // the same the user hasn't changed it, and it follows the lines.
    let vat = RwSignal::new(amount(0));
    let auto_vat = RwSignal::new(amount(0));
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
    let reading = RwSignal::new(0u32);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let form_company = StoredValue::new(String::new());

    Effect::new(move |_| {
        let date = invoice_date.get();
        if let Some(due) = plus_days(&date, 30) {
            due_date.set(due);
        }
    });
    Effect::new(move |_| {
        let lines: Vec<(i64, u32)> = rows.get().iter().map(Row::preview).collect();
        let computed = amount(preview_vat(&lines));
        if vat.get_untracked() == auto_vat.get_untracked() {
            vat.set(computed.clone());
        }
        auto_vat.set(computed);
    });
    Effect::new(move |_| {
        let company_id = companies.active.get();
        suppliers.set(Vec::new());
        accounts.set(Vec::new());
        files.set(Vec::new());
        error.set(None);
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let listed = invoicing_api()
                .list_suppliers(ipb::ListSuppliersRequest { company_id: company_id.clone() })
                .await;
            let chart = ledger_api()
                .list_accounts(lpb::ListAccountsRequest { company_id: company_id.clone() })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = listed {
                let active: Vec<_> = response.into_inner().suppliers.into_iter().filter(|s| s.active).collect();
                supplier.set(active.first().map(|s| s.number.to_string()).unwrap_or_default());
                suppliers.set(active);
            }
            if let Ok(response) = chart {
                accounts.set(response.into_inner().accounts);
            }
        });
    });

    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = form_company.get_value();
        reading.update(|n| *n += 1);
        spawn_local(async move {
            let picked = read_files(&input).await;
            reading.try_update(|n| *n -= 1);
            if company_id != form_company.get_value() {
                return;
            }
            match picked {
                Ok(picked) => files.update(|f| f.extend(picked)),
                Err(code) => error.set(Some(describe_code(code))),
            }
        });
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        if reading.get_untracked() > 0 {
            return;
        }
        let Some(lines) = rows.get_untracked().iter().map(Row::request).collect::<Option<Vec<_>>>() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        let typed_vat = vat.get_untracked();
        let vat_override = if typed_vat == auto_vat.get_untracked() {
            None
        } else {
            match parse_amount(&typed_vat) {
                Some(ore) => Some(ore),
                None => return error.set(Some("Skriv beloppen som 1 234,50.".into())),
            }
        };
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
        busy.set(true);
        let company_id = form_company.get_value();
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = ipb::RegisterSupplierInvoiceRequest {
                company_id: company_id.clone(),
                supplier_number: supplier.get_untracked().parse().unwrap_or(0),
                invoice_number: invoice_number.get_untracked(),
                invoice_date: invoice_date.get_untracked(),
                due_date: due_date.get_untracked(),
                reference: reference.get_untracked(),
                lines,
                vat: vat_override,
                attachments: files.get_untracked(),
            };
            let result = invoicing_api().register_supplier_invoice(request).await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => navigate("/supplier-invoices", Default::default()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let net_total = move || rows.get().iter().map(|r| r.preview().0).sum::<i64>();
    let to_pay = move || net_total() + parse_amount(&vat.get()).unwrap_or(0);

    view! {
        <Card title="Ny leverantörsfaktura">
            <form class="grid gap-4" data-wide novalidate on:submit=submit>
                <ErrorAlert message=error />
                <div class="grid grid-cols-2 gap-4">
                    <Select label="Leverantör" id="invoice_supplier" value=supplier>
                        {move || {
                            suppliers
                                .get()
                                .into_iter()
                                .map(|s| {
                                    let name = s.details.map(|d| d.name).unwrap_or_default();
                                    view! {
                                        <option class=SELECT_OPTION value=s.number.to_string()>
                                            {format!("{} {}", s.number, name)}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </Select>
                    <Field label="Fakturanummer" id="invoice_number" value=invoice_number />
                    <Field label="Fakturadatum" id="invoice_date" kind="date" value=invoice_date />
                    <Field label="Förfallodatum" id="invoice_due_date" kind="date" value=due_date />
                    <Field label="OCR/meddelande" id="invoice_reference" value=reference />
                </div>
                <datalist id="invoice_accounts">
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
                    <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2 text-muted-foreground">
                        <span>"Konto"</span>
                        <span>"Belopp exkl. moms"</span>
                        <span>"Moms"</span>
                        <span></span>
                    </div>
                    <For each=move || rows.get().into_iter().enumerate().collect::<Vec<_>>() key=|(i, r)| (*i, r.id) let((index, row))>
                        <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2">
                            <TextInput label=format!("Konto, rad {}", index + 1) value=row.account list="invoice_accounts" />
                            <TextInput label=format!("Belopp exkl. moms, rad {}", index + 1) value=row.net inputmode="decimal" />
                            <select
                                class=SELECT
                                aria-label=format!("Momssats, rad {}", index + 1)
                                prop:value=move || row.rate.get()
                                on:change=move |ev| row.rate.set(event_target_value(&ev))
                            >
                                <option class=SELECT_OPTION value="25">"25 %"</option>
                                <option class=SELECT_OPTION value="12">"12 %"</option>
                                <option class=SELECT_OPTION value="6">"6 %"</option>
                                <option class=SELECT_OPTION value="0">"0 %"</option>
                            </select>
                            <Button
                                variant=Variant::Ghost
                                kind="button"
                                on:click=move |_| rows.update(|all| all.retain(|other| other.id != row.id))
                            >
                                "Ta bort"
                            </Button>
                        </div>
                    </For>
                    <div>
                        <Button
                            variant=Variant::Ghost
                            kind="button"
                            on:click=move |_| {
                                let id = next_id.get_value();
                                next_id.set_value(id + 1);
                                rows.update(|all| all.push(Row::new(id)));
                            }
                        >
                            "Lägg till rad"
                        </Button>
                    </div>
                </div>
                <div class="grid grid-cols-[10rem_1fr] items-end gap-4">
                    <Field label="Moms" id="invoice_vat" value=vat />
                    <p class="text-xs/relaxed">
                        {move || format!("Netto {} · Att betala {}", amount(net_total()), amount(to_pay()))}
                    </p>
                </div>
                <div class="grid gap-2">
                    <FileInput label="Underlag" id="invoice_files" on_pick=pick />
                    <ul class="grid gap-1">
                        {move || {
                            files.with(|picked| {
                                picked
                                    .iter()
                                    .enumerate()
                                    .map(|(i, f)| {
                                        let name = f.file_name.clone();
                                        view! {
                                            <li class="flex items-center justify-between gap-4 text-xs/relaxed">
                                                <span>{format!("{} ({})", name, size_label(f.data.len() as u64))}</span>
                                                <Button
                                                    variant=Variant::Ghost
                                                    kind="button"
                                                    attr:aria-label=format!("Ta bort {name}")
                                                    on:click=move |_| files.update(|f| { f.remove(i); })
                                                >
                                                    "Ta bort"
                                                </Button>
                                            </li>
                                        }
                                    })
                                    .collect_view()
                            })
                        }}
                    </ul>
                </div>
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>"Registrera"</Button>
            </form>
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::preview_vat;

    #[test]
    fn the_preview_rounds_vat_per_rate_like_the_server() {
        assert_eq!(preview_vat(&[(33, 25), (33, 25), (33, 25)]), 25);
        assert_eq!(preview_vat(&[(1000, 12), (50, 6), (700, 0)]), 123);
        assert_eq!(preview_vat(&[]), 0);
    }
}
```

The visible "Att betala" text contains a no-break space from `amount`. The e2e test matches it with `\s`.

- [ ] **Step 6: Write `crates/web/src/pages/supplier_invoices.rs`**

```rust
//! Leverantörsfakturor: the active company's supplier invoices. They are
//! paid, cancelled and payments reversed from here; the server books it all.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::attachments::{open_in, size_label};
use crate::errors::{describe, describe_code};
use crate::format::{amount, today};
use crate::ui::{
    Button, Checkbox, ErrorAlert, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// Obetald, Förfallen (unpaid past its due date), Betald or Makulerad.
fn status_label(invoice: &ipb::SupplierInvoice, today: &str) -> &'static str {
    match invoice.status.as_str() {
        "paid" => "Betald",
        "cancelled" => "Makulerad",
        _ if invoice.due_date.as_str() < today => "Förfallen",
        _ => "Obetald",
    }
}

#[component]
pub fn SupplierInvoices() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The invoices and the company they were loaded for, set together.
    let invoices = RwSignal::new((String::new(), Vec::<ipb::SupplierInvoice>::new()));
    let show_all = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => invoices.set((company_id, response.into_inner().invoices)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        invoices.set((String::new(), Vec::new()));
        error.set(None);
        load();
    });
    let changed = Callback::new(move |()| load());

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-center justify-between">
                <h1 class="text-sm font-medium">"Leverantörsfakturor"</h1>
                <A href="/supplier-invoices/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">
                    "Ny leverantörsfaktura"
                </A>
            </div>
            <ErrorAlert message=error />
            <Checkbox label="Visa betalda och makulerade" id="show_all_invoices" checked=show_all />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Leverantör"</th>
                        <th class=TABLE_HEADER_CELL>"Fakturanr"</th>
                        <th class=TABLE_HEADER_CELL>"Fakturadatum"</th>
                        <th class=TABLE_HEADER_CELL>"Förfaller"</th>
                        <th class=TABLE_HEADER_CELL>"Belopp"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = invoices.get();
                            list.into_iter()
                                .filter(|i| i.status == "unpaid" || show_all.get())
                                .map(|i| (company_id.clone(), i))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, i)| (company_id.clone(), i.number, i.status.clone(), i.vouchers.len())
                        let((company_id, invoice))
                    >
                        <InvoiceRow company_id=company_id invoice=invoice changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Closed,
    Details,
    Pay,
    Cancel,
    Reverse,
}

#[component]
fn InvoiceRow(
    company_id: String,
    invoice: ipb::SupplierInvoice,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let number = invoice.number;
    let label = status_label(&invoice, &today());
    let (unpaid, paid) = (invoice.status == "unpaid", invoice.status == "paid");
    let invoice = StoredValue::new(invoice);
    let panel = RwSignal::new(Panel::Closed);
    let pay_date = RwSignal::new(today());
    let pay_account = RwSignal::new("1930".to_string());
    let reason = RwSignal::new(String::new());
    let toggle = move |wanted: Panel| {
        error.set(None);
        panel.update(|p| *p = if *p == wanted { Panel::Closed } else { wanted });
    };

    let act = move |action: Panel| {
        error.set(None);
        spawn_local(async move {
            let company_id = company_id.get_value();
            let mut api = invoicing_api();
            let result = match action {
                Panel::Pay => api
                    .pay_supplier_invoice(ipb::PaySupplierInvoiceRequest {
                        company_id,
                        number,
                        date: pay_date.get_untracked(),
                        account: pay_account.get_untracked().trim().parse().unwrap_or(0),
                    })
                    .await
                    .map(|_| ()),
                Panel::Cancel => api
                    .cancel_supplier_invoice(ipb::CancelSupplierInvoiceRequest {
                        company_id,
                        number,
                        reason: reason.get_untracked(),
                    })
                    .await
                    .map(|_| ()),
                Panel::Reverse => api
                    .reverse_supplier_invoice_payment(ipb::ReverseSupplierInvoicePaymentRequest {
                        company_id,
                        number,
                        reason: reason.get_untracked(),
                    })
                    .await
                    .map(|_| ()),
                Panel::Closed | Panel::Details => return,
            };
            match result {
                Ok(()) => {
                    panel.set(Panel::Closed);
                    reason.set(String::new());
                    changed.run(());
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let open_attachment = move |sha256: String| {
        // Opened by the click itself: browsers block windows opened after an await.
        let Some(tab) = window().open_with_url_and_target("", "_blank").ok().flatten() else {
            return error.set(Some(describe_code("popup_blocked")));
        };
        let company = company_id.get_value();
        spawn_local(async move {
            let result = invoicing_api()
                .get_supplier_invoice_attachment(ipb::GetSupplierInvoiceAttachmentRequest {
                    company_id: company,
                    number,
                    sha256,
                })
                .await;
            match result {
                Ok(response) => {
                    let response = response.into_inner();
                    let mime = response.attachment.map(|a| a.content_type).unwrap_or_default();
                    open_in(&tab, &mime, &response.data);
                }
                Err(status) => {
                    let _ = tab.close();
                    error.try_set(Some(describe(&status)));
                }
            }
        });
    };

    let i = invoice.get_value();
    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>{i.supplier_name.clone()}</td>
            <td class=TABLE_CELL>{i.invoice_number.clone()}</td>
            <td class=TABLE_CELL>{i.invoice_date.clone()}</td>
            <td class=TABLE_CELL>{i.due_date.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(i.total)}</td>
            <td class=if label == "Förfallen" { format!("{TABLE_CELL} text-destructive") } else { TABLE_CELL.to_owned() }>{label}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Details)>"Detaljer"</Button>
                {unpaid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Pay)>"Betala"</Button>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Cancel)>"Makulera"</Button>
                })}
                {paid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Reverse)>"Ångra betalning"</Button>
                })}
            </td>
        </tr>
        <Show when=move || panel.get() != Panel::Closed>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL colspan="8">
                    {move || match panel.get() {
                        Panel::Details => {
                            let i = invoice.get_value();
                            view! {
                                <div class="grid gap-2 text-xs/relaxed">
                                    <ul class="grid gap-1">
                                        {i.lines.iter().map(|l| view! {
                                            <li>{format!("{} · {} · {} %", l.account, amount(l.net), l.vat_rate)}</li>
                                        }).collect_view()}
                                    </ul>
                                    <p>{format!("Moms {} · Att betala {}", amount(i.vat), amount(i.total))}</p>
                                    {(!i.reference.is_empty()).then(|| view! { <p>{format!("OCR/meddelande {}", i.reference)}</p> })}
                                    {(!i.bankgiro.is_empty()).then(|| view! { <p>{format!("Bankgiro {}", i.bankgiro)}</p> })}
                                    {(!i.plusgiro.is_empty()).then(|| view! { <p>{format!("Plusgiro {}", i.plusgiro)}</p> })}
                                    {(!i.iban.is_empty()).then(|| view! { <p>{format!("IBAN {}", i.iban)}</p> })}
                                    {(!i.vouchers.is_empty()).then(|| view! {
                                        <p>
                                            {i.vouchers.iter().map(|v| format!("Ver {} ({})", v.number, v.fiscal_year_start)).collect::<Vec<_>>().join(", ")}
                                            " · "
                                            <A href="/vouchers" attr:class="underline-offset-4 hover:underline">"Verifikationer"</A>
                                        </p>
                                    })}
                                    <ul class="flex flex-wrap gap-2">
                                        {i.attachments.iter().map(|a| {
                                            let sha = a.id.clone();
                                            view! {
                                                <li>
                                                    <Button variant=Variant::Ghost kind="button" on:click=move |_| open_attachment(sha.clone())>
                                                        {format!("{} ({})", a.file_name, size_label(a.size))}
                                                    </Button>
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                </div>
                            }.into_any()
                        }
                        Panel::Pay => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Betaldatum" kind="date" value=pay_date />
                                <TextInput label="Betalkonto" value=pay_account inputmode="numeric" />
                                <Button kind="button" on:click=move |_| act(Panel::Pay)>"Bekräfta betalning"</Button>
                            </div>
                        }.into_any(),
                        Panel::Cancel => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Anledning" value=reason />
                                <Button kind="button" on:click=move |_| act(Panel::Cancel)>"Bekräfta makulering"</Button>
                            </div>
                        }.into_any(),
                        Panel::Reverse => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Anledning" value=reason />
                                <Button kind="button" on:click=move |_| act(Panel::Reverse)>"Bekräfta ångring"</Button>
                            </div>
                        }.into_any(),
                        Panel::Closed => ().into_any(),
                    }}
                </td>
            </tr>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice(status: &str, due: &str) -> ipb::SupplierInvoice {
        ipb::SupplierInvoice {
            status: status.into(),
            due_date: due.into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_unpaid_invoice_past_its_due_date_is_overdue() {
        assert_eq!(status_label(&invoice("unpaid", "2026-03-31"), "2026-03-31"), "Obetald");
        assert_eq!(status_label(&invoice("unpaid", "2026-03-31"), "2026-04-01"), "Förfallen");
        assert_eq!(status_label(&invoice("paid", "2026-03-31"), "2026-04-01"), "Betald");
        assert_eq!(status_label(&invoice("cancelled", "2026-03-31"), "2026-04-01"), "Makulerad");
    }
}
```

`TextInput` has no visible label, but its `aria-label` is what `getByLabel("Betalkonto")` finds. If `TextInput`'s `kind` prop doesn't accept `"date"`, check its signature: it does (`kind: &'static str`).

- [ ] **Step 7: Register the pages, the routes and the nav link**

`crates/web/src/pages/mod.rs`: add `mod new_supplier_invoice;`, `mod supplier_invoices;`, `pub use new_supplier_invoice::NewSupplierInvoice;` and `pub use supplier_invoices::SupplierInvoices;`.

`crates/web/src/app.rs`:
- Import `NewSupplierInvoice` and `SupplierInvoices` with the other pages.
- Add the routes after `/suppliers`:
  ```rust
  <Route path=path!("/supplier-invoices") view=|| view! { <SignedIn><SupplierInvoices /></SignedIn> } />
  <Route path=path!("/supplier-invoices/new") view=|| view! { <SignedIn><NewSupplierInvoice /></SignedIn> } />
  ```
- Add the nav link after "Leverantörer":
  ```rust
  <A href="/supplier-invoices" attr:class="text-muted-foreground hover:text-foreground">"Leverantörsfakturor"</A>
  ```

- [ ] **Step 8: Run all checks**

Run: `cargo fmt --all && cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: PASS, including the three new e2e tests and every existing one. In the existing `invoicing.spec.ts`, `addSupplier` must not clash with anything: it is a new export.

- [ ] **Step 9: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Add the Leverantörsfakturor pages"
```

---

### Task 7: Kontantmetoden on Räkenskapsår, end to end

**Files:**
- Modify: `crates/web/src/pages/fiscal_years.rs`
- Modify: `e2e/tests/fixtures.ts` (`addCompany` takes the accounting method)
- Test: `e2e/tests/supplier_invoices.spec.ts`

**Interfaces:**
- Consumes: `ListSupplierInvoicesResponse.cash_method`.
- Produces: `addCompany(page, app, orgNr, name, start?, method?)`, where `method` is `"Faktureringsmetoden" | "Kontantmetoden"` and defaults to faktureringsmetoden.

- [ ] **Step 1: Let `addCompany` pick the method**

In `e2e/tests/fixtures.ts`, change the signature and the radio line:
```ts
export async function addCompany(
  page: Page,
  app: string,
  orgNr: string,
  name: string,
  start = `${new Date().getFullYear()}-01-01`,
  method: "Faktureringsmetoden" | "Kontantmetoden" = "Faktureringsmetoden",
) {
```
```ts
  await page.getByLabel(method).check();
```

- [ ] **Step 2: Write the failing e2e test (append to `e2e/tests/supplier_invoices.spec.ts`)**

```ts
test("under kontantmetoden only the payment is booked, and Räkenskapsår warns while unpaid", async ({ page, app }) => {
  const warning = /Det finns obetalda leverantörsfakturor/;
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", undefined, "Kontantmetoden");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-4711");

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura/ })).toHaveCount(0);
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByText(warning)).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await page.getByRole("row", { name: /^1 Lev AB F-4711/ }).getByRole("button", { name: "Betala" }).click();
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(page.getByRole("row", { name: /^1 Lev AB F-4711/ })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura 1, Lev AB \(F-4711\)/ })).toContainText("1 underlag");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByRole("heading", { name: "Räkenskapsår" })).toBeVisible();
  await expect(page.getByText(warning)).toHaveCount(0);
});
```

Run: `make web && cargo build -p doris-server && (cd e2e && npx playwright test supplier_invoices -g kontantmetoden)`
Expected: FAIL. The warning is not shown.

- [ ] **Step 3: Show the warning in `crates/web/src/pages/fiscal_years.rs`**

Add `use crate::api::{invoicing_api, ipb};` next to the ledger import. In `FiscalYears`, after the existing `Effect`:
```rust
    // Kontantmetoden: unpaid supplier invoices belong in the year-end books
    // (BFL 5 kap. 2 §), which Doris doesn't book yet.
    let unpaid_under_cash = RwSignal::new(false);
    Effect::new(move |_| {
        let company_id = companies.active.get();
        unpaid_under_cash.set(false);
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = result {
                let response = response.into_inner();
                unpaid_under_cash.set(
                    response.cash_method && response.invoices.iter().any(|i| i.status == "unpaid"),
                );
            }
        });
    });
```
In the view, after `<ErrorAlert message=error />`:
```rust
            {move || unpaid_under_cash.get().then(|| view! {
                <p class="text-xs/relaxed text-muted-foreground">
                    "Det finns obetalda leverantörsfakturor. Med kontantmetoden ska de bokföras vid räkenskapsårets slut (BFL 5 kap. 2 §). Doris gör inte det än."
                </p>
            })}
```

- [ ] **Step 4: Run all checks**

Run: `cargo fmt --all && cargo test --workspace && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make e2e`
Expected: PASS. Every existing `addCompany` call still gets faktureringsmetoden.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Warn on Räkenskapsår about unpaid supplier invoices under kontantmetoden"
```

---

### Task 8: Budget, AGENTS.md and the spec

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/specs/2026-10-04-leverantorsfakturor-design.md`

- [ ] **Step 1: Check the wasm budget**

Run: `make dist`
Expected: `dist: … bytes gzipped (budget 500000)` and success. If it is over budget, shrink the new pages; never raise the budget.

- [ ] **Step 2: Update `AGENTS.md`**

- Under **Layout**, change the `crates/invoicing` line to `doris-invoicing: customers, suppliers and supplier invoices`.
- Replace the underlag rule's sentence "They are read only through `voucher_attachments` for the company's own voucher, never by hash alone." with "They are read only through the company's own voucher (`voucher_attachments`) or the company's own supplier invoice, never by hash alone."
- Add under **API**, after the `InvoicingService` bullet:
  ```
  - Supplier invoices (`supplier-invoices-{company}` stream, `supplier_invoices`
    projection) follow the company's accounting method: faktureringsmetoden
    books the registration against 2440 and the payment 2440 against the
    bank; kontantmetoden books nothing until payment, then cost and VAT
    against the bank, with the underlag. Cancelling (unpaid only) and
    reversing a payment book `correct_voucher_in` corrections, dated today
    or the voucher's fiscal year's last day. One live invoice per supplier
    and invoice number (partial unique index). `InvoicingService` takes 21
    MiB messages and sends up to 11 MiB, and `session_gate` covers it too.
    Codes: `supplier_invoice_not_found`, `supplier_inactive`,
    `invalid_invoice_number`, `duplicate_supplier_invoice`,
    `invalid_due_date`, `invalid_reference`, `invalid_invoice_lines`,
    `invalid_vat_rate`, `invalid_vat_amount`, `invalid_invoice_account`,
    `invalid_payment_account`, `supplier_invoice_paid`,
    `supplier_invoice_not_paid` and `supplier_invoice_cancelled`; ledger
    errors keep their codes. Unpaid invoices at year end under
    kontantmetoden are not booked yet; Räkenskapsår warns.
  ```
- In the `session_gate` bullet, say it answers `LedgerService` and `InvoicingService` calls without a session before the body is read.

- [ ] **Step 3: Apply the spec amendments**

Edit `docs/superpowers/specs/2026-10-04-leverantorsfakturor-design.md` to match the "Spec amendments" section at the top of this plan:
- In the event, `invoice: Registration`.
- Remove the `supplier_invoice_attachments` table and say how a file is read instead.
- `invalid_reason` and `voucher_date_in_future` come from invoicing's own variants.
- An out-of-range account gives `invalid_invoice_account`.
- `ListSupplierInvoicesResponse` has `cash_method`.
- Vouchers are shown as text with a link to Verifikationer.

- [ ] **Step 4: Run everything one last time**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md docs/superpowers/specs/2026-10-04-leverantorsfakturor-design.md
git commit -m "Document supplier invoices"
```
