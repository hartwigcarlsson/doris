# AGENTS.md: Doris

Doris is a bookkeeping system for Swedish companies. It must comply with
Bokföringslagen (SFS 1999:1078):
https://www.riksdagen.se/sv/dokument-och-lagar/dokument/svensk-forfattningssamling/bokforingslag-19991078_sfs-1999-1078/

Features are built incrementally, one step at a time. Each step gets a design
spec in `docs/superpowers/specs/` and an implementation plan in
`docs/superpowers/plans/`.

## Principles
- **Self-contained, lightweight, fast.** The system is one binary plus one
  SQLite file. There are no external services and no runtime CDN dependencies:
  fonts, icons and CSS are shipped with the app. Don't add a dependency when a
  few lines of code will do.
- **TDD is mandatory.** No functionality or bug fix goes in unless a failing
  test demonstrates it first. The cycle is red → green → refactor, and each
  cycle ends in a commit.
- **Rust only.** Node is a dev dependency, used only for the Playwright e2e
  tests and never at runtime.

## Stack
| Layer | Choice | Why |
|---|---|---|
| Backend | Rust, tonic 0.14 + tonic-web (gRPC-Web), axum via tonic | One process serves both the API and the frontend |
| Frontend | Leptos 0.8 CSR, built with Trunk, Tailwind v4 (standalone CLI) | Pure WASM SPA; no SSR, so gRPC is the only protocol |
| API | gRPC-Web (`tonic-web-wasm-client` in the browser) | Typed contract shared from `proto/` |
| Storage | SQLite via sqlx 0.9 | A single file |
| Model | Event sourcing, with projections for reads | Append-only history, as BFL requires |

## Layout
```
proto/              .proto files (package doris.<area>.v1)
migrations/         sqlx migrations, NNNN_name.sql, shared by all crates
crates/company      doris-company: companies, members, fiscal year and accounting method
crates/eventstore   doris-eventstore: append-only event log, DB open + migrations
crates/identity     doris-identity: users, passkeys, invitations, sessions
crates/ledger       doris-ledger: chart of accounts, vouchers, opening balances and year closing
crates/proto        doris-proto: generated code (feature `server` for stubs)
crates/server       doris-server: binary, gRPC services, embedded frontend
crates/web          doris-web: Leptos CSR app, UI components
e2e/                Playwright tests (virtual WebAuthn authenticator)
```

## Event sourcing rules
- The `events` table is append-only, and a trigger rejects UPDATE and DELETE.
  Never work around this.
- Payloads are JSON and carry `schema_version`. Never change the meaning of an
  existing event. To change one, add a new version or a new event type and
  upcast old payloads when reading.
- Projections are updated in the **same transaction** as the append
  (`BEGIN IMMEDIATE`). Every projection must be rebuildable from `read_all`,
  and a test must cover that.
- Rules that span many records, such as a unique email, are enforced inside the
  write transaction, for example with UNIQUE constraints on projections.
- Domain logic is pure: `decide(state, cmd) -> Result<Vec<Event>>` and
  `evolve(state, event)`. Test it given/when/then, without a database.
- Operational data is **not** events and may be purged. That covers sessions
  and WebAuthn ceremony state.
- Voucher numbers run 1..=n per company and fiscal year without gaps (BFL
  5 kap.). The number is decided inside the write transaction
  (`last_number + 1`), never by the client and never ahead of time; the
  `vouchers` projection's primary key and the `vouchers_numbered_without_gaps`
  trigger back that up. `crates/ledger/tests/stress.rs` must keep passing.
- A voucher is never changed or removed. A rättelse is a new voucher with
  every line reversed and `corrects` pointing at the original.
- The saldobalans and huvudbok (`trial_balance`, `account_ledger` in
  `crates/ledger/src/queries.rs`) are plain queries over the voucher
  projections, one fiscal year at a time. Ingående balanser are never
  stored for later years: they are the first year's typed-in
  `opening_balances` plus every earlier year's lines on accounts 1000–2999.
- Closing a year (`FiscalYearClosed`) first books "Årets resultat", 8999
  against 2099 (2019 for enskild firma, HB and KB), then locks the year:
  no voucher or rättelse goes into a closed year. Years close oldest first,
  once they have ended. Reopening (`FiscalYearReopened`, with a reason)
  comes before the reversal of that voucher and goes newest first, so no
  voucher is ever recorded while its year is closed.

## BFL requirements to keep in mind
- Varaktighet (durability): accounting data must never be altered or deleted.
  Corrections are new entries.
- Behandlingshistorik (5 kap. 11 §): record who did what and when. This lives
  in the event metadata.
- Archiving: data must be kept 7 years in a readable form, which is why events
  are stored as JSON rather than an opaque binary.
- SQLite runs with `journal_mode=WAL`, `synchronous=FULL` and
  `foreign_keys=ON`. It also runs with `busy_timeout` 30 s
  (`doris_eventstore::open`), so writers queue for the write lock instead of
  failing; `crates/ledger/tests/stress.rs` exercises that.

## Naming and language
- Code, identifiers, URLs, query params, proto, event names, commits: English.
- Only user-visible UI text is Swedish.

## Authentication
- WebAuthn/passkeys only (webauthn-rs). Never store or accept passwords.
- A user is identified by email plus a display name, and can have several
  passkeys.
- The first user to register becomes admin. After that, registration requires
  an email-bound invitation that an admin creates.
- WebAuthn ceremonies (`doris_identity::Auth`) keep their state server-side
  for 5 minutes, and each one can be finished once. The passkey name is given
  at *begin*, and the invitation token is sent again at *finish*. A plaintext
  token is never stored.
- `begin_login` never reveals whether an email exists. An unknown email gets
  a fake challenge (webauthn-rs `WebauthnFakeCredentialGenerator`, keyed by
  `server_secrets.fake_credential_key`), and every login failure is
  `Error::LoginFailed`.
- Sessions last 30 days (absolute).
- The session is an opaque token in the `doris_session` cookie, set with
  `HttpOnly; Secure; SameSite=Strict`. Only a SHA-256 hash of the token is
  stored, and the same goes for invitation tokens.
- Email is personal data: never log it.

## API
- The contract lives in `proto/doris/auth/v1/auth.proto`,
  `proto/doris/company/v1/company.proto` and `proto/doris/ledger/v1/ledger.proto`. `doris-proto` generates
  the client; its `server` feature adds the server stubs. The client builds for
  wasm32 because no transport is generated.
- gRPC-Web over HTTP/1.1 (`tonic_web::GrpcWebLayer`) shares one port with the
  embedded frontend. Integration tests speak gRPC-Web too (`GrpcWebClientLayer`
  over a hyper client), exactly like the browser.
- Error statuses carry stable snake_case codes as the message, for example
  `invalid_email`, `not_signed_in`, `not_admin`, `login_failed` and
  `ceremony_expired`. The frontend translates them. They never contain
  personal data. The mapping is in `crates/server/src/grpc.rs` (`status`,
  `domain_code`). Company codes are mapped in `crates/server/src/company.rs`
  (`status`, `domain_status`). Ledger codes are mapped in
  `crates/server/src/ledger.rs` (`status`, `domain_status`).
- Company lookup uses Bolagsverket's free "värdefulla datamängder" API (OAuth2
  client credentials, register at portal.api.bolagsverket.se). Without
  credentials the lookup answers `lookup_unavailable` and details are typed in.
  An org nr can be a personnummer (enskild firma): never log it, and never send
  a personnummer to Bolagsverket.
- The server's only outbound HTTP is `reqwest` (native-tls: on Linux the same OpenSSL
  as webauthn-rs, on macOS Security.framework).
- The session cookie is `doris_session` (HttpOnly, Secure, SameSite=Strict,
  Path=/, 30 days).
- CORS is off unless `DORIS_CORS_ORIGINS` is set. Set it only when the frontend
  is served from another origin on the same site; WebAuthn's RP ID must still
  match.
- `DORIS_CORS_ORIGINS` values are exact origins (`https://app.example.se`:
  scheme + host [+ port], no path or trailing slash).
- `DORIS_RP_ORIGIN` must be the origin the page actually runs on (under
  `trunk serve` that's the trunk port; on a CDN, the CDN's origin), and only
  one origin can do WebAuthn per server.
- Use `localhost` (not `127.0.0.1`) in dev and e2e: the RP id is `localhost`,
  and Chromium only accepts `Secure` cookies over plain http on `localhost`.
- `DORIS_LISTEN` defaults to `127.0.0.1:3000`; containers need `0.0.0.0:3000`.

## Frontend
- `crates/web` is a Leptos 0.8 CSR app built with Trunk (`crates/web/Trunk.toml`
  pins Tailwind 4.3.3, the standalone CLI, so no Node is needed). Output goes to
  `crates/web/dist`, which the server embeds; it is never committed.
- `src/api.rs` holds the gRPC-Web client (cookies always included). A
  `<meta name="doris-api" content="https://api…">` in `index.html` points a
  CDN-hosted frontend at the API; empty means same origin.
- `src/passkey.rs` does the browser half of WebAuthn: webauthn-rs JSON in,
  `navigator.credentials.*`, JSON out.
- Forms use `novalidate`. Validation messages come from the server's error
  codes, so they're always Swedish; browser messages follow the browser's
  language.
- The active company (`src/active_company.rs`) is chosen in the header and
  remembered in `localStorage` as `doris.active_company.{user id}`. Pages that
  work on "the" company read it from the `Companies` context and send its
  `company_id` with every RPC. It grants nothing: the server checks membership
  on every call. Other tabs follow a change through the `storage` event, so a
  stale tab never books in the wrong company.
- `src/errors.rs` maps API error codes to Swedish text. Add a line there for
  every new code.
- `src/ui.rs` holds the preset's components, with class lists copied from
  shadcn's generated output. Add more by generating them with
  `npx shadcn init -t vite -b radix -p b1Gdz9bFY` in a scratch directory and
  copying the classes.
- The crate also compiles for the host, so `cargo test`/`clippy --workspace`
  include it. Also lint the wasm build:
  `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- Keep the wasm small. `make dist` builds it with the `wasm-release` profile
  (opt-level "z", LTO, `panic = "abort"`), and fails if the gzipped wasm
  grows past `WASM_BUDGET` (500 KB). That is what crosses the wire: the server
  sends frontend files compressed (brotli or gzip, via tower-http), and gzip
  is the larger of the two. Check what a new dependency adds before taking it
  on.
- The wasm is built with `--cfg erase_components` (set in `.cargo/config.toml`
  for `wasm32-unknown-unknown`), which type-erases Leptos views and keeps the
  wasm under budget; an env `RUSTFLAGS` overrides it, so don't set one for wasm
  builds.
- E2E tests live in `e2e/` (Playwright). Every test spawns its own server on
  a fresh database, and pages get a Chrome DevTools virtual WebAuthn
  authenticator. Select elements by their Swedish label or role.

## Style
The UI follows shadcn preset `b1Gdz9bFY`: style mira, base color stone, theme
amber, font Inter (self-hosted), small radius, lucide icons (inlined SVG).
Design tokens live in `crates/web/style/input.css`. Build only the components
you need.

## Commands
```
make dev       # server :3000 + `trunk serve` :8080 (open http://localhost:8080)
make test      # cargo test --workspace (all crates, incl. doris-web unit tests)
make web       # debug frontend build into crates/web/dist
make e2e       # frontend + debug server, then Playwright
make dist      # target/dist/doris (frontend embedded) + doris-web-<ver>.tar.gz
make e2e-dist  # Playwright against the release binary from `make dist`
docker compose up --build  # the image on :3000, data in the `doris-data` volume
```
Server configuration comes from env vars or CLI flags: `DORIS_DATABASE`,
`DORIS_LISTEN`, `DORIS_RP_ID`, `DORIS_RP_ORIGIN`, `DORIS_CORS_ORIGINS`,
`DORIS_SERVE_FRONTEND`, `DORIS_BOLAGSVERKET_CLIENT_ID`,
`DORIS_BOLAGSVERKET_CLIENT_SECRET`, `DORIS_BOLAGSVERKET_TOKEN_URL`,
`DORIS_BOLAGSVERKET_API_URL`.

Every push runs `cargo test --workspace` in GitHub Actions
(`.github/workflows/ci.yml`). If it passes, the `Dockerfile` is built and
published to `ghcr.io/hartwigcarlsson/doris`: tagged by branch, `sha-…`,
`latest` on main, and the version for `v*` tags. Dependabot
(`.github/dependabot.yml`) opens weekly PRs for cargo, the e2e npm packages
and the actions; its branches run the tests but don't publish an image.

Requires `protoc` on PATH and the system OpenSSL (webauthn-rs links it:
`brew install openssl@3` on macOS, `libssl-dev` on Debian/Ubuntu), plus
`trunk` and the `wasm32-unknown-unknown` target for the frontend.
