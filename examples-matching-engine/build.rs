//! Compile the vendored matching-engine proto into a client (no server codegen).
//! Uses a vendored `protoc` so the build needs no system install.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    tonic_build::configure()
        .build_server(false)
        .compile_protos(&["proto/matching_engine.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/matching_engine.proto");
    Ok(())
}
