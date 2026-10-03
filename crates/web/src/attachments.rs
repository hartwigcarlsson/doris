//! Underlag in the browser: reading picked files, checking their size
//! before upload, and opening one in a new tab.

use crate::api::lpb;
use leptos::prelude::set_timeout;
use std::time::Duration;

/// The server's limits, mirrored so a pick that is too large is refused
/// before it is uploaded.
pub const MAX_FILE: usize = 10 << 20;
pub const MAX_TOTAL: usize = 20 << 20;

/// `Err("attachment_too_large")` when a file, or all of them together, are
/// over the limits.
pub fn check_sizes(files: &[lpb::NewAttachment]) -> Result<(), &'static str> {
    let total: usize = files.iter().map(|f| f.data.len()).sum();
    if files.iter().any(|f| f.data.len() > MAX_FILE) || total > MAX_TOTAL {
        Err("attachment_too_large")
    } else {
        Ok(())
    }
}

/// A picked file's `size()` over [`MAX_FILE`]: it is refused unread, so a
/// huge pick never has to fit in wasm memory.
pub fn too_large(size: f64) -> bool {
    size > MAX_FILE as f64
}

/// "120 kB", or "1,5 MB" from 1 MB up. Rounded up, never to 0.
pub fn size_label(bytes: u64) -> String {
    if bytes <= 999_000 {
        format!("{} kB", bytes.div_ceil(1000))
    } else {
        let tenths = bytes.div_ceil(100_000);
        format!("{},{} MB", tenths / 10, tenths % 10)
    }
}

/// The files picked in `input`, read into memory, or
/// `Err("attachment_too_large")` before anything is read when one is over
/// [`MAX_FILE`]. The input is cleared so the same file can be picked again.
pub async fn read_files(
    input: &web_sys::HtmlInputElement,
) -> Result<Vec<lpb::NewAttachment>, &'static str> {
    let files: Vec<web_sys::File> = input
        .files()
        .map(|list| (0..list.length()).filter_map(|i| list.get(i)).collect())
        .unwrap_or_default();
    input.set_value("");
    if files.iter().any(|file| too_large(file.size())) {
        return Err("attachment_too_large");
    }
    let mut picked = Vec::new();
    for file in files {
        let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await else {
            continue;
        };
        picked.push(lpb::NewAttachment {
            file_name: file.name(),
            data: js_sys::Uint8Array::new(&buffer).to_vec(),
        });
    }
    Ok(picked)
}

/// Shows the file in `tab`, in the browser's own PDF or image viewer. The tab
/// is opened by the click itself (browsers block `window.open` after an await).
pub fn open_in(tab: &web_sys::Window, content_type: &str, data: &[u8]) {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(content_type);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options) else {
        let _ = tab.close();
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        let _ = tab.close();
        return;
    };
    let _ = tab.location().set_href(&url);
    // The tab has loaded it long before then; free the memory.
    set_timeout(
        move || {
            let _ = web_sys::Url::revoke_object_url(&url);
        },
        Duration::from_secs(60),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: usize) -> lpb::NewAttachment {
        lpb::NewAttachment {
            file_name: "a.pdf".into(),
            data: vec![0; size],
        }
    }

    #[test]
    fn sizes_follow_the_servers_limits() {
        assert_eq!(check_sizes(&[file(MAX_FILE)]), Ok(()));
        assert_eq!(
            check_sizes(&[file(MAX_FILE + 1)]),
            Err("attachment_too_large")
        );
        assert_eq!(check_sizes(&[file(MAX_FILE), file(MAX_FILE)]), Ok(()));
        assert_eq!(
            check_sizes(&[file(MAX_FILE), file(MAX_FILE), file(1)]),
            Err("attachment_too_large")
        );
    }

    #[test]
    fn a_picked_file_over_the_limit_is_too_large_to_read() {
        assert!(!too_large(0.0));
        assert!(!too_large(MAX_FILE as f64));
        assert!(too_large(MAX_FILE as f64 + 1.0));
        assert!(too_large(5e9));
    }

    #[test]
    fn sizes_read_as_kb_or_mb_with_one_decimal() {
        assert_eq!(size_label(1), "1 kB");
        assert_eq!(size_label(120_000), "120 kB");
        assert_eq!(size_label(999_000), "999 kB");
        assert_eq!(size_label(999_001), "1,0 MB");
        assert_eq!(size_label(10_485_760), "10,5 MB");
    }
}
