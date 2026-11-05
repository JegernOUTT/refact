#[path = "build_support/gui_artifact.rs"]
mod gui_artifact;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
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
            gui_artifact::Entry::Directory { path } | gui_artifact::Entry::File { path, .. } => {
                path
            }
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
