//! `WebDist` must resolve to the real `crates/web/dist` (relative to
//! `crates/server/Cargo.toml`), not a path literally containing
//! `$CARGO_MANIFEST_DIR` (rust-embed only expands that with the
//! `interpolate-folder-path` feature, which we don't enable).

use doris_server::assets::WebDist;
use std::fs;
use std::path::{Path, PathBuf};

/// Removes the stand-in build output again, even when an assertion fails.
struct Cleanup {
    index: PathBuf,
    dir: Option<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.index);
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir(dir);
        }
    }
}

#[test]
fn web_dist_embeds_the_real_frontend_build_dir() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    let index = dist.join("index.html");
    // Without a frontend build, stand one in for the duration of the test.
    let _cleanup = (!index.exists()).then(|| {
        let created_dir = !dist.exists();
        fs::create_dir_all(&dist).unwrap();
        fs::write(&index, "<!doctype html><title>test</title>").unwrap();
        Cleanup {
            index: index.clone(),
            dir: created_dir.then(|| dist.clone()),
        }
    });

    assert!(WebDist::get("index.html").is_some());
}
