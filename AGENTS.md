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
crates/invoicing    doris-invoicing: customers, suppliers, and customer and supplier invoices
crates/eventstore   doris-eventstore: append-only event log, DB open + migrations
crates/identity     doris-identity: users, passkeys, invitations, sessions
crates/vat           doris-vat: redovisningsperiod, momsdeklaration and the momsavräkning
crates/ledger       doris-ledger: chart of accounts, vouchers, opening balances and year closing
crates/payroll      doris-payroll: employees, payroll runs and arbetsgivaravgift
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
- A module reads only its own tables. What another module owns is asked
  for through that module's functions (the server puts the answers
  together, as for member names and who recorded a voucher), never with
  SQL against its projections.
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
- Underlag (PDF, JPEG, PNG; `AttachmentAdded` in the ledger stream) keep
  their bytes in `attachment_files`: primary data like `events`, not a
  projection, append-only by trigger and keyed by SHA-256, so a file is
  stored once. They are read only through the company's own voucher
  (`voucher_attachments`) or the company's own customer or supplier invoice, never by
  hash alone. Invoicing finds the underlag on its own invoice and then asks
  the ledger for the bytes (`doris_ledger::attachment_data`); it does not
  read `attachment_files`. An underlag is never removed
  or renamed, and it may be added to a voucher in a closed year: it changes
  no amount. The type comes from the bytes, never from the client.
- Payroll (`payroll-{company_id}`: employees and runs) has its own
  stream. A run is Öppen, Färdigställd or Bokförd; only an open run
  changes, a finalized one can be reopened, and finalizing may precede
  the pay date. Booking (`PayrollRunBooked`) needs `pay_date <= today`
  and books the voucher with `doris_ledger::record_voucher_in` in the
  payroll write transaction. A booked run is never reopened directly:
  its booking is backed out with a rättelse (`correct_voucher_in`,
  dated today but no later than its fiscal year's end, from the run or
  the grundbok), and the run is Färdigställd again. Whether a run is
  booked is derived from the ledger's corrections
  (`doris_ledger::corrected_vouchers_in`, read in payroll's own
  transaction), never stored. Payroll tables have no foreign key to
  `vouchers`, and payroll never reads that table itself.
- Arbetsgivaravgift (`doris_payroll::domain::employer_fee`) is in code
  from 2026: 31,42 %; 10,21 % for those 67 when the year began; 0 for
  born 1937 or earlier; 20,81 % on the first 25 000 kr a month for
  19–23-year-olds from 2026-04-01 to 2027-09-30. The youth cap counts
  booked runs only; a booking whose fee would change is refused
  (`payroll_run_outdated`).
- Preliminary tax (`doris_payroll::tax`) comes from an employee's setting
  (`EmployeeTaxChanged`: tabell 29–42 + kolumn 1–6, or a whole percent),
  or is typed on the run line (manual). Skatteverket's monthly tables are
  reference data in `tax_tables`, not events: the server fetches a year
  from Skatteverket's open data the first time it's needed and stores it,
  replacing any earlier copy. Every locked line records its `tax_basis`
  (table/year/column, percent or manual); lines from before have none
  and read as manual.
- AGI (`doris_payroll::agi`): a period ÅÅÅÅMM declares the booked runs
  paid in it, one individuppgift per employee (whole kronor, rounded down
  on the sum), with FK487 computed as Skatteverket does (fee rates of the
  period, youth cap per individuppgift, rounded down; it may differ a few
  kronor from 2731). Doris writes the file (schema
  arbetsgivardeklaration_1.1) and the user uploads it; marking a month
  submitted appends `AgiMonthSubmitted` with what was declared. A month
  that changes afterwards shows as Ändrad and its next file carries the
  changed IUs, a Borttag for each removed one and a new HU.
  An employee's specification number is their 1-based position in the
  register (`Payroll.employees`, hire order), the same in every month.

- Moms (`vat-{company_id}`, crate `doris-vat`): `VatPeriodSet` per räkenskapsår (månad, kvartal (default), helår, ej momsregistrerad; calendar months and quarters, a period belongs to the year its last month is in, and a year's first period starts the day after the year before's last period, so a change of kind in a broken year neither repeats nor skips a month) and `VatReturnSubmitted` with what was declared. An account's box is `AccountVatBoxSet` in the chart (BAS default in `crates/ledger/src/vat_box.rs`, read from the chart's events). The boxes come from `doris_ledger::vat_box_totals_in`, leaving out Doris' own settlement vouchers and their corrections; öre are struck off per box and box 49 is computed from the rounded boxes. Marking a period submitted books the momsavräkning (VAT accounts to 2650, öre to 3740, dated the period's last day) with `record_voucher_in` in the same transaction; a later submission books only the difference, and a corrected settlement counts as not booked. `doris-vat` has no projections: it reads its own stream.

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
  `proto/doris/company/v1/company.proto`, `proto/doris/ledger/v1/ledger.proto`,
  `proto/doris/invoicing/v1/invoicing.proto`,
  `proto/doris/payroll/v1/payroll.proto` and `proto/doris/vat/v1/vat.proto`. `doris-proto` generates
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
  `crates/server/src/ledger.rs` (`status`, `domain_status`). Invoicing codes
  are mapped in `crates/server/src/invoicing.rs` (`status`, `domain_status`).
- `InvoicingService` keeps a customer and a supplier register per
  company (`customers-{company}` and `suppliers-{company}` streams; the
  `customers`/`suppliers` projections hold the details as JSON).
  Numbers run 1..=n per company and register, decided in the write
  transaction. A party is never removed, only deactivated. Codes:
  `invalid_name`, `invalid_vat_number`, `invalid_payment_terms`,
  `invalid_bankgiro`, `invalid_plusgiro`, `invalid_iban`, `invalid_bic`,
  `customer_not_found` and `supplier_not_found` (plus `invalid_org_nr`,
  `invalid_address` and `invalid_email`). The VAT number's format is
  checked, never looked up in VIES.
- Supplier invoices (`supplier-invoices-{company}` stream, `supplier_invoices`
  projection) follow the company's accounting method: faktureringsmetoden
  books the registration against 2440 and the payment 2440 against the
  bank; kontantmetoden books nothing until payment, then cost and VAT
  against the bank, with the underlag. Cancelling (unpaid only) and
  reversing a payment book `correct_voucher_in` corrections, dated today
  or the voucher's fiscal year's last day. One live invoice per supplier
  and invoice number (partial unique index). `InvoicingService` takes 21
  MiB messages and sends up to 11 MiB, and `session_gate` covers it too.
  Codes: `supplier_invoice_not_found`, `supplier_inactive`,
  `invalid_invoice_number`, `duplicate_supplier_invoice`,
  `invalid_due_date`, `invalid_reference`, `invalid_invoice_lines`,
  `invalid_vat_rate`, `invalid_vat_amount`, `invalid_invoice_account`,
  `invalid_payment_account`, `supplier_invoice_paid`,
  `supplier_invoice_not_paid` and `supplier_invoice_cancelled`; ledger
  errors keep their codes. Unpaid invoices at year end under
  kontantmetoden are not booked yet; Räkenskapsår warns for unpaid customer
  and supplier invoices.
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
- `VatService` (`proto/doris/vat/v1/vat.proto`; codes mapped in `crates/server/src/vat.rs`): `SetVatPeriod`, `ListVatReturns`, `GetVatReturn`, `ExportVatFile` (eSKD 6.0, ISO-8859-1) and `MarkVatReturnSubmitted` (with the fingerprint). Codes: `invalid_vat_period`, `vat_period_not_ended`, `vat_period_locked`, `vat_return_outdated`, `vat_return_unchanged` and `vat_not_registered`; `LedgerService.SetAccountVatBox` answers `invalid_vat_box`. Ledger refusals keep their codes.
- `LedgerService` also has `GetOpeningBalances`, `SetOpeningBalances`,
  `CloseFiscalYear` and `ReopenFiscalYear`. Their codes are
  `not_balance_sheet_account`, `duplicate_account`,
  `opening_balances_unbalanced`, `invalid_reason`, `fiscal_year_not_found`,
  `fiscal_year_closed`, `fiscal_year_open`, `fiscal_year_not_ended`,
  `previous_fiscal_year_open` and `later_fiscal_year_closed`.
- `LedgerService` also has `AddAttachment` and `GetAttachment`, and
  `RecordVoucher` takes underlag. Limits: 10 MiB per file, 20 MiB per
  request; the service accepts 21 MiB messages and sends up to 11 MiB (the
  other services keep tonic's 4 MiB). Codes: `unsupported_attachment_type`,
  `invalid_attachment_name`, `empty_attachment`, `attachment_too_large`,
  `duplicate_attachment` and `attachment_not_found`. File names are never
  logged.
- `ListVouchers` also says when each voucher was recorded and by whom
  (`recorded_at`, `recorded_by_name`). The ledger returns the recorder's
  user id from its own projection; the server asks identity
  (`doris_identity::get_user`) for the display name, once per person. An
  unknown user has an empty name. Never the email.
- `LedgerService` also has `GetFinancialStatements`: the resultaträkning
  and balansräkning for one fiscal year under ÅRL headings (K2's
  abbreviated forms), with the year before as comparison. The mapping from
  BAS account to post lives only in `crates/ledger/src/statements.rs`; the
  frontend draws the lines it gets. "Årets resultat" in the balansräkning
  is accounts 3000–8989, the same as in the resultaträkning; the result
  account (2099/2019) and 8990–8999 go with the earlier results, so the
  closing voucher cancels there and a closed year reads like an open one.
- `PayrollService` codes are mapped in `crates/server/src/payroll.rs`
  (`status`, `domain_status`): `invalid_personal_identity_number`,
  `invalid_employee_name`, `invalid_salary`, `invalid_salary_account`,
  `invalid_tax`, `empty_payroll_run`, `duplicate_payroll_run_line`,
  `duplicate_employee`, `employee_inactive`, `employee_not_found`,
  `payroll_run_not_found`, `payroll_run_not_open`,
  `payroll_run_not_finalized`, `payroll_run_booked`,
  `payroll_run_not_booked`, `payroll_run_not_due` and
  `payroll_run_outdated`. A run text over 200 characters is
  `invalid_voucher_text`; ledger refusals keep the ledger's codes.
- `PayrollService` also has `SetEmployeeTax`, and a run line's `tax` is
  optional (unset: computed). Codes: `invalid_tax_table`,
  `invalid_tax_percent`, `tax_required` and `tax_table_unavailable`
  (Skatteverket unreachable, or the year not published yet; a typed tax
  still works).
- The server's outbound HTTP also fetches tax tables from Skatteverket
  (`crates/server/src/skatteverket.rs`, no credentials). It never holds
  the SQLite write lock while fetching.
- `PayrollService` also has `GetAgiContact`, `SetAgiContact`,
  `ListAgiMonths`, `GetAgiMonth`, `ExportAgiFile` and
  `MarkAgiSubmitted`. Codes: `invalid_period`, `invalid_agi_contact`,
  `agi_contact_missing`, `agi_period_empty`, `agi_unchanged` and
  `agi_file_outdated` (marking sends the fingerprint, a hex SHA-256 of
  what would be recorded, of the file downloaded or the month shown). The AGI
  file holds personnummer by design; it goes only to a member and is
  never logged.
- A personnummer is personal data: never log it and never send it to
  an external service. It is stored as twelve digits and never changed
  on an employee.
- tonic reserves the size a frame header claims before a handler runs, so
  `session_gate` (`crates/server/src/lib.rs`) answers `LedgerService` and
  `InvoicingService` calls without a valid session with `not_signed_in` before the body is read. The
  handlers still check the session themselves.
- A reverse proxy in front of Doris must allow request bodies of about
  21 MiB (nginx's default `client_max_body_size` is 1 MiB).
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
- `index.html` sends `GetStatus` itself (`window.dorisStatus`), so the answer
  arrives while the wasm downloads instead of a round trip after it;
  `api::prefetched_status` reads it, and the app asks again if it's missing.
- Until the app starts, `#boot` in `index.html` shows "Laddar Doris…" and a
  progress bar that `boot.js` (Trunk's `data-initializer`) moves. The Tailwind
  CSS is inlined (`data-inline`) so it paints without another round trip,
  and `#boot` uses the system font so Inter doesn't compete with the wasm.
  Trunk hands the initializer the wasm's size from before wasm-opt; a
  `post_build` hook in `Trunk.toml` (perl) writes the real one. Trunk names
  the initializer `<hash>-boot.js`, which the server also caches forever.
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
- `src/attachments.rs` reads picked files (one over 10 MiB is refused
  before it is read), checks the size limits before upload, and opens an underlag as a Blob URL in a new tab. The tab is
  opened on the click and navigated once `GetAttachment` returns; a blocked
  popup shows `popup_blocked`. `ledger_api()` raises its decode limit to
  11 MiB for that.
- `src/ui.rs` holds the preset's components, with class lists copied from
  shadcn's generated output. Add more by generating them with
  `npx shadcn init -t vite -b radix -p b1Gdz9bFY` in a scratch directory and
  copying the classes.
- `src/vat_form.rs` holds SKV 4700's sections and row texts; `/vat` and `/vat/{ÅÅÅÅMM}` draw them.
- `src/nav.rs` holds the header: one row with Doris, the company picker,
  the main menu (Översikt, Bokföring (with Moms), Inköp, Försäljning, Lön) and the account
  menu. Menus are native `<details name="doris-nav">`, so the browser keeps
  one open. A click listener closes them on a click outside or on one of
  their links or buttons, a keydown listener on Escape (focus goes back to
  the menu's button), and an effect when the path changes. `section_of`
  decides which menu a path belongs to: add a line there for every new page.
- The start page (`src/pages/home.rs`) is the overview for the active
  company and a chosen räkenskapsår (kept in `?fy=`; by default the year
  that contains today). It adds no RPC: it sends `GetCompany`,
  `ListFiscalYears`, `GetTrialBalance`, `ListVouchers`,
  `ListSupplierInvoices`, `ListCustomerInvoices`, `ListPayrollRuns`,
  `ListAgiMonths` and `ListVatReturns`, and `src/overview.rs` works everything out in pure
  functions: key figures (operating income 3000–3999, operating costs
  4000–7999, class 8 only in the result, cash 1900–1999), income and costs per month, the year's progress and the
  "Att göra" rules. Each call is its own task, so a slow one holds nothing
  back. A card whose call failed shows the error and the others still show;
  if the years cannot be listed, every card that needs a year shows that
  error. The monthly sums read every voucher of the year: when that
  gets heavy, add a `GetMonthlyTotals` to the ledger.
- Verifikationer lists the year's vouchers newest first, fifty at a time.
  The search field and the "Saknar underlag"/"Rättelser" boxes filter in
  the browser (`src/voucher_search.rs`): every word must match the number
  (exactly), an account on the voucher (by prefix), a whole amount, or part
  of the text or of an account's name. A search that is one amount with
  spaces in it ("1 250,00", as the page shows it) matches that amount.
  A row's time is shown with the UTC offset that applied then, not today's.
- A page starts its tasks with `crate::task::spawn_local` (`src/task.rs`),
  never `leptos::task::spawn_local`: the answer may come after the user
  has left, the page's signals are disposed by then, and reading one
  panics, which aborts the release wasm. A page task is dropped at its
  next `await` once the path has changed (the request is already sent, and
  the server finishes it). Only what outlives a page (the session, the
  company list, the header, login and registration) uses Leptos' own.
  `e2e/tests/leaving.spec.ts` leaves every page while it loads and a few
  actions mid-flight; tests in `app.rs` fail if a page is missing from it
  or starts a task the other way.
- A view is a `grid gap-6` that starts with `PageHeader` (the page's one
  `<h1>`, actions to the right). Tables sit in `TableCard`, statuses are
  `Badge`s and "Ny …" actions are `LinkButton`s. A one-column form is a
  `narrow` `Card` (352px, left-aligned); a form with several columns or a
  line editor is a full-width `Card` or `Panel`. Login and registration
  are centered, and their card's title is the page's `<h1>` (`page_title`).
- Icons are lucide shapes inlined in `ui.rs` (`Icon`, `IconName`), copied
  from lucide-static with the closing tags written out.
- The crate also compiles for the host, so `cargo test`/`clippy --workspace`
  include it. Also lint the wasm build:
  `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`.
- Keep the wasm small. `make dist` builds it with the `wasm-release` profile
  (opt-level "z", LTO, `panic = "abort"`). The server sends frontend files
  compressed (brotli or gzip, via tower-http). Check what a new dependency
  adds before taking it on.
- The wasm is built with `--cfg erase_components` (set in `.cargo/config.toml`
  for `wasm32-unknown-unknown`), which type-erases Leptos views and keeps the
  wasm small; an env `RUSTFLAGS` overrides it, so don't set one for wasm
  builds.
- E2E tests live in `e2e/` (Playwright). Every test spawns its own server on
  a fresh database, and pages get a Chrome DevTools virtual WebAuthn
  authenticator. Select elements by their Swedish label or role. Header
  links live in menus: use `goTo(page, "Verifikationer")` from `fixtures.ts`.

## Style
The UI follows shadcn preset `b1Gdz9bFY`: style mira, base color stone, theme
amber, font Inter (self-hosted), small radius, lucide icons (inlined SVG, see
`IconName`). The page and the header are `max-w-6xl`.
`--chart-1` (amber) and `--chart-2` (stone) colour the overview's chart and
progress bar.
Design tokens live in `crates/web/style/input.css`. Build only the components
you need.

Every view follows one design, and a new or changed view is not done until
it does. The rules are in `docs/design/README.md`: read them before any
work on the UI. They cover the shell and its menus, how a view is built,
which component to use, tokens, light and dark, 390px, and the tests that
keep it so.

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
`DORIS_BOLAGSVERKET_API_URL`, `DORIS_TAX_TABLES_URL`.

Every push runs `cargo test --workspace` in GitHub Actions
(`.github/workflows/ci.yml`). If it passes, the `Dockerfile` is built and
published to `ghcr.io/hartwigcarlsson/doris`: tagged by branch, `sha-…`,
`latest` on main, and the version for `v*` tags. Dependabot
(`.github/dependabot.yml`) opens weekly PRs for cargo, the e2e npm packages
and the actions; its branches run the tests but don't publish an image.

Requires `protoc` on PATH and the system OpenSSL (webauthn-rs links it:
`brew install openssl@3` on macOS, `libssl-dev` on Debian/Ubuntu), plus
`trunk`, `perl` (Trunk's post_build hook) and the `wasm32-unknown-unknown`
target for the frontend.
