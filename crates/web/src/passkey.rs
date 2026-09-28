//! The browser side of WebAuthn: `navigator.credentials.create/get`, with
//! options and results as the webauthn-rs JSON the API speaks.

use leptos::prelude::window;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use webauthn_rs_proto::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};

/// Shown when the user cancels or the authenticator fails. The browser does
/// not tell which, on purpose.
pub const FAILED: &str = "Passkey-åtgärden avbröts eller misslyckades.";

/// Creates a passkey; returns the credential as JSON for `Finish…`.
pub async fn create(options_json: &str) -> Result<String, String> {
    let options: CreationChallengeResponse =
        serde_json::from_str(options_json).map_err(|_| FAILED.to_owned())?;
    let promise = window()
        .navigator()
        .credentials()
        .create_with_options(&options.into())
        .map_err(|_| FAILED.to_owned())?;
    let credential: web_sys::PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|_| FAILED.to_owned())?
        .unchecked_into();
    serde_json::to_string(&RegisterPublicKeyCredential::from(credential))
        .map_err(|_| FAILED.to_owned())
}

/// Signs a login challenge; returns the assertion as JSON for `FinishLogin`.
pub async fn get(options_json: &str) -> Result<String, String> {
    let options: RequestChallengeResponse =
        serde_json::from_str(options_json).map_err(|_| FAILED.to_owned())?;
    let promise = window()
        .navigator()
        .credentials()
        .get_with_options(&options.into())
        .map_err(|_| FAILED.to_owned())?;
    let credential: web_sys::PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|_| FAILED.to_owned())?
        .unchecked_into();
    serde_json::to_string(&PublicKeyCredential::from(credential)).map_err(|_| FAILED.to_owned())
}
