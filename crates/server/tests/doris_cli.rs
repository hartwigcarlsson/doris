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
    serde_json::from_str(ran.out.trim())
        .unwrap_or_else(|e| panic!("not one JSON value ({e}): {}", ran.out))
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
    assert!(
        accounts
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["number"] == 1930)
    );
}

#[tokio::test]
async fn several_companies_need_a_choice() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let a = company(&server, &anna, "556016-0680").await;
    let b = company(&server, &anna, "556036-0793").await;
    let token = api_token(
        &server,
        &anna,
        &mut annas,
        &[(&a, &["ledger:read"]), (&b, &["ledger:read"])],
    )
    .await;

    let ambiguous = cli(&server, Some(&token), &["account", "list"]).await;
    let ambiguous_json = cli(&server, Some(&token), &["account", "list", "--json"]).await;
    let chosen = cli(
        &server,
        Some(&token),
        &["account", "list", "--company", "5560360793"],
    )
    .await;

    assert_eq!(ambiguous.code, 2);
    assert!(
        ambiguous.err.contains("556016-0680") && ambiguous.err.contains("556036-0793"),
        "{}",
        ambiguous.err
    );
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
        &doris_cli::Env {
            token: Some(token.clone()),
            url: Some("http://127.0.0.1:9".into()),
            company: None,
        },
        &mut out,
        &mut err,
    )
    .await;
    let insecure = doris_cli::run(
        ["doris-cli", "auth", "status", "--json"],
        &doris_cli::Env {
            token: Some(token.clone()),
            url: Some("http://doris.example.se".into()),
            company: None,
        },
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .await;

    assert_eq!(
        (no_token.code, json(&no_token)["error"]["code"].as_str()),
        (3, Some("missing_token"))
    );
    assert_eq!(
        (bad_token.code, json(&bad_token)["error"]["code"].as_str()),
        (3, Some("not_signed_in"))
    );
    assert_eq!(unreachable, 3);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("connection_failed") && !out.contains(&token),
        "{out}"
    );
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

#[tokio::test]
async fn dry_run_keeps_lists_lists_and_flags_objects() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;

    let accounts = json(
        &cli(
            &server,
            Some(&token),
            &["account", "list", "--dry-run", "--json"],
        )
        .await,
    );
    let status = json(
        &cli(
            &server,
            Some(&token),
            &["auth", "status", "--dry-run", "--json"],
        )
        .await,
    );
    let text = cli(&server, Some(&token), &["account", "list", "--dry-run"]).await;

    assert!(accounts.is_array());
    assert_eq!(status["dry_run"], true);
    assert!(
        text.out
            .ends_with("(--dry-run: kommandot ändrar ingenting.)\n"),
        "{}",
        text.out
    );
}

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
    let new = [
        "ver",
        "new",
        "--year",
        "2026",
        "--date",
        "2026-02-02",
        "--text",
        "Kontorsmaterial",
        "--debit",
        "6110=800",
        "--debit",
        "2641=200",
        "--credit",
        "1930=1000",
        "--attach",
        receipt,
    ];

    let text = cli(&server, Some(&token), &new).await;
    let mut with_json = new.to_vec();
    with_json.push("--json");
    let second = json(&cli(&server, Some(&token), &with_json).await);
    let listed = json(
        &cli(
            &server,
            Some(&token),
            &["ver", "list", "--year", "2026", "--json"],
        )
        .await,
    );
    let viewed = json(
        &cli(
            &server,
            Some(&token),
            &["ver", "view", "1", "--year", "2026", "--json"],
        )
        .await,
    );

    assert_eq!(text.code, 0, "{}", text.err);
    assert!(text.out.contains("Verifikation 1"), "{}", text.out);
    assert_eq!(second["dry_run"], false);
    assert_eq!(second["number"], 2);
    assert_eq!(second["fiscal_year_start"], "2026-01-01");
    assert_eq!(
        second["lines"][0],
        serde_json::json!({"account": 6110, "debit": "800.00", "credit": "0.00"})
    );
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
    let args = [
        "ver",
        "new",
        "--year",
        "2026",
        "--date",
        "2026-02-02",
        "--text",
        "Prov",
        "--debit",
        "6110=100",
        "--credit",
        "1930=100",
        "--dry-run",
    ];

    let text = cli(&server, Some(&token), &args).await;
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let preview = json(&cli(&server, Some(&token), &with_json).await);
    let listed = json(
        &cli(
            &server,
            Some(&token),
            &["ver", "list", "--year", "2026", "--json"],
        )
        .await,
    );

    assert_eq!(text.code, 0, "{}", text.err);
    assert!(
        text.out.contains("Skulle bokföras som verifikation 1"),
        "{}",
        text.out
    );
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["number"], 1);
    assert_eq!(listed, serde_json::json!([]));
}

#[tokio::test]
async fn a_voucher_is_corrected_and_refusals_exit_1() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--year",
            "2026",
            "--date",
            "2026-02-02",
            "--text",
            "Fel",
            "--debit",
            "6110=100",
            "--credit",
            "1930=100",
        ],
    )
    .await;

    let dry = json(
        &cli(
            &server,
            Some(&token),
            &[
                "ver",
                "correct",
                "1",
                "--year",
                "2026",
                "--date",
                "2026-02-03",
                "--dry-run",
                "--json",
            ],
        )
        .await,
    );
    let real = json(
        &cli(
            &server,
            Some(&token),
            &[
                "ver",
                "correct",
                "1",
                "--year",
                "2026",
                "--date",
                "2026-02-03",
                "--json",
            ],
        )
        .await,
    );
    let unbalanced = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--year",
            "2026",
            "--date",
            "2026-02-02",
            "--text",
            "Obalans",
            "--debit",
            "6110=100",
            "--credit",
            "1930=99",
            "--json",
        ],
    )
    .await;
    let missing = cli(
        &server,
        Some(&token),
        &["ver", "view", "9", "--year", "2026", "--json"],
    )
    .await;

    assert_eq!(
        (dry["dry_run"].as_bool(), dry["number"].as_u64()),
        (Some(true), Some(2))
    );
    assert_eq!(
        (
            real["dry_run"].as_bool(),
            real["number"].as_u64(),
            real["corrects"].as_u64()
        ),
        (Some(false), Some(2), Some(1))
    );
    assert_eq!(unbalanced.code, 1);
    assert_eq!(json(&unbalanced)["error"]["code"], "voucher_unbalanced");
    assert_eq!(missing.code, 1);
    assert_eq!(json(&missing)["error"]["code"], "voucher_not_found");
}

#[tokio::test]
async fn a_read_only_token_cannot_book() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read"]).await;

    let refused = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--year",
            "2026",
            "--date",
            "2026-02-02",
            "--text",
            "x",
            "--debit",
            "6110=1",
            "--credit",
            "1930=1",
            "--json",
        ],
    )
    .await;

    assert_eq!(refused.code, 1);
    assert_eq!(json(&refused)["error"]["code"], "missing_scope");
}

#[tokio::test]
async fn a_voucher_comes_from_json_input() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ver.json");
    std::fs::write(
        &input,
        r#"{"date":"2026-02-02","text":"Från JSON","lines":[
        {"account":6110,"debit":"100"},{"account":1930,"credit":"100"}]}"#,
    )
    .unwrap();

    let booked = json(
        &cli(
            &server,
            Some(&token),
            &[
                "ver",
                "new",
                "--year",
                "2026",
                "--input",
                input.to_str().unwrap(),
                "--json",
            ],
        )
        .await,
    );
    let mixed = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--input",
            input.to_str().unwrap(),
            "--text",
            "x",
            "--json",
        ],
    )
    .await;

    assert_eq!(booked["text"], "Från JSON");
    assert_eq!(mixed.code, 2);
    assert_eq!(json(&mixed)["error"]["code"], "usage");
}

#[tokio::test]
async fn too_large_underlag_is_refused_before_sending() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("stor.pdf");
    std::fs::write(&big, pdf((10 << 20) + 1)).unwrap();

    let refused = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--year",
            "2026",
            "--date",
            "2026-02-02",
            "--text",
            "x",
            "--debit",
            "6110=1",
            "--credit",
            "1930=1",
            "--attach",
            big.to_str().unwrap(),
            "--json",
            "--dry-run",
        ],
    )
    .await;

    assert_eq!(refused.code, 1);
    assert_eq!(json(&refused)["error"]["code"], "attachment_too_large");
}

#[tokio::test]
async fn the_total_of_underlag_is_limited_to_20_mib() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        dir.path().join("a.pdf"),
        dir.path().join("b.pdf"),
        dir.path().join("c.pdf"),
    );
    for file in [&a, &b, &c] {
        std::fs::write(file, pdf(10 << 20)).unwrap();
    }

    let refused = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "x",
            "--debit",
            "6110=1",
            "--credit",
            "1930=1",
            "--attach",
            a.to_str().unwrap(),
            "--attach",
            b.to_str().unwrap(),
            "--attach",
            c.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    )
    .await;

    assert_eq!(refused.code, 1);
    assert_eq!(json(&refused)["error"]["code"], "attachment_too_large");
}

#[tokio::test]
async fn a_missing_attach_file_is_a_usage_error_naming_the_path() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;

    let refused = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "x",
            "--debit",
            "6110=1",
            "--credit",
            "1930=1",
            "--attach",
            "/finns/inte/kvitto.pdf",
        ],
    )
    .await;

    assert_eq!(refused.code, 2);
    assert!(
        refused.err.contains("/finns/inte/kvitto.pdf"),
        "{}",
        refused.err
    );
    let as_json = cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "x",
            "--debit",
            "6110=1",
            "--credit",
            "1930=1",
            "--attach",
            "/finns/inte/kvitto.pdf",
            "--json",
        ],
    )
    .await;
    assert_eq!(json(&as_json)["error"]["code"], "usage");
}

#[tokio::test]
async fn corrections_link_both_ways_and_others_are_null() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "Fel",
            "--debit",
            "6110=100",
            "--credit",
            "1930=100",
        ],
    )
    .await;

    let before = json(&cli(&server, Some(&token), &["ver", "list", "--json"]).await);
    cli(
        &server,
        Some(&token),
        &["ver", "correct", "1", "--date", "2026-02-03"],
    )
    .await;
    let after = json(&cli(&server, Some(&token), &["ver", "list", "--json"]).await);

    assert_eq!(before[0]["corrects"], Value::Null);
    assert_eq!(before[0]["corrected_by"], Value::Null);
    assert_eq!(
        (after[0]["number"].as_u64(), after[0]["corrects"].as_u64()),
        (Some(2), Some(1))
    );
    assert_eq!(after[0]["corrected_by"], Value::Null);
    assert_eq!(
        (
            after[1]["number"].as_u64(),
            after[1]["corrected_by"].as_u64()
        ),
        (Some(1), Some(2))
    );
    assert_eq!(after[1]["corrects"], Value::Null);
}

#[tokio::test]
async fn the_texts_are_exact() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let new = |extra: &'static [&'static str]| {
        let mut args = vec![
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "Kontor",
            "--debit",
            "6110=800",
            "--debit",
            "2641=200",
            "--credit",
            "1930=1000",
        ];
        args.extend_from_slice(extra);
        args
    };

    let dry = cli(&server, Some(&token), &new(&["--dry-run"])).await;
    let booked = cli(&server, Some(&token), &new(&[])).await;
    let dry_fix = cli(
        &server,
        Some(&token),
        &["ver", "correct", "1", "--date", "2026-02-03", "--dry-run"],
    )
    .await;
    let fix = cli(
        &server,
        Some(&token),
        &["ver", "correct", "1", "--date", "2026-02-03"],
    )
    .await;

    assert_eq!(
        dry.out,
        "Skulle bokföras som verifikation 1 i räkenskapsåret 2026 (2026-02-02, 1 000,00 kr, 0 underlag). Ingenting sparades.\n"
    );
    assert_eq!(
        booked.out,
        "Verifikation 1 i räkenskapsåret 2026 bokförd (2026-02-02, 1 000,00 kr, 0 underlag).\n"
    );
    assert_eq!(
        dry_fix.out,
        "Skulle rättas med verifikation 2. Ingenting sparades.\n"
    );
    assert_eq!(fix.out, "Verifikation 1 rättad med verifikation 2.\n");
}

#[tokio::test]
async fn reports_read_the_books() {
    let server = TestServer::start().await;
    let (_, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    cli(
        &server,
        Some(&token),
        &[
            "ver",
            "new",
            "--date",
            "2026-02-02",
            "--text",
            "Försäljning",
            "--debit",
            "1930=1250",
            "--credit",
            "3001=1000",
            "--credit",
            "2611=250",
        ],
    )
    .await;

    let balance = json(
        &cli(
            &server,
            Some(&token),
            &["report", "trial-balance", "--year", "2026", "--json"],
        )
        .await,
    );
    let ledger = json(
        &cli(
            &server,
            Some(&token),
            &["report", "ledger", "1930", "--year", "2026", "--json"],
        )
        .await,
    );
    let statements = json(
        &cli(
            &server,
            Some(&token),
            &["report", "statements", "--year", "2026", "--json"],
        )
        .await,
    );
    let text = cli(
        &server,
        Some(&token),
        &["report", "trial-balance", "--year", "2026"],
    )
    .await;

    let bank = balance["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["account"] == 1930)
        .unwrap();
    assert_eq!(bank["debit"], "1250.00");
    assert_eq!(bank["closing"], "1250.00");
    assert_eq!(ledger["account"], 1930);
    assert_eq!(ledger["entries"][0]["balance"], "1250.00");
    assert!(
        statements["income_statement"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["kind"] == "item")
    );
    assert!(text.out.contains("1 250,00"), "{}", text.out);
}
