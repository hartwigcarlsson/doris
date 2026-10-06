# Inbjudan och ny medlem med passkey – Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Att bjuda in någon och att lägga till en medlem i ett bolag bekräftas med en av användarens egna passkeys, som tokens och nya passkeys redan gör, så att en stulen session inte kan ge ett andra konto varaktig åtkomst.

**Architecture:** Identitys token-ceremoni görs allmän: `Ceremony::Confirm { user_id, action: Confirmation, state }` med `Confirmation::{ApiToken, Invitation, AddMember}`, och `Auth::begin_confirmation`/`finish_confirmation` ersätter `begin_api_token`/`finish_api_token`. Servern delar `Arc<Auth>` mellan `AuthApi` och `CompanyApi`. `CreateInvitation` och `AddMember` ersätts av begin/finish-par, och webbens två formulär kör `navigator.credentials.get` mellan dem.

**Tech Stack:** Rust 2024, webauthn-rs, sqlx 0.9, tonic 0.14 + prost, Leptos 0.8 CSR, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-06-inbjudan-medlem-passkey-design.md` (bygger på `2026-10-06-api-token-andring-design.md`)

### Avsteg från specen
1. **`BeginAddMember` svarar med ett eget `BeginAddMemberResponse { ceremony_id, options_json }`** i `company.proto`, inte `doris.auth.v1.BeginCeremonyResponse`, av samma skäl som `FinishAddMemberRequest`: company-protot importerar inte auth.
2. **`check_invitation` tar skrivlåset kort och rullar tillbaka**, som `check_registration`, eftersom regeln om upptagen e-post läser i samma transaktion som skrivningen.

## Global Constraints
- Kod, identifierare, proto och commits på engelska; synlig UI-text på svenska. Varje commit slutar med `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` och `Claude-Session: https://claude.ai/code/session_01GGh5ZH92u9PPnjiByuz674`.
- TDD: ett fallerande test först; varje cykel slutar i en commit.
- Ceremonier: tillstånd på servern i `webauthn_ceremonies`, 5 minuter (`CEREMONY_TTL`), avslutas en gång, tillhör användaren som startade; passkeyn måste vara en av användarens egna (`verified_use`).
- Att skapa och ändra en token, lägga till en passkey, bjuda in någon och lägga till en medlem kräver passkey. Att återkalla, skapa bolag, lista och logga ut gör det inte.
- Kontroller körs vid begin (innan passkeyn efterfrågas) och igen när åtgärden skrivs. `AddMember` kontrollerar medlemskap först, så att en icke-medlem inte kan se vilka e-postadresser som finns.
- Felkoder (inga nya): `ceremony_expired`, `credential_rejected`, `invalid_ceremony`, `invalid_credential`, `not_admin`, `already_exists`, `invalid_email`, `company_not_found`, `user_not_found`, `token_not_allowed`.
- Alla nya RPC:er är `SessionOnly` i `crates/server/src/access.rs`.
- E-post och tokens loggas aldrig.
- Lint: `cargo clippy --workspace -- -D warnings`, `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`, `cargo fmt --all --check`.

## Review Focus
1. **En token-ceremoni avslutas som en inbjudan eller medlem (eller tvärtom)**: `ceremony_expired`, ingenting skapas. Test i Task 2 (`a_confirmation_of_one_kind_cannot_finish_another`).
2. **En icke-admin försöker bjuda in**: `not_admin` redan vid begin, ingen ceremoni. Test i Task 1 och Task 2.
3. **En icke-medlem frågar efter en e-post**: `company_not_found` innan e-posten slås upp. Det befintliga testet `non_members_and_bad_ids_get_company_not_found` flyttas till `BeginAddMember` i Task 2.
4. **E-posten blir upptagen mellan begin och finish** (någon registrerar sig eller bjuds in under tiden): finish ger `already_exists`, eftersom `create_invitation` kontrollerar igen. Täcks av att finish går via `doris_identity::create_invitation` (Task 2).
5. **Passkey-dialogen avbryts**: inget skapas, felet visas, knappen släpps. Task 2:s webbändring (`busy` i alla grenar).

---

### Task 1: En allmän bekräftelse i identity

**Files:**
- Modify: `crates/identity/src/domain.rs` (`Confirmation`)
- Modify: `crates/identity/src/lib.rs` (`check_invitation`, `create_invitation` delar regler)
- Modify: `crates/identity/src/webauthn.rs` (`Ceremony::Confirm`, `begin_confirmation`, `finish_confirmation`; ta bort `begin_api_token`/`finish_api_token`)
- Modify: `crates/identity/tests/webauthn.rs`
- Modify: `crates/server/src/grpc.rs` (token-handlers anropar de nya funktionerna, så att servern kompilerar)

**Interfaces:**
- Produces:
  - `doris_identity::domain::Confirmation` (`Debug, Clone, PartialEq, Serialize, Deserialize`, `#[serde(tag = "action", rename_all = "snake_case")]`): `ApiToken { request: TokenRequest }`, `Invitation { email: Email }`, `AddMember { company_id: Uuid, email: Email }`
  - `doris_identity::check_invitation(pool: &SqlitePool, creator_id: Uuid, email: &str, now: Timestamp) -> Result<()>`
  - `Auth::begin_confirmation(&self, user_id: Uuid, action: Confirmation, now: Timestamp) -> Result<(Uuid, RequestChallengeResponse)>`
  - `Auth::finish_confirmation(&self, user_id: Uuid, ceremony_id: Uuid, credential: &PublicKeyCredential, now: Timestamp) -> Result<Confirmation>`

- [ ] **Step 1: Flytta token-testerna och skriv de nya fallerande testerna**

I `crates/identity/tests/webauthn.rs`: importera `Confirmation` och `Email` från `doris_identity::domain`. I de token-tester som finns (`a_passkey_confirms_a_token_request`, `a_token_request_is_refused_before_the_authenticator_is_asked`, `only_the_users_own_passkey_confirms_their_token_request`, `a_token_ceremony_is_finished_once_by_the_user_who_began_it_within_five_minutes`, `logging_in_still_fails_the_same_way_for_a_wrong_passkey`):
- `auth.begin_api_token(USER, REQUEST, now())` blir `auth.begin_confirmation(USER, Confirmation::ApiToken { request: REQUEST }, now())`.
- `auth.finish_api_token(…)` blir `auth.finish_confirmation(…)`.
- `matches!(&confirmed, TokenRequest::Create { change } if change.name == "Agent")` blir `matches!(&confirmed, Confirmation::ApiToken { request: TokenRequest::Create { change } } if change.name == "Agent")`.

Påståendena i övrigt är oförändrade. Nya tester:

```rust
fn email(raw: &str) -> Email {
    Email::parse(raw).unwrap()
}

#[tokio::test]
async fn an_invitation_and_a_new_member_are_confirmed_exactly_as_asked() {
    let (_, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let company_id = uuid::Uuid::new_v4();

    for action in [
        Confirmation::Invitation { email: email("bo@example.se") },
        Confirmation::AddMember { company_id, email: email("bo@example.se") },
    ] {
        let (ceremony, options) = auth.begin_confirmation(anna.id, action.clone(), now()).await.unwrap();
        let assertion = annas.do_authentication(origin(), options).unwrap();
        let confirmed = auth.finish_confirmation(anna.id, ceremony, &assertion, now()).await.unwrap();
        assert_eq!(confirmed, action);
    }
}

#[tokio::test]
async fn an_invitation_is_refused_before_the_authenticator_is_asked() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now()).await.unwrap();
    let (bo, _) = sign_up(&auth, &mut authenticator(), "bo@example.se", Some(&invitation)).await;
    create_invitation(&pool, anna.id, "cecilia@example.se", now()).await.unwrap();

    let by_member = auth
        .begin_confirmation(bo.id, Confirmation::Invitation { email: email("dan@example.se") }, now())
        .await
        .unwrap_err();
    let registered = auth
        .begin_confirmation(anna.id, Confirmation::Invitation { email: email("bo@example.se") }, now())
        .await
        .unwrap_err();
    let invited = auth
        .begin_confirmation(anna.id, Confirmation::Invitation { email: email("cecilia@example.se") }, now())
        .await
        .unwrap_err();

    assert!(matches!(by_member, Error::Domain(DomainError::NotAdmin)), "{by_member:?}");
    assert!(matches!(registered, Error::AlreadyExists), "{registered:?}");
    assert!(matches!(invited, Error::AlreadyExists), "{invited:?}");
    assert_eq!(ceremonies(&pool).await, 0);
}

#[tokio::test]
async fn a_confirmation_is_stored_with_its_action() {
    let action = Confirmation::AddMember {
        company_id: uuid::Uuid::nil(),
        email: email("bo@example.se"),
    };
    let json = serde_json::to_value(&action).unwrap();
    assert_eq!(json["action"], "add_member");
    assert_eq!(serde_json::from_value::<Confirmation>(json).unwrap(), action);
}
```

(`ceremonies(pool)` och `create_invitation` finns redan i filen/importerna; `serde_json` är ett dev-beroende via identity — lägg till det i `[dev-dependencies]` om det saknas.)

- [ ] **Step 2: Kör och se dem fallera**

Run: `cargo test -p doris-identity --test webauthn`
Expected: kompileringsfel: `Confirmation`, `begin_confirmation`, `finish_confirmation` finns inte.

- [ ] **Step 3: Implementera**

`crates/identity/src/domain.rs`, efter `TokenRequest`:

```rust
/// What a passkey is asked to confirm: an action that gives someone access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Confirmation {
    ApiToken { request: TokenRequest },
    Invitation { email: Email },
    /// The company module checks membership; identity only carries it.
    AddMember { company_id: Uuid, email: Email },
}
```

`crates/identity/src/lib.rs`: bryt ut reglerna i `create_invitation` till en funktion som skriver i anroparens transaktion, och låt både `create_invitation` och en ny `check_invitation` använda den:

```rust
pub async fn create_invitation(
    pool: &SqlitePool,
    creator_id: Uuid,
    email: &str,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let created = invitation_in(&mut tx, creator_id, email, now).await?;
    tx.commit().await?;
    Ok(created)
}

/// Runs every rule for an invitation without saving it, so a passkey
/// ceremony can refuse before the authenticator is asked.
pub async fn check_invitation(
    pool: &SqlitePool,
    creator_id: Uuid,
    email: &str,
    now: Timestamp,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let checked = invitation_in(&mut tx, creator_id, email, now).await;
    tx.rollback().await?;
    checked.map(drop)
}

async fn invitation_in(
    conn: &mut SqliteConnection,
    creator_id: Uuid,
    email: &str,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    // the former body of create_invitation, from `let email = Email::parse(email)?;`
    // to the commit of the InvitationCreated event, using `conn` instead of `tx`
}
```

Behåll doc-kommentaren på `create_invitation`. Kroppen i `invitation_in` är exakt den gamla, med `&mut *conn`/`conn` i stället för `&mut *tx`/`&mut tx`.

`crates/identity/src/webauthn.rs`:
- Importera `check_invitation` från `crate` och `Confirmation` från `crate::domain`.
- Ersätt varianten `ApiToken { user_id, request, state }` med:

```rust
    /// Confirms exactly this action (a token, an invitation, a member).
    Confirm {
        user_id: Uuid,
        action: Confirmation,
        state: PasskeyAuthentication,
    },
```

- Ersätt `begin_api_token`/`finish_api_token` med:

```rust
    /// Checks identity's rules for the action, then asks the browser for an
    /// assertion from one of the user's own passkeys. Nothing changes until
    /// the caller carries out what [`Auth::finish_confirmation`] returns;
    /// company membership is the caller's to check.
    pub async fn begin_confirmation(
        &self,
        user_id: Uuid,
        action: Confirmation,
        now: Timestamp,
    ) -> Result<(Uuid, RequestChallengeResponse)> {
        match &action {
            Confirmation::ApiToken {
                request: TokenRequest::Create { change },
            } => check_new_api_token(&self.pool, user_id, change, now).await?,
            Confirmation::ApiToken {
                request: TokenRequest::Change { token_id, change },
            } => check_api_token_change(&self.pool, user_id, *token_id, change, now).await?,
            Confirmation::Invitation { email } => {
                check_invitation(&self.pool, user_id, email.as_str(), now).await?
            }
            Confirmation::AddMember { .. } => {}
        }
        let (options, state) = self.start_user_authentication(user_id).await?;
        let ceremony = Ceremony::Confirm {
            user_id,
            action,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the assertion and records the passkey's use. Returns the
    /// action the passkey confirmed. A ceremony another user began looks
    /// like one that doesn't exist.
    pub async fn finish_confirmation(
        &self,
        user_id: Uuid,
        ceremony_id: Uuid,
        credential: &PublicKeyCredential,
        now: Timestamp,
    ) -> Result<Confirmation> {
        let Ceremony::Confirm {
            user_id: owner,
            action,
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
        Ok(action)
    }
```

Uppdatera modulens doc-kommentar ("… confirming an API token request, an invitation or a new member").

`crates/server/src/grpc.rs`, så att servern kompilerar:
- importera `Confirmation` från `doris_identity::domain`;
- `begin_create_api_token` och `begin_change_api_token`: `.begin_api_token(user.id, REQUEST, now)` blir `.begin_confirmation(user.id, Confirmation::ApiToken { request: REQUEST }, now)`;
- `confirmed`: ersätt anropet till `finish_api_token` med

```rust
        let Confirmation::ApiToken { request } = self
            .auth
            .finish_confirmation(
                user_id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
```

  och behåll resten av funktionen (medlemskapskontrollen och `Ok(request)`).

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-identity && cargo test -p doris-server --test api_tokens && cargo clippy -p doris-identity -p doris-server --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/identity crates/server/src/grpc.rs
git commit -m "Confirm any access-granting action with the same passkey ceremony"
```

---

### Task 2: Inbjudan och medlem över API:et och i webben

**Files:**
- Modify: `proto/doris/auth/v1/auth.proto`, `proto/doris/company/v1/company.proto`
- Modify: `crates/server/src/grpc.rs`, `crates/server/src/company.rs`, `crates/server/src/main.rs`, `crates/server/src/access.rs`
- Modify: `crates/server/tests/common/mod.rs` och alla servertester som bjuder in eller lägger till medlemmar (`grpc.rs`, `companies.rs`, `ledger.rs`, `api_tokens.rs`, `payroll.rs`, `invoicing.rs`, `supplier_invoices.rs`, `customer_invoices.rs`, `vat.rs` – sök efter `invite`, `create_invitation`, `add_member`)
- Create: `crates/server/tests/confirmations.rs`
- Modify: `crates/web/src/pages/invitations.rs`, `crates/web/src/pages/company.rs`, `crates/web/src/pages/api_tokens.rs`

**Interfaces:**
- Consumes: Task 1:s `Confirmation`, `Auth::{begin_confirmation, finish_confirmation}`.
- Produces:
  - auth: `BeginCreateInvitation(CreateInvitationRequest) returns (BeginCeremonyResponse)`, `FinishCreateInvitation(FinishConfirmationRequest) returns (CreateInvitationResponse)`; `FinishApiTokenRequest` heter nu `FinishConfirmationRequest` (samma fält); `CreateInvitation` tas bort.
  - company: `BeginAddMember(AddMemberRequest) returns (BeginAddMemberResponse)`, `FinishAddMember(FinishAddMemberRequest) returns (AddMemberResponse)`; `AddMember` tas bort.
  - `AuthApi::new(pool, auth: Arc<Auth>)`, `CompanyApi::new(pool, bolagsverket, auth: Arc<Auth>)`.
  - Testhjälpare: `TestServer::create_invitation(admin, admin_device, email) -> Result<pb::CreateInvitationResponse, Status>`, `invite(admin, admin_device, email)`, `invite_as(admin, admin_device, email, name)`, `invite_with(admin, admin_device, email, device)`, `add_member(session, device, company_id, email) -> Result<(), Status>`, `confirm_options(device, options_json) -> String` (och `confirm` byggd på den).

- [ ] **Step 1: Proto**

`auth.proto`: ersätt `rpc CreateInvitation(…)` med

```proto
  // An admin invites someone, confirmed with one of the admin's passkeys.
  rpc BeginCreateInvitation(CreateInvitationRequest) returns (BeginCeremonyResponse);
  rpc FinishCreateInvitation(FinishConfirmationRequest) returns (CreateInvitationResponse);
```

och byt namn på `message FinishApiTokenRequest` till `message FinishConfirmationRequest` (även i token-RPC:ernas signaturer), med kommentaren `// Finishes any passkey confirmation: the ceremony and the assertion.`

`company.proto`: ersätt `rpc AddMember(…)` med

```proto
  // A member adds someone, confirmed with one of the member's passkeys.
  rpc BeginAddMember(AddMemberRequest) returns (BeginAddMemberResponse);
  rpc FinishAddMember(FinishAddMemberRequest) returns (AddMemberResponse);
```

och lägg till

```proto
// WebAuthn request options for the member's passkey (as in doris.auth.v1).
message BeginAddMemberResponse {
  string ceremony_id = 1;
  string options_json = 2;
}

message FinishAddMemberRequest {
  string ceremony_id = 1;
  string credential_json = 2;
}
```

`access.rs`: i auth-armen för `SessionOnly`, ersätt `"CreateInvitation"` med `"BeginCreateInvitation" | "FinishCreateInvitation"`; i company-armen, ersätt `"AddMember"` med `"BeginAddMember" | "FinishAddMember"`. I `the_table_follows_the_spec`: `("/doris.company.v1.CompanyService/BeginAddMember", SessionOnly)` i stället för `AddMember`, och lägg till `("/doris.auth.v1.AuthService/FinishCreateInvitation", SessionOnly)`.

- [ ] **Step 2: Testhjälpare och testflytt**

`crates/server/tests/common/mod.rs`:
- `launch`: `let auth = std::sync::Arc::new(Auth::new(…).await.unwrap());`, `AuthApi::new(pool.clone(), auth.clone())`, `CompanyApi::new(pool.clone(), bolagsverket, auth)`.
- Gör om `confirm`:

```rust
    /// `device`'s assertion for WebAuthn request options, as the browser
    /// gives it after the user touches the passkey.
    pub fn confirm_options(&self, device: &mut Device, options_json: &str) -> String {
        let options: RequestChallengeResponse = serde_json::from_str(options_json).unwrap();
        let credential = device
            .do_authentication(self.origin.clone(), options)
            .unwrap();
        serde_json::to_string(&credential).unwrap()
    }

    pub fn confirm(&self, device: &mut Device, begin: &pb::BeginCeremonyResponse) -> String {
        self.confirm_options(device, &begin.options_json)
    }
```

- Nya och ändrade hjälpare:

```rust
    /// An admin invites `email`, confirming with `admin_device`'s passkey.
    pub async fn create_invitation(
        &self,
        admin: &str,
        admin_device: &mut Device,
        email: &str,
    ) -> Result<pb::CreateInvitationResponse, tonic::Status> {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_create_invitation(authed(
                pb::CreateInvitationRequest {
                    email: email.into(),
                },
                admin,
            ))
            .await?
            .into_inner();
        let finish = pb::FinishConfirmationRequest {
            credential_json: self.confirm(admin_device, &begin),
            ceremony_id: begin.ceremony_id,
        };
        Ok(grpc
            .finish_create_invitation(authed(finish, admin))
            .await?
            .into_inner())
    }

    /// An admin invites `email`, who registers; returns the new user's session.
    pub async fn invite(&self, admin: &str, admin_device: &mut Device, email: &str) -> String {
        let invite = self.create_invitation(admin, admin_device, email).await.unwrap();
        self.sign_up(&mut device(), email, Some(&invite.token)).await
    }

    /// Like `invite`, for a test that tells people apart by name.
    pub async fn invite_as(&self, admin: &str, admin_device: &mut Device, email: &str, name: &str) -> String {
        let invite = self.create_invitation(admin, admin_device, email).await.unwrap();
        self.sign_up_as(&mut device(), email, name, Some(&invite.token)).await
    }

    /// Like `invite`, keeping the new user's passkey on `device`.
    pub async fn invite_with(&self, admin: &str, admin_device: &mut Device, email: &str, device: &mut Device) -> String {
        let invite = self.create_invitation(admin, admin_device, email).await.unwrap();
        self.sign_up(device, email, Some(&invite.token)).await
    }

    /// A member adds `email` to a company, confirming with `device`'s passkey.
    pub async fn add_member(
        &self,
        session: &str,
        device: &mut Device,
        company_id: &str,
        email: &str,
    ) -> Result<(), tonic::Status> {
        use doris_proto::company::v1 as cpb;
        let mut api = self.companies();
        let begin = api
            .begin_add_member(authed(
                cpb::AddMemberRequest {
                    company_id: company_id.into(),
                    email: email.into(),
                },
                session,
            ))
            .await?
            .into_inner();
        let finish = cpb::FinishAddMemberRequest {
            credential_json: self.confirm_options(device, &begin.options_json),
            ceremony_id: begin.ceremony_id,
        };
        api.finish_add_member(authed(finish, session)).await?;
        Ok(())
    }
```

Flytta testerna mekaniskt (sök efter `.invite(`, `.invite_as(`, `.invite_with(`, `create_invitation(`, `add_member(` i `crates/server/tests/`):
- Den som bjuder in eller lägger till behöver sin enhet: `let anna = server.sign_up(&mut device(), …)` blir `let mut annas = device(); let anna = server.sign_up(&mut annas, …)`, och `server.invite(&anna, EMAIL)` blir `server.invite(&anna, &mut annas, EMAIL)` (samma för `invite_as`, `invite_with`).
- `server.grpc().create_invitation(authed(REQ, &admin)).await.unwrap().into_inner()` blir `server.create_invitation(&admin, &mut admins, EMAIL).await.unwrap()`; ett förväntat fel (`not_admin` för en medlem) testas med `server.grpc().begin_create_invitation(authed(REQ, &member))`, som fallerar redan vid begin.
- `api.add_member(authed(REQ, &anna)).await.unwrap()` blir `server.add_member(&anna, &mut annas, &id, EMAIL).await.unwrap()`. Fel som kommer redan vid begin (`not_signed_in` utan session, `company_not_found` för en icke-medlem, `user_not_found` för en okänd e-post) testas med `api.begin_add_member(…)` direkt, utan enhet.
- `api_tokens.rs`: `auth.create_invitation(bearer(…))` blir `auth.begin_create_invitation(bearer(…))`; `pb::FinishApiTokenRequest` blir `pb::FinishConfirmationRequest`.

Ändra inga påståenden.

- [ ] **Step 3: Skriv de nya fallerande testerna**

`crates/server/tests/confirmations.rs`:

```rust
mod common;

use common::{TestServer, authed, company, device};
use doris_proto::auth::v1 as pb;
use doris_proto::company::v1 as cpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

async fn ceremonies(server: &TestServer) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&server.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_invitation_is_refused_at_begin_before_any_passkey() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let mut auth = server.grpc();
    let begin = |email: &str, session: &str| {
        let mut auth = server.grpc();
        let request = authed(pb::CreateInvitationRequest { email: email.into() }, session);
        async move { auth.begin_create_invitation(request).await.unwrap_err() }
    };

    let errors = [
        (begin("cecilia@example.se", &bo).await, (Code::PermissionDenied, "not_admin")),
        (begin("inte en adress", &anna).await, (Code::InvalidArgument, "invalid_email")),
        (begin("bo@example.se", &anna).await, (Code::AlreadyExists, "already_exists")),
    ];

    for (err, (code, message)) in errors {
        assert_eq!(code_of(err), (code, message.to_owned()));
    }
    assert_eq!(ceremonies(&server).await, 0);
    let _ = &mut auth;
}

#[tokio::test]
async fn a_confirmation_of_one_kind_cannot_finish_another() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut auth = server.grpc();
    let mut companies = server.companies();

    // An invitation ceremony finished as a new member…
    let begin = auth
        .begin_create_invitation(authed(pb::CreateInvitationRequest { email: "bo@example.se".into() }, &anna))
        .await
        .unwrap()
        .into_inner();
    let as_member = companies
        .finish_add_member(authed(
            cpb::FinishAddMemberRequest {
                credential_json: server.confirm(&mut annas, &begin),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    // …a member ceremony finished as an invitation…
    server.invite(&anna, &mut annas, "bo@example.se").await;
    let begin = companies
        .begin_add_member(authed(cpb::AddMemberRequest { company_id: id.clone(), email: "bo@example.se".into() }, &anna))
        .await
        .unwrap()
        .into_inner();
    let as_invitation = auth
        .finish_create_invitation(authed(
            pb::FinishConfirmationRequest {
                credential_json: server.confirm_options(&mut annas, &begin.options_json),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    // …and a token ceremony finished as an invitation.
    let begin = auth
        .begin_create_api_token(authed(
            pb::CreateApiTokenRequest {
                name: "Agent".into(),
                expires_on: common::in_days(30),
                grants: vec![pb::TokenGrant { company_id: id.clone(), scopes: vec!["ledger:read".into()] }],
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let token_finished_as_invitation = auth
        .finish_create_invitation(authed(
            pb::FinishConfirmationRequest {
                credential_json: server.confirm(&mut annas, &begin),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    for err in [as_member, as_invitation, token_finished_as_invitation] {
        assert_eq!(code_of(err), (Code::FailedPrecondition, "ceremony_expired".into()));
    }
    let members = companies
        .list_members(authed(cpb::ListMembersRequest { company_id: id }, &anna))
        .await
        .unwrap()
        .into_inner()
        .members;
    assert_eq!(members.len(), 1, "nobody was added");
}

#[tokio::test]
async fn an_invitation_ceremony_is_finished_once() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_create_invitation(authed(pb::CreateInvitationRequest { email: "bo@example.se".into() }, &anna))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishConfirmationRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };

    let first = auth.finish_create_invitation(authed(finish.clone(), &anna)).await.unwrap().into_inner();
    let again = auth.finish_create_invitation(authed(finish, &anna)).await.unwrap_err();

    assert!(!first.token.is_empty());
    assert_eq!(code_of(again), (Code::FailedPrecondition, "ceremony_expired".into()));
    let listed = auth
        .list_invitations(authed(pb::ListInvitationsRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .invitations;
    assert_eq!(listed.len(), 1);
}
```

Om stängningen `begin` i det första testet krånglar med lånen, skriv de tre anropen direkt; `let _ = &mut auth;` kan då tas bort.

- [ ] **Step 4: Kör och se dem fallera**

Run: `cargo test -p doris-server --test confirmations`
Expected: kompileringsfel (proto ändrad, handlers saknas).

- [ ] **Step 5: Implementera i servern**

`crates/server/src/main.rs`: `let auth = std::sync::Arc::new(Auth::new(…).await?);` (behåll felhanteringen som finns), `AuthApi::new(pool.clone(), auth.clone())`, `CompanyApi::new(pool.clone(), bolagsverket, auth)`.

`crates/server/src/grpc.rs`:
- `AuthApi { pool, auth: Arc<Auth> }`, `AuthApi::new(pool: SqlitePool, auth: Arc<Auth>)`.
- Gemensamma hjälpare (fria funktioner), och använd dem i `confirmed`:

```rust
/// The action a passkey just confirmed, for any finish RPC.
pub(crate) async fn confirmation(
    auth: &Auth,
    user_id: Uuid,
    raw_ceremony_id: &str,
    credential_json: &str,
) -> Result<Confirmation, Status> {
    auth.finish_confirmation(
        user_id,
        ceremony_id(raw_ceremony_id)?,
        &credential(credential_json)?,
        Timestamp::now(),
    )
    .await
    .map_err(finish_status)
}

/// A confirmation of another kind than the finish RPC carries out.
pub(crate) fn ceremony_expired() -> Status {
    Status::failed_precondition("ceremony_expired")
}
```

- `confirmed` blir:

```rust
        let Confirmation::ApiToken { request } =
            confirmation(&self.auth, user_id, &req.ceremony_id, &req.credential_json).await?
        else {
            return Err(ceremony_expired());
        };
```

  (resten oförändrad), och `pb::FinishApiTokenRequest` blir `pb::FinishConfirmationRequest` i signaturerna.
- Ta bort `create_invitation`-handlern och lägg till:

```rust
    async fn begin_create_invitation(
        &self,
        request: Request<pb::CreateInvitationRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let admin = self.admin(&request).await?;
        let email = Email::parse(&request.get_ref().email).map_err(|e| status(Error::Domain(e)))?;
        let (ceremony_id, options) = self
            .auth
            .begin_confirmation(admin.id, Confirmation::Invitation { email }, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_create_invitation(
        &self,
        request: Request<pb::FinishConfirmationRequest>,
    ) -> Result<Response<pb::CreateInvitationResponse>, Status> {
        let admin = self.admin(&request).await?;
        let req = request.get_ref();
        let Confirmation::Invitation { email } =
            confirmation(&self.auth, admin.id, &req.ceremony_id, &req.credential_json).await?
        else {
            return Err(ceremony_expired());
        };
        let now = Timestamp::now();
        let (_, token) = doris_identity::create_invitation(&self.pool, admin.id, email.as_str(), now)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::CreateInvitationResponse {
            token,
            expires_at: (now + doris_identity::domain::INVITATION_TTL).to_string(),
        }))
    }
```

  Importera `Email` från `doris_identity::domain`.

`crates/server/src/company.rs`:
- `CompanyApi { pool, bolagsverket, auth: Arc<doris_identity::Auth> }`, `new(pool, bolagsverket, auth)`.
- Ta bort `add_member`-handlern och lägg till:

```rust
    async fn begin_add_member(
        &self,
        request: Request<pb::AddMemberRequest>,
    ) -> Result<Response<pb::BeginAddMemberResponse>, Status> {
        // Access first, so a non-member can't probe which emails exist.
        let (company, actor) = self
            .member_company(&request, &request.get_ref().company_id)
            .await?;
        let member = doris_identity::find_user_by_email(&self.pool, &request.get_ref().email)
            .await
            .map_err(grpc::status)?
            .ok_or_else(|| Status::not_found("user_not_found"))?;
        let action = Confirmation::AddMember {
            company_id: company.id,
            email: member.email,
        };
        let (ceremony_id, options) = self
            .auth
            .begin_confirmation(actor, action, jiff::Timestamp::now())
            .await
            .map_err(grpc::status)?;
        Ok(Response::new(pb::BeginAddMemberResponse {
            ceremony_id: ceremony_id.to_string(),
            options_json: serde_json::to_string(&options)
                .map_err(|_| Status::internal("internal"))?,
        }))
    }

    async fn finish_add_member(
        &self,
        request: Request<pb::FinishAddMemberRequest>,
    ) -> Result<Response<pb::AddMemberResponse>, Status> {
        let user = signed_in_user(&self.pool, &request).await?;
        let req = request.get_ref();
        let Confirmation::AddMember { company_id, email } =
            grpc::confirmation(&self.auth, user.id, &req.ceremony_id, &req.credential_json).await?
        else {
            return Err(grpc::ceremony_expired());
        };
        let member = doris_identity::find_user_by_email(&self.pool, email.as_str())
            .await
            .map_err(grpc::status)?
            .ok_or_else(|| Status::not_found("user_not_found"))?;
        // add_member checks again that the user is a member.
        doris_company::add_member(&self.pool, company_id, user.id, member.id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::AddMemberResponse {}))
    }
```

  Importera `doris_identity::domain::Confirmation`.

- [ ] **Step 6: Webben**

`crates/web/src/pages/api_tokens.rs`: `pb::FinishApiTokenRequest` blir `pb::FinishConfirmationRequest`.

`crates/web/src/pages/invitations.rs`: lägg till

```rust
/// Begins the invitation, has one of the admin's passkeys confirm it and
/// finishes; returns the invitation's token.
async fn invite_with_passkey(email: String) -> Result<String, String> {
    let mut api = api();
    let begin = api
        .begin_create_invitation(pb::CreateInvitationRequest { email })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::get(&begin.options_json).await?;
    let finish = pb::FinishConfirmationRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    };
    Ok(api
        .finish_create_invitation(finish)
        .await
        .map_err(|s| describe(&s))?
        .into_inner()
        .token)
}
```

och i `submit`, ersätt `match api().create_invitation(request).await { Ok(created) => { … let token = created.into_inner().token; … } Err(status) => error.set(Some(describe(&status))) }` med `match invite_with_passkey(email.get_untracked()).await { Ok(token) => { … } Err(message) => error.set(Some(message)) }` (behåll länkbygget, rensningen av fältet och `refresh()`). Kortets beskrivning: `"Länken gäller i 7 dagar och kan användas en gång. Du bekräftar med din passkey."`. Importera `crate::passkey`.

`crates/web/src/pages/company.rs`: lägg till

```rust
/// Begins adding the member, has one of the user's passkeys confirm it and
/// finishes.
async fn add_with_passkey(company_id: String, email: String) -> Result<(), String> {
    let mut api = company_api();
    let begin = api
        .begin_add_member(cpb::AddMemberRequest { company_id, email })
        .await
        .map_err(|s| describe(&s))?
        .into_inner();
    let credential_json = passkey::get(&begin.options_json).await?;
    api.finish_add_member(cpb::FinishAddMemberRequest {
        ceremony_id: begin.ceremony_id,
        credential_json,
    })
    .await
    .map_err(|s| describe(&s))?;
    Ok(())
}
```

och i `add`, ersätt `match company_api().add_member(request).await { Ok(_) => …, Err(status) => member_error.set(Some(describe(&status))) }` med `match add_with_passkey(id(), email.get_untracked()).await { Ok(()) => …, Err(message) => member_error.set(Some(message)) }`. Kortet "Medlemmar": `description="De som har tillgång till företaget. Du bekräftar med din passkey när du lägger till någon."`. Importera `crate::passkey`. `busy` sätts `false` efter `match` som förut, även när passkey-steget avbryts.

- [ ] **Step 7: Kör testerna**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo fmt --all --check && make web`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add proto crates
git commit -m "Confirm invitations and new members with a passkey"
```

---

### Task 3: E2E och AGENTS.md

**Files:**
- Modify (vid behov): `e2e/tests/auth.spec.ts`, `e2e/tests/companies.spec.ts`
- Modify: `AGENTS.md`

- [ ] **Step 1: Kör de berörda e2e-testerna**

Run: `cargo build -p doris-server && cd e2e && npx playwright test auth.spec.ts companies.spec.ts tokens.spec.ts --workers=1`
Expected: PASS utan ändrade selektorer; den virtuella autentiseraren svarar på bekräftelsen. Om ett test där samma sida har två autentiserare (som "a user adds a second passkey") påverkas, följ mönstret med `setPresence` från `fixtures.ts`. Ändra inte appen för testets skull; fallerar ett test på grund av appen, rapportera det.

- [ ] **Step 2: AGENTS.md**

Under `## Authentication`, efter stycket om token-ceremonin:

```markdown
- Inviting someone (`BeginCreateInvitation`/`FinishCreateInvitation`) and
  adding a member to a company (`BeginAddMember`/`FinishAddMember`) are
  confirmed with one of the user's passkeys too, through the same
  ceremony (`Ceremony::Confirm`, kind `confirm`,
  `Auth::begin_confirmation`/`finish_confirmation`), so a stolen session
  cannot give a second account of its own lasting access. Begin runs the
  rules; finish carries out what was confirmed and checks again.
```

Uppdatera stycket om token-ceremonin (det nämner `Auth::begin_api_token`/`finish_api_token` och sorten `api_token`) till `begin_confirmation`/`finish_confirmation` och sorten `confirm`. Under `## API`: ersätt `CreateInvitation` och `AddMember` med de nya paren och nämn `FinishConfirmationRequest`.

- [ ] **Step 3: Kör hela sviten och committa**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo fmt --all --check`
Expected: PASS.

```bash
git add AGENTS.md e2e/tests
git commit -m "Document confirming invitations and new members with a passkey"
```
