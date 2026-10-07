//! MCP over Streamable HTTP, without sessions: `POST /mcp` with an API
//! token. The tools are doris-cli's commands (`doris_cli::tools`); this
//! module only speaks the protocol. Scopes are not checked here: every
//! tool calls the gRPC services through `auth_gate`, like doris-cli.

use crate::grpc;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::sync::Arc;

const VERSIONS: [&str; 2] = ["2026-07-28", "2025-11-25"];
const MAX_REQUEST: usize = 1 << 20;

struct Mcp {
    pool: SqlitePool,
    // Used by tools/call (Task 4).
    #[allow(dead_code)]
    grpc: Router,
    origins: Vec<HeaderValue>,
}

pub(crate) fn routes(pool: SqlitePool, grpc: Router, origins: Vec<HeaderValue>) -> Router {
    Router::new()
        .route("/mcp", post(handle))
        .layer(DefaultBodyLimit::max(MAX_REQUEST))
        .with_state(Arc::new(Mcp {
            pool,
            grpc,
            origins,
        }))
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn error(status: StatusCode, id: Value, code: i64, message: &str, data: Option<Value>) -> Response {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json_response(status, json!({"jsonrpc": "2.0", "id": id, "error": error}))
}

fn result(id: Value, mut result: Value) -> Response {
    result["resultType"] = json!("complete");
    json_response(
        StatusCode::OK,
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
}

fn unauthorized() -> Response {
    let mut response = error(
        StatusCode::UNAUTHORIZED,
        Value::Null,
        -32600,
        "not_signed_in",
        None,
    );
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// 2026-07-28 puts the version, method and tool name in headers too;
/// they must agree with the body. Without the header, the client is
/// 2025-11-25.
#[allow(clippy::result_large_err)] // the Err is the response we send
fn check_headers(headers: &HeaderMap, id: &Value, message: &Value) -> Result<(), Response> {
    let Some(version) = header_str(headers, "mcp-protocol-version") else {
        return Ok(());
    };
    if !VERSIONS.contains(&version) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            id.clone(),
            -32022,
            "Unsupported protocol version",
            Some(json!({"supported": VERSIONS, "requested": version})),
        ));
    }
    let mismatch = |what: &str| {
        Err(error(
            StatusCode::BAD_REQUEST,
            id.clone(),
            -32020,
            &format!("Header mismatch: {what}"),
            None,
        ))
    };
    // 2025-11-25 clients send no _meta version; when one is there, it must agree.
    let meta = message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str();
    if meta.is_some_and(|m| m != version) || (version == VERSIONS[0] && meta.is_none()) {
        return mismatch("MCP-Protocol-Version");
    }
    if version == "2025-11-25" {
        return Ok(());
    }
    if header_str(headers, "mcp-method") != message["method"].as_str() {
        return mismatch("Mcp-Method");
    }
    if message["method"] == "tools/call"
        && header_str(headers, "mcp-name") != message["params"]["name"].as_str()
    {
        return mismatch("Mcp-Name");
    }
    Ok(())
}

fn server_info() -> Value {
    json!({"name": "doris", "version": env!("CARGO_PKG_VERSION")})
}

async fn handle(State(mcp): State<Arc<Mcp>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN)
        && !mcp.origins.contains(origin)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(secret) = grpc::bearer(&headers) else {
        return unauthorized();
    };
    match doris_identity::token_user(&mcp.pool, &secret, jiff::Timestamp::now()).await {
        Ok(Some(_)) => {}
        Ok(None) => return unauthorized(),
        Err(err) => {
            tracing::warn!("mcp token lookup: {err}");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                Value::Null,
                -32603,
                "internal",
                None,
            );
        }
    }
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32700,
            "Parse error",
            None,
        );
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Invalid Request",
            None,
        );
    };
    let Some(id) = message.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response(); // a notification
    };
    if let Err(response) = check_headers(&headers, &id, &message) {
        return response;
    }
    let about = || {
        json!({
            "capabilities": {"tools": {}},
            "instructions": doris_cli::tools::instructions(),
        })
    };
    match method {
        "server/discover" => {
            let mut answer = about();
            answer["supportedVersions"] = json!(VERSIONS);
            answer["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
            result(id, answer)
        }
        "initialize" => {
            let mut answer = about();
            answer["protocolVersion"] = json!("2025-11-25");
            answer["serverInfo"] = server_info();
            result(id, answer)
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({"tools": doris_cli::tools::list()})),
        _ => error(StatusCode::NOT_FOUND, id, -32601, "Method not found", None),
    }
}
