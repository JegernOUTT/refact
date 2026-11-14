#[path = "../../build_support/runtime_version.rs"]
mod runtime_version;

fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=../../build_support/runtime_version.rs");
    runtime_version::emit(&root.join("../../Cargo.toml"));
}
