# Plan 3: gRPC-Web Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single `doris` binary that serves the `doris.auth.v1.AuthService` gRPC API over gRPC-Web, keeps the session in an HttpOnly cookie, applies CORS for configured CDN origins, and serves the embedded frontend on the same port.

**Architecture:**
- `proto/doris/auth/v1/auth.proto` is the contract.
- `crates/proto` (`doris-proto`) generates types and the client with `tonic-prost-build`. The `server` feature adds the server stubs; the transport is never generated, so the client also builds for wasm32 (Plan 4).
- `crates/server` (`doris-server`) has these parts:
  - `grpc.rs` maps each RPC onto `doris_identity` and turns identity errors into stable status codes.
  - `assets.rs` serves the rust-embed'ed Trunk output, with an SPA fallback and cache headers.
  - `lib.rs` builds one axum `Router`: tonic routes plus `GrpcWebLayer`, then the asset fallback, then CORS.
  - `main.rs` handles configuration with clap/env.
- The identity crate gains the read queries the API needs.

**Tech Stack:**
- tonic 0.14, tonic-web 0.14, tonic-prost 0.14, tonic-prost-build 0.14, prost 0.14
- axum 0.8
- tower-http 0.6 (cors)
- rust-embed 8 (mime-guess)
- clap 4 (derive, env)
- tracing
- Tests: hyper-util legacy client plus `tonic_web::GrpcWebClientLayer`, which speaks gRPC-Web over HTTP/1.1 exactly like the browser.

**Spec:** `docs/superpowers/specs/2026-09-27-inloggning-webauthn-design.md`

**Plan series:** Plan 3 of 4. Plans 1–2 are done: event store, identity, WebAuthn and sessions. Plan 4 builds the Leptos frontend, Playwright e2e tests and `make dist`.

## Global Constraints
- TDD: every step that adds behavior starts from a failing test that was run and seen to fail.
- One port serves the gRPC-Web API and the embedded frontend. `DORIS_SERVE_FRONTEND=false` turns the frontend off for CDN/nginx deployments; unknown paths are then `404`.
- Session cookie: `doris_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=2592000` (30 days). Logout sends the same cookie with an empty value and `Max-Age=0`.
- CORS is off unless `DORIS_CORS_ORIGINS` lists origins. Only listed origins get `Access-Control-Allow-Origin`, with credentials allowed.
- Error statuses carry stable, snake_case codes as the message, which the frontend translates. They never contain personal data, and email is never logged.

  | Identity error | gRPC code | message |
  |---|---|---|
  | `Domain(NotAdmin)` | PermissionDenied | `not_admin` |
  | other `Domain(e)` | InvalidArgument | `invalid_email`, `invalid_display_name`, `invalid_passkey_name`, `invitation_required`, `invitation_expired`, `invitation_already_used`, `invitation_email_mismatch`, `duplicate_passkey`, `unknown_passkey` |
  | `AlreadyExists` | AlreadyExists | `already_exists` |
  | `InvitationNotFound` | NotFound | `invitation_not_found` |
  | `UserNotFound` | NotFound | `user_not_found` |
  | `CeremonyNotFound` / `CeremonyExpired` | FailedPrecondition | `ceremony_expired` |
  | `LoginFailed` | Unauthenticated | `login_failed` |
  | `Webauthn` on a *finish* call | InvalidArgument | `credential_rejected` |
  | `Webauthn` elsewhere | Internal | `internal` |
  | `Store` | Internal | `internal` |

  API-level codes:

  | Situation | gRPC code | message |
  |---|---|---|
  | No or invalid session | Unauthenticated | `not_signed_in` |
  | Malformed ceremony id | InvalidArgument | `invalid_ceremony` |
  | Credential JSON that doesn't parse | InvalidArgument | `invalid_credential` |
  | `GetInvitation` for an unusable token | NotFound | `invitation_not_found` |
- WebAuthn options and credentials travel as JSON strings (webauthn-rs types).
- Hashed assets (`<name>-<16 hex>.<ext>`, Trunk's naming) are sent with `Cache-Control: public, max-age=31536000, immutable`; everything else, including `index.html`, with `no-cache`. Missing paths that have a file extension return `404`; other paths fall back to `index.html`.
- Code, URLs, proto and identifiers are in English.

---

### Task 1: Identity read queries

**Files:**
- Create: `crates/identity/src/queries.rs`
- Modify: `crates/identity/src/lib.rs` (module and re-export)
- Test: `crates/identity/tests/queries.rs`

**Interfaces:**
- Consumes: Plan 1–2's projections (`users`, `passkeys`, `invitations`), `token::hash_token`, `register`, `create_invitation`, `add_passkey`, `record_passkey_use`.
- Produces (Task 2 uses these):
  - `bootstrap_required(pool) -> Result<bool>`
  - `invitation_email(pool, token, now) -> Result<Option<Email>>`, which returns `Some` only for an invitation that is neither accepted nor expired
  - `list_invitations(pool) -> Result<Vec<InvitationSummary>>`, where `InvitationSummary { id: Uuid, email: String, expires_at: Timestamp, accepted: bool }` is ordered with the latest expiry first
  - `list_passkeys(pool, user_id) -> Result<Vec<PasskeySummary>>`, where `PasskeySummary { credential_id, name, added_at: String, last_used_at: Option<String> }` is ordered oldest first

- [ ] **Step 1: Write the failing tests**

`crates/identity/tests/queries.rs`:

```rust
use doris_identity::domain::Passkey;
use doris_identity::{
    bootstrap_required, create_invitation, invitation_email, list_invitations, list_passkeys,
    record_passkey_use, register,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str, name: &str) -> Passkey {
    Passkey::new(id.into(), name, json!({})).unwrap()
}

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn bootstrap_is_required_until_the_first_user_registers() {
    let pool = db().await;
    assert!(bootstrap_required(&pool).await.unwrap());

    register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();

    assert!(!bootstrap_required(&pool).await.unwrap());
}

#[tokio::test]
async fn invitation_email_is_only_revealed_for_usable_invitations() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let email = invitation_email(&pool, &token, now()).await.unwrap();
    let expired = invitation_email(&pool, &token, now() + SignedDuration::from_hours(24 * 7))
        .await
        .unwrap();
    let unknown = invitation_email(&pool, "nope", now()).await.unwrap();
    register(
        &pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2", "Telefon"),
        now(),
    )
    .await
    .unwrap();
    let used = invitation_email(&pool, &token, now()).await.unwrap();

    assert_eq!(email.unwrap().as_str(), "bo@example.se");
    assert_eq!(expired, None);
    assert_eq!(unknown, None);
    assert_eq!(used, None);
}

#[tokio::test]
async fn invitations_are_listed_with_their_status() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    let (bo_id, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let later = now() + SignedDuration::from_hours(1);
    let (cecilia_id, _) = create_invitation(&pool, anna.id, "cecilia@example.se", later)
        .await
        .unwrap();
    register(
        &pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2", "Telefon"),
        now(),
    )
    .await
    .unwrap();

    let invitations = list_invitations(&pool).await.unwrap();

    let summary: Vec<(Uuid, &str, bool)> = invitations
        .iter()
        .map(|i| (i.id, i.email.as_str(), i.accepted))
        .collect();
    assert_eq!(
        summary,
        [
            (cecilia_id, "cecilia@example.se", false),
            (bo_id, "bo@example.se", true)
        ]
    );
    assert_eq!(
        invitations[1].expires_at,
        now() + SignedDuration::from_hours(24 * 7)
    );
}

#[tokio::test]
async fn passkeys_are_listed_per_user_with_last_use() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    doris_identity::add_passkey(&pool, anna.id, passkey("c2", "Telefon"))
        .await
        .unwrap();
    record_passkey_use(&pool, anna.id, "c2", json!({ "counter": 1 }))
        .await
        .unwrap();

    let passkeys = list_passkeys(&pool, anna.id).await.unwrap();
    let nobody = list_passkeys(&pool, Uuid::new_v4()).await.unwrap();

    let names: Vec<&str> = passkeys.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert_eq!(passkeys[0].last_used_at, None);
    assert!(passkeys[1].last_used_at.is_some());
    assert!(nobody.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p doris-identity --test queries`
Expected: FAIL with `unresolved imports doris_identity::bootstrap_required, ...`.

- [ ] **Step 3: Implement**

`crates/identity/src/queries.rs`:

```rust
//! Read-only views over the identity projections, for listing in the UI.

use crate::domain::Email;
use crate::{Result, token};
use jiff::Timestamp;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct PasskeySummary {
    pub credential_id: String,
    pub name: String,
    pub added_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvitationSummary {
    pub id: Uuid,
    pub email: String,
    pub expires_at: Timestamp,
    pub accepted: bool,
}

/// True until the first user has registered.
pub async fn bootstrap_required(pool: &SqlitePool) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM users)")
            .fetch_one(pool)
            .await?,
    )
}

/// The email an invitation is for, if the token belongs to an invitation
/// that is still usable (not accepted, not expired).
pub async fn invitation_email(
    pool: &SqlitePool,
    token: &str,
    now: Timestamp,
) -> Result<Option<Email>> {
    let email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM invitations
         WHERE token_hash = ? AND accepted_by IS NULL AND expires_at > ?",
    )
    .bind(token::hash_token(token))
    .bind(now.as_second())
    .fetch_optional(pool)
    .await?;
    Ok(email.map(|e| Email::parse(&e).expect("stored emails are normalized")))
}

/// All invitations, the ones expiring last first.
pub async fn list_invitations(pool: &SqlitePool) -> Result<Vec<InvitationSummary>> {
    let rows: Vec<(String, String, i64, bool)> = sqlx::query_as(
        "SELECT invitation_id, email, expires_at, accepted_by IS NOT NULL
         FROM invitations ORDER BY expires_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, expires_at, accepted)| InvitationSummary {
            id: id.parse().expect("invitation_id is a uuid"),
            email,
            expires_at: Timestamp::from_second(expires_at).expect("stored timestamps are valid"),
            accepted,
        })
        .collect())
}

/// A user's passkeys, oldest first.
pub async fn list_passkeys(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<PasskeySummary>> {
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT credential_id, name, added_at, last_used_at
         FROM passkeys WHERE user_id = ? ORDER BY added_at, credential_id",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(credential_id, name, added_at, last_used_at)| PasskeySummary {
                credential_id,
                name,
                added_at,
                last_used_at,
            },
        )
        .collect())
}
```

In `crates/identity/src/lib.rs`:
- Change `mod projections;\nmod session;` to `mod projections;\nmod queries;\nmod session;`.
- Below `pub use projections::rebuild_projections;`, add:

```rust
pub use queries::{
    InvitationSummary, PasskeySummary, bootstrap_required, invitation_email, list_invitations,
    list_passkeys,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p doris-identity`
Expected: PASS, with 4 query tests plus the existing ones.

- [ ] **Step 5: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/identity
git commit -m "Add identity read queries for the API"
```

---

### Task 2: gRPC-Web server

This task has two red/green cycles: first the gRPC API (cycle A), then static assets, CORS and the binary (cycle B).

**Files:**
- Modify: `Cargo.toml` (workspace members and dependencies)
- Create: `proto/doris/auth/v1/auth.proto`
- Create: `crates/proto/Cargo.toml`, `crates/proto/build.rs`, `crates/proto/src/lib.rs`
- Create: `crates/server/Cargo.toml`, `crates/server/src/lib.rs`, `crates/server/src/grpc.rs`, `crates/server/src/assets.rs`, `crates/server/src/main.rs`
- Test: `crates/server/tests/common/mod.rs`, `crates/server/tests/grpc.rs`, `crates/server/tests/http.rs`, `crates/server/tests/fixtures/dist/{index.html,doris-web-0123456789abcdef.js,style.css}`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes:
  - From Task 1: `bootstrap_required`, `invitation_email`, `list_invitations`, `list_passkeys`.
  - From Plan 2: `Auth::{new (async), begin_/finish_registration, begin_/finish_login, begin_/finish_add_passkey}`, `session_user`, `end_session`, `SESSION_TTL`.
  - From Plan 1: `create_invitation`, `domain::{User, Role, DomainError, INVITATION_TTL}`, `Error`.
- Produces (Plan 4 uses these):
  - The generated `doris_proto::auth::v1::{auth_service_client::AuthServiceClient, *messages}`. Without the `server` feature it builds for wasm32.
  - `doris_server::router::<E: RustEmbed>(api: AuthApi, cors_origins: Vec<HeaderValue>, serve_frontend: bool) -> axum::Router`
  - `AuthApi::new(pool, auth)`
  - `SESSION_COOKIE`
  - `assets::WebDist`, which embeds `crates/web/dist` (it may be missing: `allow_missing`)
  - The `doris` binary with `DORIS_DATABASE`, `DORIS_LISTEN`, `DORIS_RP_ID`, `DORIS_RP_ORIGIN`, `DORIS_CORS_ORIGINS` and `DORIS_SERVE_FRONTEND`.

- [ ] **Step 1: Workspace, proto contract and proto crate**

In the root `Cargo.toml`:
- Change `members` to `["crates/eventstore", "crates/identity", "crates/proto", "crates/server"]`.
- Add these after the `doris-eventstore` line in `[workspace.dependencies]`:

```toml
doris-identity = { path = "crates/identity" }
doris-proto = { path = "crates/proto" }
```

- Append to `[workspace.dependencies]`:

```toml
axum = "0.8"
clap = { version = "4", features = ["derive", "env"] }
http = "1"
http-body-util = "0.1"
hyper-util = { version = "0.1", features = ["client-legacy", "http1", "tokio"] }
mime_guess = "2"
prost = "0.14"
rust-embed = "8"
tonic = { version = "0.14", default-features = false, features = ["codegen"] }
tonic-prost = "0.14"
tonic-prost-build = "0.14"
tonic-web = "0.14"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

`proto/doris/auth/v1/auth.proto`:

```proto
syntax = "proto3";

package doris.auth.v1;

// Registration, login and passkey management. WebAuthn options and
// credentials travel as JSON strings (webauthn-rs types). The session is an
// HttpOnly cookie set by FinishRegistration/FinishLogin and cleared by Logout.
service AuthService {
  rpc GetStatus(GetStatusRequest) returns (GetStatusResponse);

  rpc BeginRegistration(BeginRegistrationRequest) returns (BeginCeremonyResponse);
  rpc FinishRegistration(FinishRegistrationRequest) returns (User);

  rpc BeginLogin(BeginLoginRequest) returns (BeginCeremonyResponse);
  rpc FinishLogin(FinishLoginRequest) returns (User);
  rpc Logout(LogoutRequest) returns (LogoutResponse);

  // Require a session.
  rpc BeginAddPasskey(BeginAddPasskeyRequest) returns (BeginCeremonyResponse);
  rpc FinishAddPasskey(FinishAddPasskeyRequest) returns (FinishAddPasskeyResponse);
  rpc ListPasskeys(ListPasskeysRequest) returns (ListPasskeysResponse);

  // Public: lets the registration page show which email an invitation is for.
  rpc GetInvitation(GetInvitationRequest) returns (GetInvitationResponse);

  // Require an admin session.
  rpc CreateInvitation(CreateInvitationRequest) returns (CreateInvitationResponse);
  rpc ListInvitations(ListInvitationsRequest) returns (ListInvitationsResponse);
}

enum Role {
  ROLE_UNSPECIFIED = 0;
  ROLE_ADMIN = 1;
  ROLE_MEMBER = 2;
}

message User {
  string id = 1;
  string email = 2;
  string display_name = 3;
  Role role = 4;
}

message GetStatusRequest {}

message GetStatusResponse {
  bool bootstrap_required = 1;
  optional User current_user = 2;
}

message BeginCeremonyResponse {
  string ceremony_id = 1;
  string options_json = 2;
}

message BeginRegistrationRequest {
  string email = 1;
  string display_name = 2;
  optional string invitation_token = 3;
  string passkey_name = 4;
}

message FinishRegistrationRequest {
  string ceremony_id = 1;
  optional string invitation_token = 2;
  string credential_json = 3;
}

message BeginLoginRequest {
  string email = 1;
}

message FinishLoginRequest {
  string ceremony_id = 1;
  string credential_json = 2;
}

message LogoutRequest {}

message LogoutResponse {}

message BeginAddPasskeyRequest {
  string passkey_name = 1;
}

message FinishAddPasskeyRequest {
  string ceremony_id = 1;
  string credential_json = 2;
}

message FinishAddPasskeyResponse {}

message ListPasskeysRequest {}

message Passkey {
  string credential_id = 1;
  string name = 2;
  string added_at = 3;
  optional string last_used_at = 4;
}

message ListPasskeysResponse {
  repeated Passkey passkeys = 1;
}

message GetInvitationRequest {
  string token = 1;
}

message GetInvitationResponse {
  string email = 1;
}

message CreateInvitationRequest {
  string email = 1;
}

message CreateInvitationResponse {
  string token = 1;
  string expires_at = 2;
}

message ListInvitationsRequest {}

message Invitation {
  string id = 1;
  string email = 2;
  string expires_at = 3;
  bool accepted = 4;
}

message ListInvitationsResponse {
  repeated Invitation invitations = 1;
}
```

`crates/proto/Cargo.toml`:

```toml
[package]
name = "doris-proto"
version.workspace = true
edition.workspace = true

[features]
# Server stubs; the browser client (wasm32) builds without them.
server = ["tonic/server", "tonic/router"]

[dependencies]
prost.workspace = true
tonic.workspace = true
tonic-prost.workspace = true

[build-dependencies]
tonic-prost-build.workspace = true
```

`crates/proto/build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(std::env::var_os("CARGO_FEATURE_SERVER").is_some())
        .build_transport(false)
        .compile_protos(&["../../proto/doris/auth/v1/auth.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
```

`crates/proto/src/lib.rs`:

```rust
//! Generated gRPC types for Doris, shared by server and browser client.

pub mod auth {
    pub mod v1 {
        tonic::include_proto!("doris.auth.v1");
    }
}
```

- [ ] **Step 2: Create the server crate manifest (library only for now)**

`crates/server/Cargo.toml` (cycle B adds the `[[bin]]` section):

```toml
[package]
name = "doris-server"
version.workspace = true
edition.workspace = true

[dependencies]
axum.workspace = true
clap.workspace = true
doris-eventstore.workspace = true
doris-identity.workspace = true
doris-proto = { workspace = true, features = ["server"] }
http.workspace = true
jiff.workspace = true
rust-embed = { workspace = true, features = ["mime-guess"] }
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
tokio.workspace = true
tonic.workspace = true
tonic-web.workspace = true
tower-http.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
url.workspace = true
uuid.workspace = true
webauthn-rs.workspace = true

[dev-dependencies]
http-body-util.workspace = true
hyper-util.workspace = true
tower.workspace = true
webauthn-authenticator-rs.workspace = true
```

Create the test fixtures:

```bash
mkdir -p crates/server/tests/fixtures/dist
echo '<!doctype html><title>Doris</title>' > crates/server/tests/fixtures/dist/index.html
echo 'console.log("app")' > crates/server/tests/fixtures/dist/doris-web-0123456789abcdef.js
echo 'body{}' > crates/server/tests/fixtures/dist/style.css
```

- [ ] **Step 3: Cycle A. Write the failing gRPC tests**

`crates/server/tests/common/mod.rs`:

```rust
//! Test harness: a real server on an ephemeral port, called over gRPC-Web
//! (HTTP/1.1) exactly like the browser does.

#![allow(dead_code)]

use doris_identity::Auth;
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_server::{AuthApi, SESSION_COOKIE};
use http::HeaderValue;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rust_embed::RustEmbed;
use sqlx::SqlitePool;
use tonic::Request;
use tonic_web::{GrpcWebCall, GrpcWebClientLayer, GrpcWebClientService};
use url::Url;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse};

pub type Grpc =
    AuthServiceClient<GrpcWebClientService<Client<HttpConnector, GrpcWebCall<tonic::body::Body>>>>;
pub type Device = WebauthnAuthenticator<SoftPasskey>;

#[derive(RustEmbed)]
#[folder = "tests/fixtures/dist"]
pub struct TestDist;

pub struct TestServer {
    pub base: String,
    pub origin: Url,
    pub pool: SqlitePool,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with(vec![], true).await
    }

    pub async fn start_with(cors_origins: Vec<HeaderValue>, serve_frontend: bool) -> Self {
        let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = Url::parse(&format!("http://localhost:{}", addr.port())).unwrap();
        let auth = Auth::new(pool.clone(), "localhost", &origin).await.unwrap();
        let app = doris_server::router::<TestDist>(
            AuthApi::new(pool.clone(), auth),
            cors_origins,
            serve_frontend,
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base: format!("http://{addr}"),
            origin,
            pool,
        }
    }

    pub fn grpc(&self) -> Grpc {
        let client = Client::builder(TokioExecutor::new()).build_http();
        let service = tower::ServiceBuilder::new()
            .layer(GrpcWebClientLayer::new())
            .service(client);
        AuthServiceClient::with_origin(service, self.base.parse().unwrap())
    }

    /// Registers through the API and returns the session cookie's token.
    pub async fn sign_up(&self, device: &mut Device, email: &str, token: Option<&str>) -> String {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_registration(pb::BeginRegistrationRequest {
                email: email.into(),
                display_name: "Anna".into(),
                invitation_token: token.map(Into::into),
                passkey_name: "Laptop".into(),
            })
            .await
            .unwrap()
            .into_inner();
        let options: CreationChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
        let credential = device
            .do_registration(self.origin.clone(), options)
            .unwrap();
        let response = grpc
            .finish_registration(pb::FinishRegistrationRequest {
                ceremony_id: begin.ceremony_id,
                invitation_token: token.map(Into::into),
                credential_json: serde_json::to_string(&credential).unwrap(),
            })
            .await
            .unwrap();
        session_from(response.metadata()).unwrap()
    }

    pub async fn log_in(&self, device: &mut Device, email: &str) -> Result<String, tonic::Status> {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_login(pb::BeginLoginRequest {
                email: email.into(),
            })
            .await?
            .into_inner();
        let options: RequestChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
        let credential = device
            .do_authentication(self.origin.clone(), options)
            .unwrap();
        let response = grpc
            .finish_login(pb::FinishLoginRequest {
                ceremony_id: begin.ceremony_id,
                credential_json: serde_json::to_string(&credential).unwrap(),
            })
            .await?;
        Ok(session_from(response.metadata()).unwrap())
    }
}

pub fn device() -> Device {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

/// A request carrying the session cookie, as the browser would send it.
pub fn authed<T>(message: T, session: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "cookie",
        format!("theme=dark; {SESSION_COOKIE}={session}")
            .parse()
            .unwrap(),
    );
    request
}

pub fn set_cookie(metadata: &tonic::metadata::MetadataMap) -> Option<String> {
    metadata
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_owned())
}

pub fn session_from(metadata: &tonic::metadata::MetadataMap) -> Option<String> {
    let cookie = set_cookie(metadata)?;
    let value = cookie
        .split(';')
        .next()?
        .strip_prefix(&format!("{SESSION_COOKIE}="))?;
    (!value.is_empty()).then(|| value.to_owned())
}

/// A plain HTTP/1.1 request (for static files and CORS preflights).
pub async fn http(
    method: http::Method,
    url: &str,
    headers: &[(&str, &str)],
) -> http::Response<String> {
    use http_body_util::{BodyExt, Empty};
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<axum::body::Bytes>>();
    let mut request = http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = client
        .request(request.body(Empty::new()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    http::Response::from_parts(parts, String::from_utf8_lossy(&bytes).into_owned())
}
```

`crates/server/tests/grpc.rs`:

```rust
mod common;

use common::{TestServer, authed, device, session_from, set_cookie};
use doris_proto::auth::v1 as pb;
use tonic::{Code, Request};
use webauthn_rs::prelude::CreationChallengeResponse;

#[tokio::test]
async fn status_reports_bootstrap_until_the_first_user_registers() {
    let server = TestServer::start().await;

    let before = server
        .grpc()
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    let session = server.sign_up(&mut device(), "anna@example.se", None).await;
    let anonymous = server
        .grpc()
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    let signed_in = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &session))
        .await
        .unwrap()
        .into_inner();

    assert!(before.bootstrap_required);
    assert_eq!(before.current_user, None);
    assert!(!anonymous.bootstrap_required);
    assert_eq!(anonymous.current_user, None);
    let user = signed_in.current_user.unwrap();
    assert_eq!(user.email, "anna@example.se");
    assert_eq!(user.role(), pb::Role::Admin);
}

#[tokio::test]
async fn the_session_cookie_is_http_only_secure_and_strict() {
    let server = TestServer::start().await;
    let mut grpc = server.grpc();
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;

    let begin = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "anna@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let options = serde_json::from_str(&begin.options_json).unwrap();
    let credential = laptop
        .do_authentication(server.origin.clone(), options)
        .unwrap();
    let response = grpc
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: begin.ceremony_id,
            credential_json: serde_json::to_string(&credential).unwrap(),
        })
        .await
        .unwrap();

    let cookie = set_cookie(response.metadata()).unwrap();
    let attributes: Vec<&str> = cookie.split("; ").skip(1).collect();
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "Secure",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=2592000"
        ]
    );
    assert_eq!(response.into_inner().email, "anna@example.se");
}

#[tokio::test]
async fn logging_out_clears_the_cookie_and_ends_the_session() {
    let server = TestServer::start().await;
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;
    let session = server.log_in(&mut laptop, "anna@example.se").await.unwrap();

    let response = server
        .grpc()
        .logout(authed(pb::LogoutRequest {}, &session))
        .await
        .unwrap();
    let status = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &session))
        .await
        .unwrap()
        .into_inner();

    let cookie = set_cookie(response.metadata()).unwrap();
    assert!(cookie.starts_with("doris_session=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");
    assert_eq!(session_from(response.metadata()), None);
    assert_eq!(status.current_user, None);
}

#[tokio::test]
async fn failed_logins_look_the_same_for_known_and_unknown_emails() {
    let server = TestServer::start().await;
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;
    let mut grpc = server.grpc();

    let real = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "anna@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let fake = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "nobody@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let options = serde_json::from_str(&real.options_json).unwrap();
    let assertion = serde_json::to_string(
        &laptop
            .do_authentication(server.origin.clone(), options)
            .unwrap(),
    )
    .unwrap();
    let finish = |ceremony_id: &str| pb::FinishLoginRequest {
        ceremony_id: ceremony_id.to_owned(),
        credential_json: assertion.clone(),
    };

    let unknown_email = grpc
        .finish_login(finish(&fake.ceremony_id))
        .await
        .unwrap_err();
    grpc.finish_login(finish(&real.ceremony_id)).await.unwrap();
    let replayed = grpc
        .finish_login(finish(&real.ceremony_id))
        .await
        .unwrap_err();

    for err in [unknown_email, replayed] {
        assert_eq!(
            (err.code(), err.message()),
            (Code::Unauthenticated, "login_failed")
        );
    }
}

#[tokio::test]
async fn protected_calls_need_a_session_and_admin_calls_an_admin() {
    let server = TestServer::start().await;
    let admin = server.sign_up(&mut device(), "anna@example.se", None).await;
    let invite = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "bo@example.se".into(),
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner();
    let member = server
        .sign_up(&mut device(), "bo@example.se", Some(&invite.token))
        .await;

    let anonymous = server
        .grpc()
        .list_passkeys(pb::ListPasskeysRequest {})
        .await
        .unwrap_err();
    let bogus = server
        .grpc()
        .list_passkeys(authed(pb::ListPasskeysRequest {}, "bogus"))
        .await
        .unwrap_err();
    let by_member = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "c@example.se".into(),
            },
            &member,
        ))
        .await
        .unwrap_err();
    let list_by_member = server
        .grpc()
        .list_invitations(authed(pb::ListInvitationsRequest {}, &member))
        .await
        .unwrap_err();

    assert_eq!(
        (anonymous.code(), anonymous.message()),
        (Code::Unauthenticated, "not_signed_in")
    );
    assert_eq!(
        (bogus.code(), bogus.message()),
        (Code::Unauthenticated, "not_signed_in")
    );
    assert_eq!(
        (by_member.code(), by_member.message()),
        (Code::PermissionDenied, "not_admin")
    );
    assert_eq!(list_by_member.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn an_admin_invites_a_member_who_registers_through_the_link() {
    let server = TestServer::start().await;
    let admin = server.sign_up(&mut device(), "anna@example.se", None).await;

    let invite = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "Bo@Example.se".into(),
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner();
    let lookup = server
        .grpc()
        .get_invitation(pb::GetInvitationRequest {
            token: invite.token.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut phone = device();
    let member = server
        .sign_up(&mut phone, "bo@example.se", Some(&invite.token))
        .await;
    let used = server
        .grpc()
        .get_invitation(pb::GetInvitationRequest {
            token: invite.token.clone(),
        })
        .await
        .unwrap_err();
    let listed = server
        .grpc()
        .list_invitations(authed(pb::ListInvitationsRequest {}, &admin))
        .await
        .unwrap()
        .into_inner();
    let status = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &member))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(lookup.email, "bo@example.se");
    assert!(invite.expires_at.ends_with('Z'), "{}", invite.expires_at);
    assert_eq!(
        (used.code(), used.message()),
        (Code::NotFound, "invitation_not_found")
    );
    assert_eq!(listed.invitations.len(), 1);
    assert!(listed.invitations[0].accepted);
    assert_eq!(status.current_user.unwrap().role(), pb::Role::Member);
    assert!(server.log_in(&mut phone, "bo@example.se").await.is_ok());
}

#[tokio::test]
async fn a_signed_in_user_adds_and_lists_passkeys() {
    let server = TestServer::start().await;
    let session = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut phone = device();
    let mut grpc = server.grpc();

    let begin = grpc
        .begin_add_passkey(authed(
            pb::BeginAddPasskeyRequest {
                passkey_name: "Telefon".into(),
            },
            &session,
        ))
        .await
        .unwrap()
        .into_inner();
    let options: CreationChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
    let credential = phone
        .do_registration(server.origin.clone(), options)
        .unwrap();
    grpc.finish_add_passkey(authed(
        pb::FinishAddPasskeyRequest {
            ceremony_id: begin.ceremony_id,
            credential_json: serde_json::to_string(&credential).unwrap(),
        },
        &session,
    ))
    .await
    .unwrap();
    let passkeys = grpc
        .list_passkeys(authed(pb::ListPasskeysRequest {}, &session))
        .await
        .unwrap()
        .into_inner()
        .passkeys;

    let names: Vec<&str> = passkeys.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert!(server.log_in(&mut phone, "anna@example.se").await.is_ok());
}

#[tokio::test]
async fn errors_carry_stable_codes_for_the_frontend() {
    let server = TestServer::start().await;
    let mut grpc = server.grpc();
    let begin = |email: &str, name: &str| pb::BeginRegistrationRequest {
        email: email.into(),
        display_name: name.into(),
        invitation_token: None,
        passkey_name: "Laptop".into(),
    };

    let bad_email = grpc
        .begin_registration(begin("anna", "Anna"))
        .await
        .unwrap_err();
    let bad_name = grpc
        .begin_registration(begin("anna@example.se", " "))
        .await
        .unwrap_err();
    server.sign_up(&mut device(), "anna@example.se", None).await;
    let uninvited = grpc
        .begin_registration(begin("bo@example.se", "Bo"))
        .await
        .unwrap_err();
    let bad_ceremony = grpc
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: "x".into(),
            credential_json: "{}".into(),
        })
        .await
        .unwrap_err();
    let unknown_invite = grpc
        .get_invitation(Request::new(pb::GetInvitationRequest {
            token: "nope".into(),
        }))
        .await
        .unwrap_err();

    assert_eq!(
        (bad_email.code(), bad_email.message()),
        (Code::InvalidArgument, "invalid_email")
    );
    assert_eq!(
        (bad_name.code(), bad_name.message()),
        (Code::InvalidArgument, "invalid_display_name")
    );
    assert_eq!(
        (uninvited.code(), uninvited.message()),
        (Code::InvalidArgument, "invitation_required")
    );
    assert_eq!(
        (bad_ceremony.code(), bad_ceremony.message()),
        (Code::InvalidArgument, "invalid_ceremony")
    );
    assert_eq!(unknown_invite.code(), Code::NotFound);
}
```

- [ ] **Step 4: Run to verify they fail**

Run: `cargo test -p doris-server --test grpc`
Expected: FAIL to compile with `unresolved imports doris_server::AuthApi, doris_server::SESSION_COOKIE` (and `router`).

- [ ] **Step 5: Implement the gRPC service**

`crates/server/src/grpc.rs`:

```rust
//! `doris.auth.v1.AuthService`: maps gRPC calls onto `doris_identity`, and
//! carries the session in an HttpOnly cookie.

use doris_identity::domain::{DomainError, Role, User};
use doris_identity::{Auth, Error, SESSION_TTL};
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_server::AuthService;
use jiff::Timestamp;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "doris_session";

pub struct AuthApi {
    pool: SqlitePool,
    auth: Auth,
}

impl AuthApi {
    pub fn new(pool: SqlitePool, auth: Auth) -> Self {
        Self { pool, auth }
    }

    /// The signed-in user, or `Unauthenticated`.
    async fn user<T>(&self, request: &Request<T>) -> Result<User, Status> {
        let token = session_token(request).ok_or_else(not_signed_in)?;
        doris_identity::session_user(&self.pool, &token, Timestamp::now())
            .await
            .map_err(status)?
            .ok_or_else(not_signed_in)
    }

    async fn admin<T>(&self, request: &Request<T>) -> Result<User, Status> {
        let user = self.user(request).await?;
        match user.role {
            Role::Admin => Ok(user),
            Role::Member => Err(Status::permission_denied("not_admin")),
        }
    }
}

#[tonic::async_trait]
impl AuthService for AuthApi {
    async fn get_status(
        &self,
        request: Request<pb::GetStatusRequest>,
    ) -> Result<Response<pb::GetStatusResponse>, Status> {
        let current_user = match session_token(&request) {
            Some(token) => doris_identity::session_user(&self.pool, &token, Timestamp::now())
                .await
                .map_err(status)?,
            None => None,
        };
        Ok(Response::new(pb::GetStatusResponse {
            bootstrap_required: doris_identity::bootstrap_required(&self.pool)
                .await
                .map_err(status)?,
            current_user: current_user.as_ref().map(user_message),
        }))
    }

    async fn begin_registration(
        &self,
        request: Request<pb::BeginRegistrationRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let req = request.into_inner();
        let (ceremony_id, options) = self
            .auth
            .begin_registration(
                &req.email,
                &req.display_name,
                req.invitation_token.as_deref(),
                &req.passkey_name,
                Timestamp::now(),
            )
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_registration(
        &self,
        request: Request<pb::FinishRegistrationRequest>,
    ) -> Result<Response<pb::User>, Status> {
        let req = request.into_inner();
        let (user, session) = self
            .auth
            .finish_registration(
                ceremony_id(&req.ceremony_id)?,
                req.invitation_token.as_deref(),
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(with_session_cookie(user_message(&user), &session))
    }

    async fn begin_login(
        &self,
        request: Request<pb::BeginLoginRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let req = request.into_inner();
        let (ceremony_id, options) = self
            .auth
            .begin_login(&req.email, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_login(
        &self,
        request: Request<pb::FinishLoginRequest>,
    ) -> Result<Response<pb::User>, Status> {
        let req = request.into_inner();
        let (user, session) = self
            .auth
            .finish_login(
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(with_session_cookie(user_message(&user), &session))
    }

    async fn logout(
        &self,
        request: Request<pb::LogoutRequest>,
    ) -> Result<Response<pb::LogoutResponse>, Status> {
        if let Some(token) = session_token(&request) {
            doris_identity::end_session(&self.pool, &token)
                .await
                .map_err(status)?;
        }
        let mut response = Response::new(pb::LogoutResponse {});
        set_cookie(&mut response, "", 0);
        Ok(response)
    }

    async fn begin_add_passkey(
        &self,
        request: Request<pb::BeginAddPasskeyRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let (ceremony_id, options) = self
            .auth
            .begin_add_passkey(user.id, &request.get_ref().passkey_name, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_add_passkey(
        &self,
        request: Request<pb::FinishAddPasskeyRequest>,
    ) -> Result<Response<pb::FinishAddPasskeyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        self.auth
            .finish_add_passkey(
                user.id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(Response::new(pb::FinishAddPasskeyResponse {}))
    }

    async fn list_passkeys(
        &self,
        request: Request<pb::ListPasskeysRequest>,
    ) -> Result<Response<pb::ListPasskeysResponse>, Status> {
        let user = self.user(&request).await?;
        let passkeys = doris_identity::list_passkeys(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|p| pb::Passkey {
                credential_id: p.credential_id,
                name: p.name,
                added_at: p.added_at,
                last_used_at: p.last_used_at,
            })
            .collect();
        Ok(Response::new(pb::ListPasskeysResponse { passkeys }))
    }

    async fn get_invitation(
        &self,
        request: Request<pb::GetInvitationRequest>,
    ) -> Result<Response<pb::GetInvitationResponse>, Status> {
        let email = doris_identity::invitation_email(
            &self.pool,
            &request.get_ref().token,
            Timestamp::now(),
        )
        .await
        .map_err(status)?
        .ok_or_else(|| Status::not_found("invitation_not_found"))?;
        Ok(Response::new(pb::GetInvitationResponse {
            email: email.as_str().to_owned(),
        }))
    }

    async fn create_invitation(
        &self,
        request: Request<pb::CreateInvitationRequest>,
    ) -> Result<Response<pb::CreateInvitationResponse>, Status> {
        let admin = self.admin(&request).await?;
        let now = Timestamp::now();
        let (_, token) =
            doris_identity::create_invitation(&self.pool, admin.id, &request.get_ref().email, now)
                .await
                .map_err(status)?;
        Ok(Response::new(pb::CreateInvitationResponse {
            token,
            expires_at: (now + doris_identity::domain::INVITATION_TTL).to_string(),
        }))
    }

    async fn list_invitations(
        &self,
        request: Request<pb::ListInvitationsRequest>,
    ) -> Result<Response<pb::ListInvitationsResponse>, Status> {
        self.admin(&request).await?;
        let invitations = doris_identity::list_invitations(&self.pool)
            .await
            .map_err(status)?
            .into_iter()
            .map(|i| pb::Invitation {
                id: i.id.to_string(),
                email: i.email,
                expires_at: i.expires_at.to_string(),
                accepted: i.accepted,
            })
            .collect();
        Ok(Response::new(pb::ListInvitationsResponse { invitations }))
    }
}

fn user_message(user: &User) -> pb::User {
    pb::User {
        id: user.id.to_string(),
        email: user.email.as_str().to_owned(),
        display_name: user.display_name.as_str().to_owned(),
        role: match user.role {
            Role::Admin => pb::Role::Admin,
            Role::Member => pb::Role::Member,
        }
        .into(),
    }
}

fn ceremony_response<T: serde::Serialize>(
    ceremony_id: Uuid,
    options: &T,
) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
    Ok(Response::new(pb::BeginCeremonyResponse {
        ceremony_id: ceremony_id.to_string(),
        options_json: serde_json::to_string(options).map_err(|_| Status::internal("internal"))?,
    }))
}

fn ceremony_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_ceremony"))
}

fn credential<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, Status> {
    serde_json::from_str(json).map_err(|_| Status::invalid_argument("invalid_credential"))
}

fn session_token<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|cookie| {
            cookie
                .trim()
                .strip_prefix(SESSION_COOKIE)
                .and_then(|rest| rest.strip_prefix('='))
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
}

fn with_session_cookie<T>(message: T, token: &str) -> Response<T> {
    let mut response = Response::new(message);
    set_cookie(&mut response, token, SESSION_TTL.as_secs());
    response
}

fn set_cookie<T>(response: &mut Response<T>, token: &str, max_age: i64) {
    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}"
    );
    response.metadata_mut().insert(
        "set-cookie",
        cookie.parse().expect("cookie is valid header ascii"),
    );
}

fn not_signed_in() -> Status {
    Status::unauthenticated("not_signed_in")
}

/// Like [`status`], but a rejected WebAuthn credential is the client's fault.
fn finish_status(err: Error) -> Status {
    match err {
        Error::Webauthn(_) => Status::invalid_argument("credential_rejected"),
        other => status(other),
    }
}

/// Maps identity errors to gRPC statuses. Messages are stable codes the
/// frontend translates; they never contain personal data.
fn status(err: Error) -> Status {
    match err {
        Error::Domain(DomainError::NotAdmin) => Status::permission_denied("not_admin"),
        Error::Domain(err) => Status::invalid_argument(domain_code(err)),
        Error::AlreadyExists => Status::already_exists("already_exists"),
        Error::InvitationNotFound => Status::not_found("invitation_not_found"),
        Error::UserNotFound => Status::not_found("user_not_found"),
        Error::CeremonyNotFound | Error::CeremonyExpired => {
            Status::failed_precondition("ceremony_expired")
        }
        Error::LoginFailed => Status::unauthenticated("login_failed"),
        Error::Webauthn(err) => {
            tracing::error!("webauthn: {err}");
            Status::internal("internal")
        }
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}

fn domain_code(err: DomainError) -> &'static str {
    match err {
        DomainError::InvalidEmail => "invalid_email",
        DomainError::InvalidDisplayName => "invalid_display_name",
        DomainError::InvalidPasskeyName => "invalid_passkey_name",
        DomainError::InvitationRequired => "invitation_required",
        DomainError::InvitationExpired => "invitation_expired",
        DomainError::InvitationAlreadyUsed => "invitation_already_used",
        DomainError::InvitationEmailMismatch => "invitation_email_mismatch",
        DomainError::DuplicatePasskey => "duplicate_passkey",
        DomainError::UnknownPasskey => "unknown_passkey",
        DomainError::NotAdmin => "not_admin",
    }
}
```

`crates/server/src/lib.rs` for cycle A (gRPC only; cycle B replaces it):

```rust
//! The Doris server: gRPC-Web API and the embedded frontend on one port.

mod grpc;

use axum::Router;
use doris_proto::auth::v1::auth_service_server::AuthServiceServer;
use http::HeaderValue;
use rust_embed::RustEmbed;
use tonic::service::Routes;
use tonic_web::GrpcWebLayer;

pub use grpc::{AuthApi, SESSION_COOKIE};

/// Builds the app. `E` is the embedded frontend.
/// With no `cors_origins`, only same-origin browsers can call the API.
pub fn router<E: RustEmbed + Send + Sync + 'static>(
    api: AuthApi,
    _cors_origins: Vec<HeaderValue>,
    _serve_frontend: bool,
) -> Router {
    Routes::new(AuthServiceServer::new(api))
        .into_axum_router()
        .layer(GrpcWebLayer::new())
}
```

- [ ] **Step 6: Run to verify cycle A passes**

Run: `cargo test -p doris-server --test grpc`
Expected: PASS, 8 tests.

- [ ] **Step 7: Cycle B. Write the failing HTTP tests**

`crates/server/tests/http.rs`:

```rust
mod common;

use common::{TestServer, http};
use http::Method;
use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE,
};

#[tokio::test]
async fn the_root_serves_index_html_without_caching() {
    let server = TestServer::start().await;

    let response = http(Method::GET, &format!("{}/", server.base), &[]).await;

    assert_eq!(response.status(), 200);
    assert!(response.body().contains("<title>Doris</title>"));
    assert_eq!(response.headers()[CONTENT_TYPE], "text/html");
    assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn app_routes_fall_back_to_index_html() {
    let server = TestServer::start().await;

    let response = http(
        Method::GET,
        &format!("{}/admin/invitations", server.base),
        &[],
    )
    .await;

    assert_eq!(response.status(), 200);
    assert!(response.body().contains("<title>Doris</title>"));
}

#[tokio::test]
async fn hashed_assets_are_cached_forever_and_others_revalidated() {
    let server = TestServer::start().await;

    let hashed = http(
        Method::GET,
        &format!("{}/doris-web-0123456789abcdef.js", server.base),
        &[],
    )
    .await;
    let plain = http(Method::GET, &format!("{}/style.css", server.base), &[]).await;

    assert_eq!(hashed.status(), 200);
    assert_eq!(hashed.headers()[CONTENT_TYPE], "text/javascript");
    assert_eq!(
        hashed.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(plain.headers()[CONTENT_TYPE], "text/css");
    assert_eq!(plain.headers()[CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn missing_files_are_not_found_instead_of_index_html() {
    let server = TestServer::start().await;

    let response = http(Method::GET, &format!("{}/missing.js", server.base), &[]).await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn the_frontend_can_be_switched_off_for_cdn_deployments() {
    let server = TestServer::start_with(vec![], false).await;

    let response = http(Method::GET, &format!("{}/", server.base), &[]).await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn cors_preflight_is_allowed_only_for_configured_origins() {
    let cdn = "https://app.example.se";
    let server = TestServer::start_with(vec![cdn.parse().unwrap()], false).await;
    let url = format!("{}/doris.auth.v1.AuthService/GetStatus", server.base);
    let preflight = |origin: &'static str| {
        let url = url.clone();
        async move {
            http(
                Method::OPTIONS,
                &url,
                &[
                    ("origin", origin),
                    ("access-control-request-method", "POST"),
                    ("access-control-request-headers", "content-type,x-grpc-web"),
                ],
            )
            .await
        }
    };

    let allowed = preflight(cdn).await;
    let denied = preflight("https://evil.example").await;

    assert_eq!(allowed.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], cdn);
    assert_eq!(allowed.headers()[ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert!(!denied.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn without_cors_origins_no_cross_origin_access_is_granted() {
    let server = TestServer::start().await;

    let response = http(
        Method::OPTIONS,
        &format!("{}/doris.auth.v1.AuthService/GetStatus", server.base),
        &[
            ("origin", "https://app.example.se"),
            ("access-control-request-method", "POST"),
        ],
    )
    .await;

    assert!(!response.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
}
```

Run: `cargo test -p doris-server --test http`
Expected: FAIL. The static-file tests get non-200 responses because nothing serves files yet, and the CORS test finds no `access-control-allow-origin`. The `without_cors_origins…` test may already pass; that's fine.

- [ ] **Step 8: Implement assets, CORS and the binary**

`crates/server/src/assets.rs`:

```rust
//! Serves the frontend embedded in the binary, with a single-page-app
//! fallback to `index.html`.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The Trunk build output. Empty when the frontend has not been built.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../web/dist"]
#[allow_missing = true]
pub struct WebDist;

pub async fn serve<E: RustEmbed>(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (path, file) = match E::get(path) {
        Some(file) => (path, file),
        // Paths with an extension are files; anything else is an app route.
        None if path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.')) =>
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        None => match E::get("index.html") {
            Some(file) => ("index.html", file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let cache = if is_hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (CONTENT_TYPE, file.metadata.mimetype().to_owned()),
            (CACHE_CONTROL, cache.to_owned()),
        ],
        file.data,
    )
        .into_response()
}

/// Trunk names build outputs `<name>-<16 hex digits>.<ext>`; those never
/// change content, so browsers may cache them forever.
fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit_once('-')
        .is_some_and(|(_, hash)| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}
```

Replace `crates/server/src/lib.rs` with:

```rust
//! The Doris server: gRPC-Web API and the embedded frontend on one port.

pub mod assets;
mod grpc;

use axum::Router;
use axum::routing::get;
use doris_proto::auth::v1::auth_service_server::AuthServiceServer;
use http::header::CONTENT_TYPE;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use rust_embed::RustEmbed;
use tonic::service::Routes;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use grpc::{AuthApi, SESSION_COOKIE};

/// Builds the app. `E` is the embedded frontend (see [`assets::WebDist`]).
/// With no `cors_origins`, only same-origin browsers can call the API.
pub fn router<E: RustEmbed + Send + Sync + 'static>(
    api: AuthApi,
    cors_origins: Vec<HeaderValue>,
    serve_frontend: bool,
) -> Router {
    let mut app = Routes::new(AuthServiceServer::new(api))
        .into_axum_router()
        .layer(GrpcWebLayer::new());
    app = if serve_frontend {
        app.fallback_service(get(assets::serve::<E>))
    } else {
        app.fallback(|| async { StatusCode::NOT_FOUND })
    };
    if !cors_origins.is_empty() {
        app = app.layer(cors(cors_origins));
    }
    app
}

fn cors(origins: Vec<HeaderValue>) -> CorsLayer {
    let headers = |names: &[&'static str]| {
        names
            .iter()
            .map(|n| HeaderName::from_static(n))
            .collect::<Vec<_>>()
    };
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([Method::POST])
        .allow_headers(
            [
                vec![CONTENT_TYPE],
                headers(&["x-grpc-web", "x-user-agent", "grpc-timeout"]),
            ]
            .concat(),
        )
        .expose_headers(headers(&[
            "grpc-status",
            "grpc-message",
            "grpc-status-details-bin",
        ]))
}
```

Add to `crates/server/Cargo.toml` after the `[package]` section:

```toml
[[bin]]
name = "doris"
path = "src/main.rs"
```

`crates/server/src/main.rs`:

```rust
use clap::Parser;
use doris_identity::Auth;
use doris_server::{AuthApi, assets::WebDist};
use http::HeaderValue;
use std::net::SocketAddr;
use url::Url;

/// Doris bookkeeping server.
#[derive(Parser)]
struct Config {
    /// SQLite database URL.
    #[arg(long, env = "DORIS_DATABASE", default_value = "sqlite://doris.db")]
    database: String,
    #[arg(long, env = "DORIS_LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
    /// WebAuthn relying party id: the domain users see, without scheme or port.
    #[arg(long, env = "DORIS_RP_ID", default_value = "localhost")]
    rp_id: String,
    /// Origin the browser loads the frontend from.
    #[arg(long, env = "DORIS_RP_ORIGIN", default_value = "http://localhost:3000")]
    rp_origin: Url,
    /// Other origins allowed to call the API (frontend on a CDN), comma separated.
    #[arg(long, env = "DORIS_CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<HeaderValue>,
    /// Serve the embedded frontend.
    #[arg(long, env = "DORIS_SERVE_FRONTEND", default_value_t = true, action = clap::ArgAction::Set)]
    serve_frontend: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = Config::parse();
    let pool = doris_eventstore::open(&config.database).await?;
    let auth = Auth::new(pool.clone(), &config.rp_id, &config.rp_origin).await?;
    let app = doris_server::router::<WebDist>(
        AuthApi::new(pool, auth),
        config.cors_origins,
        config.serve_frontend,
    );
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("listening on http://{}", config.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 9: Run all tests to verify they pass**

Run: `make test`
Expected: PASS. The suites are:

| Suite | Tests |
|---|---|
| eventstore | 5 |
| domain | 11 |
| queries | 4 |
| session | 5 |
| store | 15 |
| webauthn | 15 |
| server grpc | 8 |
| server http | 7 |

- [ ] **Step 10: Smoke-test the binary**

```bash
cargo build -p doris-server
DORIS_DATABASE=sqlite:///tmp/doris-smoke.db DORIS_LISTEN=127.0.0.1:38123 ./target/debug/doris &
sleep 1; curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:38123/; kill %1; rm -f /tmp/doris-smoke.db*
./target/debug/doris --help
```

Expected:
- The log shows `listening on http://127.0.0.1:38123`.
- `curl` prints `404`, because the frontend isn't built yet and `crates/web/dist` is missing.
- `--help` lists every `DORIS_*` variable.

- [ ] **Step 11: Document in AGENTS.md**

In `AGENTS.md`, add after the `## Authentication` section:

```markdown
## API
- The contract lives in `proto/doris/auth/v1/auth.proto`. `doris-proto` generates
  the client; its `server` feature adds the server stubs. The client builds for
  wasm32 because no transport is generated.
- gRPC-Web over HTTP/1.1 (`tonic_web::GrpcWebLayer`) shares one port with the
  embedded frontend. Integration tests speak gRPC-Web too (`GrpcWebClientLayer`
  over a hyper client), exactly like the browser.
- Error statuses carry stable snake_case codes as the message, for example
  `invalid_email`, `not_signed_in`, `not_admin`, `login_failed` and
  `ceremony_expired`. The frontend translates them. They never contain
  personal data. The mapping is in `crates/server/src/grpc.rs` (`status`,
  `domain_code`).
- The session cookie is `doris_session` (HttpOnly, Secure, SameSite=Strict,
  Path=/, 30 days).
- CORS is off unless `DORIS_CORS_ORIGINS` is set. Set it only when the frontend
  is served from another origin on the same site; WebAuthn's RP ID must still
  match.
```

- [ ] **Step 12: Refactor check and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add Cargo.toml Cargo.lock AGENTS.md proto crates/proto crates/server
git commit -m "Add gRPC-Web server with session cookies, CORS and embedded frontend"
```
