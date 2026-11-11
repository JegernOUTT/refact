#[path = "gui_artifact.rs"]
mod gui_artifact;

use gui_artifact::*;
use std::fs;
use std::path::PathBuf;

struct Fixture {
    _root: tempfile::TempDir,
    gui: PathBuf,
    engine: PathBuf,
    controller: Controller,
    manifest: Manifest,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(root.path()).unwrap();
        let gui = base.join("gui");
        let engine = base.join("engine");
        fs::create_dir_all(gui.join("src/lib")).unwrap();
        fs::create_dir_all(gui.join("dist/chat")).unwrap();
        fs::create_dir_all(engine.join("build_support/gui_artifact")).unwrap();
        fs::create_dir_all(engine.join("assets/chat")).unwrap();
        fs::write(
            gui.join("package.json"),
            b"{\"name\":\"fixture\",\"version\":\"1\"}",
        )
        .unwrap();
        fs::write(
            gui.join("package-lock.json"),
            b"{\"lockfileVersion\":3,\"packages\":{\"\":{\"name\":\"fixture\",\"version\":\"1\"}}}",
        )
        .unwrap();
        for path in [
            "src/lib/index.ts",
            "vite.config.ts",
            "vite.node.config.ts",
            "tsconfig.json",
        ] {
            fs::write(gui.join(path), b"fixture input").unwrap();
        }
        for path in [
            "build.rs",
            "build_support/gui_artifact.rs",
            "build_support/gui_artifact/trusted_controller.rs",
            "assets/chat/index.html",
        ] {
            fs::write(engine.join(path), b"fixture producer").unwrap();
        }
        for path in ["index.js", "index.umd.cjs", "style.css", "index.d.ts"] {
            fs::write(gui.join("dist/chat").join(path), b"fixture artifact").unwrap();
        }
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "package.json"],
            vec![
                "-c",
                "user.name=GUI fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&gui)
                .status()
                .unwrap()
                .success());
        }
        let toolchain = Toolchain {
            node: "fixture-node".into(),
            node_abi: "fixture-abi".into(),
            node_sha256: "a".repeat(64),
            npm: "fixture-npm".into(),
            npm_cli_sha256: "b".repeat(64),
            typescript: "fixture-tsc".into(),
            vite: "fixture-vite".into(),
            platform: match std::env::consts::OS {
                "windows" => "win32",
                "macos" => "darwin",
                os => os,
            }
            .into(),
            arch: match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                arch => arch,
            }
            .into(),
            libc: if cfg!(target_env = "musl") {
                "musl"
            } else if cfg!(target_env = "gnu") {
                "glibc"
            } else {
                "none"
            }
            .into(),
        };
        let manifest = produce(&gui, inputs(&gui, &engine, toolchain.clone()).unwrap()).unwrap();
        let pin = digest(&fs::read(gui.join("dist").join(MANIFEST)).unwrap());
        let controller = Controller {
            schema: 1,
            kind: "refact-gui-artifact-controller".into(),
            manifest_sha256: pin,
            toolchain,
        };
        Self {
            _root: root,
            gui,
            engine,
            controller,
            manifest,
        }
    }

    fn consume(&self) -> Result<()> {
        let manifest = validate_prebuilt(&self.gui, &self.engine, &self.controller)?;
        publish(&self.gui, &self.engine, &manifest, false, || {
            validate_prebuilt(&self.gui, &self.engine, &self.controller)?;
            Ok(())
        })
    }

    fn destination(&self) -> PathBuf {
        self.engine.join("assets/chat/dist/chat")
    }

    fn assert_refused_preserving(&self) {
        assert!(self.consume().is_err());
        assert_eq!(
            fs::read(self.destination().join("index.js")).unwrap(),
            b"fixture artifact"
        );
        assert_eq!(
            self.controller.manifest_sha256,
            digest(&fs::read(self.gui.join("dist").join(MANIFEST)).unwrap())
        );
    }
}

#[test]
fn gui_artifact_pinned_copy_and_owned_replacement_retain_prior() {
    let fixture = Fixture::new();
    assert_eq!(
        validate_prebuilt(&fixture.gui, &fixture.engine, &fixture.controller).unwrap(),
        fixture.manifest
    );
    fixture.consume().unwrap();
    assert_eq!(
        fs::read(fixture.destination().join("index.js")).unwrap(),
        b"fixture artifact"
    );
    fixture.consume().unwrap();
    let retained: Vec<_> = fs::read_dir(fixture.engine.join("assets"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".refact-gui-")
        })
        .collect();
    assert!(retained.iter().any(|p| p
        .join(if cfg!(target_os = "linux") {
            "chat/index.js"
        } else {
            "previous/index.js"
        })
        .is_file()));
    assert!(!fixture.engine.join("assets/chat/dist/previous").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            fs::metadata(fixture.gui.join("dist/chat/index.js"))
                .unwrap()
                .ino(),
            fs::metadata(fixture.destination().join("index.js"))
                .unwrap()
                .ino()
        );
    }
}

#[test]
fn gui_artifact_changed_source_and_lock_preserve_existing() {
    let fixture = Fixture::new();
    fixture.consume().unwrap();
    fs::write(fixture.gui.join("src/lib/index.ts"), b"changed source").unwrap();
    fixture.assert_refused_preserving();
    fs::write(fixture.gui.join("src/lib/index.ts"), b"fixture input").unwrap();
    fs::write(fixture.gui.join("package-lock.json"), b"{\"lockfileVersion\":3,\"packages\":{\"\":{\"name\":\"fixture\",\"version\":\"1\"}},\"changed\":true}").unwrap();
    fixture.assert_refused_preserving();
}

#[test]
fn gui_artifact_corrupt_or_incomplete_payload_preserves_existing() {
    let fixture = Fixture::new();
    fixture.consume().unwrap();
    fs::write(fixture.gui.join("dist/chat/index.js"), b"corrupt").unwrap();
    fixture.assert_refused_preserving();
    fs::write(fixture.gui.join("dist/chat/index.js"), b"fixture artifact").unwrap();
    fs::remove_file(fixture.gui.join("dist/chat/style.css")).unwrap();
    fixture.assert_refused_preserving();
}

#[cfg(unix)]
#[test]
fn gui_artifact_escaping_link_and_special_file_preserve_existing() {
    let fixture = Fixture::new();
    fixture.consume().unwrap();
    let outside = fixture.engine.join("outside");
    fs::write(&outside, b"untouched").unwrap();
    let artifact = fixture.gui.join("dist/chat/index.js");
    fs::remove_file(&artifact).unwrap();
    std::os::unix::fs::symlink(&outside, &artifact).unwrap();
    fixture.assert_refused_preserving();
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    fs::remove_file(&artifact).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(artifact.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    fixture.assert_refused_preserving();
}

#[test]
fn gui_artifact_untrusted_manifest_and_toolchain_refuse() {
    let mut fixture = Fixture::new();
    fixture.consume().unwrap();
    fixture.controller.toolchain.node.push_str("-different");
    assert!(fixture.consume().is_err());
    fixture.controller.toolchain = fixture.manifest.inputs.toolchain.clone();
    let mut forged = fixture.manifest.clone();
    fs::write(fixture.gui.join("dist/chat/index.js"), b"forged payload").unwrap();
    forged.artifacts = inventory(&fixture.gui.join("dist/chat"), false).unwrap();
    fs::write(
        fixture.gui.join("dist").join(MANIFEST),
        serde_json::to_vec(&forged).unwrap(),
    )
    .unwrap();
    assert!(fixture.consume().is_err());
    assert_eq!(
        fs::read(fixture.destination().join("index.js")).unwrap(),
        b"fixture artifact"
    );
}

#[test]
fn gui_artifact_late_refusal_and_unowned_destination_preserve_existing() {
    let fixture = Fixture::new();
    fixture.consume().unwrap();
    let before = inventory(&fixture.destination(), false).unwrap();
    assert!(publish(
        &fixture.gui,
        &fixture.engine,
        &fixture.manifest,
        false,
        || Err("controller revoked".into())
    )
    .is_err());
    assert_eq!(inventory(&fixture.destination(), false).unwrap(), before);
    fs::remove_file(fixture.destination().join(".refact-gui-owner.json")).unwrap();
    assert!(fixture.consume().is_err());
    assert_eq!(
        fs::read(fixture.destination().join("index.js")).unwrap(),
        b"fixture artifact"
    );
}

#[test]
fn gui_artifact_node_options_preserve_preload_guards_and_heap_policy() {
    let guard = "--require=\"/fixture/offline guard.cjs\" --disable-proto=throw";
    assert_eq!(
        merged_node_options(Some(guard.into())),
        std::ffi::OsString::from(format!("{guard} --max-old-space-size=8192"))
    );
    let bounded = format!("{guard} --max-old-space-size=2048");
    assert_eq!(
        merged_node_options(Some(bounded.clone().into())),
        std::ffi::OsString::from(bounded)
    );
    assert_eq!(merged_node_options(None), "--max-old-space-size=8192");
}

#[cfg(unix)]
#[test]
fn gui_artifact_installed_program_resolution_accepts_hardlinks_and_launcher_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    let program = root.join("node");
    let alias = root.join("nodejs");
    let launcher = root.join("npm");
    fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::hard_link(&program, &alias).unwrap();
    symlink(&alias, &launcher).unwrap();
    assert!(std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .next()
        .is_some());
    assert_eq!(resolve_program(program.to_str().unwrap()).unwrap(), program);
    assert_eq!(resolve_program(alias.to_str().unwrap()).unwrap(), alias);
    assert_eq!(resolve_program(launcher.to_str().unwrap()).unwrap(), alias);
}

#[cfg(unix)]
#[test]
fn gui_artifact_source_and_artifact_hardlinks_preserve_existing_and_external_bytes() {
    for relative in ["src/lib/index.ts", "dist/chat/index.js"] {
        let fixture = Fixture::new();
        fixture.consume().unwrap();
        let before = inventory(&fixture.destination(), false).unwrap();
        let linked = fixture.gui.join(relative);
        let bytes = fs::read(&linked).unwrap();
        let outside = fixture.engine.join("outside");
        fs::hard_link(&linked, &outside).unwrap();
        let (inventory_root, source) = if relative.starts_with("src/") {
            (fixture.gui.clone(), true)
        } else {
            (fixture.gui.join("dist/chat"), false)
        };
        let error = inventory(&inventory_root, source).unwrap_err().to_string();
        assert!(error.contains("hard links"), "{error}");
        fixture.assert_refused_preserving();
        assert!(publish(
            &fixture.gui,
            &fixture.engine,
            &fixture.manifest,
            false,
            || Ok(())
        )
        .is_err());
        assert_eq!(inventory(&fixture.destination(), false).unwrap(), before);
        assert_eq!(fs::read(&outside).unwrap(), bytes);
        assert_eq!(fs::read(&linked).unwrap(), bytes);
    }
}
