//! The Doris server: gRPC-Web API and the embedded frontend on one port.

mod access;
pub mod assets;
pub mod bolagsverket;
mod company;
mod grpc;
mod invoicing;
mod ledger;
mod payroll;
pub mod skatteverket;
mod vat;

use axum::Router;
use axum::extract::State;
use axum::middleware::Next;
use axum::routing::get;
use doris_proto::auth::v1::auth_service_server::AuthServiceServer;
use doris_proto::company::v1::company_service_server::CompanyServiceServer;
use doris_proto::invoicing::v1::invoicing_service_server::InvoicingServiceServer;
use doris_proto::ledger::v1::ledger_service_server::LedgerServiceServer;
use doris_proto::payroll::v1::payroll_service_server::PayrollServiceServer;
use doris_proto::vat::v1::vat_service_server::VatServiceServer;
use http::header::CONTENT_TYPE;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use rust_embed::RustEmbed;
use sqlx::SqlitePool;
use std::time::Duration;
use tonic::service::Routes;
use tonic_web::GrpcWebLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Marks a response as a dry run: nothing was saved, so the call does not
/// count as the token's use.
#[derive(Clone, Copy)]
pub(crate) struct DryRun;

pub use company::CompanyApi;
pub use grpc::{AuthApi, SESSION_COOKIE};
pub use invoicing::InvoicingApi;
pub use ledger::LedgerApi;
pub use payroll::PayrollApi;
pub use vat::VatApi;

/// Builds the app. `E` is the embedded frontend (see [`assets::WebDist`]).
/// With no `cors_origins`, only same-origin browsers can call the API.
#[allow(clippy::too_many_arguments)] // one parameter per service, by design
pub fn router<E: RustEmbed + Send + Sync + 'static>(
    api: AuthApi,
    companies: CompanyApi,
    ledger: LedgerApi,
    payroll: PayrollApi,
    invoicing: InvoicingApi,
    vat: VatApi,
    cors_origins: Vec<HeaderValue>,
    serve_frontend: bool,
) -> Router {
    let pool = ledger.pool.clone();
    let mut app = Routes::new(AuthServiceServer::new(api))
        .add_service(CompanyServiceServer::new(companies))
        .add_service(
            LedgerServiceServer::new(ledger)
                .max_decoding_message_size(ledger::MAX_REQUEST)
                .max_encoding_message_size(ledger::MAX_RESPONSE),
        )
        .add_service(PayrollServiceServer::new(payroll))
        .add_service(
            // Underlag ride along with supplier invoices, as with vouchers.
            InvoicingServiceServer::new(invoicing)
                .max_decoding_message_size(ledger::MAX_REQUEST)
                .max_encoding_message_size(ledger::MAX_RESPONSE),
        )
        .add_service(VatServiceServer::new(vat))
        .into_axum_router()
        .layer(axum::middleware::from_fn_with_state(pool, auth_gate))
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

/// Authenticates every gRPC call before its body is read. With an
/// `authorization: Bearer` header the call runs as the token's owner, if
/// `access` lets tokens make it, with the token recorded on every event it
/// appends. Without one, LedgerService and InvoicingService (bodies of up
/// to 21 MiB, which tonic reserves from the frame header before any handler
/// runs) need a valid session cookie here; handlers still check it.
async fn auth_gate(
    State(pool): State<SqlitePool>,
    mut request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    if let Some(secret) = grpc::bearer(request.headers()) {
        let now = jiff::Timestamp::now();
        let (user, access) = match doris_identity::token_user(&pool, &secret, now).await {
            Ok(Some(found)) => found,
            Ok(None) => return grpc::not_signed_in().into_http(),
            Err(err) => return grpc::status(err).into_http(),
        };
        let required = access::access(&path);
        if required == access::Access::SessionOnly {
            return tonic::Status::permission_denied("token_not_allowed").into_http();
        }
        let token_id = access.token_id;
        request.extensions_mut().insert(grpc::TokenCaller {
            user,
            access,
            required,
        });
        let response = doris_eventstore::VIA_TOKEN
            .scope(token_id.to_string(), next.run(request))
            .await;
        let dry_run = response.extensions().get::<DryRun>().is_some();
        if !dry_run && let Err(err) = doris_identity::touch_api_token(&pool, token_id, now).await {
            tracing::warn!("api token usage: {err}");
        }
        return response;
    }
    let large = path.starts_with("/doris.ledger.v1.LedgerService/")
        || path.starts_with("/doris.invoicing.v1.InvoicingService/");
    if large && let Err(status) = grpc::session_user(&pool, request.headers()).await {
        return status.into_http();
    }
    next.run(request).await
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
