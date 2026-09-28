# Doris – Steg 1: Grundarkitektur + registrering/inloggning med WebAuthn

## Kontext
Doris ska bli ett bokföringsprogram som följer Bokföringslagen (SFS 1999:1078) och ska vara så self-contained, lättviktigt och snabbt som möjligt. Stacken är Rust-backend, Leptos-frontend, SQLite, event sourcing som källa till sanningen med projektioner för läsning, och gRPC mellan klient och server. Repot är tomt (bara `AGENTS.md`). Steg 1 lägger grundarkitekturen och levererar den första featuren: **flera användare och autentisering enbart med passkeys (WebAuthn)**. Inga lösenord lagras.

Arbetet följer strikt TDD: ingen funktionalitet skrivs utan ett felande test först (red → green → refactor).

### Fattade beslut
| Område | Beslut |
|---|---|
| Registrering | Den första användaren registrerar sig fritt och blir **admin**. Därefter krävs en **inbjudan** som bara admin kan skapa. Inbjudan är **knuten till en e-postadress**. Länken visas i UI:t, kopieras manuellt, gäller i 7 dagar och kan användas en gång. |
| Identitet | Unik **e-postadress** plus ett **visningsnamn** (1–100 tecken). E-posten normaliseras (trim, gemener, max 254 tecken, exakt ett `@`, domän med punkt). Ingen verifiering eller utskick i steg 1: inbjudan binder e-posten, och bootstrap-användaren litas på. E-post är personuppgift, så den läggs aldrig i loggar. |
| Återställning | En användare kan ha **flera passkeys** och kan lägga till fler när hen är inloggad. Admin-återställning kommer i ett senare steg. |
| Session | Opakt token i en cookie med `HttpOnly; Secure; SameSite=Strict`. Servern sparar bara en SHA-256-hash av token. |
| Frontend | Leptos **CSR**. Tillgångarna **bäddas in i serverbinären** och byggs dessutom som ett **separat assets-paket** för CDN/nginx. Den inbäddade servningen kan stängas av i konfigurationen. |
| E2E | Playwright med Chromes virtuella WebAuthn-authenticator. Node behövs bara som dev-beroende. |
| Stil | shadcn-presetet `b1Gdz9bFY` avkodat: stil *mira*, bas *stone*, tema *amber*, font *Inter* (hostas själv), radie *small*, ikoner *lucide*. |
| Språk | Gränssnittet på svenska, koden på engelska. |

### Versioner (kontrollerade 2026-09-27)
- Leptos 0.8.x (`csr`), Trunk 0.21.x med Tailwind v4 fastlåst i `Trunk.toml` (standalone-CLI, ingen Node)
- tonic / tonic-web / tonic-prost / tonic-prost-build 0.14.x, prost 0.14.x, tonic-web-wasm-client 0.9.x
- webauthn-rs 0.5.5 (feature `danger-allow-state-serialisation`), webauthn-rs-proto (feature `wasm`), webauthn-authenticator-rs som test-authenticator
- sqlx 0.9 (`sqlite`, `runtime-tokio`, `migrate`)
- rust-embed (`allow_missing`), axum (via tonic)
- protoc finns redan i `/opt/homebrew/bin`

## Arkitektur

```
Cargo.toml              (workspace)
migrations/             (sqlx-migreringar, delas av alla crates)
proto/doris/auth/v1/auth.proto
crates/
  eventstore/  doris-eventstore – append-only-händelselogg i SQLite, optimistisk samtidighet, öppnar DB och kör migreringar
  identity/    doris-identity   – domän (User, Invitation), händelser, projektioner, WebAuthn-ceremonier, sessioner
  proto/       doris-proto      – genererad kod från proto/; feature `server` ger serverstubbar, klienten fungerar på wasm32
  server/      doris-server     – binär: tonic + GrpcWebLayer + cookie-sessioner + inbäddade tillgångar (axum-fallback), konfiguration
  web/         doris-web        – Leptos CSR (Trunk, Tailwind v4, UI-komponenter i shadcn-stil)
e2e/                    (Playwright)
Makefile                (dev, test, e2e, dist)
```
Inga fler crates än så här. En separat `ui`-crate skapas först när en andra frontend behöver den.

### Event store (`doris-eventstore`, `migrations/0001_events.sql`)
- Tabellen `events` har kolumnerna `global_position INTEGER PRIMARY KEY AUTOINCREMENT`, `stream_id`, `stream_version`, `event_type`, `schema_version`, `payload` (JSON), `metadata` (JSON: aktör, korrelations-id) och `recorded_at` (UTC).
  - `UNIQUE(stream_id, stream_version)` ger optimistisk samtidighet.
- Triggers `BEFORE UPDATE` och `BEFORE DELETE` gör `RAISE(ABORT)`. Loggen är därmed append-only på databasnivå, i linje med kravet på varaktighet och behandlingshistorik i BFL (5 kap. 11 §, 7 kap.).
- Payload sparas som **JSON** och inte som protobuf. Formatet ska vara läsbart och tolkningsbart under hela arkiveringstiden.
- API:
  - `append(tx, stream, expected_version, events, metadata)`
  - `load(stream)`
  - `read_all(from_position)`, som används för att bygga om projektioner
- Projektionerna uppdateras **synkront i samma transaktion** som append (`BEGIN IMMEDIATE`). Därför finns ingen eventual consistency och ingen bakgrundsprocess.
- SQLite-pragman: `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`.

### Identitetsdomänen (`doris-identity`)
**Strömmar och händelser**

| Ström | Händelser |
|---|---|
| `user-{uuid}` | `UserRegistered{email, display_name, role, invitation_id?}`, `PasskeyAdded{credential_id, name, passkey_json}`, `PasskeyUsed{credential_id, counter}` (vid inloggning; uppdaterar räknaren och fungerar som inloggningslogg) |
| `invitation-{uuid}` | `InvitationCreated{email, token_hash, created_by, expires_at}`, `InvitationAccepted{user_id}` |

- Domänlogiken är rena funktioner: `decide(state, command) -> Result<Vec<Event>>` och `evolve(state, event)`. De testas enligt given/when/then utan databas.
- **Mängdregler** som unik e-post och "första användaren blir admin" hanteras i skrivtransaktionen. Projektionen `users` har `UNIQUE(email)`, och bootstrap kontrollerar att `users` är tom inom samma `BEGIN IMMEDIATE`.
- **Projektionstabeller** (`migrations/0002_identity.sql`):
  - `users`
  - `passkeys` (credential_id, user_id, namn, passkey-JSON, senast använd)
  - `invitations`
  - `projection_checkpoints`
  - `rebuild_projections()` tömmer tabellerna och spelar upp `read_all`.
- **Operativ, flyktig data** som inte är händelser: `webauthn_ceremonies` (id, typ, serialiserad state, utgångstid; engångs, 5 min) och `sessions` (token_hash, user_id, utgångstid). Det här är inte bokföringsdata och får rensas.
- **Tjänstelagret `IdentityService`**:
  - `begin/finish_registration`
  - `begin/finish_login`
  - `begin/finish_add_passkey`
  - `create_invitation`
  - `list_passkeys`
  - `status`
  - `session_user(token)`
  - `logout`

### gRPC (`proto/doris/auth/v1/auth.proto`)
```
service AuthService {
  GetStatus            -> { bootstrap_required, optional current_user }
  BeginRegistration    { email, display_name, optional invitation_token }    -> { ceremony_id, options_json }
  FinishRegistration   { ceremony_id, credential_json, passkey_name }        -> User   (+ set-cookie)
  BeginLogin           { email }                                             -> { ceremony_id, options_json }
  FinishLogin          { ceremony_id, credential_json }                      -> User   (+ set-cookie)
  Logout                                                                      (rensar cookie)
  BeginAddPasskey / FinishAddPasskey / ListPasskeys                          (kräver session)
  GetInvitation        { token } -> { email }                                (för att förifylla registreringen)
  CreateInvitation     { email } -> { token, expires_at } / ListInvitations  (kräver admin)
}
```
- WebAuthn-options och -svar skickas som JSON-strängar. `webauthn-rs-proto` med feature `wasm` konverterar dem till och från `web_sys`-typer.
- Servern använder tonic med `accept_http1(true)` och `GrpcWebLayer`. En interceptor läser cookien `doris_session` och lägger användaren i request extensions.
- Felhantering:
  - `Unauthenticated` när sessionen saknas
  - `PermissionDenied` när användaren inte är admin
  - `InvalidArgument` vid valideringsfel
  - `AlreadyExists` när e-posten redan är registrerad eller redan har en aktiv inbjudan
- `BeginLogin` avslöjar inte om e-posten finns. Svaret blir ett generiskt fel först vid `FinishLogin`.
- CSRF: `SameSite=Strict` kombinerat med gRPC-Webs content-type, som alltid utlöser en CORS-preflight. CORS släpper bara igenom origins som är konfigurerade.

### Server och paketering (`doris-server`)
- Konfiguration via clap och env:
  - `DORIS_DATABASE`
  - `DORIS_LISTEN`
  - `DORIS_RP_ID`
  - `DORIS_RP_ORIGIN`
  - `DORIS_CORS_ORIGINS`
  - `DORIS_SERVE_FRONTEND=true|false`
- En port betjänar allt. gRPC-routrarna görs om till en axum-router, och `rust-embed` (`crates/web/dist`) står för fallback. SPA-fallback skickar okända sökvägar till `index.html`, och hashade tillgångar får långa cache-headers.
- Under utveckling kör `trunk serve` en proxy för `/doris.` till backend. Allt blir då same-origin och cookies fungerar direkt på `localhost`.
- `make dist` bygger följande i ordning:
  1. `trunk build --release`
  2. `cargo build --release -p doris-server`
  3. `target/dist/doris` (binär med inbäddade tillgångar) och `target/dist/doris-web-<version>.tar.gz` (samma tillgångar för CDN/nginx)

### Frontend (`doris-web`)
- **Designtokens:** CSS-variabler för mira/stone/amber tas fram en gång med `npx shadcn init --preset b1Gdz9bFY` i en scratch-katalog och kopieras in i `crates/web/style/input.css` (Tailwind v4 `@theme`). Inter läggs in som woff2 i repot, och ljust/mörkt tema styrs av `prefers-color-scheme`.
- **Komponenter** (bara de som behövs): `Button`, `Input`, `Label`, `Card`, `Alert`, `Field`. Lucide-ikoner läggs in som inline-SVG, bara de som används.
- **Sidor:**

  | Sökväg | Innehåll |
  |---|---|
  | `/register` | Bootstrap, eller `?invitation=<token>` (e-posten förifylls och är låst) |
  | `/login` | Inloggning |
  | `/` | Startsida: inloggad som X, logga ut |
  | `/settings/passkeys` | Lista och lägg till passkeys |
  | `/admin/invitations` | Skapa och lista inbjudningar; bara admin |

- URL:er, query-parametrar och gRPC-namn är alltid på engelska. Bara synlig text i gränssnittet är på svenska.
- `GetStatus` styr routingen: bootstrap krävs → `/register`, ingen session → `/login`.
- Formulärvalidering görs med HTML-attribut (`type="email"`, `required`, `autocomplete="username webauthn"`). Servern har sista ordet och dess fel visas i en `Alert`.

## Genomförande (TDD, varje punkt = röda tester → grönt → refactor → commit)
0. **Uppstart:**
   - Skriv designen till `docs/superpowers/specs/2026-09-27-inloggning-webauthn-design.md` och en detaljerad plan per uppgift med skillen writing-plans.
   - Sätt upp workspace-skelett, `Makefile` och `.gitignore`.
   - Fyll `AGENTS.md` med det som varje framtida session behöver veta:
     - **Syfte:** bokföringsprogram enligt BFL (SFS 1999:1078), med länk. Features byggs i delsteg.
     - **Principer:** self-contained, lättviktigt och snabbt. Inga externa tjänster eller CDN-beroenden vid körning, och inga nya beroenden utan skäl.
     - **TDD är obligatoriskt:** först ett felande test, sedan implementation, sedan refactor. Ingen kod utan ett rött test före.
     - **Stacken och motiveringen:** Rust, Leptos CSR, SQLite, gRPC-Web och event sourcing.
     - **Katalogstruktur:** `crates/`, `migrations/`, `proto/`, `e2e/`, plus vad varje crate ansvarar för.
     - **Event sourcing-regler:**
       - Händelser är append-only (trigger) och sparas som JSON med `schema_version`. De ändras aldrig; en ny version läggs till.
       - Projektioner uppdateras i samma transaktion och ska gå att bygga om.
       - Operativ data (sessioner, ceremonier) är inte händelser.
     - **BFL-relevanta krav:** varaktighet, behandlingshistorik (5 kap. 11 §), arkivering i 7 år i läsbar form.
     - **Namngivning:** kod, URL:er, query-parametrar, proto och händelsenamn på engelska. Bara UI-text på svenska.
     - **Auth:** endast WebAuthn/passkeys och aldrig lösenord. Sessionscookies är HttpOnly, Secure och SameSite=Strict, och bara token-hashen lagras.
     - **Stil:** shadcn-presetet `b1Gdz9bFY` (mira/stone/amber, Inter, small radius, lucide). Tokens finns i `crates/web/style/input.css`.
     - **Kommandon:** `make dev`, `make test`, `make e2e`, `make dist`, samt de viktigaste `DORIS_*`-variablerna.
   - Inga funktioner ännu. Målet är bara att `cargo test` körs.
1. **Event store:** append+load, versionskonflikt → fel, `UPDATE`/`DELETE` på `events` misslyckas, `read_all` i ordning, migreringar körs på `sqlite::memory:`.
2. **Domän (rent):**
   - Normalisering och validering av e-post och visningsnamn
   - Bootstrap-användaren blir admin
   - Registrering utan inbjudan efter bootstrap nekas
   - Utgången eller redan använd inbjudan nekas
   - Registrering med en annan e-post än inbjudans nekas
   - Dubblett-credential nekas
3. **Projektioner:**
   - Registrering syns i `users`/`passkeys`
   - Upptagen e-post, även med annan skiftläge, → `AlreadyExists` och ingen händelse skrivs
   - Samtidig bootstrap ger exakt en admin
   - `rebuild_projections()` ger samma tillstånd som den inkrementella uppdateringen
4. **WebAuthn-ceremonier** (med `webauthn-authenticator-rs` SoftPasskey):
   - Registrering och inloggning fungerar
   - Ceremonin är engångs och går ut
   - Fel credential nekas
   - `PasskeyUsed` uppdaterar räknaren
   - Andra passkey kan läggas till och båda kan logga in
5. **Sessioner:** skapas vid registrering/inloggning, bara hashen lagras, utgången session nekas, logout invaliderar.
6. **Inbjudningar:** bara admin kan skapa, bunden till e-post (redan registrerad e-post → `AlreadyExists`), token lagras hashad, engångs, 7 dagar.
7. **gRPC-server** (integrationstester med tonic-klient mot server på en efemär port):
   - Hela flödet med SoftPasskey
   - `Set-Cookie` har rätt attribut
   - Anrop utan session → `Unauthenticated`
   - Anrop som inte-admin → `PermissionDenied`
   - Ett anrop med gRPC-Web-content-type lyckas
   - CORS-preflight godkänns bara för konfigurerade origins
8. **Statiska tillgångar:** `/` → `index.html`, okänd sökväg → SPA-fallback, cache-headers, `DORIS_SERVE_FRONTEND=false` → 404.
9. **Frontend + e2e** (Playwright-specar skrivs röda först, en per flöde):
   - Bootstrap-registrering → inloggad som admin
   - Logga ut → logga in
   - Admin skapar inbjudan → ny användare registrerar sig via länken i en separat browser context
   - Lägg till en andra passkey och logga in med den
   - Vanlig användare ser inte `/admin/invitations`
10. **Paketering:** ett smoke-test för `make dist` startar binären mot en tom databas och kontrollerar att `GET /` och `GetStatus` svarar, och att tarballen innehåller `index.html` och wasm-filen.

### Risker att verifiera tidigt (i punkt 7)
- Går `set-cookie` i tonics response-metadata fram till webbläsaren via gRPC-Web och fetch? Vid cross-origin (CDN) krävs `credentials: include` i `tonic-web-wasm-client`. Detta verifieras och klientkonfigurationen justeras vid behov.
- WebAuthn kräver att RP ID matchar domänen. En CDN-driftsättning måste därför ligga på samma site som API:t, vilket dokumenteras.

## Utanför steg 1 (medvetet)
Admin-återställning av passkeys, att ta bort eller byta namn på passkeys, att återkalla inbjudningar, flera bolag per användare, roller utöver admin/användare, e-postutskick och e-postverifiering, byte av e-post, rate limiting och audit-UI.

## Verifiering
- `cargo test --workspace` → alla enhets- och integrationstester är gröna.
- `make e2e` (bygger frontend, startar servern mot en temporär DB och kör Playwright med virtuell authenticator) → alla flöden i punkt 9 är gröna.
- `make dist` och därefter `./target/dist/doris` → öppna `http://localhost:3000`, registrera första användaren med en riktig passkey (Touch ID) och logga ut och in igen.
- `sqlite3 doris.db "UPDATE events SET payload='{}'"` → nekas av en trigger.

## Beslut i Plan 2 (2026-09-28)
- **OpenSSL:** webauthn-rs länkar mot systemets OpenSSL (Homebrew `openssl@3` på macOS, `libssl-dev` på Debian/Ubuntu). Den är inte vendored.
- **Sessioner:** gäller i 30 dagar räknat från inloggningen. Utgångna sessioner rensas när en ny skapas.
- **BeginLogin för okänd e-post:** svaret är en fejkad utmaning från webauthn-rs `WebauthnFakeCredentialGenerator`. Credential-id:na är stabila per e-post och räknas fram med HMAC och en serverhemlighet som sparas i tabellen `server_secrets`. Alla inloggningsfel ger samma `LoginFailed`.
- **Passkeyns namn:** anges vid *Begin* (`BeginRegistration`, `BeginAddPasskey`) i stället för vid *Finish*. Då valideras namnet innan authenticatorn skapar något credential.
- **FinishRegistration:** skickar inbjudningstoken igen, så token aldrig lagras i klartext på servern.
- **Villkorligt UI:** `autocomplete="username webauthn"` går inte att använda, eftersom webauthn-rs registrerar passkeys med `residentKey: discouraged`. Frontend använder `autocomplete="username"`.
- **Bootstrap:** en inbjudningstoken ignoreras när inga användare finns ännu.
- **Registreringsförhandskontroll:** `check_registration` kör registreringsreglerna i en transaktion och rullar sedan tillbaka. Ceremonin nekar därmed ogiltig registrering innan authenticatorn tillfrågas.
