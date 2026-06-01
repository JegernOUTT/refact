use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=REFACT_SKIP_GUI_BUILD");
    println!("cargo:rerun-if-env-changed=REFACT_USE_PREBUILT_GUI");
    println!("cargo:rerun-if-changed=../gui/package.json");
    println!("cargo:rerun-if-changed=../gui/package-lock.json");
    println!("cargo:rerun-if-changed=../gui/src");
    println!("cargo:rerun-if-changed=assets/chat/index.html");

    if env::var("REFACT_SKIP_GUI_BUILD").ok().as_deref() != Some("1") {
        build_gui_assets();
    }

    shadow_rs::ShadowBuilder::builder().build().unwrap();
}

fn build_gui_assets() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let gui_dir = manifest_dir.parent().unwrap().join("gui");
    let dist_dir = gui_dir.join("dist").join("chat");
    let asset_dist_dir = manifest_dir.join("assets").join("chat").join("dist");
    let target_chat_dir = asset_dist_dir.join("chat");

    if !gui_dir.join("package.json").exists() {
        panic!("GUI package.json not found at {}", gui_dir.display());
    }

    if env::var("REFACT_USE_PREBUILT_GUI").ok().as_deref() != Some("1") {
        if !gui_dir.join("node_modules").exists() {
            run_command(&gui_dir, npm_program(), &["ci"]);
        }

        run_command(&gui_dir, npm_program(), &["run", "build"]);
    }

    if target_chat_dir.exists() {
        std::fs::remove_dir_all(&target_chat_dir).unwrap_or_else(|error| {
            panic!(
                "failed to remove old GUI assets at {}: {error}",
                target_chat_dir.display()
            )
        });
    }
    std::fs::create_dir_all(&asset_dist_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create GUI asset directory {}: {error}",
            asset_dist_dir.display()
        )
    });
    copy_dir_all(&dist_dir, &target_chat_dir).unwrap_or_else(|error| {
        panic!(
            "failed to copy GUI assets from {} to {}: {error}",
            dist_dir.display(),
            target_chat_dir.display()
        )
    });
}

fn run_command(cwd: &Path, program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {program} in {}: {error}", cwd.display()));
    if !status.success() {
        panic!(
            "command failed in {}: {} {}",
            cwd.display(),
            program,
            args.join(" ")
        );
    }
}

fn npm_program() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
