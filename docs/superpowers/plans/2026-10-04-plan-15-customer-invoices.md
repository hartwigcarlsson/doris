# Plan 15: Customer Invoices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Members register customer invoices (kundfakturor) with a proposed, editable invoice number. Doris books them for faktureringsmetoden or kontantmetoden, records the payment, and can cancel an invoice or reverse a payment. This reuses supplier-invoice code that is extracted rather than copied.

**Architecture:**
- First the parts both directions share are extracted:
  - `invoices.rs` (pure): `Status`, the transition checks, the voucher-line builder, `correction_date`, `voucher_text`.
  - `vat::by_rate`/`VatAmount`.
  - `lib.rs`: `store_all`.

  Supplier invoices keep their events and behaviour, and their tests pass unchanged.
- Then `customer_invoices.rs` (pure) and `customer_invoice_store.rs` (I/O, one IMMEDIATE transaction per write) are added. Writes book through `doris_ledger` and append to `customer-invoices-{company}`.
- `InvoicingService` gets six RPCs. The web pages share `invoice_ui.rs` with the supplier pages.

**Tech Stack:** Rust, sqlx 0.9 (SQLite), jiff, serde_json, tonic 0.14 gRPC-Web, prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-04-kundfakturor-design.md`

## Global Constraints
- **TDD is mandatory.** Each task ends with a commit, and every commit message ends with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01RBPdfcwGSBXA9x2g5jEGxd
  ```
- **Lint is part of each task's green**, and is run as its own command before committing:
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`

  Never chain the commit after a piped lint.
- **Language:** code, identifiers, URLs, proto and event names are English. Only user-visible UI text is Swedish.
- **Supplier invoices must not change.** Their event names and JSON shapes (including `Status` as `{"status":"unpaid"}` / `{"status":"paid","date":…,"account":…,"voucher":{…}}`) and their behaviour stay as they are. Their domain, store, server and e2e tests pass. The only test edits allowed are the two in Task 1 (new reserved account 1510 in the VAT test) and Task 7 (the warning's new wording).
- **Event sourcing:**
  - The stream is `customer-invoices-{company_id}`, with `schema_version` 1.
  - Each write is one `doris_eventstore::begin` transaction.
  - The projection is updated in the same transaction and is rebuildable from `read_all`.
- **Invoice numbers:**
  - Unique per company **including cancelled invoices**: an issued number is never reused (`DuplicateCustomerInvoice`, backed by a full unique index).
  - The proposal is the highest all-digit number + 1, else `"1"`. A number longer than `u64` counts as non-numeric.
- **Accounts:**
  - Receivable 1510. Output VAT 2611 (25 %), 2621 (12 %) and 2631 (6 %). An invoice line may not book 1510, 2440 or 2600–2699.
  - VAT is computed per rate and rounded half up. It cannot be overridden on customer invoices.
- **Wasm budget:** another session handles it. Do not change `WASM_BUDGET`. If `make dist` exceeds it, record that in the ledger and carry on.
- **No new dependencies.**

## Spec amendments (applied to the spec in the same commit as this plan)
- `InvoiceLine::new` refuses 1510, 2440 and 2600–2699 for both directions, instead of a per-direction check. Neither reskontra account belongs on an invoice line in either direction.
- The shared transition checks take `&Status` (`check_unpaid`/`check_paid`). The caller already handles "not found" with its own error.

## Review Focus
1. **Re-registering the number of a cancelled invoice:** it must be refused with `duplicate_customer_invoice`, never accepted. Pinned in Task 3 (`a_cancelled_invoice_number_is_never_reused`).
2. **Invoice numbers that aren't plain digits, or are absurdly long:** "2026-17", "A12" and a 30-digit number must neither break nor panic the proposal. Pinned in Task 2 (`the_next_number_skips_what_isnt_a_plain_number`).
3. **Mixed rates including 0 %:** a 0 % line books its net to its account but adds no VAT entry, and the total equals the net plus the VAT that was booked. Pinned in Task 2 (`mixed_rates_book_vat_per_rate_and_none_for_0_percent`).
4. **Supplier invoices stored before this change:** they must still load, because `Status` moved modules. Pinned in Task 1 (`the_status_is_stored_as_before`) and by every supplier store test.
5. **A customer with 0 days payment terms:** the proposed due date equals the invoice date and is accepted. Pinned in Task 2 (`a_due_date_on_the_invoice_date_is_accepted`) and Task 6 (e2e uses terms from the customer).

---

### Task 1: Extract what invoices share

**Files:**
- Create: `crates/invoicing/src/invoices.rs`
- Modify: `crates/invoicing/src/vat.rs`, `crates/invoicing/src/supplier_invoices.rs`, `crates/invoicing/src/lib.rs`
- Test: `crates/invoicing/tests/invoices.rs`, `crates/invoicing/tests/vat.rs`

**Interfaces:**
- Produces (in `doris_invoicing::invoices`):
  - `Status`, the same type that was in `supplier_invoices`, which re-exports it.
  - `trait InvoiceKind { const PAID: DomainError; const NOT_PAID: DomainError; const CANCELLED: DomainError; }`
  - `check_unpaid::<K>(&Status) -> Result<(), DomainError>`
  - `check_paid::<K>(&Status) -> Result<VoucherRef, DomainError>`
  - `enum Side { Debit, Credit }`
  - `entry(AccountNumber, i64, Side) -> VoucherLine`
  - `posting(&[InvoiceLine], &[(AccountNumber, i64)], Side) -> Vec<VoucherLine>`
  - `voucher_text(String) -> String`
  - `correction_date(Date, Date) -> Date`, which `supplier_invoices` re-exports too.
- Produces (in `doris_invoicing::vat`): `VatAmount { vat_rate: VatRate, amount: i64 }` and `by_rate(&[InvoiceLine]) -> Vec<VatAmount>`. `InvoiceLine::new` now also refuses 1510.
- Produces (in `lib.rs`): `async fn store_all(conn, Vec<NewAttachment>) -> Result<Vec<Attachment>>`, for the store modules.

- [ ] **Step 1: Write the failing tests**

`crates/invoicing/tests/invoices.rs`:
```rust
use doris_invoicing::domain::DomainError;
use doris_invoicing::invoices::{self, InvoiceKind, Side, Status};
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;
use serde_json::json;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(n: u32) -> AccountNumber {
    AccountNumber::parse(n).unwrap()
}

struct Kind;
impl InvoiceKind for Kind {
    const PAID: DomainError = DomainError::SupplierInvoicePaid;
    const NOT_PAID: DomainError = DomainError::SupplierInvoiceNotPaid;
    const CANCELLED: DomainError = DomainError::SupplierInvoiceCancelled;
}

fn paid() -> Status {
    Status::Paid {
        date: d("2026-03-20"),
        account: a(1930),
        voucher: VoucherRef { fiscal_year_start: d("2026-01-01"), number: 2 },
    }
}

#[test]
fn the_status_is_stored_as_before() {
    assert_eq!(serde_json::to_value(Status::Unpaid).unwrap(), json!({"status": "unpaid"}));
    assert_eq!(serde_json::to_value(Status::Cancelled).unwrap(), json!({"status": "cancelled"}));
    assert_eq!(
        serde_json::to_value(paid()).unwrap(),
        json!({"status": "paid", "date": "2026-03-20", "account": 1930,
               "voucher": {"fiscal_year_start": "2026-01-01", "number": 2}})
    );
}

#[test]
fn only_unpaid_invoices_are_paid_or_cancelled_and_only_paid_ones_reversed() {
    assert_eq!(invoices::check_unpaid::<Kind>(&Status::Unpaid), Ok(()));
    assert_eq!(invoices::check_unpaid::<Kind>(&paid()), Err(DomainError::SupplierInvoicePaid));
    assert_eq!(invoices::check_unpaid::<Kind>(&Status::Cancelled), Err(DomainError::SupplierInvoiceCancelled));
    assert_eq!(invoices::check_paid::<Kind>(&paid()).unwrap().number, 2);
    assert_eq!(invoices::check_paid::<Kind>(&Status::Unpaid), Err(DomainError::SupplierInvoiceNotPaid));
    assert_eq!(invoices::check_paid::<Kind>(&Status::Cancelled), Err(DomainError::SupplierInvoiceCancelled));
}

#[test]
fn posting_puts_lines_and_vat_on_one_side() {
    let lines = [InvoiceLine::new(3001, 80_000, 25).unwrap(), InvoiceLine::new(3004, 5_000, 0).unwrap()];
    assert_eq!(
        invoices::posting(&lines, &[(a(2611), 20_000)], Side::Credit),
        [
            VoucherLine { account: a(3001), debit: 0, credit: 80_000 },
            VoucherLine { account: a(3004), debit: 0, credit: 5_000 },
            VoucherLine { account: a(2611), debit: 0, credit: 20_000 },
        ]
    );
    assert_eq!(
        invoices::entry(a(1510), 105_000, Side::Debit),
        VoucherLine { account: a(1510), debit: 105_000, credit: 0 }
    );
}

#[test]
fn voucher_texts_are_cut_to_200_characters() {
    assert_eq!(invoices::voucher_text("Kort".into()), "Kort");
    assert_eq!(invoices::voucher_text("å".repeat(250)).chars().count(), 200);
}
```

In `crates/invoicing/tests/vat.rs`:
- Add `1510` to the account list in `a_line_needs_a_positive_amount_and_a_cost_account`, so it reads `for account in [1510, 2440, 2640, 2611, 2600, 2699, 999, 9000] {`.
- Append:
```rust
#[test]
fn vat_by_rate_lists_the_rates_with_vat_highest_first() {
    let lines = [
        line(6110, 700, 0),
        line(4010, 50, 6),
        line(5410, 33, 25),
        line(5410, 1000, 12),
        line(5410, 33, 25),
        line(5410, 33, 25),
    ];
    let by_rate: Vec<(u32, i64)> = vat::by_rate(&lines).iter().map(|v| (v.vat_rate.percent(), v.amount)).collect();
    assert_eq!(by_rate, [(25, 25), (12, 120), (6, 3)]);
    assert_eq!(vat::computed(&lines), 148);
}
```

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test invoices --test vat`
Expected: compile errors, because `doris_invoicing::invoices` and `vat::by_rate` don't exist.

- [ ] **Step 3: Write `crates/invoicing/src/invoices.rs`**

```rust
//! What supplier and customer invoices share: the status and its
//! transitions, how their vouchers are laid out, and how a correction is
//! dated. Pure.

use crate::domain::DomainError;
use crate::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

const MAX_VOUCHER_TEXT: usize = 200;

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

/// The errors one kind of invoice reports for a wrong transition.
pub trait InvoiceKind {
    const PAID: DomainError;
    const NOT_PAID: DomainError;
    const CANCELLED: DomainError;
}

/// Only an unpaid invoice can be paid or cancelled.
pub fn check_unpaid<K: InvoiceKind>(status: &Status) -> Result<(), DomainError> {
    match status {
        Status::Unpaid => Ok(()),
        Status::Paid { .. } => Err(K::PAID),
        Status::Cancelled => Err(K::CANCELLED),
    }
}

/// Only a paid invoice's payment can be reversed: its payment voucher.
pub fn check_paid<K: InvoiceKind>(status: &Status) -> Result<VoucherRef, DomainError> {
    match status {
        Status::Paid { voucher, .. } => Ok(*voucher),
        Status::Unpaid => Err(K::NOT_PAID),
        Status::Cancelled => Err(K::CANCELLED),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

pub fn entry(account: AccountNumber, amount: i64, side: Side) -> VoucherLine {
    match side {
        Side::Debit => VoucherLine { account, debit: amount, credit: 0 },
        Side::Credit => VoucherLine { account, debit: 0, credit: amount },
    }
}

/// Each line's net, then each VAT entry, all on `side`.
pub fn posting(lines: &[InvoiceLine], vat: &[(AccountNumber, i64)], side: Side) -> Vec<VoucherLine> {
    lines
        .iter()
        .map(|l| entry(l.account, l.net, side))
        .chain(vat.iter().map(|&(account, amount)| entry(account, amount, side)))
        .collect()
}

/// Cut to the 200 characters a voucher text may have.
pub fn voucher_text(text: String) -> String {
    text.chars().take(MAX_VOUCHER_TEXT).collect()
}

/// A correction is dated today, or the last day of the corrected voucher's
/// fiscal year once today is past it (it must stay in that year).
pub fn correction_date(fiscal_year_end: Date, today: Date) -> Date {
    today.min(fiscal_year_end)
}
```
Add `pub mod invoices;` to `crates/invoicing/src/lib.rs`.

- [ ] **Step 4: Extend `vat.rs`**

In `InvoiceLine::new`, replace the reserved-account check and its doc line with:
```rust
    /// The reskontra accounts 1510 and 2440 and the VAT accounts 2600–2699
    /// are booked by Doris itself, and a number outside 1000–8999 is no
    /// account at all.
```
```rust
        if matches!(account.get(), 1510 | 2440 | 2600..=2699) {
            return Err(DomainError::InvalidInvoiceAccount);
        }
```
Replace `computed` with:
```rust
/// VAT at one rate on an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VatAmount {
    pub vat_rate: VatRate,
    pub amount: i64,
}

/// VAT per rate on that rate's summed net, rounded half up to whole öre;
/// highest rate first, rates without VAT left out.
pub fn by_rate(lines: &[InvoiceLine]) -> Vec<VatAmount> {
    let mut nets = BTreeMap::<VatRate, i64>::new();
    for line in lines {
        *nets.entry(line.vat_rate).or_default() += line.net;
    }
    // VatRate orders 25, 12, 6, 0: the declaration order.
    nets.into_iter()
        .map(|(vat_rate, net)| VatAmount {
            vat_rate,
            amount: (net * i64::from(vat_rate.percent()) + 50) / 100,
        })
        .filter(|v| v.amount > 0)
        .collect()
}

/// All the VAT, per rate as in [`by_rate`].
pub fn computed(lines: &[InvoiceLine]) -> i64 {
    by_rate(lines).iter().map(|v| v.amount).sum()
}
```

- [ ] **Step 5: Use the shared code in `supplier_invoices.rs`**

- Delete the `Status` enum, `MAX_VOUCHER_TEXT` and `correction_date`, and add:
  ```rust
  pub use crate::invoices::{Status, correction_date};
  use crate::invoices::{self, InvoiceKind, Side};
  ```
- Add:
  ```rust
  /// Supplier invoices' transition errors.
  pub struct SupplierInvoiceKind;

  impl InvoiceKind for SupplierInvoiceKind {
      const PAID: DomainError = DomainError::SupplierInvoicePaid;
      const NOT_PAID: DomainError = DomainError::SupplierInvoiceNotPaid;
      const CANCELLED: DomainError = DomainError::SupplierInvoiceCancelled;
  }
  ```
- Replace the bodies of `unpaid` and `paid`:
  ```rust
  pub fn unpaid(state: &SupplierInvoices, number: u32) -> Result<&SupplierInvoice, DomainError> {
      let invoice = state.get(number).ok_or(DomainError::SupplierInvoiceNotFound)?;
      invoices::check_unpaid::<SupplierInvoiceKind>(&invoice.status)?;
      Ok(invoice)
  }

  pub fn paid(
      state: &SupplierInvoices,
      number: u32,
  ) -> Result<(&SupplierInvoice, VoucherRef), DomainError> {
      let invoice = state.get(number).ok_or(DomainError::SupplierInvoiceNotFound)?;
      let voucher = invoices::check_paid::<SupplierInvoiceKind>(&invoice.status)?;
      Ok((invoice, voucher))
  }
  ```
- Replace `text`, `cost_side`, `registration_lines` and `payment_lines` (keep their doc comments):
  ```rust
  pub fn text(number: u32, invoice: &Registration) -> String {
      invoices::voucher_text(format!(
          "Leverantörsfaktura {number}, {} ({})",
          invoice.supplier.name.as_str(),
          invoice.invoice_number.as_str()
      ))
  }

  fn cost_side(invoice: &Registration) -> Vec<VoucherLine> {
      let vat: Vec<_> = (invoice.vat > 0)
          .then(|| (account(INPUT_VAT), invoice.vat))
          .into_iter()
          .collect();
      invoices::posting(&invoice.lines, &vat, Side::Debit)
  }

  pub fn registration_lines(invoice: &Registration) -> Vec<VoucherLine> {
      let mut lines = cost_side(invoice);
      lines.push(invoices::entry(account(ACCOUNTS_PAYABLE), invoice.total, Side::Credit));
      lines
  }

  pub fn payment_lines(
      invoice: &Registration,
      method: AccountingMethod,
      paid_from: AccountNumber,
  ) -> Vec<VoucherLine> {
      let mut lines = match method {
          AccountingMethod::Invoice => {
              vec![invoices::entry(account(ACCOUNTS_PAYABLE), invoice.total, Side::Debit)]
          }
          AccountingMethod::Cash => cost_side(invoice),
      };
      lines.push(invoices::entry(paid_from, invoice.total, Side::Credit));
      lines
  }
  ```
- In `register`, `existing.status != Status::Cancelled` keeps compiling through the re-export.

- [ ] **Step 6: Extract `store_all` in `lib.rs`**

Add:
```rust
/// Stores each underlag in the caller's transaction; the same file twice in
/// one request is `duplicate_attachment`.
async fn store_all(
    conn: &mut SqliteConnection,
    attachments: Vec<NewAttachment>,
) -> Result<Vec<Attachment>> {
    let mut stored: Vec<Attachment> = Vec::new();
    for new in attachments {
        let attachment = doris_ledger::store_attachment_in(conn, new).await?;
        if stored.iter().any(|s| s.sha256 == attachment.sha256) {
            return Err(ledger(LedgerError::DuplicateAttachment));
        }
        stored.push(attachment);
    }
    Ok(stored)
}
```
In `register_supplier_invoice`, replace the `let mut stored … }` loop with `let stored = store_all(&mut tx, attachments).await?;`.

- [ ] **Step 7: Run every test and the lints**

Run: `cargo fmt -p doris-invoicing && cargo test -p doris-invoicing && cargo test -p doris-server`
Expected: PASS. Every pre-existing supplier test passes unchanged.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/invoicing
git commit -m "Extract what supplier and customer invoices share"
```

---

### Task 2: The customer invoice domain

**Files:**
- Create: `crates/invoicing/src/customer_invoices.rs`
- Modify: `crates/invoicing/src/domain.rs` (error variants), `crates/invoicing/src/lib.rs` (`pub mod customer_invoices;`), `crates/server/src/invoicing.rs` (`domain_status` arms)
- Test: `crates/invoicing/tests/customer_invoices_domain.rs`

**Interfaces:**
- Consumes: Task 1, `supplier_invoices::{InvoiceNumber, PaymentReference}`, `domain::{CustomerDetails, Party, PartyName, VatNumber, Email, optional}`.
- Produces (in `doris_invoicing::customer_invoices`):
  - `output_vat_account(VatRate) -> Option<AccountNumber>`
  - `CustomerInvoiceKind`
  - `CustomerSnapshot`, with `CustomerSnapshot::of(&Party<CustomerDetails>)`
  - `NewCustomerInvoice<'a> { invoice_number, invoice_date, due_date, reference, lines }`
  - `CustomerRegistration { customer, invoice_number, invoice_date, due_date, reference, lines, vat: Vec<VatAmount>, total }`, with `::new` and `vat_total()`
  - `CustomerInvoiceEvent` (four variants), with `number()`
  - `CustomerInvoice { number, invoice, attachments, status, vouchers, registration_voucher }`, with `registered`, `apply` and `status_code`
  - `CustomerInvoices`, with `from_events`, `apply`, `get` and `next_number`
  - The decisions `register`, `unpaid` and `paid`
  - `next_invoice_number<'a>(impl IntoIterator<Item = &'a str>) -> String`
  - The voucher helpers `text(&CustomerRegistration) -> String`, `registration_lines(&CustomerRegistration) -> Vec<VoucherLine>` and `payment_lines(&CustomerRegistration, AccountingMethod, AccountNumber) -> Vec<VoucherLine>`
- Produces: the `DomainError` variants `CustomerInactive`, `CustomerInvoiceNotFound`, `DuplicateCustomerInvoice`, `CustomerInvoicePaid`, `CustomerInvoiceNotPaid` and `CustomerInvoiceCancelled`.

- [ ] **Step 1: Write the failing tests in `crates/invoicing/tests/customer_invoices_domain.rs`**

```rust
use doris_company::domain::AccountingMethod;
use doris_invoicing::customer_invoices::*;
use doris_invoicing::domain::{CustomerDetails, CustomerForm, DomainError, Party, PartyName};
use doris_invoicing::invoices::Status;
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(n: u32) -> AccountNumber {
    AccountNumber::parse(n).unwrap()
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

fn customer(active: bool) -> Party<CustomerDetails> {
    let details = CustomerDetails::parse(&CustomerForm {
        name: "Kund AB",
        org_nr: "556036-0793",
        vat_number: "",
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        email: "",
        payment_terms: 30,
    })
    .unwrap();
    Party { number: 7, details, active }
}

fn new_invoice(lines: Vec<InvoiceLine>) -> NewCustomerInvoice<'static> {
    NewCustomerInvoice {
        invoice_number: "1017",
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines,
    }
}

fn registration(lines: Vec<InvoiceLine>) -> CustomerRegistration {
    CustomerRegistration::new(CustomerSnapshot::of(&customer(true)).unwrap(), &new_invoice(lines)).unwrap()
}

fn voucher(number: u32) -> VoucherRef {
    VoucherRef { fiscal_year_start: d("2026-01-01"), number }
}

fn registered(number: u32, invoice: CustomerRegistration) -> CustomerInvoiceEvent {
    CustomerInvoiceEvent::CustomerInvoiceRegistered { number, invoice, attachments: vec![], voucher: Some(voucher(number)) }
}

#[test]
fn output_vat_goes_to_2611_2621_and_2631() {
    use doris_invoicing::vat::VatRate;
    assert_eq!(output_vat_account(VatRate::parse(25).unwrap()), Some(a(2611)));
    assert_eq!(output_vat_account(VatRate::parse(12).unwrap()), Some(a(2621)));
    assert_eq!(output_vat_account(VatRate::parse(6).unwrap()), Some(a(2631)));
    assert_eq!(output_vat_account(VatRate::parse(0).unwrap()), None);
}

#[test]
fn a_registration_copies_the_customer_and_works_out_vat_per_rate() {
    let r = registration(vec![line(3001, 80_000, 25), line(3002, 10_000, 12)]);
    assert_eq!(r.customer.number, 7);
    assert_eq!(r.customer.name.as_str(), "Kund AB");
    assert_eq!(r.customer.address.city.as_deref(), Some("Stockholm"));
    let vat: Vec<_> = r.vat.iter().map(|v| (v.vat_rate.percent(), v.amount)).collect();
    assert_eq!(vat, [(25, 20_000), (12, 1_200)]);
    assert_eq!((r.vat_total(), r.total), (21_200, 111_200));
}

#[test]
fn an_inactive_customer_cannot_be_invoiced() {
    assert_eq!(CustomerSnapshot::of(&customer(false)), Err(DomainError::CustomerInactive));
}

#[test]
fn each_bad_registration_field_gives_its_own_error() {
    let snapshot = || CustomerSnapshot::of(&customer(true)).unwrap();
    let ok = || new_invoice(vec![line(3001, 100, 25)]);
    let bad = |new: NewCustomerInvoice| CustomerRegistration::new(snapshot(), &new).unwrap_err();
    assert_eq!(bad(NewCustomerInvoice { invoice_number: " ", ..ok() }), DomainError::InvalidInvoiceNumber);
    assert_eq!(bad(NewCustomerInvoice { due_date: d("2026-02-28"), ..ok() }), DomainError::InvalidDueDate);
    let long = "1".repeat(51);
    assert_eq!(bad(NewCustomerInvoice { reference: &long, ..ok() }), DomainError::InvalidReference);
    assert_eq!(bad(new_invoice(vec![])), DomainError::InvalidInvoiceLines);
    assert_eq!(InvoiceLine::new(1510, 100, 25), Err(DomainError::InvalidInvoiceAccount));
}

#[test]
fn a_due_date_on_the_invoice_date_is_accepted() {
    let snapshot = CustomerSnapshot::of(&customer(true)).unwrap();
    let new = NewCustomerInvoice { due_date: d("2026-03-01"), ..new_invoice(vec![line(3001, 100, 25)]) };
    assert!(CustomerRegistration::new(snapshot, &new).is_ok());
}

#[test]
fn faktureringsmetoden_books_1510_against_income_and_output_vat() {
    let r = registration(vec![line(3001, 80_000, 25), line(3002, 10_000, 12)]);
    assert_eq!(
        registration_lines(&r),
        [debit(1510, 111_200), credit(3001, 80_000), credit(3002, 10_000), credit(2611, 20_000), credit(2621, 1_200)]
    );
    assert_eq!(
        payment_lines(&r, AccountingMethod::Invoice, a(1930)),
        [debit(1930, 111_200), credit(1510, 111_200)]
    );
}

#[test]
fn kontantmetoden_books_income_and_output_vat_at_payment() {
    let r = registration(vec![line(3001, 80_000, 25)]);
    assert_eq!(
        payment_lines(&r, AccountingMethod::Cash, a(1930)),
        [debit(1930, 100_000), credit(3001, 80_000), credit(2611, 20_000)]
    );
}

#[test]
fn mixed_rates_book_vat_per_rate_and_none_for_0_percent() {
    let r = registration(vec![line(3004, 5_000, 0), line(3003, 1_000, 6), line(3001, 33, 25)]);
    assert_eq!(
        registration_lines(&r),
        [debit(1510, 6_101), credit(3004, 5_000), credit(3003, 1_000), credit(3001, 33), credit(2611, 8), credit(2631, 60)]
    );
    assert_eq!(r.total, 5_000 + 1_000 + 33 + 8 + 60);
}

#[test]
fn the_voucher_text_names_the_invoice_and_fits_200_characters() {
    let mut r = registration(vec![line(3001, 100, 25)]);
    assert_eq!(text(&r), "Kundfaktura 1017, Kund AB");
    r.customer.name = PartyName::parse(&"å".repeat(200)).unwrap();
    assert_eq!(text(&r).chars().count(), 200);
}

#[test]
fn an_invoice_number_is_never_reused_even_after_cancelling() {
    let mut state = CustomerInvoices::default();
    let r = || registration(vec![line(3001, 100, 25)]);
    assert_eq!(register(&state, &r()), Ok(1));
    state.apply(registered(1, r()));
    assert_eq!(register(&state, &r()), Err(DomainError::DuplicateCustomerInvoice));
    state.apply(CustomerInvoiceEvent::CustomerInvoiceCancelled { number: 1, reason: "Fel kund".into(), voucher: Some(voucher(2)) });
    assert_eq!(register(&state, &r()), Err(DomainError::DuplicateCustomerInvoice));
    let mut other = r();
    other.invoice_number = doris_invoicing::supplier_invoices::InvoiceNumber::parse("1018").unwrap();
    assert_eq!(register(&state, &other), Ok(2));
}

#[test]
fn an_invoice_is_paid_once_and_a_reversed_payment_makes_it_unpaid() {
    let mut state = CustomerInvoices::from_events([registered(1, registration(vec![line(3001, 100, 25)]))]);
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(paid(&state, 1).unwrap_err(), DomainError::CustomerInvoiceNotPaid);
    state.apply(CustomerInvoiceEvent::CustomerInvoicePaid { number: 1, date: d("2026-03-20"), account: a(1930), voucher: voucher(2) });
    assert_eq!(unpaid(&state, 1).unwrap_err(), DomainError::CustomerInvoicePaid);
    assert_eq!(paid(&state, 1).unwrap().1, voucher(2));
    assert_eq!(state.get(1).unwrap().status_code(), "paid");
    state.apply(CustomerInvoiceEvent::CustomerInvoicePaymentReversed { number: 1, reason: "Fel".into(), voucher: voucher(3) });
    assert_eq!(state.get(1).unwrap().status, Status::Unpaid);
    assert_eq!(state.get(1).unwrap().vouchers, [voucher(1), voucher(2), voucher(3)]);
    assert_eq!(unpaid(&state, 9).unwrap_err(), DomainError::CustomerInvoiceNotFound);
    state.apply(CustomerInvoiceEvent::CustomerInvoiceCancelled { number: 1, reason: "Fel".into(), voucher: None });
    assert_eq!(paid(&state, 1).unwrap_err(), DomainError::CustomerInvoiceCancelled);
}

#[test]
fn the_next_number_follows_the_highest_plain_number() {
    assert_eq!(next_invoice_number([]), "1");
    assert_eq!(next_invoice_number(["1017", "998", "1016"]), "1018");
    assert_eq!(next_invoice_number(["0017"]), "18");
}

#[test]
fn the_next_number_skips_what_isnt_a_plain_number() {
    assert_eq!(next_invoice_number(["2026-17", "A12", " 5", ""]), "1");
    assert_eq!(next_invoice_number(["9", &"9".repeat(30)]), "10");
}

#[test]
fn stored_events_name_the_customer_invoice() {
    let event = registered(1, registration(vec![line(3001, 100, 25)]));
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "CustomerInvoiceRegistered");
    assert_eq!(json["invoice"]["vat"][0]["vat_rate"], 25);
    assert_eq!(serde_json::from_value::<CustomerInvoiceEvent>(json).unwrap(), event);
}
```

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test customer_invoices_domain`
Expected: compile error, because `customer_invoices` doesn't exist.

- [ ] **Step 3: Add the error variants and their codes**

In `crates/invoicing/src/domain.rs`, add after `InvoiceDateInFuture`:
```rust
    #[error("customer is inactive")]
    CustomerInactive,
    #[error("no such customer invoice")]
    CustomerInvoiceNotFound,
    #[error("this invoice number is already used")]
    DuplicateCustomerInvoice,
    #[error("customer invoice is paid")]
    CustomerInvoicePaid,
    #[error("customer invoice is not paid")]
    CustomerInvoiceNotPaid,
    #[error("customer invoice is cancelled")]
    CustomerInvoiceCancelled,
```
In `crates/server/src/invoicing.rs` `domain_status`:
```rust
        CustomerInactive => Status::failed_precondition("customer_inactive"),
        CustomerInvoiceNotFound => Status::not_found("customer_invoice_not_found"),
        DuplicateCustomerInvoice => Status::already_exists("duplicate_customer_invoice"),
        CustomerInvoicePaid => Status::failed_precondition("customer_invoice_paid"),
        CustomerInvoiceNotPaid => Status::failed_precondition("customer_invoice_not_paid"),
        CustomerInvoiceCancelled => Status::failed_precondition("customer_invoice_cancelled"),
```

- [ ] **Step 4: Write `crates/invoicing/src/customer_invoices.rs`**

```rust
//! Pure rules for customer invoices (kundfakturor): the values, the events,
//! the state and the vouchers Doris books for them. No I/O.

use crate::domain::{CustomerDetails, DomainError, Email, Party, PartyName, VatNumber, optional};
use crate::invoices::{self, InvoiceKind, Side, Status};
use crate::supplier_invoices::{InvoiceNumber, PaymentReference};
use crate::vat::{self, InvoiceLine, VatAmount, VatRate};
use doris_company::domain::{AccountingMethod, Address, OrgNr};
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, Attachment, VoucherLine};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACCOUNTS_RECEIVABLE: u32 = 1510;

fn account(number: u32) -> AccountNumber {
    AccountNumber::parse(number).expect("a BAS account")
}

/// Where output VAT goes: 2611, 2621 or 2631; nothing at 0 %.
pub fn output_vat_account(rate: VatRate) -> Option<AccountNumber> {
    match rate {
        VatRate::Rate25 => Some(account(2611)),
        VatRate::Rate12 => Some(account(2621)),
        VatRate::Rate6 => Some(account(2631)),
        VatRate::Rate0 => None,
    }
}

/// Customer invoices' transition errors.
pub struct CustomerInvoiceKind;

impl InvoiceKind for CustomerInvoiceKind {
    const PAID: DomainError = DomainError::CustomerInvoicePaid;
    const NOT_PAID: DomainError = DomainError::CustomerInvoiceNotPaid;
    const CANCELLED: DomainError = DomainError::CustomerInvoiceCancelled;
}

/// The customer as it was when the invoice was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerSnapshot {
    pub number: u32,
    pub name: PartyName,
    pub org_nr: Option<OrgNr>,
    pub vat_number: Option<VatNumber>,
    pub address: Address,
    pub email: Option<Email>,
}

impl CustomerSnapshot {
    pub fn of(customer: &Party<CustomerDetails>) -> Result<Self, DomainError> {
        if !customer.active {
            return Err(DomainError::CustomerInactive);
        }
        let d = &customer.details;
        Ok(Self {
            number: customer.number,
            name: d.name.clone(),
            org_nr: d.org_nr.clone(),
            vat_number: d.vat_number.clone(),
            address: d.address.clone(),
            email: d.email.clone(),
        })
    }
}

/// A customer invoice as typed in, before it has a number. Its VAT is
/// always computed.
pub struct NewCustomerInvoice<'a> {
    pub invoice_number: &'a str,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: &'a str,
    pub lines: Vec<InvoiceLine>,
}

/// What a registered customer invoice says. Never changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerRegistration {
    pub customer: CustomerSnapshot,
    pub invoice_number: InvoiceNumber,
    pub invoice_date: Date,
    pub due_date: Date,
    pub reference: Option<PaymentReference>,
    pub lines: Vec<InvoiceLine>,
    pub vat: Vec<VatAmount>,
    pub total: i64,
}

impl CustomerRegistration {
    pub fn new(customer: CustomerSnapshot, new: &NewCustomerInvoice) -> Result<Self, DomainError> {
        let invoice_number = InvoiceNumber::parse(new.invoice_number)?;
        if new.due_date < new.invoice_date {
            return Err(DomainError::InvalidDueDate);
        }
        let reference = optional(new.reference, PaymentReference::parse)?;
        vat::check_lines(&new.lines)?;
        let vat = vat::by_rate(&new.lines);
        let total = vat::net(&new.lines) + vat.iter().map(|v| v.amount).sum::<i64>();
        Ok(Self {
            customer,
            invoice_number,
            invoice_date: new.invoice_date,
            due_date: new.due_date,
            reference,
            lines: new.lines.clone(),
            vat,
            total,
        })
    }

    pub fn vat_total(&self) -> i64 {
        self.vat.iter().map(|v| v.amount).sum()
    }
}

// The registration carries the whole invoice; events are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustomerInvoiceEvent {
    CustomerInvoiceRegistered {
        number: u32,
        invoice: CustomerRegistration,
        attachments: Vec<Attachment>,
        /// `None` under kontantmetoden: nothing is booked until payment.
        voucher: Option<VoucherRef>,
    },
    CustomerInvoicePaid {
        number: u32,
        date: Date,
        account: AccountNumber,
        voucher: VoucherRef,
    },
    CustomerInvoiceCancelled {
        number: u32,
        reason: String,
        voucher: Option<VoucherRef>,
    },
    CustomerInvoicePaymentReversed {
        number: u32,
        reason: String,
        voucher: VoucherRef,
    },
}

impl CustomerInvoiceEvent {
    pub fn number(&self) -> u32 {
        match self {
            Self::CustomerInvoiceRegistered { number, .. }
            | Self::CustomerInvoicePaid { number, .. }
            | Self::CustomerInvoiceCancelled { number, .. }
            | Self::CustomerInvoicePaymentReversed { number, .. } => *number,
        }
    }
}

/// A customer invoice and what has happened to it. Also the projection's
/// `details` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerInvoice {
    pub number: u32,
    pub invoice: CustomerRegistration,
    pub attachments: Vec<Attachment>,
    pub status: Status,
    pub vouchers: Vec<VoucherRef>,
    pub registration_voucher: Option<VoucherRef>,
}

impl CustomerInvoice {
    pub fn registered(
        number: u32,
        invoice: CustomerRegistration,
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
    pub fn apply(&mut self, event: &CustomerInvoiceEvent) {
        match event {
            CustomerInvoiceEvent::CustomerInvoiceRegistered { .. } => {}
            CustomerInvoiceEvent::CustomerInvoicePaid { date, account, voucher, .. } => {
                self.status = Status::Paid { date: *date, account: *account, voucher: *voucher };
                self.vouchers.push(*voucher);
            }
            CustomerInvoiceEvent::CustomerInvoiceCancelled { voucher, .. } => {
                self.status = Status::Cancelled;
                self.vouchers.extend(*voucher);
            }
            CustomerInvoiceEvent::CustomerInvoicePaymentReversed { voucher, .. } => {
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

/// One company's customer invoices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomerInvoices {
    invoices: BTreeMap<u32, CustomerInvoice>,
}

impl CustomerInvoices {
    pub fn from_events(events: impl IntoIterator<Item = CustomerInvoiceEvent>) -> Self {
        let mut state = Self::default();
        events.into_iter().for_each(|e| state.apply(e));
        state
    }

    pub fn apply(&mut self, event: CustomerInvoiceEvent) {
        match event {
            CustomerInvoiceEvent::CustomerInvoiceRegistered { number, invoice, attachments, voucher } => {
                self.invoices.insert(number, CustomerInvoice::registered(number, invoice, attachments, voucher));
            }
            other => {
                if let Some(invoice) = self.invoices.get_mut(&other.number()) {
                    invoice.apply(&other);
                }
            }
        }
    }

    pub fn get(&self, number: u32) -> Option<&CustomerInvoice> {
        self.invoices.get(&number)
    }

    pub fn next_number(&self) -> u32 {
        self.invoices.keys().next_back().map_or(1, |n| n + 1)
    }
}

/// The new invoice's internal number. An invoice number already used,
/// cancelled or not, is refused: an issued number is never reused.
pub fn register(state: &CustomerInvoices, invoice: &CustomerRegistration) -> Result<u32, DomainError> {
    if state.invoices.values().any(|e| e.invoice.invoice_number == invoice.invoice_number) {
        return Err(DomainError::DuplicateCustomerInvoice);
    }
    Ok(state.next_number())
}

pub fn unpaid(state: &CustomerInvoices, number: u32) -> Result<&CustomerInvoice, DomainError> {
    let invoice = state.get(number).ok_or(DomainError::CustomerInvoiceNotFound)?;
    invoices::check_unpaid::<CustomerInvoiceKind>(&invoice.status)?;
    Ok(invoice)
}

pub fn paid(state: &CustomerInvoices, number: u32) -> Result<(&CustomerInvoice, VoucherRef), DomainError> {
    let invoice = state.get(number).ok_or(DomainError::CustomerInvoiceNotFound)?;
    let voucher = invoices::check_paid::<CustomerInvoiceKind>(&invoice.status)?;
    Ok((invoice, voucher))
}

/// The highest invoice number made only of digits, plus one; "1" if there
/// is none. A number too long for u64 counts as not a plain number.
pub fn next_invoice_number<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    existing
        .into_iter()
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|n| n.parse::<u64>().ok())
        .max()
        .map_or_else(|| "1".to_owned(), |n| n.saturating_add(1).to_string())
}

/// "Kundfaktura 1017, Kund AB", cut to a voucher text's 200 characters.
pub fn text(invoice: &CustomerRegistration) -> String {
    invoices::voucher_text(format!(
        "Kundfaktura {}, {}",
        invoice.invoice_number.as_str(),
        invoice.customer.name.as_str()
    ))
}

/// Each line's net and the output VAT per rate, credited.
fn income_side(invoice: &CustomerRegistration) -> Vec<VoucherLine> {
    let vat: Vec<(AccountNumber, i64)> = invoice
        .vat
        .iter()
        .filter_map(|v| output_vat_account(v.vat_rate).map(|a| (a, v.amount)))
        .collect();
    invoices::posting(&invoice.lines, &vat, Side::Credit)
}

/// Faktureringsmetoden, at registration: 1510 against income and VAT.
pub fn registration_lines(invoice: &CustomerRegistration) -> Vec<VoucherLine> {
    let mut lines = vec![invoices::entry(account(ACCOUNTS_RECEIVABLE), invoice.total, Side::Debit)];
    lines.extend(income_side(invoice));
    lines
}

/// At payment: the bank against 1510 under faktureringsmetoden, against
/// income and VAT under kontantmetoden.
pub fn payment_lines(
    invoice: &CustomerRegistration,
    method: AccountingMethod,
    paid_into: AccountNumber,
) -> Vec<VoucherLine> {
    let mut lines = vec![invoices::entry(paid_into, invoice.total, Side::Debit)];
    match method {
        AccountingMethod::Invoice => {
            lines.push(invoices::entry(account(ACCOUNTS_RECEIVABLE), invoice.total, Side::Credit))
        }
        AccountingMethod::Cash => lines.extend(income_side(invoice)),
    }
    lines
}
```
Add `pub mod customer_invoices;` to `lib.rs`. If `domain::optional`, `Email` or `VatNumber` aren't reachable, they are `pub` / `pub(crate)` already (`optional` became `pub(crate)` in plan 14).

- [ ] **Step 5: Run the tests and the lints**

Run: `cargo fmt -p doris-invoicing -p doris-server && cargo test -p doris-invoicing`
Expected: PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/invoicing crates/server/src/invoicing.rs
git commit -m "Decide customer invoices and the vouchers they book"
```

---

### Task 3: Storing customer invoices

**Files:**
- Create: `migrations/0012_customer_invoices.sql`
- Create: `crates/invoicing/src/customer_invoice_store.rs`
- Modify: `crates/invoicing/src/projections.rs`, `crates/invoicing/src/lib.rs`
- Test: `crates/invoicing/tests/customer_invoices.rs`

**Interfaces:**
- Consumes: Tasks 1–2, plus `lib.rs`'s `load`, `append`, `link_all`, `correct`, `ledger` and `store_all`. These are private, but a child module of the crate root can use them.
- Produces (re-exported from `doris_invoicing`):
  ```rust
  pub async fn register_customer_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, customer: u32, new: NewCustomerInvoice<'_>, attachments: Vec<NewAttachment>, today: Date) -> Result<u32>
  pub async fn pay_customer_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, date: Date, account: u32, today: Date) -> Result<()>
  pub async fn cancel_customer_invoice(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, reason: &str, today: Date) -> Result<()>
  pub async fn reverse_customer_invoice_payment(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, reason: &str, today: Date) -> Result<()>
  pub async fn list_customer_invoices(pool: &SqlitePool, company_id: Uuid, actor: Uuid) -> Result<(Vec<CustomerInvoice>, String)>   // newest first, next invoice number
  pub async fn customer_invoice_attachment(pool: &SqlitePool, company_id: Uuid, actor: Uuid, number: u32, sha256: &str) -> Result<(Attachment, Vec<u8>)>
  ```

- [ ] **Step 1: Write the failing store tests in `crates/invoicing/tests/customer_invoices.rs`**

```rust
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::customer_invoices::NewCustomerInvoice;
use doris_invoicing::domain::{CustomerForm, DomainError};
use doris_invoicing::invoices::Status;
use doris_invoicing::vat::InvoiceLine;
use doris_invoicing::{
    Error, add_customer, cancel_customer_invoice, customer_invoice_attachment, list_customer_invoices,
    pay_customer_invoice, rebuild_projections, register_customer_invoice, reverse_customer_invoice_payment,
    set_customer_active,
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
    add_customer(
        pool,
        id,
        owner,
        &CustomerForm {
            name: "Kund AB",
            org_nr: "556036-0793",
            vat_number: "",
            street: "",
            postal_code: "",
            city: "Stockholm",
            email: "",
            payment_terms: 30,
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

fn invoice(number: &str) -> NewCustomerInvoice<'_> {
    NewCustomerInvoice {
        invoice_number: number,
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines: vec![InvoiceLine::new(3001, 80_000, 25).unwrap()],
    }
}

fn pdf(name: &str) -> NewAttachment {
    NewAttachment { file_name: name.into(), data: format!("%PDF-1.7\n{name}").into_bytes() }
}

async fn register(pool: &SqlitePool, id: Uuid, anna: Uuid, number: &str) -> Result<u32, Error> {
    register_customer_invoice(pool, id, anna, 1, invoice(number), vec![pdf("faktura.pdf")], d(TODAY)).await
}

async fn pay(pool: &SqlitePool, id: Uuid, anna: Uuid, number: u32) -> Result<(), Error> {
    pay_customer_invoice(pool, id, anna, number, d("2026-03-20"), 1930, d(TODAY)).await
}

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
async fn faktureringsmetoden_books_the_invoice_and_the_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;

    assert_eq!(register(&pool, id, anna, "1017").await.unwrap(), 1);
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked[0].text, "Kundfaktura 1017, Kund AB");
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 1510).await, 100_000);
    assert_eq!(balance(&pool, id, anna, 2611).await, -20_000);
    assert_eq!(balance(&pool, id, anna, 3001).await, -80_000);

    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
    assert_eq!(balance(&pool, id, anna, 1930).await, 100_000);
    let (listed, next) = list_customer_invoices(&pool, id, anna).await.unwrap();
    assert!(matches!(listed[0].status, Status::Paid { .. }));
    assert_eq!(next, "1018");
}

#[tokio::test]
async fn kontantmetoden_books_only_the_payment_with_the_underlag() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "1").await.unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());

    pay(&pool, id, anna, 1).await.unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 1930).await, 100_000);
    assert_eq!(balance(&pool, id, anna, 2611).await, -20_000);
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
}

#[tokio::test]
async fn a_cancelled_invoice_number_is_never_reused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1017").await.unwrap();
    cancel_customer_invoice(&pool, id, anna, 1, "Fel kund", d(TODAY)).await.unwrap();
    assert_eq!(vouchers(&pool, id, anna).await[1].corrects, Some(1));
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);

    let err = register(&pool, id, anna, "1017").await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::DuplicateCustomerInvoice)));
    let (listed, next) = list_customer_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].status, Status::Cancelled);
    assert_eq!(next, "1018");
}

#[tokio::test]
async fn a_payment_is_reversed_and_paid_again() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    reverse_customer_invoice_payment(&pool, id, anna, 1, "Fel konto", d(TODAY)).await.unwrap();
    assert_eq!(list_customer_invoices(&pool, id, anna).await.unwrap().0[0].status, Status::Unpaid);
    assert_eq!(balance(&pool, id, anna, 1510).await, 100_000);
    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
}

#[tokio::test]
async fn hand_corrections_are_followed() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    doris_ledger::correct_voucher(&pool, id, anna, d("2026-01-01"), 1, d(TODAY), d(TODAY)).await.unwrap();
    cancel_customer_invoice(&pool, id, anna, 1, "Rättad i grundboken", d(TODAY)).await.unwrap();
    assert_eq!(vouchers(&pool, id, anna).await.len(), 2);
    assert_eq!(list_customer_invoices(&pool, id, anna).await.unwrap().0[0].status, Status::Cancelled);
}

#[tokio::test]
async fn wrong_transitions_and_inputs_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    let err = register_customer_invoice(&pool, id, anna, 9, invoice("1"), vec![], d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerNotFound)));
    let future = NewCustomerInvoice { invoice_date: d("2026-10-03"), due_date: d("2026-11-03"), ..invoice("2") };
    let err = register_customer_invoice(&pool, id, anna, 1, future, vec![], d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvoiceDateInFuture)));
    let err = register_customer_invoice(&pool, id, anna, 1, invoice("3"), vec![pdf("a.pdf"), pdf("a.pdf")], d(TODAY))
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::DuplicateAttachment);

    register(&pool, id, anna, "1").await.unwrap();
    let err = reverse_customer_invoice_payment(&pool, id, anna, 1, "Fel", d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerInvoiceNotPaid)));
    pay(&pool, id, anna, 1).await.unwrap();
    let err = cancel_customer_invoice(&pool, id, anna, 1, "Fel", d(TODAY)).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerInvoicePaid)));
    let err = pay(&pool, id, anna, 9).await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerInvoiceNotFound)));

    set_customer_active(&pool, id, anna, 1, false).await.unwrap();
    let err = register(&pool, id, anna, "4").await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerInactive)));
}

#[tokio::test]
async fn a_rejected_registration_leaves_nothing_and_uses_up_no_number() {
    for method in [AccountingMethod::Invoice, AccountingMethod::Cash] {
        let (pool, anna, id) = setup(method).await;
        doris_ledger::set_account_active(&pool, id, anna, 3001, false).await.unwrap();
        let err = register(&pool, id, anna, "1").await.unwrap_err();
        assert_eq!(ledger_error(err), LedgerError::AccountInactive, "{method:?}");
        assert!(list_customer_invoices(&pool, id, anna).await.unwrap().0.is_empty());
        doris_ledger::set_account_active(&pool, id, anna, 3001, true).await.unwrap();
        assert_eq!(register(&pool, id, anna, "1").await.unwrap(), 1);
    }
}

#[tokio::test]
async fn a_closed_year_takes_no_invoice() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), d(TODAY)).await.unwrap();
    let old = NewCustomerInvoice { invoice_date: d("2025-06-01"), due_date: d("2025-07-01"), ..invoice("1") };
    let err = register_customer_invoice(&pool, id, anna, 1, old, vec![], d(TODAY)).await.unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::FiscalYearClosed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_registrations_of_the_same_number_give_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("customers.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", AccountingMethod::Invoice).await;
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { register(&pool, id, anna, "1017").await })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(Error::Domain(DomainError::DuplicateCustomerInvoice)) => {}
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(ok, 1);
}

#[tokio::test]
async fn the_projection_rebuilds_from_the_events() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    register(&pool, id, anna, "2").await.unwrap();
    cancel_customer_invoice(&pool, id, anna, 2, "Fel", d(TODAY)).await.unwrap();
    let sql = "SELECT company_id || number || customer_number || invoice_number || status || details FROM customer_invoices ORDER BY number";
    let rows = || async { sqlx::query_scalar::<_, String>(sql).fetch_all(&pool).await.unwrap() };
    let before = rows().await;
    rebuild_projections(&pool).await.unwrap();
    assert_eq!(rows().await, before);
    assert_eq!(before.len(), 2);
}

#[tokio::test]
async fn an_underlag_is_read_only_through_the_companys_own_invoice() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "1").await.unwrap();
    let sha = list_customer_invoices(&pool, id, anna).await.unwrap().0[0].attachments[0].sha256.clone();
    let (_, data) = customer_invoice_attachment(&pool, id, anna, 1, &sha).await.unwrap();
    assert_eq!(data, pdf("faktura.pdf").data);
    let err = customer_invoice_attachment(&pool, id, anna, 2, &sha).await.unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AttachmentNotFound);
    let err = customer_invoice_attachment(&pool, id, Uuid::new_v4(), 1, &sha).await.unwrap_err();
    assert!(matches!(err, Error::NotFound));
}

#[tokio::test]
async fn an_empty_register_proposes_number_1_and_strangers_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    assert_eq!(list_customer_invoices(&pool, id, anna).await.unwrap().1, "1");
    assert!(matches!(list_customer_invoices(&pool, id, Uuid::new_v4()).await, Err(Error::NotFound)));
}
```

- [ ] **Step 2: Run them and check that they fail**

Run: `cargo test -p doris-invoicing --test customer_invoices`
Expected: compile errors, because the store functions don't exist.

- [ ] **Step 3: Write the migration `migrations/0012_customer_invoices.sql`**

```sql
-- Customer invoices: the projection of the customer-invoices-{company}
-- streams. `details` is the invoice as listed (JSON).
CREATE TABLE customer_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    customer_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    details         TEXT    NOT NULL,
    PRIMARY KEY (company_id, number)
);

-- An issued invoice number is never reused, not even after cancelling.
CREATE UNIQUE INDEX customer_invoices_unique_number
    ON customer_invoices (company_id, invoice_number);
```

- [ ] **Step 4: Project customer invoices in `crates/invoicing/src/projections.rs`**

- Import `use crate::customer_invoices::{CustomerInvoice, CustomerInvoiceEvent};` and add `CUSTOMER_INVOICES_STREAM` to the `use crate::{…}` list.
- In `apply`, before the final `Ok(())`:
  ```rust
      if let Some(company_id) = event.stream_id.strip_prefix(CUSTOMER_INVOICES_STREAM) {
          return apply_customer_invoice(conn, company_id, event.decode()?).await;
      }
  ```
- Add:
  ```rust
  async fn apply_customer_invoice(
      conn: &mut SqliteConnection,
      company_id: &str,
      event: CustomerInvoiceEvent,
  ) -> crate::Result<()> {
      if let CustomerInvoiceEvent::CustomerInvoiceRegistered { number, invoice, attachments, voucher } = event {
          let invoice = CustomerInvoice::registered(number, invoice, attachments, voucher);
          sqlx::query(
              "INSERT INTO customer_invoices
               (company_id, number, customer_number, invoice_number, status, details)
               VALUES (?, ?, ?, ?, ?, ?)",
          )
          .bind(company_id)
          .bind(number)
          .bind(invoice.invoice.customer.number)
          .bind(invoice.invoice.invoice_number.as_str())
          .bind(invoice.status_code())
          .bind(serde_json::to_string(&invoice)?)
          .execute(&mut *conn)
          .await?;
          return Ok(());
      }
      let details: String =
          sqlx::query_scalar("SELECT details FROM customer_invoices WHERE company_id = ? AND number = ?")
              .bind(company_id)
              .bind(event.number())
              .fetch_one(&mut *conn)
              .await?;
      let mut invoice: CustomerInvoice = serde_json::from_str(&details)?;
      invoice.apply(&event);
      sqlx::query("UPDATE customer_invoices SET status = ?, details = ? WHERE company_id = ? AND number = ?")
          .bind(invoice.status_code())
          .bind(serde_json::to_string(&invoice)?)
          .bind(company_id)
          .bind(event.number())
          .execute(&mut *conn)
          .await?;
      Ok(())
  }
  ```
- In `rebuild_projections`, also run `DELETE FROM customer_invoices` before replaying.

- [ ] **Step 5: Write `crates/invoicing/src/customer_invoice_store.rs`**

```rust
//! Customer invoices in the database. Each write books its vouchers and
//! appends its event in one IMMEDIATE transaction.

use crate::customer_invoices::{
    self, CustomerInvoice, CustomerInvoiceEvent, CustomerInvoices, CustomerRegistration,
    CustomerSnapshot, NewCustomerInvoice,
};
use crate::domain::{CustomerEvent, DomainError, Register};
use crate::supplier_invoices::{payment_account, reason};
use crate::{
    CUSTOMER_INVOICES_STREAM, CUSTOMERS_STREAM, Result, append, correct, ledger, link_all, load,
    store_all,
};
use doris_company::domain::AccountingMethod;
use doris_ledger::NewAttachment;
use doris_ledger::domain::{Attachment, DomainError as LedgerError, RecordVoucher};
use jiff::civil::Date;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

fn stream(company_id: Uuid) -> String {
    format!("{CUSTOMER_INVOICES_STREAM}{company_id}")
}

async fn load_invoices(conn: &mut SqliteConnection, company_id: Uuid) -> Result<(CustomerInvoices, i64)> {
    let (events, version) = load::<CustomerInvoiceEvent>(conn, &stream(company_id)).await?;
    Ok((CustomerInvoices::from_events(events), version))
}

/// Registers a customer invoice and, under faktureringsmetoden, books it
/// with its underlag, all in one transaction.
pub async fn register_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    customer: u32,
    new: NewCustomerInvoice<'_>,
    attachments: Vec<NewAttachment>,
    today: Date,
) -> Result<u32> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (customer_events, _) =
        load::<CustomerEvent>(&mut tx, &format!("{CUSTOMERS_STREAM}{company_id}")).await?;
    let customers = Register::from_changes(customer_events.into_iter().map(Into::into));
    let customer = customers.get(customer).ok_or(DomainError::CustomerNotFound)?;
    let invoice = CustomerRegistration::new(CustomerSnapshot::of(customer)?, &new)?;
    if invoice.invoice_date > today {
        return Err(DomainError::InvoiceDateInFuture.into());
    }
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let number = customer_invoices::register(&state, &invoice)?;
    let stored = store_all(&mut tx, attachments).await?;
    let voucher = match company.accounting_method {
        AccountingMethod::Invoice => {
            let booked = doris_ledger::record_voucher_in(
                &mut tx,
                company_id,
                actor,
                RecordVoucher {
                    date: invoice.invoice_date,
                    text: customer_invoices::text(&invoice),
                    lines: customer_invoices::registration_lines(&invoice),
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
    let event = CustomerInvoiceEvent::CustomerInvoiceRegistered { number, invoice, attachments: stored, voucher };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(number)
}

/// Books the payment of an unpaid invoice. Under kontantmetoden that is the
/// income and VAT, and the underlag go on the payment voucher.
pub async fn pay_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    date: Date,
    account: u32,
    today: Date,
) -> Result<()> {
    let account = payment_account(account)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = customer_invoices::unpaid(&state, number)?;
    let voucher = doris_ledger::record_voucher_in(
        &mut tx,
        company_id,
        actor,
        RecordVoucher {
            date,
            text: customer_invoices::text(&invoice.invoice),
            lines: customer_invoices::payment_lines(&invoice.invoice, company.accounting_method, account),
        },
        today,
    )
    .await?;
    if company.accounting_method == AccountingMethod::Cash {
        link_all(&mut tx, company_id, actor, voucher, &invoice.attachments, today).await?;
    }
    let event = CustomerInvoiceEvent::CustomerInvoicePaid { number, date, account, voucher };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Cancels an unpaid invoice; under faktureringsmetoden its registration
/// voucher is corrected. Its number stays used.
pub async fn cancel_customer_invoice(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason_text: &str,
    today: Date,
) -> Result<()> {
    let reason = reason(reason_text)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let invoice = customer_invoices::unpaid(&state, number)?;
    let voucher = match invoice.registration_voucher {
        Some(registered) => Some(correct(&mut tx, &company, actor, registered, today).await?),
        None => None,
    };
    let event = CustomerInvoiceEvent::CustomerInvoiceCancelled { number, reason, voucher };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

/// Corrects the payment voucher of a paid invoice, which is unpaid again.
pub async fn reverse_customer_invoice_payment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    reason_text: &str,
    today: Date,
) -> Result<()> {
    let reason = reason(reason_text)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let company = doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let (state, version) = load_invoices(&mut tx, company_id).await?;
    let (_, payment) = customer_invoices::paid(&state, number)?;
    let voucher = correct(&mut tx, &company, actor, payment, today).await?;
    let event = CustomerInvoiceEvent::CustomerInvoicePaymentReversed { number, reason, voucher };
    append(&mut tx, &stream(company_id), version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of invoices.
/// The company's customer invoices, newest first, and the next invoice
/// number to propose.
pub async fn list_customer_invoices(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<(Vec<CustomerInvoice>, String)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT invoice_number, details FROM customer_invoices WHERE company_id = ? ORDER BY number DESC",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    let next = customer_invoices::next_invoice_number(rows.iter().map(|(n, _)| n.as_str()));
    let invoices = rows
        .iter()
        .map(|(_, details)| Ok(serde_json::from_str(details)?))
        .collect::<Result<Vec<_>>>()?;
    Ok((invoices, next))
}

/// An underlag of the company's own invoice `number`: found in that invoice
/// first, never by its hash alone.
pub async fn customer_invoice_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, actor).await?;
    let details: Option<String> =
        sqlx::query_scalar("SELECT details FROM customer_invoices WHERE company_id = ? AND number = ?")
            .bind(company_id.to_string())
            .bind(number)
            .fetch_optional(pool)
            .await?;
    let invoice: Option<CustomerInvoice> = details.map(|d| serde_json::from_str(&d)).transpose()?;
    let attachment = invoice
        .and_then(|i| i.attachments.into_iter().find(|a| a.sha256 == sha256))
        .ok_or_else(|| ledger(LedgerError::AttachmentNotFound))?;
    let data: Vec<u8> = sqlx::query_scalar("SELECT data FROM attachment_files WHERE sha256 = ?")
        .bind(&attachment.sha256)
        .fetch_one(pool)
        .await?;
    Ok((attachment, data))
}
```

In `lib.rs`:
- Add `const CUSTOMER_INVOICES_STREAM: &str = "customer-invoices-";`.
- Add `pub mod customer_invoices;` (if Task 2 hasn't) and `mod customer_invoice_store;`.
- Add:
  ```rust
  pub use customer_invoice_store::{
      cancel_customer_invoice, customer_invoice_attachment, list_customer_invoices,
      pay_customer_invoice, register_customer_invoice, reverse_customer_invoice_payment,
  };
  ```

- [ ] **Step 6: Run the tests and the lints**

Run: `cargo fmt -p doris-invoicing && cargo test -p doris-invoicing && cargo test -p doris-ledger --test stress`
Expected: PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add migrations/0012_customer_invoices.sql crates/invoicing
git commit -m "Register, pay, cancel and reverse customer invoices with their vouchers"
```

---

### Task 4: Customer invoices over gRPC-Web

**Files:**
- Modify: `proto/doris/invoicing/v1/invoicing.proto`, `crates/server/src/invoicing.rs`
- Test: `crates/server/tests/customer_invoices.rs`

**Interfaces:**
- Consumes: Task 3. Also `crate::ledger::{date, new_attachments, attachment_message}` and `crate::grpc::today`, which already exist.
- Produces: six RPCs, plus the messages `VatAmount`, `CustomerInvoice`, `ListCustomerInvoicesResponse { invoices, cash_method, next_invoice_number }` and the request/response messages from the spec.

- [ ] **Step 1: Extend the proto**

Add to `service InvoicingService`:
```proto
  rpc ListCustomerInvoices(ListCustomerInvoicesRequest) returns (ListCustomerInvoicesResponse);
  rpc RegisterCustomerInvoice(RegisterCustomerInvoiceRequest) returns (RegisterCustomerInvoiceResponse);
  rpc PayCustomerInvoice(PayCustomerInvoiceRequest) returns (PayCustomerInvoiceResponse);
  rpc CancelCustomerInvoice(CancelCustomerInvoiceRequest) returns (CancelCustomerInvoiceResponse);
  rpc ReverseCustomerInvoicePayment(ReverseCustomerInvoicePaymentRequest) returns (ReverseCustomerInvoicePaymentResponse);
  rpc GetCustomerInvoiceAttachment(GetCustomerInvoiceAttachmentRequest) returns (GetCustomerInvoiceAttachmentResponse);
```
Append:
```proto
// VAT at one rate (25, 12 or 6) on a customer invoice.
message VatAmount {
  uint32 vat_rate = 1;
  int64 amount = 2;
}

message CustomerInvoice {
  uint32 number = 1;
  uint32 customer_number = 2;
  string customer_name = 3;
  string invoice_number = 4;
  string invoice_date = 5;
  string due_date = 6;
  string reference = 7;
  repeated InvoiceLine lines = 8;
  repeated VatAmount vat = 9;
  int64 total = 10;
  string status = 11;                 // "unpaid", "paid" or "cancelled"
  string paid_date = 12;              // "" unless paid
  repeated VoucherRef vouchers = 13;
  repeated doris.ledger.v1.Attachment attachments = 14;
}

message ListCustomerInvoicesRequest { string company_id = 1; }
message ListCustomerInvoicesResponse {
  repeated CustomerInvoice invoices = 1;  // newest first
  bool cash_method = 2;
  string next_invoice_number = 3;         // the proposal for a new invoice
}
message RegisterCustomerInvoiceRequest {
  string company_id = 1;
  uint32 customer_number = 2;
  string invoice_number = 3;
  string invoice_date = 4;
  string due_date = 5;
  string reference = 6;
  repeated InvoiceLine lines = 7;
  repeated doris.ledger.v1.NewAttachment attachments = 8;
}
message RegisterCustomerInvoiceResponse { uint32 number = 1; }
message PayCustomerInvoiceRequest { string company_id = 1; uint32 number = 2; string date = 3; uint32 account = 4; }
message PayCustomerInvoiceResponse {}
message CancelCustomerInvoiceRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message CancelCustomerInvoiceResponse {}
message ReverseCustomerInvoicePaymentRequest { string company_id = 1; uint32 number = 2; string reason = 3; }
message ReverseCustomerInvoicePaymentResponse {}
message GetCustomerInvoiceAttachmentRequest { string company_id = 1; uint32 number = 2; string sha256 = 3; }
message GetCustomerInvoiceAttachmentResponse { doris.ledger.v1.Attachment attachment = 1; bytes data = 2; }
```
Run: `cargo build -p doris-proto --features server`
Expected: it builds. Then `doris-server` fails on the missing trait methods, which is expected.

- [ ] **Step 2: Write the failing server tests in `crates/server/tests/customer_invoices.rs`**

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

/// Anna's company (first year 2026) with customer 1, "Kund AB".
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
        .add_customer(authed(
            pb::AddCustomerRequest {
                company_id: id.clone(),
                details: Some(pb::CustomerDetails { name: "Kund AB".into(), payment_terms: 30, ..Default::default() }),
            },
            session,
        ))
        .await
        .unwrap();
    id
}

fn request(company_id: &str, invoice_number: &str) -> pb::RegisterCustomerInvoiceRequest {
    pb::RegisterCustomerInvoiceRequest {
        company_id: company_id.into(),
        customer_number: 1,
        invoice_number: invoice_number.into(),
        invoice_date: "2026-01-15".into(),
        due_date: "2026-02-14".into(),
        reference: "".into(),
        lines: vec![
            pb::InvoiceLine { account: 3001, net: 80_000, vat_rate: 25 },
            pb::InvoiceLine { account: 3002, net: 10_000, vat_rate: 12 },
        ],
        attachments: vec![lpb::NewAttachment { file_name: "faktura.pdf".into(), data: b"%PDF-1.7\nfaktura".to_vec() }],
    }
}

async fn list(api: &mut Invoicing, id: &str, session: &str) -> pb::ListCustomerInvoicesResponse {
    api.list_customer_invoices(authed(pb::ListCustomerInvoicesRequest { company_id: id.into() }, session))
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
    assert_eq!(list(&mut api, &id, &anna).await.next_invoice_number, "1");

    let number = api.register_customer_invoice(authed(request(&id, "1017"), &anna)).await.unwrap().into_inner().number;
    api.pay_customer_invoice(authed(
        pb::PayCustomerInvoiceRequest { company_id: id.clone(), number, date: "2026-01-20".into(), account: 1930 },
        &anna,
    ))
    .await
    .unwrap();

    let listed = list(&mut api, &id, &anna).await;
    assert_eq!(listed.next_invoice_number, "1018");
    assert!(!listed.cash_method);
    let invoice = &listed.invoices[0];
    assert_eq!(invoice.invoice_number, "1017");
    assert_eq!(invoice.customer_name, "Kund AB");
    let vat: Vec<_> = invoice.vat.iter().map(|v| (v.vat_rate, v.amount)).collect();
    assert_eq!(vat, [(25, 20_000), (12, 1_200)]);
    assert_eq!(invoice.total, 111_200);
    assert_eq!((invoice.status.as_str(), invoice.paid_date.as_str()), ("paid", "2026-01-20"));
    assert_eq!(invoice.vouchers.len(), 2);

    api.reverse_customer_invoice_payment(authed(
        pb::ReverseCustomerInvoicePaymentRequest { company_id: id.clone(), number, reason: "Fel".into() },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(list(&mut api, &id, &anna).await.invoices[0].status, "unpaid");
    let file = api
        .get_customer_invoice_attachment(authed(
            pb::GetCustomerInvoiceAttachmentRequest { company_id: id.clone(), number, sha256: invoice.attachments[0].id.clone() },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.data, b"%PDF-1.7\nfaktura");
}

#[tokio::test]
async fn bad_requests_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    let ok = || request(&id, "1");

    for (req, code, expected) in [
        (pb::RegisterCustomerInvoiceRequest { customer_number: 9, ..ok() }, Code::NotFound, "customer_not_found"),
        (pb::RegisterCustomerInvoiceRequest { lines: vec![pb::InvoiceLine { account: 1510, net: 100, vat_rate: 25 }], ..ok() }, Code::InvalidArgument, "invalid_invoice_account"),
        (pb::RegisterCustomerInvoiceRequest { invoice_number: "".into(), ..ok() }, Code::InvalidArgument, "invalid_invoice_number"),
        (pb::RegisterCustomerInvoiceRequest { due_date: "2026-01-01".into(), ..ok() }, Code::InvalidArgument, "invalid_due_date"),
    ] {
        let err = api.register_customer_invoice(authed(req, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (code, expected.to_owned()));
    }

    api.register_customer_invoice(authed(ok(), &anna)).await.unwrap();
    api.cancel_customer_invoice(authed(pb::CancelCustomerInvoiceRequest { company_id: id.clone(), number: 1, reason: "Fel".into() }, &anna))
        .await
        .unwrap();
    let err = api.register_customer_invoice(authed(ok(), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::AlreadyExists, "duplicate_customer_invoice".into()));
    let pay = |number| pb::PayCustomerInvoiceRequest { company_id: id.clone(), number, date: "2026-01-20".into(), account: 1930 };
    let err = api.pay_customer_invoice(authed(pay(1), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "customer_invoice_cancelled".into()));
    let err = api.pay_customer_invoice(authed(pay(9), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "customer_invoice_not_found".into()));

    api.register_customer_invoice(authed(request(&id, "2"), &anna)).await.unwrap();
    let err = api
        .reverse_customer_invoice_payment(authed(pb::ReverseCustomerInvoicePaymentRequest { company_id: id.clone(), number: 2, reason: "Fel".into() }, &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "customer_invoice_not_paid".into()));
    api.pay_customer_invoice(authed(pay(2), &anna)).await.unwrap();
    let err = api
        .cancel_customer_invoice(authed(pb::CancelCustomerInvoiceRequest { company_id: id.clone(), number: 2, reason: "Fel".into() }, &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "customer_invoice_paid".into()));

    api.set_customer_active(authed(pb::SetCustomerActiveRequest { company_id: id.clone(), number: 1, active: false }, &anna))
        .await
        .unwrap();
    let err = api.register_customer_invoice(authed(request(&id, "3"), &anna)).await.unwrap_err();
    assert_eq!(code_of(err), (Code::FailedPrecondition, "customer_inactive".into()));
}

#[tokio::test]
async fn kontantmetoden_is_reported_and_others_are_refused() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna, cpb::AccountingMethod::Cash).await;
    let mut api = server.invoicing();
    api.register_customer_invoice(authed(request(&id, "1"), &anna)).await.unwrap();
    let listed = list(&mut api, &id, &anna).await;
    assert!(listed.cash_method);
    assert!(listed.invoices[0].vouchers.is_empty());

    let err = api
        .list_customer_invoices(authed(pb::ListCustomerInvoicesRequest { company_id: id.clone() }, &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api.list_customer_invoices(pb::ListCustomerInvoicesRequest { company_id: id.clone() }).await.unwrap_err();
    assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".into()));
}
```

Run: `cargo test -p doris-server --test customer_invoices`
Expected: compile errors, because the trait methods are missing.

- [ ] **Step 3: Implement the RPCs in `crates/server/src/invoicing.rs`**

Extend the imports with `use doris_invoicing::customer_invoices::{CustomerInvoice, NewCustomerInvoice};`. Add to the trait impl:
```rust
    async fn list_customer_invoices(
        &self,
        request: Request<pb::ListCustomerInvoicesRequest>,
    ) -> Result<Response<pb::ListCustomerInvoicesResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let (invoices, next_invoice_number) = doris_invoicing::list_customer_invoices(&self.pool, company, user)
            .await
            .map_err(status)?;
        let method = doris_company::get_company(&self.pool, company, user)
            .await
            .map_err(|err| status(err.into()))?
            .accounting_method;
        Ok(Response::new(pb::ListCustomerInvoicesResponse {
            invoices: invoices.into_iter().map(customer_invoice_pb).collect(),
            cash_method: method == AccountingMethod::Cash,
            next_invoice_number,
        }))
    }

    async fn register_customer_invoice(
        &self,
        request: Request<pb::RegisterCustomerInvoiceRequest>,
    ) -> Result<Response<pb::RegisterCustomerInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let mut req = request.into_inner();
        let lines = req
            .lines
            .iter()
            .map(|l| InvoiceLine::new(l.account, l.net, l.vat_rate))
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain_status)?;
        let attachments = new_attachments(std::mem::take(&mut req.attachments))?;
        let new = NewCustomerInvoice {
            invoice_number: &req.invoice_number,
            invoice_date: date(&req.invoice_date)?,
            due_date: date(&req.due_date)?,
            reference: &req.reference,
            lines,
        };
        let number = doris_invoicing::register_customer_invoice(
            &self.pool, company, user, req.customer_number, new, attachments, today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::RegisterCustomerInvoiceResponse { number }))
    }

    async fn pay_customer_invoice(
        &self,
        request: Request<pb::PayCustomerInvoiceRequest>,
    ) -> Result<Response<pb::PayCustomerInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::pay_customer_invoice(&self.pool, company, user, req.number, date(&req.date)?, req.account, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::PayCustomerInvoiceResponse {}))
    }

    async fn cancel_customer_invoice(
        &self,
        request: Request<pb::CancelCustomerInvoiceRequest>,
    ) -> Result<Response<pb::CancelCustomerInvoiceResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::cancel_customer_invoice(&self.pool, company, user, req.number, &req.reason, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CancelCustomerInvoiceResponse {}))
    }

    async fn reverse_customer_invoice_payment(
        &self,
        request: Request<pb::ReverseCustomerInvoicePaymentRequest>,
    ) -> Result<Response<pb::ReverseCustomerInvoicePaymentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        doris_invoicing::reverse_customer_invoice_payment(&self.pool, company, user, req.number, &req.reason, today())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ReverseCustomerInvoicePaymentResponse {}))
    }

    async fn get_customer_invoice_attachment(
        &self,
        request: Request<pb::GetCustomerInvoiceAttachmentRequest>,
    ) -> Result<Response<pb::GetCustomerInvoiceAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        let (attachment, data) =
            doris_invoicing::customer_invoice_attachment(&self.pool, company, user, req.number, &req.sha256)
                .await
                .map_err(status)?;
        Ok(Response::new(pb::GetCustomerInvoiceAttachmentResponse {
            attachment: Some(attachment_message(&attachment)),
            data,
        }))
    }
```
Add the conversion:
```rust
fn customer_invoice_pb(i: CustomerInvoice) -> pb::CustomerInvoice {
    let status_code = i.status_code().to_owned();
    let paid_date = match &i.status {
        InvoiceStatus::Paid { date, .. } => date.to_string(),
        _ => String::new(),
    };
    let r = i.invoice;
    pb::CustomerInvoice {
        number: i.number,
        customer_number: r.customer.number,
        customer_name: r.customer.name.as_str().to_owned(),
        invoice_number: r.invoice_number.as_str().to_owned(),
        invoice_date: r.invoice_date.to_string(),
        due_date: r.due_date.to_string(),
        reference: r.reference.map(|x| x.as_str().to_owned()).unwrap_or_default(),
        lines: r
            .lines
            .iter()
            .map(|l| pb::InvoiceLine { account: l.account.get().into(), net: l.net, vat_rate: l.vat_rate.percent() })
            .collect(),
        vat: r.vat.iter().map(|v| pb::VatAmount { vat_rate: v.vat_rate.percent(), amount: v.amount }).collect(),
        total: r.total,
        status: status_code,
        paid_date,
        vouchers: i
            .vouchers
            .iter()
            .map(|v| pb::VoucherRef { fiscal_year_start: v.fiscal_year_start.to_string(), number: v.number })
            .collect(),
        attachments: i.attachments.iter().map(attachment_message).collect(),
    }
}
```
`InvoiceStatus` is the alias plan 14 already imports for `doris_invoicing::supplier_invoices::Status`, which is the same type re-exported.

- [ ] **Step 4: Run all tests and the lints**

Run: `cargo fmt --all && cargo test -p doris-server`
Expected: PASS, including every supplier-invoice server test.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add proto crates/server Cargo.lock
git commit -m "Serve customer invoices over gRPC-Web"
```

---

### Task 5: Shared invoice UI, adopted by the supplier pages

**Files:**
- Create: `crates/web/src/invoice_ui.rs`
- Modify: `crates/web/src/main.rs` (`mod invoice_ui;`), `crates/web/src/pages/new_supplier_invoice.rs`, `crates/web/src/pages/supplier_invoices.rs`
- Test: unit tests in `invoice_ui.rs`. The supplier e2e spec runs unchanged.

**Interfaces:**
- Produces (in `crate::invoice_ui`):
  - `status_label(status: &str, due_date: &str, today: &str) -> &'static str`
  - `preview_vat(&[(i64, u32)]) -> Vec<(u32, i64)>` (per rate, highest first, only rates with VAT > 0)
  - `LineRow`, with `new(u32)`, `preview()` and `request() -> Option<ipb::InvoiceLine>`
  - The components `InvoiceLineRows(rows, next_id, list)`, `PickedFiles(id, files, reading, error, company)`, `PayForm(date, account, confirm, on_confirm)` and `ReasonForm(reason, confirm, on_confirm)`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/web/src/invoice_ui.rs` with only the tests first, and add `mod invoice_ui;` to `crates/web/src/main.rs` next to the other modules:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpaid_invoice_past_its_due_date_is_overdue() {
        assert_eq!(status_label("unpaid", "2026-03-31", "2026-03-31"), "Obetald");
        assert_eq!(status_label("unpaid", "2026-03-31", "2026-04-01"), "Förfallen");
        assert_eq!(status_label("paid", "2026-03-31", "2026-04-01"), "Betald");
        assert_eq!(status_label("cancelled", "2026-03-31", "2026-04-01"), "Makulerad");
    }

    #[test]
    fn the_preview_rounds_vat_per_rate_like_the_server() {
        assert_eq!(preview_vat(&[(33, 25), (33, 25), (33, 25)]), [(25, 25)]);
        assert_eq!(preview_vat(&[(1000, 12), (50, 6), (700, 0)]), [(12, 120), (6, 3)]);
        assert!(preview_vat(&[]).is_empty());
    }
}
```
Run: `cargo test -p doris-web invoice_ui`
Expected: compile errors (`status_label`, `preview_vat` missing).

- [ ] **Step 2: Write the rest of `invoice_ui.rs` above the tests**

```rust
//! Pieces the supplier and customer invoice pages share: the status label,
//! the line editor with its VAT preview, the picked underlag, and the
//! pay and reason forms.

use crate::api::{ipb, lpb};
use crate::attachments::{read_files, size_label};
use crate::errors::describe_code;
use crate::format::parse_amount;
use crate::ui::{Button, FileInput, SELECT, SELECT_OPTION, TextInput, Variant};
use crate::voucher_lines::account_number;
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::collections::BTreeMap;

/// Obetald, Förfallen (unpaid past its due date), Betald or Makulerad.
pub fn status_label(status: &str, due_date: &str, today: &str) -> &'static str {
    match status {
        "paid" => "Betald",
        "cancelled" => "Makulerad",
        _ if due_date < today => "Förfallen",
        _ => "Obetald",
    }
}

/// VAT per rate on that rate's summed net, rounded half up, highest rate
/// first and only rates with VAT: the server's rule, shown while typing.
pub fn preview_vat(lines: &[(i64, u32)]) -> Vec<(u32, i64)> {
    let mut by_rate = BTreeMap::<u32, i64>::new();
    for &(net, rate) in lines {
        *by_rate.entry(rate).or_default() += net;
    }
    by_rate
        .into_iter()
        .rev()
        .map(|(rate, net)| (rate, (net * i64::from(rate) + 50) / 100))
        .filter(|&(_, vat)| vat > 0)
        .collect()
}

/// One line in the editor: Konto, Belopp exkl. moms, Momssats.
#[derive(Clone, Copy)]
pub struct LineRow {
    pub id: u32,
    pub account: RwSignal<String>,
    pub net: RwSignal<String>,
    pub rate: RwSignal<String>,
}

impl LineRow {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            account: RwSignal::new(String::new()),
            net: RwSignal::new(String::new()),
            rate: RwSignal::new("25".into()),
        }
    }

    /// (net in öre, rate) as typed; an unreadable amount counts as 0.
    pub fn preview(&self) -> (i64, u32) {
        (parse_amount(&self.net.get()).unwrap_or(0), self.rate.get().parse().unwrap_or(25))
    }

    pub fn request(&self) -> Option<ipb::InvoiceLine> {
        Some(ipb::InvoiceLine {
            account: account_number(&self.account.get_untracked()),
            net: parse_amount(&self.net.get_untracked())?,
            vat_rate: self.rate.get_untracked().parse().unwrap_or(25),
        })
    }
}

/// The line editor. `list` is the id of the page's account `<datalist>`.
#[component]
pub fn InvoiceLineRows(rows: RwSignal<Vec<LineRow>>, next_id: StoredValue<u32>, list: &'static str) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2 text-muted-foreground">
                <span>"Konto"</span>
                <span>"Belopp exkl. moms"</span>
                <span>"Moms"</span>
                <span></span>
            </div>
            <For each=move || { rows.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, r)| (*i, r.id) let((index, row))>
                <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2">
                    <TextInput label=format!("Konto, rad {}", index + 1) value=row.account list=list />
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
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| rows.update(|all| all.retain(|other| other.id != row.id))>
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
                        rows.update(|all| all.push(LineRow::new(id)));
                    }
                >
                    "Lägg till rad"
                </Button>
            </div>
        </div>
    }
}

/// The Underlag picker and the picked files, each removable. `reading`
/// counts picks still being read. Picks that finish after the form has
/// moved to another `company` are dropped.
#[component]
pub fn PickedFiles(
    id: &'static str,
    files: RwSignal<Vec<lpb::NewAttachment>>,
    reading: RwSignal<u32>,
    error: RwSignal<Option<String>>,
    company: StoredValue<String>,
) -> impl IntoView {
    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = company.get_value();
        reading.update(|n| *n += 1);
        spawn_local(async move {
            let picked = read_files(&input).await;
            reading.try_update(|n| *n -= 1);
            if company_id != company.get_value() {
                return;
            }
            match picked {
                Ok(picked) => files.update(|f| f.extend(picked)),
                Err(code) => error.set(Some(describe_code(code))),
            }
        });
    };
    view! {
        <div class="grid gap-2">
            <FileInput label="Underlag" id=id on_pick=pick />
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
    }
}

/// Betaldatum and Betalkonto, and a button that confirms.
#[component]
pub fn PayForm(date: RwSignal<String>, account: RwSignal<String>, confirm: &'static str, on_confirm: Callback<()>) -> impl IntoView {
    view! {
        <div class="flex items-end gap-2">
            <TextInput label="Betaldatum" kind="date" value=date />
            <TextInput label="Betalkonto" value=account inputmode="numeric" />
            <Button kind="button" on:click=move |_| on_confirm.run(())>{confirm}</Button>
        </div>
    }
}

/// Anledning, and a button that confirms.
#[component]
pub fn ReasonForm(reason: RwSignal<String>, confirm: &'static str, on_confirm: Callback<()>) -> impl IntoView {
    view! {
        <div class="flex items-end gap-2">
            <TextInput label="Anledning" value=reason />
            <Button kind="button" on:click=move |_| on_confirm.run(())>{confirm}</Button>
        </div>
    }
}
```
Run: `cargo test -p doris-web invoice_ui`
Expected: PASS. Ignore unused-item warnings until Step 3.

- [ ] **Step 3: Move the supplier pages onto the shared pieces**

`crates/web/src/pages/new_supplier_invoice.rs`:
- Delete `fn preview_vat`, `struct Row` with its `impl Row`, the `pick` closure, and the `#[cfg(test)] mod tests` at the bottom (its test moved to `invoice_ui`).
- Import `use crate::invoice_ui::{InvoiceLineRows, LineRow, PickedFiles, preview_vat};` and drop the imports that become unused (`read_files`, `size_label`, `FileInput`, `SELECT`, `TextInput`, `account_number`, and possibly `Variant`. Let the compiler say which).
- Replace `Row::new(` with `LineRow::new(` and `Row::preview` with `LineRow::preview`.
- In the VAT effect, replace `let computed = amount(preview_vat(&lines));` with `let computed = amount(preview_vat(&lines).iter().map(|(_, vat)| vat).sum());`.
- In `submit`, replace `.map(Row::request)` with `.map(LineRow::request)`.
- In the view, replace the whole `<div class="grid gap-2">` that holds the column headers, the `<For …>` over `rows` and the "Lägg till rad" button with `<InvoiceLineRows rows=rows next_id=next_id list="invoice_accounts" />`.
- Replace the `<div class="grid gap-2">` that holds `<FileInput label="Underlag" id="invoice_files" …/>` and the picked-files `<ul>` with `<PickedFiles id="invoice_files" files=files reading=reading error=error company=form_company />`.

`crates/web/src/pages/supplier_invoices.rs`:
- Delete `fn status_label` and the `#[cfg(test)] mod tests` (both moved to `invoice_ui`).
- Import `use crate::invoice_ui::{PayForm, ReasonForm, status_label};` and drop `TextInput` from the `ui` import if it becomes unused.
- Replace `let label = status_label(&invoice, &today());` with `let label = status_label(&invoice.status, &invoice.due_date, &today());`.
- Replace the three panel arms with:
  ```rust
                          Panel::Pay => view! {
                              <PayForm date=pay_date account=pay_account confirm="Bekräfta betalning" on_confirm=Callback::new(move |()| act(Panel::Pay)) />
                          }.into_any(),
                          Panel::Cancel => view! {
                              <ReasonForm reason=reason confirm="Bekräfta makulering" on_confirm=Callback::new(move |()| act(Panel::Cancel)) />
                          }.into_any(),
                          Panel::Reverse => view! {
                              <ReasonForm reason=reason confirm="Bekräfta ångring" on_confirm=Callback::new(move |()| act(Panel::Reverse)) />
                          }.into_any(),
  ```

- [ ] **Step 4: Run the checks**

Run: `cargo fmt -p doris-web && cargo test -p doris-web`
Expected: PASS.

Run: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: no warnings.

Run: `make web && cargo build -p doris-server && (cd e2e && npx playwright test supplier_invoices invoicing)`
Expected: PASS, with the supplier specs unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Share the invoice line editor, status and forms between invoice pages"
```

---

### Task 6: The Kundfakturor pages

**Files:**
- Create: `crates/web/src/pages/customer_invoices.rs`, `crates/web/src/pages/new_customer_invoice.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs`, `crates/web/src/errors.rs`
- Modify: `e2e/tests/fixtures.ts` (`addCustomer`)
- Test: `crates/web/src/errors.rs` unit test, `e2e/tests/customer_invoices.spec.ts`

**Interfaces:**
- Consumes: Task 4's `ipb` messages and Task 5's `invoice_ui`.
- Produces: the routes `/customer-invoices` and `/customer-invoices/new`, and the e2e helper `addCustomer(page, app, name, terms = "30")`.

- [ ] **Step 1: Write the failing unit test (in `crates/web/src/errors.rs` `mod tests`)**

```rust
    #[test]
    fn customer_invoice_codes_have_swedish_messages() {
        for code in [
            "customer_invoice_not_found",
            "customer_inactive",
            "duplicate_customer_invoice",
            "customer_invoice_paid",
            "customer_invoice_not_paid",
            "customer_invoice_cancelled",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("invalid_invoice_account"),
            "Raderna kan inte bokföras på reskontrakontot (1510/2440) eller ett momskonto."
        );
    }
```
Run: `cargo test -p doris-web customer_invoice_codes`
Expected: FAIL.

- [ ] **Step 2: Add the messages**

In `message`, replace the `invalid_invoice_account` arm and add the new codes before the `_` arm:
```rust
        "invalid_invoice_account" => {
            "Raderna kan inte bokföras på reskontrakontot (1510/2440) eller ett momskonto."
        }
        "customer_invoice_not_found" => "Kundfakturan finns inte.",
        "customer_inactive" => "Kunden är inaktiv. Aktivera den eller välj en annan.",
        "duplicate_customer_invoice" => {
            "Fakturanumret är redan använt. Ett utfärdat nummer återanvänds aldrig, inte heller efter makulering."
        }
        "customer_invoice_paid" => "Fakturan är redan betald.",
        "customer_invoice_not_paid" => "Fakturan är inte betald.",
        "customer_invoice_cancelled" => "Fakturan är makulerad.",
```
Run: `cargo test -p doris-web customer_invoice_codes`
Expected: PASS.

- [ ] **Step 3: Write the failing e2e test**

Add to `e2e/tests/fixtures.ts`:
```ts
export async function addCustomer(page: Page, app: string, name: string, terms = "30") {
  await page.goto(`${app}/customers`);
  await page.getByRole("button", { name: "Ny kund" }).click();
  await page.getByLabel("Namn", { exact: true }).fill(name);
  await page.getByLabel("Betalningsvillkor (dagar)").fill(terms);
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^1 ${name}`) })).toBeVisible();
}
```
`e2e/tests/customer_invoices.spec.ts`:
```ts
import type { Locator, Page } from "@playwright/test";
import { addCompany, addCustomer, expect, register, test } from "./fixtures";

// The Status cell; the row's buttons have words of their own.
const status = (row: Locator) => row.getByRole("cell").nth(5);

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function registerInvoice(page: Page, app: string, invoiceNumber?: string) {
  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  if (invoiceNumber) await page.getByLabel("Fakturanummer").fill(invoiceNumber);
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await expect(page.getByText(/Att betala 1\s000,00/)).toBeVisible();
  await page.getByLabel("Underlag").setInputFiles([pdf("faktura.pdf")]);
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Kundfakturor" })).toBeVisible();
}

test("a customer invoice gets the proposed number, is booked, paid, reversed and paid again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB", "10");
  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByRole("link", { name: "Ny kundfaktura" }).click();
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1");
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  await page.getByLabel("Fakturadatum").fill(`${new Date().getFullYear()}-01-05`);
  await expect(page.getByLabel("Förfallodatum")).toHaveValue(`${new Date().getFullYear()}-01-15`);
  await registerInvoice(page, app, "1017");

  const row = page.getByRole("row", { name: /^1017 Kund AB/ });
  await expect(status(row)).toHaveText("Obetald");
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura 1017, Kund AB/ })).toContainText("1 underlag");

  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByLabel("Visa betalda och makulerade").check();
  await row.getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(status(row)).toHaveText("Betald");
  await row.getByRole("button", { name: "Ångra betalning" }).click();
  await page.getByLabel("Anledning").fill("Fel konto");
  await page.getByRole("button", { name: "Bekräfta ångring" }).click();
  await expect(status(row)).toHaveText("Obetald");
  await row.getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(status(row)).toHaveText("Betald");

  await page.getByRole("link", { name: "Ny kundfaktura" }).click();
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1018");
});

test("a cancelled invoice's number cannot be used again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB");
  await registerInvoice(page, app);

  const row = page.getByRole("row", { name: /^1 Kund AB/ });
  await row.getByRole("button", { name: "Makulera" }).click();
  await page.getByLabel("Anledning").fill("Fel kund");
  await page.getByRole("button", { name: "Bekräfta makulering" }).click();
  await expect(row).toHaveCount(0);

  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  await page.getByLabel("Fakturanummer").fill("1");
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("alert")).toHaveText(/Fakturanumret är redan använt/);
});

test("switching company clears the customer invoice form", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB");
  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Fakturanummer").fill("77");
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1");
  await expect(page.getByLabel("Konto, rad 1")).toHaveValue("");
  await expect(page.getByLabel("Kund", { exact: true })).toHaveValue("");
});
```
Run: `make web && cargo build -p doris-server && (cd e2e && npx playwright test customer_invoices)`
Expected: FAIL, because there is no "Kundfakturor" link.

- [ ] **Step 4: Write `crates/web/src/pages/new_customer_invoice.rs`**

```rust
//! Register a customer invoice in the active company. The number is
//! proposed by the server and may be changed; the server books it.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::attachments::check_sizes;
use crate::errors::{describe, describe_code};
use crate::format::{amount, plus_days, today};
use crate::invoice_ui::{InvoiceLineRows, LineRow, PickedFiles, preview_vat};
use crate::ui::{Button, Card, ErrorAlert, Field, SELECT_OPTION, Select};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

#[component]
pub fn NewCustomerInvoice() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let customers = RwSignal::new(Vec::<ipb::Customer>::new());
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let customer = RwSignal::new(String::new());
    let invoice_number = RwSignal::new(String::new());
    let invoice_date = RwSignal::new(today());
    let due_date = RwSignal::new(today());
    let reference = RwSignal::new(String::new());
    let next_id = StoredValue::new(1u32);
    let rows = RwSignal::new(vec![LineRow::new(0)]);
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
    let reading = RwSignal::new(0u32);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let form_company = StoredValue::new(String::new());

    // The due date follows the invoice date and the chosen customer's terms.
    Effect::new(move |_| {
        let date = invoice_date.get();
        let chosen = customer.get();
        let terms = customers.with(|all| {
            all.iter()
                .find(|c| c.number.to_string() == chosen)
                .and_then(|c| c.details.as_ref())
                .map_or(30, |d| d.payment_terms)
        });
        if let Some(due) = plus_days(&date, i64::from(terms)) {
            due_date.set(due);
        }
    });
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Nothing typed for the previous company may be registered in this one.
        customers.set(Vec::new());
        customer.set(String::new());
        accounts.set(Vec::new());
        invoice_number.set(String::new());
        reference.set(String::new());
        rows.set(vec![LineRow::new(next_id.get_value())]);
        next_id.update_value(|id| *id += 1);
        files.set(Vec::new());
        error.set(None);
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = invoicing_api();
            let listed = api.list_customers(ipb::ListCustomersRequest { company_id: company_id.clone() }).await;
            let invoices = api
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest { company_id: company_id.clone() })
                .await;
            let chart = ledger_api().list_accounts(lpb::ListAccountsRequest { company_id: company_id.clone() }).await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = listed {
                let active: Vec<_> = response.into_inner().customers.into_iter().filter(|c| c.active).collect();
                customer.set(active.first().map(|c| c.number.to_string()).unwrap_or_default());
                customers.set(active);
            }
            if let Ok(response) = invoices {
                invoice_number.set(response.into_inner().next_invoice_number);
            }
            if let Ok(response) = chart {
                accounts.set(response.into_inner().accounts);
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        if reading.get_untracked() > 0 {
            return;
        }
        let Some(lines) = rows.get_untracked().iter().map(LineRow::request).collect::<Option<Vec<_>>>() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
        busy.set(true);
        let company_id = form_company.get_value();
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = ipb::RegisterCustomerInvoiceRequest {
                company_id: company_id.clone(),
                customer_number: customer.get_untracked().parse().unwrap_or(0),
                invoice_number: invoice_number.get_untracked(),
                invoice_date: invoice_date.get_untracked(),
                due_date: due_date.get_untracked(),
                reference: reference.get_untracked(),
                lines,
                attachments: files.get_untracked(),
            };
            let result = invoicing_api().register_customer_invoice(request).await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => navigate("/customer-invoices", Default::default()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let lines = move || rows.get().iter().map(LineRow::preview).collect::<Vec<_>>();
    let summary = move || {
        let lines = lines();
        let net: i64 = lines.iter().map(|(net, _)| net).sum();
        let vat = preview_vat(&lines);
        let vat_text = vat.iter().map(|(rate, ore)| format!("{rate} %: {}", amount(*ore))).collect::<Vec<_>>().join(", ");
        let total = net + vat.iter().map(|(_, ore)| ore).sum::<i64>();
        format!("Netto {} · Moms {} · Att betala {}", amount(net), if vat_text.is_empty() { amount(0) } else { vat_text }, amount(total))
    };

    view! {
        <Card title="Ny kundfaktura">
            <form class="grid gap-4" data-wide novalidate on:submit=submit>
                <ErrorAlert message=error />
                <div class="grid grid-cols-2 gap-4">
                    <Select label="Kund" id="invoice_customer" value=customer>
                        {move || {
                            customers
                                .get()
                                .into_iter()
                                .map(|c| {
                                    let name = c.details.map(|d| d.name).unwrap_or_default();
                                    view! { <option class=SELECT_OPTION value=c.number.to_string()>{format!("{} {}", c.number, name)}</option> }
                                })
                                .collect_view()
                        }}
                    </Select>
                    <Field label="Fakturanummer" id="customer_invoice_number" value=invoice_number />
                    <Field label="Fakturadatum" id="customer_invoice_date" kind="date" value=invoice_date />
                    <Field label="Förfallodatum" id="customer_invoice_due_date" kind="date" value=due_date />
                    <Field label="OCR/meddelande" id="customer_invoice_reference" value=reference />
                </div>
                <datalist id="customer_invoice_accounts">
                    {move || {
                        accounts
                            .get()
                            .into_iter()
                            .filter(|a| a.active)
                            .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                            .collect_view()
                    }}
                </datalist>
                <InvoiceLineRows rows=rows next_id=next_id list="customer_invoice_accounts" />
                <p class="text-xs/relaxed">{summary}</p>
                <PickedFiles id="customer_invoice_files" files=files reading=reading error=error company=form_company />
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>"Registrera"</Button>
            </form>
        </Card>
    }
}
```

- [ ] **Step 5: Write `crates/web/src/pages/customer_invoices.rs`**

```rust
//! Kundfakturor: the active company's customer invoices. Payments are
//! registered, invoices cancelled and payments reversed from here; the
//! server books it all.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::attachments::{open_in, size_label};
use crate::errors::{describe, describe_code};
use crate::format::{amount, today};
use crate::invoice_ui::{PayForm, ReasonForm, status_label};
use crate::ui::{Button, Checkbox, ErrorAlert, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, Variant};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn CustomerInvoices() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The invoices and the company they were loaded for, set together.
    let invoices = RwSignal::new((String::new(), Vec::<ipb::CustomerInvoice>::new()));
    let show_all = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest { company_id: company_id.clone() })
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
                <h1 class="text-sm font-medium">"Kundfakturor"</h1>
                <A href="/customer-invoices/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ny kundfaktura"</A>
            </div>
            <ErrorAlert message=error />
            <Checkbox label="Visa betalda och makulerade" id="show_all_customer_invoices" checked=show_all />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Fakturanr"</th>
                        <th class=TABLE_HEADER_CELL>"Kund"</th>
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
    invoice: ipb::CustomerInvoice,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let number = invoice.number;
    let label = status_label(&invoice.status, &invoice.due_date, &today());
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
                    .pay_customer_invoice(ipb::PayCustomerInvoiceRequest {
                        company_id,
                        number,
                        date: pay_date.get_untracked(),
                        account: pay_account.get_untracked().trim().parse().unwrap_or(0),
                    })
                    .await
                    .map(|_| ()),
                Panel::Cancel => api
                    .cancel_customer_invoice(ipb::CancelCustomerInvoiceRequest { company_id, number, reason: reason.get_untracked() })
                    .await
                    .map(|_| ()),
                Panel::Reverse => api
                    .reverse_customer_invoice_payment(ipb::ReverseCustomerInvoicePaymentRequest {
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
                .get_customer_invoice_attachment(ipb::GetCustomerInvoiceAttachmentRequest { company_id: company, number, sha256 })
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
            <td class=TABLE_CELL>{i.invoice_number.clone()}</td>
            <td class=TABLE_CELL>{i.customer_name.clone()}</td>
            <td class=TABLE_CELL>{i.invoice_date.clone()}</td>
            <td class=TABLE_CELL>{i.due_date.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(i.total)}</td>
            <td class=if label == "Förfallen" { format!("{TABLE_CELL} text-destructive") } else { TABLE_CELL.to_owned() }>{label}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Details)>"Detaljer"</Button>
                {unpaid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Pay)>"Registrera inbetalning"</Button>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Cancel)>"Makulera"</Button>
                })}
                {paid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Reverse)>"Ångra betalning"</Button>
                })}
            </td>
        </tr>
        <Show when=move || panel.get() != Panel::Closed>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL colspan="7">
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
                                    <p>
                                        {i.vat.iter().map(|v| format!("Moms {} % {}", v.vat_rate, amount(v.amount))).collect::<Vec<_>>().join(" · ")}
                                        {format!(" · Att betala {}", amount(i.total))}
                                    </p>
                                    {(!i.reference.is_empty()).then(|| view! { <p>{format!("OCR/meddelande {}", i.reference)}</p> })}
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
                            <PayForm date=pay_date account=pay_account confirm="Bekräfta inbetalning" on_confirm=Callback::new(move |()| act(Panel::Pay)) />
                        }.into_any(),
                        Panel::Cancel => view! {
                            <ReasonForm reason=reason confirm="Bekräfta makulering" on_confirm=Callback::new(move |()| act(Panel::Cancel)) />
                        }.into_any(),
                        Panel::Reverse => view! {
                            <ReasonForm reason=reason confirm="Bekräfta ångring" on_confirm=Callback::new(move |()| act(Panel::Reverse)) />
                        }.into_any(),
                        Panel::Closed => ().into_any(),
                    }}
                </td>
            </tr>
        </Show>
    }
}
```

- [ ] **Step 6: Register the pages, the routes and the nav link**

- `crates/web/src/pages/mod.rs`: add `mod customer_invoices;`, `mod new_customer_invoice;`, `pub use customer_invoices::CustomerInvoices;` and `pub use new_customer_invoice::NewCustomerInvoice;`.
- `crates/web/src/app.rs`: import both pages (keep the `use crate::pages::{…}` list sorted as rustfmt formats it), and add after the `/customers` route:
  ```rust
  <Route path=path!("/customer-invoices") view=|| view! { <SignedIn><CustomerInvoices /></SignedIn> } />
  <Route path=path!("/customer-invoices/new") view=|| view! { <SignedIn><NewCustomerInvoice /></SignedIn> } />
  ```
  Add the nav link after "Kunder":
  ```rust
  <A href="/customer-invoices" attr:class="text-muted-foreground hover:text-foreground">"Kundfakturor"</A>
  ```

- [ ] **Step 7: Run all checks**

Run: `cargo fmt -p doris-web && cargo test -p doris-web`
Expected: PASS.

Run: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: no warnings.

Run: `make e2e`
Expected: PASS, including the three new specs. `fiscal_year.spec.ts › a year opens with balances…` is known to time out now and then in full parallel runs. If it fails, re-run it alone, and record the outcome in the ledger.

- [ ] **Step 8: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Add the Kundfakturor pages"
```

---

### Task 7: Kontantmetoden on Räkenskapsår for both directions

**Files:**
- Modify: `crates/web/src/pages/fiscal_years.rs`
- Modify: `e2e/tests/supplier_invoices.spec.ts` (only the warning regex)
- Test: `e2e/tests/customer_invoices.spec.ts`

- [ ] **Step 1: Write the failing e2e test (append to `customer_invoices.spec.ts`)**

```ts
test("under kontantmetoden only the payment is booked, and Räkenskapsår warns while unpaid", async ({ page, app }) => {
  const warning = /Det finns obetalda kund- eller leverantörsfakturor/;
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", undefined, "Kontantmetoden");
  await addCustomer(page, app, "Kund AB");
  await registerInvoice(page, app);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura/ })).toHaveCount(0);
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByText(warning)).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByRole("row", { name: /^1 Kund AB/ }).getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(page.getByRole("row", { name: /^1 Kund AB/ })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura 1, Kund AB/ })).toContainText("1 underlag");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByRole("heading", { name: "Räkenskapsår" })).toBeVisible();
  await expect(page.getByText(warning)).toHaveCount(0);
});
```
In `e2e/tests/supplier_invoices.spec.ts`, change `const warning = /Det finns obetalda leverantörsfakturor/;` to `const warning = /Det finns obetalda kund- eller leverantörsfakturor/;`. This is the only supplier test edit, and it follows from the spec's new wording.

Run: `make web && cargo build -p doris-server && (cd e2e && npx playwright test customer_invoices supplier_invoices -g kontantmetoden)`
Expected: FAIL. The warning is missing for customer invoices, and the supplier text no longer matches.

- [ ] **Step 2: Count both directions in `fiscal_years.rs`**

Replace the body of the `spawn_local` in the `unpaid_under_cash` effect with:
```rust
        spawn_local(async move {
            let mut api = invoicing_api();
            let suppliers = api
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest { company_id: company_id.clone() })
                .await;
            let customers = api
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest { company_id: company_id.clone() })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            let supplier_unpaid = suppliers.map(|r| {
                let r = r.into_inner();
                r.cash_method && r.invoices.iter().any(|i| i.status == "unpaid")
            });
            let customer_unpaid = customers.map(|r| {
                let r = r.into_inner();
                r.cash_method && r.invoices.iter().any(|i| i.status == "unpaid")
            });
            unpaid_under_cash.set(supplier_unpaid.unwrap_or(false) || customer_unpaid.unwrap_or(false));
        });
```
Change the warning text to `"Det finns obetalda kund- eller leverantörsfakturor. Med kontantmetoden ska de bokföras vid räkenskapsårets slut (BFL 5 kap. 2 §). Doris gör inte det än."`, and the comment above the effect to say "unpaid customer and supplier invoices".

- [ ] **Step 3: Run all checks**

Run: `cargo fmt -p doris-web && cargo test --workspace`
Expected: PASS.

Run: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: no warnings.

Run: `make e2e`
Expected: PASS. The known `fiscal_year` flake is handled as in Task 6.

- [ ] **Step 4: Commit**

```bash
git add crates/web e2e/tests
git commit -m "Warn on Räkenskapsår about unpaid customer invoices under kontantmetoden too"
```

---

### Task 8: AGENTS.md and the dist check

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Build the dist and record its size**

Run: `make dist`
Expected: it builds. If the wasm is over `WASM_BUDGET`, `make dist` fails at the check. Then record `Task 8: make dist over budget (<bytes>), not changed — budget handled in another session` in the ledger and carry on. Do not change the budget.

- [ ] **Step 2: Update `AGENTS.md`**

- Change the `crates/invoicing` layout line to `doris-invoicing: customers, suppliers, and customer and supplier invoices`.
- Change the underlag rule to "the company's own voucher (`voucher_attachments`) or the company's own customer or supplier invoice".
- Add after the supplier-invoices bullet:
  ```
  - Customer invoices (`customer-invoices-{company}` stream, `customer_invoices`
    projection) mirror supplier invoices with 1510 as the reskontra account
    and output VAT per rate on 2611/2621/2631 (computed, never overridden).
    The invoice number is proposed (highest all-digit number + 1) and may be
    changed; it is unique per company even after cancelling (full unique
    index), so an issued number is never reused. Both directions share
    `crates/invoicing/src/invoices.rs` (status, transitions, voucher lines,
    correction dates) and `crates/web/src/invoice_ui.rs`. Codes:
    `customer_invoice_not_found`, `customer_inactive`,
    `duplicate_customer_invoice`, `customer_invoice_paid`,
    `customer_invoice_not_paid` and `customer_invoice_cancelled`.
  ```
- In the supplier bullet, change "Räkenskapsår warns" to "Räkenskapsår warns for unpaid customer and supplier invoices".

- [ ] **Step 3: Run everything one last time**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Document customer invoices"
```
