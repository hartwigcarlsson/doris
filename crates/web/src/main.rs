//! Doris web app: Leptos CSR, talking to the server over gRPC-Web.

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(|| view! { <main class="p-4 text-sm">"Doris"</main> });
}
