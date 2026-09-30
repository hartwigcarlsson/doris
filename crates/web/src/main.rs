//! Doris web app: Leptos CSR, talking to the server over gRPC-Web.

mod api;
mod app;
mod errors;
mod fiscal_year;
mod format;
mod pages;
mod passkey;
mod ui;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
