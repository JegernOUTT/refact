#[path = "build_support/gui_artifact.rs"]
mod gui_artifact;
#[path = "build_support/runtime_version.rs"]
mod runtime_version;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let version = runtime_version::emit(&root.join("Cargo.toml"));
    emit_build_identity(&root, &version);
    if env::var("TARGET").as_deref() == Ok("i686-pc-windows-msvc") {
        cc::Build::new()
            .file("build_support/sqlite_vec_i686_popcnt.c")
            .compile("sqlite_vec_i686_popcnt");
    }
    for name in [
        "REFACT_SKIP_GUI_BUILD",
        "REFACT_USE_PREBUILT_GUI",
        "REFACT_GUI_ALWAYS_BUILD",
        "REFACT_GUI_DIR",
        "REFACT_OFFLINE_NETNS",
        "CARGO_NET_OFFLINE",
        "npm_config_offline",
        "NPM_CONFIG_OFFLINE",
        "NODE_OPTIONS",
        "DEBUG",
        "REFACT_LSP_PORT",
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "VERCEL_GIT_COMMIT_SHA",
        "BUILD_VCS_NUMBER",
        "NODE_ENV",
        "PATH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    for path in [
        "build_support",
        "assets/chat/index.html",
        "src/daemon/web_picker.html",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let skip_gui_build = env::var("REFACT_SKIP_GUI_BUILD").ok().as_deref() == Some("1");
    println!(
        "cargo:rustc-env=REFACT_SKIP_GUI_BUILD={}",
        if skip_gui_build { "1" } else { "0" }
    );
    if skip_gui_build {
        println!(
            "cargo:warning=GUI refresh skipped for API-only build; existing assets NOT validated"
        );
    } else {
        if env::var("PROFILE").as_deref() == Ok("release")
            || env::var("REFACT_USE_PREBUILT_GUI").as_deref() == Ok("1")
            || env::var("REFACT_GUI_ALWAYS_BUILD").as_deref() == Ok("1")
        {
            let watched = PathBuf::from(env::var_os("OUT_DIR").unwrap())
                .join("gui-revalidate-every-invocation");
            println!("cargo:rerun-if-changed={}", watched.display());
            println!("cargo:rerun-if-changed={}", gui_artifact::CONTROLLER);
        }
        if let Err(error) = build_gui_assets(&gui_dir()) {
            panic!("GUI artifact refusal: {error}. Prior assets retained; no install/download/build fallback. See build_support/GUI_ARTIFACT.md");
        }
    }
    shadow_rs::ShadowBuilder::builder().build().unwrap();
}

fn gui_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let path = env::var_os("REFACT_GUI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.parent().unwrap().join("gui"));
    if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    }
}

fn build_gui_assets(gui: &Path) -> gui_artifact::Result<()> {
    let engine = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    gui_artifact::real_directory(gui)?;
    let gui_identity = gui_artifact::directory_identity(gui)?;
    let engine_identity = gui_artifact::directory_identity(&engine)?;
    let prebuilt = env::var("REFACT_USE_PREBUILT_GUI").ok().as_deref() == Some("1");
    let manifest = if prebuilt {
        println!("cargo:rerun-if-changed={}", gui.display());
        println!("cargo:warning=GUI prebuilt requested; validating independent producer pin and exact inputs");
        gui_artifact::trusted_prebuilt(gui, &engine)?
    } else {
        let options = gui_artifact::merged_node_options(env::var_os("NODE_OPTIONS"));
        let node = gui_artifact::resolve_program(if cfg!(windows) { "node.exe" } else { "node" })?;
        println!("cargo:rerun-if-changed={}", node.display());
        let npm = gui_artifact::resolve_program(if cfg!(windows) { "npm.cmd" } else { "npm" })?;
        println!("cargo:rerun-if-changed={}", npm.display());
        for tool in [
            "node_modules/typescript/bin/tsc",
            "node_modules/vite/bin/vite.js",
        ] {
            if !gui.join(tool).is_file() {
                return Err(format!("Missing local GUI tool {tool}; explicitly prepare matching dependencies before cargo build (offline policy: {}). No npm ci or npx is invoked", gui_artifact::offline()).into());
            }
            println!("cargo:rerun-if-changed={}", gui.join(tool).display());
        }
        for path in [
            "node_modules/typescript/package.json",
            "node_modules/vite/package.json",
            "node_modules/.package-lock.json",
        ] {
            if std::fs::symlink_metadata(gui.join(path)).is_ok() {
                println!("cargo:rerun-if-changed={}", gui.join(path).display());
            }
        }
        gui_artifact::check_build_outputs(gui, &engine)?;
        let toolchain = gui_artifact::producer_toolchain(gui, &node, &options)?;
        let before = gui_artifact::inputs(gui, &engine, toolchain.clone())?;
        watch_inputs(gui, &before);
        for args in [
            vec!["node_modules/typescript/bin/tsc"],
            vec!["node_modules/vite/bin/vite.js", "build"],
            vec![
                "node_modules/vite/bin/vite.js",
                "build",
                "-c",
                "vite.node.config.ts",
            ],
        ] {
            gui_artifact::check_directory(gui, &gui_identity)?;
            gui_artifact::check_directory(&engine, &engine_identity)?;
            gui_artifact::check_build_outputs(gui, &engine)?;
            let status = Command::new(&node)
                .args(&args)
                .env("NODE_OPTIONS", &options)
                .env("npm_package_version", &before.version)
                .current_dir(gui)
                .status()?;
            if !status.success() {
                return Err(format!("Local GUI build failed: {}", args.join(" ")).into());
            }
        }
        if gui_artifact::producer_toolchain(gui, &node, &options)? != toolchain
            || gui_artifact::inputs(gui, &engine, toolchain)? != before
        {
            return Err("GUI source/config/environment changed during authorized build".into());
        }
        println!("cargo:warning=GUI built with local tools; writing producer candidate (not controller authorization)");
        gui_artifact::produce(gui, before)?
    };
    watch_inputs(gui, &manifest.inputs);
    gui_artifact::publish(gui, &engine, &manifest, !prebuilt, || {
        gui_artifact::check_directory(gui, &gui_identity)?;
        gui_artifact::check_directory(&engine, &engine_identity)?;
        if prebuilt && gui_artifact::trusted_prebuilt(gui, &engine)? != manifest {
            return Err("GUI pinned producer changed before publication".into());
        }
        Ok(())
    })?;
    println!("cargo:warning=GUI {} staged and published with verified content; runtime qualification not implied", if prebuilt { "pinned prebuilt" } else { "local build" });
    Ok(())
}

fn watch_inputs(gui: &Path, inputs: &gui_artifact::Inputs) {
    for key in inputs
        .environment
        .keys()
        .filter(|key| key.starts_with("VITE_"))
    {
        println!("cargo:rerun-if-env-changed={key}");
    }
    for entry in &inputs.inventory {
        let path = match entry {
            gui_artifact::Entry::Directory {
                path,
            }
            | gui_artifact::Entry::File {
                path, ..
            } => path,
        };
        println!("cargo:rerun-if-changed={}", gui.join(path).display());
    }
    for name in [
        ".env",
        ".env.local",
        ".env.production",
        ".env.production.local",
        "patches",
        ".npmrc",
    ] {
        if std::fs::symlink_metadata(gui.join(name)).is_ok() {
            println!("cargo:rerun-if-changed={}", gui.join(name).display());
        }
    }
    let mut git_paths = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Ok(output) = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(gui)
        .output()
    {
        if output.status.success() {
            git_paths.push(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    for path in git_paths {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--path-format=absolute", "--git-path", &path])
            .current_dir(gui)
            .output()
        {
            if output.status.success() {
                let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                if std::fs::symlink_metadata(&path).is_ok() {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
    }
}

fn emit_build_identity(root: &Path, version: &str) {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let git = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output();
    let source = git.ok().filter(|o| o.status.success()).map(|o| {
        String::from_utf8(o.stdout)
            .expect("Git SHA UTF-8")
            .trim()
            .to_string()
    });
    let supplied = env::var("GITHUB_SHA").ok();
    if let (Some(actual), Some(supplied)) = (&source, &supplied) {
        assert_eq!(actual, supplied, "GITHUB_SHA must match checkout HEAD");
    }
    let source = source.or(supplied).unwrap_or_else(|| "unknown".to_string());
    assert!(
        source == "unknown"
            || (source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit())),
        "invalid source SHA"
    );
    for name in ["HEAD", "packed-refs"] {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--path-format=absolute", "--git-path", name])
            .current_dir(root)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if Path::new(&path).exists() {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
        }
    }
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--symbolic-full-name", "HEAD"])
        .current_dir(root)
        .output()
    {
        let reference = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() && reference.starts_with("refs/") {
            if let Ok(path) = Command::new("git")
                .args([
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    &reference,
                ])
                .current_dir(root)
                .output()
            {
                if path.status.success() {
                    let path = PathBuf::from(String::from_utf8_lossy(&path.stdout).trim());
                    if let Some(watched) = path.ancestors().find(|candidate| candidate.exists()) {
                        println!("cargo:rerun-if-changed={}", watched.display());
                    }
                }
            }
        }
    }
    let payload = serde_json::json!({"schema_version": 1, "version": version, "source_sha": source, "target": env::var("TARGET").unwrap()}).to_string();
    assert!(payload.len() <= 1024, "build identity exceeds frame bound");
    let frame = [
        b"\0REFACT_BUILD_INFO_V1\0".as_slice(),
        payload.as_bytes(),
        b"\0END_REFACT_BUILD_INFO\0".as_slice(),
    ]
    .concat();
    std::fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("runtime-build-info.bin"),
        frame,
    )
    .unwrap();
}
