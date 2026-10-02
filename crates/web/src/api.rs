//! gRPC-Web clients for the Doris API.

use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;
use leptos::prelude::window;
use tonic_web_wasm_client::Client;
use tonic_web_wasm_client::options::{Credentials, FetchOptions};

pub use doris_proto::auth::v1 as pb;
pub use doris_proto::company::v1 as cpb;
#[allow(unused_imports)]
pub use doris_proto::ledger::v1 as lpb;

pub type Api = AuthServiceClient<Client>;
pub type CompanyApi = CompanyServiceClient<Client>;
#[allow(dead_code)]
pub type LedgerApi = LedgerServiceClient<Client>;

pub fn api() -> Api {
    AuthServiceClient::new(client())
}

pub fn company_api() -> CompanyApi {
    CompanyServiceClient::new(client())
}

#[allow(dead_code)]
pub fn ledger_api() -> LedgerApi {
    LedgerServiceClient::new(client())
}

/// Cookies are always sent, so the session also works when the frontend is
/// served from another origin (CDN) on the same site.
fn client() -> Client {
    let options = FetchOptions::new().credentials(Credentials::Include);
    Client::new_with_options(base_url(), options)
}

/// `<meta name="doris-api" content="…">` when set, otherwise this page's origin.
fn base_url() -> String {
    window()
        .document()
        .and_then(|doc| doc.query_selector("meta[name=doris-api]").ok().flatten())
        .and_then(|meta| meta.get_attribute("content"))
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| window().location().origin().expect("page has an origin"))
}
