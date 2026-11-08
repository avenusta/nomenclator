//! Compiles the protobuf contract with protox, a pure-Rust compiler, so that the build
//! does not require a system `protoc`.

use prost::Message;
use std::error::Error;
use std::path::PathBuf;

const PROTO_ROOT: &str = "proto";
const CONTRACT: &str = "nomenclator/v1/nomenclator.proto";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={PROTO_ROOT}");

    let file_descriptors = protox::compile([CONTRACT], [PROTO_ROOT])?;
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    std::fs::write(
        out_dir.join("nomenclator_descriptors.bin"),
        file_descriptors.encode_to_vec(),
    )?;
    let mut prost_config = prost_build::Config::new();
    prost_config.enable_type_names();
    tonic_prost_build::configure().compile_fds_with_config(file_descriptors, prost_config)?;

    Ok(())
}
