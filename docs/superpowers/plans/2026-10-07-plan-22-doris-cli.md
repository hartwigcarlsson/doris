# doris-cli Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ett kommandoradsverktyg, `doris-cli`, som en människa eller en AI-agent använder för att läsa och bokföra i Doris via API-token: gh-formade kommandon, `--json` överallt och ett `--dry-run` som kontrolleras på servern.

**Architecture:** Servern får `dry_run` på `RecordVoucher` och `CorrectVoucher` (samma transaktion, rollback i stället för commit). Felmeddelandena flyttas från webben till `doris-proto::messages` så att båda klienterna delar dem. En ny crate `crates/cli` (bibliotek + binär) använder `doris-proto`s klient över gRPC-Web (`tonic-web` + `hyper-util` + `hyper-tls`), `clap` för argumenten och en `run(args, env, out, err) -> i32` som tester anropar direkt mot en riktig testserver.

**Tech Stack:** Rust 2024, clap 4 (derive), tonic 0.14 + tonic-web, hyper-util (legacy client), hyper-tls (native-tls), serde_json, jiff.

**Spec:** `docs/superpowers/specs/2026-10-07-doris-cli-design.md`

### Avsteg från specen
1. **CLI:ns integrationstester ligger i `crates/server/tests/doris_cli.rs`**, med serverns befintliga `TestServer` (passkey-enheter, `api_token`, `company`). `doris-cli` blir dev-beroende till `doris-server`. Att bygga en egen testserver i `crates/cli` hade duplicerat hela harnessen.
2. **Testkörningens märkning till `auth_gate`** är en response-extension (`DryRun`) som handlern sätter; lagret hoppar då över `touch_api_token`.
3. **Ledgern behåller sina befintliga funktioner** och får två nya, `record_voucher_or_preview` och `correct_voucher_or_preview`, med `dry_run`; de gamla anropar de nya med `false`. Så slipper ett dussin befintliga tester ändras.

## Global Constraints
- Kod, identifierare, kommandon, flaggor, JSON-fält och commits på engelska; text som visas för användaren (textläge, felmeddelanden) på svenska. Varje commit slutar med `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` och `Claude-Session: https://claude.ai/code/session_01GGh5ZH92u9PPnjiByuz674`.
- TDD: ett fallerande test först; varje cykel slutar i en commit.
- `DORIS_TOKEN` krävs; skickas som `authorization: Bearer …`; skrivs aldrig ut, loggas aldrig, finns aldrig i ett felmeddelande. `DORIS_URL` krävs (ingen standard); `http://` bara till `localhost`/`127.0.0.1` (`insecure_url`). `DORIS_COMPANY` valfri.
- Kommandon: `auth status`, `company list|view`, `year list`, `account list`, `ver list|view|new|correct`, `report trial-balance|ledger|statements`. Globala flaggor `--json`, `--company`; `--year` där ett räkenskapsår gäller; `--dry-run` på alla (gör något bara på `ver new`/`ver correct`).
- `--json`: exakt ett JSON-värde på stdout; fel som `{"error":{"code","message"}}` på stdout och inget annat. Belopp: decimalsträngar i kronor med två decimaler (`"1250.00"`, `"-80.00"`), aldrig flyttal. Datum `"ÅÅÅÅ-MM-DD"`, tider RFC 3339. Skrivande kommandon har alltid `"dry_run"`.
- Exit-koder: 0 ok; 1 servern/bokföringen nekade; 2 fel användning; 3 autentisering/anslutning.
- Klientens egna felkoder: `usage`, `missing_token`, `missing_url`, `insecure_url`, `connection_failed`, `company_ambiguous`, `company_not_found`, `fiscal_year_not_found`, `voucher_not_found`. Serverns koder används som de är.
- Belopp in: kronor, högst två decimaler, punkt eller komma, inget tecken. Underlag: 10 MiB per fil, 20 MiB per verifikation, kontrollerat innan något skickas.
- `dry_run` sparar ingenting (inga events, inga rader i `vouchers`/`attachment_files`, ingen `api_token_usage`) och ger samma fel som en riktig körning.
- Lint: `cargo clippy --workspace -- -D warnings`, `cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`, `cargo fmt --all --check`.

## Review Focus
1. **Belopp med tre decimaler, tecken, tusentalsavgränsare eller tomt** (`1250.505`, `-80`, `1 250`, `=100`): exit 2 med `usage`, inget skickas. Test i Task 3 (`amounts`).
2. **Flera bolag och inget `--company`/`DORIS_COMPANY`**: exit 2, `company_ambiguous`, och textläget listar bolagen. Test i Task 3.
3. **Servern svarar inte eller fel adress** (`DORIS_URL=http://127.0.0.1:9`): exit 3, `connection_failed`, ingen panik, ingen token i utdata. Test i Task 3.
4. **`--dry-run` med underlag som är för stora eller av fel typ**: samma fel som en riktig körning och inget sparat. Test i Task 1 (servern) och Task 4.
5. **`--json` och ett fel mitt i**: stdout är fortfarande exakt ett JSON-värde (felobjektet), stderr tom. Test i Task 3 och 4.

---

### Task 1: `dry_run` på servern

**Files:**
- Modify: `proto/doris/ledger/v1/ledger.proto`
- Modify: `crates/ledger/src/lib.rs`
- Modify: `crates/server/src/ledger.rs`, `crates/server/src/lib.rs`
- Test: `crates/ledger/tests/store.rs`, `crates/server/tests/ledger.rs`

**Interfaces:**
- Produces:
  - proto: `RecordVoucherRequest.dry_run = 6`, `RecordVoucherResponse { …, bool dry_run = 3; repeated Attachment attachments = 4; }`, `CorrectVoucherRequest.dry_run = 5`, `CorrectVoucherResponse { …, bool dry_run = 2; }`.
  - `doris_ledger::record_voucher_or_preview(pool, company_id, actor, cmd: RecordVoucher, attachments: Vec<NewAttachment>, today: Date, dry_run: bool) -> Result<(VoucherRef, Vec<Attachment>)>`
  - `doris_ledger::correct_voucher_or_preview(pool, company_id, actor, fiscal_year_start: Date, number: u32, date: Date, today: Date, dry_run: bool) -> Result<VoucherRef>`
  - `doris_server` intern: `pub(crate) struct DryRun;` (response-extension).

- [ ] **Step 1: Fallerande tester i ledger**

I `crates/ledger/tests/store.rs` (läs filens hjälpare för bolag, användare och verifikationer först; använd samma som testerna för `record_voucher_with_attachments`):

```rust
#[tokio::test]
async fn a_dry_run_books_nothing_but_answers_as_if_it_had() {
    // given a company with no vouchers in 2026 (use the file's setup helpers)
    // when record_voucher_or_preview(.., dry_run = true) with one PDF underlag
    // then it returns VoucherRef { number: 1, .. } and one Attachment with the PDF's sha256,
    // and afterwards: list_vouchers for the year is empty, `SELECT COUNT(*) FROM events`
    // is unchanged, `SELECT COUNT(*) FROM attachment_files` is 0;
    // and a real record_voucher_or_preview(.., false) right after also gets number 1.
}

#[tokio::test]
async fn a_dry_run_refuses_exactly_like_a_real_one() {
    // an unbalanced voucher: both dry_run true and false give the same Err (VoucherUnbalanced);
    // a voucher in a closed year: both give FiscalYearClosed; an empty attachment: both EmptyAttachment.
}

#[tokio::test]
async fn a_dry_run_correction_saves_nothing() {
    // book voucher 1, then correct_voucher_or_preview(.., 1, today, today, true) returns number 2;
    // the voucher list still has one voucher and voucher 1 has no corrected_by; a real
    // correction afterwards also gets number 2.
}
```

Skriv ut testerna fullständigt med filens hjälpare (konstruktionen av `RecordVoucher`, `NewAttachment`, ett stängt år) — kommentarerna ovan är kraven, inte koden.

- [ ] **Step 2: Kör och se dem fallera**

Run: `cargo test -p doris-ledger --test store dry_run`
Expected: kompileringsfel (`record_voucher_or_preview` finns inte).

- [ ] **Step 3: Implementera i ledger**

```rust
/// Books a voucher with its underlag in one transaction and returns what
/// was attached. With `dry_run`, everything runs (numbering, rules, file
/// checks) and is then rolled back: nothing is saved.
pub async fn record_voucher_or_preview(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    attachments: Vec<NewAttachment>,
    today: Date,
    dry_run: bool,
) -> Result<(VoucherRef, Vec<Attachment>)> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = record_voucher_in(&mut tx, company_id, actor, cmd, today).await?;
    let mut added = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        added.push(
            add_attachment_in(
                &mut tx,
                company_id,
                actor,
                voucher.fiscal_year_start,
                voucher.number,
                attachment,
                today,
            )
            .await?,
        );
    }
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok((voucher, added))
}
```

`record_voucher_with_attachments` blir `record_voucher_or_preview(…, false).await.map(|(voucher, _)| voucher)` (behåll ponytail-kommentaren vid loopen i den nya funktionen). Samma mönster för `correct_voucher_or_preview` (rollback vid `dry_run`), och `correct_voucher` anropar den med `false`.

- [ ] **Step 4: Fallerande servertester**

I `crates/server/tests/ledger.rs` (filen har `company`, `sale`, `pdf`, `upload`, `code_of`, `Ledger`):

```rust
#[tokio::test]
async fn record_voucher_with_dry_run_answers_but_saves_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let mut request = sale(&id, 12_500);
    request.dry_run = true;
    request.attachments = vec![upload("kvitto.pdf", pdf(1000))];

    let preview = api.record_voucher(authed(request, &anna)).await.unwrap().into_inner();
    let vouchers = api
        .list_vouchers(authed(pb::ListVouchersRequest { company_id: id.clone(), fiscal_year_start: "2026-01-01".into() }, &anna))
        .await
        .unwrap()
        .into_inner()
        .vouchers;
    let real = api.record_voucher(authed(sale(&id, 12_500), &anna)).await.unwrap().into_inner();

    assert!(preview.dry_run);
    assert_eq!(preview.number, 1);
    assert_eq!(preview.attachments.len(), 1);
    assert_eq!(preview.attachments[0].file_name, "kvitto.pdf");
    assert!(vouchers.is_empty());
    assert!(!real.dry_run);
    assert_eq!(real.number, 1);
}

#[tokio::test]
async fn a_dry_run_is_refused_like_a_real_run() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let mut unbalanced = sale(&id, 100);
    unbalanced.lines[1].credit = 99;
    let mut dry = unbalanced.clone();
    dry.dry_run = true;

    let real = api.record_voucher(authed(unbalanced, &anna)).await.unwrap_err();
    let preview = api.record_voucher(authed(dry, &anna)).await.unwrap_err();

    assert_eq!(code_of(preview), code_of(real));
}

#[tokio::test]
async fn correct_voucher_with_dry_run_saves_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.record_voucher(authed(sale(&id, 100), &anna)).await.unwrap();
    let correct = |dry_run| pb::CorrectVoucherRequest {
        company_id: id.clone(),
        fiscal_year_start: "2026-01-01".into(),
        number: 1,
        date: "2026-01-16".into(),
        dry_run,
    };

    let preview = api.correct_voucher(authed(correct(true), &anna)).await.unwrap().into_inner();
    let real = api.correct_voucher(authed(correct(false), &anna)).await.unwrap().into_inner();

    assert!(preview.dry_run);
    assert_eq!((preview.number, real.number), (2, 2));
    assert!(!real.dry_run);
}
```

Och i `crates/server/tests/api_tokens.rs` (som har `api_token`, `bearer`, `sale`, `vouchers`, `only_token_id`):

```rust
#[tokio::test]
async fn a_dry_run_with_a_token_does_not_count_as_use() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:write"])]).await;
    let mut request = sale(&id);
    request.dry_run = true;

    server.ledger().record_voucher(bearer(request, &secret)).await.unwrap();

    let listed = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert_eq!(listed[0].last_used_at, None);
}
```

(`sale` i `api_tokens.rs` bygger en `lpb::RecordVoucherRequest`; lägg till `dry_run: false` i dess struct-literal, liksom i alla andra struct-literaler av `RecordVoucherRequest`/`CorrectVoucherRequest` i testerna och webben — eller använd `..Default::default()`. Webben: `crates/web/src/pages/new_voucher.rs` och `vouchers.rs`.)

- [ ] **Step 5: Implementera i proto och servern**

`ledger.proto`:

```proto
message RecordVoucherRequest {
  …
  // Run every rule and answer as if booked, then save nothing.
  bool dry_run = 6;
}

message RecordVoucherResponse {
  string fiscal_year_start = 1;
  uint32 number = 2;
  bool dry_run = 3;
  repeated Attachment attachments = 4;
}

message CorrectVoucherRequest {
  …
  bool dry_run = 5;
}

message CorrectVoucherResponse {
  uint32 number = 1;
  bool dry_run = 2;
}
```

`crates/server/src/ledger.rs`, `record_voucher`: anropa `doris_ledger::record_voucher_or_preview(…, req.dry_run)` (spara `let dry_run = req.dry_run;` innan `req` flyttas) och svara med `dry_run` och `attachments: added.iter().map(attachment_message).collect()`. Vid `dry_run`, sätt markeringen:

```rust
        let mut response = Response::new(pb::RecordVoucherResponse { … });
        if dry_run {
            response.extensions_mut().insert(crate::DryRun);
        }
        Ok(response)
```

Samma för `correct_voucher` med `correct_voucher_or_preview`.

`crates/server/src/lib.rs`:

```rust
/// Marks a response as a dry run: nothing was saved, so the call does not
/// count as the token's use.
#[derive(Clone, Copy)]
pub(crate) struct DryRun;
```

och i `auth_gate`, efter `let response = …scope(…).await;`:

```rust
        let dry_run = response.extensions().get::<DryRun>().is_some();
        if !dry_run && let Err(err) = doris_identity::touch_api_token(&pool, token_id, now).await {
            tracing::warn!("api token usage: {err}");
        }
```

Om tonic inte för över `Response::extensions` till http-svaret i den här versionen (testet `a_dry_run_with_a_token_does_not_count_as_use` visar det), sätt i stället en intern header `x-doris-dry-run: 1` på svaret i handlern (`response.metadata_mut()`), läs den i `auth_gate` och ta bort den där innan svaret skickas. Beskriv valet i rapporten.

- [ ] **Step 6: Kör testerna**

Run: `cargo test -p doris-ledger && cargo test -p doris-server && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add proto crates
git commit -m "Let RecordVoucher and CorrectVoucher run dry: every rule, nothing saved"
```

---

### Task 2: Felmeddelandena i `doris-proto`

**Files:**
- Create: `crates/proto/src/messages.rs`
- Modify: `crates/proto/src/lib.rs`, `crates/web/src/errors.rs`

**Interfaces:**
- Produces: `doris_proto::messages::message(code: &str) -> &'static str` (samma tabell som i dag, plus CLI:ns koder); webbens `describe`/`describe_code` oförändrade utåt.

- [ ] **Step 1: Flytta testerna först**

Flytta `#[cfg(test)] mod tests` från `crates/web/src/errors.rs` till den nya `crates/proto/src/messages.rs` (de anropar `message(...)`), och lägg till:

```rust
    #[test]
    fn cli_codes_have_swedish_messages() {
        for code in [
            "usage",
            "missing_token",
            "missing_url",
            "insecure_url",
            "connection_failed",
            "company_ambiguous",
            "company_not_found",
            "fiscal_year_not_found",
            "voucher_not_found",
        ] {
            assert_ne!(message(code), "Något gick fel. Försök igen.", "{code}");
        }
        assert_eq!(message("missing_token"), "Ange en API-token i DORIS_TOKEN.");
    }
```

Behåll ett test i webben som visar att `describe_code` går via tabellen:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn codes_come_from_the_shared_table() {
        assert_eq!(super::describe_code("not_signed_in"), "Du är inte inloggad.");
    }
}
```

- [ ] **Step 2: Kör och se dem fallera**

Run: `cargo test -p doris-proto`
Expected: kompileringsfel (modulen finns inte).

- [ ] **Step 3: Implementera**

`crates/proto/src/messages.rs`: modul-doc `//! Swedish messages for the API's stable error codes, shared by the web app and doris-cli.` och `pub fn message(code: &str) -> &'static str` med webbens `match` oförändrad, plus raderna:

```rust
        "usage" => "Kommandot saknar eller har felaktiga argument.",
        "missing_token" => "Ange en API-token i DORIS_TOKEN.",
        "missing_url" => "Ange serverns adress i DORIS_URL.",
        "insecure_url" => "Använd https:// (http:// bara till localhost).",
        "connection_failed" => "Kunde inte nå Doris. Kontrollera DORIS_URL och anslutningen.",
        "company_ambiguous" => "Token har flera bolag. Välj ett med --company eller DORIS_COMPANY.",
        // "company_not_found" and "fiscal_year_not_found" exist already; keep their texts.
        "voucher_not_found" => "Verifikationen finns inte.",
```

(Kontrollera att `company_not_found`, `fiscal_year_not_found` och `voucher_not_found` inte redan finns i tabellen; lägg bara till dem som saknas.) `crates/proto/src/lib.rs`: `pub mod messages;`. `crates/web/src/errors.rs`:

```rust
//! Swedish messages for the API's stable error codes (the table lives in
//! `doris_proto::messages`, shared with doris-cli).

use doris_proto::messages::message;

/// The text to show for a failed API call.
pub fn describe(status: &tonic::Status) -> String {
    message(status.message()).to_owned()
}

/// The text for an error code found in the browser, before any API call.
pub fn describe_code(code: &str) -> String {
    message(code).to_owned()
}
```

- [ ] **Step 4: Kör testerna**

Run: `cargo test -p doris-proto -p doris-web && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/proto crates/web/src/errors.rs
git commit -m "Share the Swedish error messages between the web app and doris-cli"
```

---

### Task 3: `doris-cli`: grunden och de första kommandona

**Files:**
- Create: `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`, `crates/cli/src/lib.rs`, `crates/cli/src/amount.rs`, `crates/cli/src/client.rs`, `crates/cli/src/output.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/{auth,company,year,account}.rs`
- Modify: `Cargo.toml` (workspace: medlem, `doris-cli`, `hyper-tls`), `crates/server/Cargo.toml` (dev-dep `doris-cli`)
- Test: enhetstester i `crates/cli/src/*.rs`; `crates/server/tests/doris_cli.rs` (ny)

**Interfaces:**
- Produces (crate `doris_cli`):
  - `pub struct Env { pub token: Option<String>, pub url: Option<String>, pub company: Option<String> }` med `Env::from_process()` (läser `DORIS_TOKEN`, `DORIS_URL`, `DORIS_COMPANY`).
  - `pub async fn run<I, T>(args: I, env: &Env, out: &mut dyn std::io::Write, err: &mut dyn std::io::Write) -> i32 where I: IntoIterator<Item = T>, T: Into<std::ffi::OsString> + Clone` — `args` inklusive programnamnet.
  - `amount::{parse_kronor(&str) -> Option<i64>, kronor(i64) -> String /* "1250.00" */, display(i64) -> String /* "1 250,00" */}`
  - `output::Failure { code: String, message: String, exit: i32 }` och `output::exit_code(code: &str) -> i32`.

- [ ] **Step 1: Workspace och crate**

`Cargo.toml`: lägg till `"crates/cli"` i `members`, `doris-cli = { path = "crates/cli" }` och `hyper-tls = "0.6"` under `[workspace.dependencies]`; lägg till `hyper = { version = "1", features = ["client", "http1"] }` om den inte redan finns som workspace-beroende.

`crates/cli/Cargo.toml`:

```toml
[package]
name = "doris-cli"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "doris-cli"
path = "src/main.rs"

[dependencies]
clap.workspace = true
doris-proto.workspace = true
hyper-tls.workspace = true
hyper-util.workspace = true
jiff = { workspace = true, features = ["tzdb-bundle-always"] }
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tonic.workspace = true
tonic-web.workspace = true
tower.workspace = true
```

(Lägg till `http`/`hyper` om tonic-webs klienttyper kräver dem; följ hur `crates/server/tests/common/mod.rs` bygger sin `Transport`.)

`crates/server/Cargo.toml` `[dev-dependencies]`: `doris-cli.workspace = true`.

- [ ] **Step 2: Fallerande enhetstester**

`crates/cli/src/amount.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(parse_kronor("1250"), Some(125_000));
        assert_eq!(parse_kronor("1250.5"), Some(125_050));
        assert_eq!(parse_kronor("1250,50"), Some(125_050));
        assert_eq!(parse_kronor("0.01"), Some(1));
        for bad in ["", "-80", "+80", "1250.505", "1 250", "12,5,0", "abc", ".5", "5."] {
            assert_eq!(parse_kronor(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn kronor_are_written_exactly() {
        assert_eq!(kronor(125_000), "1250.00");
        assert_eq!(kronor(1), "0.01");
        assert_eq!(kronor(-8_000), "-80.00");
        assert_eq!(display(125_050), "1 250,50");
        assert_eq!(display(-100_000_000), "-1 000 000,00");
    }
}
```

`crates/cli/src/output.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_tell_what_to_fix() {
        for (code, exit) in [
            ("voucher_unbalanced", 1),
            ("fiscal_year_closed", 1),
            ("missing_scope", 1),
            ("company_not_found", 1),
            ("usage", 2),
            ("company_ambiguous", 2),
            ("missing_token", 3),
            ("missing_url", 3),
            ("insecure_url", 3),
            ("not_signed_in", 3),
            ("connection_failed", 3),
        ] {
            assert_eq!(exit_code(code), exit, "{code}");
        }
    }
}
```

`crates/cli/src/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_only_for_this_machine() {
        assert!(checked_url("https://doris.example.se").is_ok());
        assert!(checked_url("http://localhost:3000").is_ok());
        assert!(checked_url("http://127.0.0.1:3000").is_ok());
        assert_eq!(checked_url("http://doris.example.se").unwrap_err().code, "insecure_url");
        assert_eq!(checked_url("ftp://x").unwrap_err().code, "insecure_url");
    }
}
```

`crates/cli/src/lib.rs` (årstolkning):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_year_is_its_start_year_or_its_start_date() {
        let years = ["2025-07-01", "2026-07-01"].map(String::from);
        assert_eq!(pick_year(&years, Some("2026"), "2026-10-07"), Some("2026-07-01".into()));
        assert_eq!(pick_year(&years, Some("2025-07-01"), "2026-10-07"), Some("2025-07-01".into()));
        assert_eq!(pick_year(&years, None, "2026-03-01"), Some("2025-07-01".into()));
        assert_eq!(pick_year(&years, Some("2030"), "2026-10-07"), None);
    }
}
```

(`pick_year(starts: &[String], wanted: Option<&str>, today: &str) -> Option<String>`: med `wanted` = år väljs året vars start börjar på `"ÅÅÅÅ-"`; med datum exakt matchning; utan väljs det senaste år vars start ≤ `today`. Ett års slut behövs inte för valet eftersom åren ligger efter varandra.)

- [ ] **Step 3: Kör och se dem fallera**

Run: `cargo test -p doris-cli`
Expected: kompileringsfel (funktionerna finns inte).

- [ ] **Step 4: Implementera grunden**

`amount.rs`:

```rust
//! Kronor in, öre inside, exact strings out.

/// Öre from kronor typed as `1250`, `1250.5` or `1250,50`: no sign, no
/// grouping, at most two decimals.
pub fn parse_kronor(raw: &str) -> Option<i64> {
    let (whole, fraction) = raw.split_once(['.', ',']).unwrap_or((raw, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || (raw.len() > whole.len() && !digits(fraction)) || fraction.len() > 2 {
        return None;
    }
    let ore: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole.parse::<i64>().ok()?.checked_mul(100)?.checked_add(ore)
}

/// Öre as an exact decimal string in kronor, for JSON: `"1250.00"`.
pub fn kronor(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    format!("{sign}{}.{:02}", ore / 100, ore % 100)
}

/// Öre as the web shows kronor, with spaces between thousands: `"1 250,00"`.
pub fn display(ore: i64) -> String {
    let sign = if ore < 0 { "-" } else { "" };
    let ore = ore.unsigned_abs();
    let whole = (ore / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped},{:02}", ore % 100)
}
```

`output.rs`:

```rust
//! What a command prints: text for people, one JSON value with --json,
//! and an exit code that says what to fix.

use doris_proto::messages::message;
use serde_json::{Value, json};
use std::io::Write;

/// A command that did not succeed.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub exit: i32,
}

impl Failure {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message(code).to_owned(),
            exit: exit_code(code),
        }
    }

    /// A usage error with a message of its own (which flag, which value).
    pub fn usage(text: impl Into<String>) -> Self {
        Self {
            code: "usage".into(),
            message: text.into(),
            exit: 2,
        }
    }

    /// The server's refusal: its stable code, the shared Swedish text.
    pub fn from_status(status: &tonic::Status) -> Self {
        if status.code() == tonic::Code::Unavailable || status.code() == tonic::Code::Unknown && status.message().is_empty() {
            return Self::new("connection_failed");
        }
        Self::new(status.message())
    }
}

/// 1: the books refused; 2: fix the arguments; 3: fix the token or connection.
pub fn exit_code(code: &str) -> i32 {
    match code {
        "usage" | "company_ambiguous" => 2,
        "missing_token" | "missing_url" | "insecure_url" | "not_signed_in" | "connection_failed" => 3,
        _ => 1,
    }
}

/// Where a command writes, and how.
pub struct Output<'a> {
    pub json: bool,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Output<'_> {
    /// Prints `value` with --json, else `text`.
    pub fn print(&mut self, value: Value, text: &str) {
        if self.json {
            let _ = writeln!(self.out, "{value}");
        } else {
            let _ = write!(self.out, "{text}");
        }
    }

    /// Prints a failure and returns its exit code.
    pub fn fail(&mut self, failure: &Failure) -> i32 {
        if self.json {
            let error = json!({ "error": { "code": failure.code, "message": failure.message } });
            let _ = writeln!(self.out, "{error}");
        } else {
            let _ = writeln!(self.err, "{}", failure.message);
        }
        failure.exit
    }
}
```

`client.rs`: anslutningen. Följ `crates/server/tests/common/mod.rs` för transporttypen, med `hyper_tls::HttpsConnector<HttpConnector>` i stället för `HttpConnector`:

```rust
//! The connection to Doris: gRPC-Web over HTTP/1.1, with the API token.

use crate::output::Failure;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use tonic::Request;
use tonic_web::{GrpcWebCall, GrpcWebClientLayer, GrpcWebClientService};

pub type Transport = GrpcWebClientService<Client<HttpsConnector<HttpConnector>, GrpcWebCall<tonic::body::Body>>>;

/// `https://` anywhere; `http://` only to this machine, so a token never
/// crosses a network in the clear.
pub fn checked_url(raw: &str) -> Result<http::Uri, Failure> {
    let uri: http::Uri = raw.parse().map_err(|_| Failure::new("insecure_url"))?;
    let local = matches!(uri.host(), Some("localhost" | "127.0.0.1"));
    match uri.scheme_str() {
        Some("https") => Ok(uri),
        Some("http") if local => Ok(uri),
        _ => Err(Failure::new("insecure_url")),
    }
}

/// Talks to one Doris with one token.
pub struct Doris {
    pub origin: http::Uri,
    token: String,
}

impl Doris {
    pub fn new(origin: http::Uri, token: String) -> Self {
        Self { origin, token }
    }

    pub fn transport(&self) -> Transport {
        let client = Client::builder(TokioExecutor::new()).build(HttpsConnector::new());
        tower::ServiceBuilder::new()
            .layer(GrpcWebClientLayer::new())
            .service(client)
    }

    /// A request carrying the token. The token goes nowhere else.
    pub fn request<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        let value = format!("Bearer {}", self.token)
            .parse()
            .expect("a token is header-safe ascii");
        request.metadata_mut().insert("authorization", value);
        request
    }

    pub fn auth(&self) -> doris_proto::auth::v1::auth_service_client::AuthServiceClient<Transport> {
        doris_proto::auth::v1::auth_service_client::AuthServiceClient::with_origin(self.transport(), self.origin.clone())
    }

    pub fn companies(&self) -> doris_proto::company::v1::company_service_client::CompanyServiceClient<Transport> {
        doris_proto::company::v1::company_service_client::CompanyServiceClient::with_origin(self.transport(), self.origin.clone())
    }

    /// Room for a 10 MiB underlag both ways.
    pub fn ledger(&self) -> doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient<Transport> {
        doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient::with_origin(self.transport(), self.origin.clone())
            .max_decoding_message_size(11 << 20)
            .max_encoding_message_size(21 << 20)
    }
}
```

Om token inte är ASCII (felaktig `DORIS_TOKEN`) ska det inte panika: kontrollera i `run` att token bara innehåller tecken som går i en header och ge annars `not_signed_in` (exit 3). Ta `http` som beroende om det behövs för `http::Uri`.

`lib.rs`: clap-definitionen och `run`:

```rust
//! doris-cli: Doris' books from the command line, for people and agents.

pub mod amount;
pub mod client;
mod commands;
pub mod output;

use clap::{Args, Parser, Subcommand};
use client::{Doris, checked_url};
use output::{Failure, Output};
use std::ffi::OsString;
use std::io::Write;

#[derive(Parser)]
#[command(name = "doris-cli", version, about = "Doris bokföring från terminalen")]
struct Cli {
    /// Answer with one JSON value (and errors as JSON), for programs and agents.
    #[arg(long, global = true)]
    json: bool,
    /// The company: org nr or id. Defaults to DORIS_COMPANY, or the only one.
    #[arg(long, global = true)]
    company: Option<String>,
    /// Run every rule on the server and show what would happen; save nothing.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Area,
}

#[derive(Subcommand)]
enum Area {
    /// Who the token belongs to.
    Auth { #[command(subcommand)] action: AuthAction },
    /// The companies the token reaches.
    Company { #[command(subcommand)] action: CompanyAction },
    /// Räkenskapsår.
    Year { #[command(subcommand)] action: YearAction },
    /// The chart of accounts.
    Account { #[command(subcommand)] action: AccountAction },
}

#[derive(Subcommand)]
enum AuthAction { Status }
#[derive(Subcommand)]
enum CompanyAction { List, View }
#[derive(Subcommand)]
enum YearAction { List }
#[derive(Subcommand)]
enum AccountAction { List }

/// The environment doris-cli reads: the token, the server, the company.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub token: Option<String>,
    pub url: Option<String>,
    pub company: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        Self {
            token: var("DORIS_TOKEN"),
            url: var("DORIS_URL"),
            company: var("DORIS_COMPANY"),
        }
    }
}

/// Parses `args` (with the program name first), runs the command and
/// returns the exit code. Writes only to `out` and `err`.
pub async fn run<I, T>(args: I, env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let json = args.iter().any(|a| a == "--json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion) => {
            let _ = write!(out, "{e}");
            return 0;
        }
        Err(e) => {
            let mut output = Output { json, out, err };
            return output.fail(&Failure::usage(e.to_string().trim().to_owned()));
        }
    };
    let mut output = Output { json: cli.json, out, err };
    match execute(cli, env, &mut output).await {
        Ok(()) => 0,
        Err(failure) => output.fail(&failure),
    }
}

async fn execute(cli: Cli, env: &Env, output: &mut Output<'_>) -> Result<(), Failure> {
    let token = env.token.clone().ok_or_else(|| Failure::new("missing_token"))?;
    let url = env.url.as_deref().ok_or_else(|| Failure::new("missing_url"))?;
    let doris = Doris::new(checked_url(url)?, token);
    let context = commands::Context {
        doris,
        company: cli.company.or_else(|| env.company.clone()),
        dry_run: cli.dry_run,
    };
    match cli.command {
        Area::Auth { action: AuthAction::Status } => commands::auth::status(&context, output).await,
        Area::Company { action: CompanyAction::List } => commands::company::list(&context, output).await,
        Area::Company { action: CompanyAction::View } => commands::company::view(&context, output).await,
        Area::Year { action: YearAction::List } => commands::year::list(&context, output).await,
        Area::Account { action: AccountAction::List } => commands::account::list(&context, output).await,
    }
}
```

`commands/mod.rs`: `Context { doris, company: Option<String>, dry_run }` och de gemensamma uppslagen:

```rust
/// The company the command is about: --company / DORIS_COMPANY (org nr,
/// with or without the dash, or id), or the token's only company.
pub async fn company(context: &Context) -> Result<cpb::CompanySummary, Failure>

/// The fiscal year's start date: --year (a year or a start date), or the
/// year today (in Sweden) falls in.
pub async fn fiscal_year(context: &Context, company_id: &str, wanted: Option<&str>) -> Result<String, Failure>
```

- `company`: `ListCompanies` med `context.doris.request(…)`; matcha `id` eller `org_nr` (jämför utan bindestreck). Ingen träff: `company_not_found`. Inget valt och flera bolag: `Failure { code: "company_ambiguous", message: format!("{}\n{}", message("company_ambiguous"), lista med "orgnr  namn" per rad), exit: 2 }`. Inget bolag alls: `company_not_found`.
- `fiscal_year`: `ListFiscalYears` → `pick_year(&starts, wanted, &today)` (lägg `pick_year` i `lib.rs` eller `commands/mod.rs` där testet ligger), dagens datum i Sverige: `jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::get("Europe/Stockholm").unwrap()).date().to_string()`. Ingen träff: `fiscal_year_not_found`.
- När `context.dry_run` är satt på ett läsande kommando: lägg `"dry_run": true` i JSON och skriv sist i textläget "(--dry-run: kommandot ändrar ingenting.)".

`commands/auth.rs`, `status`: `GetStatus` → `{"name","email"}`; text `"{namn} <{e-post}>\n"`. Ingen `current_user` i svaret: `Failure::new("not_signed_in")`.

`commands/company.rs`:
- `list`: `ListCompanies` → `[{"id","org_nr","name"}]`; text en rad per bolag `"{org_nr}  {name}\n"`.
- `view`: `company(context)` och `GetCompany` → `{"id","org_nr","name","legal_form","accounting_method","fiscal_year_start","fiscal_year_end"}` där `legal_form`/`accounting_method` är prost-namnet utan prefix i gemener (`LegalForm::as_str_name()` → `"LEGAL_FORM_AKTIEBOLAG"` → `"aktiebolag"`); text några rader `Namn: …`, `Org.nr: …`, `Räkenskapsår: … – …`.

`commands/year.rs`, `list`: `ListFiscalYears` → `[{"start","end","closed"}]`; text `"{start} – {end}  {Öppet|Stängt}\n"`.

`commands/account.rs`, `list`: `ListAccounts` → `[{"number","name","active"}]`; text `"{number}  {name}{ (inaktivt)}\n"`.

Alla RPC-fel går genom `Failure::from_status`. Ett anslutningsfel (tonic `Status` med kod `Unavailable` eller `Unknown`, eller ett transportfel) blir `connection_failed`; kontrollera i integrationstestet med `DORIS_URL=http://127.0.0.1:9` vad tonic faktiskt ger och anpassa `from_status` så att det blir `connection_failed` utan att något annat serverfel gör det.

`main.rs`:

```rust
#[tokio::main]
async fn main() {
    let env = doris_cli::Env::from_process();
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let code = doris_cli::run(std::env::args_os(), &env, &mut out, &mut err).await;
    std::process::exit(code);
}
```

- [ ] **Step 5: Fallerande integrationstester**

`crates/server/tests/doris_cli.rs`:

```rust
mod common;

use common::{TestServer, api_token, company, device};
use serde_json::Value;

struct Ran {
    code: i32,
    out: String,
    err: String,
}

async fn cli(server: &TestServer, token: Option<&str>, args: &[&str]) -> Ran {
    let env = doris_cli::Env {
        token: token.map(String::from),
        url: Some(server.base.clone()),
        company: None,
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut all = vec!["doris-cli"];
    all.extend_from_slice(args);
    let code = doris_cli::run(all, &env, &mut out, &mut err).await;
    Ran {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn json(ran: &Ran) -> Value {
    assert!(ran.err.is_empty(), "stderr with --json: {}", ran.err);
    serde_json::from_str(ran.out.trim()).unwrap_or_else(|e| panic!("not one JSON value ({e}): {}", ran.out))
}

/// Anna, her company, and a token for it with these scopes.
async fn anna_with_token(server: &TestServer, scopes: &[&str]) -> (String, String) {
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(server, &anna, "556016-0680").await;
    let secret = api_token(server, &anna, &mut annas, &[(&id, scopes)]).await;
    (id, secret)
}

#[tokio::test]
async fn auth_status_names_the_tokens_owner() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;

    let text = cli(&server, Some(&token), &["auth", "status"]).await;
    let as_json = cli(&server, Some(&token), &["auth", "status", "--json"]).await;

    assert_eq!(text.code, 0, "{}", text.err);
    assert!(text.out.contains("anna@example.se"), "{}", text.out);
    assert_eq!(json(&as_json)["email"], "anna@example.se");
    assert!(!text.out.contains(&token) && !as_json.out.contains(&token));
}

#[tokio::test]
async fn the_only_company_is_chosen_and_listed() {
    let server = TestServer::start().await;
    let (id, token) = anna_with_token(&server, &["ledger:read", "company:read"]).await;

    let list = json(&cli(&server, Some(&token), &["company", "list", "--json"]).await);
    let view = json(&cli(&server, Some(&token), &["company", "view", "--json"]).await);
    let years = json(&cli(&server, Some(&token), &["year", "list", "--json"]).await);
    let accounts = json(&cli(&server, Some(&token), &["account", "list", "--json"]).await);

    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(view["org_nr"], "556016-0680");
    assert_eq!(view["legal_form"], "aktiebolag");
    assert_eq!(years[0]["start"], "2026-01-01");
    assert!(accounts.as_array().unwrap().iter().any(|a| a["number"] == 1930));
}

#[tokio::test]
async fn several_companies_need_a_choice() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let a = company(&server, &anna, "556016-0680").await;
    let b = company(&server, &anna, "556036-0793").await;
    let token = api_token(&server, &anna, &mut annas, &[(&a, &["ledger:read"]), (&b, &["ledger:read"])]).await;

    let ambiguous = cli(&server, Some(&token), &["account", "list"]).await;
    let ambiguous_json = cli(&server, Some(&token), &["account", "list", "--json"]).await;
    let chosen = cli(&server, Some(&token), &["account", "list", "--company", "5560360793"]).await;

    assert_eq!(ambiguous.code, 2);
    assert!(ambiguous.err.contains("556016-0680") && ambiguous.err.contains("556036-0793"), "{}", ambiguous.err);
    assert_eq!(json(&ambiguous_json)["error"]["code"], "company_ambiguous");
    assert_eq!(chosen.code, 0, "{}", chosen.err);
}

#[tokio::test]
async fn missing_settings_and_bad_connections_exit_3() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;
    let no_token = cli(&server, None, &["auth", "status", "--json"]).await;
    let bad_token = cli(&server, Some("doris_nope"), &["auth", "status", "--json"]).await;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let unreachable = doris_cli::run(
        ["doris-cli", "auth", "status", "--json"],
        &doris_cli::Env { token: Some(token.clone()), url: Some("http://127.0.0.1:9".into()), company: None },
        &mut out,
        &mut err,
    )
    .await;
    let insecure = doris_cli::run(
        ["doris-cli", "auth", "status", "--json"],
        &doris_cli::Env { token: Some(token.clone()), url: Some("http://doris.example.se".into()), company: None },
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .await;

    assert_eq!((no_token.code, json(&no_token)["error"]["code"].as_str()), (3, Some("missing_token")));
    assert_eq!((bad_token.code, json(&bad_token)["error"]["code"].as_str()), (3, Some("not_signed_in")));
    assert_eq!(unreachable, 3);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("connection_failed") && !out.contains(&token), "{out}");
    assert_eq!(insecure, 3);
}

#[tokio::test]
async fn bad_arguments_exit_2_with_one_json_error() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;

    let unknown = cli(&server, Some(&token), &["account", "frobnicate", "--json"]).await;

    assert_eq!(unknown.code, 2);
    assert_eq!(json(&unknown)["error"]["code"], "usage");
}
```

- [ ] **Step 6: Kör, se dem fallera, implementera kommandona, kör igen**

Run: `cargo test -p doris-server --test doris_cli`
Expected först: kompileringsfel eller FAIL; efter att kommandona i Step 4 är på plats: PASS.

- [ ] **Step 7: Kör allt och committa**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --all --check`

```bash
git add Cargo.toml Cargo.lock crates/cli crates/server/Cargo.toml crates/server/tests/doris_cli.rs
git commit -m "Add doris-cli with auth, company, year and account commands"
```

---

### Task 4: `ver list|view|new|correct`

**Files:**
- Create: `crates/cli/src/commands/ver.rs`
- Modify: `crates/cli/src/lib.rs`, `crates/cli/src/commands/mod.rs`
- Test: enhetstester i `ver.rs`; `crates/server/tests/doris_cli.rs`

**Interfaces:**
- Consumes: Task 1:s `dry_run` och `attachments` i svaren; Task 3:s `Context`, `company`, `fiscal_year`, `Output`, `amount`.
- Produces: kommandona nedan.

Argument (clap):

```rust
#[derive(Subcommand)]
enum VerAction {
    /// The year's vouchers, newest first.
    List { #[arg(long)] year: Option<String> },
    /// One voucher with its lines and underlag.
    View { number: u32, #[arg(long)] year: Option<String> },
    /// Book a voucher (or, with --dry-run, see what would be booked).
    New(NewVoucher),
    /// Correct a voucher with a rättelse that reverses every line.
    Correct { number: u32, #[arg(long)] date: String, #[arg(long)] year: Option<String> },
}

#[derive(Args)]
struct NewVoucher {
    #[arg(long)] date: Option<String>,
    #[arg(long)] text: Option<String>,
    /// KONTO=BELOPP in kronor; repeat for each debit line.
    #[arg(long = "debit", value_name = "KONTO=BELOPP")] debits: Vec<String>,
    /// KONTO=BELOPP in kronor; repeat for each credit line.
    #[arg(long = "credit", value_name = "KONTO=BELOPP")] credits: Vec<String>,
    /// An underlag (PDF, JPEG, PNG); repeat for more.
    #[arg(long = "attach", value_name = "FIL")] attachments: Vec<std::path::PathBuf>,
    /// The whole voucher as JSON, from a file or - for stdin.
    #[arg(long, value_name = "FIL")] input: Option<String>,
}
```

`--input -` läser stdin; för testbarhet tar `run` inte stdin som parameter — läs stdin i `ver.rs` bara när värdet är `-`, och testa `--input` med en fil (tempfile, dev-dep i servern finns redan).

- [ ] **Step 1: Fallerande enhetstester i `ver.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_an_account_and_an_amount() {
        assert_eq!(line("6110=800").unwrap(), (6110, 80_000));
        assert_eq!(line("2641=200,50").unwrap(), (2641, 20_050));
        for bad in ["6110", "=800", "6110=", "61a0=800", "6110=-80", "6110=1.234"] {
            assert_eq!(line(bad).unwrap_err().code, "usage", "{bad}");
        }
    }

    #[test]
    fn json_input_says_the_same_as_the_flags() {
        let from_json = voucher_from_json(r#"{"date":"2026-02-02","text":"Kontor","lines":[
            {"account":6110,"debit":"800"},{"account":2641,"debit":200},{"account":1930,"credit":"1000.00"}],
            "attachments":[]}"#).unwrap();
        let from_flags = voucher_from_flags(
            Some("2026-02-02".into()), Some("Kontor".into()),
            &["6110=800".into(), "2641=200".into()], &["1930=1000".into()], &[],
        ).unwrap();
        assert_eq!(from_json, from_flags);
    }

    #[test]
    fn input_and_flags_do_not_mix() {
        // voucher_input(NewVoucher { input: Some(path), text: Some(..), .. }) → usage
    }
}
```

(`line(&str) -> Result<(u32, i64), Failure>`; `Voucher { date: String, text: String, lines: Vec<(u32, i64 /* debit */, i64 /* credit */)>, attachments: Vec<PathBuf> }` med `PartialEq, Debug`; i JSON-indata får `debit`/`credit` vara sträng eller heltal/tal i kronor och tolkas med `parse_kronor` på strängformen; ett tal med fler än två decimaler är `usage`. Skriv ut det tredje testet fullständigt.)

- [ ] **Step 2: Fallerande integrationstester**

Lägg till i `crates/server/tests/doris_cli.rs` (`tempfile` finns som dev-dep):

```rust
fn pdf(size: usize) -> Vec<u8> {
    let mut data = b"%PDF-1.7\n".to_vec();
    data.resize(size, b'x');
    data
}

#[tokio::test]
async fn a_voucher_is_booked_and_shown() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let receipt = dir.path().join("kvitto.pdf");
    std::fs::write(&receipt, pdf(2000)).unwrap();
    let receipt = receipt.to_str().unwrap();
    let new = ["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "Kontorsmaterial",
        "--debit", "6110=800", "--debit", "2641=200", "--credit", "1930=1000", "--attach", receipt];

    let text = cli(&server, Some(&token), &new).await;
    let mut with_json = new.to_vec();
    with_json.push("--json");
    let second = json(&cli(&server, Some(&token), &with_json).await);
    let listed = json(&cli(&server, Some(&token), &["ver", "list", "--year", "2026", "--json"]).await);
    let viewed = json(&cli(&server, Some(&token), &["ver", "view", "1", "--year", "2026", "--json"]).await);

    assert_eq!(text.code, 0, "{}", text.err);
    assert!(text.out.contains("Verifikation 1"), "{}", text.out);
    assert_eq!(second["dry_run"], false);
    assert_eq!(second["number"], 2);
    assert_eq!(second["fiscal_year_start"], "2026-01-01");
    assert_eq!(second["lines"][0], serde_json::json!({"account": 6110, "debit": "800.00", "credit": "0.00"}));
    assert_eq!(second["attachments"][0]["file_name"], "kvitto.pdf");
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[0]["number"], 2, "newest first");
    assert_eq!(viewed["text"], "Kontorsmaterial");
    assert_eq!(viewed["lines"][2]["credit"], "1000.00");
    assert_eq!(viewed["attachments"][0]["file_name"], "kvitto.pdf");
}

#[tokio::test]
async fn a_dry_run_shows_the_number_and_books_nothing() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let args = ["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "Prov",
        "--debit", "6110=100", "--credit", "1930=100", "--dry-run"];

    let text = cli(&server, Some(&token), &args).await;
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let preview = json(&cli(&server, Some(&token), &with_json).await);
    let listed = json(&cli(&server, Some(&token), &["ver", "list", "--year", "2026", "--json"]).await);

    assert_eq!(text.code, 0, "{}", text.err);
    assert!(text.out.contains("Skulle bokföras som verifikation 1"), "{}", text.out);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["number"], 1);
    assert_eq!(listed, serde_json::json!([]));
}

#[tokio::test]
async fn a_voucher_is_corrected_and_refusals_exit_1() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    cli(&server, Some(&token), &["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "Fel",
        "--debit", "6110=100", "--credit", "1930=100"]).await;

    let dry = json(&cli(&server, Some(&token), &["ver", "correct", "1", "--year", "2026", "--date", "2026-02-03", "--dry-run", "--json"]).await);
    let real = json(&cli(&server, Some(&token), &["ver", "correct", "1", "--year", "2026", "--date", "2026-02-03", "--json"]).await);
    let unbalanced = cli(&server, Some(&token), &["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "Obalans",
        "--debit", "6110=100", "--credit", "1930=99", "--json"]).await;
    let missing = cli(&server, Some(&token), &["ver", "view", "9", "--year", "2026", "--json"]).await;

    assert_eq!((dry["dry_run"].as_bool(), dry["number"].as_u64()), (Some(true), Some(2)));
    assert_eq!((real["dry_run"].as_bool(), real["number"].as_u64(), real["corrects"].as_u64()), (Some(false), Some(2), Some(1)));
    assert_eq!(unbalanced.code, 1);
    assert_eq!(json(&unbalanced)["error"]["code"], "voucher_unbalanced");
    assert_eq!(missing.code, 1);
    assert_eq!(json(&missing)["error"]["code"], "voucher_not_found");
}

#[tokio::test]
async fn a_read_only_token_cannot_book() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;

    let refused = cli(&server, Some(&token), &["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "x",
        "--debit", "6110=1", "--credit", "1930=1", "--json"]).await;

    assert_eq!(refused.code, 1);
    assert_eq!(json(&refused)["error"]["code"], "missing_scope");
}

#[tokio::test]
async fn a_voucher_comes_from_json_input() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ver.json");
    std::fs::write(&input, r#"{"date":"2026-02-02","text":"Från JSON","lines":[
        {"account":6110,"debit":"100"},{"account":1930,"credit":"100"}]}"#).unwrap();

    let booked = json(&cli(&server, Some(&token), &["ver", "new", "--year", "2026", "--input", input.to_str().unwrap(), "--json"]).await);
    let mixed = cli(&server, Some(&token), &["ver", "new", "--input", input.to_str().unwrap(), "--text", "x", "--json"]).await;

    assert_eq!(booked["text"], "Från JSON");
    assert_eq!(mixed.code, 2);
}

#[tokio::test]
async fn too_large_underlag_is_refused_before_sending() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("stor.pdf");
    std::fs::write(&big, pdf((10 << 20) + 1)).unwrap();

    let refused = cli(&server, Some(&token), &["ver", "new", "--year", "2026", "--date", "2026-02-02", "--text", "x",
        "--debit", "6110=1", "--credit", "1930=1", "--attach", big.to_str().unwrap(), "--json", "--dry-run"]).await;

    assert_eq!(refused.code, 1);
    assert_eq!(json(&refused)["error"]["code"], "attachment_too_large");
}
```

- [ ] **Step 3: Kör och se dem fallera**

Run: `cargo test -p doris-cli && cargo test -p doris-server --test doris_cli`
Expected: FAIL / kompileringsfel.

- [ ] **Step 4: Implementera `ver.rs`**

- **`list`**: `company` → `fiscal_year` → `ListVouchers`. JSON: array, nyast först (sortera på `number` fallande), `{"number","date","text","total","corrects","corrected_by","attachments","recorded_at","recorded_by"}` där `total` är summan av debet (`kronor`), `corrects`/`corrected_by` är `null` när 0, `attachments` är antalet, `recorded_by` är `recorded_by_name` (`null` om tom). Text: `"{number:>4}  {date}  {total:>12}  {text}\n"` (`total` med `display`).
- **`view NR`**: samma lista; hitta numret, annars `Failure::new("voucher_not_found")`. JSON `{"fiscal_year_start","number","date","text","lines":[{"account","debit","credit"}],"corrects","corrected_by","attachments":[{"file_name","sha256","size","content_type"}],"recorded_at","recorded_by"}`. Text: rubrikrad `"Verifikation {nr}  {datum}  {text}"`, en rad per kontering `"  {konto}  {debet:>12}  {kredit:>12}"` (tom kolumn för 0), underlag listade.
- **`new`**: `voucher_input(&NewVoucher) -> Result<Voucher, Failure>`:
  - `--input` med någon av `--date`, `--text`, `--debit`, `--credit` → `usage` ("--input kan inte kombineras med --date, --text, --debit eller --credit.").
  - Utan `--input`: `--date` och `--text` krävs, minst en `--debit`/`--credit` → annars `usage` med vilken flagga som saknas.
  - Underlag: läs varje fil; filnamnet är `file_name()` som sträng; en fil > 10 MiB → `Failure::new("attachment_too_large")`; totalt > 20 MiB → samma; filen saknas → `usage` med sökvägen.
  - Bygg `RecordVoucherRequest { company_id, date, text, lines: [VoucherLine { account, debit, credit }], attachments: [NewAttachment { file_name, data }], dry_run: context.dry_run }`. `--year` påverkar inte bokföringen (datumet avgör året) men används inte heller för något här; behåll flaggan för `list`/`view`/`correct` och ignorera den på `new` (dokumentera i README).
  - Svar → JSON `{"dry_run","fiscal_year_start","number","date","text","lines":[{"account","debit","credit"}],"attachments":[{"file_name","sha256","size","content_type"}]}`. Text: `"Verifikation {nr} i räkenskapsåret {år} bokförd ({datum}, {summa debet} kr, {n} underlag).\n"` eller med `dry_run`: `"Skulle bokföras som verifikation {nr} i räkenskapsåret {år} ({datum}, {summa} kr, {n} underlag). Ingenting sparades.\n"`; `{år}` är räkenskapsårets start (`2026-01-01`), eller bara året om det börjar 1 januari.
- **`correct NR --date D`**: `company` → `fiscal_year(--year)` → `CorrectVoucher { company_id, fiscal_year_start, number, date, dry_run }`. JSON `{"dry_run","fiscal_year_start","number","corrects": NR}`. Text: `"Verifikation {NR} rättad med verifikation {nr}.\n"` eller `"Skulle rättas med verifikation {nr}. Ingenting sparades.\n"`.

- [ ] **Step 5: Kör testerna**

Run: `cargo test -p doris-cli && cargo test -p doris-server --test doris_cli && cargo clippy --workspace -- -D warnings && cargo fmt --all --check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cli crates/server/tests/doris_cli.rs
git commit -m "Book, list, show and correct vouchers from doris-cli, with --dry-run"
```

---

### Task 5: Rapporter, README, dist och AGENTS.md

**Files:**
- Create: `crates/cli/src/commands/report.rs`, `crates/cli/README.md`
- Modify: `crates/cli/src/lib.rs`, `Makefile`, `AGENTS.md`
- Test: `crates/server/tests/doris_cli.rs`

- [ ] **Step 1: Fallerande integrationstest**

```rust
#[tokio::test]
async fn reports_read_the_books() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    cli(&server, Some(&token), &["ver", "new", "--date", "2026-02-02", "--text", "Försäljning",
        "--debit", "1930=1250", "--credit", "3001=1000", "--credit", "2611=250"]).await;

    let balance = json(&cli(&server, Some(&token), &["report", "trial-balance", "--year", "2026", "--json"]).await);
    let ledger = json(&cli(&server, Some(&token), &["report", "ledger", "1930", "--year", "2026", "--json"]).await);
    let statements = json(&cli(&server, Some(&token), &["report", "statements", "--year", "2026", "--json"]).await);
    let text = cli(&server, Some(&token), &["report", "trial-balance", "--year", "2026"]).await;

    let bank = balance["rows"].as_array().unwrap().iter().find(|r| r["account"] == 1930).unwrap();
    assert_eq!(bank["debit"], "1250.00");
    assert_eq!(bank["closing"], "1250.00");
    assert_eq!(ledger["account"], 1930);
    assert_eq!(ledger["entries"][0]["balance"], "1250.00");
    assert!(statements["income_statement"].as_array().unwrap().iter().any(|l| l["kind"] == "item"));
    assert!(text.out.contains("1 250,00"), "{}", text.out);
}
```

- [ ] **Step 2: Kör och se det fallera**

Run: `cargo test -p doris-server --test doris_cli reports`
Expected: FAIL (`report` finns inte).

- [ ] **Step 3: Implementera `report.rs`**

```rust
#[derive(Subcommand)]
enum ReportAction {
    /// Saldobalans.
    TrialBalance { #[arg(long)] year: Option<String> },
    /// Huvudbok for one account.
    Ledger { account: u32, #[arg(long)] year: Option<String> },
    /// Resultat- och balansräkning.
    Statements { #[arg(long)] year: Option<String> },
}
```

- **`trial-balance`**: `GetTrialBalance` → `{"fiscal_year_start","rows":[{"account","name","opening","debit","credit","closing"}]}` med `closing = opening + debit − credit`, alla som `kronor`. Text: en rad per konto `"{konto}  {namn:<30}  {ib:>12}  {debet:>12}  {kredit:>12}  {ub:>12}"` med rubrikrad `Konto  Namn  IB  Debet  Kredit  UB`.
- **`ledger KONTO`**: `GetAccountLedger` → `{"fiscal_year_start","account","opening","entries":[{"date","number","text","debit","credit","balance"}]}`. Text: `Ingående balans {ib}` och en rad per post.
- **`statements`**: `GetFinancialStatements` → `{"fiscal_year_start","previous_fiscal_year_start","income_statement":[{"label","kind","amount","previous"}],"balance_sheet":[…],"difference"}` där `kind` är `heading`/`item`/`subtotal` (prost-namnet utan `STATEMENT_LINE_KIND_` i gemener), `amount`/`previous` som `kronor` (`previous` och `previous_fiscal_year_start` `null` när de saknas; `amount` `null` för `heading`). Text: rubriker som de är, poster indragna med belopp.

- [ ] **Step 4: README, dist, AGENTS.md**

`crates/cli/README.md` (engelska, för människor och agenter): installation (`cargo install --path crates/cli` eller `make dist`), miljövariablerna, globala flaggor, exit-koder, felobjektet, och för varje kommando ett exempel i textläge och dess `--json`-form (ta formerna från Task 3–5). Säg att `--year` ignoreras av `ver new` (datumet avgör året) och att `--dry-run` kontrolleras på servern och inte sparar något.

`Makefile`, `dist`: efter `cargo build --release -p doris-server`, lägg till `cargo build --release -p doris-cli`, `rm -f $(DIST)/doris-cli` och `cp target/release/doris-cli $(DIST)/doris-cli`, och nämn den i `@echo`-raden.

`AGENTS.md`: i Layout `crates/cli           doris-cli: the command line for people and agents (API token)`. Ett nytt stycke (under API eller efter Frontend):

```markdown
## doris-cli
- `crates/cli` is a gh-style command line (`doris-cli <area> <action>`) over
  the same gRPC-Web API, for people and AI agents. It reads `DORIS_TOKEN`
  (an API token, never printed or logged), `DORIS_URL` (required; `http://`
  only to localhost) and `DORIS_COMPANY` (optional).
- Every command takes `--json`: exactly one JSON value on stdout, errors as
  `{"error":{"code","message"}}`, amounts as kronor strings ("1250.00").
  Exit codes: 1 the server refused, 2 usage, 3 token or connection.
- `--dry-run` is checked on the server: `RecordVoucher`/`CorrectVoucher`
  take `dry_run`, run everything in the same transaction and roll back; the
  call does not count as the token's use.
- The Swedish error texts live in `doris_proto::messages`, shared with the
  web app. Commands grow area by area (invoicing, payroll, VAT next); their
  JSON shapes are in `crates/cli/README.md`.
```

- [ ] **Step 5: Kör allt och committa**

Run: `make test && cargo clippy --workspace -- -D warnings && cargo clippy -p doris-web --target wasm32-unknown-unknown -- -D warnings && cargo fmt --all --check && make dist`
Expected: PASS, och `target/dist/doris-cli --help` skriver hjälpen.

```bash
git add crates/cli crates/server/tests/doris_cli.rs Makefile AGENTS.md
git commit -m "Add doris-cli reports, its README and the release binary"
```
