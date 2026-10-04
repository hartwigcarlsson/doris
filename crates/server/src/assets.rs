//! Serves the frontend embedded in the binary, with a single-page-app
//! fallback to `index.html`.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The Trunk build output. Empty when the frontend has not been built.
///
/// The path is relative to this crate's `Cargo.toml` (`crates/server/`), not
/// to this file: rust-embed resolves `folder` that way, and only expands
/// `$CARGO_MANIFEST_DIR` with the `interpolate-folder-path` feature, which we
/// don't enable.
#[derive(RustEmbed)]
#[folder = "../web/dist"]
#[allow_missing = true]
pub struct WebDist;

pub async fn serve<E: RustEmbed>(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (path, file) = match E::get(path) {
        Some(file) => (path, file),
        // Paths with an extension are files; anything else is an app route.
        None if path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.')) =>
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        None => match E::get("index.html") {
            Some(file) => ("index.html", file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let cache = if is_hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    // Weak, because the compression layer may send another representation.
    let etag = format!(
        "W/\"{}\"",
        file.metadata
            .sha256_hash()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let not_modified = headers
        .get(IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes());
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(CONTENT_TYPE, file.metadata.mimetype().to_owned())],
            file.data,
        )
            .into_response()
    };
    let h = response.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(ETAG, HeaderValue::from_str(&etag).expect("hex is ascii"));
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if path == "index.html" {
        h.insert(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("frame-ancestors 'none'"),
        );
        h.insert(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        );
    }
    response
}

/// Trunk names build outputs `<name>-<hash>.<ext>` (the wasm gets a `_bg`
/// suffix on the stem: `<name>-<hash>_bg.wasm`), where `<hash>` is 8-16
/// lowercase or uppercase hex digits (Trunk 0.21 formats it with `{:x}`,
/// unpadded, so it can be shorter than 16 digits). The initializer module
/// (`data-initializer`) is `<hash>-<name>.<ext>` instead. Those names never
/// change content, so browsers may cache them forever.
fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    let stem = stem.strip_suffix("_bg").unwrap_or(stem);
    let is_hash =
        |hash: &str| (8..=16).contains(&hash.len()) && hash.bytes().all(|b| b.is_ascii_hexdigit());
    stem.rsplit_once('-').is_some_and(|(_, hash)| is_hash(hash))
        || stem.split_once('-').is_some_and(|(hash, _)| is_hash(hash))
}
