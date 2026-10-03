//! The Doris server: gRPC-Web API and the embedded frontend on one port.

pub mod assets;
pub mod bolagsverket;
mod company;
mod grpc;
mod ledger;

use axum::Router;
use axum::routing::get;
use doris_proto::auth::v1::auth_service_server::AuthServiceServer;
use doris_proto::company::v1::company_service_server::CompanyServiceServer;
use doris_proto::ledger::v1::ledger_service_server::LedgerServiceServer;
use http::header::CONTENT_TYPE;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use rust_embed::RustEmbed;
use std::time::Duration;
use tonic::service::Routes;
use tonic_web::GrpcWebLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use company::CompanyApi;
pub use grpc::{AuthApi, SESSION_COOKIE};
pub use ledger::LedgerApi;

/// Builds the app. `E` is the embedded frontend (see [`assets::WebDist`]).
/// With no `cors_origins`, only same-origin browsers can call the API.
pub fn router<E: RustEmbed + Send + Sync + 'static>(
    api: AuthApi,
    companies: CompanyApi,
    ledger: LedgerApi,
    cors_origins: Vec<HeaderValue>,
    serve_frontend: bool,
) -> Router {
    let mut app = Routes::new(AuthServiceServer::new(api))
        .add_service(CompanyServiceServer::new(companies))
        .add_service(
            LedgerServiceServer::new(ledger)
                .max_decoding_message_size(ledger::MAX_REQUEST)
                .max_encoding_message_size(ledger::MAX_RESPONSE),
        )
        .into_axum_router()
        .layer(GrpcWebLayer::new())
        .layer(axum::middleware::map_response(hide_internal_messages));
    app = if serve_frontend {
        // Compressed on the fly: brotli cuts the wasm to about a third.
        app.fallback_service(get(assets::serve::<E>).layer(CompressionLayer::new()))
    } else {
        app.fallback(|| async { StatusCode::NOT_FOUND })
    };
    if !cors_origins.is_empty() {
        app = app.layer(cors(cors_origins));
    }
    app
}

/// Our statuses carry stable codes. tonic's own internal errors (e.g. a
/// malformed request body) would expose implementation details instead, so
/// they are logged and replaced with `internal`.
async fn hide_internal_messages(
    mut response: axum::response::Response,
) -> axum::response::Response {
    let headers = response.headers_mut();
    let internal = headers.get("grpc-status").is_some_and(|s| s == "13");
    if let Some(message) = grpc_message(headers).filter(|m| internal && m != "internal") {
        tracing::warn!("grpc internal error: {message}");
        headers.insert("grpc-message", HeaderValue::from_static("internal"));
    }
    response
}

/// The `grpc-message` header, percent-decoded as gRPC sends it.
fn grpc_message(headers: &http::HeaderMap) -> Option<String> {
    tonic::Status::from_header_map(headers).map(|status| status.message().to_owned())
}

fn cors(origins: Vec<HeaderValue>) -> CorsLayer {
    let headers = |names: &[&'static str]| {
        names
            .iter()
            .map(|n| HeaderName::from_static(n))
            .collect::<Vec<_>>()
    };
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([Method::POST])
        .allow_headers(
            [
                vec![CONTENT_TYPE],
                headers(&["x-grpc-web", "x-user-agent", "grpc-timeout"]),
            ]
            .concat(),
        )
        .expose_headers(headers(&[
            "grpc-status",
            "grpc-message",
            "grpc-status-details-bin",
        ]))
        .max_age(Duration::from_secs(7200))
}

#[cfg(test)]
mod tests {
    use super::grpc_message;
    use http::HeaderMap;

    #[test]
    fn grpc_messages_are_percent_decoded_for_the_log() {
        let mut headers = HeaderMap::new();
        headers.insert("grpc-status", "13".parse().unwrap());
        headers.insert(
            "grpc-message",
            "protocol%20error:%20invalid%20flag".parse().unwrap(),
        );

        assert_eq!(
            grpc_message(&headers).as_deref(),
            Some("protocol error: invalid flag")
        );
        assert_eq!(grpc_message(&HeaderMap::new()), None);
    }
}
