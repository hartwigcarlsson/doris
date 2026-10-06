# Ändra API-tokens med passkey – Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ägaren kan ändra en API-tokens namn, sista dag och behörigheter utan att hemligheten byts, och både att skapa och att ändra en token bekräftas med en passkey-ceremoni som godkänner just den ändringen.

**Architecture:** Identity får eventet `ApiTokenChanged` och de rena typerna `TokenChange`/`TokenRequest`. `doris_identity::Auth` får ceremonin `ApiToken` (begin: kontrollera allt och starta en passkey-autentisering mot användarens egna passkeys; finish: ta ceremonin, verifiera assertionen, returnera den godkända `TokenRequest`). Servern kontrollerar medlemskap vid begin och igen efter finish, och skriver sedan `ApiTokenCreated` eller `ApiTokenChanged` i en egen skrivtransaktion. `CreateApiToken` ersätts av fyra RPC:er (begin/finish för skapa och ändra). Webben får ett gemensamt formulär för "Ny token" och "Ändra token" som kör `navigator.credentials.get` mellan begin och finish.

**Tech Stack:** Rust 2024, jiff, serde, sqlx 0.9 (SQLite), webauthn-rs, tonic 0.14 + prost, Leptos 0.8 CSR, Playwright (virtuell autentiserare).

**Spec:** `docs/superpowers/specs/2026-10-06-api-token-andring-design.md` (bygger på `docs/superpowers/specs/2026-10-06-api-tokens-design.md`)

### Avsteg från specen (medvetna, enklare)
1. **En ceremonisort i stället för två.** `Ceremony::ApiToken { user_id, request: TokenRequest, state }`, där `TokenRequest` är `Create { change }` eller `Change { token_id, change }`. Samma beteende; en finish av fel sort ger `ceremony_expired` i servern.
2. **Medlemskapet kontrolleras i servern**, eftersom identity inte läser company. Därför returnerar `Auth::finish_api_token` den godkända begäran, och servern kontrollerar bolagen och skriver eventet. Assertionen och skrivningen ligger alltså i två transaktioner, och domänen beslutar om igen i skrivningen (återkallad token: `api_token_revoked`).
3. **Ändringssidan hämtar token via `ListApiTokens`** (ingen ny `GetApiToken`).

## Global Constraints
- Kod, identifierare, URL:er, proto, händelsenamn och commits på engelska; bara synlig UI-text på svenska.
- TDD: inget utan ett test som först fallerar. Varje cykel slutar i en commit, med raderna `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` och `Claude-Session: https://claude.ai/code/session_01GGh5ZH92u9PPnjiByuz674` sist.
- `events` är append-only. Nya händelser är JSON med `schema_version` 1; `ApiTokenCreated` och `ApiTokenRevoked` får inte ändra betydelse.
- Projektioner uppdateras i samma transaktion som appenden och kan byggas om från `read_all`.
- WebAuthn-ceremonier lever i `webauthn_ceremonies` i 5 minuter (`CEREMONY_TTL`) och kan avslutas en gång.
- Att skapa och att ändra en token kräver en passkey-assertion från den inloggade användarens egen passkey; att återkalla gör det inte.
- Bara ägaren ändrar en token (även en admin får `api_token_not_found`). En återkallad token kan inte ändras (`api_token_revoked`). En utgången kan få en ny sista dag.
- Sista dag högst 366 dagar fram, slut vid midnatt svensk tid efter dagen (`token_expiry`); domänen kräver `now < expires_at <= now + MAX_TOKEN_LIFETIME` (368 dagar).
- Felkoder: `api_token_revoked` (ny), `ceremony_expired`, `credential_rejected`, `invalid_ceremony`, `invalid_credential`, `api_token_not_found`, `company_not_found`, `invalid_token_name`, `invalid_token_expiry`, `invalid_token_grants`, `token_not_allowed`.
- Token loggas aldrig. E-post och personnummer loggas aldrig.
- UI följer `docs/design/README.md`; sidor startar uppgifter med `crate::task::spawn_local`.
- Lint: `cargo clippy --workspace -- -D warnings`, `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings` och `cargo fmt --all --check` för filer branchen rör.

## Review Focus
1. **Passkey-dialogen avbryts** (användaren trycker Avbryt): ingen ändring sparas, formuläret visar ett fel och knappen går att trycka igen. Ceremonin blir liggande och går ut av sig själv. Täcks i Task 5 (`busy` släpps i alla grenar) och kontrolleras för hand.
2. **Samma ändring skickas två gånger** (dubbelklick, eller två flikar): den andra finish ger `ceremony_expired` eller, om den har egen ceremoni, inget nytt event (oförändrad token ger inga events). Test i Task 1 (`an_unchanged_token_yields_no_events`) och Task 4 (`each_ceremony_is_finished_once_by_the_user_who_began_it`).
3. **Token återkallas mitt i ceremonin** (från en annan flik): finish ger `api_token_revoked` och ingenting ändras. Test i Task 4 (`a_token_revoked_during_the_ceremony_is_not_changed`).
4. **Utgången token förlängs**: samma hemlighet fungerar igen efteråt. Test i Task 2 (`an_expired_token_is_found_again_after_a_new_last_day`) och Task 4 (`an_expired_token_gets_a_new_last_day`).
5. **Ett bolag användaren inte längre har i listan** finns kvar i tokenens grants: ändringsformuläret visar det inte och det följer inte med (bolagslistan styr raderna). Kontrolleras i Task 5 via `boxes_from` och formulärets rader; servern kontrollerar ändå varje bolag vid begin och efter finish.

---

### Task 1: Domänen: ändra en token

**Files:**
- Modify: `crates/identity/src/domain.rs`
- Modify: `crates/server/src/grpc.rs` (`domain_code` och `status`: `TokenRevoked`)
- Test: `crates/identity/tests/api_token_domain.rs`

**Interfaces:**
- Produces (i `doris_identity::domain`):
  - `DomainError::TokenRevoked`
  - `ApiTokenEvent::ApiTokenChanged { name: String, expires_at: Timestamp, grants: Vec<Grant> }`
  - `pub struct TokenChange { pub name: String, pub expires_at: Timestamp, pub grants: Vec<Grant> }` (`Debug, Clone, PartialEq, Serialize, Deserialize`)
  - `pub enum TokenRequest { Create { change: TokenChange }, Change { token_id: Uuid, change: TokenChange } }` (samma derives, `#[serde(tag = "action", rename_all = "snake_case")]`) med `fn change(&self) -> &TokenChange`
  - `pub fn change_api_token(token: &ApiToken, actor: &User, change: TokenChange, now: Timestamp) -> Result<Vec<ApiTokenEvent>, DomainError>`

- [ ] **Step 1: Skriv de fallerande testerna**

Lägg till i `crates/identity/tests/api_token_domain.rs` (filen har redan `now()`, `user(role)`, `grant(company, scopes)`, `cmd(name, expires_in, grants)`, `DAY` och `token_of(owner)`):

```rust
fn change(name: &str, expires_in: SignedDuration, grants: Vec<Grant>) -> TokenChange {
    TokenChange {
        name: name.into(),
        expires_at: now() + expires_in,
        grants,
    }
}

#[test]
fn the_owner_changes_name_expiry_and_grants() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let company = Uuid::new_v4();

    let events = change_api_token(
        &token,
        &anna,
        change(
            " CLI ",
            90 * DAY,
            vec![grant(company, &[Scope::LedgerWrite, Scope::LedgerRead])],
        ),
        now(),
    )
    .unwrap();

    assert_eq!(
        events,
        vec![ApiTokenEvent::ApiTokenChanged {
            name: "CLI".into(),
            expires_at: now() + 90 * DAY,
            grants: vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])],
        }]
    );
    let changed = ApiToken::from_events(
        [
            ApiTokenEvent::ApiTokenCreated {
                token_id: token.id,
                user_id: anna.id,
                name: token.name.clone(),
                token_hash: "h".into(),
                expires_at: token.expires_at,
                grants: token.grants.clone(),
            },
            events[0].clone(),
        ]
        .iter(),
    )
    .unwrap();
    assert_eq!(changed.name, "CLI");
    assert_eq!(changed.expires_at, now() + 90 * DAY);
    assert_eq!(changed.grants, vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])]);
    assert!(!changed.revoked);
}

#[test]
fn a_change_is_validated_like_a_new_token() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let company = Uuid::new_v4();
    for (bad, expected) in [
        (change("  ", DAY, vec![grant(company, &[Scope::LedgerRead])]), DomainError::InvalidTokenName),
        (change("CLI", -DAY, vec![grant(company, &[Scope::LedgerRead])]), DomainError::InvalidTokenExpiry),
        (change("CLI", 369 * DAY, vec![grant(company, &[Scope::LedgerRead])]), DomainError::InvalidTokenExpiry),
        (change("CLI", DAY, vec![]), DomainError::InvalidTokenGrants),
        (change("CLI", DAY, vec![grant(company, &[])]), DomainError::InvalidTokenGrants),
    ] {
        assert_eq!(change_api_token(&token, &anna, bad.clone(), now()), Err(expected), "{bad:?}");
    }
}

#[test]
fn only_the_owner_changes_a_token_not_even_an_admin() {
    let token = token_of(&user(Role::Member));
    let new = change("CLI", DAY, vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])]);
    for actor in [user(Role::Member), user(Role::Admin)] {
        assert_eq!(
            change_api_token(&token, &actor, new.clone(), now()),
            Err(DomainError::NotTokenOwner)
        );
    }
}

#[test]
fn a_revoked_token_cannot_be_changed() {
    let anna = user(Role::Member);
    let mut token = token_of(&anna);
    token.revoked = true;
    assert_eq!(
        change_api_token(&token, &anna, change("CLI", DAY, token.grants.clone()), now()),
        Err(DomainError::TokenRevoked)
    );
}

#[test]
fn an_expired_token_gets_a_new_last_day() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let later = now() + 10 * DAY; // the token expired at now() + DAY
    let events = change_api_token(
        &token,
        &anna,
        TokenChange {
            name: token.name.clone(),
            expires_at: later + 30 * DAY,
            grants: token.grants.clone(),
        },
        later,
    )
    .unwrap();
    assert!(matches!(
        &events[..],
        [ApiTokenEvent::ApiTokenChanged { expires_at, .. }] if *expires_at == later + 30 * DAY
    ));
}

#[test]
fn an_unchanged_token_yields_no_events() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let same = TokenChange {
        name: format!(" {} ", token.name),
        expires_at: token.expires_at,
        grants: token.grants.clone(),
    };
    assert_eq!(change_api_token(&token, &anna, same, now()).unwrap(), vec![]);
}

#[test]
fn a_token_request_is_stored_with_its_action() {
    let request = TokenRequest::Change {
        token_id: Uuid::nil(),
        change: change("CLI", DAY, vec![grant(Uuid::nil(), &[Scope::VatRead])]),
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["action"], "change");
    assert_eq!(serde_json::from_value::<TokenRequest>(json).unwrap(), request);
    assert_eq!(request.change().name, "CLI");
}
```

Om `token_of` i filen inte ger tokenen `name: "Agent"`, `expires_at: now() + DAY` och ett bolag med `LedgerRead`, läs den och anpassa testerna efter dess värden.

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-identity --test api_token_domain`
Expected: kompileringsfel: `cannot find type TokenChange`, `cannot find function change_api_token`.

- [ ] **Step 3: Implementera**

I `crates/identity/src/domain.rs`, `DomainError`, efter `NotTokenOwner`:

```rust
    #[error("api token is revoked")]
    TokenRevoked,
```

I `ApiTokenEvent`, efter `ApiTokenRevoked`:

```rust
    /// The owner replaced its name, expiry and grants; the secret stays.
    ApiTokenChanged {
        name: String,
        expires_at: Timestamp,
        grants: Vec<Grant>,
    },
```

I `ApiToken::from_events`, före `(_, None) => {}`:

```rust
                (
                    ApiTokenEvent::ApiTokenChanged {
                        name,
                        expires_at,
                        grants,
                    },
                    Some(token),
                ) => {
                    token.name = name.clone();
                    token.expires_at = *expires_at;
                    token.grants = grants.clone();
                }
```

Nya typer (efter `NewApiToken`):

```rust
/// A token's name, expiry and grants, as created or changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenChange {
    pub name: String,
    pub expires_at: Timestamp,
    pub grants: Vec<Grant>,
}

/// What a passkey is asked to confirm about an API token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TokenRequest {
    Create { change: TokenChange },
    Change { token_id: Uuid, change: TokenChange },
}

impl TokenRequest {
    pub fn change(&self) -> &TokenChange {
        match self {
            TokenRequest::Create { change } | TokenRequest::Change { change, .. } => change,
        }
    }
}
```

Bryt ut valideringen ur `create_api_token` och använd den i båda:

```rust
/// The change with its name trimmed and grants normalized, if it is valid
/// at `now`.
fn validated(change: TokenChange, now: Timestamp) -> Result<TokenChange, DomainError> {
    let name = bounded_text(&change.name, 100).ok_or(DomainError::InvalidTokenName)?;
    if change.expires_at <= now || change.expires_at > now + MAX_TOKEN_LIFETIME {
        return Err(DomainError::InvalidTokenExpiry);
    }
    Ok(TokenChange {
        name,
        expires_at: change.expires_at,
        grants: normalized(change.grants)?,
    })
}

pub fn create_api_token(
    owner: &User,
    cmd: NewApiToken,
    token_hash: String,
    now: Timestamp,
) -> Result<ApiTokenEvent, DomainError> {
    let valid = validated(
        TokenChange {
            name: cmd.name,
            expires_at: cmd.expires_at,
            grants: cmd.grants,
        },
        now,
    )?;
    Ok(ApiTokenEvent::ApiTokenCreated {
        token_id: cmd.token_id,
        user_id: owner.id,
        name: valid.name,
        token_hash,
        expires_at: valid.expires_at,
        grants: valid.grants,
    })
}

/// Only the owner changes a token, and never a revoked one. A change that
/// changes nothing yields no events.
pub fn change_api_token(
    token: &ApiToken,
    actor: &User,
    change: TokenChange,
    now: Timestamp,
) -> Result<Vec<ApiTokenEvent>, DomainError> {
    if token.user_id != actor.id {
        return Err(DomainError::NotTokenOwner);
    }
    if token.revoked {
        return Err(DomainError::TokenRevoked);
    }
    let valid = validated(change, now)?;
    if valid.name == token.name && valid.expires_at == token.expires_at && valid.grants == token.grants {
        return Ok(vec![]);
    }
    Ok(vec![ApiTokenEvent::ApiTokenChanged {
        name: valid.name,
        expires_at: valid.expires_at,
        grants: valid.grants,
    }])
}
```

`crates/server/src/grpc.rs`: i `domain_code`, lägg till `DomainError::TokenRevoked => "api_token_revoked",`; i `status`, före armen `Error::Domain(err) =>`:

```rust
        Error::Domain(DomainError::TokenRevoked) => Status::failed_precondition("api_token_revoked"),
```

`crates/identity/src/projections.rs` matchar `ApiTokenEvent` fullständigt och slutar kompilera. Lägg till en arm `ApiTokenEvent::ApiTokenChanged { .. } => {}` tills vidare, med kommentaren `// Projected in the next commit.` – Task 2 ersätter den.

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity && cargo build -p doris-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/identity crates/server/src/grpc.rs
git commit -m "Decide a change to an API token: owner only, never when revoked"
```

---

### Task 2: Lagra en ändring och kontrollera utan att spara

**Files:**
- Modify: `crates/identity/src/projections.rs`
- Modify: `crates/identity/src/api_token.rs`
- Modify: `crates/identity/src/lib.rs` (exporter)
- Test: `crates/identity/tests/api_tokens.rs`

**Interfaces:**
- Consumes: Task 1:s `TokenChange`, `change_api_token`.
- Produces (i `doris_identity`):
  - `pub async fn change_api_token(pool: &SqlitePool, actor_id: Uuid, token_id: Uuid, change: TokenChange, now: Timestamp) -> Result<()>`
  - `pub async fn check_new_api_token(pool: &SqlitePool, owner_id: Uuid, change: &TokenChange, now: Timestamp) -> Result<()>`
  - `pub async fn check_api_token_change(pool: &SqlitePool, actor_id: Uuid, token_id: Uuid, change: &TokenChange, now: Timestamp) -> Result<()>`

- [ ] **Step 1: Skriv de fallerande testerna**

Lägg till i `crates/identity/tests/api_tokens.rs` (filen har `now()`, `DAY`, `db()`, `admin_and_member(pool) -> (Uuid, Uuid)` och `grants(company)`); utöka importerna med `change_api_token, check_api_token_change, check_new_api_token` och `doris_identity::domain::TokenChange`:

```rust
fn to(name: &str, expires_at: Timestamp, grants: Vec<Grant>) -> TokenChange {
    TokenChange {
        name: name.into(),
        expires_at,
        grants,
    }
}

async fn events(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_changed_token_keeps_its_secret_and_reaches_what_it_now_has() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let (id, secret) = create_api_token(&pool, bo, "Agent", now() + DAY, grants(a), now())
        .await
        .unwrap();
    let wider = vec![
        Grant { company_id: a, scopes: vec![Scope::LedgerRead, Scope::LedgerWrite] },
        Grant { company_id: b, scopes: vec![Scope::VatRead] },
    ];

    change_api_token(&pool, bo, id, to("CLI", now() + 30 * DAY, wider.clone()), now())
        .await
        .unwrap();

    let (_, access) = token_user(&pool, &secret, now()).await.unwrap().unwrap();
    let mut expected = wider.clone();
    expected.sort_by_key(|g| g.company_id);
    assert_eq!(access.grants, expected);
    let listed = &list_api_tokens(&pool, bo).await.unwrap()[0];
    assert_eq!(listed.name, "CLI");
    assert_eq!(listed.expires_at, now() + 30 * DAY);
}

#[tokio::test]
async fn an_expired_token_is_found_again_after_a_new_last_day() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (id, secret) = create_api_token(&pool, bo, "Agent", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    let later = now() + 10 * DAY;
    assert!(token_user(&pool, &secret, later).await.unwrap().is_none());

    let same_grants = list_api_tokens(&pool, bo).await.unwrap()[0].grants.clone();
    change_api_token(&pool, bo, id, to("Agent", later + DAY, same_grants), later)
        .await
        .unwrap();

    assert!(token_user(&pool, &secret, later).await.unwrap().is_some());
}

#[tokio::test]
async fn checks_refuse_like_the_writes_and_save_nothing() {
    let pool = db().await;
    let (anna, bo) = admin_and_member(&pool).await;
    let (bos, _) = create_api_token(&pool, bo, "Bo", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();
    let before = events(&pool).await;

    check_new_api_token(&pool, bo, &to("Agent", now() + DAY, grants(Uuid::new_v4())), now())
        .await
        .unwrap();
    check_api_token_change(&pool, bo, bos, &to("CLI", now() + DAY, grants(Uuid::new_v4())), now())
        .await
        .unwrap();
    assert!(matches!(
        check_new_api_token(&pool, bo, &to(" ", now() + DAY, grants(Uuid::new_v4())), now()).await,
        Err(Error::Domain(DomainError::InvalidTokenName))
    ));
    assert!(matches!(
        check_api_token_change(&pool, anna, bos, &to("CLI", now() + DAY, grants(Uuid::new_v4())), now()).await,
        Err(Error::Domain(DomainError::NotTokenOwner))
    ));
    assert!(matches!(
        check_api_token_change(&pool, bo, Uuid::new_v4(), &to("CLI", now() + DAY, grants(Uuid::new_v4())), now()).await,
        Err(Error::ApiTokenNotFound)
    ));
    revoke_api_token(&pool, bo, bos).await.unwrap();
    let after_revoke = events(&pool).await;
    assert!(matches!(
        change_api_token(&pool, bo, bos, to("CLI", now() + DAY, grants(Uuid::new_v4())), now()).await,
        Err(Error::Domain(DomainError::TokenRevoked))
    ));

    assert_eq!(after_revoke, before + 1, "only the revocation was saved");
    assert_eq!(events(&pool).await, after_revoke);
}
```

I det befintliga testet `api_tokens_rebuild_from_the_log`: ändra den andra token innan snapshoten tas, så att rebuild täcker `ApiTokenChanged`. Spara id:t från den andra `create_api_token` (`let (second, _) = …`) och lägg till före `let snapshot = …`:

```rust
    change_api_token(&pool, bo, second, to("Ändrad", now() + 2 * DAY, grants(Uuid::new_v4())), now())
        .await
        .unwrap();
```

och efter `assert_eq!(before.len(), 2);`:

```rust
    assert!(before.iter().any(|row| row.2 == "Ändrad"), "{before:?}");
```

(`row.2` är kolumnen `name`.)

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-identity --test api_tokens`
Expected: kompileringsfel: `change_api_token`, `check_new_api_token`, `check_api_token_change` finns inte.

- [ ] **Step 3: Implementera**

`crates/identity/src/projections.rs`: ersätt platshållararmen från Task 1 med:

```rust
        ApiTokenEvent::ApiTokenChanged {
            name,
            expires_at,
            grants,
        } => {
            sqlx::query(
                "UPDATE api_tokens SET name = ?, expires_at = ?, grants = ? WHERE token_id = ?",
            )
            .bind(name)
            .bind(expires_at.as_second())
            .bind(serde_json::to_string(&grants)?)
            .bind(token_id)
            .execute(&mut *conn)
            .await?;
        }
```

`crates/identity/src/api_token.rs` (importera `TokenChange` från `crate::domain`):

```rust
/// Replaces a token's name, expiry and grants. Its secret stays.
pub async fn change_api_token(
    pool: &SqlitePool,
    actor_id: Uuid,
    token_id: Uuid,
    change: TokenChange,
    now: Timestamp,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (actor, _) = load_user(&mut tx, actor_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let (events, version) = load_stream::<ApiTokenEvent>(&mut tx, &stream(token_id)).await?;
    let token = ApiToken::from_events(&events).ok_or(Error::ApiTokenNotFound)?;
    let events = domain::change_api_token(&token, &actor, change, now)?;
    commit(&mut tx, &stream(token_id), version, &events, Some(actor_id)).await?;
    tx.commit().await?;
    Ok(())
}

/// Runs every rule for a new token without saving it, so a passkey
/// ceremony can refuse before the authenticator is asked.
pub async fn check_new_api_token(
    pool: &SqlitePool,
    owner_id: Uuid,
    change: &TokenChange,
    now: Timestamp,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let (owner, _) = load_user(&mut conn, owner_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let cmd = NewApiToken {
        token_id: Uuid::nil(),
        name: change.name.clone(),
        expires_at: change.expires_at,
        grants: change.grants.clone(),
    };
    domain::create_api_token(&owner, cmd, String::new(), now)?;
    Ok(())
}

/// Runs every rule for a change without saving it.
pub async fn check_api_token_change(
    pool: &SqlitePool,
    actor_id: Uuid,
    token_id: Uuid,
    change: &TokenChange,
    now: Timestamp,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let (actor, _) = load_user(&mut conn, actor_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let (events, _) = load_stream::<ApiTokenEvent>(&mut conn, &stream(token_id)).await?;
    let token = ApiToken::from_events(&events).ok_or(Error::ApiTokenNotFound)?;
    domain::change_api_token(&token, &actor, change.clone(), now)?;
    Ok(())
}
```

`crates/identity/src/lib.rs`: lägg till `change_api_token, check_api_token_change, check_new_api_token` i `pub use api_token::{…}`.

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity && cargo build -p doris-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/identity
git commit -m "Store a changed API token and check token requests without saving"
```

---

### Task 3: Passkey-ceremonin för tokens

**Files:**
- Modify: `crates/identity/src/webauthn.rs`
- Modify: `crates/identity/src/lib.rs` (`Error::CredentialRejected`)
- Modify: `crates/server/src/grpc.rs` (`status`: `CredentialRejected`)
- Test: `crates/identity/tests/webauthn.rs`

**Interfaces:**
- Consumes: Task 1:s `TokenRequest`; Task 2:s `check_new_api_token`, `check_api_token_change`.
- Produces:
  - `Error::CredentialRejected` i `doris_identity`
  - `Auth::begin_api_token(&self, user_id: Uuid, request: TokenRequest, now: Timestamp) -> Result<(Uuid, RequestChallengeResponse)>`
  - `Auth::finish_api_token(&self, user_id: Uuid, ceremony_id: Uuid, credential: &PublicKeyCredential, now: Timestamp) -> Result<TokenRequest>`

- [ ] **Step 1: Skriv de fallerande testerna**

Lägg till i `crates/identity/tests/webauthn.rs` (filen har `now()`, `origin()`, `authenticator()`, `setup()`, `sign_up(auth, device, email, token) -> (User, String)` och `log_in`); utöka importerna med `doris_identity::domain::{Grant, Scope, TokenChange, TokenRequest}`, `doris_identity::create_api_token` och `jiff::SignedDuration`:

```rust
fn new_token() -> TokenRequest {
    TokenRequest::Create {
        change: TokenChange {
            name: "Agent".into(),
            expires_at: now() + SignedDuration::from_hours(24),
            grants: vec![Grant {
                company_id: uuid::Uuid::new_v4(),
                scopes: vec![Scope::LedgerRead],
            }],
        },
    }
}

async fn ceremonies(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn passkey_uses(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE event_type = 'PasskeyUsed'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_passkey_confirms_a_token_request() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;

    let (ceremony, options) = auth.begin_api_token(anna.id, new_token(), now()).await.unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let confirmed = auth.finish_api_token(anna.id, ceremony, &assertion, now()).await.unwrap();

    assert!(matches!(&confirmed, TokenRequest::Create { change } if change.name == "Agent"));
    assert_eq!(passkey_uses(&pool).await, 1, "the use (and counter) is recorded");
    assert_eq!(ceremonies(&pool).await, 0);
}

#[tokio::test]
async fn a_token_request_is_refused_before_the_authenticator_is_asked() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let mut bad = new_token();
    if let TokenRequest::Create { change } = &mut bad {
        change.name = " ".into();
    }

    let err = auth.begin_api_token(anna.id, bad, now()).await.unwrap_err();
    let someone_elses = auth
        .begin_api_token(
            anna.id,
            TokenRequest::Change {
                token_id: uuid::Uuid::new_v4(),
                change: new_token().change().clone(),
            },
            now(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Domain(DomainError::InvalidTokenName)), "{err:?}");
    assert!(matches!(someone_elses, Error::ApiTokenNotFound), "{someone_elses:?}");
    assert_eq!(ceremonies(&pool).await, 0);
}

#[tokio::test]
async fn only_the_users_own_passkey_confirms_their_token_request() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let mut bos = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now()).await.unwrap();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&invitation)).await;

    let (ceremony, _) = auth.begin_api_token(anna.id, new_token(), now()).await.unwrap();
    let (_, bos_options) = auth.begin_login("bo@example.se", now()).await.unwrap();
    let bos_assertion = bos.do_authentication(origin(), bos_options).unwrap();
    let err = auth
        .finish_api_token(anna.id, ceremony, &bos_assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::CredentialRejected), "{err:?}");
}

#[tokio::test]
async fn a_token_ceremony_is_finished_once_by_the_user_who_began_it_within_five_minutes() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now()).await.unwrap();
    let (bo, _) = sign_up(&auth, &mut authenticator(), "bo@example.se", Some(&invitation)).await;

    // Finished by someone else: gone, as if it never was.
    let (ceremony, options) = auth.begin_api_token(anna.id, new_token(), now()).await.unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let by_bo = auth.finish_api_token(bo.id, ceremony, &assertion, now()).await.unwrap_err();
    let again = auth.finish_api_token(anna.id, ceremony, &assertion, now()).await.unwrap_err();

    // Finished too late.
    let (late, options) = auth.begin_api_token(anna.id, new_token(), now()).await.unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let expired = auth
        .finish_api_token(anna.id, late, &assertion, now() + CEREMONY_TTL)
        .await
        .unwrap_err();

    assert!(matches!(by_bo, Error::CeremonyNotFound), "{by_bo:?}");
    assert!(matches!(again, Error::CeremonyNotFound), "{again:?}");
    assert!(matches!(expired, Error::CeremonyExpired), "{expired:?}");
}

#[tokio::test]
async fn logging_in_still_fails_the_same_way_for_a_wrong_passkey() {
    // finish_login now shares the assertion check with token ceremonies.
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now()).await.unwrap();
    let mut bos = authenticator();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&invitation)).await;
    let (_, token_options) = auth.begin_api_token(anna.id, new_token(), now()).await.unwrap();
    let annas_assertion = annas.do_authentication(origin(), token_options).unwrap();
    let (bos_login, _) = auth.begin_login("bo@example.se", now()).await.unwrap();

    let err = auth.finish_login(bos_login, &annas_assertion, now()).await.unwrap_err();

    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}
```

`create_api_token` behövs inte i testerna ovan; lämna den importen bort om kompilatorn varnar.

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-identity --test webauthn`
Expected: kompileringsfel: `begin_api_token`, `finish_api_token`, `Error::CredentialRejected` finns inte.

- [ ] **Step 3: Implementera**

`crates/identity/src/lib.rs`, i `Error`, efter `LoginFailed`:

```rust
    /// A passkey assertion that doesn't verify against the user's own
    /// passkeys, outside login (where every failure is `LoginFailed`).
    #[error("passkey not accepted")]
    CredentialRejected,
```

`crates/server/src/grpc.rs`, i `status`, efter armen för `LoginFailed`:

```rust
        Error::CredentialRejected => Status::invalid_argument("credential_rejected"),
```

`crates/identity/src/webauthn.rs`:
- Modulens doc: `//! WebAuthn ceremonies: registering with a passkey, adding passkeys, logging in and confirming an API token request. …`
- Importera `check_api_token_change, check_new_api_token` från `crate` och `TokenRequest` från `crate::domain`.
- Ny variant i `Ceremony`:

```rust
    /// Confirms exactly this create or change of an API token.
    ApiToken {
        user_id: Uuid,
        request: TokenRequest,
        state: PasskeyAuthentication,
    },
```

- I `impl Auth`:

```rust
    /// Checks every rule for the token request, then asks the browser for
    /// an assertion from one of the user's own passkeys. Nothing changes
    /// until [`Auth::finish_api_token`]; company membership is the caller's
    /// to check.
    pub async fn begin_api_token(
        &self,
        user_id: Uuid,
        request: TokenRequest,
        now: Timestamp,
    ) -> Result<(Uuid, RequestChallengeResponse)> {
        match &request {
            TokenRequest::Create { change } => {
                check_new_api_token(&self.pool, user_id, change, now).await?
            }
            TokenRequest::Change { token_id, change } => {
                check_api_token_change(&self.pool, user_id, *token_id, change, now).await?
            }
        }
        let (options, state) = self.start_user_authentication(user_id).await?;
        let ceremony = Ceremony::ApiToken {
            user_id,
            request,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the assertion and records the passkey's use. Returns the
    /// request the passkey confirmed, for the caller to carry out. A
    /// ceremony another user began looks like one that doesn't exist.
    pub async fn finish_api_token(
        &self,
        user_id: Uuid,
        ceremony_id: Uuid,
        credential: &PublicKeyCredential,
        now: Timestamp,
    ) -> Result<TokenRequest> {
        let Ceremony::ApiToken {
            user_id: owner,
            request,
            state,
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::CeremonyNotFound);
        };
        if owner != user_id {
            return Err(Error::CeremonyNotFound);
        }
        self.verified_use(user_id, credential, &state)
            .await?
            .ok_or(Error::CredentialRejected)?;
        Ok(request)
    }

    /// An authentication challenge for one user's own passkeys.
    async fn start_user_authentication(
        &self,
        user_id: Uuid,
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication)> {
        let user = get_user(&self.pool, user_id)
            .await?
            .ok_or(Error::UserNotFound)?;
        let passkeys = user
            .passkeys
            .iter()
            .map(webauthn_passkey)
            .collect::<Result<Vec<_>>>()?;
        if passkeys.is_empty() {
            return Err(Error::CredentialRejected);
        }
        Ok(self.webauthn.start_passkey_authentication(&passkeys)?)
    }

    /// Verifies an assertion against the user's own passkeys and records
    /// the use with its updated counter. `None` when it doesn't verify.
    async fn verified_use(
        &self,
        user_id: Uuid,
        credential: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<Option<User>> {
        let Ok(result) = self.webauthn.finish_passkey_authentication(credential, state) else {
            return Ok(None);
        };
        let Some(user) = get_user(&self.pool, user_id).await? else {
            return Ok(None);
        };
        let used_id = credential_id(result.cred_id());
        let Some(stored) = user.passkeys.iter().find(|p| p.credential_id == used_id) else {
            return Ok(None);
        };
        let mut passkey = webauthn_passkey(stored)?;
        passkey.update_credential(&result);
        record_passkey_use(&self.pool, user_id, &used_id, serde_json::to_value(&passkey)?).await?;
        Ok(Some(user))
    }
```

- `finish_login` använder `verified_use` i stället för sin egen kontroll (samma beteende: varje fel blir `LoginFailed`):

```rust
        let Ceremony::Login {
            user_id: Some(user_id),
            state: Some(state),
        } = ceremony
        else {
            return Err(Error::LoginFailed);
        };
        let user = self
            .verified_use(user_id, credential, &state)
            .await?
            .ok_or(Error::LoginFailed)?;
        let session = create_session(&self.pool, user_id, now).await?;
        Ok((user, session))
```

Användaren som returneras från `finish_login` hämtas nu före `record_passkey_use`, precis som förut.

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity && cargo build -p doris-server`
Expected: PASS, inklusive alla befintliga inloggningstester.

- [ ] **Step 5: Commit**

```bash
git add crates/identity crates/server/src/grpc.rs
git commit -m "Confirm an API token request with the user's own passkey"
```

---

### Task 4: AuthService: skapa och ändra med passkey

**Files:**
- Modify: `proto/doris/auth/v1/auth.proto`
- Modify: `crates/server/src/grpc.rs` (handlers och hjälpfunktioner)
- Modify: `crates/server/src/access.rs` (tabellen och dess test)
- Modify: `crates/server/tests/common/mod.rs` (hjälpare)
- Modify: `crates/server/tests/api_tokens.rs` (befintliga tester flyttas till de nya RPC:erna, nya tester)

**Interfaces:**
- Consumes: Task 2:s `doris_identity::{create_api_token, change_api_token}`; Task 3:s `Auth::{begin_api_token, finish_api_token}`; Task 1:s `TokenChange`, `TokenRequest`.
- Produces: RPC:erna `BeginCreateApiToken`, `FinishCreateApiToken`, `BeginChangeApiToken`, `FinishChangeApiToken` (proto nedan; `CreateApiToken` tas bort); testhjälparna `TestServer::confirm(device, begin) -> String`, `TestServer::create_token(session, device, request) -> Result<pb::CreateApiTokenResponse, Status>`, `TestServer::change_token(session, device, request) -> Result<(), Status>`, `TestServer::invite_with(admin, email, device) -> String` och `api_token(server, session, device, grants) -> String`.

- [ ] **Step 1: Ändra proto**

I `service AuthService`, ersätt raden `rpc CreateApiToken(…)` (och dess kommentar) med:

```proto
  // Require a session (never a token). Creating and changing a token are
  // ceremonies: Begin checks the request and returns WebAuthn request
  // options; Finish takes the assertion from one of the user's own
  // passkeys and carries out exactly what Begin was given. A token is shown
  // once, by FinishCreateApiToken.
  rpc BeginCreateApiToken(CreateApiTokenRequest) returns (BeginCeremonyResponse);
  rpc FinishCreateApiToken(FinishApiTokenRequest) returns (CreateApiTokenResponse);
  rpc BeginChangeApiToken(ChangeApiTokenRequest) returns (BeginCeremonyResponse);
  rpc FinishChangeApiToken(FinishApiTokenRequest) returns (ChangeApiTokenResponse);
```

Nya meddelanden efter `CreateApiTokenResponse`:

```proto
// Replaces the token's name, last day and grants; its secret stays.
message ChangeApiTokenRequest {
  string token_id = 1;
  string name = 2;
  string expires_on = 3; // YYYY-MM-DD: the last day the token works (Swedish time)
  repeated TokenGrant grants = 4;
}

message FinishApiTokenRequest {
  string ceremony_id = 1;
  string credential_json = 2;
}

message ChangeApiTokenResponse {}
```

`crates/server/src/access.rs`: i `AuthService`-armen för `SessionOnly`, ersätt `"CreateApiToken"` med `"BeginCreateApiToken" | "FinishCreateApiToken" | "BeginChangeApiToken" | "FinishChangeApiToken"`. I `the_table_follows_the_spec`, ersätt raden för `/doris.auth.v1.AuthService/CreateApiToken` med `("/doris.auth.v1.AuthService/BeginCreateApiToken", SessionOnly)` och lägg till `("/doris.auth.v1.AuthService/FinishChangeApiToken", SessionOnly)`.

- [ ] **Step 2: Testhjälpare**

I `crates/server/tests/common/mod.rs`, i `impl TestServer` (efter `invite_as`):

```rust
    /// Like `invite`, keeping the new user's passkey on `device`.
    pub async fn invite_with(&self, admin: &str, email: &str, device: &mut Device) -> String {
        let invite = self
            .grpc()
            .create_invitation(authed(
                pb::CreateInvitationRequest {
                    email: email.into(),
                },
                admin,
            ))
            .await
            .unwrap()
            .into_inner();
        self.sign_up(device, email, Some(&invite.token)).await
    }

    /// `device`'s assertion for a ceremony's request options, as the browser
    /// gives it after the user touches the passkey.
    pub fn confirm(&self, device: &mut Device, begin: &pb::BeginCeremonyResponse) -> String {
        let options: RequestChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
        let credential = device
            .do_authentication(self.origin.clone(), options)
            .unwrap();
        serde_json::to_string(&credential).unwrap()
    }

    /// Creates a token: begins, confirms with `device`'s passkey, finishes.
    pub async fn create_token(
        &self,
        session: &str,
        device: &mut Device,
        request: pb::CreateApiTokenRequest,
    ) -> Result<pb::CreateApiTokenResponse, tonic::Status> {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_create_api_token(authed(request, session))
            .await?
            .into_inner();
        let finish = pb::FinishApiTokenRequest {
            credential_json: self.confirm(device, &begin),
            ceremony_id: begin.ceremony_id,
        };
        Ok(grpc
            .finish_create_api_token(authed(finish, session))
            .await?
            .into_inner())
    }

    /// Changes a token: begins, confirms with `device`'s passkey, finishes.
    pub async fn change_token(
        &self,
        session: &str,
        device: &mut Device,
        request: pb::ChangeApiTokenRequest,
    ) -> Result<(), tonic::Status> {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_change_api_token(authed(request, session))
            .await?
            .into_inner();
        let finish = pb::FinishApiTokenRequest {
            credential_json: self.confirm(device, &begin),
            ceremony_id: begin.ceremony_id,
        };
        grpc.finish_change_api_token(authed(finish, session)).await?;
        Ok(())
    }
```

Ersätt den fria funktionen `api_token` med:

```rust
/// A 30-day token of `session`'s with these scopes per company, confirmed
/// with `device`'s passkey; returns its secret.
pub async fn api_token(
    server: &TestServer,
    session: &str,
    device: &mut Device,
    grants: &[(&str, &[&str])],
) -> String {
    server
        .create_token(
            session,
            device,
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
        )
        .await
        .unwrap()
        .secret
}
```

- [ ] **Step 3: Flytta de befintliga testerna till de nya RPC:erna**

I `crates/server/tests/api_tokens.rs`, mekaniskt:
- `let anna = server.sign_up(&mut device(), "anna@example.se", None).await;` blir `let mut annas = device();` + `let anna = server.sign_up(&mut annas, "anna@example.se", None).await;` i varje test som skapar eller ändrar en token. Samma för `bo` och `cecilia` när de skapar tokens: `let mut bos = device(); let bo = server.invite_with(&anna, "bo@example.se", &mut bos).await;`.
- `api_token(&server, &anna, GRANTS)` blir `api_token(&server, &anna, &mut annas, GRANTS)`.
- `api.create_api_token(authed(REQUEST, &SESSION)).await` blir `server.create_token(&SESSION, &mut DEVICE, REQUEST).await`. Resultatet är `Result<pb::CreateApiTokenResponse, Status>` som förut (`.unwrap()` i stället för `.unwrap().into_inner()`, `.unwrap_err()` som förut).
- Fel som kommer redan vid begin (`invalid_token_grants`, `company_not_found`, `invalid_token_expiry`) kan testas direkt med `server.grpc().begin_create_api_token(authed(REQUEST, &anna))`, vilket inte behöver en passkey. Gör så i `a_token_without_grants_is_refused`, `a_token_is_only_for_the_users_own_companies_and_known_scopes` och `the_last_day_is_today_at_the_earliest_and_a_year_off_at_most` (för den giltiga 366-dagarsraden räcker att begin lyckas).
- I `a_token_cannot_manage_tokens_invite_or_create_companies_but_knows_its_owner`: `auth.create_api_token(bearer(…))` blir `auth.begin_create_api_token(bearer(…))`.
- `doris_identity::create_api_token(&server.pool, …)` för den utgångna tokenen är oförändrad.

Kör `cargo test -p doris-server --test api_tokens` efter Step 5 för att se att de flyttade testerna fortfarande säger samma sak.

- [ ] **Step 4: Skriv de nya fallerande testerna**

Lägg till i `crates/server/tests/api_tokens.rs` (filen har `code_of`, `create`, `grant`, `sale`, `vouchers`):

```rust
fn change_of(token_id: &str, company: &str, scopes: &[&str]) -> pb::ChangeApiTokenRequest {
    pb::ChangeApiTokenRequest {
        token_id: token_id.into(),
        name: "Agent".into(),
        expires_on: in_days(30),
        grants: vec![grant(company, scopes)],
    }
}

async fn only_token_id(server: &TestServer, session: &str) -> String {
    server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, session))
        .await
        .unwrap()
        .into_inner()
        .tokens[0]
        .id
        .clone()
}

#[tokio::test]
async fn a_token_is_changed_with_a_passkey_and_its_secret_keeps_working() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let token_id = only_token_id(&server, &anna).await;
    let mut ledger = server.ledger();
    let before = ledger.record_voucher(bearer(sale(&id), &secret)).await.unwrap_err();

    server
        .change_token(&anna, &mut annas, change_of(&token_id, &id, &["ledger:read", "ledger:write"]))
        .await
        .unwrap();
    ledger.record_voucher(bearer(sale(&id), &secret)).await.unwrap();
    server
        .change_token(&anna, &mut annas, change_of(&token_id, &id, &["company:read"]))
        .await
        .unwrap();
    let after = ledger.list_vouchers(bearer(vouchers(&id), &secret)).await.unwrap_err();

    assert_eq!(code_of(before), (Code::PermissionDenied, "missing_scope".into()));
    assert_eq!(code_of(after), (Code::PermissionDenied, "missing_scope".into()));
    let listed = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert_eq!(listed.len(), 1, "changed, not replaced");
    assert_eq!(listed[0].grants, vec![grant(&id, &["company:read"])]);
}

#[tokio::test]
async fn each_ceremony_is_finished_once_by_the_user_who_began_it() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_create_api_token(authed(create("Agent", in_days(30), vec![grant(&id, &["ledger:read"])]), &anna))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishApiTokenRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id.clone(),
    };

    let by_bo = auth.finish_create_api_token(authed(finish.clone(), &bo)).await.unwrap_err();
    let again = auth.finish_create_api_token(authed(finish, &anna)).await.unwrap_err();

    // A create ceremony finished as a change.
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let begin = auth
        .begin_create_api_token(authed(create("Agent", in_days(30), vec![grant(&id, &["ledger:read"])]), &anna))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishApiTokenRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };
    let wrong_kind = auth.finish_change_api_token(authed(finish, &anna)).await.unwrap_err();

    for err in [by_bo, again, wrong_kind] {
        assert_eq!(code_of(err), (Code::FailedPrecondition, "ceremony_expired".into()));
    }
    assert!(!secret.is_empty());
}

#[tokio::test]
async fn a_change_is_refused_at_begin_before_any_passkey() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let mut bos = device();
    let bo = server.invite_with(&anna, "bo@example.se", &mut bos).await;
    let annas_company = company(&server, &anna, "556016-0680").await;
    let bos_company = company(&server, &bo, "556036-0793").await;
    api_token(&server, &anna, &mut annas, &[(&annas_company, &["ledger:read"])]).await;
    let annas_token = only_token_id(&server, &anna).await;
    api_token(&server, &bo, &mut bos, &[(&bos_company, &["ledger:read"])]).await;
    let bos_token = only_token_id(&server, &bo).await;
    let begin = |request: pb::ChangeApiTokenRequest| {
        let mut auth = server.grpc();
        let anna = anna.clone();
        async move { auth.begin_change_api_token(authed(request, &anna)).await.unwrap_err() }
    };

    let mut empty_name = change_of(&annas_token, &annas_company, &["ledger:read"]);
    empty_name.name = " ".into();
    let errors = [
        (begin(empty_name).await, (Code::InvalidArgument, "invalid_token_name")),
        (begin(change_of(&annas_token, &annas_company, &[])).await, (Code::InvalidArgument, "invalid_token_grants")),
        (begin(change_of(&annas_token, &bos_company, &["ledger:read"])).await, (Code::NotFound, "company_not_found")),
        (begin(change_of(&bos_token, &annas_company, &["ledger:read"])).await, (Code::NotFound, "api_token_not_found")),
        (begin(change_of("nej", &annas_company, &["ledger:read"])).await, (Code::NotFound, "api_token_not_found")),
    ];

    for (err, (code, message)) in errors {
        assert_eq!(code_of(err), (code, message.to_owned()));
    }
    let ceremonies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(ceremonies, 0);
}

#[tokio::test]
async fn a_token_revoked_during_the_ceremony_is_not_changed() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let token_id = only_token_id(&server, &anna).await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_change_api_token(authed(change_of(&token_id, &id, &["ledger:write"]), &anna))
        .await
        .unwrap()
        .into_inner();
    auth.revoke_api_token(authed(pb::RevokeApiTokenRequest { token_id }, &anna))
        .await
        .unwrap();
    let finish = pb::FinishApiTokenRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };

    let err = auth.finish_change_api_token(authed(finish, &anna)).await.unwrap_err();

    assert_eq!(code_of(err), (Code::FailedPrecondition, "api_token_revoked".into()));
}

#[tokio::test]
async fn an_expired_token_gets_a_new_last_day() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let me: uuid::Uuid = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .current_user
        .unwrap()
        .id
        .parse()
        .unwrap();
    let then = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(48);
    let (token_id, secret) = doris_identity::create_api_token(
        &server.pool,
        me,
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
    let mut ledger = server.ledger();
    let before = ledger.list_vouchers(bearer(vouchers(&id), &secret)).await.unwrap_err();

    server
        .change_token(&anna, &mut annas, change_of(&token_id.to_string(), &id, &["ledger:read"]))
        .await
        .unwrap();

    assert_eq!(code_of(before), (Code::Unauthenticated, "not_signed_in".into()));
    ledger.list_vouchers(bearer(vouchers(&id), &secret)).await.unwrap();
}
```

`pb::FinishApiTokenRequest` behöver `Clone`, vilket prost genererar.

- [ ] **Step 5: Kör testerna och se dem fallera**

Run: `cargo test -p doris-server --test api_tokens`
Expected: kompileringsfel: `AuthService` saknar `begin_create_api_token` m.fl. (proto är ändrad men inte handlers).

- [ ] **Step 6: Implementera**

`crates/server/src/grpc.rs`, importer: `use doris_identity::domain::{DomainError, Grant, Role, Scope, TokenChange, TokenRequest, User};`.

Ta bort `create_api_token`-handlern. I `impl AuthApi` (inte trait-impl), lägg till:

```rust
    /// A token's name, last day and grants as the client sent them. Every
    /// company must be one the user is a member of.
    async fn token_change(
        &self,
        user_id: Uuid,
        name: &str,
        expires_on: &str,
        grants: &[pb::TokenGrant],
    ) -> Result<TokenChange, Status> {
        let expires_at = token_expiry(expires_on, Timestamp::now())?;
        let mut parsed = Vec::with_capacity(grants.len());
        for grant in grants {
            let company_id: Uuid = grant
                .company_id
                .parse()
                .map_err(|_| Status::not_found("company_not_found"))?;
            self.member_of(user_id, company_id).await?;
            let scopes = grant
                .scopes
                .iter()
                .map(|s| Scope::parse(s))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| Status::invalid_argument("invalid_token_grants"))?;
            parsed.push(Grant { company_id, scopes });
        }
        Ok(TokenChange {
            name: name.to_owned(),
            expires_at,
            grants: parsed,
        })
    }

    /// Membership has one source: the company module.
    async fn member_of(&self, user_id: Uuid, company_id: Uuid) -> Result<(), Status> {
        doris_company::get_company(&self.pool, company_id, user_id)
            .await
            .map_err(crate::company::status)?;
        Ok(())
    }

    /// The token request a passkey just confirmed, with every company
    /// checked again: the user may have left one meanwhile.
    async fn confirmed(
        &self,
        user_id: Uuid,
        req: &pb::FinishApiTokenRequest,
    ) -> Result<TokenRequest, Status> {
        let request = self
            .auth
            .finish_api_token(
                user_id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        for grant in &request.change().grants {
            self.member_of(user_id, grant.company_id).await?;
        }
        Ok(request)
    }
```

I `impl AuthService for AuthApi`:

```rust
    async fn begin_create_api_token(
        &self,
        request: Request<pb::CreateApiTokenRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let change = self
            .token_change(user.id, &req.name, &req.expires_on, &req.grants)
            .await?;
        let (ceremony_id, options) = self
            .auth
            .begin_api_token(user.id, TokenRequest::Create { change }, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_create_api_token(
        &self,
        request: Request<pb::FinishApiTokenRequest>,
    ) -> Result<Response<pb::CreateApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let TokenRequest::Create { change } = self.confirmed(user.id, request.get_ref()).await?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
        let (token_id, secret) = doris_identity::create_api_token(
            &self.pool,
            user.id,
            &change.name,
            change.expires_at,
            change.grants,
            Timestamp::now(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CreateApiTokenResponse {
            token_id: token_id.to_string(),
            secret,
        }))
    }

    async fn begin_change_api_token(
        &self,
        request: Request<pb::ChangeApiTokenRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let token_id = api_token_id(&req.token_id)?;
        let change = self
            .token_change(user.id, &req.name, &req.expires_on, &req.grants)
            .await?;
        let (ceremony_id, options) = self
            .auth
            .begin_api_token(
                user.id,
                TokenRequest::Change { token_id, change },
                Timestamp::now(),
            )
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_change_api_token(
        &self,
        request: Request<pb::FinishApiTokenRequest>,
    ) -> Result<Response<pb::ChangeApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let TokenRequest::Change { token_id, change } =
            self.confirmed(user.id, request.get_ref()).await?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
        doris_identity::change_api_token(&self.pool, user.id, token_id, change, Timestamp::now())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ChangeApiTokenResponse {}))
    }
```

Fri funktion, och använd den även i `revoke_api_token` i stället för den inbyggda parsningen:

```rust
fn api_token_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("api_token_not_found"))
}
```

`ceremony_id`, `credential`, `ceremony_response` och `finish_status` finns redan i `grpc.rs`.

- [ ] **Step 7: Kör testerna**

Run: `cargo test -p doris-server && cargo clippy --workspace -- -D warnings`
Expected: PASS, inga varningar.

- [ ] **Step 8: Commit**

```bash
git add proto/doris/auth/v1/auth.proto crates/server
git commit -m "Create and change API tokens through a passkey ceremony"
```

---

### Task 5: Webben: ändra token, och passkey vid skapa och ändra

**Files:**
- Modify: `crates/web/src/pages/api_tokens.rs`
- Modify: `crates/web/src/pages/mod.rs`, `crates/web/src/app.rs` (route `/settings/tokens/:id`)
- Modify: `crates/web/src/errors.rs`
- Modify: `e2e/tests/design.spec.ts` (`paths`), `e2e/tests/leaving.spec.ts` (`pages`)

**Interfaces:**
- Consumes: Task 4:s RPC:er via `crate::api::{api, pb}`; `crate::passkey::get(options_json) -> Result<String, String>`.
- Produces: komponenterna `NewApiToken` (oförändrat namn) och `EditApiToken`; den privata komponenten `TokenForm`; den rena funktionen `boxes_from(grants: &[pb::TokenGrant], company_id: &str) -> Vec<(bool, bool)>`.

- [ ] **Step 1: Skriv de fallerande testerna**

I `crates/web/src/pages/api_tokens.rs`, `mod tests`:

```rust
    #[test]
    fn the_boxes_show_what_a_token_grants_in_a_company() {
        let grants = vec![
            pb::TokenGrant {
                company_id: "c1".into(),
                scopes: ["ledger:read", "ledger:write", "vat:read", "company:read"]
                    .map(String::from)
                    .to_vec(),
            },
            pb::TokenGrant {
                company_id: "c2".into(),
                scopes: vec!["payroll:read".into()],
            },
        ];
        let c1 = boxes_from(&grants, "c1");
        assert_eq!(
            c1,
            vec![(true, true), (false, false), (false, false), (true, false), (true, false)]
        );
        // The boxes give back the same grant.
        assert_eq!(grant("c1", &c1), Some(grants[0].clone()));
        assert_eq!(boxes_from(&grants, "c3"), vec![(false, false); 5]);
    }
```

I `crates/web/src/errors.rs`, i `api_token_codes_have_swedish_messages`, lägg till `"api_token_revoked"` i listan.

- [ ] **Step 2: Kör testerna och se dem fallera**

Run: `cargo test -p doris-web api_token`
Expected: kompileringsfel (`boxes_from` finns inte), sedan fallerar errors-testet för `api_token_revoked`.

- [ ] **Step 3: Implementera**

`crates/web/src/errors.rs`: `"api_token_revoked" => "Token är återkallad och kan inte ändras.",`.

`crates/web/src/pages/api_tokens.rs`:

```rust
/// One company's (read, write) boxes, in `AREAS` order, for what `grants`
/// gives it: the inverse of [`grant`].
pub fn boxes_from(grants: &[pb::TokenGrant], company_id: &str) -> Vec<(bool, bool)> {
    let scopes = grants
        .iter()
        .find(|g| g.company_id == company_id)
        .map(|g| g.scopes.as_slice())
        .unwrap_or_default();
    let has = |scope: &str| scopes.iter().any(|s| s == scope);
    AREAS
        .iter()
        .map(|(_, read, write)| (has(read), write.is_some_and(|(_, w)| has(w))))
        .collect()
}

/// Begins creating (`token_id` `None`) or changing a token, has one of the
/// user's passkeys confirm it, and finishes. A new token's secret comes back.
async fn save_with_passkey(
    token_id: Option<String>,
    name: String,
    expires_on: String,
    grants: Vec<pb::TokenGrant>,
) -> Result<Option<String>, String> {
    let mut api = api();
    let begin = match &token_id {
        None => {
            api.begin_create_api_token(pb::CreateApiTokenRequest { name, expires_on, grants })
                .await
        }
        Some(id) => {
            api.begin_change_api_token(pb::ChangeApiTokenRequest {
                token_id: id.clone(),
                name,
                expires_on,
                grants,
            })
            .await
        }
    }
    .map_err(|s| describe(&s))?
    .into_inner();
    let credential_json = passkey::get(&begin.options_json).await?;
    let finish = pb::FinishApiTokenRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    };
    match token_id {
        None => Ok(Some(
            api.finish_create_api_token(finish)
                .await
                .map_err(|s| describe(&s))?
                .into_inner()
                .secret,
        )),
        Some(_) => {
            api.finish_change_api_token(finish)
                .await
                .map_err(|s| describe(&s))?;
            Ok(None)
        }
    }
}
```

Flytta formulärdelen av dagens `NewApiToken` (namn, sista dag, `boxes_of`, raderna per bolag med effekterna, AGI-texten, felet och knappen, och `submit`) till en ny komponent och låt den fyllas i från en token:

```rust
/// The token form: name, last day and boxes per company. With `token` it
/// is filled in from that token and saving changes it; without, saving
/// creates one. Saving asks for a passkey. `saved` gets a new token's
/// secret, or `None` after a change. Companies come from the user's list,
/// so one the user no longer has is neither shown nor kept.
#[component]
fn TokenForm(token: Option<pb::ApiToken>, saved: Callback<Option<String>>) -> impl IntoView
```

- `name` startar som `token.name` eller tom; `last_day` som `date(&token.expires_at)` eller `plus_days(&today(), 90)`.
- Varje rads `boxes` skapas från `boxes_from(&grants, &company.id)` (`grants` är tokenens, eller tom) i stället för alla `false`: `RwSignal::new(r), RwSignal::new(w)`.
- `submit` bygger `grants` som förut och kör `save_with_passkey(token_id, name, last_day, grants)` i `crate::task::spawn_local`; `Ok(secret)` anropar `saved.run(secret)`, `Err(message)` sätter `error`. `busy` sätts `false` i båda fallen.
- Knappen heter `"Skapa med passkey"` utan token och `"Spara med passkey"` med token.

`NewApiToken` blir:

```rust
#[component]
pub fn NewApiToken() -> impl IntoView {
    let secret = RwSignal::new(None::<String>);
    view! {
        <div class="grid gap-6">
            <PageHeader title="Ny token" />
            {move || match secret.get() {
                Some(value) => view! { /* the existing "Din token" card, unchanged */ }.into_any(),
                None => view! {
                    <TokenForm token=None saved=Callback::new(move |s: Option<String>| secret.set(s)) />
                }.into_any(),
            }}
        </div>
    }
}
```

(behåll "Din token"-kortet exakt som det är).

```rust
/// Changes a token of the user's: the same form, filled in.
#[component]
pub fn EditApiToken() -> impl IntoView {
    let params = use_params_map();
    let id = params.read_untracked().get("id").unwrap_or_default();
    let token = RwSignal::new(None::<pb::ApiToken>);
    let error = RwSignal::new(None::<String>);
    let navigate = use_navigate();
    spawn_local(async move {
        match api().list_api_tokens(pb::ListApiTokensRequest {}).await {
            Ok(list) => match list.into_inner().tokens.into_iter().find(|t| t.id == id) {
                Some(found) if found.revoked_at.is_none() => token.set(Some(found)),
                Some(_) => error.set(Some(describe_code("api_token_revoked"))),
                None => error.set(Some(describe_code("api_token_not_found"))),
            },
            Err(status) => error.set(Some(describe(&status))),
        }
    });
    let done = Callback::new(move |_: Option<String>| navigate("/settings/tokens", Default::default()));
    view! {
        <div class="grid gap-6">
            <PageHeader title="Ändra token" />
            <ErrorAlert message=error />
            {move || token.get().map(|t| view! { <TokenForm token=Some(t) saved=done /> })}
        </div>
    }
}
```

Importera `use leptos_router::hooks::{use_navigate, use_params_map};`, `crate::errors::describe_code` och `crate::passkey`. Följ `pages/company.rs` för `use_params_map` och `pages/new_company.rs` för `use_navigate` om API:erna skiljer sig från skissen.

I listan (`ApiTokens`), lägg till i åtgärdscellen, före "Återkalla", för varje token som inte är återkallad (`status != TokenStatus::Revoked`):

```rust
<LinkButton href=format!("/settings/tokens/{}", token.id) variant=Variant::Ghost>"Ändra"</LinkButton>
```

`crates/web/src/pages/mod.rs`: exportera `EditApiToken`. `crates/web/src/app.rs`, efter routen för `/settings/tokens/new` (den måste stå före `:id`, som för `/payroll-runs/new`):

```rust
<Route path=path!("/settings/tokens/:id") view=|| view! { <SignedIn><EditApiToken /></SignedIn> } />
```

`e2e/tests/design.spec.ts` (`paths`) och `e2e/tests/leaving.spec.ts` (`pages`): lägg till `"/settings/tokens/00000000-0000-0000-0000-000000000000"` efter `"/settings/tokens/new"` (sidan visar då felet och sin `<h1>`).

- [ ] **Step 4: Kör testerna och lint**

Run: `cargo test -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && make web`
Expected: PASS, inga varningar.

- [ ] **Step 5: Kör designkontrollerna**

Run: `cargo build -p doris-server && cd e2e && npx playwright test design.spec.ts leaving.spec.ts`
Expected: PASS (ett instabilt "login and registration each have one h1" under last har setts förut; kör det ensamt om det fallerar).

- [ ] **Step 6: Commit**

```bash
git add crates/web e2e/tests/design.spec.ts e2e/tests/leaving.spec.ts
git commit -m "Change an API token from its page, and confirm saving with a passkey"
```

---

### Task 6: E2E: skapa och ändra med passkey

**Files:**
- Modify: `e2e/tests/tokens.spec.ts`

**Interfaces:**
- Consumes: Task 5:s sidor och etiketter ("Skapa med passkey", "Spara med passkey", "Ändra", fieldset med bolagets namn, "Läsa bolagsuppgifter").

- [ ] **Step 1: Uppdatera och utöka testet**

I `e2e/tests/tokens.spec.ts`: byt båda `getByRole("button", { name: "Skapa token" })` mot `{ name: "Skapa med passkey" }`. Lägg till:

```ts
/** GetCompany over gRPC-Web with only a bearer token. Returns the grpc-status. */
async function getCompanyWith(app: string, token: string, companyId: string): Promise<string | undefined> {
  const id = Buffer.from(companyId, "utf8");
  const message = Buffer.from([0x0a, id.length, ...id]); // field 1, length-delimited
  const frame = Buffer.concat([Buffer.from([0, 0, 0, 0, message.length]), message]);
  const cli = await request.newContext();
  const response = await cli.post(`${app}/doris.company.v1.CompanyService/GetCompany`, {
    headers: { "content-type": "application/grpc-web+proto", "x-grpc-web": "1", authorization: `Bearer ${token}` },
    data: frame,
  });
  const status = response.headers()["grpc-status"] ?? (await response.body()).toString("latin1").match(/grpc-status: ?(\d+)/)?.[1];
  await cli.dispose();
  return status;
}

test("a token's scopes are changed with a passkey and its secret keeps working", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const companyId = await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/settings/tokens/new`);
  await page.getByLabel("Namn").fill("doris-cli");
  await page.getByRole("group", { name: "Exempel AB" }).getByLabel("Läsa bokföring").check();
  await page.getByRole("button", { name: "Skapa med passkey" }).click();
  const secret = await page.getByLabel("Token").inputValue();
  expect(await getCompanyWith(app, secret, companyId)).toBe("7"); // permission_denied: no company:read

  await page.getByRole("link", { name: "Klar" }).click();
  await page.getByRole("row", { name: /doris-cli/ }).getByRole("link", { name: "Ändra" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Ändra token" })).toBeVisible();
  const group = page.getByRole("group", { name: "Exempel AB" });
  await expect(group.getByLabel("Läsa bokföring")).toBeChecked();
  await group.getByLabel("Läsa bolagsuppgifter").check();
  await page.getByRole("button", { name: "Spara med passkey" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "API-tokens" })).toBeVisible();

  expect(await getCompanyWith(app, secret, companyId)).toBe("0");
  await expect(page.getByRole("row", { name: /doris-cli/ })).toHaveCount(1);
});
```

`addCompany` returnerar bolagets id (se `fixtures.ts`). Ett bolags-id är 36 tecken, så längdbyten `id.length` ryms i en byte.

- [ ] **Step 2: Kör testet**

Run: `make web && cargo build -p doris-server && cd e2e && npx playwright test tokens.spec.ts`
Expected: PASS (3 tester). Testet täcker beteende från Task 4–5; fallerar det på grund av appen, rätta appen med ett fallerande enhetstest först.

- [ ] **Step 3: Commit**

```bash
git add e2e/tests/tokens.spec.ts
git commit -m "Test changing a token's scopes with a passkey end to end"
```

---

### Task 7: AGENTS.md

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Uppdatera**

I stycket om API-tokens under `## Authentication`, ersätt "A token is never changed: it is revoked and a new one made." med:

```markdown
  Creating and changing a token are passkey ceremonies
  (`Auth::begin_api_token`/`finish_api_token`, kind `api_token` in
  `webauthn_ceremonies`): Begin checks the request and asks for one of the
  user's own passkeys, and Finish carries out exactly what Begin was given
  (`ApiTokenCreated` or `ApiTokenChanged`), after the server has checked
  each company's membership again. Only the owner changes a token (name,
  last day, grants; never its secret), and never a revoked one; revoking
  needs no passkey.
```

Under `## API`, i punkten om `AuthService` och tokens: ersätt `CreateApiToken` med `BeginCreateApiToken`, `FinishCreateApiToken`, `BeginChangeApiToken`, `FinishChangeApiToken`, och lägg till `api_token_revoked` bland koderna (`ceremony_expired` och `credential_rejected` gäller också).

- [ ] **Step 2: Kör hela sviten**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo fmt --all --check`
Expected: PASS. (`cargo fmt --check` kan klaga på filer som branchen inte rör; nämn dem i rapporten, rör dem inte.)

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "Document changing API tokens with a passkey in AGENTS.md"
```
