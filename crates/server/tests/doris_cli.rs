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
