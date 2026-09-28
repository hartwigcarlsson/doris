//! The Doris server: gRPC-Web API and the embedded frontend on one port.

pub mod assets;
mod grpc;

use axum::Router;
use axum::routing::get;
use doris_proto::auth::v1::auth_service_server::AuthServiceServer;
use http::header::CONTENT_TYPE;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use rust_embed::RustEmbed;
use std::time::Duration;
use tonic::service::Routes;
use tonic_web::GrpcWebLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub use grpc::{AuthApi, SESSION_COOKIE};

/// Builds the app. `E` is the embedded frontend (see [`assets::WebDist`]).
/// With no `cors_origins`, only same-origin browsers can call the API.
pub fn router<E: RustEmbed + Send + Sync + 'static>(
    api: AuthApi,
    cors_origins: Vec<HeaderValue>,
    serve_frontend: bool,
) -> Router {
    let mut app = Routes::new(AuthServiceServer::new(api))
        .into_axum_router()
        .layer(GrpcWebLayer::new());
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
