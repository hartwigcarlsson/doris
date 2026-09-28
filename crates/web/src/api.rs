//! gRPC-Web client for `doris.auth.v1.AuthService`.

use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use leptos::prelude::window;
use tonic_web_wasm_client::Client;
use tonic_web_wasm_client::options::{Credentials, FetchOptions};

pub use doris_proto::auth::v1 as pb;

pub type Api = AuthServiceClient<Client>;

/// A client for the API. Cookies are always sent, so the session also works
/// when the frontend is served from another origin (CDN) on the same site.
pub fn api() -> Api {
    let options = FetchOptions::new().credentials(Credentials::Include);
    AuthServiceClient::new(Client::new_with_options(base_url(), options))
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
