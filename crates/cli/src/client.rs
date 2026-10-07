//! The connection to Doris: gRPC-Web over HTTP/1.1, with the API token.

use crate::output::Failure;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use tonic::Request;
use tonic_web::{GrpcWebCall, GrpcWebClientLayer, GrpcWebClientService};

pub type Transport =
    GrpcWebClientService<Client<HttpsConnector<HttpConnector>, GrpcWebCall<tonic::body::Body>>>;

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
    /// `None` when the token cannot go in a header.
    pub fn new(origin: http::Uri, token: String) -> Option<Self> {
        let ok = !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic());
        ok.then_some(Self { origin, token })
    }

    pub fn transport(&self) -> Transport {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        let client =
            Client::builder(TokioExecutor::new()).build(HttpsConnector::new_with_connector(http));
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
        doris_proto::auth::v1::auth_service_client::AuthServiceClient::with_origin(
            self.transport(),
            self.origin.clone(),
        )
    }

    pub fn companies(
        &self,
    ) -> doris_proto::company::v1::company_service_client::CompanyServiceClient<Transport> {
        doris_proto::company::v1::company_service_client::CompanyServiceClient::with_origin(
            self.transport(),
            self.origin.clone(),
        )
    }

    /// Room for a 10 MiB underlag both ways.
    pub fn ledger(
        &self,
    ) -> doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient<Transport> {
        doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient::with_origin(
            self.transport(),
            self.origin.clone(),
        )
        .max_decoding_message_size(11 << 20)
        .max_encoding_message_size(21 << 20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_only_for_this_machine() {
        assert!(checked_url("https://doris.example.se").is_ok());
        assert!(checked_url("http://localhost:3000").is_ok());
        assert!(checked_url("http://127.0.0.1:3000").is_ok());
        assert_eq!(
            checked_url("http://doris.example.se").unwrap_err().code,
            "insecure_url"
        );
        assert_eq!(checked_url("ftp://x").unwrap_err().code, "insecure_url");
    }
}
