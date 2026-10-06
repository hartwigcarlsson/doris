# API-tokens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** En användare skapar en API-token med behörigheter per bolag (område × läs/skriv) och kan med den anropa Doris gRPC-Web-API utan passkey, som en kommande `doris-cli` eller AI-agent ska göra.

**Architecture:** Tokens är events i identity (`api-token-{id}`, projektionen `api_tokens`). Ett lager i servern (`auth_gate`) slår upp en bearer-token, nekar det som tabellen i `access.rs` inte släpper till tokens, lägger `TokenCaller` i anropets extensions och kör anropet med `doris_eventstore::VIA_TOKEN` satt, så att varje event får `via_token` i metadata. Tjänsternas `caller()` kontrollerar att token har behörigheten för just det bolaget; modulerna kontrollerar medlemskap som förut. AuthService får `CreateApiToken`/`ListApiTokens`/`RevokeApiToken`, och webben får sidorna `/settings/tokens` och `/settings/tokens/new`.

**Tech Stack:** Rust 2024, jiff, serde, sqlx 0.9 (SQLite), tokio (`task_local!`), tonic 0.14 + prost, axum-middleware, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-06-api-tokens-design.md`

## Global Constraints
- Kod, identifierare, URL:er, proto, händelsenamn och commits på engelska; bara synlig UI-text på svenska.
- TDD: inget utan ett test som först fallerar. Varje cykel slutar i en commit.
- `events` är append-only. Händelser är JSON med `schema_version` 1 (envelopet). Ändra aldrig befintliga händelsers betydelse.
- Projektioner uppdateras i samma transaktion som appenden (`doris_eventstore::begin`, `BEGIN IMMEDIATE`) och kan byggas om från `read_all`.
- Token-format: `doris_` + `token::new_token()`. Bara `token::hash_token(secret)` sparas. Token loggas aldrig, varken klartext eller hash.
- En token är en delmängd av ägarens åtkomst; medlemskap kontrolleras av modulen vid varje anrop.
- Behörigheter: `ledger:read`, `ledger:write`, `invoicing:read`, `invoicing:write`, `payroll:read`, `payroll:write`, `vat:read`, `vat:write`, `company:read`.
- Utgång: sista giltiga dag högst 366 dagar efter i dag; token slutar gälla vid midnatt svensk tid efter den dagen; domänen kräver `now < expires_at <= now + 367 dagar`.
- Felkoder (snake_case, utan personuppgifter): `invalid_token_name`, `invalid_token_expiry`, `invalid_token_grants`, `api_token_not_found`, `missing_scope`, `token_not_allowed`. Utgången/okänd/återkallad token: `not_signed_in`. Bolag utanför grants: `company_not_found`.
- En e-postadress eller ett personnummer loggas aldrig.
- UI följer `docs/design/README.md`: `PageHeader`, `TableCard`, `Badge`, `LinkButton`, endast tokens, ljust/mörkt, 390 px. Sidor startar uppgifter med `crate::task::spawn_local`.
- Lint: `cargo clippy --workspace -- -D warnings` och `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.

## Review Focus
1. **Schemat skrivet med små bokstäver eller med mellanslag** (`authorization: bearer doris_…`, eller en token med ett avslutande mellanslag från en kopierad rad): HTTP-schemat är skiftlägesokänsligt, så det ska fungera. Test i Task 6 (`the_bearer_scheme_is_case_insensitive_and_spaces_are_ignored`).
2. **Sista giltiga dag i dag, och över sommartidens gräns**: en token vars sista dag är i dag fungerar resten av dagen och slutar vid svensk midnatt, både vintertid (23:00Z) och sommartid (22:00Z). Test i Task 4 (`a_token_ends_at_midnight_in_sweden_after_its_last_day`).
3. **Olika behörighet i två bolag**: en token med skriv i A och bara läs i B bokför i A men får `missing_scope` i B. Test i Task 6 (`scopes_are_per_company`).
4. **Återkallande från två flikar**: den andra återkallningen lyckas utan att något händer. Test i Task 4 (`revoking_twice_is_fine`).
5. **Formuläret skickat utan någon kryssruta**: servern svarar `invalid_token_grants`, som webben visar på svenska. Test i Task 4 (`a_token_without_grants_is_refused`) och Task 7 (felkoden har en text).

---

### Task 1: `via_token` i eventens metadata

**Files:**
- Modify: `crates/eventstore/Cargo.toml`
- Modify: `crates/eventstore/src/lib.rs:31-35` (Metadata), `:131-160` (append)
- Modify: `crates/identity/src/lib.rs:338`, `crates/company/src/lib.rs:175`, `crates/invoicing/src/lib.rs:255`, `crates/ledger/src/lib.rs:219`, `crates/payroll/src/lib.rs:383`, `crates/vat/src/lib.rs:157`, `crates/ledger/tests/store.rs:1964`
- Test: `crates/eventstore/tests/eventstore.rs`

**Interfaces:**
- Produces: `doris_eventstore::Metadata { actor: Option<String>, via_token: Option<String> }` (`Default`); `doris_eventstore::VIA_TOKEN: tokio::task::LocalKey<String>`. `append` fyller i `via_token` från `VIA_TOKEN` när anroparens metadata saknar det.

- [ ] **Step 1: Skriv de fallerande testerna**

Lägg till i `crates/eventstore/tests/eventstore.rs` och ändra `actor()`:

```rust
fn actor() -> Metadata {
    Metadata {
        actor: Some("tester".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn events_appended_within_a_token_scope_record_the_token() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut tx = begin(&pool).await.unwrap();
    let recorded = doris_eventstore::VIA_TOKEN
        .scope(
            "token-1".to_owned(),
            append(&mut tx, "counter-1", 0, &[event(1)], &actor()),
        )
        .await
        .unwrap();
    append(&mut tx, "counter-1", 1, &[event(2)], &actor())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let events = load(&mut conn, "counter-1").await.unwrap();
    assert_eq!(recorded[0].metadata.via_token.as_deref(), Some("token-1"));
    assert_eq!(events[0].metadata.via_token.as_deref(), Some("token-1"));
    assert_eq!(events[0].metadata.actor.as_deref(), Some("tester"));
    assert_eq!(events[1].metadata, actor());
}

#[test]
fn metadata_without_a_token_reads_and_writes_as_before() {
    let old: Metadata = serde_json::from_str(r#"{"actor":"u1"}"#).unwrap();
    assert_eq!(
        old,
        Metadata {
            actor: Some("u1".into()),
            via_token: None
        }
    );
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"actor":"u1"}"#);
}
```

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-eventstore --test eventstore`
Expected: kompileringsfel, `no field via_token` och `cannot find value VIA_TOKEN`.

- [ ] **Step 3: Implementera**

`crates/eventstore/Cargo.toml`: flytta `tokio.workspace = true` från `[dev-dependencies]` till `[dependencies]`.

`crates/eventstore/src/lib.rs`:

```rust
/// Who caused an event. Stored with every event (behandlingshistorik).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub actor: Option<String>,
    /// The API token the actor used, if any. [`append`] fills it from
    /// [`VIA_TOKEN`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_token: Option<String>,
}

tokio::task_local! {
    /// The API token the current request runs with. The server sets it for
    /// the whole call, so every event appended meanwhile records it.
    pub static VIA_TOKEN: String;
}
```

I `append`, ersätt `let metadata = serde_json::to_string(metadata)?;` med:

```rust
    let metadata = Metadata {
        via_token: metadata
            .via_token
            .clone()
            .or_else(|| VIA_TOKEN.try_with(Clone::clone).ok()),
        ..metadata.clone()
    };
    let metadata = serde_json::to_string(&metadata)?;
```

I var och en av de sju andra platserna som bygger `Metadata { actor: … }` (listade under Files), lägg till `..Default::default()` efter `actor`-fältet, till exempel i `crates/ledger/src/lib.rs`:

```rust
    let metadata = Metadata {
        actor: Some(actor.to_string()),
        ..Default::default()
    };
```

- [ ] **Step 4: Kör testerna**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/eventstore crates/identity/src/lib.rs crates/company/src/lib.rs crates/invoicing/src/lib.rs crates/ledger/src/lib.rs crates/ledger/tests/store.rs crates/payroll/src/lib.rs crates/vat/src/lib.rs
git commit -m "Record the API token an event was appended under"
```

---

### Task 2: Domänen för tokens

**Files:**
- Modify: `crates/identity/src/domain.rs`
- Test: `crates/identity/tests/api_token_domain.rs` (ny)

**Interfaces:**
- Produces (i `doris_identity::domain`):
  - `pub const MAX_TOKEN_LIFETIME: SignedDuration` (367 dagar)
  - `pub enum Scope { LedgerRead, LedgerWrite, InvoicingRead, InvoicingWrite, PayrollRead, PayrollWrite, VatRead, VatWrite, CompanyRead }` med `Scope::ALL: [Scope; 9]`, `fn as_str(self) -> &'static str`, `fn parse(&str) -> Option<Scope>`; serialiseras som `"ledger:read"` osv.
  - `pub struct Grant { pub company_id: Uuid, pub scopes: Vec<Scope> }`
  - `pub enum ApiTokenEvent { ApiTokenCreated { token_id, user_id, name: String, token_hash: String, expires_at: Timestamp, grants: Vec<Grant> }, ApiTokenRevoked { revoked_by: Uuid } }`
  - `pub struct ApiToken { pub id, pub user_id, pub name, pub expires_at, pub grants, pub revoked: bool }` med `ApiToken::from_events`
  - `pub struct NewApiToken { pub token_id: Uuid, pub name: String, pub expires_at: Timestamp, pub grants: Vec<Grant> }`
  - `pub fn create_api_token(owner: &User, cmd: NewApiToken, token_hash: String, now: Timestamp) -> Result<ApiTokenEvent, DomainError>`
  - `pub fn revoke_api_token(token: &ApiToken, actor: &User) -> Result<Vec<ApiTokenEvent>, DomainError>`
  - Nya `DomainError`: `InvalidTokenName`, `InvalidTokenExpiry`, `InvalidTokenGrants`, `NotTokenOwner`

- [ ] **Step 1: Skriv de fallerande testerna**

`crates/identity/tests/api_token_domain.rs`:

```rust
use doris_identity::domain::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-10-06T10:00:00Z".parse().unwrap()
}

fn user(role: Role) -> User {
    User {
        id: Uuid::new_v4(),
        email: Email::parse("anna@example.se").unwrap(),
        display_name: DisplayName::parse("Anna").unwrap(),
        role,
        passkeys: vec![Passkey::new("c1".into(), "Laptop", json!({})).unwrap()],
    }
}

fn grant(company_id: Uuid, scopes: &[Scope]) -> Grant {
    Grant {
        company_id,
        scopes: scopes.to_vec(),
    }
}

fn cmd(name: &str, expires_in: SignedDuration, grants: Vec<Grant>) -> NewApiToken {
    NewApiToken {
        token_id: Uuid::new_v4(),
        name: name.into(),
        expires_at: now() + expires_in,
        grants,
    }
}

const DAY: SignedDuration = SignedDuration::from_hours(24);

#[test]
fn scopes_are_written_as_area_and_level() {
    assert_eq!(Scope::LedgerWrite.as_str(), "ledger:write");
    assert_eq!(Scope::parse("payroll:read"), Some(Scope::PayrollRead));
    assert_eq!(Scope::parse("ledger:admin"), None);
    for scope in Scope::ALL {
        assert_eq!(Scope::parse(scope.as_str()), Some(scope));
        assert_eq!(
            serde_json::to_value(scope).unwrap(),
            json!(scope.as_str())
        );
    }
}

#[test]
fn an_owner_creates_a_token_with_sorted_unique_scopes() {
    let anna = user(Role::Member);
    let company = Uuid::new_v4();
    let cmd = cmd(
        "  Agent  ",
        30 * DAY,
        vec![grant(
            company,
            &[Scope::LedgerWrite, Scope::LedgerRead, Scope::LedgerWrite],
        )],
    );
    let token_id = cmd.token_id;

    let event = create_api_token(&anna, cmd, "hash".into(), now()).unwrap();

    assert_eq!(
        event,
        ApiTokenEvent::ApiTokenCreated {
            token_id,
            user_id: anna.id,
            name: "Agent".into(),
            token_hash: "hash".into(),
            expires_at: now() + 30 * DAY,
            grants: vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])],
        }
    );
}

#[test]
fn a_token_needs_a_name_of_1_to_100_characters() {
    let anna = user(Role::Member);
    let grants = || vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])];
    for name in ["   ", &"å".repeat(101)] {
        assert_eq!(
            create_api_token(&anna, cmd(name, DAY, grants()), "h".into(), now()),
            Err(DomainError::InvalidTokenName),
            "{name:?}"
        );
    }
    assert!(create_api_token(&anna, cmd(&"å".repeat(100), DAY, grants()), "h".into(), now()).is_ok());
}

#[test]
fn a_token_expires_after_now_and_within_367_days() {
    let anna = user(Role::Member);
    let grants = || vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])];
    for expires_in in [SignedDuration::ZERO, -DAY, 367 * DAY + SignedDuration::from_secs(1)] {
        assert_eq!(
            create_api_token(&anna, cmd("Agent", expires_in, grants()), "h".into(), now()),
            Err(DomainError::InvalidTokenExpiry),
            "{expires_in:?}"
        );
    }
    assert!(create_api_token(&anna, cmd("Agent", 367 * DAY, grants()), "h".into(), now()).is_ok());
}

#[test]
fn grants_need_a_company_once_and_a_scope_each() {
    let anna = user(Role::Member);
    let company = Uuid::new_v4();
    for grants in [
        vec![],
        vec![grant(company, &[])],
        vec![
            grant(company, &[Scope::LedgerRead]),
            grant(company, &[Scope::VatRead]),
        ],
    ] {
        assert_eq!(
            create_api_token(&anna, cmd("Agent", DAY, grants.clone()), "h".into(), now()),
            Err(DomainError::InvalidTokenGrants),
            "{grants:?}"
        );
    }
}

fn token_of(owner: &User) -> ApiToken {
    let event = create_api_token(
        owner,
        cmd("Agent", DAY, vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])]),
        "h".into(),
        now(),
    )
    .unwrap();
    ApiToken::from_events(&[event]).unwrap()
}

#[test]
fn the_owner_or_an_admin_revokes_a_token() {
    let anna = user(Role::Member);
    let admin = user(Role::Admin);
    let token = token_of(&anna);

    assert_eq!(
        revoke_api_token(&token, &anna).unwrap(),
        vec![ApiTokenEvent::ApiTokenRevoked { revoked_by: anna.id }]
    );
    assert_eq!(
        revoke_api_token(&token, &admin).unwrap(),
        vec![ApiTokenEvent::ApiTokenRevoked { revoked_by: admin.id }]
    );
}

#[test]
fn someone_else_cannot_revoke_a_token() {
    let token = token_of(&user(Role::Member));
    assert_eq!(
        revoke_api_token(&token, &user(Role::Member)),
        Err(DomainError::NotTokenOwner)
    );
}

#[test]
fn a_revoked_token_is_revoked_once() {
    let anna = user(Role::Member);
    let created = create_api_token(
        &anna,
        cmd("Agent", DAY, vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])]),
        "h".into(),
        now(),
    )
    .unwrap();
    let revoked = ApiTokenEvent::ApiTokenRevoked { revoked_by: anna.id };
    let token = ApiToken::from_events(&[created, revoked]).unwrap();

    assert!(token.revoked);
    assert_eq!(revoke_api_token(&token, &anna).unwrap(), vec![]);
}
```

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-identity --test api_token_domain`
Expected: kompileringsfel, `cannot find type Scope` m.fl.

- [ ] **Step 3: Implementera**

I `crates/identity/src/domain.rs`, lägg till i `DomainError`:

```rust
    #[error("token name must be 1-100 characters")]
    InvalidTokenName,
    #[error("token must expire after now and within 367 days")]
    InvalidTokenExpiry,
    #[error("token needs each company once, each with a scope")]
    InvalidTokenGrants,
    #[error("only the owner or an admin may revoke a token")]
    NotTokenOwner,
```

Och i slutet av filen:

```rust
/// A token lives at most a year: its last day may be 366 days off, and it
/// ends at the following midnight in Sweden (an hour's summer time to spare).
pub const MAX_TOKEN_LIFETIME: SignedDuration = SignedDuration::from_hours(24 * 367);

/// What an API token may do in one company. Stored as its string, so a
/// scope added later changes no old event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "ledger:read")]
    LedgerRead,
    #[serde(rename = "ledger:write")]
    LedgerWrite,
    #[serde(rename = "invoicing:read")]
    InvoicingRead,
    #[serde(rename = "invoicing:write")]
    InvoicingWrite,
    #[serde(rename = "payroll:read")]
    PayrollRead,
    #[serde(rename = "payroll:write")]
    PayrollWrite,
    #[serde(rename = "vat:read")]
    VatRead,
    #[serde(rename = "vat:write")]
    VatWrite,
    #[serde(rename = "company:read")]
    CompanyRead,
}

impl Scope {
    pub const ALL: [Scope; 9] = [
        Scope::LedgerRead,
        Scope::LedgerWrite,
        Scope::InvoicingRead,
        Scope::InvoicingWrite,
        Scope::PayrollRead,
        Scope::PayrollWrite,
        Scope::VatRead,
        Scope::VatWrite,
        Scope::CompanyRead,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::LedgerRead => "ledger:read",
            Scope::LedgerWrite => "ledger:write",
            Scope::InvoicingRead => "invoicing:read",
            Scope::InvoicingWrite => "invoicing:write",
            Scope::PayrollRead => "payroll:read",
            Scope::PayrollWrite => "payroll:write",
            Scope::VatRead => "vat:read",
            Scope::VatWrite => "vat:write",
            Scope::CompanyRead => "company:read",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == raw)
    }
}

/// The scopes a token has in one company.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub company_id: Uuid,
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApiTokenEvent {
    /// When, and by whom, is in the event's envelope and metadata.
    ApiTokenCreated {
        token_id: Uuid,
        user_id: Uuid,
        name: String,
        token_hash: String,
        expires_at: Timestamp,
        grants: Vec<Grant>,
    },
    ApiTokenRevoked {
        revoked_by: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub expires_at: Timestamp,
    pub grants: Vec<Grant>,
    pub revoked: bool,
}

impl ApiToken {
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a ApiTokenEvent>) -> Option<Self> {
        let mut token: Option<Self> = None;
        for event in events {
            match (event, token.as_mut()) {
                (
                    ApiTokenEvent::ApiTokenCreated {
                        token_id,
                        user_id,
                        name,
                        expires_at,
                        grants,
                        ..
                    },
                    _,
                ) => {
                    token = Some(Self {
                        id: *token_id,
                        user_id: *user_id,
                        name: name.clone(),
                        expires_at: *expires_at,
                        grants: grants.clone(),
                        revoked: false,
                    });
                }
                (ApiTokenEvent::ApiTokenRevoked { .. }, Some(token)) => token.revoked = true,
                (_, None) => {}
            }
        }
        token
    }
}

#[derive(Debug, Clone)]
pub struct NewApiToken {
    pub token_id: Uuid,
    pub name: String,
    pub expires_at: Timestamp,
    pub grants: Vec<Grant>,
}

pub fn create_api_token(
    owner: &User,
    cmd: NewApiToken,
    token_hash: String,
    now: Timestamp,
) -> Result<ApiTokenEvent, DomainError> {
    let name = bounded_text(&cmd.name, 100).ok_or(DomainError::InvalidTokenName)?;
    if cmd.expires_at <= now || cmd.expires_at > now + MAX_TOKEN_LIFETIME {
        return Err(DomainError::InvalidTokenExpiry);
    }
    Ok(ApiTokenEvent::ApiTokenCreated {
        token_id: cmd.token_id,
        user_id: owner.id,
        name,
        token_hash,
        expires_at: cmd.expires_at,
        grants: normalized(cmd.grants)?,
    })
}

/// Companies in id order, each once; scopes sorted, each once.
fn normalized(mut grants: Vec<Grant>) -> Result<Vec<Grant>, DomainError> {
    grants.sort_by_key(|g| g.company_id);
    let repeated = grants.windows(2).any(|w| w[0].company_id == w[1].company_id);
    if grants.is_empty() || repeated {
        return Err(DomainError::InvalidTokenGrants);
    }
    for grant in &mut grants {
        grant.scopes.sort();
        grant.scopes.dedup();
        if grant.scopes.is_empty() {
            return Err(DomainError::InvalidTokenGrants);
        }
    }
    Ok(grants)
}

/// Idempotent: revoking a revoked token yields no events.
pub fn revoke_api_token(token: &ApiToken, actor: &User) -> Result<Vec<ApiTokenEvent>, DomainError> {
    if token.user_id != actor.id && actor.role != Role::Admin {
        return Err(DomainError::NotTokenOwner);
    }
    if token.revoked {
        return Ok(vec![]);
    }
    Ok(vec![ApiTokenEvent::ApiTokenRevoked { revoked_by: actor.id }])
}
```

`DomainError` derivar `Copy`; de nya varianterna har inga fält, så det håller.

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity`
Expected: PASS. (`crates/server/src/grpc.rs` `domain_code` slutar kompilera eftersom matchningen inte längre är fullständig; lägg därför redan nu till armarna där:)

```rust
        DomainError::InvalidTokenName => "invalid_token_name",
        DomainError::InvalidTokenExpiry => "invalid_token_expiry",
        DomainError::InvalidTokenGrants => "invalid_token_grants",
        DomainError::NotTokenOwner => "api_token_not_found",
```

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/identity crates/server/src/grpc.rs
git commit -m "Decide API tokens: scopes per company, expiry and revocation"
```

---

### Task 3: Lagring och uppslag av tokens

**Files:**
- Create: `migrations/0015_api_tokens.sql`
- Create: `crates/identity/src/api_token.rs`
- Modify: `crates/identity/src/lib.rs` (modul, export, `API_TOKEN_STREAM`, `Error::ApiTokenNotFound`)
- Modify: `crates/identity/src/projections.rs`
- Test: `crates/identity/tests/api_tokens.rs` (ny)

**Interfaces:**
- Consumes: Task 2:s domän.
- Produces (i `doris_identity`):
  - `pub const API_TOKEN_PREFIX: &str = "doris_"`
  - `pub async fn create_api_token(pool: &SqlitePool, owner_id: Uuid, name: &str, expires_at: Timestamp, grants: Vec<Grant>, now: Timestamp) -> Result<(Uuid, String)>`
  - `pub async fn revoke_api_token(pool: &SqlitePool, actor_id: Uuid, token_id: Uuid) -> Result<()>`
  - `pub struct ApiTokenSummary { pub id: Uuid, pub name: String, pub grants: Vec<Grant>, pub created_at: String, pub expires_at: Timestamp, pub last_used_at: Option<Timestamp>, pub revoked_at: Option<String> }`
  - `pub async fn list_api_tokens(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<ApiTokenSummary>>` (nyast först)
  - `pub struct TokenAccess { pub token_id: Uuid, pub grants: Vec<Grant> }` (`Clone`)
  - `pub async fn token_user(pool: &SqlitePool, secret: &str, now: Timestamp) -> Result<Option<(User, TokenAccess)>>`
  - `pub async fn touch_api_token(pool: &SqlitePool, token_id: Uuid, now: Timestamp) -> Result<()>`
  - `Error::ApiTokenNotFound`

- [ ] **Step 1: Skriv de fallerande testerna**

`crates/identity/tests/api_tokens.rs`:

```rust
use doris_identity::domain::{DomainError, Grant, Passkey, Scope};
use doris_identity::{
    Error, create_api_token, create_invitation, list_api_tokens, rebuild_projections, register,
    revoke_api_token, token_user, touch_api_token,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-10-06T10:00:00Z".parse().unwrap()
}

const DAY: SignedDuration = SignedDuration::from_hours(24);

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// The first user (admin) and an invited member.
async fn admin_and_member(pool: &SqlitePool) -> (Uuid, Uuid) {
    let passkey = |id: &str| Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap();
    let admin = register(pool, Uuid::new_v4(), "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, invitation) = create_invitation(pool, admin.id, "bo@example.se", now()).await.unwrap();
    let member = register(pool, Uuid::new_v4(), "bo@example.se", "Bo", Some(&invitation), passkey("c2"), now())
        .await
        .unwrap();
    (admin.id, member.id)
}

fn grants(company: Uuid) -> Vec<Grant> {
    vec![Grant {
        company_id: company,
        scopes: vec![Scope::LedgerRead],
    }]
}

#[tokio::test]
async fn a_created_token_finds_its_owner_and_grants() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let company = Uuid::new_v4();

    let (id, secret) = create_api_token(&pool, bo, "Agent", now() + 30 * DAY, grants(company), now())
        .await
        .unwrap();

    assert!(secret.starts_with("doris_") && secret.len() > 40, "{secret}");
    let (user, access) = token_user(&pool, &secret, now()).await.unwrap().unwrap();
    assert_eq!(user.id, bo);
    assert_eq!(access.token_id, id);
    assert_eq!(access.grants, grants(company));
    // Only the hash is stored.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE payload LIKE ?")
        .bind(format!("%{}%", &secret["doris_".len()..]))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn unknown_expired_and_revoked_tokens_find_no_one() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (id, secret) = create_api_token(&pool, bo, "Agent", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();

    assert!(token_user(&pool, "doris_nope", now()).await.unwrap().is_none());
    assert!(token_user(&pool, &secret["doris_".len()..], now()).await.unwrap().is_none());
    assert!(token_user(&pool, &secret, now() + DAY).await.unwrap().is_none());

    revoke_api_token(&pool, bo, id).await.unwrap();
    assert!(token_user(&pool, &secret, now()).await.unwrap().is_none());
}

#[tokio::test]
async fn tokens_are_listed_newest_first_with_their_state() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let company = Uuid::new_v4();
    let (first, _) = create_api_token(&pool, bo, "Första", now() + DAY, grants(company), now())
        .await
        .unwrap();
    let (second, _) = create_api_token(&pool, bo, "Andra", now() + 2 * DAY, grants(company), now())
        .await
        .unwrap();
    revoke_api_token(&pool, bo, first).await.unwrap();
    touch_api_token(&pool, second, now()).await.unwrap();

    let list = list_api_tokens(&pool, bo).await.unwrap();

    assert_eq!(list.iter().map(|t| t.id).collect::<Vec<_>>(), [second, first]);
    assert_eq!(list[0].name, "Andra");
    assert_eq!(list[0].grants, grants(company));
    assert_eq!(list[0].expires_at, now() + 2 * DAY);
    assert_eq!(list[0].last_used_at, Some(now()));
    assert_eq!(list[0].revoked_at, None);
    assert_eq!(list[1].last_used_at, None);
    assert!(list[1].revoked_at.is_some());
}

#[tokio::test]
async fn last_use_is_written_at_most_once_an_hour() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (id, _) = create_api_token(&pool, bo, "Agent", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    let last_used = || async { list_api_tokens(&pool, bo).await.unwrap()[0].last_used_at };

    touch_api_token(&pool, id, now()).await.unwrap();
    touch_api_token(&pool, id, now() + SignedDuration::from_mins(59)).await.unwrap();
    assert_eq!(last_used().await, Some(now()));

    touch_api_token(&pool, id, now() + SignedDuration::from_mins(60)).await.unwrap();
    assert_eq!(last_used().await, Some(now() + SignedDuration::from_mins(60)));
}

#[tokio::test]
async fn an_admin_revokes_any_token_and_others_see_none() {
    let pool = db().await;
    let (anna, bo) = admin_and_member(&pool).await;
    let (annas, _) = create_api_token(&pool, anna, "Anna", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    let (bos, _) = create_api_token(&pool, bo, "Bo", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();

    assert!(matches!(
        revoke_api_token(&pool, bo, annas).await,
        Err(Error::Domain(DomainError::NotTokenOwner))
    ));
    assert!(matches!(
        revoke_api_token(&pool, bo, Uuid::new_v4()).await,
        Err(Error::ApiTokenNotFound)
    ));
    revoke_api_token(&pool, anna, bos).await.unwrap();
    // Again, as from a second tab: nothing happens.
    revoke_api_token(&pool, anna, bos).await.unwrap();
    assert!(list_api_tokens(&pool, bo).await.unwrap()[0].revoked_at.is_some());
}

#[tokio::test]
async fn api_tokens_rebuild_from_the_log() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (first, _) = create_api_token(&pool, bo, "Första", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    create_api_token(&pool, bo, "Andra", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    revoke_api_token(&pool, bo, first).await.unwrap();
    let snapshot = || async {
        let rows: Vec<(String, String, String, String, String, String, i64, Option<String>)> =
            sqlx::query_as("SELECT * FROM api_tokens ORDER BY token_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        rows
    };
    let before = snapshot().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(snapshot().await, before);
    assert_eq!(before.len(), 2);
}
```

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-identity --test api_tokens`
Expected: kompileringsfel, `cannot find function create_api_token`.

- [ ] **Step 3: Implementera**

`migrations/0015_api_tokens.sql`:

```sql
-- Projection of ApiTokenCreated/ApiTokenRevoked (streams api-token-{id}).
-- Rebuildable. Only the SHA-256 of a token is stored.
CREATE TABLE api_tokens (
    token_id    TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    name        TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    grants      TEXT NOT NULL,      -- JSON, as in the event
    created_at  TEXT NOT NULL,      -- recorded_at
    expires_at  INTEGER NOT NULL,   -- unix seconds
    revoked_at  TEXT
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- When a token was last used. Operational state, not events: safe to purge.
CREATE TABLE api_token_usage (
    token_id     TEXT PRIMARY KEY,
    last_used_at INTEGER NOT NULL   -- unix seconds
);
```

`crates/identity/src/lib.rs`:
- `mod api_token;` bland modulerna, och
  `pub use api_token::{API_TOKEN_PREFIX, ApiTokenSummary, TokenAccess, create_api_token, list_api_tokens, revoke_api_token, token_user, touch_api_token};`
- `const API_TOKEN_STREAM: &str = "api-token-";` bredvid `INVITATION_STREAM`.
- I `Error`: `#[error("api token not found")] ApiTokenNotFound,` efter `UserNotFound`.

`crates/identity/src/api_token.rs`:

```rust
//! API tokens: event-sourced grants per company, looked up by hash on
//! every call. When a token was last used is operational state.

use crate::domain::{self, ApiToken, ApiTokenEvent, Grant, NewApiToken, User};
use crate::{API_TOKEN_STREAM, Error, Result, commit, get_user, load_stream, load_user, token};
use jiff::Timestamp;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Every API token starts with this, so it is recognisable in a config file.
pub const API_TOKEN_PREFIX: &str = "doris_";
/// Last use is written at most this often (seconds), so reads rarely write.
const USAGE_INTERVAL: i64 = 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiTokenSummary {
    pub id: Uuid,
    pub name: String,
    pub grants: Vec<Grant>,
    pub created_at: String,
    pub expires_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
    pub revoked_at: Option<String>,
}

/// What a live token may reach.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenAccess {
    pub token_id: Uuid,
    pub grants: Vec<Grant>,
}

fn stream(id: Uuid) -> String {
    format!("{API_TOKEN_STREAM}{id}")
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("stored timestamps are valid")
}

/// Creates a token. Returns its id and the plaintext, which is never stored.
pub async fn create_api_token(
    pool: &SqlitePool,
    owner_id: Uuid,
    name: &str,
    expires_at: Timestamp,
    grants: Vec<Grant>,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (owner, _) = load_user(&mut tx, owner_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let id = Uuid::new_v4();
    let secret = format!("{API_TOKEN_PREFIX}{}", token::new_token());
    let cmd = NewApiToken {
        token_id: id,
        name: name.to_owned(),
        expires_at,
        grants,
    };
    let event = domain::create_api_token(&owner, cmd, token::hash_token(&secret), now)?;
    commit(&mut tx, &stream(id), 0, &[event], Some(owner_id)).await?;
    tx.commit().await?;
    Ok((id, secret))
}

pub async fn revoke_api_token(pool: &SqlitePool, actor_id: Uuid, token_id: Uuid) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (actor, _) = load_user(&mut tx, actor_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let (events, version) = load_stream::<ApiTokenEvent>(&mut tx, &stream(token_id)).await?;
    let token = ApiToken::from_events(&events).ok_or(Error::ApiTokenNotFound)?;
    let events = domain::revoke_api_token(&token, &actor)?;
    commit(&mut tx, &stream(token_id), version, &events, Some(actor_id)).await?;
    tx.commit().await?;
    Ok(())
}

/// A user's tokens, newest first, revoked and expired ones too.
pub async fn list_api_tokens(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<ApiTokenSummary>> {
    type Row = (String, String, String, String, i64, Option<String>, Option<i64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.token_id, t.name, t.grants, t.created_at, t.expires_at, t.revoked_at,
                u.last_used_at
         FROM api_tokens t LEFT JOIN api_token_usage u ON u.token_id = t.token_id
         WHERE t.user_id = ? ORDER BY t.created_at DESC, t.rowid DESC",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(
            |(id, name, grants, created_at, expires_at, revoked_at, last_used_at)| {
                Ok(ApiTokenSummary {
                    id: id.parse().expect("token_id is a uuid"),
                    name,
                    grants: serde_json::from_str(&grants)?,
                    created_at,
                    expires_at: timestamp(expires_at),
                    last_used_at: last_used_at.map(timestamp),
                    revoked_at,
                })
            },
        )
        .collect()
}

/// The owner of a live token and what it may reach; `None` for an unknown,
/// expired or revoked one.
pub async fn token_user(
    pool: &SqlitePool,
    secret: &str,
    now: Timestamp,
) -> Result<Option<(User, TokenAccess)>> {
    if !secret.starts_with(API_TOKEN_PREFIX) {
        return Ok(None);
    }
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT token_id, user_id, grants FROM api_tokens
         WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(token::hash_token(secret))
    .bind(now.as_second())
    .fetch_optional(pool)
    .await?;
    let Some((token_id, user_id, grants)) = row else {
        return Ok(None);
    };
    let Some(user) = get_user(pool, user_id.parse().expect("user_id is a uuid")).await? else {
        return Ok(None);
    };
    let access = TokenAccess {
        token_id: token_id.parse().expect("token_id is a uuid"),
        grants: serde_json::from_str(&grants)?,
    };
    Ok(Some((user, access)))
}

/// Notes that a token was used. Reads first, so most calls write nothing.
pub async fn touch_api_token(pool: &SqlitePool, token_id: Uuid, now: Timestamp) -> Result<()> {
    let last: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_token_usage WHERE token_id = ?")
            .bind(token_id.to_string())
            .fetch_optional(pool)
            .await?;
    if last.is_some_and(|at| now.as_second() - at < USAGE_INTERVAL) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO api_token_usage (token_id, last_used_at) VALUES (?, ?)
         ON CONFLICT (token_id) DO UPDATE SET last_used_at = excluded.last_used_at",
    )
    .bind(token_id.to_string())
    .bind(now.as_second())
    .execute(pool)
    .await?;
    Ok(())
}
```

`ORDER BY t.created_at DESC, t.rowid DESC`: två tokens som skapas inom samma sekund (testet) sorteras på insättningsordning.

`crates/identity/src/projections.rs`: i `apply`, före `else { Ok(()) }`:

```rust
    } else if let Some(token_id) = event.stream_id.strip_prefix(crate::API_TOKEN_STREAM) {
        apply_api_token(conn, token_id, event).await
```

och funktionen:

```rust
async fn apply_api_token(
    conn: &mut SqliteConnection,
    token_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode::<ApiTokenEvent>()? {
        ApiTokenEvent::ApiTokenCreated {
            user_id,
            name,
            token_hash,
            expires_at,
            grants,
            ..
        } => {
            sqlx::query(
                "INSERT INTO api_tokens
                     (token_id, user_id, name, token_hash, grants, created_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(token_id)
            .bind(user_id.to_string())
            .bind(name)
            .bind(token_hash)
            .bind(serde_json::to_string(&grants)?)
            .bind(&event.recorded_at)
            .bind(expires_at.as_second())
            .execute(&mut *conn)
            .await?;
        }
        ApiTokenEvent::ApiTokenRevoked { .. } => {
            sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE token_id = ?")
                .bind(&event.recorded_at)
                .bind(token_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}
```

Importera `ApiTokenEvent` i `use crate::domain::{…}`, och lägg till `"DELETE FROM api_tokens",` först i listan i `rebuild_projections` (inte `api_token_usage`: den är operativ).

`crates/server/src/grpc.rs` matchar `Error` fullständigt i `status`, så lägg till armen där (före `Error::Domain(err) =>`), annars kompilerar inte servern:

```rust
        Error::Domain(DomainError::NotTokenOwner) | Error::ApiTokenNotFound => {
            Status::not_found("api_token_not_found")
        }
```

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity && cargo test -p doris-eventstore --test migrations && cargo build -p doris-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/0015_api_tokens.sql crates/identity crates/server/src/grpc.rs
git commit -m "Store API tokens as events with a hashed lookup projection"
```

---

### Task 4: AuthService: skapa, lista och återkalla tokens

**Files:**
- Modify: `proto/doris/auth/v1/auth.proto`
- Modify: `crates/server/src/grpc.rs` (tre handlers, `token_expiry`, felmappning, enhetstest)
- Modify: `crates/server/src/company.rs` (`fn status` → `pub(crate) fn status`)
- Modify: `crates/server/tests/common/mod.rs` (`bearer`, `company`, `api_token`)
- Test: `crates/server/tests/api_tokens.rs` (ny)

**Interfaces:**
- Consumes: Task 3:s `doris_identity::{create_api_token, list_api_tokens, revoke_api_token}`, `domain::{Grant, Scope}`.
- Produces: RPC:erna `CreateApiToken`, `ListApiTokens`, `RevokeApiToken` (proto nedan); testhjälparna `common::bearer(message, secret) -> Request<T>`, `common::company(server, session, org_nr) -> String` och `common::api_token(server, session, &[(&str, &[&str])]) -> String` (klartexten).

- [ ] **Step 1: Lägg till proto**

I `service AuthService`, efter `ListInvitations`:

```proto
  // Require a session (never a token). A token is shown once, at creation.
  rpc CreateApiToken(CreateApiTokenRequest) returns (CreateApiTokenResponse);
  rpc ListApiTokens(ListApiTokensRequest) returns (ListApiTokensResponse);
  // The owner, or an admin for anyone's token.
  rpc RevokeApiToken(RevokeApiTokenRequest) returns (RevokeApiTokenResponse);
```

Meddelanden i slutet av filen:

```proto
// Scopes are "ledger:read", "ledger:write", "invoicing:read",
// "invoicing:write", "payroll:read", "payroll:write", "vat:read",
// "vat:write" and "company:read".
message TokenGrant {
  string company_id = 1;
  repeated string scopes = 2;
}

message CreateApiTokenRequest {
  string name = 1;
  string expires_on = 2; // YYYY-MM-DD: the last day the token works (Swedish time)
  repeated TokenGrant grants = 3;
}

message CreateApiTokenResponse {
  string token_id = 1;
  string secret = 2;
}

message ApiToken {
  string id = 1;
  string name = 2;
  repeated TokenGrant grants = 3;
  string created_at = 4;
  string expires_at = 5; // RFC 3339: the moment it stops working
  optional string last_used_at = 6;
  optional string revoked_at = 7;
}

message ListApiTokensRequest {}

message ListApiTokensResponse {
  repeated ApiToken tokens = 1;
}

message RevokeApiTokenRequest {
  string token_id = 1;
}

message RevokeApiTokenResponse {}
```

- [ ] **Step 2: Skriv de fallerande testerna**

I `crates/server/tests/common/mod.rs`, efter `authed`:

```rust
/// A request carrying an API token, as doris-cli sends it.
pub fn bearer<T>(message: T, secret: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {secret}").parse().unwrap(),
    );
    request
}

/// A company of `session`'s with the räkenskapsår 2026.
pub async fn company(server: &TestServer, session: &str, org_nr: &str) -> String {
    use doris_proto::company::v1 as cpb;
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: org_nr.into(),
                name: format!("Bolag {org_nr}"),
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

/// The day `days` from today in Sweden, as `CreateApiToken` takes it.
pub fn in_days(days: i64) -> String {
    use jiff::ToSpan;
    let sweden = jiff::tz::TimeZone::get("Europe/Stockholm").unwrap();
    let today = jiff::Timestamp::now().to_zoned(sweden).date();
    today.checked_add(days.days()).unwrap().to_string()
}

/// A 30-day token of `session`'s with these scopes per company; returns
/// its secret.
pub async fn api_token(server: &TestServer, session: &str, grants: &[(&str, &[&str])]) -> String {
    server
        .grpc()
        .create_api_token(authed(
            pb::CreateApiTokenRequest {
                name: "Agent".into(),
                expires_on: in_days(30),
                grants: grants
                    .iter()
                    .map(|(company, scopes)| pb::TokenGrant {
                        company_id: company.to_string(),
                        scopes: scopes.iter().map(|s| s.to_string()).collect(),
                    })
                    .collect(),
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .secret
}
```

`crates/server/tests/api_tokens.rs`:

```rust
mod common;

use common::{TestServer, api_token, authed, company, device, in_days};
use doris_proto::auth::v1 as pb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

fn create(name: &str, expires_on: String, grants: Vec<pb::TokenGrant>) -> pb::CreateApiTokenRequest {
    pb::CreateApiTokenRequest {
        name: name.into(),
        expires_on,
        grants,
    }
}

fn grant(company: &str, scopes: &[&str]) -> pb::TokenGrant {
    pb::TokenGrant {
        company_id: company.into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn a_user_creates_lists_and_revokes_a_token() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let created = api
        .create_api_token(authed(
            create("Agent", in_days(90), vec![grant(&id, &["ledger:write", "ledger:read"])]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let listed = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;

    assert!(created.secret.starts_with("doris_"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.token_id);
    assert_eq!(listed[0].name, "Agent");
    assert_eq!(listed[0].grants, vec![grant(&id, &["ledger:read", "ledger:write"])]);
    assert_eq!(listed[0].revoked_at, None);

    api.revoke_api_token(authed(
        pb::RevokeApiTokenRequest {
            token_id: created.token_id.clone(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let listed = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert!(listed[0].revoked_at.is_some());
}

#[tokio::test]
async fn revoking_twice_is_fine() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();
    let created = api
        .create_api_token(authed(create("Agent", in_days(1), vec![grant(&id, &["ledger:read"])]), &anna))
        .await
        .unwrap()
        .into_inner();
    let revoke = || pb::RevokeApiTokenRequest {
        token_id: created.token_id.clone(),
    };

    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
}

#[tokio::test]
async fn a_token_without_grants_is_refused() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;

    let err = server
        .grpc()
        .create_api_token(authed(create("Agent", in_days(30), vec![]), &anna))
        .await
        .unwrap_err();

    assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_token_grants".into()));
}

#[tokio::test]
async fn a_token_is_only_for_the_users_own_companies_and_known_scopes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let annas = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let not_member = api
        .create_api_token(authed(create("Agent", in_days(30), vec![grant(&annas, &["ledger:read"])]), &bo))
        .await
        .unwrap_err();
    let unknown_scope = api
        .create_api_token(authed(create("Agent", in_days(30), vec![grant(&annas, &["ledger:admin"])]), &anna))
        .await
        .unwrap_err();

    assert_eq!(code_of(not_member), (Code::NotFound, "company_not_found".into()));
    assert_eq!(code_of(unknown_scope), (Code::InvalidArgument, "invalid_token_grants".into()));
}

#[tokio::test]
async fn the_last_day_is_today_at_the_earliest_and_a_year_off_at_most() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();
    let try_day = |day: String| create("Agent", day, vec![grant(&id, &["ledger:read"])]);

    for day in [in_days(-1), in_days(367), "i morgon".to_owned()] {
        let err = api.create_api_token(authed(try_day(day.clone()), &anna)).await.unwrap_err();
        assert_eq!(code_of(err), (Code::InvalidArgument, "invalid_token_expiry".into()), "{day}");
    }
    api.create_api_token(authed(try_day(in_days(366)), &anna)).await.unwrap();
}

#[tokio::test]
async fn someone_elses_token_cannot_be_seen_or_revoked_but_an_admin_can_revoke_it() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let cecilia = server.invite(&anna, "cecilia@example.se").await;
    let bos_company = company(&server, &bo, "556016-0680").await;
    let mut api = server.grpc();
    let created = api
        .create_api_token(authed(create("Bo", in_days(30), vec![grant(&bos_company, &["ledger:read"])]), &bo))
        .await
        .unwrap()
        .into_inner();
    let revoke = || pb::RevokeApiTokenRequest {
        token_id: created.token_id.clone(),
    };

    let by_cecilia = api.revoke_api_token(authed(revoke(), &cecilia)).await.unwrap_err();
    let unknown = api
        .revoke_api_token(authed(pb::RevokeApiTokenRequest { token_id: "nej".into() }, &bo))
        .await
        .unwrap_err();
    let cecilias_list = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &cecilia))
        .await
        .unwrap()
        .into_inner()
        .tokens;

    assert_eq!(code_of(by_cecilia), (Code::NotFound, "api_token_not_found".into()));
    assert_eq!(code_of(unknown), (Code::NotFound, "api_token_not_found".into()));
    assert!(cecilias_list.is_empty());
    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
}
```

`api_token` och `bearer` används först i Task 6; `#![allow(dead_code)]` i `common` täcker det.

I `crates/server/src/grpc.rs`, i `mod tests`, lägg till:

```rust
    #[test]
    fn a_token_ends_at_midnight_in_sweden_after_its_last_day() {
        let at = |s: &str| -> jiff::Timestamp { s.parse().unwrap() };
        let now = at("2026-10-06T10:00:00Z");
        // The last day today: the rest of today, until midnight in Sweden (summer time).
        assert_eq!(token_expiry("2026-10-06", now).unwrap(), at("2026-10-06T22:00:00Z"));
        // In winter time, midnight is 23:00Z.
        assert_eq!(token_expiry("2026-12-01", now).unwrap(), at("2026-12-01T23:00:00Z"));
        // The day before summer time starts ends at 23:00Z too.
        assert_eq!(token_expiry("2027-03-27", now).unwrap(), at("2027-03-27T23:00:00Z"));
        // Today counts in Sweden: at 23:30Z on the 6th it is already the 7th.
        assert!(token_expiry("2026-10-06", at("2026-10-06T23:30:00Z")).is_err());
        assert!(token_expiry("2027-10-07", now).is_ok());
        assert!(token_expiry("2027-10-08", now).is_err());
        assert!(token_expiry("2026-10-05", now).is_err());
        assert!(token_expiry("i morgon", now).is_err());
    }
```

(2026-10-06 + 366 dagar = 2027-10-07.)

- [ ] **Step 3: Kör testerna och se dem fallera**

Run: `cargo test -p doris-server --test api_tokens; cargo test -p doris-server --lib token_ends`
Expected: kompileringsfel, `create_api_token` saknas i `AuthService`-impl och `token_expiry` finns inte.

- [ ] **Step 4: Implementera**

`crates/server/src/company.rs`: ändra `fn status(err: Error) -> Status` till `pub(crate) fn status(…)`.

`crates/server/src/grpc.rs`, importer: `use doris_identity::domain::{DomainError, Grant, Role, Scope, User};`.

(Felmappningen för `api_token_not_found` lades till i Task 3.)

Handlers i `impl AuthService for AuthApi`:

```rust
    async fn create_api_token(
        &self,
        request: Request<pb::CreateApiTokenRequest>,
    ) -> Result<Response<pb::CreateApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let now = Timestamp::now();
        let expires_at = token_expiry(&req.expires_on, now)?;
        let mut grants = Vec::with_capacity(req.grants.len());
        for grant in &req.grants {
            let company_id: Uuid = grant
                .company_id
                .parse()
                .map_err(|_| Status::not_found("company_not_found"))?;
            // Only the user's own companies: membership has one source.
            doris_company::get_company(&self.pool, company_id, user.id)
                .await
                .map_err(crate::company::status)?;
            let scopes = grant
                .scopes
                .iter()
                .map(|s| Scope::parse(s))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| Status::invalid_argument("invalid_token_grants"))?;
            grants.push(Grant { company_id, scopes });
        }
        let (token_id, secret) = doris_identity::create_api_token(
            &self.pool, user.id, &req.name, expires_at, grants, now,
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CreateApiTokenResponse {
            token_id: token_id.to_string(),
            secret,
        }))
    }

    async fn list_api_tokens(
        &self,
        request: Request<pb::ListApiTokensRequest>,
    ) -> Result<Response<pb::ListApiTokensResponse>, Status> {
        let user = self.user(&request).await?;
        let tokens = doris_identity::list_api_tokens(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|t| pb::ApiToken {
                id: t.id.to_string(),
                name: t.name,
                grants: t.grants.iter().map(grant_message).collect(),
                created_at: t.created_at,
                expires_at: t.expires_at.to_string(),
                last_used_at: t.last_used_at.map(|at| at.to_string()),
                revoked_at: t.revoked_at,
            })
            .collect();
        Ok(Response::new(pb::ListApiTokensResponse { tokens }))
    }

    async fn revoke_api_token(
        &self,
        request: Request<pb::RevokeApiTokenRequest>,
    ) -> Result<Response<pb::RevokeApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let token_id: Uuid = request
            .get_ref()
            .token_id
            .parse()
            .map_err(|_| Status::not_found("api_token_not_found"))?;
        doris_identity::revoke_api_token(&self.pool, user.id, token_id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::RevokeApiTokenResponse {}))
    }
```

Fria funktioner (efter `user_message`):

```rust
fn grant_message(grant: &Grant) -> pb::TokenGrant {
    pb::TokenGrant {
        company_id: grant.company_id.to_string(),
        scopes: grant.scopes.iter().map(|s| s.as_str().to_owned()).collect(),
    }
}

/// When a token whose last day is `raw` (`YYYY-MM-DD`) stops working:
/// midnight in Sweden after that day. The day is today at the earliest and
/// 366 days off at most.
fn token_expiry(raw: &str, now: Timestamp) -> Result<Timestamp, Status> {
    use jiff::ToSpan;
    let invalid = || Status::invalid_argument("invalid_token_expiry");
    let last_day: Date = raw.parse().map_err(|_| invalid())?;
    let today = today_in_sweden(now);
    let latest = today.checked_add(366.days()).map_err(|_| invalid())?;
    if last_day < today || last_day > latest {
        return Err(invalid());
    }
    let sweden = TimeZone::get("Europe/Stockholm").expect("bundled tz database");
    let midnight = last_day.tomorrow().map_err(|_| invalid())?;
    Ok(midnight.to_zoned(sweden).map_err(|_| invalid())?.timestamp())
}
```

- [ ] **Step 5: Kör testerna**

Run: `cargo test -p doris-server`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add proto/doris/auth/v1/auth.proto crates/server
git commit -m "Create, list and revoke API tokens over AuthService"
```

---

### Task 5: Tabellen över vad en token får anropa

**Files:**
- Create: `crates/server/src/access.rs`
- Modify: `crates/server/src/lib.rs` (`mod access;`)

**Interfaces:**
- Consumes: `doris_identity::domain::Scope`.
- Produces: `pub(crate) enum Access { Company(Scope), Owner, SessionOnly }` (`Debug, Clone, Copy, PartialEq, Eq`) och `pub(crate) fn access(path: &str) -> Access`, där `path` är `/<package>.<Service>/<Method>`.

- [ ] **Step 1: Skriv de fallerande testerna**

`crates/server/src/access.rs` (testerna först; `access` och `classify` som `todo!()` tills Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PROTOS: [&str; 6] = [
        include_str!("../../../proto/doris/auth/v1/auth.proto"),
        include_str!("../../../proto/doris/company/v1/company.proto"),
        include_str!("../../../proto/doris/invoicing/v1/invoicing.proto"),
        include_str!("../../../proto/doris/ledger/v1/ledger.proto"),
        include_str!("../../../proto/doris/payroll/v1/payroll.proto"),
        include_str!("../../../proto/doris/vat/v1/vat.proto"),
    ];

    /// `(package.Service, Method)` for every rpc in a .proto file.
    fn methods(proto: &str) -> Vec<(String, String)> {
        let line = |start: &str| {
            proto
                .lines()
                .map(str::trim)
                .find_map(|l| l.strip_prefix(start))
                .map(|rest| rest.trim_end_matches([';', '{', ' ']).trim().to_owned())
                .unwrap()
        };
        let service = format!("{}.{}", line("package "), line("service "));
        proto
            .lines()
            .filter_map(|l| l.trim().strip_prefix("rpc "))
            .map(|rest| (service.clone(), rest.split('(').next().unwrap().trim().to_owned()))
            .collect()
    }

    #[test]
    fn every_rpc_is_in_the_table() {
        let all: Vec<_> = PROTOS.iter().flat_map(|p| methods(p)).collect();
        assert!(all.len() > 80, "the protos were not read: {}", all.len());
        let missing: Vec<_> = all
            .iter()
            .filter(|(service, method)| classify(service, method).is_none())
            .collect();
        assert_eq!(missing, Vec::<&(String, String)>::new(), "add these to `classify`");
    }

    #[test]
    fn the_table_follows_the_spec() {
        use Access::*;
        use Scope::*;
        for (path, expected) in [
            ("/doris.ledger.v1.LedgerService/RecordVoucher", Company(LedgerWrite)),
            ("/doris.ledger.v1.LedgerService/ListVouchers", Company(LedgerRead)),
            ("/doris.payroll.v1.PayrollService/ExportAgiFile", Company(PayrollRead)),
            ("/doris.payroll.v1.PayrollService/BookPayrollRun", Company(PayrollWrite)),
            ("/doris.invoicing.v1.InvoicingService/PaySupplierInvoice", Company(InvoicingWrite)),
            ("/doris.vat.v1.VatService/MarkVatReturnSubmitted", Company(VatWrite)),
            ("/doris.company.v1.CompanyService/GetCompany", Company(CompanyRead)),
            ("/doris.company.v1.CompanyService/ListCompanies", Owner),
            ("/doris.auth.v1.AuthService/GetStatus", Owner),
            ("/doris.auth.v1.AuthService/CreateApiToken", SessionOnly),
            ("/doris.company.v1.CompanyService/AddMember", SessionOnly),
            ("/doris.ledger.v1.LedgerService/SomethingNew", SessionOnly),
            ("/index.html", SessionOnly),
        ] {
            assert_eq!(access(path), expected, "{path}");
        }
    }
}
```

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-server --lib access`
Expected: FAIL (`not yet implemented`, eller kompileringsfel innan `mod access;` finns: lägg till `mod access;` i `lib.rs` först).

- [ ] **Step 3: Implementera**

Överst i `crates/server/src/access.rs`:

```rust
//! What a call with an API token may do, per gRPC method. Sessions are not
//! limited by this table. A method missing from it is closed to tokens, and
//! a test keeps every rpc in `proto/` in it.

use doris_identity::domain::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Concerns one company and needs this scope for it.
    Company(Scope),
    /// Any token may call it: it answers about the token's owner.
    Owner,
    /// Sessions only.
    SessionOnly,
}

/// The access a gRPC path (`/package.Service/Method`) needs from a token.
pub(crate) fn access(path: &str) -> Access {
    path.strip_prefix('/')
        .and_then(|p| p.split_once('/'))
        .and_then(|(service, method)| classify(service, method))
        .unwrap_or(Access::SessionOnly)
}

fn classify(service: &str, method: &str) -> Option<Access> {
    use Access::*;
    use Scope::*;
    Some(match (service, method) {
        ("doris.auth.v1.AuthService", "GetStatus") => Owner,
        (
            "doris.auth.v1.AuthService",
            "BeginRegistration" | "FinishRegistration" | "BeginLogin" | "FinishLogin" | "Logout"
            | "BeginAddPasskey" | "FinishAddPasskey" | "ListPasskeys" | "GetInvitation"
            | "CreateInvitation" | "ListInvitations" | "CreateApiToken" | "ListApiTokens"
            | "RevokeApiToken",
        ) => SessionOnly,

        ("doris.company.v1.CompanyService", "ListCompanies") => Owner,
        ("doris.company.v1.CompanyService", "GetCompany" | "ListMembers") => Company(CompanyRead),
        (
            "doris.company.v1.CompanyService",
            "GetLookupStatus" | "LookupCompany" | "CreateCompany" | "AddMember",
        ) => SessionOnly,

        (
            "doris.ledger.v1.LedgerService",
            "ListAccounts" | "ListFiscalYears" | "ListVouchers" | "GetAttachment"
            | "GetTrialBalance" | "GetAccountLedger" | "GetFinancialStatements"
            | "GetOpeningBalances",
        ) => Company(LedgerRead),
        (
            "doris.ledger.v1.LedgerService",
            "AddAccount" | "RenameAccount" | "SetAccountActive" | "SetAccountVatBox"
            | "RecordVoucher" | "CorrectVoucher" | "AddAttachment" | "SetOpeningBalances"
            | "CloseFiscalYear" | "ReopenFiscalYear",
        ) => Company(LedgerWrite),

        (
            "doris.invoicing.v1.InvoicingService",
            "ListCustomers" | "ListSuppliers" | "ListSupplierInvoices"
            | "GetSupplierInvoiceAttachment" | "ListCustomerInvoices"
            | "GetCustomerInvoiceAttachment",
        ) => Company(InvoicingRead),
        (
            "doris.invoicing.v1.InvoicingService",
            "AddCustomer" | "UpdateCustomer" | "SetCustomerActive" | "AddSupplier"
            | "UpdateSupplier" | "SetSupplierActive" | "RegisterSupplierInvoice"
            | "PaySupplierInvoice" | "CancelSupplierInvoice" | "ReverseSupplierInvoicePayment"
            | "RegisterCustomerInvoice" | "PayCustomerInvoice" | "CancelCustomerInvoice"
            | "ReverseCustomerInvoicePayment",
        ) => Company(InvoicingWrite),

        (
            "doris.payroll.v1.PayrollService",
            "ListEmployees" | "PreviewPayrollRun" | "GetPayrollRun" | "ListPayrollRuns"
            | "GetAgiContact" | "ListAgiMonths" | "GetAgiMonth" | "ExportAgiFile",
        ) => Company(PayrollRead),
        (
            "doris.payroll.v1.PayrollService",
            "AddEmployee" | "UpdateEmployee" | "DeactivateEmployee" | "SetEmployeeTax"
            | "CreatePayrollRun" | "UpdatePayrollRun" | "FinalizePayrollRun"
            | "ReopenPayrollRun" | "BookPayrollRun" | "UnbookPayrollRun" | "SetAgiContact"
            | "MarkAgiSubmitted",
        ) => Company(PayrollWrite),

        ("doris.vat.v1.VatService", "ListVatReturns" | "GetVatReturn" | "ExportVatFile") => {
            Company(VatRead)
        }
        ("doris.vat.v1.VatService", "SetVatPeriod" | "MarkVatReturnSubmitted") => Company(VatWrite),

        _ => return None,
    })
}
```

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-server --lib access`
Expected: PASS. (`access` används inte än utanför testerna; om clippy klagar på `dead_code` före Task 6, sätt `#[cfg_attr(not(test), allow(dead_code))]` på `access` och ta bort det i Task 6.)

- [ ] **Step 5: Commit**

```bash
git add crates/server/src/access.rs crates/server/src/lib.rs
git commit -m "Table what an API token may call, kept in step with the protos"
```

---

### Task 6: Anrop med token

**Files:**
- Modify: `crates/server/src/lib.rs` (`session_gate` → `auth_gate`)
- Modify: `crates/server/src/grpc.rs` (`TokenCaller`, `bearer`, `signed_in_user`, `company_caller`, `get_status`, `not_signed_in` → `pub(crate)`)
- Modify: `crates/server/src/ledger.rs:36-46`, `invoicing.rs:32-42`, `payroll.rs:60-70`, `vat.rs:28-38`, `company.rs:28-40` (`caller`/`member_company`), `company.rs` `list_companies`
- Test: `crates/server/tests/api_tokens.rs`

**Interfaces:**
- Consumes: Task 3:s `token_user`, `touch_api_token`, `TokenAccess`; Task 5:s `access`; Task 1:s `VIA_TOKEN`; Task 4:s testhjälpare.
- Produces: `pub(crate) struct TokenCaller { pub user: User, pub access: TokenAccess, pub required: Access }` i extensions; `pub(crate) async fn company_caller<T>(pool: &SqlitePool, request: &Request<T>, company_id: &str) -> Result<(Uuid, Uuid), Status>` som returnerar `(company, user)`.

- [ ] **Step 1: Skriv de fallerande testerna**

Lägg till i `crates/server/tests/api_tokens.rs` (utöka importerna med `bearer`, och `use doris_proto::company::v1 as cpb; use doris_proto::ledger::v1 as lpb; use doris_proto::payroll::v1 as ppb; use doris_proto::invoicing::v1 as ipb; use doris_proto::vat::v1 as vpb;`):

```rust
fn sale(company: &str) -> lpb::RecordVoucherRequest {
    lpb::RecordVoucherRequest {
        company_id: company.into(),
        date: "2026-01-15".into(),
        text: "Försäljning".into(),
        lines: vec![
            lpb::VoucherLine { account: 1930, debit: 100, credit: 0 },
            lpb::VoucherLine { account: 3001, debit: 0, credit: 100 },
        ],
        attachments: vec![],
    }
}

fn vouchers(company: &str) -> lpb::ListVouchersRequest {
    lpb::ListVouchersRequest {
        company_id: company.into(),
        fiscal_year_start: "2026-01-01".into(),
    }
}

#[tokio::test]
async fn a_read_token_lists_but_does_not_book_and_a_write_token_books_with_its_id_recorded() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let reader = api_token(&server, &anna, &[(&id, &["ledger:read"])]).await;
    let writer = api_token(&server, &anna, &[(&id, &["ledger:read", "ledger:write"])]).await;
    let mut ledger = server.ledger();

    ledger.list_vouchers(bearer(vouchers(&id), &reader)).await.unwrap();
    let refused = ledger.record_voucher(bearer(sale(&id), &reader)).await.unwrap_err();
    ledger.record_voucher(bearer(sale(&id), &writer)).await.unwrap();

    assert_eq!(code_of(refused), (Code::PermissionDenied, "missing_scope".into()));
    let token_id = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens
        .into_iter()
        .find(|t| t.grants[0].scopes.len() == 2)
        .unwrap()
        .id;
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM events WHERE event_type = 'VoucherRecorded'
         ORDER BY global_position DESC LIMIT 1",
    )
    .fetch_one(&server.pool)
    .await
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["via_token"], token_id.as_str());
}

#[tokio::test]
async fn scopes_are_per_company() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let a = company(&server, &anna, "556016-0680").await;
    let b = company(&server, &anna, "556036-0793").await;
    let token = api_token(&server, &anna, &[(&a, &["ledger:write"]), (&b, &["ledger:read"])]).await;
    let mut ledger = server.ledger();

    ledger.record_voucher(bearer(sale(&a), &token)).await.unwrap();
    let in_b = ledger.record_voucher(bearer(sale(&b), &token)).await.unwrap_err();

    assert_eq!(code_of(in_b), (Code::PermissionDenied, "missing_scope".into()));
}

#[tokio::test]
async fn a_company_outside_the_grants_does_not_exist_for_the_token() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let granted = company(&server, &anna, "556016-0680").await;
    let other = company(&server, &anna, "556036-0793").await;
    let token = api_token(&server, &anna, &[(&granted, &["ledger:read"])]).await;

    let err = server.ledger().list_vouchers(bearer(vouchers(&other), &token)).await.unwrap_err();
    let listed = server
        .companies()
        .list_companies(bearer(cpb::ListCompaniesRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .companies;

    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    assert_eq!(listed.into_iter().map(|c| c.id).collect::<Vec<_>>(), [granted]);
}

#[tokio::test]
async fn expired_revoked_and_malformed_tokens_are_not_signed_in_even_with_a_cookie() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let me = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .current_user
        .unwrap()
        .id;
    let then = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(48);
    let (_, expired) = doris_identity::create_api_token(
        &server.pool,
        me.parse().unwrap(),
        "Gammal",
        then + jiff::SignedDuration::from_hours(24),
        vec![doris_identity::domain::Grant {
            company_id: id.parse().unwrap(),
            scopes: vec![doris_identity::domain::Scope::LedgerRead],
        }],
        then,
    )
    .await
    .unwrap();
    let revoked = api_token(&server, &anna, &[(&id, &["ledger:read"])]).await;
    let revoked_id = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens[0]
        .id
        .clone();
    server
        .grpc()
        .revoke_api_token(authed(pb::RevokeApiTokenRequest { token_id: revoked_id }, &anna))
        .await
        .unwrap();

    for secret in [expired.as_str(), revoked.as_str(), "doris_nope", "nope"] {
        let mut request = bearer(vouchers(&id), secret);
        request.metadata_mut().insert(
            "cookie",
            format!("{}={anna}", doris_server::SESSION_COOKIE).parse().unwrap(),
        );
        let err = server.ledger().list_vouchers(request).await.unwrap_err();
        assert_eq!(code_of(err), (Code::Unauthenticated, "not_signed_in".into()), "{secret}");
    }
}

#[tokio::test]
async fn a_token_cannot_manage_tokens_invite_or_create_companies_but_knows_its_owner() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &[(&id, &["ledger:read", "company:read"])]).await;
    let mut auth = server.grpc();

    let errors = [
        auth.create_api_token(bearer(create("Ny", in_days(1), vec![grant(&id, &["ledger:read"])]), &token))
            .await
            .unwrap_err(),
        auth.create_invitation(bearer(pb::CreateInvitationRequest { email: "bo@example.se".into() }, &token))
            .await
            .unwrap_err(),
        server
            .companies()
            .create_company(bearer(
                cpb::CreateCompanyRequest {
                    org_nr: "556036-0793".into(),
                    name: "Nytt AB".into(),
                    legal_form: cpb::LegalForm::Aktiebolag as i32,
                    address: None,
                    fiscal_year_start: "2026-01-01".into(),
                    fiscal_year_end: "2026-12-31".into(),
                    accounting_method: cpb::AccountingMethod::Invoice as i32,
                },
                &token,
            ))
            .await
            .unwrap_err(),
    ];
    let status = auth.get_status(bearer(pb::GetStatusRequest {}, &token)).await.unwrap().into_inner();
    let company = server
        .companies()
        .get_company(bearer(cpb::GetCompanyRequest { company_id: id.clone() }, &token))
        .await
        .unwrap()
        .into_inner();

    for err in errors {
        assert_eq!(code_of(err), (Code::PermissionDenied, "token_not_allowed".into()));
    }
    assert_eq!(status.current_user.unwrap().email, "anna@example.se");
    assert_eq!(company.id, id);
}

#[tokio::test]
async fn the_bearer_scheme_is_case_insensitive_and_spaces_are_ignored() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &[(&id, &["ledger:read"])]).await;

    for header in [format!("bearer {token}"), format!("BEARER  {token} ")] {
        let mut request = tonic::Request::new(vouchers(&id));
        request.metadata_mut().insert("authorization", header.parse().unwrap());
        server.ledger().list_vouchers(request).await.unwrap();
    }
}

#[tokio::test]
async fn each_service_checks_the_area() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let ledger_only = api_token(&server, &anna, &[(&id, &["ledger:read"])]).await;
    let all = api_token(&server, &anna, &[(&id, &["payroll:read", "invoicing:read", "vat:read"])]).await;
    let employees = || ppb::ListEmployeesRequest { company_id: id.clone() };
    let customers = || ipb::ListCustomersRequest { company_id: id.clone() };
    let returns = || vpb::ListVatReturnsRequest {
        company_id: id.clone(),
        fiscal_year_start: "2026-01-01".into(),
    };

    let refused = [
        server.payroll().list_employees(bearer(employees(), &ledger_only)).await.unwrap_err(),
        server.invoicing().list_customers(bearer(customers(), &ledger_only)).await.unwrap_err(),
        server.vat().list_vat_returns(bearer(returns(), &ledger_only)).await.unwrap_err(),
    ];
    for err in refused {
        assert_eq!(code_of(err), (Code::PermissionDenied, "missing_scope".into()));
    }
    server.payroll().list_employees(bearer(employees(), &all)).await.unwrap();
    server.invoicing().list_customers(bearer(customers(), &all)).await.unwrap();
    server.vat().list_vat_returns(bearer(returns(), &all)).await.unwrap();
}

#[tokio::test]
async fn a_token_sends_large_underlag_through_the_gate() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &[(&id, &["ledger:write"])]).await;
    let pdf = |size: usize| {
        let mut data = b"%PDF-1.7\n".to_vec();
        data.resize(size, b'x');
        data
    };
    let mut request = sale(&id);
    request.attachments = vec![
        lpb::NewAttachment { file_name: "a.pdf".into(), data: pdf(10 << 20) },
        lpb::NewAttachment { file_name: "b.pdf".into(), data: pdf((10 << 20) - 1) },
    ];

    server.ledger().record_voucher(bearer(request, &token)).await.unwrap();
}
```

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-server --test api_tokens`
Expected: de nya testerna fallerar med `Unauthenticated: not_signed_in`, eftersom servern bara läser cookien.

- [ ] **Step 3: Implementera**

`crates/server/src/grpc.rs`:

```rust
/// A call made with an API token. `auth_gate` puts it in the request.
#[derive(Clone)]
pub(crate) struct TokenCaller {
    pub user: User,
    pub access: doris_identity::TokenAccess,
    pub required: crate::access::Access,
}

/// The token in an `authorization: Bearer …` header. The scheme is
/// case-insensitive; anything else in the header is not a token, but is
/// still returned, so that it fails as one rather than falling back to the
/// cookie.
pub(crate) fn bearer(headers: &http::HeaderMap) -> Option<String> {
    let value = headers.get(http::header::AUTHORIZATION)?;
    let value = value.to_str().unwrap_or_default().trim();
    let token = match value.split_once(' ') {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => rest.trim(),
        _ => value,
    };
    Some(token.to_owned())
}

/// The signed-in user (by session or API token), or `Unauthenticated`.
/// Shared by every service.
pub(crate) async fn signed_in_user<T>(
    pool: &SqlitePool,
    request: &Request<T>,
) -> Result<User, Status> {
    if let Some(token) = request.extensions().get::<TokenCaller>() {
        return Ok(token.user.clone());
    }
    session_user(pool, request.metadata().as_ref()).await
}

/// The company asked about and the caller. A token needs the call's scope
/// for that company; membership is then checked by each module.
pub(crate) async fn company_caller<T>(
    pool: &SqlitePool,
    request: &Request<T>,
    company_id: &str,
) -> Result<(Uuid, Uuid), Status> {
    let user = signed_in_user(pool, request).await?;
    let company: Uuid = company_id
        .parse()
        .map_err(|_| Status::not_found("company_not_found"))?;
    if let Some(token) = request.extensions().get::<TokenCaller>() {
        let grant = token
            .access
            .grants
            .iter()
            .find(|g| g.company_id == company)
            .ok_or_else(|| Status::not_found("company_not_found"))?;
        match token.required {
            crate::access::Access::Company(scope) if grant.scopes.contains(&scope) => {}
            _ => return Err(Status::permission_denied("missing_scope")),
        }
    }
    Ok((company, user.id))
}
```

Gör `fn not_signed_in()` till `pub(crate) fn not_signed_in()`.

I `get_status`, ersätt `let current_user = match session_token(&request) { … };` med:

```rust
        let current_user = match request.extensions().get::<TokenCaller>() {
            Some(token) => Some(token.user.clone()),
            None => match session_token(&request) {
                Some(token) => doris_identity::session_user(&self.pool, &token, Timestamp::now())
                    .await
                    .map_err(status)?,
                None => None,
            },
        };
```

I `ledger.rs`, `invoicing.rs`, `payroll.rs` och `vat.rs`, ersätt kroppen i `caller` med ett anrop, till exempel i `ledger.rs`:

```rust
    /// The company asked about and the caller. Membership is checked by
    /// `doris_ledger` itself; a token's scope by `company_caller`.
    async fn caller<T>(
        &self,
        request: &Request<T>,
        company_id: &str,
    ) -> Result<(Uuid, Uuid), Status> {
        crate::grpc::company_caller(&self.pool, request, company_id).await
    }
```

och ta bort `signed_in_user` ur `use crate::grpc::{…}` där det inte längre används (`ledger.rs`, `invoicing.rs`, `payroll.rs`, `vat.rs`). Ta också bort `company_not_found()` om den blir oanvänd.

I `company.rs`, `member_company`:

```rust
        let (id, user) = grpc::company_caller(&self.pool, request, company_id).await?;
        let company = doris_company::get_company(&self.pool, id, user)
            .await
            .map_err(status)?;
        Ok((company, user))
```

och i `list_companies`, efter `let user = signed_in_user(&self.pool, &request).await?;`:

```rust
        let token = request.extensions().get::<grpc::TokenCaller>().cloned();
        let companies = doris_company::list_companies(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            // A token sees only the companies it was given.
            .filter(|c| {
                token
                    .as_ref()
                    .is_none_or(|t| t.access.grants.iter().any(|g| g.company_id == c.id))
            })
            .map(|c| pb::CompanySummary { … oförändrat … })
            .collect();
```

`crates/server/src/lib.rs`: ersätt `session_gate` och dess dokumentation med:

```rust
/// Authenticates every gRPC call before its body is read. With an
/// `authorization: Bearer` header the call runs as the token's owner, if
/// `access` lets tokens make it, with the token recorded on every event it
/// appends. Without one, LedgerService and InvoicingService (bodies of up
/// to 21 MiB, which tonic reserves from the frame header before any handler
/// runs) need a valid session cookie here; handlers still check it.
async fn auth_gate(
    State(pool): State<SqlitePool>,
    mut request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    if let Some(secret) = grpc::bearer(request.headers()) {
        let now = jiff::Timestamp::now();
        let (user, access) = match doris_identity::token_user(&pool, &secret, now).await {
            Ok(Some(found)) => found,
            Ok(None) => return grpc::not_signed_in().into_http(),
            Err(err) => return grpc::status(err).into_http(),
        };
        let required = access::access(&path);
        if required == access::Access::SessionOnly {
            return tonic::Status::permission_denied("token_not_allowed").into_http();
        }
        let token_id = access.token_id;
        request.extensions_mut().insert(grpc::TokenCaller {
            user,
            access,
            required,
        });
        let response = doris_eventstore::VIA_TOKEN
            .scope(token_id.to_string(), next.run(request))
            .await;
        if let Err(err) = doris_identity::touch_api_token(&pool, token_id, now).await {
            tracing::warn!("api token usage: {err}");
        }
        return response;
    }
    let large = path.starts_with("/doris.ledger.v1.LedgerService/")
        || path.starts_with("/doris.invoicing.v1.InvoicingService/");
    if large && let Err(status) = grpc::session_user(&pool, request.headers()).await {
        return status.into_http();
    }
    next.run(request).await
}
```

och i `router`: `.layer(axum::middleware::from_fn_with_state(pool, auth_gate))`. `jiff` finns redan bland serverns beroenden. Om `Status::into_http()` ger en annan kroppstyp än axums `Response` i den här tonic-versionen, gör som den befintliga `session_gate` gjorde (den returnerade `status.into_http()` från samma signatur).

Ta bort `#[cfg_attr(not(test), allow(dead_code))]` från `access` om det lades till i Task 5.

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-server && cargo clippy --workspace -- -D warnings`
Expected: PASS och inga varningar.

- [ ] **Step 5: Commit**

```bash
git add crates/server
git commit -m "Let API tokens call the services their scopes allow"
```

---

### Task 7: Webben: sidan API-tokens

**Files:**
- Create: `crates/web/src/pages/api_tokens.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs` (två routes), `crates/web/src/nav.rs` (NavItem, test), `crates/web/src/ui.rs` (`IconName::SquareTerminal`), `crates/web/src/errors.rs`
- Modify: `e2e/tests/design.spec.ts` (`paths`), `e2e/tests/leaving.spec.ts` (`pages`), `e2e/tests/fixtures.ts` (`MENU_OF`)

**Interfaces:**
- Consumes: `pb::{ApiToken, TokenGrant, CreateApiTokenRequest, ListApiTokensRequest, RevokeApiTokenRequest}` via `crate::api::{api, pb}`; `Companies`-kontexten (`list: RwSignal<Vec<cpb::CompanySummary>>`); `format::{date, today, plus_days}`.
- Produces: komponenterna `ApiTokens` (`/settings/tokens`) och `NewApiToken` (`/settings/tokens/new`); rena funktioner `status_of(&pb::ApiToken, now: &str) -> TokenStatus` och `grant(company_id: &str, boxes: &[(bool, bool)]) -> Option<pb::TokenGrant>`.

- [ ] **Step 1: Skriv de fallerande testerna**

Längst ned i den nya `crates/web/src/pages/api_tokens.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn token(expires_at: &str, revoked_at: Option<&str>) -> pb::ApiToken {
        pb::ApiToken {
            expires_at: expires_at.into(),
            revoked_at: revoked_at.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn a_token_is_active_until_it_expires_or_is_revoked() {
        let now = "2026-10-06T10:00:00.000Z";
        assert_eq!(status_of(&token("2026-10-06T22:00:00Z", None), now), TokenStatus::Active);
        assert_eq!(status_of(&token("2026-10-06T09:00:00Z", None), now), TokenStatus::Expired);
        assert_eq!(
            status_of(&token("2026-10-07T22:00:00Z", Some("2026-10-05T08:00:00Z")), now),
            TokenStatus::Revoked
        );
    }

    #[test]
    fn writing_brings_reading_and_empty_companies_are_left_out() {
        // Bokföring skriva, Lön läsa, Bolag läsa (its write box does nothing).
        let boxes = [(false, true), (false, false), (true, false), (false, false), (true, true)];
        assert_eq!(
            grant("c1", &boxes),
            Some(pb::TokenGrant {
                company_id: "c1".into(),
                scopes: ["ledger:read", "ledger:write", "payroll:read", "company:read"]
                    .map(String::from)
                    .to_vec(),
            })
        );
        assert_eq!(grant("c2", &[(false, false); 5]), None);
    }
}
```

I `crates/web/src/errors.rs`, i `mod tests`:

```rust
    #[test]
    fn api_token_codes_have_swedish_messages() {
        for code in [
            "invalid_token_name",
            "invalid_token_expiry",
            "invalid_token_grants",
            "api_token_not_found",
            "missing_scope",
            "token_not_allowed",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
    }
```

I `crates/web/src/nav.rs`, i testet `a_path_belongs_to_the_menu_that_lists_it`, lägg till `("/settings/tokens", None),` och `("/settings/tokens/new", None),`.

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-web`
Expected: kompileringsfel (`api_tokens` saknas), och när det byggs: `api_token_codes_have_swedish_messages` fallerar.

- [ ] **Step 3: Implementera**

`crates/web/src/errors.rs`, i `message`:

```rust
        "invalid_token_name" => "Namnet måste vara 1–100 tecken.",
        "invalid_token_expiry" => "Välj en sista giltig dag från i dag och högst ett år fram.",
        "invalid_token_grants" => "Ge token minst en behörighet.",
        "api_token_not_found" => "Token finns inte.",
        "missing_scope" => "Token saknar behörighet för det här.",
        "token_not_allowed" => "Det här kan inte göras med en token.",
```

`crates/web/src/ui.rs`: lägg till `SquareTerminal` sist i `IconName`, i `ALL` (`[IconName; 23]`) och i `shapes`:

```rust
            IconName::SquareTerminal => {
                r#"<path d="m7 11 2-2-2-2"></path><path d="M11 13h4"></path><rect width="18" height="18" x="3" y="3" rx="2" ry="2"></rect>"#
            }
```

`crates/web/src/nav.rs`, efter Passkeys-raden:

```rust
                            <NavItem href="/settings/tokens" icon=IconName::SquareTerminal label="API-tokens" />
```

`crates/web/src/pages/mod.rs`: `mod api_tokens;` och `pub use api_tokens::{ApiTokens, NewApiToken};`. `crates/web/src/app.rs`: importera dem och lägg till efter passkeys-routen:

```rust
                        <Route path=path!("/settings/tokens") view=|| view! { <SignedIn><ApiTokens /></SignedIn> } />
                        <Route path=path!("/settings/tokens/new") view=|| view! { <SignedIn><NewApiToken /></SignedIn> } />
```

`crates/web/src/pages/api_tokens.rs`:

```rust
//! The signed-in user's API tokens: list and revoke, and create one with
//! scopes per company. The secret is shown once, right after creation.

use crate::active_company::Companies;
use crate::api::{api, pb};
use crate::errors::describe;
use crate::format::{date, plus_days, today};
use crate::task::spawn_local;
use crate::ui::{
    Badge, BadgeVariant, Button, Card, Checkbox, ErrorAlert, Field, IconName, LinkButton,
    PageHeader, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table,
    TableCard, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;

/// The form's areas in order: what reading and (if any) writing grant.
const AREAS: [(&str, &str, Option<(&str, &str)>); 5] = [
    ("Läsa bokföring", "ledger:read", Some(("Skriva bokföring", "ledger:write"))),
    ("Läsa fakturor", "invoicing:read", Some(("Skriva fakturor", "invoicing:write"))),
    ("Läsa lön", "payroll:read", Some(("Skriva lön", "payroll:write"))),
    ("Läsa moms", "vat:read", Some(("Skriva moms", "vat:write"))),
    ("Läsa bolagsuppgifter", "company:read", None),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenStatus {
    Active,
    Expired,
    Revoked,
}

/// A token's status at `now` (RFC 3339, UTC, as `Date.toISOString` gives).
pub fn status_of(token: &pb::ApiToken, now: &str) -> TokenStatus {
    if token.revoked_at.is_some() {
        TokenStatus::Revoked
    } else if token.expires_at.as_str() <= now {
        TokenStatus::Expired
    } else {
        TokenStatus::Active
    }
}

/// One company's grant from its (read, write) boxes in `AREAS` order.
/// Writing brings reading; a company with nothing ticked gets no grant.
pub fn grant(company_id: &str, boxes: &[(bool, bool)]) -> Option<pb::TokenGrant> {
    let mut scopes = Vec::new();
    for ((_, read, write), &(r, w)) in AREAS.iter().zip(boxes) {
        let write = write.filter(|_| w);
        if r || write.is_some() {
            scopes.push(read.to_string());
        }
        if let Some((_, scope)) = write {
            scopes.push(scope.to_string());
        }
    }
    (!scopes.is_empty()).then(|| pb::TokenGrant {
        company_id: company_id.into(),
        scopes,
    })
}

fn now_utc() -> String {
    js_sys::Date::new_0().to_iso_string().into()
}

#[component]
pub fn ApiTokens() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let tokens = RwSignal::new(Vec::<pb::ApiToken>::new());
    let error = RwSignal::new(None::<String>);

    let refresh = move || {
        spawn_local(async move {
            match api().list_api_tokens(pb::ListApiTokensRequest {}).await {
                Ok(list) => tokens.set(list.into_inner().tokens),
                Err(status) => error.set(Some(describe(&status))),
            }
        })
    };
    refresh();

    let revoke = move |token: pb::ApiToken| {
        let question = format!("Återkalla token {}?", token.name);
        if !window().confirm_with_message(&question).unwrap_or(false) {
            return;
        }
        error.set(None);
        spawn_local(async move {
            let request = pb::RevokeApiTokenRequest { token_id: token.id };
            match api().revoke_api_token(request).await {
                Ok(_) => refresh(),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let company_names = move |grants: &[pb::TokenGrant]| {
        let list = companies.list.get();
        grants
            .iter()
            .map(|g| {
                list.iter()
                    .find(|c| c.id == g.company_id)
                    .map_or("Okänt bolag".to_owned(), |c| c.name.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="API-tokens" description="För doris-cli och andra program som arbetar åt dig.">
                <LinkButton href="/settings/tokens/new" icon=IconName::Plus>"Ny token"</LinkButton>
            </PageHeader>
            <ErrorAlert message=error />
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Bolag"</th>
                        <th class=TABLE_HEADER_CELL>"Giltig till"</th>
                        <th class=TABLE_HEADER_CELL>"Senast använd"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || tokens.get() key=|t| (t.id.clone(), t.revoked_at.clone()) let(token)>
                        {
                            let status = status_of(&token, &now_utc());
                            let names = company_names(&token.grants);
                            let row = token.clone();
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>{token.name.clone()}</td>
                                    <td class=TABLE_CELL>{names}</td>
                                    <td class=TABLE_CELL>{date(&token.expires_at).to_owned()}</td>
                                    <td class=TABLE_CELL>
                                        {token.last_used_at.as_deref().map_or("Aldrig".to_owned(), |at| date(at).to_owned())}
                                    </td>
                                    <td class=TABLE_CELL>
                                        {match status {
                                            TokenStatus::Active => view! { <Badge>"Aktiv"</Badge> }.into_any(),
                                            TokenStatus::Expired => view! { <Badge variant=BadgeVariant::Outline>"Utgången"</Badge> }.into_any(),
                                            TokenStatus::Revoked => view! { <Badge variant=BadgeVariant::Outline>"Återkallad"</Badge> }.into_any(),
                                        }}
                                    </td>
                                    <td class=TABLE_CELL>
                                        <Show when=move || status == TokenStatus::Active>
                                            {
                                                let row = row.clone();
                                                view! {
                                                    <Button variant=Variant::Ghost kind="button" on:click=move |_| revoke(row.clone())>
                                                        "Återkalla"
                                                    </Button>
                                                }
                                            }
                                        </Show>
                                    </td>
                                </tr>
                            }
                        }
                    </For>
                </tbody>
            </Table></TableCard>
        </div>
    }
}

/// One company's boxes in the form: (read, write) per area.
#[derive(Clone)]
struct CompanyBoxes {
    id: String,
    name: String,
    boxes: Vec<(RwSignal<bool>, RwSignal<bool>)>,
}

#[component]
pub fn NewApiToken() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let name = RwSignal::new(String::new());
    let last_day = RwSignal::new(plus_days(&today(), 90).unwrap_or_default());
    let rows = RwSignal::new(Vec::<CompanyBoxes>::new());
    let secret = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    Effect::new(move |_| {
        let list = companies.list.get();
        rows.set(
            list.into_iter()
                .map(|c| CompanyBoxes {
                    id: c.id,
                    name: c.name,
                    boxes: AREAS
                        .iter()
                        .map(|_| (RwSignal::new(false), RwSignal::new(false)))
                        .collect(),
                })
                .collect(),
        );
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let grants = rows
            .get_untracked()
            .iter()
            .filter_map(|row| {
                let boxes: Vec<_> = row
                    .boxes
                    .iter()
                    .map(|(r, w)| (r.get_untracked(), w.get_untracked()))
                    .collect();
                grant(&row.id, &boxes)
            })
            .collect();
        spawn_local(async move {
            let request = pb::CreateApiTokenRequest {
                name: name.get_untracked(),
                expires_on: last_day.get_untracked(),
                grants,
            };
            match api().create_api_token(request).await {
                Ok(created) => secret.set(Some(created.into_inner().secret)),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Ny token" />
            {move || match secret.get() {
                Some(value) => view! {
                    <Card title="Din token" description="Token visas bara nu. Spara den på ett säkert ställe.">
                        <div class="grid gap-4">
                            <div class="grid gap-2">
                                <label for="api_token_secret" class="text-xs/relaxed font-medium">"Token"</label>
                                <input id="api_token_secret" readonly value=value class="h-7 w-full rounded-md border border-input bg-muted px-2 font-mono text-xs/relaxed" />
                            </div>
                            <div><LinkButton href="/settings/tokens">"Klar"</LinkButton></div>
                        </div>
                    </Card>
                }.into_any(),
                None => view! {
                    <Card title="Behörigheter" description="En token kan aldrig mer än du själv. Skriva innefattar läsa.">
                        <form class="grid gap-4" novalidate on:submit=submit>
                            <div class="grid gap-4 sm:grid-cols-2 sm:max-w-xl">
                                <Field label="Namn" id="token_name" placeholder="t.ex. doris-cli på laptopen" value=name />
                                <Field label="Giltig till och med" id="token_last_day" kind="date" value=last_day hint=Signal::derive(|| Some("Högst ett år.")) />
                            </div>
                            <For each=move || rows.get() key=|row| row.id.clone() let(row)>
                                <fieldset class="grid gap-2 rounded-md border border-border p-3">
                                    <legend class="px-1 text-xs/relaxed font-medium">{row.name.clone()}</legend>
                                    <div class="grid gap-2 sm:grid-cols-2">
                                        {AREAS.iter().zip(row.boxes.clone()).map(|((read_label, read, write), (r, w))| {
                                            Effect::new(move |_| if w.get() { r.set(true) });
                                            view! {
                                                <Checkbox label=read_label.to_string() id=format!("{}-{read}", row.id) checked=r />
                                                {match write {
                                                    Some((write_label, scope)) => view! {
                                                        <Checkbox label=write_label.to_string() id=format!("{}-{scope}", row.id) checked=w />
                                                    }.into_any(),
                                                    None => view! { <span></span> }.into_any(),
                                                }}
                                            }
                                        }).collect_view()}
                                    </div>
                                </fieldset>
                            </For>
                            <p class="text-xs/relaxed text-muted-foreground">
                                "Läsa lön ger också AGI-filen, som innehåller de anställdas personnummer."
                            </p>
                            <ErrorAlert message=error />
                            <div><Button disabled=busy>"Skapa token"</Button></div>
                        </form>
                    </Card>
                }.into_any(),
            }}
        </div>
    }
}
```

`js_sys::Date::to_iso_string` finns i `js-sys` som webben redan använder. Kompilerar `For`-raden inte med `key` som tupel, använd `key=|t| format!("{}{:?}", t.id, t.revoked_at)`.

Lägg till `"/settings/tokens", "/settings/tokens/new",` efter `"/settings/passkeys",` i `paths` i `e2e/tests/design.spec.ts` och i `pages` i `e2e/tests/leaving.spec.ts`. I `e2e/tests/fixtures.ts`, `MENU_OF`: lägg till `"API-tokens": "Konto",`.

- [ ] **Step 4: Kör testerna och lint**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make web`
Expected: PASS, inga varningar, frontend byggs.

- [ ] **Step 5: Kör designkontrollerna**

Run: `cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts leaving.spec.ts`
Expected: PASS (en `<h1>`, ingen sidledsskroll vid 1280 och 390 px, sidorna kan lämnas mitt i laddningen).

- [ ] **Step 6: Commit**

```bash
git add crates/web e2e/tests/design.spec.ts e2e/tests/leaving.spec.ts e2e/tests/fixtures.ts
git commit -m "Add the API tokens pages to the account menu"
```

---

### Task 8: E2E: skapa, använda och återkalla en token

**Files:**
- Create: `e2e/tests/tokens.spec.ts`

**Interfaces:**
- Consumes: Task 7:s sidor; `register`, `addCompany`, `goTo` från `fixtures.ts`.

- [ ] **Step 1: Skriv testet**

```ts
import { request } from "@playwright/test";
import { test, expect, register, addCompany, goTo } from "./fixtures";

/** ListCompanies (an empty message) over gRPC-Web with only a bearer token:
 * no browser, no cookie, as doris-cli calls. Returns the grpc-status. */
async function listCompaniesWith(app: string, token: string): Promise<string | undefined> {
  const cli = await request.newContext();
  const response = await cli.post(`${app}/doris.company.v1.CompanyService/ListCompanies`, {
    headers: { "content-type": "application/grpc-web+proto", "x-grpc-web": "1", authorization: `Bearer ${token}` },
    data: Buffer.from([0, 0, 0, 0, 0]),
  });
  const status = response.headers()["grpc-status"] ?? (await response.body()).toString("latin1").match(/grpc-status: ?(\d+)/)?.[1];
  await cli.dispose();
  return status;
}

test("a token is created, shown once, used without a session and revoked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await goTo(page, "API-tokens");
  await expect(page.getByRole("heading", { level: 1, name: "API-tokens" })).toBeVisible();
  await page.getByRole("link", { name: "Ny token" }).click();
  await page.getByLabel("Namn").fill("doris-cli");
  await page.getByRole("group", { name: "Exempel AB" }).getByLabel("Läsa bokföring").check();
  await page.getByRole("button", { name: "Skapa token" }).click();

  const secret = await page.getByLabel("Token").inputValue();
  expect(secret).toMatch(/^doris_[A-Za-z0-9_-]{43}$/);
  expect(await listCompaniesWith(app, secret)).toBe("0");

  await page.getByRole("link", { name: "Klar" }).click();
  const row = page.getByRole("row", { name: /doris-cli/ });
  await expect(row.getByText("Aktiv")).toBeVisible();
  await expect(row.getByText("Exempel AB")).toBeVisible();
  page.once("dialog", (dialog) => dialog.accept());
  await row.getByRole("button", { name: "Återkalla" }).click();
  await expect(row.getByText("Återkallad")).toBeVisible();

  expect(await listCompaniesWith(app, secret)).toBe("16");
});

test("a token without any scope is refused in Swedish", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/settings/tokens/new`);
  await page.getByLabel("Namn").fill("doris-cli");
  await page.getByRole("button", { name: "Skapa token" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ge token minst en behörighet.");
});
```

Om `fixtures.ts` inte exporterar `expect`, importera den från `@playwright/test` som de andra spec-filerna gör.

- [ ] **Step 2: Kör testet**

Run: `make e2e` (eller `cd e2e && npx playwright test tokens.spec.ts` efter `make web` och `cargo build -p doris-server`)
Expected: PASS. (Testet täcker beteende som redan finns från Task 4–7; fallerar det, är det en bugg att rätta med ett fallerande enhetstest först.)

- [ ] **Step 3: Commit**

```bash
git add e2e/tests/tokens.spec.ts
git commit -m "Test an API token end to end, from the form to a cookieless call"
```

---

### Task 9: AGENTS.md

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Uppdatera**

Under `## Authentication`, efter stycket om sessioner:

```markdown
- API tokens (`doris_` + 256 random bits, sent as `authorization: Bearer
  …`) let doris-cli and agents call the API without a passkey. A user
  creates them in the account menu (API-tokens); each has a last day at
  most a year off, ends at midnight in Sweden after it, and can be revoked
  by its owner or an admin. A token is never changed: it is revoked and a
  new one made. Only its SHA-256 is stored (`api_tokens`, events in
  `api-token-{id}`); it is never logged.
- A token has scopes per company (`ledger|invoicing|payroll|vat:read|write`,
  `company:read`) and never more than its owner: membership is checked on
  every call, as for a session. `crates/server/src/access.rs` says what each
  rpc needs from a token; an rpc missing there is closed to tokens, and a
  test keeps it in step with `proto/`. `auth_gate` (`crates/server/src/lib.rs`)
  authenticates the token, and every event appended in the call records it
  as `via_token` in the metadata (`doris_eventstore::VIA_TOKEN`). A token
  cannot manage tokens, invite, create companies or add members.
```

Under `## API`, en ny punkt:

```markdown
- `AuthService` also has `CreateApiToken`, `ListApiTokens` and
  `RevokeApiToken` (session only). Codes: `invalid_token_name`,
  `invalid_token_expiry`, `invalid_token_grants`, `api_token_not_found`,
  `missing_scope` (the token lacks the scope for that company) and
  `token_not_allowed` (the rpc is for sessions only).
```

Ersätt "`session_gate` (`crates/server/src/lib.rs`) answers `LedgerService` and `InvoicingService` calls without a valid session" med "`auth_gate` (`crates/server/src/lib.rs`) answers `LedgerService` and `InvoicingService` calls without a valid session or token".

- [ ] **Step 2: Kör hela sviten**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "Document API tokens in AGENTS.md"
```
