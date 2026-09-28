//! `WebDist` must resolve to the real `crates/web/dist` (relative to
//! `crates/server/Cargo.toml`), not a path literally containing
//! `$CARGO_MANIFEST_DIR` (rust-embed only expands that with the
//! `interpolate-folder-path` feature, which we don't enable).

use doris_server::assets::WebDist;
use std::fs;
use std::path::Path;

#[test]
fn web_dist_embeds_the_real_frontend_build_dir() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    let index = dist.join("index.html");

    let created_dir = !dist.exists();
    if !index.exists() {
        fs::create_dir_all(&dist).unwrap();
        fs::write(&index, "<!doctype html><title>test</title>").unwrap();

        assert!(WebDist::get("index.html").is_some());

        fs::remove_file(&index).unwrap();
        if created_dir {
            fs::remove_dir(&dist).unwrap();
        }
    } else {
        assert!(WebDist::get("index.html").is_some());
    }
}
