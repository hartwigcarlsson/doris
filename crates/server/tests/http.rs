mod common;

use common::{TestServer, http};
use http::Method;
use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE,
    CACHE_CONTROL, CONTENT_TYPE, ETAG,
};

#[tokio::test]
async fn the_root_serves_index_html_without_caching() {
    let server = TestServer::start().await;

    let response = http(Method::GET, &format!("{}/", server.base), &[]).await;

    assert_eq!(response.status(), 200);
    assert!(response.body().contains("<title>Doris</title>"));
    assert_eq!(response.headers()[CONTENT_TYPE], "text/html");
    assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn app_routes_fall_back_to_index_html() {
    let server = TestServer::start().await;

    let response = http(
        Method::GET,
        &format!("{}/admin/invitations", server.base),
        &[],
    )
    .await;

    assert_eq!(response.status(), 200);
    assert!(response.body().contains("<title>Doris</title>"));
}

#[tokio::test]
async fn hashed_assets_are_cached_forever_and_others_revalidated() {
    let server = TestServer::start().await;

    let hashed = http(
        Method::GET,
        &format!("{}/doris-web-0123456789abcdef.js", server.base),
        &[],
    )
    .await;
    let plain = http(Method::GET, &format!("{}/style.css", server.base), &[]).await;

    assert_eq!(hashed.status(), 200);
    assert_eq!(hashed.headers()[CONTENT_TYPE], "text/javascript");
    assert_eq!(
        hashed.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(plain.headers()[CONTENT_TYPE], "text/css");
    assert_eq!(plain.headers()[CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn trunk_hashed_wasm_and_short_hashes_are_recognized() {
    let server = TestServer::start().await;

    let wasm = http(
        Method::GET,
        &format!("{}/doris-web-0123456789abcdef_bg.wasm", server.base),
        &[],
    )
    .await;
    let short_hash_css = http(
        Method::GET,
        &format!("{}/app-abc1234f.css", server.base),
        &[],
    )
    .await;
    let plain = http(Method::GET, &format!("{}/style.css", server.base), &[]).await;

    assert_eq!(wasm.status(), 200);
    assert_eq!(wasm.headers()[CONTENT_TYPE], "application/wasm");
    assert_eq!(
        wasm.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        short_hash_css.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(plain.headers()[CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn etag_supports_conditional_requests() {
    let server = TestServer::start().await;

    let first = http(Method::GET, &format!("{}/", server.base), &[]).await;
    let etag = first
        .headers()
        .get(ETAG)
        .expect("response has an etag")
        .to_str()
        .unwrap()
        .to_owned();
    let second = http(
        Method::GET,
        &format!("{}/", server.base),
        &[("if-none-match", &etag)],
    )
    .await;

    assert_eq!(second.status(), 304);
    assert_eq!(second.headers()[ETAG], etag);
    assert!(second.body().is_empty());
}

#[tokio::test]
async fn security_headers_are_set_on_frontend_responses() {
    let server = TestServer::start().await;

    let html = http(Method::GET, &format!("{}/", server.base), &[]).await;
    let js = http(
        Method::GET,
        &format!("{}/doris-web-0123456789abcdef.js", server.base),
        &[],
    )
    .await;

    assert_eq!(html.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        html.headers()["content-security-policy"],
        "frame-ancestors 'none'"
    );
    assert_eq!(html.headers()["referrer-policy"], "no-referrer");
    assert_eq!(js.headers()["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn missing_files_are_not_found_instead_of_index_html() {
    let server = TestServer::start().await;

    let response = http(Method::GET, &format!("{}/missing.js", server.base), &[]).await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn the_frontend_can_be_switched_off_for_cdn_deployments() {
    let server = TestServer::start_with(vec![], false).await;

    let response = http(Method::GET, &format!("{}/", server.base), &[]).await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn cors_preflight_is_allowed_only_for_configured_origins() {
    let cdn = "https://app.example.se";
    let server = TestServer::start_with(vec![cdn.parse().unwrap()], false).await;
    let url = format!("{}/doris.auth.v1.AuthService/GetStatus", server.base);
    let preflight = |origin: &'static str| {
        let url = url.clone();
        async move {
            http(
                Method::OPTIONS,
                &url,
                &[
                    ("origin", origin),
                    ("access-control-request-method", "POST"),
                    ("access-control-request-headers", "content-type,x-grpc-web"),
                ],
            )
            .await
        }
    };

    let allowed = preflight(cdn).await;
    let denied = preflight("https://evil.example").await;

    assert_eq!(allowed.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], cdn);
    assert_eq!(allowed.headers()[ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert_eq!(allowed.headers()[ACCESS_CONTROL_MAX_AGE], "7200");
    assert!(!denied.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn without_cors_origins_no_cross_origin_access_is_granted() {
    let server = TestServer::start().await;

    let response = http(
        Method::OPTIONS,
        &format!("{}/doris.auth.v1.AuthService/GetStatus", server.base),
        &[
            ("origin", "https://app.example.se"),
            ("access-control-request-method", "POST"),
        ],
    )
    .await;

    assert!(!response.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
}
