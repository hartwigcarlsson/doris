# Plan 1: Backend Core Implementation Plan (event store + identity)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Cargo workspace with an append-only SQLite event store and an event-sourced identity crate (users, passkeys, email-bound invitations), fully tested without WebAuthn or network.

**Architecture:** `doris-eventstore` owns the `events` table (append-only via triggers), opens the DB and runs `migrations/`. `doris-identity` has a pure `domain` module (decide/evolve, no I/O) and an async store API that, inside one `BEGIN IMMEDIATE` transaction, loads state, decides, appends and updates projections. WebAuthn credentials are opaque JSON here; Plan 2 plugs in webauthn-rs.

**Tech Stack:** Rust 2024, sqlx 0.9 (SQLite), serde/serde_json, jiff 0.2, uuid 1, sha2 0.11, base64 0.23, getrandom 0.4, thiserror 2, tokio (tests), tempfile (tests).

**Spec:** `docs/superpowers/specs/2026-09-27-inloggning-webauthn-design.md`

**Plan series:** This is Plan 1 of 4 for step 1 of the spec.
- Plan 1 (this): workspace, event store, identity domain, projections, invitations.
- Plan 2: WebAuthn ceremonies (webauthn-rs, SoftPasskey tests) and sessions.
- Plan 3: proto + gRPC-Web server, cookies, CORS, embedded assets.
- Plan 4: Leptos frontend (shadcn preset), Playwright e2e, `make dist`.

Plans 2–4 are written after this one lands, against the real code.

## Global Constraints
- TDD: every task writes failing tests first, runs them to see the failure, then implements. No production code without a red test first.
- The `events` table is append-only (triggers). Never UPDATE/DELETE events.
- Event payloads are JSON with `schema_version` (1 for all events in this plan); enums use `#[serde(tag = "type")]`.
- Projections update in the same transaction as the append and must be rebuildable (`rebuild_projections`).
- Invitation tokens are stored only as SHA-256 hex hashes; plaintext is returned once.
- Invitations are valid 7 days, single use, bound to one email; only admins create them.
- Email is normalized: trimmed, lowercased, ≤254 chars, exactly one `@`, domain contains a dot, no whitespace. Display name 1–100 chars, passkey name 1–64 chars (after trim).
- Code, identifiers, event names: English.
- SQLite pragmas: `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`.
- Refinement vs spec: `PasskeyUsed` carries the full updated credential JSON (`passkey`) instead of only a counter, because webauthn-rs updates the credential in place (`Passkey::update_credential`). There is no `projection_checkpoints` table: projections are synchronous, so there's nothing to checkpoint.

---

### Task 1: Workspace and event store

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `Makefile`
- Create: `migrations/0001_events.sql`
- Create: `crates/eventstore/Cargo.toml`, `crates/eventstore/src/lib.rs`
- Test: `crates/eventstore/tests/eventstore.rs`

**Interfaces:**
- Produces (used by Task 3 and later plans):
  - `doris_eventstore::open(url: &str) -> Result<SqlitePool, Error>` (creates the file, sets pragmas, runs `migrations/`; `sqlite::memory:` uses one connection)
  - `begin(pool) -> Result<Transaction<'static, Sqlite>, Error>` (BEGIN IMMEDIATE)
  - `append(conn, stream, expected_version, &[NewEvent], &Metadata) -> Result<Vec<RecordedEvent>, Error>`
  - `load(conn, stream)`, `read_all(conn, after_position)`, `stream_version(conn, stream)`
  - `NewEvent::from_tagged(&T, schema_version)`, `RecordedEvent::decode::<T>()`
  - `Metadata { actor: Option<String> }`
  - `Error::{Conflict, Db, Migrate, Json}`

- [ ] **Step 1: Create the workspace skeleton**

`Cargo.toml` (Task 2 adds `crates/identity` to `members`):

```toml
[workspace]
resolver = "3"
members = ["crates/eventstore"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
doris-eventstore = { path = "crates/eventstore" }
base64 = "0.23"
getrandom = "0.4"
jiff = { version = "0.2", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.11"
sqlx = { version = "0.9", features = ["sqlite", "runtime-tokio", "migrate"] }
tempfile = "3"
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

`.gitignore`:

```
/target
*.db
*.db-shm
*.db-wal
```

`Makefile` (Plans 3–4 add `dev`, `e2e`, `dist`):

```make
.PHONY: test
test:
	cargo test --workspace
```

`crates/eventstore/Cargo.toml`:

```toml
[package]
name = "doris-eventstore"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true

[dev-dependencies]
tokio.workspace = true
```

`crates/eventstore/src/lib.rs` starts empty except for its doc comment:

```rust
//! Append-only event log in SQLite: the source of truth for Doris.
```

- [ ] **Step 2: Write the failing tests**

`crates/eventstore/tests/eventstore.rs`:

```rust
use doris_eventstore::{Error, Metadata, NewEvent, append, begin, load, open, read_all};
use serde_json::json;

fn event(n: i64) -> NewEvent {
    NewEvent {
        event_type: "Counted".into(),
        schema_version: 1,
        payload: json!({ "type": "Counted", "n": n }),
    }
}

fn actor() -> Metadata {
    Metadata {
        actor: Some("tester".into()),
    }
}

#[tokio::test]
async fn appended_events_are_loaded_in_stream_order() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut tx = begin(&pool).await.unwrap();
    append(&mut tx, "counter-1", 0, &[event(1), event(2)], &actor())
        .await
        .unwrap();
    append(&mut tx, "counter-1", 2, &[event(3)], &actor())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let events = load(&mut conn, "counter-1").await.unwrap();
    let versions: Vec<i64> = events.iter().map(|e| e.stream_version).collect();
    let ns: Vec<i64> = events
        .iter()
        .map(|e| e.payload["n"].as_i64().unwrap())
        .collect();
    assert_eq!(versions, [1, 2, 3]);
    assert_eq!(ns, [1, 2, 3]);
    assert_eq!(events[0].metadata, actor());
    assert!(events[0].recorded_at.ends_with('Z'));
}

#[tokio::test]
async fn append_with_stale_expected_version_is_a_conflict() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "counter-1", 0, &[event(1)], &actor())
        .await
        .unwrap();

    let err = append(&mut conn, "counter-1", 0, &[event(2)], &actor())
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            Error::Conflict {
                expected: 0,
                actual: 1,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(load(&mut conn, "counter-1").await.unwrap().len(), 1);
}

#[tokio::test]
async fn events_can_never_be_updated_or_deleted() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "counter-1", 0, &[event(1)], &actor())
        .await
        .unwrap();

    let update = sqlx::query("UPDATE events SET payload = '{}'")
        .execute(&mut *conn)
        .await
        .unwrap_err();
    let delete = sqlx::query("DELETE FROM events")
        .execute(&mut *conn)
        .await
        .unwrap_err();

    assert!(update.to_string().contains("append-only"), "{update}");
    assert!(delete.to_string().contains("append-only"), "{delete}");
}

#[tokio::test]
async fn read_all_returns_events_across_streams_after_a_position() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "a", 0, &[event(1)], &actor())
        .await
        .unwrap();
    append(&mut conn, "b", 0, &[event(2)], &actor())
        .await
        .unwrap();
    append(&mut conn, "a", 1, &[event(3)], &actor())
        .await
        .unwrap();

    let all = read_all(&mut conn, 0).await.unwrap();
    let after_first = read_all(&mut conn, all[0].global_position).await.unwrap();

    let streams: Vec<&str> = all.iter().map(|e| e.stream_id.as_str()).collect();
    assert_eq!(streams, ["a", "b", "a"]);
    assert_eq!(after_first.len(), 2);
    assert_eq!(after_first[0].stream_id, "b");
}

#[test]
fn from_tagged_takes_event_type_from_serde_tag() {
    #[derive(serde::Serialize)]
    #[serde(tag = "type")]
    enum Thing {
        Happened { x: i32 },
    }

    let event = NewEvent::from_tagged(&Thing::Happened { x: 7 }, 1).unwrap();

    assert_eq!(event.event_type, "Happened");
    assert_eq!(event.payload, json!({ "type": "Happened", "x": 7 }));
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-eventstore`
Expected: FAIL. The build error is `unresolved imports doris_eventstore::Error, ...`.

- [ ] **Step 4: Write the migration**

`migrations/0001_events.sql`:

```sql
CREATE TABLE events (
    global_position INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id       TEXT    NOT NULL,
    stream_version  INTEGER NOT NULL,
    event_type      TEXT    NOT NULL,
    schema_version  INTEGER NOT NULL,
    payload         TEXT    NOT NULL CHECK (json_valid(payload)),
    metadata        TEXT    NOT NULL CHECK (json_valid(metadata)),
    recorded_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (stream_id, stream_version)
);
CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
```

- [ ] **Step 5: Implement the event store**

`crates/eventstore/src/lib.rs`:

```rust
//! Append-only event log in SQLite: the source of truth for Doris.
//!
//! Writers must hold an IMMEDIATE transaction (see [`begin`]) so version
//! checks and projection updates are serialized.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("stream {stream} is at version {actual}, expected {expected}")]
    Conflict {
        stream: String,
        expected: i64,
        actual: i64,
    },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Who caused an event. Stored with every event (behandlingshistorik).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub actor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub event_type: String,
    pub schema_version: i64,
    pub payload: Value,
}

impl NewEvent {
    /// Builds an event from a serde enum tagged with `#[serde(tag = "type")]`.
    pub fn from_tagged<T: Serialize>(event: &T, schema_version: i64) -> Result<Self, Error> {
        let payload = serde_json::to_value(event)?;
        let event_type = payload["type"]
            .as_str()
            .expect("events must use #[serde(tag = \"type\")]")
            .to_owned();
        Ok(Self {
            event_type,
            schema_version,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedEvent {
    pub global_position: i64,
    pub stream_id: String,
    pub stream_version: i64,
    pub event_type: String,
    pub schema_version: i64,
    pub payload: Value,
    pub metadata: Metadata,
    pub recorded_at: String,
}

impl RecordedEvent {
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }

    fn from_row(row: &SqliteRow) -> Result<Self, Error> {
        Ok(Self {
            global_position: row.try_get("global_position")?,
            stream_id: row.try_get("stream_id")?,
            stream_version: row.try_get("stream_version")?,
            event_type: row.try_get("event_type")?,
            schema_version: row.try_get("schema_version")?,
            payload: serde_json::from_str(row.try_get("payload")?)?,
            metadata: serde_json::from_str(row.try_get("metadata")?)?,
            recorded_at: row.try_get("recorded_at")?,
        })
    }
}

/// Opens (creating if missing) the database and runs all migrations.
pub async fn open(url: &str) -> Result<SqlitePool, Error> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true);
    // Every connection to `:memory:` is its own database, so keep exactly one.
    let max_connections = if url.contains(":memory:") { 1 } else { 8 };
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

/// Starts a write transaction that takes the SQLite write lock up front.
pub async fn begin(pool: &SqlitePool) -> Result<Transaction<'static, Sqlite>, Error> {
    Ok(pool.begin_with("BEGIN IMMEDIATE").await?)
}

/// Current version of a stream; 0 when it has no events.
pub async fn stream_version(conn: &mut SqliteConnection, stream: &str) -> Result<i64, Error> {
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(stream_version) FROM events WHERE stream_id = ?")
            .bind(stream)
            .fetch_one(&mut *conn)
            .await?;
    Ok(version.unwrap_or(0))
}

/// Appends events to a stream, failing with [`Error::Conflict`] unless the
/// stream is at `expected_version`.
pub async fn append(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[NewEvent],
    metadata: &Metadata,
) -> Result<Vec<RecordedEvent>, Error> {
    let actual = stream_version(conn, stream).await?;
    if actual != expected_version {
        return Err(Error::Conflict {
            stream: stream.to_owned(),
            expected: expected_version,
            actual,
        });
    }
    let metadata = serde_json::to_string(metadata)?;
    let mut recorded = Vec::with_capacity(events.len());
    for (version, event) in (expected_version + 1..).zip(events) {
        let row = sqlx::query(
            "INSERT INTO events (stream_id, stream_version, event_type, schema_version, payload, metadata)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(stream)
        .bind(version)
        .bind(&event.event_type)
        .bind(event.schema_version)
        .bind(event.payload.to_string())
        .bind(&metadata)
        .fetch_one(&mut *conn)
        .await?;
        recorded.push(RecordedEvent::from_row(&row)?);
    }
    Ok(recorded)
}

pub async fn load(conn: &mut SqliteConnection, stream: &str) -> Result<Vec<RecordedEvent>, Error> {
    sqlx::query("SELECT * FROM events WHERE stream_id = ? ORDER BY stream_version")
        .bind(stream)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(RecordedEvent::from_row)
        .collect()
}

/// All events after `position`, in global order.
// ponytail: loads everything into memory; stream in batches when the log gets large.
pub async fn read_all(
    conn: &mut SqliteConnection,
    position: i64,
) -> Result<Vec<RecordedEvent>, Error> {
    sqlx::query("SELECT * FROM events WHERE global_position > ? ORDER BY global_position")
        .bind(position)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(RecordedEvent::from_row)
        .collect()
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p doris-eventstore`
Expected: PASS, 5 tests.

- [ ] **Step 7: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no output from clippy.

```bash
git add Cargo.toml Cargo.lock .gitignore Makefile migrations crates/eventstore
git commit -m "Add append-only SQLite event store"
```

---

### Task 2: Identity domain (pure)

**Files:**
- Modify: `Cargo.toml` (members → `["crates/eventstore", "crates/identity"]`)
- Create: `crates/identity/Cargo.toml`, `crates/identity/src/lib.rs`, `crates/identity/src/domain.rs`
- Test: `crates/identity/tests/domain.rs`

**Interfaces:**
- Consumes: nothing from Task 1. The domain has no I/O.
- Produces (module `doris_identity::domain`, used by Task 3 and Plan 2):
  - `Email::parse(&str) -> Result<Email, DomainError>`, `Email::as_str`
  - `DisplayName::parse`, `DisplayName::as_str`
  - `Role::{Admin, Member}`, `Role::as_str` (`"admin"` / `"member"`)
  - `Passkey { credential_id: String, name: String, passkey: serde_json::Value }`, `Passkey::new(credential_id, name, passkey)`
  - `UserEvent::{UserRegistered, PasskeyAdded, PasskeyUsed}`, `InvitationEvent::{InvitationCreated, InvitationAccepted}`
  - `User { id, email, display_name, role, passkeys }`, `User::from_events`
  - `Invitation { id, email, token_hash, created_by, expires_at, accepted_by }`, `Invitation::from_events`
  - `Admission::{Bootstrap, Invited(&Invitation), Uninvited}`
  - `RegisterUser { user_id, email, display_name, passkey }`
  - `register_user(admission, cmd, now) -> Result<(Vec<UserEvent>, Option<InvitationEvent>), DomainError>`
  - `add_passkey(&User, Passkey) -> Result<Vec<UserEvent>, DomainError>`
  - `record_passkey_use(&User, credential_id, passkey_json) -> Result<Vec<UserEvent>, DomainError>`
  - `create_invitation(&User, id, Email, token_hash, now) -> Result<InvitationEvent, DomainError>`
  - `INVITATION_TTL` (7 days)
  - `DomainError`: `InvalidEmail`, `InvalidDisplayName`, `InvalidPasskeyName`, `InvitationRequired`, `InvitationExpired`, `InvitationAlreadyUsed`, `InvitationEmailMismatch`, `DuplicatePasskey`, `UnknownPasskey`, `NotAdmin`

- [ ] **Step 1: Create the crate**

`crates/identity/Cargo.toml` (the full dependency list Task 3 needs too; unused ones are harmless for one task):

```toml
[package]
name = "doris-identity"
version.workspace = true
edition.workspace = true

[dependencies]
base64.workspace = true
doris-eventstore.workspace = true
getrandom.workspace = true
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

`crates/identity/src/lib.rs` for now:

```rust
//! Users, passkeys and invitations, event-sourced into SQLite.

pub mod domain;
```

Create `crates/identity/src/domain.rs` containing only the doc line `//! Pure identity rules: value types, events, state and decisions. No I/O.`

- [ ] **Step 2: Write the failing tests**

`crates/identity/tests/domain.rs`:

```rust
use doris_identity::domain::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str) -> Passkey {
    Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap()
}

fn cmd(email: &str) -> RegisterUser {
    RegisterUser {
        user_id: Uuid::new_v4(),
        email: Email::parse(email).unwrap(),
        display_name: DisplayName::parse("Anna Andersson").unwrap(),
        passkey: passkey("cred-1"),
    }
}

fn user(role: Role) -> User {
    User {
        id: Uuid::new_v4(),
        email: Email::parse("anna@example.se").unwrap(),
        display_name: DisplayName::parse("Anna").unwrap(),
        role,
        passkeys: vec![passkey("cred-1")],
    }
}

fn invitation(email: &str) -> Invitation {
    Invitation {
        id: Uuid::new_v4(),
        email: Email::parse(email).unwrap(),
        token_hash: "hash".into(),
        created_by: Uuid::new_v4(),
        expires_at: now() + SignedDuration::from_hours(1),
        accepted_by: None,
    }
}

#[test]
fn email_is_trimmed_and_lowercased() {
    let email = Email::parse("  Anna.Andersson@Example.SE ").unwrap();
    assert_eq!(email.as_str(), "anna.andersson@example.se");
}

#[test]
fn malformed_emails_are_rejected() {
    let long = format!("{}@example.se", "a".repeat(250));
    for raw in [
        "",
        "anna",
        "@example.se",
        "anna@",
        "anna@example",
        "a@b@c.se",
        "an na@example.se",
        "anna@.se",
        "anna@example.",
        &long,
    ] {
        assert_eq!(Email::parse(raw), Err(DomainError::InvalidEmail), "{raw:?}");
    }
}

#[test]
fn display_name_must_be_1_to_100_characters() {
    assert_eq!(DisplayName::parse(" Åsa ").unwrap().as_str(), "Åsa");
    assert!(DisplayName::parse(&"å".repeat(100)).is_ok());
    assert_eq!(
        DisplayName::parse("   "),
        Err(DomainError::InvalidDisplayName)
    );
    assert_eq!(
        DisplayName::parse(&"å".repeat(101)),
        Err(DomainError::InvalidDisplayName)
    );
}

#[test]
fn passkey_name_must_be_1_to_64_characters() {
    assert!(Passkey::new("c".into(), &"x".repeat(64), json!({})).is_ok());
    assert_eq!(
        Passkey::new("c".into(), " ", json!({})),
        Err(DomainError::InvalidPasskeyName)
    );
    assert_eq!(
        Passkey::new("c".into(), &"x".repeat(65), json!({})),
        Err(DomainError::InvalidPasskeyName)
    );
}

#[test]
fn bootstrap_registration_creates_an_admin_with_a_passkey() {
    let cmd = cmd("anna@example.se");
    let user_id = cmd.user_id;

    let (events, accepted) = register_user(Admission::Bootstrap, cmd, now()).unwrap();

    let user = User::from_events(&events).unwrap();
    assert_eq!(user.id, user_id);
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.passkeys, [passkey("cred-1")]);
    assert_eq!(accepted, None);
}

#[test]
fn registration_without_invitation_after_bootstrap_is_rejected() {
    let err = register_user(Admission::Uninvited, cmd("anna@example.se"), now()).unwrap_err();
    assert_eq!(err, DomainError::InvitationRequired);
}

#[test]
fn invited_registration_creates_a_member_and_accepts_the_invitation() {
    let invitation = invitation("bo@example.se");
    let cmd = cmd("bo@example.se");
    let user_id = cmd.user_id;

    let (events, accepted) = register_user(Admission::Invited(&invitation), cmd, now()).unwrap();

    assert_eq!(User::from_events(&events).unwrap().role, Role::Member);
    assert!(matches!(
        &events[0],
        UserEvent::UserRegistered { invitation_id: Some(id), .. } if *id == invitation.id
    ));
    assert_eq!(
        accepted,
        Some(InvitationEvent::InvitationAccepted { user_id })
    );
}

#[test]
fn expired_used_or_mismatched_invitations_are_rejected() {
    let mut expired = invitation("bo@example.se");
    expired.expires_at = now();
    let mut used = invitation("bo@example.se");
    used.accepted_by = Some(Uuid::new_v4());
    let other = invitation("cecilia@example.se");

    let register = |inv: &Invitation| {
        register_user(Admission::Invited(inv), cmd("bo@example.se"), now()).unwrap_err()
    };

    assert_eq!(register(&expired), DomainError::InvitationExpired);
    assert_eq!(register(&used), DomainError::InvitationAlreadyUsed);
    assert_eq!(register(&other), DomainError::InvitationEmailMismatch);
}

#[test]
fn adding_a_passkey_the_user_already_has_is_rejected() {
    let user = user(Role::Member);
    assert_eq!(
        add_passkey(&user, passkey("cred-1")),
        Err(DomainError::DuplicatePasskey)
    );

    let events = add_passkey(&user, passkey("cred-2")).unwrap();
    let mut all = vec![UserEvent::UserRegistered {
        user_id: user.id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        role: user.role,
        invitation_id: None,
    }];
    all.extend(events);
    assert_eq!(
        User::from_events(&all).unwrap().passkeys,
        [passkey("cred-2")]
    );
}

#[test]
fn passkey_use_replaces_the_stored_credential() {
    let user = user(Role::Member);
    assert_eq!(
        record_passkey_use(&user, "nope", json!({})),
        Err(DomainError::UnknownPasskey)
    );

    let events = record_passkey_use(&user, "cred-1", json!({ "counter": 5 })).unwrap();

    let registered = UserEvent::UserRegistered {
        user_id: user.id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        role: user.role,
        invitation_id: None,
    };
    let added = UserEvent::PasskeyAdded {
        credential_id: "cred-1".into(),
        name: "Laptop".into(),
        passkey: json!({}),
    };
    let state = User::from_events([&registered, &added].into_iter().chain(&events)).unwrap();
    assert_eq!(state.passkeys[0].passkey, json!({ "counter": 5 }));
}

#[test]
fn only_admins_create_invitations_valid_for_seven_days() {
    let email = Email::parse("bo@example.se").unwrap();
    let id = Uuid::new_v4();

    assert_eq!(
        create_invitation(&user(Role::Member), id, email.clone(), "h".into(), now()),
        Err(DomainError::NotAdmin)
    );

    let admin = user(Role::Admin);
    let event = create_invitation(&admin, id, email, "h".into(), now()).unwrap();
    let invitation = Invitation::from_events([&event]).unwrap();
    assert_eq!(invitation.created_by, admin.id);
    assert_eq!(
        invitation.expires_at,
        now() + SignedDuration::from_hours(24 * 7)
    );
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test domain`
Expected: FAIL. The build fails with unresolved names (`Email`, `Passkey`, `register_user`, ...).

- [ ] **Step 4: Implement the domain**

`crates/identity/src/domain.rs`:

```rust
//! Pure identity rules: value types, events, state and decisions. No I/O.

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const INVITATION_TTL: SignedDuration = SignedDuration::from_hours(24 * 7);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid email address")]
    InvalidEmail,
    #[error("display name must be 1-100 characters")]
    InvalidDisplayName,
    #[error("passkey name must be 1-64 characters")]
    InvalidPasskeyName,
    #[error("registration requires an invitation")]
    InvitationRequired,
    #[error("invitation has expired")]
    InvitationExpired,
    #[error("invitation has already been used")]
    InvitationAlreadyUsed,
    #[error("email does not match the invitation")]
    InvitationEmailMismatch,
    #[error("passkey is already registered")]
    DuplicatePasskey,
    #[error("unknown passkey")]
    UnknownPasskey,
    #[error("only admins may do this")]
    NotAdmin,
}

/// Normalized (trimmed, lowercase) email address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(String);

impl Email {
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

fn bounded_text(raw: &str, max_chars: usize) -> Option<String> {
    let text = raw.trim();
    (!text.is_empty() && text.chars().count() <= max_chars).then(|| text.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded_text(raw, 100)
            .map(Self)
            .ok_or(DomainError::InvalidDisplayName)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
}

/// A registered WebAuthn credential. `passkey` is the serialized
/// `webauthn_rs::prelude::Passkey`; this crate treats it as opaque.
#[derive(Debug, Clone, PartialEq)]
pub struct Passkey {
    pub credential_id: String,
    pub name: String,
    pub passkey: Value,
}

impl Passkey {
    pub fn new(credential_id: String, name: &str, passkey: Value) -> Result<Self, DomainError> {
        let name = bounded_text(name, 64).ok_or(DomainError::InvalidPasskeyName)?;
        Ok(Self {
            credential_id,
            name,
            passkey,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UserEvent {
    UserRegistered {
        user_id: Uuid,
        email: Email,
        display_name: DisplayName,
        role: Role,
        invitation_id: Option<Uuid>,
    },
    PasskeyAdded {
        credential_id: String,
        name: String,
        passkey: Value,
    },
    /// A successful login. Carries the credential with its updated counter.
    PasskeyUsed {
        credential_id: String,
        passkey: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InvitationEvent {
    InvitationCreated {
        invitation_id: Uuid,
        email: Email,
        token_hash: String,
        created_by: Uuid,
        expires_at: Timestamp,
    },
    InvitationAccepted {
        user_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: Uuid,
    pub email: Email,
    pub display_name: DisplayName,
    pub role: Role,
    pub passkeys: Vec<Passkey>,
}

impl User {
    /// Folds a user stream into state; `None` for an empty stream.
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a UserEvent>) -> Option<Self> {
        let mut user: Option<Self> = None;
        for event in events {
            match (event, user.as_mut()) {
                (
                    UserEvent::UserRegistered {
                        user_id,
                        email,
                        display_name,
                        role,
                        ..
                    },
                    _,
                ) => {
                    user = Some(Self {
                        id: *user_id,
                        email: email.clone(),
                        display_name: display_name.clone(),
                        role: *role,
                        passkeys: Vec::new(),
                    });
                }
                (
                    UserEvent::PasskeyAdded {
                        credential_id,
                        name,
                        passkey,
                    },
                    Some(user),
                ) => user.passkeys.push(Passkey {
                    credential_id: credential_id.clone(),
                    name: name.clone(),
                    passkey: passkey.clone(),
                }),
                (
                    UserEvent::PasskeyUsed {
                        credential_id,
                        passkey,
                    },
                    Some(user),
                ) => {
                    if let Some(p) = user
                        .passkeys
                        .iter_mut()
                        .find(|p| &p.credential_id == credential_id)
                    {
                        p.passkey = passkey.clone();
                    }
                }
                (_, None) => {}
            }
        }
        user
    }

    fn passkey(&self, credential_id: &str) -> Option<&Passkey> {
        self.passkeys
            .iter()
            .find(|p| p.credential_id == credential_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invitation {
    pub id: Uuid,
    pub email: Email,
    pub token_hash: String,
    pub created_by: Uuid,
    pub expires_at: Timestamp,
    pub accepted_by: Option<Uuid>,
}

impl Invitation {
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a InvitationEvent>) -> Option<Self> {
        let mut invitation: Option<Self> = None;
        for event in events {
            match (event, invitation.as_mut()) {
                (
                    InvitationEvent::InvitationCreated {
                        invitation_id,
                        email,
                        token_hash,
                        created_by,
                        expires_at,
                    },
                    _,
                ) => {
                    invitation = Some(Self {
                        id: *invitation_id,
                        email: email.clone(),
                        token_hash: token_hash.clone(),
                        created_by: *created_by,
                        expires_at: *expires_at,
                        accepted_by: None,
                    });
                }
                (InvitationEvent::InvitationAccepted { user_id }, Some(inv)) => {
                    inv.accepted_by = Some(*user_id);
                }
                (_, None) => {}
            }
        }
        invitation
    }
}

/// On what grounds someone is allowed to register.
#[derive(Debug, Clone, Copy)]
pub enum Admission<'a> {
    /// No users exist yet: the first user becomes admin.
    Bootstrap,
    Invited(&'a Invitation),
    Uninvited,
}

#[derive(Debug, Clone)]
pub struct RegisterUser {
    pub user_id: Uuid,
    pub email: Email,
    pub display_name: DisplayName,
    pub passkey: Passkey,
}

/// Events for the new user's stream, plus the invitation event if one was used.
pub fn register_user(
    admission: Admission<'_>,
    cmd: RegisterUser,
    now: Timestamp,
) -> Result<(Vec<UserEvent>, Option<InvitationEvent>), DomainError> {
    let (role, invitation_id) = match admission {
        Admission::Bootstrap => (Role::Admin, None),
        Admission::Uninvited => return Err(DomainError::InvitationRequired),
        Admission::Invited(invitation) => {
            if invitation.accepted_by.is_some() {
                return Err(DomainError::InvitationAlreadyUsed);
            }
            if now >= invitation.expires_at {
                return Err(DomainError::InvitationExpired);
            }
            if invitation.email != cmd.email {
                return Err(DomainError::InvitationEmailMismatch);
            }
            (Role::Member, Some(invitation.id))
        }
    };
    let user_events = vec![
        UserEvent::UserRegistered {
            user_id: cmd.user_id,
            email: cmd.email,
            display_name: cmd.display_name,
            role,
            invitation_id,
        },
        UserEvent::PasskeyAdded {
            credential_id: cmd.passkey.credential_id,
            name: cmd.passkey.name,
            passkey: cmd.passkey.passkey,
        },
    ];
    let accepted = invitation_id.map(|_| InvitationEvent::InvitationAccepted {
        user_id: cmd.user_id,
    });
    Ok((user_events, accepted))
}

pub fn add_passkey(user: &User, passkey: Passkey) -> Result<Vec<UserEvent>, DomainError> {
    if user.passkey(&passkey.credential_id).is_some() {
        return Err(DomainError::DuplicatePasskey);
    }
    Ok(vec![UserEvent::PasskeyAdded {
        credential_id: passkey.credential_id,
        name: passkey.name,
        passkey: passkey.passkey,
    }])
}

pub fn record_passkey_use(
    user: &User,
    credential_id: &str,
    passkey: Value,
) -> Result<Vec<UserEvent>, DomainError> {
    user.passkey(credential_id)
        .ok_or(DomainError::UnknownPasskey)?;
    Ok(vec![UserEvent::PasskeyUsed {
        credential_id: credential_id.to_owned(),
        passkey,
    }])
}

pub fn create_invitation(
    creator: &User,
    invitation_id: Uuid,
    email: Email,
    token_hash: String,
    now: Timestamp,
) -> Result<InvitationEvent, DomainError> {
    if creator.role != Role::Admin {
        return Err(DomainError::NotAdmin);
    }
    Ok(InvitationEvent::InvitationCreated {
        invitation_id,
        email,
        token_hash,
        created_by: creator.id,
        expires_at: now + INVITATION_TTL,
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p doris-identity --test domain`
Expected: PASS, 11 tests.

- [ ] **Step 6: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add Cargo.toml Cargo.lock crates/identity
git commit -m "Add pure identity domain: users, passkeys, invitations"
```

---

### Task 3: Identity store: projections, registration, invitations

**Files:**
- Create: `migrations/0002_identity.sql`
- Create: `crates/identity/src/token.rs`, `crates/identity/src/projections.rs`
- Modify: `crates/identity/src/lib.rs` (full content below)
- Test: `crates/identity/tests/store.rs`

**Interfaces:**
- Consumes: Task 1 (`open`, `begin`, `append`, `load`, `read_all`, `NewEvent::from_tagged`, `RecordedEvent::decode`, `Metadata`) and Task 2 (`domain::*`).
- Produces (used by Plans 2–3). Every function takes `now: jiff::Timestamp` where time matters.
  - `doris_identity::register(pool, email, display_name, invitation_token: Option<&str>, passkey: Passkey, now) -> Result<User>`
  - `create_invitation(pool, creator_id: Uuid, email, now) -> Result<(Uuid, String /*plaintext token*/)>`
  - `add_passkey(pool, user_id, Passkey) -> Result<()>`
  - `record_passkey_use(pool, user_id, credential_id, passkey_json) -> Result<()>`
  - `get_user(pool, user_id) -> Result<Option<User>>`, `find_user_by_email(pool, email) -> Result<Option<User>>`
  - `rebuild_projections(pool) -> Result<()>`
  - `token::new_token() -> String` (base64url, 256 bits), `token::hash_token(&str) -> String` (hex SHA-256). Plan 2's sessions reuse both.
  - `Error::{Domain(DomainError), AlreadyExists, InvitationNotFound, UserNotFound, Store(doris_eventstore::Error)}`. A UNIQUE violation anywhere in a write becomes `AlreadyExists`, and the whole transaction rolls back.
  - Projection tables `users`, `passkeys`, `invitations` (schema below)

- [ ] **Step 1: Write the failing tests**

`crates/identity/tests/store.rs`:

```rust
use doris_identity::domain::{DomainError, Passkey, Role};
use doris_identity::{
    Error, add_passkey, create_invitation, find_user_by_email, get_user, rebuild_projections,
    record_passkey_use, register,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str) -> Passkey {
    Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap()
}

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

async fn event_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn first_user_becomes_admin_and_is_findable_by_email() {
    let pool = db().await;

    let anna = register(&pool, "Anna@Example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    assert_eq!(anna.role, Role::Admin);
    let found = find_user_by_email(&pool, "ANNA@example.se")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found, anna);
    assert_eq!(get_user(&pool, anna.id).await.unwrap(), Some(anna));
    assert_eq!(
        find_user_by_email(&pool, "nobody@example.se")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn second_user_needs_an_invitation() {
    let pool = db().await;
    register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    let err = register(&pool, "bo@example.se", "Bo", None, passkey("c2"), now())
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Domain(DomainError::InvitationRequired)),
        "{err:?}"
    );
    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some("bogus"),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::InvitationNotFound), "{err:?}");
}

#[tokio::test]
async fn invited_user_registers_once_as_member() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let bo = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
    let again = register(
        &pool,
        "bo2@example.se",
        "Bo",
        Some(&token),
        passkey("c3"),
        now(),
    )
    .await
    .unwrap_err();

    assert_eq!(bo.role, Role::Member);
    assert!(
        matches!(again, Error::Domain(DomainError::InvitationAlreadyUsed)),
        "{again:?}"
    );
}

#[tokio::test]
async fn expired_invitation_is_rejected() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let later = now() + SignedDuration::from_hours(24 * 7);
    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        later,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, Error::Domain(DomainError::InvitationExpired)),
        "{err:?}"
    );
}

#[tokio::test]
async fn invitation_tokens_are_stored_only_as_hashes() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let leaks: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM events WHERE instr(payload, ?1) > 0)
              + (SELECT COUNT(*) FROM invitations WHERE instr(token_hash, ?1) > 0)",
    )
    .bind(&token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaks, 0);
}

#[tokio::test]
async fn invitations_are_admin_only_and_unique_per_email() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let bo = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();

    let by_member = create_invitation(&pool, bo.id, "cecilia@example.se", now())
        .await
        .unwrap_err();
    let registered = create_invitation(&pool, anna.id, "BO@example.se", now())
        .await
        .unwrap_err();
    create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap();
    let pending = create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap_err();
    let after_expiry = now() + SignedDuration::from_hours(24 * 7);
    create_invitation(&pool, anna.id, "cecilia@example.se", after_expiry)
        .await
        .unwrap();

    assert!(
        matches!(by_member, Error::Domain(DomainError::NotAdmin)),
        "{by_member:?}"
    );
    assert!(matches!(registered, Error::AlreadyExists), "{registered:?}");
    assert!(matches!(pending, Error::AlreadyExists), "{pending:?}");
}

#[tokio::test]
async fn duplicate_email_is_rejected_without_writing_events() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    // A pending invitation for an email that registers in the meantime is the
    // only way to reach the users UNIQUE constraint through the public API.
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email = 'bo@example.se' WHERE user_id = ?")
        .bind(anna.id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let err = register(
        &pool,
        "Bo@Example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, Error::AlreadyExists), "{err:?}");
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn a_credential_cannot_belong_to_two_users() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c1"),
        now(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, Error::AlreadyExists), "{err:?}");
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn users_can_add_passkeys_and_logins_update_them() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    add_passkey(&pool, anna.id, passkey("c2")).await.unwrap();
    let dup = add_passkey(&pool, anna.id, passkey("c2"))
        .await
        .unwrap_err();
    record_passkey_use(&pool, anna.id, "c2", json!({ "counter": 3 }))
        .await
        .unwrap();

    assert!(
        matches!(dup, Error::Domain(DomainError::DuplicatePasskey)),
        "{dup:?}"
    );
    let anna = get_user(&pool, anna.id).await.unwrap().unwrap();
    let ids: Vec<&str> = anna
        .passkeys
        .iter()
        .map(|p| p.credential_id.as_str())
        .collect();
    assert_eq!(ids, ["c1", "c2"]);
    assert_eq!(anna.passkeys[1].passkey, json!({ "counter": 3 }));
    let (stored, last_used): (String, Option<String>) =
        sqlx::query_as("SELECT passkey, last_used_at FROM passkeys WHERE credential_id = 'c2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, r#"{"counter":3}"#);
    assert!(last_used.is_some());
}

#[tokio::test]
async fn events_record_who_acted() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let actors: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT json_extract(metadata, '$.actor') FROM events")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(actors, [anna.id.to_string()]);
}

#[tokio::test]
async fn rebuilt_projections_equal_incremental_ones() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
    create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap();
    record_passkey_use(&pool, anna.id, "c1", json!({ "counter": 1 }))
        .await
        .unwrap();
    let snapshot = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT json_group_array(json_array(user_id, email, display_name, role, registered_at))
                 || (SELECT json_group_array(json_array(credential_id, user_id, name, passkey, added_at, last_used_at)) FROM (SELECT * FROM passkeys ORDER BY credential_id))
                 || (SELECT json_group_array(json_array(invitation_id, email, token_hash, created_by, expires_at, accepted_by)) FROM (SELECT * FROM invitations ORDER BY invitation_id))
             FROM (SELECT * FROM users ORDER BY user_id)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = snapshot().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(snapshot().await, before);
    assert!(before.contains("cecilia@example.se"), "{before}");
}

#[tokio::test]
async fn concurrent_bootstrap_yields_exactly_one_admin() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("doris.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();

    let tasks: Vec<_> = (0..4)
        .map(|i| {
            let pool = pool.clone();
            tokio::spawn(async move {
                register(
                    &pool,
                    &format!("u{i}@example.se"),
                    "U",
                    None,
                    passkey(&format!("c{i}")),
                    now(),
                )
                .await
            })
        })
        .collect();
    let mut admins = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(user) => {
                assert_eq!(user.role, Role::Admin);
                admins += 1;
            }
            Err(err) => assert!(
                matches!(err, Error::Domain(DomainError::InvitationRequired)),
                "{err:?}"
            ),
        }
    }
    assert_eq!(admins, 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test store`
Expected: FAIL. The build error is `unresolved imports doris_identity::Error, doris_identity::register, ...`.

- [ ] **Step 3: Write the projection migration**

`migrations/0002_identity.sql`:

```sql
-- Projections of the user-* and invitation-* streams. Rebuildable from events.

CREATE TABLE users (
    user_id       TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL,
    registered_at TEXT NOT NULL
);

CREATE TABLE passkeys (
    credential_id TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users (user_id),
    name          TEXT NOT NULL,
    passkey       TEXT NOT NULL,
    added_at      TEXT NOT NULL,
    last_used_at  TEXT
);

CREATE TABLE invitations (
    invitation_id TEXT PRIMARY KEY,
    email         TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    created_by    TEXT NOT NULL,
    expires_at    INTEGER NOT NULL, -- unix seconds, for range queries
    accepted_by   TEXT
);
```

- [ ] **Step 4: Implement tokens**

`crates/identity/src/token.rs`:

```rust
//! Random bearer tokens (invitations, sessions). Only hashes are stored.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// 256 random bits, base64url without padding.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Lowercase hex SHA-256 of the token.
pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
```

- [ ] **Step 5: Implement projections**

`crates/identity/src/projections.rs`:

```rust
//! Read models for users, passkeys and invitations. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::{InvitationEvent, UserEvent};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(user_id) = event.stream_id.strip_prefix(crate::USER_STREAM) {
        apply_user(conn, user_id, event).await
    } else if let Some(invitation_id) = event.stream_id.strip_prefix(crate::INVITATION_STREAM) {
        apply_invitation(conn, invitation_id, event).await
    } else {
        Ok(())
    }
}

async fn apply_user(
    conn: &mut SqliteConnection,
    user_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    let at = &event.recorded_at;
    match event.decode::<UserEvent>()? {
        UserEvent::UserRegistered {
            email,
            display_name,
            role,
            ..
        } => {
            sqlx::query(
                "INSERT INTO users (user_id, email, display_name, role, registered_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(user_id)
            .bind(email.as_str())
            .bind(display_name.as_str())
            .bind(role.as_str())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        UserEvent::PasskeyAdded {
            credential_id,
            name,
            passkey,
        } => {
            sqlx::query(
                "INSERT INTO passkeys (credential_id, user_id, name, passkey, added_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(credential_id)
            .bind(user_id)
            .bind(name)
            .bind(passkey.to_string())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        UserEvent::PasskeyUsed {
            credential_id,
            passkey,
        } => {
            sqlx::query(
                "UPDATE passkeys SET passkey = ?, last_used_at = ? WHERE credential_id = ?",
            )
            .bind(passkey.to_string())
            .bind(at)
            .bind(credential_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

async fn apply_invitation(
    conn: &mut SqliteConnection,
    invitation_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode::<InvitationEvent>()? {
        InvitationEvent::InvitationCreated {
            email,
            token_hash,
            created_by,
            expires_at,
            ..
        } => {
            sqlx::query(
                "INSERT INTO invitations (invitation_id, email, token_hash, created_by, expires_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(invitation_id)
            .bind(email.as_str())
            .bind(token_hash)
            .bind(created_by.to_string())
            .bind(expires_at.as_second())
            .execute(&mut *conn)
            .await?;
        }
        InvitationEvent::InvitationAccepted { user_id } => {
            sqlx::query("UPDATE invitations SET accepted_by = ? WHERE invitation_id = ?")
                .bind(user_id.to_string())
                .bind(invitation_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Empties all identity projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in [
        "DELETE FROM passkeys",
        "DELETE FROM invitations",
        "DELETE FROM users",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 6: Implement the store API**

Replace `crates/identity/src/lib.rs` with:

```rust
//! Users, passkeys and invitations, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
mod projections;
pub mod token;

use domain::{
    Admission, DisplayName, DomainError, Email, Invitation, InvitationEvent, Passkey, RegisterUser,
    User, UserEvent,
};
use doris_eventstore::{Metadata, NewEvent};
use jiff::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;

const USER_STREAM: &str = "user-";
const INVITATION_STREAM: &str = "invitation-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("already exists")]
    AlreadyExists,
    #[error("invitation not found")]
    InvitationNotFound,
    #[error("user not found")]
    UserNotFound,
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

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        doris_eventstore::Error::from(err).into()
    }
}

/// Registers a user with their first passkey. The very first user becomes
/// admin; everyone after that needs a valid invitation for the same email.
pub async fn register(
    pool: &SqlitePool,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    passkey: Passkey,
    now: Timestamp,
) -> Result<User> {
    let cmd = RegisterUser {
        user_id: Uuid::new_v4(),
        email: Email::parse(email)?,
        display_name: DisplayName::parse(display_name)?,
        passkey,
    };
    let mut tx = doris_eventstore::begin(pool).await?;

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    let invitation = match invitation_token {
        Some(token) => Some(load_invitation_by_token(&mut tx, token).await?),
        None => None,
    };
    let admission = match (&invitation, user_count) {
        (_, 0) => Admission::Bootstrap,
        (Some((invitation, _)), _) => Admission::Invited(invitation),
        (None, _) => Admission::Uninvited,
    };

    let user_id = cmd.user_id;
    let (user_events, accepted) = domain::register_user(admission, cmd, now)?;
    let actor = Some(user_id);
    commit(&mut tx, &user_stream(user_id), 0, &user_events, actor).await?;
    if let (Some(event), Some((invitation, version))) = (accepted, &invitation) {
        let stream = invitation_stream(invitation.id);
        commit(&mut tx, &stream, *version, &[event], actor).await?;
    }
    tx.commit().await?;
    Ok(User::from_events(&user_events).expect("registration yields a user"))
}

/// Creates an email-bound invitation. Returns its id and the plaintext token,
/// which is never stored.
pub async fn create_invitation(
    pool: &SqlitePool,
    creator_id: Uuid,
    email: &str,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    let email = Email::parse(email)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let (creator, _) = load_user(&mut tx, creator_id)
        .await?
        .ok_or(Error::UserNotFound)?;

    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1)
             OR EXISTS (SELECT 1 FROM invitations
                        WHERE email = ?1 AND accepted_by IS NULL AND expires_at > ?2)",
    )
    .bind(email.as_str())
    .bind(now.as_second())
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(Error::AlreadyExists);
    }

    let id = Uuid::new_v4();
    let token = token::new_token();
    let event = domain::create_invitation(&creator, id, email, token::hash_token(&token), now)?;
    commit(
        &mut tx,
        &invitation_stream(id),
        0,
        &[event],
        Some(creator_id),
    )
    .await?;
    tx.commit().await?;
    Ok((id, token))
}

pub async fn add_passkey(pool: &SqlitePool, user_id: Uuid, passkey: Passkey) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (user, version) = load_user(&mut tx, user_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let events = domain::add_passkey(&user, passkey)?;
    commit(
        &mut tx,
        &user_stream(user_id),
        version,
        &events,
        Some(user_id),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Records a successful login with the credential's updated state.
pub async fn record_passkey_use(
    pool: &SqlitePool,
    user_id: Uuid,
    credential_id: &str,
    passkey: serde_json::Value,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (user, version) = load_user(&mut tx, user_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let events = domain::record_passkey_use(&user, credential_id, passkey)?;
    commit(
        &mut tx,
        &user_stream(user_id),
        version,
        &events,
        Some(user_id),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_user(pool: &SqlitePool, user_id: Uuid) -> Result<Option<User>> {
    let mut conn = pool.acquire().await?;
    Ok(load_user(&mut conn, user_id).await?.map(|(user, _)| user))
}

/// Looks a user up by email (case-insensitive, via normalization).
pub async fn find_user_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>> {
    let Ok(email) = Email::parse(email) else {
        return Ok(None);
    };
    let mut conn = pool.acquire().await?;
    let user_id: Option<String> = sqlx::query_scalar("SELECT user_id FROM users WHERE email = ?")
        .bind(email.as_str())
        .fetch_optional(&mut *conn)
        .await?;
    match user_id {
        Some(id) => Ok(load_user(&mut conn, id.parse().expect("user_id is a uuid"))
            .await?
            .map(|(user, _)| user)),
        None => Ok(None),
    }
}

fn user_stream(id: Uuid) -> String {
    format!("{USER_STREAM}{id}")
}

fn invitation_stream(id: Uuid) -> String {
    format!("{INVITATION_STREAM}{id}")
}

async fn load_stream<T: DeserializeOwned>(
    conn: &mut SqliteConnection,
    stream: &str,
) -> Result<(Vec<T>, i64)> {
    let recorded = doris_eventstore::load(conn, stream).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode())
        .collect::<Result<_, _>>()?;
    Ok((events, version))
}

async fn load_user(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<(User, i64)>> {
    let (events, version) = load_stream::<UserEvent>(conn, &user_stream(id)).await?;
    Ok(User::from_events(&events).map(|user| (user, version)))
}

async fn load_invitation_by_token(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<(Invitation, i64)> {
    let id: String =
        sqlx::query_scalar("SELECT invitation_id FROM invitations WHERE token_hash = ?")
            .bind(token::hash_token(token))
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(Error::InvitationNotFound)?;
    let id: Uuid = id.parse().expect("invitation_id is a uuid");
    let (events, version) = load_stream::<InvitationEvent>(conn, &invitation_stream(id)).await?;
    let invitation = Invitation::from_events(&events).ok_or(Error::InvitationNotFound)?;
    Ok((invitation, version))
}

/// Appends events and updates projections within the caller's transaction.
async fn commit<T: Serialize>(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[T],
    actor: Option<Uuid>,
) -> Result<()> {
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: actor.map(|id| id.to_string()),
    };
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
```

- [ ] **Step 7: Run all tests to verify they pass**

Run: `make test`
Expected: PASS. That is 5 eventstore tests, 11 domain tests and 12 store tests.

- [ ] **Step 8: Verify immutability by hand**

```bash
sqlite3 /tmp/doris-check.db < migrations/0001_events.sql \
  && sqlite3 /tmp/doris-check.db "INSERT INTO events (stream_id, stream_version, event_type, schema_version, payload, metadata) VALUES ('s',1,'X',1,'{}','{}'); UPDATE events SET payload='{}';"; rm -f /tmp/doris-check.db
```

Expected: `Runtime error: events are append-only`.

- [ ] **Step 9: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add migrations crates/identity
git commit -m "Add identity store: projections, registration, invitations"
```
