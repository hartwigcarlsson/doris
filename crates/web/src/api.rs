//! gRPC-Web clients for the Doris API.

use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
use doris_proto::invoicing::v1::invoicing_service_client::InvoicingServiceClient;
use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;
use doris_proto::payroll::v1::payroll_service_client::PayrollServiceClient;
use doris_proto::vat::v1::vat_service_client::VatServiceClient;
use leptos::prelude::window;
use tonic_web_wasm_client::Client;
use tonic_web_wasm_client::options::{Credentials, FetchOptions};

pub use doris_proto::auth::v1 as pb;
pub use doris_proto::company::v1 as cpb;
pub use doris_proto::invoicing::v1 as ipb;
pub use doris_proto::ledger::v1 as lpb;
pub use doris_proto::payroll::v1 as ppb;
pub use doris_proto::vat::v1 as vpb;

pub type Api = AuthServiceClient<Client>;
pub type CompanyApi = CompanyServiceClient<Client>;
pub type InvoicingApi = InvoicingServiceClient<Client>;
pub type LedgerApi = LedgerServiceClient<Client>;
pub type PayrollApi = PayrollServiceClient<Client>;
pub type VatApi = VatServiceClient<Client>;

pub fn api() -> Api {
    AuthServiceClient::new(client())
}

pub fn company_api() -> CompanyApi {
    CompanyServiceClient::new(client())
}

pub fn invoicing_api() -> InvoicingApi {
    // Room for a 10 MiB underlag coming back from GetSupplierInvoiceAttachment.
    InvoicingServiceClient::new(client()).max_decoding_message_size(11 << 20)
}

pub fn payroll_api() -> PayrollApi {
    PayrollServiceClient::new(client())
}

pub fn ledger_api() -> LedgerApi {
    // Room for a 10 MiB underlag coming back from GetAttachment.
    LedgerServiceClient::new(client()).max_decoding_message_size(11 << 20)
}

pub fn vat_api() -> VatApi {
    VatServiceClient::new(client())
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

/// The `GetStatus` answer that `index.html` asked for while the wasm was
/// still downloading (`window.dorisStatus`), if it came back usable.
pub async fn prefetched_status() -> Option<pb::GetStatusResponse> {
    use wasm_bindgen::JsCast;
    let promise: js_sys::Promise = js_sys::Reflect::get(&window(), &"dorisStatus".into())
        .ok()?
        .dyn_into()
        .ok()?;
    let body = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    if body.is_null() {
        return None;
    }
    decode_status(&js_sys::Uint8Array::new(&body).to_vec())
}

/// Reads a gRPC-Web response body: the message is the first frame, with flag
/// 0 and a 4-byte big-endian length. An error has no such frame.
fn decode_status(body: &[u8]) -> Option<pb::GetStatusResponse> {
    if body.first() != Some(&0) {
        return None;
    }
    let len = u32::from_be_bytes(body.get(1..5)?.try_into().ok()?) as usize;
    prost::Message::decode(body.get(5..5usize.checked_add(len)?)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn frame(flag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![flag];
        out.extend((payload.len() as u32).to_be_bytes());
        out.extend(payload);
        out
    }

    #[test]
    fn decode_status_reads_the_first_data_frame() {
        let status = pb::GetStatusResponse {
            bootstrap_required: true,
            current_user: Some(pb::User {
                id: "u1".into(),
                ..Default::default()
            }),
        };
        let mut body = frame(0, &status.encode_to_vec());
        body.extend(frame(0x80, b"grpc-status:0\r\n"));
        assert_eq!(decode_status(&body), Some(status));
    }

    #[test]
    fn decode_status_reads_an_empty_message() {
        assert_eq!(
            decode_status(&frame(0, &[])),
            Some(pb::GetStatusResponse::default())
        );
    }

    #[test]
    fn decode_status_refuses_anything_else() {
        let ok = frame(0, &pb::GetStatusResponse::default().encode_to_vec());
        for bad in [
            Vec::new(),
            frame(0x80, b"grpc-status:13\r\n"),
            vec![0, 0, 0],
            frame(0, &[0xff, 0xff])[..6].to_vec(),
            frame(0, &[0xff]),
        ] {
            assert_eq!(decode_status(&bad), None, "{bad:?}");
        }
        assert!(decode_status(&ok).is_some());
    }
}
