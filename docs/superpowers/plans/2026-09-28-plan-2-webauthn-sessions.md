# Plan 2: WebAuthn Ceremonies and Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real passkey registration, login and "add another passkey" on top of Plan 1's identity store, plus 30-day login sessions. All of it is tested end to end with a software authenticator and no browser.

**Architecture:** `doris-identity` gains three pieces:
- `session.rs` (token sessions, hashed, 30 days).
- `webauthn.rs` with an `Auth` service wrapping `webauthn_rs::Webauthn`. Ceremony state is stored server-side in SQLite, is single use and lasts 5 minutes.
- A dry-run `check_registration`, so a ceremony refuses bad input *before* the authenticator creates a credential.

An unknown email at login gets a fake challenge from webauthn-rs's `WebauthnFakeCredentialGenerator`. It is keyed by a secret persisted in the DB, so it never reveals whether the email exists. Sessions, ceremonies and secrets are operational tables, not events.

**Tech Stack:**
- webauthn-rs 0.5 with feature `danger-allow-state-serialisation`
- webauthn-rs-proto 0.5
- webauthn-authenticator-rs 0.5 with feature `softpasskey` (dev only)
- url 2 (dev)
- Plus everything from Plan 1

webauthn-rs links the **system OpenSSL**.

**Spec:** `docs/superpowers/specs/2026-09-27-inloggning-webauthn-design.md` (see its section "Beslut i Plan 2")

**Plan series:** Plan 2 of 4. Plan 1 (event store and identity core) is done. Plan 3 adds gRPC-Web and the server; Plan 4 adds the Leptos frontend and e2e tests.

## Global Constraints
- TDD: every task writes failing tests first, runs them to see the failure, then implements. No production code without a red test first.
- WebAuthn/passkeys only; never passwords.
- Session tokens and invitation tokens are stored only as SHA-256 hex hashes (`token::hash_token`), and plaintext is returned once.
- Sessions last **30 days** (absolute, `SESSION_TTL = 24 * 30 h`). Ceremonies last **5 minutes** (`CEREMONY_TTL`), are single use and are deleted when taken.
- `BeginLogin` must not reveal whether an email is registered. An unknown email gets a fake challenge with the same JSON keys, `timeout`, `rpId` and `userVerification` as a real one. Its credential ids are stable per normalized email and survive restarts. Every login failure is `Error::LoginFailed`.
- Sessions, ceremonies and server secrets are operational state: plain tables, not events. No foreign key to projection tables, because those are emptied on rebuild.
- Email is personal data: never log it.
- OpenSSL comes from the system (Homebrew `openssl@3` on macOS, `libssl-dev` on Debian/Ubuntu). It is not vendored.
- Code and identifiers are in English.
- Decisions that refine the spec:
  - The passkey name is given at **begin** (`begin_registration`, `begin_add_passkey`), so it is validated before the authenticator is involved.
  - `finish_registration` receives the invitation token **again** from the client, so no plaintext token is ever stored server-side.
  - webauthn-rs registers passkeys with `residentKey: "discouraged"`, so conditional UI (`autocomplete="username webauthn"`) is out. The frontend uses `autocomplete="username"`.
  - At bootstrap (no users yet) a supplied invitation token is ignored.

---

### Task 1: Dry-run registration check

**Files:**
- Modify: `crates/identity/src/lib.rs` (replace the `register` function with the block below)
- Test: `crates/identity/tests/store.rs` (append two tests, add `check_registration` to the imports)

**Interfaces:**
- Consumes: Plan 1's `register`, `load_invitation_by_token`, `commit`, `domain::register_user`.
- Produces (Task 3 uses it):
  - `doris_identity::check_registration(pool, email: &str, display_name: &str, invitation_token: Option<&str>, now) -> Result<()>`. It runs every registration rule inside a transaction and then rolls back.
  - `register`'s signature is unchanged. At bootstrap it now ignores `invitation_token`.

- [ ] **Step 1: Write the failing tests**

In `crates/identity/tests/store.rs`, change the second `use` to:

```rust
use doris_identity::{
    Error, add_passkey, check_registration, create_invitation, find_user_by_email, get_user,
    rebuild_projections, record_passkey_use, register,
};
```

Append:

```rust
#[tokio::test]
async fn first_user_with_a_stray_invitation_token_still_becomes_admin() {
    let pool = db().await;

    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        Some("stray"),
        passkey("c1"),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(anna.role, Role::Admin);
}

#[tokio::test]
async fn check_registration_applies_every_rule_but_saves_nothing() {
    let pool = db().await;

    check_registration(&pool, "anna@example.se", "Anna", None, now())
        .await
        .unwrap();
    assert_eq!(event_count(&pool).await, 0);

    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1"),
        now(),
    )
    .await
    .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let uninvited = check_registration(&pool, "bo@example.se", "Bo", None, now())
        .await
        .unwrap_err();
    let mismatch = check_registration(&pool, "cecilia@example.se", "C", Some(&token), now())
        .await
        .unwrap_err();
    let bad_name = check_registration(&pool, "bo@example.se", " ", Some(&token), now())
        .await
        .unwrap_err();
    check_registration(&pool, "bo@example.se", "Bo", Some(&token), now())
        .await
        .unwrap();

    assert!(
        matches!(uninvited, Error::Domain(DomainError::InvitationRequired)),
        "{uninvited:?}"
    );
    assert!(
        matches!(
            mismatch,
            Error::Domain(DomainError::InvitationEmailMismatch)
        ),
        "{mismatch:?}"
    );
    assert!(
        matches!(bad_name, Error::Domain(DomainError::InvalidDisplayName)),
        "{bad_name:?}"
    );
    assert_eq!(event_count(&pool).await, before);
    register(
        &pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test store`
Expected: FAIL. The build fails with `unresolved import doris_identity::check_registration`. (Once that is added, the stray-token test fails with `InvitationNotFound` until the bootstrap change is in.)

- [ ] **Step 3: Implement**

In `crates/identity/src/lib.rs`, replace the whole `register` function (doc comment included) with:

```rust
/// Registers a user with their first passkey. The very first user becomes
/// admin (any invitation token is then ignored); everyone after that needs a
/// valid invitation for the same email.
///
/// The caller chooses `user_id`: the WebAuthn ceremony picks it at
/// `begin_registration`, where it becomes the WebAuthn user handle.
pub async fn register(
    pool: &SqlitePool,
    user_id: Uuid,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    passkey: Passkey,
    now: Timestamp,
) -> Result<User> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let user = register_in(
        &mut tx,
        user_id,
        email,
        display_name,
        invitation_token,
        passkey,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(user)
}

/// Runs every registration rule without saving anything, so a WebAuthn
/// ceremony can refuse before the authenticator creates a credential.
pub async fn check_registration(
    pool: &SqlitePool,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    now: Timestamp,
) -> Result<()> {
    let placeholder = Passkey::new(String::new(), "placeholder", serde_json::Value::Null)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    register_in(
        &mut tx,
        Uuid::new_v4(),
        email,
        display_name,
        invitation_token,
        placeholder,
        now,
    )
    .await?;
    tx.rollback().await?;
    Ok(())
}

async fn register_in(
    conn: &mut SqliteConnection,
    user_id: Uuid,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    passkey: Passkey,
    now: Timestamp,
) -> Result<User> {
    let cmd = RegisterUser {
        user_id,
        email: Email::parse(email)?,
        display_name: DisplayName::parse(display_name)?,
        passkey,
    };
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *conn)
        .await?;
    let invitation = match invitation_token {
        Some(token) if user_count > 0 => Some(load_invitation_by_token(conn, token).await?),
        _ => None,
    };
    let admission = match (&invitation, user_count) {
        (_, 0) => Admission::Bootstrap,
        (Some((invitation, _)), _) => Admission::Invited(invitation),
        (None, _) => Admission::Uninvited,
    };

    let (user_events, accepted) = domain::register_user(admission, cmd, now)?;
    let actor = Some(user_id);
    commit(conn, &user_stream(user_id), 0, &user_events, actor).await?;
    if let (Some(event), Some((invitation, version))) = (accepted, &invitation) {
        let stream = invitation_stream(invitation.id);
        commit(conn, &stream, *version, &[event], actor).await?;
    }
    Ok(User::from_events(&user_events).expect("registration yields a user"))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-identity`
Expected: PASS: 11 domain tests and 15 store tests.

- [ ] **Step 5: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/identity
git commit -m "Add dry-run registration check; ignore invitation token at bootstrap"
```

---

### Task 2: Sessions

**Files:**
- Create: `migrations/0003_sessions.sql`, `crates/identity/src/session.rs`
- Modify: `crates/identity/src/lib.rs` (module and re-export lines)
- Test: `crates/identity/tests/session.rs`

**Interfaces:**
- Consumes: `token::new_token`, `token::hash_token`, `get_user`, `doris_eventstore::begin`.
- Produces (Task 3 and Plan 3 use these):
  - `SESSION_TTL` (30 days)
  - `create_session(pool, user_id, now) -> Result<String>` returns the plaintext token and purges expired sessions.
  - `session_user(pool, token, now) -> Result<Option<User>>`
  - `end_session(pool, token) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

`crates/identity/tests/session.rs`:

```rust
use doris_identity::domain::Passkey;
use doris_identity::{SESSION_TTL, create_session, end_session, register, session_user};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

async fn db_with_user() -> (SqlitePool, Uuid) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let passkey = Passkey::new("c1".into(), "Laptop", json!({})).unwrap();
    let user = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey,
        now(),
    )
    .await
    .unwrap();
    (pool, user.id)
}

#[tokio::test]
async fn a_session_token_identifies_its_user() {
    let (pool, user_id) = db_with_user().await;

    let token = create_session(&pool, user_id, now()).await.unwrap();

    let user = session_user(&pool, &token, now()).await.unwrap().unwrap();
    assert_eq!(user.id, user_id);
    assert_eq!(
        session_user(&pool, "not-a-token", now()).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn session_tokens_are_stored_only_as_hashes() {
    let (pool, user_id) = db_with_user().await;

    let token = create_session(&pool, user_id, now()).await.unwrap();

    let leaks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE instr(token_hash, ?) > 0")
            .bind(&token)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(leaks, 0);
}

#[tokio::test]
async fn sessions_last_thirty_days() {
    let (pool, user_id) = db_with_user().await;
    let token = create_session(&pool, user_id, now()).await.unwrap();

    let almost = now() + SESSION_TTL - SignedDuration::from_secs(1);
    let expired = now() + SESSION_TTL;

    assert_eq!(SESSION_TTL, SignedDuration::from_hours(24 * 30));
    assert!(session_user(&pool, &token, almost).await.unwrap().is_some());
    assert_eq!(session_user(&pool, &token, expired).await.unwrap(), None);
}

#[tokio::test]
async fn ending_a_session_invalidates_only_that_token() {
    let (pool, user_id) = db_with_user().await;
    let laptop = create_session(&pool, user_id, now()).await.unwrap();
    let phone = create_session(&pool, user_id, now()).await.unwrap();

    end_session(&pool, &laptop).await.unwrap();

    assert_eq!(session_user(&pool, &laptop, now()).await.unwrap(), None);
    assert!(session_user(&pool, &phone, now()).await.unwrap().is_some());
}

#[tokio::test]
async fn creating_a_session_purges_expired_ones() {
    let (pool, user_id) = db_with_user().await;
    create_session(&pool, user_id, now()).await.unwrap();

    create_session(&pool, user_id, now() + SESSION_TTL)
        .await
        .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test session`
Expected: FAIL with `unresolved imports doris_identity::SESSION_TTL, doris_identity::create_session, ...`.

- [ ] **Step 3: Write the migration**

`migrations/0003_sessions.sql`:

```sql
-- Login sessions. Operational state, not events: safe to purge.
-- No foreign key to users: that projection is emptied on rebuild.

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    expires_at INTEGER NOT NULL -- unix seconds
);

CREATE INDEX sessions_expires_at ON sessions (expires_at);
```

- [ ] **Step 4: Implement sessions**

`crates/identity/src/session.rs`:

```rust
//! Login sessions: opaque bearer tokens, stored only as hashes. Sessions are
//! operational state, not events.

use crate::domain::User;
use crate::{Result, get_user, token};
use jiff::{SignedDuration, Timestamp};
use sqlx::SqlitePool;
use uuid::Uuid;

pub const SESSION_TTL: SignedDuration = SignedDuration::from_hours(24 * 30);

/// Starts a session and returns its plaintext token, which is never stored.
/// Also purges expired sessions.
pub async fn create_session(pool: &SqlitePool, user_id: Uuid, now: Timestamp) -> Result<String> {
    let token = token::new_token();
    let mut tx = doris_eventstore::begin(pool).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now.as_second())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(token::hash_token(&token))
        .bind(user_id.to_string())
        .bind((now + SESSION_TTL).as_second())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(token)
}

/// The user behind a live session; `None` for unknown or expired tokens.
pub async fn session_user(pool: &SqlitePool, token: &str, now: Timestamp) -> Result<Option<User>> {
    let user_id: Option<String> =
        sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > ?")
            .bind(token::hash_token(token))
            .bind(now.as_second())
            .fetch_optional(pool)
            .await?;
    match user_id {
        Some(id) => get_user(pool, id.parse().expect("user_id is a uuid")).await,
        None => Ok(None),
    }
}

pub async fn end_session(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token::hash_token(token))
        .execute(pool)
        .await?;
    Ok(())
}
```

In `crates/identity/src/lib.rs`:
- Change `mod projections;\npub mod token;` to `mod projections;\nmod session;\npub mod token;`.
- Below `pub use projections::rebuild_projections;`, add:

```rust
pub use session::{SESSION_TTL, create_session, end_session, session_user};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p doris-identity`
Expected: PASS, with 5 session tests plus the existing ones.

- [ ] **Step 6: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add migrations/0003_sessions.sql crates/identity
git commit -m "Add hashed 30-day login sessions"
```

---

### Task 3: WebAuthn ceremonies

**Files:**
- Modify: `Cargo.toml` (workspace deps), `crates/identity/Cargo.toml`
- Create: `migrations/0004_webauthn.sql`, `crates/identity/src/webauthn.rs`
- Modify: `crates/identity/src/domain.rs` (`Passkey::validate_name`), `crates/identity/src/lib.rs` (error variants, `From<serde_json::Error>`, module and re-export)
- Modify: `AGENTS.md` (OpenSSL requirement, WebAuthn notes)
- Test: `crates/identity/tests/webauthn.rs`

**Interfaces:**
- Consumes:
  - From Task 1: `check_registration`.
  - From Task 2: `create_session`, `session_user`.
  - From Plan 1: `register`, `add_passkey`, `record_passkey_use`, `get_user`, `find_user_by_email`, `create_invitation`, `domain::{Email, DisplayName, Passkey, User}`.
- Produces (Plan 3's gRPC layer uses these; the options and credentials are the `webauthn_rs::prelude` types, serialized to and from JSON at the gRPC boundary):
  - `doris_identity::Auth::new(pool: SqlitePool, rp_id: &str, rp_origin: &Url) -> Result<Auth>`
  - `Auth::begin_registration(&self, email, display_name, invitation_token: Option<&str>, passkey_name, now) -> Result<(Uuid, CreationChallengeResponse)>`
  - `Auth::finish_registration(&self, ceremony_id, invitation_token: Option<&str>, &RegisterPublicKeyCredential, now) -> Result<(User, String /*session token*/)>`
  - `Auth::begin_add_passkey(&self, user_id, passkey_name, now) -> Result<(Uuid, CreationChallengeResponse)>`
  - `Auth::finish_add_passkey(&self, user_id, ceremony_id, &RegisterPublicKeyCredential, now) -> Result<()>`
  - `Auth::begin_login(&self, email, now) -> Result<(Uuid, RequestChallengeResponse)>`
  - `Auth::finish_login(&self, ceremony_id, &PublicKeyCredential, now) -> Result<(User, String)>`
  - `CEREMONY_TTL` (5 min)
  - `domain::Passkey::validate_name(&str) -> Result<String, DomainError>`
  - New `Error` variants: `CeremonyNotFound`, `CeremonyExpired`, `LoginFailed`, `Webauthn(WebauthnError)`
  - A stored `domain::Passkey.credential_id` is the base64url (no padding) credential id, and `passkey` is the serialized `webauthn_rs::prelude::Passkey`.

- [ ] **Step 1: Add dependencies**

In the root `Cargo.toml` `[workspace.dependencies]`, after `uuid`, add:

```toml
url = "2"
webauthn-authenticator-rs = { version = "0.5", features = ["softpasskey"] }
webauthn-rs = { version = "0.5", features = ["danger-allow-state-serialisation"] }
webauthn-rs-proto = "0.5"
```

`crates/identity/Cargo.toml` becomes:

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
webauthn-rs.workspace = true
webauthn-rs-proto.workspace = true

[dev-dependencies]
url.workspace = true
webauthn-authenticator-rs.workspace = true
tempfile.workspace = true
tokio.workspace = true
```

Run: `cargo build -p doris-identity`
Expected: it builds. If `openssl-sys` fails to find OpenSSL on macOS, install it with `brew install openssl@3`. Don't vendor it; the project uses the system OpenSSL.

- [ ] **Step 2: Write the failing tests**

`crates/identity/tests/webauthn.rs`:

```rust
use doris_identity::domain::{DomainError, Role, User};
use doris_identity::{Auth, CEREMONY_TTL, Error, create_invitation, get_user, session_user};
use jiff::Timestamp;
use sqlx::SqlitePool;
use url::Url;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

type Authenticator = WebauthnAuthenticator<SoftPasskey>;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn origin() -> Url {
    Url::parse("http://localhost:3000").unwrap()
}

fn authenticator() -> Authenticator {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

async fn setup() -> (SqlitePool, Auth) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let auth = Auth::new(pool.clone(), "localhost", &origin()).unwrap();
    (pool, auth)
}

async fn sign_up(
    auth: &Auth,
    device: &mut Authenticator,
    email: &str,
    token: Option<&str>,
) -> (User, String) {
    let (ceremony, options) = auth
        .begin_registration(email, "Anna", token, "Laptop", now())
        .await
        .unwrap();
    let credential = device.do_registration(origin(), options).unwrap();
    auth.finish_registration(ceremony, token, &credential, now())
        .await
        .unwrap()
}

async fn log_in(
    auth: &Auth,
    device: &mut Authenticator,
    email: &str,
) -> Result<(User, String), Error> {
    let (ceremony, options) = auth.begin_login(email, now()).await.unwrap();
    let credential = device.do_authentication(origin(), options).unwrap();
    auth.finish_login(ceremony, &credential, now()).await
}

#[tokio::test]
async fn registering_with_a_passkey_creates_the_admin_and_a_session() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();

    let (anna, session) = sign_up(&auth, &mut laptop, "Anna@Example.se", None).await;

    assert_eq!(anna.role, Role::Admin);
    assert_eq!(anna.email.as_str(), "anna@example.se");
    assert_eq!(anna.passkeys.len(), 1);
    assert_eq!(anna.passkeys[0].name, "Laptop");
    assert_eq!(
        session_user(&pool, &session, now()).await.unwrap(),
        Some(anna)
    );
}

#[tokio::test]
async fn logging_in_with_the_passkey_starts_a_new_session() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let (anna, first) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (user, second) = log_in(&auth, &mut laptop, "ANNA@example.se").await.unwrap();

    assert_eq!(user.id, anna.id);
    assert_ne!(first, second);
    assert_eq!(
        session_user(&pool, &second, now())
            .await
            .unwrap()
            .unwrap()
            .id,
        anna.id
    );
}

#[tokio::test]
async fn logging_in_updates_the_stored_credential() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let (anna, _) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    log_in(&auth, &mut laptop, "anna@example.se").await.unwrap();

    let after = get_user(&pool, anna.id).await.unwrap().unwrap();
    assert_ne!(after.passkeys[0].passkey, anna.passkeys[0].passkey);
    let last_used: Option<String> = sqlx::query_scalar("SELECT last_used_at FROM passkeys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(last_used.is_some());
}

#[tokio::test]
async fn registration_is_refused_before_the_authenticator_is_asked() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let refuse = |email: &'static str, token: Option<String>, passkey_name: &'static str| {
        let auth = &auth;
        async move {
            auth.begin_registration(email, "Bo", token.as_deref(), passkey_name, now())
                .await
                .unwrap_err()
        }
    };

    let uninvited = refuse("bo@example.se", None, "Laptop").await;
    let other_email = refuse("cecilia@example.se", Some(token.clone()), "Laptop").await;
    let bad_passkey_name = refuse("bo@example.se", Some(token.clone()), " ").await;
    let bad_email = refuse("bo", Some(token), "Laptop").await;

    assert!(
        matches!(uninvited, Error::Domain(DomainError::InvitationRequired)),
        "{uninvited:?}"
    );
    assert!(
        matches!(
            other_email,
            Error::Domain(DomainError::InvitationEmailMismatch)
        ),
        "{other_email:?}"
    );
    assert!(
        matches!(
            bad_passkey_name,
            Error::Domain(DomainError::InvalidPasskeyName)
        ),
        "{bad_passkey_name:?}"
    );
    assert!(
        matches!(bad_email, Error::Domain(DomainError::InvalidEmail)),
        "{bad_email:?}"
    );
    let ceremonies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ceremonies, 0);
}

#[tokio::test]
async fn an_invited_user_registers_with_their_own_passkey() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let mut phone = authenticator();

    let (bo, _) = sign_up(&auth, &mut phone, "bo@example.se", Some(&token)).await;
    let (logged_in, _) = log_in(&auth, &mut phone, "bo@example.se").await.unwrap();

    assert_eq!(bo.role, Role::Member);
    assert_eq!(logged_in.id, bo.id);
}

#[tokio::test]
async fn a_ceremony_can_be_finished_only_once() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    let (ceremony, options) = auth
        .begin_registration("anna@example.se", "Anna", None, "Laptop", now())
        .await
        .unwrap();
    let credential = laptop.do_registration(origin(), options).unwrap();
    auth.finish_registration(ceremony, None, &credential, now())
        .await
        .unwrap();

    let again = auth
        .finish_registration(ceremony, None, &credential, now())
        .await
        .unwrap_err();

    assert!(matches!(again, Error::CeremonyNotFound), "{again:?}");
}

#[tokio::test]
async fn a_ceremony_expires_after_five_minutes() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    let (ceremony, options) = auth
        .begin_registration("anna@example.se", "Anna", None, "Laptop", now())
        .await
        .unwrap();
    let credential = laptop.do_registration(origin(), options).unwrap();

    let late = auth
        .finish_registration(ceremony, None, &credential, now() + CEREMONY_TTL)
        .await
        .unwrap_err();

    assert_eq!(CEREMONY_TTL, jiff::SignedDuration::from_mins(5));
    assert!(matches!(late, Error::CeremonyExpired), "{late:?}");
}

#[tokio::test]
async fn another_users_passkey_cannot_finish_a_login() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let mut bos = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&token)).await;

    let (annas_login, _) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let (_, bos_options) = auth.begin_login("bo@example.se", now()).await.unwrap();
    let bos_assertion = bos.do_authentication(origin(), bos_options).unwrap();
    let err = auth
        .finish_login(annas_login, &bos_assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}

#[tokio::test]
async fn an_unknown_email_gets_a_convincing_fake_challenge() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (_, real) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let (fake_ceremony, fake) = auth.begin_login("nobody@example.se", now()).await.unwrap();
    let (_, fake_again) = auth.begin_login(" NOBODY@example.se", now()).await.unwrap();
    let restarted = Auth::new(pool.clone(), "localhost", &origin()).unwrap();
    let (_, after_restart) = restarted
        .begin_login("nobody@example.se", now())
        .await
        .unwrap();

    let real = serde_json::to_value(&real).unwrap();
    let [fake, fake_again, after_restart] =
        [fake, fake_again, after_restart].map(|o| serde_json::to_value(o).unwrap());
    let keys = |v: &serde_json::Value| {
        let mut keys: Vec<String> = v["publicKey"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&fake), keys(&real));
    for field in ["timeout", "rpId", "userVerification"] {
        assert_eq!(
            fake["publicKey"][field], real["publicKey"][field],
            "{field}"
        );
    }
    assert_ne!(
        fake["publicKey"]["challenge"],
        fake_again["publicKey"]["challenge"]
    );
    assert_eq!(
        fake["publicKey"]["allowCredentials"],
        fake_again["publicKey"]["allowCredentials"]
    );
    assert_eq!(
        fake["publicKey"]["allowCredentials"],
        after_restart["publicKey"]["allowCredentials"]
    );

    let (_, real_options) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let assertion = laptop.do_authentication(origin(), real_options).unwrap();
    let err = auth
        .finish_login(fake_ceremony, &assertion, now())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}

#[tokio::test]
async fn a_user_can_add_a_second_passkey_and_log_in_with_either() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let mut phone = authenticator();
    let (anna, _) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (ceremony, options) = auth
        .begin_add_passkey(anna.id, "Telefon", now())
        .await
        .unwrap();
    let credential = phone.do_registration(origin(), options).unwrap();
    auth.finish_add_passkey(anna.id, ceremony, &credential, now())
        .await
        .unwrap();

    let names: Vec<String> = get_user(&pool, anna.id)
        .await
        .unwrap()
        .unwrap()
        .passkeys
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert_eq!(
        log_in(&auth, &mut phone, "anna@example.se")
            .await
            .unwrap()
            .0
            .id,
        anna.id
    );
    assert_eq!(
        log_in(&auth, &mut laptop, "anna@example.se")
            .await
            .unwrap()
            .0
            .id,
        anna.id
    );
}

#[tokio::test]
async fn an_add_passkey_ceremony_belongs_to_the_user_who_started_it() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let (bo, _) = sign_up(&auth, &mut authenticator(), "bo@example.se", Some(&token)).await;

    let (ceremony, options) = auth
        .begin_add_passkey(anna.id, "Telefon", now())
        .await
        .unwrap();
    let credential = authenticator().do_registration(origin(), options).unwrap();
    let err = auth
        .finish_add_passkey(bo.id, ceremony, &credential, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::CeremonyNotFound), "{err:?}");
    assert_eq!(
        get_user(&pool, anna.id)
            .await
            .unwrap()
            .unwrap()
            .passkeys
            .len(),
        1
    );
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test webauthn`
Expected: FAIL with `unresolved imports doris_identity::Auth, doris_identity::CEREMONY_TTL`.

- [ ] **Step 4: Write the migration**

`migrations/0004_webauthn.sql`:

```sql
-- WebAuthn ceremony state (single use, short-lived) and server secrets.
-- Operational state, not events: safe to purge.

CREATE TABLE webauthn_ceremonies (
    ceremony_id TEXT PRIMARY KEY,
    data        TEXT NOT NULL CHECK (json_valid(data)),
    expires_at  INTEGER NOT NULL -- unix seconds
);

CREATE TABLE server_secrets (
    name  TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
```

- [ ] **Step 5: Add `Passkey::validate_name`**

In `crates/identity/src/domain.rs`, replace `Passkey::new` with:

```rust
    pub fn new(credential_id: String, name: &str, passkey: Value) -> Result<Self, DomainError> {
        Ok(Self {
            credential_id,
            name: Self::validate_name(name)?,
            passkey,
        })
    }

    /// Trimmed passkey name, 1-64 characters.
    pub fn validate_name(name: &str) -> Result<String, DomainError> {
        bounded_text(name, 64).ok_or(DomainError::InvalidPasskeyName)
    }
```

- [ ] **Step 6: Extend the crate root**

In `crates/identity/src/lib.rs`:

1. Change the module list to:

```rust
pub mod domain;
mod projections;
mod session;
pub mod token;
mod webauthn;
```

2. Below the `pub use session::...` line, add:

```rust
pub use webauthn::{Auth, CEREMONY_TTL};
```

3. In `enum Error`, after `UserNotFound`, add:

```rust
    #[error("ceremony not found")]
    CeremonyNotFound,
    #[error("ceremony expired")]
    CeremonyExpired,
    /// Deliberately vague: never reveals whether the email exists.
    #[error("login failed")]
    LoginFailed,
    #[error(transparent)]
    Webauthn(#[from] webauthn_rs::prelude::WebauthnError),
```

4. Above `impl From<sqlx::Error> for Error`, add:

```rust
impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}
```

- [ ] **Step 7: Implement the ceremonies**

`crates/identity/src/webauthn.rs`:

```rust
//! WebAuthn ceremonies: registering with a passkey, adding passkeys and
//! logging in. Ceremony state is kept server-side, is single use and expires
//! after [`CEREMONY_TTL`].

use crate::domain::{DisplayName, Email, Passkey, User};
use crate::{
    Error, Result, add_passkey, check_registration, create_session, find_user_by_email, get_user,
    record_passkey_use, register,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;
use webauthn_rs::fake::{FakePasskeyDistribution, WebauthnFakeCredentialGenerator};
use webauthn_rs::prelude::{
    Base64UrlSafeData, CreationChallengeResponse, CredentialID, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Webauthn, WebauthnBuilder,
};
use webauthn_rs_proto::{
    AllowCredentials, PublicKeyCredentialRequestOptions, UserVerificationPolicy,
};

pub const CEREMONY_TTL: SignedDuration = SignedDuration::from_mins(5);

/// Browser timeout webauthn-rs uses for real challenges; fakes must match.
const CHALLENGE_TIMEOUT_MS: u32 = 300_000;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Ceremony {
    Registration {
        user_id: Uuid,
        email: String,
        display_name: String,
        passkey_name: String,
        state: PasskeyRegistration,
    },
    AddPasskey {
        user_id: Uuid,
        passkey_name: String,
        state: PasskeyRegistration,
    },
    /// `None` fields mark a fake login for an unknown email.
    Login {
        user_id: Option<Uuid>,
        state: Option<PasskeyAuthentication>,
    },
}

pub struct Auth {
    pool: SqlitePool,
    webauthn: Webauthn,
    rp_id: String,
}

impl Auth {
    pub fn new(pool: SqlitePool, rp_id: &str, rp_origin: &Url) -> Result<Self> {
        let webauthn = WebauthnBuilder::new(rp_id, rp_origin)?
            .rp_name("Doris")
            .build()?;
        Ok(Self {
            pool,
            webauthn,
            rp_id: rp_id.to_owned(),
        })
    }

    /// Checks every registration rule, then asks the browser to create a
    /// passkey. Nothing is saved until [`Auth::finish_registration`].
    pub async fn begin_registration(
        &self,
        email: &str,
        display_name: &str,
        invitation_token: Option<&str>,
        passkey_name: &str,
        now: Timestamp,
    ) -> Result<(Uuid, CreationChallengeResponse)> {
        let email = Email::parse(email)?;
        let display_name = DisplayName::parse(display_name)?;
        let passkey_name = Passkey::validate_name(passkey_name)?;
        check_registration(
            &self.pool,
            email.as_str(),
            display_name.as_str(),
            invitation_token,
            now,
        )
        .await?;

        let user_id = Uuid::new_v4();
        let (options, state) = self.webauthn.start_passkey_registration(
            user_id,
            email.as_str(),
            display_name.as_str(),
            None,
        )?;
        let ceremony = Ceremony::Registration {
            user_id,
            email: email.as_str().to_owned(),
            display_name: display_name.as_str().to_owned(),
            passkey_name,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the new credential, registers the user and starts a session.
    /// The invitation token is sent again rather than kept server-side.
    pub async fn finish_registration(
        &self,
        ceremony_id: Uuid,
        invitation_token: Option<&str>,
        credential: &RegisterPublicKeyCredential,
        now: Timestamp,
    ) -> Result<(User, String)> {
        let Ceremony::Registration {
            user_id,
            email,
            display_name,
            passkey_name,
            state,
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::CeremonyNotFound);
        };
        let passkey = self
            .webauthn
            .finish_passkey_registration(credential, &state)?;
        let passkey = Passkey::new(
            credential_id(passkey.cred_id()),
            &passkey_name,
            serde_json::to_value(&passkey)?,
        )?;
        let user = register(
            &self.pool,
            user_id,
            &email,
            &display_name,
            invitation_token,
            passkey,
            now,
        )
        .await?;
        let session = create_session(&self.pool, user.id, now).await?;
        Ok((user, session))
    }

    /// Asks the browser to create another passkey for a logged-in user.
    pub async fn begin_add_passkey(
        &self,
        user_id: Uuid,
        passkey_name: &str,
        now: Timestamp,
    ) -> Result<(Uuid, CreationChallengeResponse)> {
        let passkey_name = Passkey::validate_name(passkey_name)?;
        let user = get_user(&self.pool, user_id)
            .await?
            .ok_or(Error::UserNotFound)?;
        let existing = user
            .passkeys
            .iter()
            .map(|p| Ok(webauthn_passkey(p)?.cred_id().clone()))
            .collect::<Result<Vec<_>>>()?;
        let (options, state) = self.webauthn.start_passkey_registration(
            user.id,
            user.email.as_str(),
            user.display_name.as_str(),
            Some(existing),
        )?;
        let ceremony = Ceremony::AddPasskey {
            user_id,
            passkey_name,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    pub async fn finish_add_passkey(
        &self,
        user_id: Uuid,
        ceremony_id: Uuid,
        credential: &RegisterPublicKeyCredential,
        now: Timestamp,
    ) -> Result<()> {
        let Ceremony::AddPasskey {
            user_id: owner,
            passkey_name,
            state,
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::CeremonyNotFound);
        };
        if owner != user_id {
            return Err(Error::CeremonyNotFound);
        }
        let passkey = self
            .webauthn
            .finish_passkey_registration(credential, &state)?;
        let passkey = Passkey::new(
            credential_id(passkey.cred_id()),
            &passkey_name,
            serde_json::to_value(&passkey)?,
        )?;
        add_passkey(&self.pool, user_id, passkey).await
    }

    /// Starts a login. An unknown email gets a fake challenge that looks like
    /// a real one, so this never reveals whether the email is registered.
    pub async fn begin_login(
        &self,
        email: &str,
        now: Timestamp,
    ) -> Result<(Uuid, RequestChallengeResponse)> {
        let user = find_user_by_email(&self.pool, email).await?;
        let (options, ceremony) = match user {
            Some(user) if !user.passkeys.is_empty() => {
                let passkeys = user
                    .passkeys
                    .iter()
                    .map(webauthn_passkey)
                    .collect::<Result<Vec<_>>>()?;
                let (options, state) = self.webauthn.start_passkey_authentication(&passkeys)?;
                let ceremony = Ceremony::Login {
                    user_id: Some(user.id),
                    state: Some(state),
                };
                (options, ceremony)
            }
            _ => {
                let ceremony = Ceremony::Login {
                    user_id: None,
                    state: None,
                };
                (self.fake_login_options(email).await?, ceremony)
            }
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the assertion, records the use and starts a session. Every
    /// failure is [`Error::LoginFailed`].
    pub async fn finish_login(
        &self,
        ceremony_id: Uuid,
        credential: &PublicKeyCredential,
        now: Timestamp,
    ) -> Result<(User, String)> {
        let Ceremony::Login {
            user_id: Some(user_id),
            state: Some(state),
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::LoginFailed);
        };
        let result = self
            .webauthn
            .finish_passkey_authentication(credential, &state)
            .map_err(|_| Error::LoginFailed)?;
        let user = get_user(&self.pool, user_id)
            .await?
            .ok_or(Error::LoginFailed)?;
        let used_id = credential_id(result.cred_id());
        let stored = user
            .passkeys
            .iter()
            .find(|p| p.credential_id == used_id)
            .ok_or(Error::LoginFailed)?;
        let mut passkey = webauthn_passkey(stored)?;
        passkey.update_credential(&result);
        record_passkey_use(
            &self.pool,
            user_id,
            &used_id,
            serde_json::to_value(&passkey)?,
        )
        .await?;
        let session = create_session(&self.pool, user_id, now).await?;
        Ok((user, session))
    }

    async fn start(&self, ceremony: &Ceremony, now: Timestamp) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let mut tx = doris_eventstore::begin(&self.pool).await?;
        sqlx::query("DELETE FROM webauthn_ceremonies WHERE expires_at <= ?")
            .bind(now.as_second())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO webauthn_ceremonies (ceremony_id, data, expires_at) VALUES (?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(serde_json::to_string(ceremony)?)
        .bind((now + CEREMONY_TTL).as_second())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Removes and returns a ceremony: each one can be finished only once.
    async fn take(&self, id: Uuid, now: Timestamp) -> Result<Ceremony> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "DELETE FROM webauthn_ceremonies WHERE ceremony_id = ? RETURNING data, expires_at",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let (data, expires_at) = row.ok_or(Error::CeremonyNotFound)?;
        if now.as_second() >= expires_at {
            return Err(Error::CeremonyExpired);
        }
        Ok(serde_json::from_str(&data)?)
    }

    /// A challenge shaped like webauthn-rs's real ones, with credential ids
    /// that are stable per email (HMAC keyed by a persisted server secret).
    async fn fake_login_options(&self, email: &str) -> Result<RequestChallengeResponse> {
        let key = self.fake_credential_key().await?;
        let generator = WebauthnFakeCredentialGenerator::<FakePasskeyDistribution>::new(&key)?;
        let fake_ids = generator.generate(email.trim().to_lowercase().as_bytes())?;
        let mut challenge = [0u8; 32];
        getrandom::fill(&mut challenge).expect("OS random source unavailable");
        Ok(RequestChallengeResponse {
            public_key: PublicKeyCredentialRequestOptions {
                challenge: Base64UrlSafeData::from(challenge.to_vec()),
                timeout: Some(CHALLENGE_TIMEOUT_MS),
                rp_id: self.rp_id.clone(),
                allow_credentials: fake_ids
                    .into_iter()
                    .map(|id| AllowCredentials {
                        type_: "public-key".to_owned(),
                        id: Base64UrlSafeData::from(id.to_vec()),
                        transports: None,
                    })
                    .collect(),
                user_verification: UserVerificationPolicy::Required,
                hints: None,
                extensions: None,
            },
            mediation: None,
        })
    }

    async fn fake_credential_key(&self) -> Result<Vec<u8>> {
        let key = WebauthnFakeCredentialGenerator::<FakePasskeyDistribution>::new_hmac_key()?;
        sqlx::query(
            "INSERT OR IGNORE INTO server_secrets (name, value) VALUES ('fake_credential_key', ?)",
        )
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT value FROM server_secrets WHERE name = 'fake_credential_key'",
        )
        .fetch_one(&self.pool)
        .await?)
    }
}

fn credential_id(id: &CredentialID) -> String {
    URL_SAFE_NO_PAD.encode(id.as_ref())
}

fn webauthn_passkey(passkey: &Passkey) -> Result<webauthn_rs::prelude::Passkey> {
    Ok(serde_json::from_value(passkey.passkey.clone())?)
}
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `make test`
Expected: PASS, 47 tests in total:

| Suite | Tests |
|---|---|
| eventstore | 5 |
| domain | 11 |
| session | 5 |
| store | 15 |
| webauthn | 11 |

- [ ] **Step 9: Document in AGENTS.md**

In `AGENTS.md`:

1. Under `## Authentication`, add these bullets:

```markdown
- WebAuthn ceremonies (`doris_identity::Auth`) keep their state server-side
  for 5 minutes, and each one can be finished once. The passkey name is given
  at *begin*, and the invitation token is sent again at *finish*. A plaintext
  token is never stored.
- `begin_login` never reveals whether an email exists. An unknown email gets
  a fake challenge (webauthn-rs `WebauthnFakeCredentialGenerator`, keyed by
  `server_secrets.fake_credential_key`), and every login failure is
  `Error::LoginFailed`.
- Sessions last 30 days (absolute).
```

2. Change the line that starts `Requires \`protoc\` on PATH` to:

```markdown
Requires `protoc` on PATH and the system OpenSSL (webauthn-rs links it:
`brew install openssl@3` on macOS, `libssl-dev` on Debian/Ubuntu), plus
`trunk` and the `wasm32-unknown-unknown` target for the frontend.
```

- [ ] **Step 10: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add Cargo.toml Cargo.lock AGENTS.md migrations/0004_webauthn.sql crates/identity
git commit -m "Add WebAuthn ceremonies: passkey registration, login, adding passkeys"
```
