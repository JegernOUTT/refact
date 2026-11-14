#[path = "../build_support/runtime_version.rs"]
mod runtime_version;

use std::process::Command;

#[test]
fn resolves_package_stable_and_suffix_versions() {
    assert_eq!(runtime_version::resolve("8.7.0", None).unwrap(), "8.7.0");
    for version in ["9.2.3", "9.2.3-rc.1", "9.2.3-nightly.20261104+build.02"] {
        assert_eq!(
            runtime_version::resolve("8.7.0", Some(version)).unwrap(),
            version
        );
    }
    for version in [
        "",
        " ",
        "9.2",
        "v9.2.3",
        "09.2.3",
        "9.2.3-01",
        "9.2.3-",
        "9.2.3+",
        "9.2.3\n",
        "9.2.3+a+b",
    ] {
        assert!(
            runtime_version::resolve("8.7.0", Some(version)).is_err(),
            "{version:?}"
        );
    }
}

#[test]
fn runtime_identity_consumers_agree() {
    let version = refact_lsp::version::VERSION;
    assert_eq!(
        refact_lsp::cli_dispatch::version_text()
            .lines()
            .next()
            .unwrap(),
        format!("refact {version}")
    );
    assert_eq!(
        refact_lsp::http::routers::info::get_build_info()["version"],
        version
    );
    let lease =
        refact_lsp::daemon::lock::WorkspaceLeaseInfo::for_current_process(0, 0, "metadata-test");
    assert_eq!(lease.version, version);
    let metadata: serde_json::Value =
        serde_json::from_str(refact_lsp::version::build_info_json()).unwrap();
    assert_eq!(metadata["version"], version);
}

#[test]
fn native_stripped_executable_retains_exact_metadata() {
    let scratch = tempfile::tempdir().unwrap();
    let binary = scratch.path().join("refact-lsp");
    std::fs::copy(env!("CARGO_BIN_EXE_refact-lsp"), &binary).unwrap();
    if cfg!(target_os = "linux") {
        assert!(Command::new("strip")
            .arg(&binary)
            .status()
            .unwrap()
            .success());
    }
    let bytes = std::fs::read(&binary).unwrap();
    let start = b"\0REFACT_BUILD_INFO_V1\0";
    let end = b"\0END_REFACT_BUILD_INFO\0";
    let starts: Vec<_> = bytes
        .windows(start.len())
        .enumerate()
        .filter_map(|(i, b)| (b == start).then_some(i))
        .collect();
    assert_eq!(starts.len(), 1, "exactly one retained frame");
    let payload = &bytes[starts[0] + start.len()..];
    let finish = payload.windows(end.len()).position(|b| b == end).unwrap();
    assert!(finish <= 1024);
    let metadata: serde_json::Value = serde_json::from_slice(&payload[..finish]).unwrap();
    assert_eq!(metadata.as_object().unwrap().len(), 4);
    assert_eq!(metadata["schema_version"], 1);
    let runtime_metadata: serde_json::Value =
        serde_json::from_str(refact_lsp::version::build_info_json()).unwrap();
    assert_eq!(metadata, runtime_metadata);
    let source = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(source.status.success());
    assert_eq!(
        metadata["source_sha"],
        String::from_utf8(source.stdout).unwrap().trim()
    );
    assert_eq!(metadata["version"], refact_lsp::version::VERSION);
    let output = Command::new(&binary).arg("--build-info").output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        metadata,
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    );
    let management = Command::new(env!("CARGO_BIN_EXE_refact"))
        .arg("--build-info")
        .output()
        .unwrap();
    assert!(management.status.success());
    assert_eq!(
        metadata,
        serde_json::from_slice::<serde_json::Value>(&management.stdout).unwrap()
    );
    let output = Command::new(&binary).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("refact-lsp {}", refact_lsp::version::VERSION)
    );
}

#[test]
fn cargo_same_target_override_a_b_unset_refreshes_both_build_scripts() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("crates/refact-self-update/src")).unwrap();
    std::fs::create_dir(root.join("build_support")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='version-probe'\nversion='8.7.0'\nedition='2021'\n[workspace]\nmembers=['crates/refact-self-update']\n[dependencies]\nrefact-self-update={path='crates/refact-self-update'}\n").unwrap();
    std::fs::write(
        root.join("build_support/runtime_version.rs"),
        include_str!("../build_support/runtime_version.rs"),
    )
    .unwrap();
    std::fs::write(root.join("build.rs"), "#[path=\"build_support/runtime_version.rs\"] mod runtime_version; fn main(){runtime_version::emit(std::path::Path::new(\"Cargo.toml\"));}").unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main(){println!(\"{} {}\",env!(\"REFACT_ENGINE_VERSION\"),refact_self_update::VERSION);}").unwrap();
    std::fs::write(
        root.join("crates/refact-self-update/Cargo.toml"),
        "[package]\nname='refact-self-update'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/refact-self-update/build.rs"),
        include_str!("../crates/refact-self-update/build.rs"),
    )
    .unwrap();
    std::fs::write(
        root.join("crates/refact-self-update/src/lib.rs"),
        "pub const VERSION: &str=env!(\"REFACT_ENGINE_VERSION\");",
    )
    .unwrap();
    for version in [Some("9.1.0"), Some("9.2.0-rc.1"), None] {
        let mut command = Command::new(env!("CARGO"));
        command
            .args(["run", "--quiet", "--offline"])
            .current_dir(root)
            .env_remove("REFACT_BUILD_VERSION")
            .env_remove("CARGO_TARGET_DIR");
        if let Some(version) = version {
            command.env("REFACT_BUILD_VERSION", version);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = version.unwrap_or("8.7.0");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("{expected} {expected}")
        );
    }
    for invalid in ["", "9.2", "9.2.3-01"] {
        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--offline"])
            .current_dir(root)
            .env_remove("CARGO_TARGET_DIR")
            .env("REFACT_BUILD_VERSION", invalid)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {invalid:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("expected SemVer"));
    }
}
