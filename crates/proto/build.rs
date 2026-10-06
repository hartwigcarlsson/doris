fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(std::env::var_os("CARGO_FEATURE_SERVER").is_some())
        .build_transport(false)
        .compile_protos(
            &[
                "../../proto/doris/auth/v1/auth.proto",
                "../../proto/doris/company/v1/company.proto",
                "../../proto/doris/ledger/v1/ledger.proto",
                "../../proto/doris/payroll/v1/payroll.proto",
                "../../proto/doris/invoicing/v1/invoicing.proto",
                "../../proto/doris/vat/v1/vat.proto",
            ],
            &["../../proto"],
        )?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
