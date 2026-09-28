//! Serves the frontend embedded in the binary, with a single-page-app
//! fallback to `index.html`.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The Trunk build output. Empty when the frontend has not been built.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../web/dist"]
#[allow_missing = true]
pub struct WebDist;

pub async fn serve<E: RustEmbed>(uri: Uri) -> Response {
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
    (
        [
            (CONTENT_TYPE, file.metadata.mimetype().to_owned()),
            (CACHE_CONTROL, cache.to_owned()),
        ],
        file.data,
    )
        .into_response()
}

/// Trunk names build outputs `<name>-<16 hex digits>.<ext>`; those never
/// change content, so browsers may cache them forever.
fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit_once('-')
        .is_some_and(|(_, hash)| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}
