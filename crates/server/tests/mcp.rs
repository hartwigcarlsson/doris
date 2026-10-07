mod common;

use common::{TestServer, api_token, company, device, http, post_json};
use serde_json::{Value, json};

const V2026: &str = "2026-07-28";

/// Anna, her company, and a token for it with these scopes.
async fn anna_with_token(server: &TestServer, scopes: &[&str]) -> (String, String, String) {
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(server, &anna, "556016-0680").await;
    let secret = api_token(server, &anna, &mut annas, &[(&id, scopes)]).await;
    (anna, id, secret)
}

fn message(id: Value, method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": V2026});
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// A 2026-07-28 request with matching headers.
async fn rpc(
    server: &TestServer,
    token: &str,
    id: Value,
    method: &str,
    params: Value,
) -> (u16, Value) {
    let body = message(id, method, params.clone());
    let bearer = format!("Bearer {token}");
    let name = params["name"].as_str().unwrap_or_default().to_owned();
    let mut headers = vec![
        ("authorization", bearer.as_str()),
        ("mcp-protocol-version", V2026),
        ("mcp-method", method),
    ];
    if method == "tools/call" {
        headers.push(("mcp-name", name.as_str()));
    }
    raw(server, &headers, body.to_string().into_bytes()).await
}

async fn raw(server: &TestServer, headers: &[(&str, &str)], body: Vec<u8>) -> (u16, Value) {
    let response = post_json(&format!("{}/mcp", server.base), headers, body).await;
    let status = response.status().as_u16();
    let value = serde_json::from_str(response.body()).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn discover_names_doris_its_versions_tools_and_instructions() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;

    let (status, answer) = rpc(&server, &token, json!("d-1"), "server/discover", json!({})).await;

    assert_eq!(status, 200);
    assert_eq!(answer["id"], "d-1");
    let result = &answer["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"], json!([V2026, "2025-11-25"]));
    assert_eq!(result["capabilities"], json!({"tools": {}}));
    let info = &result["_meta"]["io.modelcontextprotocol/serverInfo"];
    assert_eq!(info["name"], "doris");
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("record_voucher")
    );
}

#[tokio::test]
async fn an_older_client_initializes_and_is_acknowledged() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;
    let bearer = format!("Bearer {token}");
    let auth = [("authorization", bearer.as_str())];

    let (status, answer) = raw(
        &server,
        &auth,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "t", "version": "1"}}
        })
        .to_string()
        .into_bytes(),
    )
    .await;
    let (ack, _) = raw(
        &server,
        &auth,
        json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        })
        .to_string()
        .into_bytes(),
    )
    .await;
    let older = [
        ("authorization", bearer.as_str()),
        ("mcp-protocol-version", "2025-11-25"),
    ];
    let (listed, tools) = raw(
        &server,
        &older,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list"
        })
        .to_string()
        .into_bytes(),
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(answer["result"]["serverInfo"]["name"], "doris");
    assert!(answer["result"]["instructions"].is_string());
    assert_eq!(ack, 202);
    assert_eq!(listed, 200);
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 12);
}

#[tokio::test]
async fn tools_are_listed_and_ping_answers() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;

    let (_, tools) = rpc(&server, &token, json!(1), "tools/list", json!({})).await;
    let (_, ping) = rpc(&server, &token, json!(2), "ping", json!({})).await;

    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"record_voucher") && names.contains(&"trial_balance"));
    assert_eq!(ping["result"]["resultType"], "complete");
}

#[tokio::test]
async fn without_a_valid_token_it_is_401_even_with_a_session() {
    let server = TestServer::start().await;
    let (anna, _, token) = anna_with_token(&server, &["ledger:read"]).await;
    let body = || {
        message(json!(1), "tools/list", json!({}))
            .to_string()
            .into_bytes()
    };
    let cookie = format!("{}={anna}", doris_server::SESSION_COOKIE);
    let version = ("mcp-protocol-version", V2026);
    let method = ("mcp-method", "tools/list");

    let (none, _) = raw(&server, &[version, method], body()).await;
    let (unknown, _) = raw(
        &server,
        &[("authorization", "Bearer doris_nope"), version, method],
        body(),
    )
    .await;
    let (session, _) = raw(
        &server,
        &[("cookie", cookie.as_str()), version, method],
        body(),
    )
    .await;
    let response = post_json(&format!("{}/mcp", server.base), &[version, method], body()).await;

    assert_eq!((none, unknown, session), (401, 401, 401));
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    assert!(!response.body().contains(&token));
}

#[tokio::test]
async fn a_foreign_origin_is_403_and_the_apps_own_is_allowed() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;
    let bearer = format!("Bearer {token}");
    let own = server.origin.as_str().trim_end_matches('/').to_owned();
    let body = || {
        message(json!(1), "ping", json!({}))
            .to_string()
            .into_bytes()
    };
    let base = [
        ("authorization", bearer.as_str()),
        ("mcp-protocol-version", V2026),
        ("mcp-method", "ping"),
    ];

    let (foreign, _) = raw(
        &server,
        &[
            base[0],
            base[1],
            base[2],
            ("origin", "https://evil.example"),
        ],
        body(),
    )
    .await;
    let (allowed, _) = raw(
        &server,
        &[base[0], base[1], base[2], ("origin", own.as_str())],
        body(),
    )
    .await;

    assert_eq!((foreign, allowed), (403, 200));
}

#[tokio::test]
async fn headers_must_match_the_body_and_the_version_must_be_known() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;
    let bearer = format!("Bearer {token}");
    let auth = ("authorization", bearer.as_str());
    let list = message(json!(1), "tools/list", json!({}))
        .to_string()
        .into_bytes();

    let (method, mismatch) = raw(
        &server,
        &[
            auth,
            ("mcp-protocol-version", V2026),
            ("mcp-method", "ping"),
        ],
        list.clone(),
    )
    .await;
    let (version, _) = raw(
        &server,
        &[
            auth,
            ("mcp-protocol-version", "2025-11-25"),
            ("mcp-method", "tools/list"),
        ],
        list.clone(),
    )
    .await;
    let (name, _) = rpc_with_name(&server, &bearer, "whoami", "list_accounts").await;
    let (unknown, unsupported) = raw(
        &server,
        &[
            auth,
            ("mcp-protocol-version", "1900-01-01"),
            ("mcp-method", "tools/list"),
        ],
        list,
    )
    .await;

    assert_eq!(
        (method, mismatch["error"]["code"].clone()),
        (400, json!(-32020))
    );
    assert_eq!(version, 400);
    assert_eq!(name, 400);
    assert_eq!(unknown, 400);
    assert_eq!(unsupported["error"]["code"], -32022);
    assert_eq!(
        unsupported["error"]["data"]["supported"],
        json!([V2026, "2025-11-25"])
    );
}

/// A tools/call whose Mcp-Name header says `header` but whose body says `body`.
async fn rpc_with_name(
    server: &TestServer,
    bearer: &str,
    header: &str,
    body: &str,
) -> (u16, Value) {
    let message = message(
        json!(1),
        "tools/call",
        json!({"name": body, "arguments": {}}),
    );
    raw(
        server,
        &[
            ("authorization", bearer),
            ("mcp-protocol-version", V2026),
            ("mcp-method", "tools/call"),
            ("mcp-name", header),
        ],
        message.to_string().into_bytes(),
    )
    .await
}

#[tokio::test]
async fn malformed_batched_unknown_and_oversized_requests_are_refused() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;
    let bearer = format!("Bearer {token}");
    let auth = [("authorization", bearer.as_str())];

    let (parse, parse_error) = raw(&server, &auth, b"{not json".to_vec()).await;
    let (batch, batch_error) = raw(&server, &auth, b"[]".to_vec()).await;
    let (unknown, unknown_error) =
        rpc(&server, &token, json!(7), "resources/list", json!({})).await;
    let (big, _) = raw(&server, &auth, vec![b' '; (1 << 20) + 1]).await;
    let get = http(http::Method::GET, &format!("{}/mcp", server.base), &auth).await;

    assert_eq!(
        (parse, parse_error["error"]["code"].clone()),
        (400, json!(-32700))
    );
    assert_eq!(
        (batch, batch_error["error"]["code"].clone()),
        (400, json!(-32600))
    );
    assert_eq!(
        (unknown, unknown_error["error"]["code"].clone()),
        (404, json!(-32601))
    );
    assert_eq!(unknown_error["id"], 7);
    assert_eq!(big, 413);
    assert_eq!(get.status(), 405);
}

async fn tool(
    server: &TestServer,
    token: &str,
    name: &str,
    arguments: Value,
) -> (bool, Value, String) {
    let params = if arguments.is_null() {
        json!({"name": name})
    } else {
        json!({"name": name, "arguments": arguments})
    };
    let body = message(json!(1), "tools/call", params);
    let bearer = format!("Bearer {token}");
    let response = post_json(
        &format!("{}/mcp", server.base),
        &[
            ("authorization", bearer.as_str()),
            ("mcp-protocol-version", V2026),
            ("mcp-method", "tools/call"),
            ("mcp-name", name),
        ],
        body.to_string().into_bytes(),
    )
    .await;
    assert_eq!(response.status(), 200, "{}", response.body());
    let raw = response.body().clone();
    let answer: Value = serde_json::from_str(&raw).unwrap();
    let result = &answer["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    (
        result["isError"].as_bool().unwrap(),
        serde_json::from_str(text).unwrap(),
        raw,
    )
}

fn sale() -> Value {
    json!({
        "date": "2026-02-02", "text": "Försäljning",
        "lines": [
            {"account": 1930, "debit": "1250.00"},
            {"account": 3001, "credit": "1000.00"},
            {"account": 2611, "credit": 250}
        ]
    })
}

#[tokio::test]
async fn a_rehearsal_saves_nothing_and_the_booking_records_the_token() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    let mut rehearsal = sale();
    rehearsal["dry_run"] = json!(true);

    let (failed, dry, _) = tool(&server, &token, "record_voucher", rehearsal).await;
    let (_, before, _) = tool(&server, &token, "list_vouchers", json!({})).await;
    let (_, booked, raw) = tool(&server, &token, "record_voucher", sale()).await;

    assert!(!failed);
    assert_eq!(
        (dry["dry_run"].clone(), dry["number"].clone()),
        (json!(true), json!(1))
    );
    assert_eq!(before, json!([]));
    assert_eq!(
        (booked["dry_run"].clone(), booked["number"].clone()),
        (json!(false), json!(1))
    );
    assert_eq!(booked["attachments"], json!([]));
    assert!(!raw.contains(&token));
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM events WHERE event_type = 'VoucherRecorded'
         ORDER BY global_position DESC LIMIT 1",
    )
    .fetch_one(&server.pool)
    .await
    .unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    assert!(metadata["via_token"].is_string());
}

#[tokio::test]
async fn a_correction_points_at_the_original() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    tool(&server, &token, "record_voucher", sale()).await;
    let today = jiff::Zoned::now()
        .with_time_zone(jiff::tz::TimeZone::get("Europe/Stockholm").unwrap())
        .date();
    let date = if today.year() == 2026 {
        today.to_string()
    } else {
        "2026-12-31".into()
    };

    let (failed, corrected, _) = tool(
        &server,
        &token,
        "correct_voucher",
        json!({"number": 1, "date": date, "year": "2026"}),
    )
    .await;

    assert!(!failed, "{corrected}");
    assert_eq!(
        (corrected["number"].clone(), corrected["corrects"].clone()),
        (json!(2), json!(1))
    );
}

#[tokio::test]
async fn refusals_are_tool_errors_with_the_servers_code() {
    let server = TestServer::start().await;
    let (_, _, reader) = anna_with_token(&server, &["ledger:read"]).await;

    let (failed, error, raw) = tool(&server, &reader, "record_voucher", sale()).await;

    assert!(failed);
    assert_eq!(error["error"]["code"], "missing_scope");
    assert!(error["error"]["message"].as_str().unwrap().len() > 3);
    assert!(!raw.contains(&reader));
}

#[tokio::test]
async fn several_companies_need_a_choice_with_or_without_the_dash() {
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

    let (failed, ambiguous, _) = tool(&server, &token, "list_accounts", json!({})).await;
    let (dashed_failed, _, _) = tool(
        &server,
        &token,
        "list_accounts",
        json!({"company": "556036-0793"}),
    )
    .await;
    let (plain_failed, _, _) = tool(
        &server,
        &token,
        "list_accounts",
        json!({"company": "5560360793"}),
    )
    .await;

    assert!(failed);
    assert_eq!(ambiguous["error"]["code"], "company_ambiguous");
    assert!(
        ambiguous["error"]["message"]
            .as_str()
            .unwrap()
            .contains("556016-0680")
    );
    assert!(!dashed_failed && !plain_failed);
}

#[tokio::test]
async fn a_tool_without_parameters_needs_no_arguments() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;

    let (failed, me, _) = tool(&server, &token, "whoami", Value::Null).await;

    assert!(!failed);
    assert_eq!(me["email"], "anna@example.se");
}

#[tokio::test]
async fn an_unknown_tool_is_invalid_params() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read"]).await;

    let (status, answer) = rpc(
        &server,
        &token,
        json!(1),
        "tools/call",
        json!({"name": "delete_voucher", "arguments": {}}),
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(answer["error"]["code"], -32602);
}

#[tokio::test]
async fn the_tools_answer_exactly_as_doris_cli_does() {
    let server = TestServer::start().await;
    let (_, _, token) = anna_with_token(&server, &["ledger:read", "ledger:write"]).await;
    tool(&server, &token, "record_voucher", sale()).await;
    let env = doris_cli::Env {
        token: Some(token.clone()),
        url: Some(server.base.clone()),
        company: None,
    };
    let cli = |args: &'static [&'static str]| {
        let env = env.clone();
        async move {
            let mut out = Vec::new();
            let mut all = vec!["doris-cli", "--json"];
            all.extend_from_slice(args);
            doris_cli::run(all, &env, &mut out, &mut Vec::new()).await;
            serde_json::from_slice::<Value>(&out).unwrap()
        }
    };

    let (_, vouchers, _) = tool(&server, &token, "list_vouchers", json!({"year": "2026"})).await;
    let (_, balance, _) = tool(&server, &token, "trial_balance", json!({"year": "2026"})).await;

    assert_eq!(vouchers, cli(&["ver", "list", "--year", "2026"]).await);
    assert_eq!(
        balance,
        cli(&["report", "trial-balance", "--year", "2026"]).await
    );
}
