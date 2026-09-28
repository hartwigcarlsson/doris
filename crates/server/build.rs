fn main() {
    // rust-embed decides at compile time whether crates/web/dist exists (and
    // embeds it in release builds): rebuild the server when the frontend does.
    // Without the directory it compiles `WebDist` to an empty stub that never
    // reads the disk, even in debug builds, so make sure it exists.
    std::fs::create_dir_all("../web/dist").unwrap();
    println!("cargo:rerun-if-changed=../web/dist");
}
