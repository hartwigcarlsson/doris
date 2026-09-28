fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(std::env::var_os("CARGO_FEATURE_SERVER").is_some())
        .build_transport(false)
        .compile_protos(&["../../proto/doris/auth/v1/auth.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
