//! Doris web app: Leptos CSR, talking to the server over gRPC-Web.

mod active_company;
mod api;
mod app;
mod attachments;
mod errors;
mod fiscal_year;
mod format;
mod invoice_ui;
mod pages;
mod passkey;
mod ui;
mod voucher_lines;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
