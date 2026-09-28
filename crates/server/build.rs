fn main() {
    // rust-embed decides at compile time whether crates/web/dist exists (and
    // embeds it in release builds): rebuild the server when the frontend does.
    println!("cargo:rerun-if-changed=../web/dist");
}
