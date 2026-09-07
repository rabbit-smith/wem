//! Compiles the frozen wwise.v1 proto tree at build time.
//!
//! The proto files under `../../proto` are the machine-readable contract
//! (proto/AGENTS.md); the generated Rust goes to OUT_DIR and is never
//! committed. A vendored protoc binary keeps the build hermetic: no system
//! protoc install is required.
//!
//! Toolchain notes (tonic 0.14 split):
//! * `tonic-prost-build` owns message + service codegen (the old
//!   `tonic-build::configure` API);
//! * the generated code references `tonic_prost::ProstCodec`, so the crate
//!   depends on `tonic-prost` at runtime.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let proto_root = std::path::Path::new(&manifest_dir)
        .join("../../proto")
        .canonicalize()?;

    // prost-build (used by tonic-prost-build) honors PROTOC; point it at the
    // vendored binary.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    tonic_prost_build::configure().compile_protos(
        &[
            proto_root.join("wwise/v1/common.proto"),
            proto_root.join("wwise/v1/profile.proto"),
            proto_root.join("wwise/v1/encode.proto"),
        ],
        std::slice::from_ref(&proto_root),
    )?;

    Ok(())
}
