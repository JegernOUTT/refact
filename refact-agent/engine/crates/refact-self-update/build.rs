use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let engine_manifest = manifest_dir.join("../..").join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", engine_manifest.display());
    let Ok(text) = fs::read_to_string(&engine_manifest) else {
        return;
    };
    let Some(version) = package_version(&text) else {
        return;
    };
    println!("cargo:rustc-env=REFACT_ENGINE_VERSION={version}");
}

fn package_version(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "version" {
            continue;
        }
        return Some(value.trim().trim_matches('"').to_string());
    }
    None
}
