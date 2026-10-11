//! Generates the protobuf messages from `proto/debonair.proto` and `proto/storage.proto`.
use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let proto = manifest.join("../../../proto");
    for name in ["debonair.proto", "storage.proto"] {
        println!("cargo:rerun-if-changed={}", proto.join(name).display());
    }
    let descriptors =
        protox::compile(["debonair.proto", "storage.proto"], [&proto]).expect("compile schemas");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("generate messages");
}
