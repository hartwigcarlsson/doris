# Underlag till verifikationer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users attach PDF/JPEG/PNG underlag to a voucher, when booking it and afterwards (also in a closed year), stored immutably in the SQLite file and opened in the browser.

**Architecture:** Bytes go into an append-only `attachment_files` table keyed by SHA-256 (primary data, like `events`). Linking a file to a voucher is a new `AttachmentAdded` event in the existing `ledger-{company}-{fy}` stream, projected into `voucher_attachments`. The lib layer books a voucher and its files in one `BEGIN IMMEDIATE` transaction by calling `add_attachment_in` per file after `record_voucher_in`. gRPC-Web carries the bytes (`bytes` fields), with the ledger service's message limits raised; the browser opens a download as a Blob URL.

**Tech Stack:** Rust, sqlx/SQLite, tonic 0.14 + tonic-web, prost, sha2 0.11, Leptos 0.8 CSR, web-sys, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-03-underlag-design.md`

## Global Constraints

- File types: PDF (`%PDF-`), JPEG (`FF D8 FF`), PNG (`89 50 4E 47 0D 0A 1A 0A`), decided from the bytes, never from the client.
- Per file: 1 byte to 10 MiB (10 485 760 bytes). Per `RecordVoucher` request: at most 20 MiB of attachment data in total.
- `LedgerService` limits: `max_decoding_message_size` 21 MiB, `max_encoding_message_size` 11 MiB. Other services keep tonic's 4 MiB default.
- File name: trimmed, 1–255 characters, no `/`, `\` or control characters.
- An underlag is never removed or renamed. `attachment_files` is append-only (UPDATE/DELETE triggers). A closed year accepts `AttachmentAdded` and nothing else new.
- New error codes (snake_case, stable): `unsupported_attachment_type`, `invalid_attachment_name`, `empty_attachment`, `attachment_too_large`, `duplicate_attachment` (all `InvalidArgument`), `attachment_not_found` (`NotFound`).
- Never log file names (they can carry personal data). Code and identifiers in English; only UI text in Swedish.
- TDD: every behaviour starts with a failing test; each task ends in a commit. Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb
  ```
- Wasm stays under `WASM_BUDGET` (500 KB gzipped, `make dist`). Never set `RUSTFLAGS` for wasm builds.

## Review Focus

1. **The same file in two companies** — stored once, readable by each company only through its own voucher; a hash alone never reads the other company's file. Pinned in Task 4.
2. **Swedish and emoji file names** (`Kvitto åäö 🧾.pdf`) — round-trip unchanged through event, projection and list. Pinned in Task 3.
3. **A 10 MiB file through the real gRPC-Web stack, both ways** — upload passes the raised server limit and download passes the client's decode limit (tests' client and the browser's `ledger_api()` both raise it to 11 MiB). Pinned in Task 5 (server test) and Task 6 (`api.rs`).
4. **Switching company while files are picked** — the picked files belong to the company the form was filled for and are cleared on a switch, so they are never booked in another company. Pinned in Task 7 (e2e).
5. **An attachment aimed at a missing voucher or a non-existent fiscal year** (number 99, a start that isn't a fiscal-year start, a far-future start) — `voucher_not_found`, never a panic. Pinned in Task 3.

---

### Task 1: Attachment value objects and error codes

**Files:**
- Modify: `crates/ledger/src/domain.rs` (DomainError at lines 8–63; new items after `VoucherLine` impl, ~line 284)
- Modify: `crates/server/src/ledger.rs:310-347` (`domain_status`, to keep the exhaustive match compiling)
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Produces: `DomainError::{UnsupportedAttachmentType, InvalidAttachmentName, EmptyAttachment, AttachmentTooLarge, DuplicateAttachment, AttachmentNotFound}`; `pub const MAX_ATTACHMENT_SIZE: usize`; `pub enum ContentType { Pdf, Jpeg, Png }` with `as_mime(self) -> &'static str` and `from_mime(&str) -> Option<Self>`; `pub fn sniff(&[u8]) -> Result<ContentType, DomainError>`; `pub struct AttachmentName` with `parse(&str)` and `as_str()`; `pub struct Attachment { pub sha256: String, pub file_name: AttachmentName, pub content_type: ContentType, pub size: u64 }` with `Attachment::new(file_name: &str, data: &[u8], sha256: String) -> Result<Self, DomainError>`.

- [ ] **Step 1: Write the failing tests** — append to `crates/ledger/tests/domain.rs`:

```rust
fn pdf_of(size: usize) -> Vec<u8> {
    let mut data = b"%PDF-1.7\n".to_vec();
    data.resize(size, 0);
    data
}

#[test]
fn the_file_type_comes_from_the_first_bytes() {
    assert_eq!(sniff(b"%PDF-1.7\n"), Ok(ContentType::Pdf));
    assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Ok(ContentType::Jpeg));
    assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0"), Ok(ContentType::Png));
    for bad in [&b"GIF89a"[..], b"hej", b"", b"%PDF"] {
        assert_eq!(sniff(bad), Err(DomainError::UnsupportedAttachmentType), "{bad:?}");
    }
    assert_eq!(ContentType::Pdf.as_mime(), "application/pdf");
    assert_eq!(ContentType::Jpeg.as_mime(), "image/jpeg");
    assert_eq!(ContentType::from_mime("image/png"), Some(ContentType::Png));
    assert_eq!(ContentType::from_mime("image/gif"), None);
}

#[test]
fn attachment_names_are_trimmed_1_to_255_characters_without_paths() {
    assert_eq!(AttachmentName::parse("  Kvitto åäö 🧾.pdf ").unwrap().as_str(), "Kvitto åäö 🧾.pdf");
    assert!(AttachmentName::parse(&"å".repeat(255)).is_ok());
    for bad in ["", "   ", &"å".repeat(256), "a/b.pdf", "a\\b.pdf", "a\nb.pdf", "a\0.pdf"] {
        assert_eq!(AttachmentName::parse(bad), Err(DomainError::InvalidAttachmentName), "{bad:?}");
    }
}

#[test]
fn an_attachment_is_1_byte_to_10_mib_of_pdf_jpeg_or_png() {
    let max = Attachment::new("kvitto.pdf", &pdf_of(MAX_ATTACHMENT_SIZE), "ab".into()).unwrap();
    assert_eq!(max.size, 10_485_760);
    assert_eq!(max.content_type, ContentType::Pdf);
    assert_eq!(max.file_name.as_str(), "kvitto.pdf");
    assert_eq!(max.sha256, "ab");
    assert_eq!(
        Attachment::new("kvitto.pdf", &pdf_of(MAX_ATTACHMENT_SIZE + 1), "ab".into()),
        Err(DomainError::AttachmentTooLarge)
    );
    assert_eq!(Attachment::new("tom.pdf", b"", "ab".into()), Err(DomainError::EmptyAttachment));
    assert_eq!(
        Attachment::new("bild.gif", b"GIF89a", "ab".into()),
        Err(DomainError::UnsupportedAttachmentType)
    );
    assert_eq!(
        Attachment::new("", b"%PDF-1.7\n", "ab".into()),
        Err(DomainError::InvalidAttachmentName)
    );
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-ledger --test domain attachment`
Expected: compile errors — `sniff`, `ContentType`, `AttachmentName`, `Attachment`, `MAX_ATTACHMENT_SIZE` not found.

- [ ] **Step 3: Implement** — in `crates/ledger/src/domain.rs`, add to `DomainError` before `Overflow`:

```rust
    #[error("an underlag must be a PDF, JPEG or PNG")]
    UnsupportedAttachmentType,
    #[error("file name must be 1-255 characters, without / \\ or control characters")]
    InvalidAttachmentName,
    #[error("the file is empty")]
    EmptyAttachment,
    #[error("the file is too large")]
    AttachmentTooLarge,
    #[error("the voucher already has this file")]
    DuplicateAttachment,
    #[error("no such underlag")]
    AttachmentNotFound,
```

After the `impl VoucherLine` block, add:

```rust
/// The most one underlag may weigh: 10 MiB.
pub const MAX_ATTACHMENT_SIZE: usize = 10 * 1024 * 1024;

/// The file types an underlag may have. All open in a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Pdf,
    Jpeg,
    Png,
}

impl ContentType {
    pub fn as_mime(self) -> &'static str {
        match self {
            ContentType::Pdf => "application/pdf",
            ContentType::Jpeg => "image/jpeg",
            ContentType::Png => "image/png",
        }
    }

    pub fn from_mime(mime: &str) -> Option<Self> {
        [ContentType::Pdf, ContentType::Jpeg, ContentType::Png]
            .into_iter()
            .find(|t| t.as_mime() == mime)
    }
}

/// The file's type, from its first bytes. What a client claims is never
/// trusted.
pub fn sniff(data: &[u8]) -> Result<ContentType, DomainError> {
    if data.starts_with(b"%PDF-") {
        Ok(ContentType::Pdf)
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Ok(ContentType::Jpeg)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(ContentType::Png)
    } else {
        Err(DomainError::UnsupportedAttachmentType)
    }
}

/// An underlag's file name, as the user picked it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttachmentName(String);

impl AttachmentName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        let safe = !name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control());
        if safe && (1..=255).contains(&name.chars().count()) {
            Ok(Self(name.to_owned()))
        } else {
            Err(DomainError::InvalidAttachmentName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An underlag (BFL 5 kap. 6–7 §). The bytes are stored once per `sha256`,
/// apart from the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub sha256: String,
    pub file_name: AttachmentName,
    pub content_type: ContentType,
    pub size: u64,
}

impl Attachment {
    /// `sha256` is the lowercase hex SHA-256 of `data`, worked out by the
    /// caller so the domain stays free of crypto.
    pub fn new(file_name: &str, data: &[u8], sha256: String) -> Result<Self, DomainError> {
        let file_name = AttachmentName::parse(file_name)?;
        if data.is_empty() {
            return Err(DomainError::EmptyAttachment);
        }
        if data.len() > MAX_ATTACHMENT_SIZE {
            return Err(DomainError::AttachmentTooLarge);
        }
        Ok(Self {
            sha256,
            file_name,
            content_type: sniff(data)?,
            size: data.len() as u64,
        })
    }
}
```

In `crates/server/src/ledger.rs` `domain_status`, add before `Overflow =>`:

```rust
        UnsupportedAttachmentType => Status::invalid_argument("unsupported_attachment_type"),
        InvalidAttachmentName => Status::invalid_argument("invalid_attachment_name"),
        EmptyAttachment => Status::invalid_argument("empty_attachment"),
        AttachmentTooLarge => Status::invalid_argument("attachment_too_large"),
        DuplicateAttachment => Status::invalid_argument("duplicate_attachment"),
        AttachmentNotFound => Status::not_found("attachment_not_found"),
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p doris-ledger --test domain && cargo build -p doris-server`
Expected: all domain tests PASS; server builds.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger/src/domain.rs crates/ledger/tests/domain.rs crates/server/src/ledger.rs
git commit -m "Add attachment value objects: type sniffing, file names and size limits

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 2: AttachmentAdded event and decision

**Files:**
- Modify: `crates/ledger/src/domain.rs` (`LedgerEvent` ~line 288, `Voucher` ~line 308, `Ledger::apply` ~line 409, new `add_attachment` after `correct_voucher`)
- Modify: `crates/ledger/src/queries.rs:105-112, 285-292` (add `attachments: Vec::new()` to `Voucher` literals)
- Modify: `crates/ledger/src/projections.rs:87-178` (temporary no-op arm, replaced in Task 3)
- Test: `crates/ledger/tests/domain.rs`

**Interfaces:**
- Consumes: `Attachment` (Task 1).
- Produces: `LedgerEvent::AttachmentAdded { voucher: u32, attachment: Attachment }`; `Voucher.attachments: Vec<Attachment>`; `pub fn add_attachment(ledger: &Ledger, voucher: u32, attachment: Attachment) -> Result<LedgerEvent, DomainError>`.

- [ ] **Step 1: Write the failing tests** — append to `crates/ledger/tests/domain.rs`:

```rust
fn attachment(name: &str, sha256: &str) -> Attachment {
    Attachment::new(name, b"%PDF-1.7\n", sha256.into()).unwrap()
}

#[test]
fn an_attachment_is_added_to_an_existing_voucher_once_in_order() {
    let booked = record(&[], &seeded(), sale("2025-03-01", 100)).unwrap();
    let ledger = Ledger::from_events(first_year(), std::slice::from_ref(&booked));
    let (a, b) = (attachment("kvitto.pdf", "aa"), attachment("faktura.pdf", "bb"));

    let first = add_attachment(&ledger, 1, a.clone()).unwrap();
    assert_eq!(
        first,
        LedgerEvent::AttachmentAdded { voucher: 1, attachment: a.clone() }
    );
    assert_eq!(add_attachment(&ledger, 2, a.clone()), Err(DomainError::VoucherNotFound));

    let ledger = Ledger::from_events(first_year(), &[booked.clone(), first.clone()]);
    assert_eq!(
        add_attachment(&ledger, 1, attachment("kopia.pdf", "aa")),
        Err(DomainError::DuplicateAttachment)
    );
    let second = add_attachment(&ledger, 1, b.clone()).unwrap();
    let ledger = Ledger::from_events(first_year(), &[booked, first, second]);
    assert_eq!(ledger.voucher(1).unwrap().attachments, vec![a, b]);
}

#[test]
fn a_closed_year_still_takes_an_attachment() {
    let booked = record(&[], &seeded(), sale("2025-03-01", 100)).unwrap();
    let ledger = closed_year(vec![booked]);

    assert!(add_attachment(&ledger, 1, attachment("kvitto.pdf", "aa")).is_ok());
}

#[test]
fn attachment_events_are_readable_json() {
    let event = LedgerEvent::AttachmentAdded {
        voucher: 1,
        attachment: attachment("kvitto.pdf", "aa"),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({
            "type": "AttachmentAdded",
            "voucher": 1,
            "attachment": {
                "sha256": "aa",
                "file_name": "kvitto.pdf",
                "content_type": "pdf",
                "size": 9
            }
        })
    );
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-ledger --test domain attachment`
Expected: compile errors — no variant `AttachmentAdded`, no field `attachments`, no function `add_attachment`.

- [ ] **Step 3: Implement** — in `domain.rs`:

Add to `LedgerEvent` after `FiscalYearReopened`:

```rust
    /// An underlag for voucher `voucher` (BFL 5 kap. 6–7 §). Never removed;
    /// who added it, and when, is in the event metadata.
    AttachmentAdded { voucher: u32, attachment: Attachment },
```

Add to `Voucher` after `corrected_by`:

```rust
    /// In the order they were added.
    pub attachments: Vec<Attachment>,
```

In `Ledger::apply`, the `VoucherRecorded` arm's `Voucher { … }` gets `attachments: Vec::new(),`; add the arm:

```rust
            LedgerEvent::AttachmentAdded { voucher, attachment } => {
                if let Some(v) = self.vouchers.iter_mut().find(|v| v.number == voucher) {
                    v.attachments.push(attachment);
                }
            }
```

After `correct_voucher`, add:

```rust
/// Adds an underlag to a voucher. A closed year takes it too: it changes no
/// amount, and the event records who added it and when.
pub fn add_attachment(
    ledger: &Ledger,
    voucher: u32,
    attachment: Attachment,
) -> Result<LedgerEvent, DomainError> {
    let existing = ledger.voucher(voucher).ok_or(DomainError::VoucherNotFound)?;
    if existing.attachments.iter().any(|a| a.sha256 == attachment.sha256) {
        return Err(DomainError::DuplicateAttachment);
    }
    Ok(LedgerEvent::AttachmentAdded { voucher, attachment })
}
```

In `crates/ledger/src/queries.rs`, add `attachments: Vec::new(),` to both `Voucher { … }` literals (in `list_vouchers` and in the `a_line_without_its_voucher_is_skipped` test).

In `crates/ledger/src/projections.rs` `apply_ledger`, add a temporary arm (Task 3 replaces it):

```rust
        LedgerEvent::AttachmentAdded { .. } => {}
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p doris-ledger && cargo build -p doris-server`
Expected: PASS; server builds.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger
git commit -m "Decide AttachmentAdded: once per file and voucher, also in a closed year

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 3: Store files, book with attachments, project and list them

**Files:**
- Create: `migrations/0008_attachments.sql`
- Modify: `crates/ledger/Cargo.toml` (add `sha2.workspace = true`)
- Modify: `crates/ledger/src/lib.rs` (new `NewAttachment`, `record_voucher_with_attachments`, `add_attachment`, `add_attachment_in`, `sha256_hex`)
- Modify: `crates/ledger/src/projections.rs` (real `AttachmentAdded` arm; rebuild list)
- Modify: `crates/ledger/src/queries.rs` (`list_vouchers` reads attachments; `voucher_at`, `projected_attachment` helpers)
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: `Attachment`, `ContentType`, `AttachmentName`, `domain::add_attachment` (Tasks 1–2).
- Produces:
  - `pub struct NewAttachment { pub file_name: String, pub data: Vec<u8> }` (in `doris_ledger`)
  - `pub async fn record_voucher_with_attachments(pool: &SqlitePool, company_id: Uuid, actor: Uuid, cmd: RecordVoucher, attachments: Vec<NewAttachment>, today: Date) -> Result<VoucherRef>`
  - `pub async fn add_attachment(pool: &SqlitePool, company_id: Uuid, actor: Uuid, fiscal_year_start: Date, number: u32, attachment: NewAttachment, today: Date) -> Result<Attachment>`
  - `list_vouchers` now fills `Voucher.attachments`.
  - `fn projected_attachment(sha256: String, file_name: &str, content_type: &str, size: i64) -> Attachment` in `queries.rs` (used again in Task 4).

- [ ] **Step 1: Write the failing tests** — in `crates/ledger/tests/store.rs`, extend the imports:

```rust
use doris_ledger::domain::{ContentType, DomainError, RecordVoucher, TrialBalanceRow, VoucherLine};
use doris_ledger::{
    Error, NewAttachment, VoucherRef, account_ledger, add_account, add_attachment,
    close_fiscal_year, correct_voucher, list_accounts, list_fiscal_years, list_vouchers,
    opening_balances, rebuild_projections, record_voucher, record_voucher_in,
    record_voucher_with_attachments, rename_account, reopen_fiscal_year, set_account_active,
    set_opening_balances, trial_balance,
};
```

and append:

```rust
/// A small PDF: the header plus `body`.
fn file(name: &str, body: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: format!("%PDF-1.7\n{body}").into_bytes(),
    }
}

fn png(name: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: b"\x89PNG\r\n\x1a\nbild".to_vec(),
    }
}

#[tokio::test]
async fn a_voucher_is_booked_with_its_attachments_in_one_transaction() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let booked = record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("Kvitto åäö 🧾.pdf", "a"), png("foto.png")],
        d(TODAY),
    )
    .await
    .unwrap();

    assert_eq!(booked.number, 1);
    assert_eq!(
        events_of(&pool, "ledger-").await,
        ["VoucherRecorded", "AttachmentAdded", "AttachmentAdded"]
    );
    let vouchers = list_vouchers(&pool, id, anna, d("2025-01-01")).await.unwrap();
    let listed: Vec<_> = vouchers[0]
        .attachments
        .iter()
        .map(|a| (a.file_name.as_str(), a.content_type, a.size))
        .collect();
    assert_eq!(
        listed,
        [("Kvitto åäö 🧾.pdf", ContentType::Pdf, 10), ("foto.png", ContentType::Png, 12)]
    );
    assert_eq!(vouchers[0].attachments[0].sha256.len(), 64);
    assert_eq!(
        table(&pool, "SELECT position || ':' || added_by FROM voucher_attachments ORDER BY position").await,
        [format!("1:{anna}"), format!("2:{anna}")]
    );
}

#[tokio::test]
async fn a_file_is_stored_once_however_often_it_is_attached() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("kvitto.pdf", "a")], today)
        .await
        .unwrap();
    record_voucher_with_attachments(&pool, id, anna, sale("2025-03-02", 100), vec![file("kopia.pdf", "a")], today)
        .await
        .unwrap();

    assert_eq!(table(&pool, "SELECT sha256 FROM attachment_files").await.len(), 1);
    assert_eq!(table(&pool, "SELECT sha256 FROM voucher_attachments").await.len(), 2);
}

#[tokio::test]
async fn a_rejected_booking_keeps_neither_the_voucher_nor_its_files() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let unbalanced = RecordVoucher {
        lines: vec![
            VoucherLine::new(1930, 100, 0).unwrap(),
            VoucherLine::new(3001, 0, 99).unwrap(),
        ],
        ..sale("2025-03-01", 100)
    };
    let gif = NewAttachment { file_name: "bild.gif".into(), data: b"GIF89a".to_vec() };

    assert!(matches!(
        record_voucher_with_attachments(&pool, id, anna, unbalanced, vec![file("kvitto.pdf", "a")], today).await,
        Err(Error::Domain(DomainError::VoucherUnbalanced))
    ));
    assert!(matches!(
        record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("kvitto.pdf", "a"), gif], today).await,
        Err(Error::Domain(DomainError::UnsupportedAttachmentType))
    ));
    assert!(matches!(
        record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("a.pdf", "x"), file("b.pdf", "x")], today).await,
        Err(Error::Domain(DomainError::DuplicateAttachment))
    ));

    assert!(events_of(&pool, "ledger-").await.is_empty());
    assert!(table(&pool, "SELECT sha256 FROM attachment_files").await.is_empty());
    let booked = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();
    assert_eq!(booked.number, 1);
}

#[tokio::test]
async fn attachment_files_are_append_only() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("kvitto.pdf", "a")], d(TODAY))
        .await
        .unwrap();

    for sql in ["UPDATE attachment_files SET size = 0", "DELETE FROM attachment_files"] {
        let err = sqlx::query(sql).execute(&pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
}

#[tokio::test]
async fn an_attachment_is_added_later_even_in_a_closed_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today).await.unwrap();
    close_fiscal_year(&pool, id, anna, start, today).await.unwrap();

    let added = add_attachment(&pool, id, anna, start, 1, file("faktura.pdf", "f"), today)
        .await
        .unwrap();

    assert_eq!(added.file_name.as_str(), "faktura.pdf");
    let vouchers = list_vouchers(&pool, id, anna, start).await.unwrap();
    assert_eq!(vouchers[0].attachments, vec![added]);
    for (fy_start, number) in [(start, 99), (d("2025-02-01"), 1), (d("2999-01-01"), 1)] {
        assert!(
            matches!(
                add_attachment(&pool, id, anna, fy_start, number, file("x.pdf", "x"), today).await,
                Err(Error::Domain(DomainError::VoucherNotFound))
            ),
            "{fy_start} {number}"
        );
    }
    assert!(matches!(
        add_attachment(&pool, id, Uuid::new_v4(), start, 1, file("x.pdf", "x"), today).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn attachments_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("a.pdf", "a"), file("b.pdf", "b")], today)
        .await
        .unwrap();
    add_attachment(&pool, id, anna, start, 1, file("c.pdf", "c"), today).await.unwrap();
    let sql = "SELECT company_id || fiscal_year_start || number || position || sha256 || file_name
               || content_type || size || added_at || added_by FROM voucher_attachments ORDER BY 1";
    let before = table(&pool, sql).await;
    let listed = list_vouchers(&pool, id, anna, start).await.unwrap();

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(before.len(), 3);
    assert_eq!(table(&pool, sql).await, before);
    assert_eq!(list_vouchers(&pool, id, anna, start).await.unwrap(), listed);
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-ledger --test store attach`
Expected: compile errors — `NewAttachment`, `record_voucher_with_attachments`, `add_attachment` not in `doris_ledger`.

- [ ] **Step 3: Implement**

`migrations/0008_attachments.sql`:

```sql
-- Underlag (räkenskapsinformation, BFL 7 kap.). Primary data, like events:
-- not a projection, never updated or deleted. A file is stored once.
CREATE TABLE attachment_files (
    sha256 TEXT    PRIMARY KEY,
    size   INTEGER NOT NULL,
    data   BLOB    NOT NULL
);
CREATE TRIGGER attachment_files_no_update BEFORE UPDATE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;
CREATE TRIGGER attachment_files_no_delete BEFORE DELETE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;

-- Projection of AttachmentAdded. Rebuildable from events. Files are read
-- only through here, so a hash alone never reads another company's file.
CREATE TABLE voucher_attachments (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    position          INTEGER NOT NULL,
    sha256            TEXT    NOT NULL REFERENCES attachment_files (sha256),
    file_name         TEXT    NOT NULL,
    content_type      TEXT    NOT NULL,
    size              INTEGER NOT NULL,
    added_at          TEXT    NOT NULL,
    added_by          TEXT    NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number, position),
    UNIQUE (company_id, fiscal_year_start, number, sha256),
    FOREIGN KEY (company_id, fiscal_year_start, number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);
```

`crates/ledger/Cargo.toml` `[dependencies]`: add `sha2.workspace = true`.

`crates/ledger/src/lib.rs`: extend `use domain::{…}` with `Attachment`; add `use sha2::{Digest, Sha256};`; export `get_attachment` later (Task 4). After `correct_voucher_in`, add:

```rust
/// An underlag as it arrives: the file's name and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttachment {
    pub file_name: String,
    pub data: Vec<u8>,
}

/// Books a voucher and its underlag in one transaction: all of it, or
/// nothing and no number used up.
pub async fn record_voucher_with_attachments(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    attachments: Vec<NewAttachment>,
    today: Date,
) -> Result<VoucherRef> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = record_voucher_in(&mut tx, company_id, actor, cmd, today).await?;
    for attachment in attachments {
        add_attachment_in(
            &mut tx,
            company_id,
            actor,
            voucher.fiscal_year_start,
            voucher.number,
            attachment,
            today,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(voucher)
}

/// Adds an underlag to voucher `number` of the fiscal year starting on
/// `fiscal_year_start`, also when that year is closed.
pub async fn add_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    attachment: NewAttachment,
    today: Date,
) -> Result<Attachment> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let added = add_attachment_in(
        &mut tx,
        company_id,
        actor,
        fiscal_year_start,
        number,
        attachment,
        today,
    )
    .await?;
    tx.commit().await?;
    Ok(added)
}

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
    sqlx::query("INSERT OR IGNORE INTO attachment_files (sha256, size, data) VALUES (?, ?, ?)")
        .bind(&attachment.sha256)
        .bind(new.data.len() as i64)
        .bind(&new.data)
        .execute(&mut *conn)
        .await?;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(conn, &stream, version, &[event], actor).await?;
    Ok(attachment)
}

/// Lowercase hex SHA-256.
fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
```

`crates/ledger/src/projections.rs`: replace the temporary `AttachmentAdded` arm with:

```rust
        LedgerEvent::AttachmentAdded { voucher, attachment } => {
            // MAX without GROUP BY always yields one row, so the first
            // underlag of a voucher gets position 1.
            sqlx::query(
                "INSERT INTO voucher_attachments (company_id, fiscal_year_start, number, position,
                     sha256, file_name, content_type, size, added_at, added_by)
                 SELECT ?, ?, ?, COALESCE(MAX(position), 0) + 1, ?, ?, ?, ?, ?, ?
                 FROM voucher_attachments
                 WHERE company_id = ? AND fiscal_year_start = ? AND number = ?",
            )
            .bind(company_id)
            .bind(fiscal_year_start)
            .bind(voucher)
            .bind(&attachment.sha256)
            .bind(attachment.file_name.as_str())
            .bind(attachment.content_type.as_mime())
            .bind(attachment.size as i64)
            .bind(&event.recorded_at)
            .bind(event.metadata.actor.as_deref().unwrap_or_default())
            .bind(company_id)
            .bind(fiscal_year_start)
            .bind(voucher)
            .execute(&mut *conn)
            .await?;
        }
```

In `rebuild_projections`, insert `"DELETE FROM voucher_attachments",` before `"DELETE FROM voucher_lines",` (it refers to `vouchers`). `attachment_files` is never touched.

`crates/ledger/src/queries.rs`: extend the `use crate::domain::{…}` with `Attachment, AttachmentName, ContentType`. In `list_vouchers`, inside the read transaction after the `lines` query:

```rust
    let attachments: Vec<(u32, String, String, String, i64)> = sqlx::query_as(
        "SELECT number, sha256, file_name, content_type, size FROM voucher_attachments
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number, position",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(&mut *tx)
    .await?;
```

and after `attach_lines(&mut vouchers, lines);`:

```rust
    for (number, sha256, file_name, content_type, size) in attachments {
        if let Some(voucher) = voucher_at(&mut vouchers, number) {
            let attachment = projected_attachment(sha256, &file_name, &content_type, size);
            voucher.attachments.push(attachment);
        }
    }
```

Replace `attach_lines` with a shared lookup:

```rust
/// Voucher `number` in `vouchers`. Numbers run 1..=n (the trigger
/// guarantees it), so number - 1 is the index. A row whose head is missing
/// (committed after the heads were read) gives `None`, never a panic.
fn voucher_at(vouchers: &mut [Voucher], number: u32) -> Option<&mut Voucher> {
    (number as usize)
        .checked_sub(1)
        .and_then(|i| vouchers.get_mut(i))
        .filter(|v| v.number == number)
}

/// Puts each `(number, account, debit, credit)` line on its voucher.
fn attach_lines(vouchers: &mut [Voucher], lines: Vec<(u32, u32, i64, i64)>) {
    for (number, account, debit, credit) in lines {
        if let Some(voucher) = voucher_at(vouchers, number) {
            voucher.lines.push(
                VoucherLine::new(account, debit, credit).expect("projected accounts are valid"),
            );
        }
    }
}

/// An underlag as the projection holds it.
fn projected_attachment(sha256: String, file_name: &str, content_type: &str, size: i64) -> Attachment {
    Attachment {
        sha256,
        file_name: AttachmentName::parse(file_name).expect("projected names are valid"),
        content_type: ContentType::from_mime(content_type).expect("projected types are valid"),
        size: size as u64,
    }
}
```

`lib.rs` `pub use` stays as is apart from the new public items above (they're defined in `lib.rs` itself).

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p doris-ledger`
Expected: all PASS, including `stress`.

- [ ] **Step 5: Commit**

```bash
git add migrations/0008_attachments.sql crates/ledger
git commit -m "Store underlag once by hash and book a voucher with its files in one transaction

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 4: Read an attachment through the company's own voucher

**Files:**
- Modify: `crates/ledger/src/queries.rs` (new `get_attachment`)
- Modify: `crates/ledger/src/lib.rs:25-28` (`pub use queries::{…, get_attachment}`)
- Test: `crates/ledger/tests/store.rs`

**Interfaces:**
- Consumes: `projected_attachment` (Task 3), `DomainError::AttachmentNotFound` (Task 1).
- Produces: `pub async fn get_attachment(pool: &SqlitePool, company_id: Uuid, user_id: Uuid, fiscal_year_start: Date, number: u32, sha256: &str) -> Result<(Attachment, Vec<u8>)>`.

- [ ] **Step 1: Write the failing test** — add `get_attachment` to the `doris_ledger` import in `store.rs`, then append:

```rust
#[tokio::test]
async fn an_attachment_is_read_only_through_the_companys_own_voucher() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let other = second_company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher_with_attachments(&pool, id, anna, sale("2025-03-01", 100), vec![file("kvitto.pdf", "a")], today)
        .await
        .unwrap();
    record_voucher(&pool, other, anna, sale("2025-03-01", 100), today).await.unwrap();
    let sha = list_vouchers(&pool, id, anna, start).await.unwrap()[0].attachments[0]
        .sha256
        .clone();

    let (attachment, data) = get_attachment(&pool, id, anna, start, 1, &sha).await.unwrap();
    assert_eq!(attachment.file_name.as_str(), "kvitto.pdf");
    assert_eq!(attachment.content_type, ContentType::Pdf);
    assert_eq!(data, file("", "a").data);

    // The right hash on another company's voucher, or on another voucher.
    for (company_id, number) in [(other, 1), (id, 2)] {
        assert!(matches!(
            get_attachment(&pool, company_id, anna, start, number, &sha).await,
            Err(Error::Domain(DomainError::AttachmentNotFound))
        ));
    }
    assert!(matches!(
        get_attachment(&pool, id, Uuid::new_v4(), start, 1, &sha).await,
        Err(Error::NotFound)
    ));

    // The same file in the other company: stored once, read there by its own name.
    add_attachment(&pool, other, anna, start, 1, file("kopia.pdf", "a"), today).await.unwrap();
    let (theirs, _) = get_attachment(&pool, other, anna, start, 1, &sha).await.unwrap();
    assert_eq!(theirs.file_name.as_str(), "kopia.pdf");
    assert_eq!(table(&pool, "SELECT sha256 FROM attachment_files").await.len(), 1);
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-ledger --test store read_only_through`
Expected: compile error — `get_attachment` not found in `doris_ledger`.

- [ ] **Step 3: Implement** — in `queries.rs`, after `list_vouchers`:

```rust
/// One underlag and its bytes, found through the company's own voucher: a
/// hash alone never reads another company's file.
pub async fn get_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let row: Option<(String, String, i64, Vec<u8>)> = sqlx::query_as(
        "SELECT a.file_name, a.content_type, a.size, f.data
         FROM voucher_attachments a JOIN attachment_files f ON f.sha256 = a.sha256
         WHERE a.company_id = ? AND a.fiscal_year_start = ? AND a.number = ? AND a.sha256 = ?",
    )
    .bind(company_id.to_string())
    .bind(fiscal_year_start.to_string())
    .bind(number)
    .bind(sha256)
    .fetch_optional(pool)
    .await?;
    let (file_name, content_type, size, data) = row.ok_or(DomainError::AttachmentNotFound)?;
    Ok((
        projected_attachment(sha256.to_owned(), &file_name, &content_type, size),
        data,
    ))
}
```

Add `DomainError` to the `use crate::domain::{…}` in `queries.rs`, and `get_attachment` to `pub use queries::{…}` in `lib.rs`.

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p doris-ledger`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ledger
git commit -m "Read an underlag only through the company's own voucher

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 5: API — proto, server handlers and message limits

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`
- Modify: `crates/server/src/ledger.rs` (handlers, `voucher_message`, `attachment_message`, `new_attachments`, limit constants)
- Modify: `crates/server/src/lib.rs:37` (`LedgerServiceServer` limits)
- Modify: `crates/server/tests/common/mod.rs` (`ledger()` client decode limit)
- Modify: `crates/server/tests/ledger.rs` (`sale` and the `pb::Voucher` literal get `attachments: vec![]`; new tests)
- Modify: `crates/web/src/pages/new_voucher.rs:68-73` (`attachments: Vec::new()` so the workspace builds; Task 7 fills it)

**Interfaces:**
- Consumes: `doris_ledger::{NewAttachment, record_voucher_with_attachments, add_attachment, get_attachment}`, `Attachment` (Tasks 1–4).
- Produces (proto, `doris.ledger.v1`): messages `NewAttachment { file_name, data }`, `Attachment { id, file_name, content_type, size }`; `RecordVoucherRequest.attachments = 5`; `Voucher.attachments = 7`; RPCs `AddAttachment(AddAttachmentRequest{company_id, fiscal_year_start, number, attachment}) -> AddAttachmentResponse{attachment}` and `GetAttachment(GetAttachmentRequest{company_id, fiscal_year_start, number, id}) -> GetAttachmentResponse{attachment, data}`. Rust: `pb::NewAttachment { file_name: String, data: Vec<u8> }` etc.

- [ ] **Step 1: Write the failing tests** — in `crates/server/tests/common/mod.rs`, change `ledger()`:

```rust
    pub fn ledger(&self) -> Ledger {
        // Room for a 10 MiB underlag coming back from GetAttachment.
        LedgerServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
            .max_decoding_message_size(11 << 20)
    }
```

In `crates/server/tests/ledger.rs`, add `attachments: vec![],` to the `pb::RecordVoucherRequest` in `sale` and to the `pb::Voucher { … }` literal in `a_member_keeps_the_chart_and_books_and_corrects_vouchers`. Add `use common::Ledger;` to the imports and append:

```rust
const MIB: usize = 1 << 20;

/// A PDF of exactly `size` bytes.
fn pdf(size: usize) -> Vec<u8> {
    let mut data = b"%PDF-1.7\n".to_vec();
    data.resize(size, b'x');
    data
}

fn upload(name: &str, data: Vec<u8>) -> pb::NewAttachment {
    pb::NewAttachment { file_name: name.into(), data }
}

fn with_files(company_id: &str, files: Vec<pb::NewAttachment>) -> pb::RecordVoucherRequest {
    pb::RecordVoucherRequest { attachments: files, ..sale(company_id, 100) }
}

async fn refusal(api: &mut Ledger, session: &str, request: pb::RecordVoucherRequest) -> (Code, String) {
    code_of(api.record_voucher(authed(request, session)).await.unwrap_err())
}

#[tokio::test]
async fn underlag_go_up_with_a_voucher_and_come_back_byte_for_byte() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let big = pdf(10 * MIB);

    let booked = api
        .record_voucher(authed(with_files(&id, vec![upload("kvitto.pdf", big.clone())]), &anna))
        .await
        .unwrap()
        .into_inner();
    let listed = api
        .list_vouchers(authed(
            pb::ListVouchersRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers[0]
        .attachments
        .clone();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].file_name.as_str(), listed[0].content_type.as_str(), listed[0].size),
        ("kvitto.pdf", "application/pdf", (10 * MIB) as u64)
    );
    assert_eq!(listed[0].id.len(), 64);

    let got = api
        .get_attachment(authed(
            pb::GetAttachmentRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start.clone(),
                number: booked.number,
                id: listed[0].id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.attachment.as_ref(), Some(&listed[0]));
    assert!(got.data == big, "the bytes differ");

    let added = api
        .add_attachment(authed(
            pb::AddAttachmentRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start,
                number: booked.number,
                attachment: Some(upload("foto.png", b"\x89PNG\r\n\x1a\nbild".to_vec())),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .attachment
        .unwrap();
    assert_eq!((added.file_name.as_str(), added.content_type.as_str()), ("foto.png", "image/png"));

    // Two files of 10 MiB fit in one request.
    let mut other = pdf(10 * MIB);
    other[20] = b'y';
    api.record_voucher(authed(with_files(&id, vec![upload("a.pdf", pdf(10 * MIB)), upload("b.pdf", other)]), &anna))
        .await
        .unwrap();
}

#[tokio::test]
async fn attachment_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let invalid = |code: &str| (Code::InvalidArgument, code.to_owned());
    let mut other = pdf(10 * MIB);
    other[20] = b'y';

    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload("kvitto.pdf", pdf(10 * MIB + 1))])).await,
        invalid("attachment_too_large")
    );
    // 20 MiB + 10 bytes: over the per-request total, under the 21 MiB message limit.
    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload("a.pdf", pdf(10 * MIB)), upload("b.pdf", other), upload("c.pdf", pdf(10))])).await,
        invalid("attachment_too_large")
    );
    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload("bild.gif", b"GIF89a".to_vec())])).await,
        invalid("unsupported_attachment_type")
    );
    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload(" ", pdf(10))])).await,
        invalid("invalid_attachment_name")
    );
    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload("tom.pdf", vec![])])).await,
        invalid("empty_attachment")
    );

    let booked = api
        .record_voucher(authed(with_files(&id, vec![upload("kvitto.pdf", pdf(10))]), &anna))
        .await
        .unwrap()
        .into_inner();
    let add = |number: u32| pb::AddAttachmentRequest {
        company_id: id.clone(),
        fiscal_year_start: booked.fiscal_year_start.clone(),
        number,
        attachment: Some(upload("kopia.pdf", pdf(10))),
    };
    assert_eq!(
        code_of(api.add_attachment(authed(add(booked.number), &anna)).await.unwrap_err()),
        invalid("duplicate_attachment")
    );
    assert_eq!(
        code_of(api.add_attachment(authed(add(99), &anna)).await.unwrap_err()),
        (Code::NotFound, "voucher_not_found".into())
    );
    let get = pb::GetAttachmentRequest {
        company_id: id.clone(),
        fiscal_year_start: booked.fiscal_year_start.clone(),
        number: booked.number,
        id: "0".repeat(64),
    };
    assert_eq!(
        code_of(api.get_attachment(authed(get.clone(), &anna)).await.unwrap_err()),
        (Code::NotFound, "attachment_not_found".into())
    );
    let bertil = server.invite(&anna, "bertil@example.se").await;
    assert_eq!(
        code_of(api.get_attachment(authed(get, &bertil)).await.unwrap_err()),
        (Code::NotFound, "company_not_found".into())
    );
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-server --test ledger`
Expected: compile errors — no `attachments` field on `RecordVoucherRequest`/`Voucher`, no `NewAttachment`, no `get_attachment`/`add_attachment` methods.

- [ ] **Step 3: Implement**

`proto/doris/ledger/v1/ledger.proto`: add to `service LedgerService`:

```proto
  rpc AddAttachment(AddAttachmentRequest) returns (AddAttachmentResponse);
  rpc GetAttachment(GetAttachmentRequest) returns (GetAttachmentResponse);
```

add `repeated NewAttachment attachments = 5;` to `RecordVoucherRequest`, `repeated Attachment attachments = 7;` to `Voucher`, and the messages:

```proto
// An underlag as uploaded. The server decides its type from the bytes.
message NewAttachment {
  string file_name = 1;
  bytes data = 2;
}

message Attachment {
  string id = 1;           // sha256, lowercase hex
  string file_name = 2;
  string content_type = 3; // application/pdf, image/jpeg or image/png
  uint64 size = 4;
}

message AddAttachmentRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 number = 3;
  NewAttachment attachment = 4;
}

message AddAttachmentResponse {
  Attachment attachment = 1;
}

message GetAttachmentRequest {
  string company_id = 1;
  string fiscal_year_start = 2;
  uint32 number = 3;
  string id = 4;
}

message GetAttachmentResponse {
  Attachment attachment = 1;
  bytes data = 2;
}
```

`crates/server/src/ledger.rs`: extend the domain import with `Attachment`; add near the top:

```rust
/// Two 10 MiB underlag plus the rest of a `RecordVoucher`.
pub(crate) const MAX_REQUEST: usize = 21 << 20;
/// One 10 MiB underlag in a `GetAttachment` answer.
pub(crate) const MAX_RESPONSE: usize = 11 << 20;
/// The most underlag data one request may carry, all files together.
const MAX_ATTACHMENTS_PER_REQUEST: usize = 20 << 20;
```

In `record_voucher`, replace the booking call:

```rust
        let attachments = new_attachments(req.attachments)?;
        let booked = doris_ledger::record_voucher_with_attachments(
            &self.pool,
            company,
            user,
            cmd,
            attachments,
            today(),
        )
        .await
        .map_err(status)?;
```

(`cmd` is built before `req.attachments` is moved; build `cmd` from `req.date`, `req.text`, `&req.lines` first as today, then call `new_attachments(req.attachments)`.)

Add the two handlers inside `impl LedgerService for LedgerApi`:

```rust
    async fn add_attachment(
        &self,
        request: Request<pb::AddAttachmentRequest>,
    ) -> Result<Response<pb::AddAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.into_inner();
        // A missing attachment arrives as an empty name and is refused as such.
        let new = req.attachment.unwrap_or_default();
        let added = doris_ledger::add_attachment(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            doris_ledger::NewAttachment { file_name: new.file_name, data: new.data },
            today(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::AddAttachmentResponse {
            attachment: Some(attachment_message(&added)),
        }))
    }

    async fn get_attachment(
        &self,
        request: Request<pb::GetAttachmentRequest>,
    ) -> Result<Response<pb::GetAttachmentResponse>, Status> {
        let (company, user) = self.caller(&request, &request.get_ref().company_id).await?;
        let req = request.get_ref();
        let (attachment, data) = doris_ledger::get_attachment(
            &self.pool,
            company,
            user,
            date(&req.fiscal_year_start)?,
            req.number,
            &req.id,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::GetAttachmentResponse {
            attachment: Some(attachment_message(&attachment)),
            data,
        }))
    }
```

Below `domain_lines`:

```rust
/// The uploaded files, refused as a whole if together they are over the
/// per-request limit. Each file is checked by `doris_ledger`.
fn new_attachments(
    files: Vec<pb::NewAttachment>,
) -> Result<Vec<doris_ledger::NewAttachment>, Status> {
    if files.iter().map(|f| f.data.len()).sum::<usize>() > MAX_ATTACHMENTS_PER_REQUEST {
        return Err(domain_status(DomainError::AttachmentTooLarge));
    }
    Ok(files
        .into_iter()
        .map(|f| doris_ledger::NewAttachment { file_name: f.file_name, data: f.data })
        .collect())
}

fn attachment_message(a: &Attachment) -> pb::Attachment {
    pb::Attachment {
        id: a.sha256.clone(),
        file_name: a.file_name.as_str().to_owned(),
        content_type: a.content_type.as_mime().to_owned(),
        size: a.size,
    }
}
```

In `voucher_message`, add `attachments: v.attachments.iter().map(attachment_message).collect(),`.

`crates/server/src/lib.rs`: replace `.add_service(LedgerServiceServer::new(ledger))` with:

```rust
        .add_service(
            LedgerServiceServer::new(ledger)
                .max_decoding_message_size(ledger::MAX_REQUEST)
                .max_encoding_message_size(ledger::MAX_RESPONSE),
        )
```

`crates/web/src/pages/new_voucher.rs`: add `attachments: Vec::new(),` to the `lpb::RecordVoucherRequest` literal (Task 7 replaces it).

- [ ] **Step 4: Run to see it pass**

Run: `cargo test --workspace`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add proto crates/server crates/web/src/pages/new_voucher.rs
git commit -m "Serve underlag over gRPC-Web: upload with a voucher, add later, download

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 6: Frontend groundwork — messages, limits, file helpers

**Files:**
- Create: `crates/web/src/attachments.rs`
- Modify: `crates/web/src/main.rs` (`mod attachments;`)
- Modify: `crates/web/src/errors.rs` (6 messages, `describe_code`, test)
- Modify: `crates/web/src/api.rs:26-28` (decode limit)
- Modify: `crates/web/src/ui.rs` (`FileInput`, `PaperclipIcon`)
- Modify: `crates/web/Cargo.toml` (web-sys features)

**Interfaces:**
- Produces:
  - `attachments::MAX_FILE: usize`, `attachments::MAX_TOTAL: usize`
  - `attachments::check_sizes(files: &[lpb::NewAttachment]) -> Result<(), &'static str>` (the error is the code `attachment_too_large`)
  - `attachments::size_label(bytes: u64) -> String`
  - `async attachments::read_files(input: &web_sys::HtmlInputElement) -> Vec<lpb::NewAttachment>`
  - `attachments::open(content_type: &str, data: &[u8])`
  - `errors::describe_code(code: &str) -> String`
  - `ui::FileInput(label: String, id: String, on_pick: impl Fn(web_sys::HtmlInputElement) + 'static)`, `ui::PaperclipIcon()`

- [ ] **Step 1: Write the failing tests**

In `crates/web/src/errors.rs` `mod tests`, add:

```rust
    #[test]
    fn attachment_codes_have_swedish_messages() {
        for code in [
            "unsupported_attachment_type",
            "invalid_attachment_name",
            "empty_attachment",
            "attachment_too_large",
            "duplicate_attachment",
            "attachment_not_found",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(
            message("unsupported_attachment_type"),
            "Underlaget måste vara en PDF, JPEG eller PNG."
        );
    }
```

Create `crates/web/src/attachments.rs` with only the tests first:

```rust
//! Underlag in the browser: reading picked files, checking their size
//! before upload, and opening one in a new tab.

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: usize) -> lpb::NewAttachment {
        lpb::NewAttachment { file_name: "a.pdf".into(), data: vec![0; size] }
    }

    #[test]
    fn sizes_follow_the_servers_limits() {
        assert_eq!(check_sizes(&[file(MAX_FILE)]), Ok(()));
        assert_eq!(check_sizes(&[file(MAX_FILE + 1)]), Err("attachment_too_large"));
        assert_eq!(check_sizes(&[file(MAX_FILE), file(MAX_FILE)]), Ok(()));
        assert_eq!(
            check_sizes(&[file(MAX_FILE), file(MAX_FILE), file(1)]),
            Err("attachment_too_large")
        );
    }

    #[test]
    fn sizes_read_as_kb_or_mb_with_one_decimal() {
        assert_eq!(size_label(1), "1 kB");
        assert_eq!(size_label(120_000), "120 kB");
        assert_eq!(size_label(999_000), "999 kB");
        assert_eq!(size_label(999_001), "1,0 MB");
        assert_eq!(size_label(10_485_760), "10,5 MB");
    }
}
```

and add `mod attachments;` to `crates/web/src/main.rs` (alphabetically, after `mod app;`).

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p doris-web`
Expected: compile errors — `check_sizes`, `size_label`, `MAX_FILE`, `lpb` not found; then (once those exist) `attachment_codes_have_swedish_messages` FAILS on the fallback text.

- [ ] **Step 3: Implement**

`crates/web/Cargo.toml` — extend the `web-sys` features with `"Blob", "BlobPropertyBag", "File", "FileList", "HtmlInputElement", "Url"`.

`crates/web/src/attachments.rs`, above the tests:

```rust
use crate::api::lpb;
use leptos::prelude::{set_timeout, window};
use std::time::Duration;

/// The server's limits, mirrored so a pick that is too large is refused
/// before it is uploaded.
pub const MAX_FILE: usize = 10 << 20;
pub const MAX_TOTAL: usize = 20 << 20;

/// `Err("attachment_too_large")` when a file, or all of them together, are
/// over the limits.
pub fn check_sizes(files: &[lpb::NewAttachment]) -> Result<(), &'static str> {
    let total: usize = files.iter().map(|f| f.data.len()).sum();
    if files.iter().any(|f| f.data.len() > MAX_FILE) || total > MAX_TOTAL {
        Err("attachment_too_large")
    } else {
        Ok(())
    }
}

/// "120 kB", or "1,5 MB" from 1 MB up. Rounded up, never to 0.
pub fn size_label(bytes: u64) -> String {
    if bytes <= 999_000 {
        format!("{} kB", bytes.div_ceil(1000))
    } else {
        let tenths = bytes.div_ceil(100_000);
        format!("{},{} MB", tenths / 10, tenths % 10)
    }
}

/// The files picked in `input`, read into memory. The input is cleared so
/// the same file can be picked again.
pub async fn read_files(input: &web_sys::HtmlInputElement) -> Vec<lpb::NewAttachment> {
    let mut picked = Vec::new();
    if let Some(files) = input.files() {
        for i in 0..files.length() {
            let Some(file) = files.get(i) else { continue };
            let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await else {
                continue;
            };
            picked.push(lpb::NewAttachment {
                file_name: file.name(),
                data: js_sys::Uint8Array::new(&buffer).to_vec(),
            });
        }
    }
    input.set_value("");
    picked
}

/// Opens the file in a new tab, in the browser's own PDF or image viewer.
pub fn open(content_type: &str, data: &[u8]) {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(content_type);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    let _ = window().open_with_url_and_target(&url, "_blank");
    // The tab has loaded it long before then; free the memory.
    set_timeout(
        move || {
            let _ = web_sys::Url::revoke_object_url(&url);
        },
        Duration::from_secs(60),
    );
}
```

`crates/web/src/errors.rs`: add after `describe`:

```rust
/// The text for an error code found in the browser, before any API call.
pub fn describe_code(code: &str) -> String {
    message(code).to_owned()
}
```

and in `message`, before `_ =>`:

```rust
        "unsupported_attachment_type" => "Underlaget måste vara en PDF, JPEG eller PNG.",
        "invalid_attachment_name" => "Filnamnet är ogiltigt.",
        "empty_attachment" => "Filen är tom.",
        "attachment_too_large" => {
            "Underlaget är för stort (högst 10 MB per fil och 20 MB totalt)."
        }
        "duplicate_attachment" => "Underlaget finns redan på verifikationen.",
        "attachment_not_found" => "Underlaget hittades inte.",
```

`crates/web/src/api.rs`:

```rust
pub fn ledger_api() -> LedgerApi {
    // Room for a 10 MiB underlag coming back from GetAttachment.
    LedgerServiceClient::new(client()).max_decoding_message_size(11 << 20)
}
```

`crates/web/src/ui.rs`: add a constant after `INPUT` (shadcn Input's `file:` part):

```rust
const INPUT_FILE: &str = "file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-xs/relaxed file:font-medium file:text-foreground";
```

and the components:

```rust
/// A labelled native file picker for underlag (PDF, JPEG, PNG). On mobile
/// it offers the camera.
#[component]
pub fn FileInput(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    on_pick: impl Fn(web_sys::HtmlInputElement) + 'static,
) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <label for=id.clone() class=LABEL>
                {label}
            </label>
            <input
                id=id
                type="file"
                multiple
                accept="application/pdf,image/jpeg,image/png"
                class=format!("{INPUT} {INPUT_FILE}")
                on:change=move |ev| on_pick(event_target::<web_sys::HtmlInputElement>(&ev))
            />
        </div>
    }
}

/// lucide `paperclip`.
#[component]
pub fn PaperclipIcon() -> impl IntoView {
    view! {
        <svg
            class="size-3.5"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551" />
        </svg>
    }
}
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p doris-web && cargo build -p doris-web --target wasm32-unknown-unknown`
Expected: tests PASS and the wasm build succeeds. `read_files`, `open`, `FileInput`, `PaperclipIcon` and `describe_code` are unused until Task 7, so dead-code warnings are expected here; the `-D warnings` clippy runs belong to Task 7 and Task 8. Don't add `#[allow(dead_code)]`.

- [ ] **Step 5: Commit**

```bash
git add crates/web
git commit -m "Add Swedish messages, size checks and file helpers for underlag

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 7: Pick underlag when booking, see, open and add them in the grundbok

**Files:**
- Modify: `crates/web/src/pages/new_voucher.rs`
- Modify: `crates/web/src/pages/vouchers.rs`
- Create: `e2e/tests/attachments.spec.ts`

**Interfaces:**
- Consumes: everything Task 6 produces; `lpb::{NewAttachment, Attachment, AddAttachmentRequest, GetAttachmentRequest}` (Task 5).

- [ ] **Step 1: Write the failing e2e tests** — `e2e/tests/attachments.spec.ts`:

```ts
import type { Page } from "@playwright/test";
import { addCompany, expect, register, test } from "./fixtures";

// Last calendar year has always ended, so it can be closed.
const last = new Date().getFullYear() - 1;
const lastStart = `${last}-01-01`;

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function fillVoucher(page: Page, app: string, date?: string) {
  await page.goto(`${app}/vouchers/new`);
  if (date) await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Kontorsmaterial");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("125");
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill("125");
}

test("a voucher is booked with its underlag, which opens in a new tab", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await fillVoucher(page, app);
  await page.getByLabel("Underlag").setInputFiles([pdf("kvitto.pdf"), pdf("faktura.pdf")]);
  await expect(page.getByText("kvitto.pdf (1 kB)")).toBeVisible();
  await page.getByRole("button", { name: "Ta bort faktura.pdf" }).click();
  await expect(page.getByText("faktura.pdf")).toHaveCount(0);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await expect(page.getByText("kvitto.pdf")).toHaveCount(0);

  await page.goto(`${app}/vouchers`);
  const first = page.getByRole("row", { name: /^1 / });
  await expect(first).toContainText("1 underlag");
  await first.getByRole("button", { name: "1", exact: true }).click();
  const popup = page.waitForEvent("popup");
  await page.getByRole("button", { name: "kvitto.pdf (1 kB)" }).click();
  expect((await popup).url()).toMatch(/^blob:/);
});

test("underlag are added later, even in a closed year, and only as PDF, JPEG or PNG", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);
  await fillVoucher(page, app, `${last}-06-01`);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(lastYear).toContainText("Stängt");

  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(lastStart);
  const first = page.getByRole("row", { name: /^1 / });
  await first.getByRole("button", { name: "1", exact: true }).click();
  await page.getByLabel("Lägg till underlag till ver 1").setInputFiles([pdf("faktura.pdf")]);
  await expect(page.getByRole("button", { name: "faktura.pdf (1 kB)" })).toBeVisible();
  await expect(first).toContainText("1 underlag");

  await page
    .getByLabel("Lägg till underlag till ver 1")
    .setInputFiles([{ name: "bild.gif", mimeType: "image/gif", buffer: Buffer.from("GIF89a") }]);
  await expect(page.getByRole("alert")).toHaveText("Underlaget måste vara en PDF, JPEG eller PNG.");
});

test("picked underlag stay with the company they were picked for", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");

  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Underlag").setInputFiles([pdf("kvitto.pdf")]);
  await expect(page.getByText("kvitto.pdf (1 kB)")).toBeVisible();
  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByText("kvitto.pdf")).toHaveCount(0);
});
```

- [ ] **Step 2: Run to see it fail**

Run: `make e2e` (or, with a built frontend and server, `cd e2e && npx playwright test attachments`)
Expected: FAIL — no field labelled "Underlag".

- [ ] **Step 3: Implement**

`crates/web/src/pages/new_voucher.rs` — imports become:

```rust
use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::attachments::{check_sizes, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::format::today;
use crate::ui::{Button, Card, ErrorAlert, Field, FileInput, Variant};
use crate::voucher_lines::{LineRows, Lines};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
```

In `NewVoucher`, after `let lines = Lines::new();`:

```rust
    // Read into memory when picked; they go up with the voucher.
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
```

`clear` also empties them (it runs on a company switch and after booking):

```rust
    let clear = move || {
        text.set(String::new());
        lines.clear();
        files.set(Vec::new());
    };
```

After `clear`, add the picker handler:

```rust
    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = form_company.get_value();
        spawn_local(async move {
            let picked = read_files(&input).await;
            // Picked for a company that is no longer the form's: drop them.
            if company_id == form_company.get_value() {
                files.update(|f| f.extend(picked));
            }
        });
    };
```

In `submit`, after the `lines.request()` check:

```rust
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
```

and the request gets `attachments: files.get_untracked(),` (replacing Task 5's `Vec::new()`). A failed booking keeps the picked files, so the user can fix the voucher and try again.

In the view, after the `<div class="grid grid-cols-[10rem_1fr] gap-4">…</div>` block:

```rust
                <div class="grid gap-2">
                    <FileInput label="Underlag" id="voucher_files" on_pick=pick />
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
```

`crates/web/src/pages/vouchers.rs` — imports add:

```rust
use crate::attachments::{check_sizes, open, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::ui::{FileInput, PaperclipIcon};
```

(merge `FileInput, PaperclipIcon` into the existing `crate::ui::{…}` list and replace the `describe` import.)

Table header: add between "Status" and the empty actions header:

```rust
                        <th class=TABLE_HEADER_CELL><span class="sr-only">"Underlag"</span></th>
```

In `VoucherRow`, after `let lines = voucher.lines.clone();`:

```rust
    let attachments = RwSignal::new(voucher.attachments.clone());
    let open_attachment = move |id: String| {
        error.set(None);
        let Some(year) = fiscal_year.get_value() else {
            return;
        };
        spawn_local(async move {
            let result = ledger_api()
                .get_attachment(lpb::GetAttachmentRequest {
                    company_id: company_id.get_value(),
                    fiscal_year_start: year.start,
                    number,
                    id,
                })
                .await;
            match result {
                Ok(response) => {
                    let response = response.into_inner();
                    let mime = response.attachment.map(|a| a.content_type).unwrap_or_default();
                    open(&mime, &response.data);
                }
                Err(status) if company_id.get_value() == companies.active.get_untracked() => {
                    error.set(Some(describe(&status)))
                }
                Err(_) => {}
            }
        });
    };
    // One file per request; also in a closed year.
    let add_attachments = move |input: web_sys::HtmlInputElement| {
        error.set(None);
        let Some(year) = fiscal_year.get_value() else {
            return;
        };
        spawn_local(async move {
            let picked = read_files(&input).await;
            if let Err(code) = picked.iter().try_for_each(|f| check_sizes(std::slice::from_ref(f))) {
                return error.set(Some(describe_code(code)));
            }
            for file in picked {
                let result = ledger_api()
                    .add_attachment(lpb::AddAttachmentRequest {
                        company_id: company_id.get_value(),
                        fiscal_year_start: year.start.clone(),
                        number,
                        attachment: Some(file),
                    })
                    .await;
                match result {
                    Ok(response) => {
                        if let Some(added) = response.into_inner().attachment {
                            attachments.update(|list| list.push(added));
                        }
                    }
                    Err(status) => {
                        if company_id.get_value() == companies.active.get_untracked() {
                            error.set(Some(describe(&status)));
                        }
                        return;
                    }
                }
            }
        });
    };
```

In the row, add a cell between the status cell and the actions cell:

```rust
            <td class=TABLE_CELL>
                {move || {
                    let count = attachments.with(Vec::len);
                    (count > 0)
                        .then(|| view! {
                            <span class="inline-flex items-center gap-1 text-muted-foreground">
                                <PaperclipIcon />
                                {count}
                                <span class="sr-only">" underlag"</span>
                            </span>
                        })
                }}
            </td>
```

In the expanded row, change `colspan="5"` to `colspan="6"`, and after the `</ul>` of the lines add:

```rust
                    <div class="mt-3 grid gap-2">
                        <h2 class="text-xs/relaxed font-medium">"Underlag"</h2>
                        <ul class="grid gap-1">
                            {move || {
                                attachments
                                    .get()
                                    .into_iter()
                                    .map(|a| {
                                        let label = format!("{} ({})", a.file_name, size_label(a.size));
                                        view! {
                                            <li>
                                                <button
                                                    type="button"
                                                    class="underline-offset-4 hover:underline"
                                                    on:click=move |_| open_attachment(a.id.clone())
                                                >
                                                    {label}
                                                </button>
                                            </li>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </ul>
                        <div class="w-72">
                            <FileInput
                                label=format!("Lägg till underlag till ver {number}")
                                id=format!("attach_{number}")
                                on_pick=add_attachments
                            />
                        </div>
                    </div>
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo test --workspace && make e2e`
Expected: no clippy warnings; all Rust tests PASS; all Playwright tests PASS (existing `ledger.spec.ts` and `fiscal_year.spec.ts` included — the new column must not break their row selectors).

If the popup assertion fails because Chromium blocks `window.open` after the awaited RPC, keep the test and move the `window.open` to before the await: open `about:blank` synchronously in the click handler, then set the returned window's `location` to the Blob URL once the data has arrived.

- [ ] **Step 5: Commit**

```bash
git add crates/web e2e/tests/attachments.spec.ts
git commit -m "Pick underlag when booking, and open and add them in the grundbok

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```

---

### Task 8: Document the rules and verify the release build

**Files:**
- Modify: `AGENTS.md` (Event sourcing rules, API, Frontend sections)

- [ ] **Step 1: Update AGENTS.md**

Under "Event sourcing rules", after the bullet about closing a year, add:

```markdown
- Underlag (PDF, JPEG, PNG; `AttachmentAdded` in the ledger stream) keep
  their bytes in `attachment_files`: primary data like `events`, not a
  projection, append-only by trigger and keyed by SHA-256, so a file is
  stored once. They are read only through `voucher_attachments` for the
  company's own voucher, never by hash alone. An underlag is never removed
  or renamed, and it may be added to a voucher in a closed year: it changes
  no amount. The type comes from the bytes, never from the client.
```

Under "API", after the `LedgerService` codes bullet, add:

```markdown
- `LedgerService` also has `AddAttachment` and `GetAttachment`, and
  `RecordVoucher` takes underlag. Limits: 10 MiB per file, 20 MiB per
  request; the service accepts 21 MiB messages and sends up to 11 MiB (the
  other services keep tonic's 4 MiB). Codes: `unsupported_attachment_type`,
  `invalid_attachment_name`, `empty_attachment`, `attachment_too_large`,
  `duplicate_attachment` and `attachment_not_found`. File names are never
  logged.
```

Under "Frontend", after the `src/errors.rs` bullet, add:

```markdown
- `src/attachments.rs` reads picked files, checks the size limits before
  upload, and opens an underlag as a Blob URL in a new tab. `ledger_api()`
  raises its decode limit to 11 MiB for that.
```

- [ ] **Step 2: Run the full verification**

Run:

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings
make e2e
make dist
```

Expected: everything PASS; `make dist` prints the gzipped wasm size under the 500 000 byte budget.

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "Document underlag: storage, limits and error codes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01EbZRR11FzJ6p1XAunAyDgb"
```
