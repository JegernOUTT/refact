use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[path = "gui_artifact/trusted_controller.rs"]
mod trusted_controller;

pub const CONTROLLER: &str = "/etc/refact/worktree-cache/gui-controller.json";
pub const MANIFEST: &str = "gui-artifact.json";
const OWNER: &str = ".refact-gui-owner.json";
const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 30_000;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn refuse<T>(message: &str) -> Result<T> {
    Err(io::Error::other(message).into())
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encoded<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub node: String,
    pub node_abi: String,
    pub node_sha256: String,
    pub npm: String,
    pub npm_cli_sha256: String,
    pub typescript: String,
    pub vite: String,
    pub platform: String,
    pub arch: String,
    pub libc: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Entry {
    Directory {
        path: String,
    },
    File {
        path: String,
        size: u64,
        sha256: String,
    },
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Directory {
                path,
            }
            | Self::File {
                path, ..
            } => path,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    pub inventory: Vec<Entry>,
    pub producer_files: BTreeMap<String, String>,
    pub environment: BTreeMap<String, Option<String>>,
    pub version: String,
    pub commit: String,
    pub toolchain: Toolchain,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub kind: String,
    pub producer: String,
    pub inputs: Inputs,
    pub inputs_sha256: String,
    pub artifacts: Vec<Entry>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Controller {
    pub schema: u32,
    pub kind: String,
    pub manifest_sha256: String,
    pub toolchain: Toolchain,
}

struct Budget {
    bytes: u64,
    entries: usize,
}

impl Budget {
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or("GUI byte limit")?;
        self.entries += 1;
        if self.bytes > MAX_BYTES || self.entries > MAX_ENTRIES {
            return refuse("GUI inventory exceeds bounded byte/entry limit");
        }
        Ok(())
    }
}

fn valid_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.is_ascii()
        && !path.contains('\\')
        && !path.bytes().any(|b| b.is_ascii_control() || b == b':')
        && path.split('/').count() <= 48
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && p.len() <= 255)
}

pub fn real_directory(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return refuse("GUI paths must be absolute");
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if !matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::Normal(_)
        ) {
            return refuse("GUI paths cannot contain traversal");
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)?;
        if !metadata.is_dir() || is_link(&metadata) {
            return refuse("GUI directory or ancestor is a symlink/reparse point or non-directory");
        }
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
pub struct DirectoryIdentity(u64, u64);

pub fn directory_identity(path: &Path) -> Result<DirectoryIdentity> {
    real_directory(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        Ok(DirectoryIdentity(metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut [u32; 13],
            ) -> i32;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x02200000)
            .open(path)?;
        let mut info = [0u32; 13];
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if info[0] & 0x400 != 0 {
            return refuse("GUI directory became a reparse point");
        }
        Ok(DirectoryIdentity(
            u64::from(info[7]),
            (u64::from(info[11]) << 32) | u64::from(info[12]),
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        refuse("GUI directory identity unsupported on this platform")
    }
}

pub fn check_directory(path: &Path, expected: &DirectoryIdentity) -> Result<()> {
    if &directory_identity(path)? != expected {
        return refuse("GUI directory identity changed; retained without publication");
    }
    Ok(())
}

pub fn check_build_outputs(gui: &Path, engine: &Path) -> Result<()> {
    if gui.starts_with(engine.join("assets")) || engine.join("assets").starts_with(gui) {
        return refuse("GUI package and engine assets overlap");
    }
    for relative in [
        "node_modules",
        "node_modules/typescript",
        "node_modules/vite",
    ] {
        real_directory(&gui.join(relative))?;
    }
    for relative in [
        "node_modules/typescript/bin/tsc",
        "node_modules/vite/bin/vite.js",
    ] {
        regular(&gui.join(relative))?;
    }
    for relative in ["dist", "dist/chat", "dist/events"] {
        let path = gui.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                inventory(&path, false)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn regular(path: &Path) -> Result<File> {
    real_directory(path.parent().ok_or("GUI file has no parent")?)?;
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || is_link(&before) {
        return refuse("GUI input/artifact must be a regular non-link file");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let after = file.metadata()?;
    if !after.is_file() || is_link(&after) || after.len() > MAX_FILE {
        return refuse("GUI file type/size refused");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() || after.nlink() != 1 {
            return refuse("GUI file identity changed or has hard links");
        }
    }
    Ok(file)
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = regular(path)?;
    if file.metadata()?.len() > limit {
        return refuse("GUI document exceeds size bound");
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return refuse("GUI document grew beyond size bound");
    }
    Ok(bytes)
}

fn file_entry(path: &Path, relative: String, budget: &mut Budget) -> Result<Entry> {
    let mut file = regular(path)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        budget.charge(n as u64)?;
        if size > MAX_FILE {
            return refuse("GUI file grew beyond size bound");
        }
        hash.update(&buffer[..n]);
    }
    Ok(Entry::File {
        path: relative,
        size,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn excluded(relative: &str) -> bool {
    let root = relative.split('/').next().unwrap_or("");
    [
        "node_modules",
        "dist",
        ".git",
        ".refact",
        "coverage",
        "storybook-static",
        "test-results",
        "playwright-report",
        ".cache",
        ".vite",
    ]
    .contains(&root)
        || relative.ends_with(".tsbuildinfo")
        || relative == ".eslintcache"
        || relative.starts_with("vite.config.ts.timestamp-")
        || relative.starts_with("vite.node.config.ts.timestamp-")
}

pub fn inventory(root: &Path, source: bool) -> Result<Vec<Entry>> {
    fn walk(
        root: &Path,
        prefix: &str,
        source: bool,
        output: &mut Vec<Entry>,
        budget: &mut Budget,
    ) -> Result<()> {
        let directory = if prefix.is_empty() {
            root.to_path_buf()
        } else {
            root.join(prefix)
        };
        real_directory(&directory)?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&directory)? {
            budget.charge(0)?;
            names.push(
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|_| "Non-UTF8 GUI path")?,
            );
        }
        names.sort();
        for name in names {
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if !valid_relative(&relative) {
                return refuse("Unsafe GUI inventory path");
            }
            if source && excluded(&relative) {
                continue;
            }
            let path = root.join(&relative);
            let metadata = fs::symlink_metadata(&path)?;
            if is_link(&metadata) {
                return refuse("GUI inventory refuses symlinks/reparse points");
            }
            if metadata.is_dir() {
                output.push(Entry::Directory {
                    path: relative.clone(),
                });
                walk(root, &relative, source, output, budget)?;
            } else if metadata.is_file() {
                output.push(file_entry(&path, relative, budget)?);
            } else {
                return refuse("GUI inventory refuses special files");
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    walk(
        root,
        "",
        source,
        &mut output,
        &mut Budget {
            bytes: 0,
            entries: 0,
        },
    )?;
    Ok(output)
}

pub fn merged_node_options(incoming: Option<OsString>) -> OsString {
    let mut options = incoming.unwrap_or_default();
    let text = options.to_string_lossy();
    if !text.contains("--max-old-space-size") && !text.contains("--max_old_space_size") {
        if !options.is_empty() {
            options.push(" ");
        }
        options.push("--max-old-space-size=8192");
    }
    options
}

pub fn offline() -> bool {
    [
        "REFACT_OFFLINE_NETNS",
        "CARGO_NET_OFFLINE",
        "npm_config_offline",
        "NPM_CONFIG_OFFLINE",
    ]
    .iter()
    .any(|key| {
        std::env::var(key).is_ok_and(|value| !value.is_empty() && value != "0" && value != "false")
    })
}

fn host() -> (&'static str, &'static str, &'static str) {
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "gnu") {
        "glibc"
    } else {
        "none"
    };
    (platform, arch, libc)
}

fn validate_toolchain(toolchain: &Toolchain) -> Result<()> {
    let (platform, arch, libc) = host();
    if toolchain.platform != platform
        || toolchain.arch != arch
        || toolchain.libc != libc
        || toolchain.node.is_empty()
        || toolchain.node_abi.is_empty()
        || toolchain.npm.is_empty()
        || toolchain.typescript.is_empty()
        || toolchain.vite.is_empty()
        || !is_digest(&toolchain.node_sha256)
        || !is_digest(&toolchain.npm_cli_sha256)
        || encoded(toolchain)?.len() > 4096
    {
        return refuse("GUI producer toolchain/platform does not match authorized host identity");
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn output(command: &mut Command, label: &str) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return refuse(&format!(
            "GUI local tool identity command failed ({label}, status={}, stdout_exceeds_limit={})",
            output.status,
            output.stdout.len() > 4096
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub fn resolve_program(program: &str) -> Result<PathBuf> {
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            let canonical = fs::canonicalize(candidate)?;
            regular(&canonical)?;
            return Ok(canonical);
        }
    }
    refuse("GUI build needs installed local Node/npm binaries; no download attempted")
}

pub fn producer_toolchain(gui: &Path, node: &Path, options: &OsString) -> Result<Toolchain> {
    let script = "console.log(JSON.stringify({node:process.version,node_abi:process.versions.modules,typescript:require('./node_modules/typescript/package.json').version,vite:require('./node_modules/vite/package.json').version}));";
    let data = output(
        Command::new(node)
            .args(["-e", script])
            .env("NODE_OPTIONS", options)
            .current_dir(gui),
        "node-identity",
    )?;
    let value: serde_json::Value = serde_json::from_str(&data)?;
    let field = |name: &str| -> Result<String> {
        Ok(value[name]
            .as_str()
            .ok_or("Missing GUI tool identity")?
            .to_string())
    };
    let npm = resolve_program(if cfg!(windows) { "npm.cmd" } else { "npm" })?;
    let npm_version = output(
        Command::new(&npm)
            .arg("--version")
            .env("NODE_OPTIONS", options)
            .current_dir(gui),
        "npm-version",
    )?;
    let (platform, arch, libc) = host();
    let toolchain = Toolchain {
        node: field("node")?,
        node_abi: field("node_abi")?,
        node_sha256: digest(&read(node, MAX_FILE)?),
        npm: npm_version,
        npm_cli_sha256: digest(&read(&npm, MAX_FILE)?),
        typescript: field("typescript")?,
        vite: field("vite")?,
        platform: platform.into(),
        arch: arch.into(),
        libc: libc.into(),
    };
    validate_toolchain(&toolchain)?;
    Ok(toolchain)
}

pub fn inputs(gui: &Path, engine: &Path, toolchain: Toolchain) -> Result<Inputs> {
    validate_toolchain(&toolchain)?;
    let inventory = inventory(gui, true)?;
    for required in [
        "package.json",
        "package-lock.json",
        "src/lib/index.ts",
        "vite.config.ts",
        "vite.node.config.ts",
        "tsconfig.json",
    ] {
        if !inventory
            .iter()
            .any(|e| matches!(e, Entry::File { path, .. } if path == required))
        {
            return refuse("GUI source inventory is incomplete");
        }
    }
    let package: serde_json::Value =
        serde_json::from_slice(&read(&gui.join("package.json"), MAX_JSON)?)?;
    let lock: serde_json::Value =
        serde_json::from_slice(&read(&gui.join("package-lock.json"), MAX_JSON)?)?;
    let root = &lock["packages"][""];
    if !matches!(lock["lockfileVersion"].as_u64(), Some(2 | 3)) || !root.is_object() {
        return refuse("GUI requires npm lockfile v2/v3 with a root package");
    }
    for field in [
        "name",
        "version",
        "dependencies",
        "devDependencies",
        "optionalDependencies",
    ] {
        if package.get(field) != root.get(field) {
            return refuse("GUI package/lockfile mismatch");
        }
    }
    let version = package["version"]
        .as_str()
        .ok_or("Missing GUI package version")?
        .to_string();
    let mut environment: BTreeMap<_, _> = [
        "DEBUG",
        "REFACT_LSP_PORT",
        "NODE_OPTIONS",
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "VERCEL_GIT_COMMIT_SHA",
        "BUILD_VCS_NUMBER",
        "NODE_ENV",
    ]
    .into_iter()
    .map(|name| {
        let value = match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(error),
        }?;
        Ok((name.to_string(), value.map(|s| digest(s.as_bytes()))))
    })
    .collect::<std::result::Result<_, std::env::VarError>>()?;
    for (key, value) in std::env::vars_os() {
        let key = key
            .into_string()
            .map_err(|_| "Non-UTF8 GUI environment key")?;
        if key.starts_with("VITE_") {
            let value = value
                .into_string()
                .map_err(|_| "Non-UTF8 Vite environment")?;
            environment.insert(key, Some(digest(value.as_bytes())));
        }
    }
    let commit = match [
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "VERCEL_GIT_COMMIT_SHA",
        "BUILD_VCS_NUMBER",
    ]
    .into_iter()
    .find_map(|key| std::env::var(key).ok())
    .filter(|s| s.len() >= 7)
    {
        Some(value) => value.chars().take(7).collect(),
        None => output(
            Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(gui),
            "git-version-identity",
        )?,
    };
    let mut producer_files = BTreeMap::new();
    for name in [
        "build.rs",
        "build_support/gui_artifact.rs",
        "build_support/gui_artifact/trusted_controller.rs",
        "assets/chat/index.html",
    ] {
        producer_files.insert(name.into(), digest(&read(&engine.join(name), MAX_FILE)?));
    }
    Ok(Inputs {
        inventory,
        producer_files,
        environment,
        version,
        commit,
        toolchain,
    })
}

pub fn produce(gui: &Path, inputs: Inputs) -> Result<Manifest> {
    let artifacts = inventory(&gui.join("dist/chat"), false)?;
    complete(&artifacts)?;
    let manifest = Manifest {
        schema: 1,
        kind: "refact-gui-artifact".into(),
        producer: "refact-engine-build-v1".into(),
        inputs_sha256: digest(&encoded(&inputs)?),
        inputs,
        artifacts,
    };
    let path = gui.join("dist").join(MANIFEST);
    if fs::symlink_metadata(&path).is_ok() {
        regular(&path)?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(gui.join("dist"))?;
    temporary.write_all(&encoded(&manifest)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(manifest)
}

fn complete(entries: &[Entry]) -> Result<()> {
    if entries.len() > MAX_ENTRIES {
        return refuse("GUI artifact inventory too large");
    }
    for required in ["index.js", "index.umd.cjs", "style.css", "index.d.ts"] {
        if !entries
            .iter()
            .any(|e| matches!(e, Entry::File { path, size, .. } if path == required && *size > 0))
        {
            return refuse("GUI artifact is incomplete: missing required nonempty entrypoint");
        }
    }
    for entry in entries {
        if !valid_relative(entry.path()) || entry.path().split('/').any(|p| p == OWNER) {
            return refuse("Unsafe/reserved GUI artifact path");
        }
        if let Entry::File {
            size,
            sha256,
            ..
        } = entry
        {
            if *size > MAX_FILE || !is_digest(sha256) {
                return refuse("Invalid GUI artifact digest/size");
            }
        }
    }
    Ok(())
}

pub fn trusted_prebuilt(gui: &Path, engine: &Path) -> Result<Manifest> {
    let controller = trusted_controller::load()?;
    let manifest = validate_prebuilt(gui, engine, &controller)?;
    if trusted_controller::load()? != controller {
        return refuse("GUI controller changed during validation");
    }
    Ok(manifest)
}

pub fn validate_prebuilt(gui: &Path, engine: &Path, controller: &Controller) -> Result<Manifest> {
    if controller.schema != 1
        || controller.kind != "refact-gui-artifact-controller"
        || !is_digest(&controller.manifest_sha256)
    {
        return refuse("GUI prebuilt provenance controller is invalid");
    }
    validate_toolchain(&controller.toolchain)?;
    let bytes = read(&gui.join("dist").join(MANIFEST), MAX_JSON)?;
    if digest(&bytes) != controller.manifest_sha256 {
        return refuse("GUI prebuilt producer manifest does not match independent controller pin");
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    let current = inputs(gui, engine, controller.toolchain.clone())?;
    if manifest.schema != 1
        || manifest.kind != "refact-gui-artifact"
        || manifest.producer != "refact-engine-build-v1"
        || manifest.inputs != current
        || manifest.inputs_sha256 != digest(&encoded(&current)?)
    {
        return refuse("GUI prebuilt miss: exact source/config/lock/producer/environment/toolchain identity mismatch");
    }
    complete(&manifest.artifacts)?;
    if inventory(&gui.join("dist/chat"), false)? != manifest.artifacts {
        return refuse("GUI prebuilt artifact integrity/completeness mismatch");
    }
    Ok(manifest)
}

pub fn publish(
    gui: &Path,
    engine: &Path,
    manifest: &Manifest,
    allow_legacy: bool,
    revalidate: impl FnOnce() -> Result<()>,
) -> Result<()> {
    complete(&manifest.artifacts)?;
    if gui.starts_with(engine.join("assets")) || engine.join("assets").starts_with(gui) {
        return refuse("GUI package and engine assets overlap");
    }
    let assets = engine.join("assets");
    real_directory(&assets)?;
    real_directory(&assets.join("chat"))?;
    let parent = assets.join("chat/dist");
    match fs::symlink_metadata(&parent) {
        Ok(_) => real_directory(&parent)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&parent)?,
        Err(error) => return Err(error.into()),
    }
    let destination = parent.join("chat");
    let parent_identity = directory_identity(&parent)?;
    let prior = match fs::symlink_metadata(&destination) {
        Ok(_) => {
            validate_existing(&destination, engine, allow_legacy)?;
            Some(directory_identity(&destination)?)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let retained = tempfile::Builder::new()
        .prefix(".refact-gui-")
        .tempdir_in(&assets)?
        .keep();
    let stage = retained.join("chat");
    fs::create_dir(&stage)?;
    let stage_identity = directory_identity(&stage)?;
    println!(
        "cargo:warning=GUI private staging/retention directory: {}",
        retained.display()
    );
    for entry in &manifest.artifacts {
        let target = stage.join(entry.path());
        match entry {
            Entry::Directory {
                ..
            } => fs::create_dir(&target)?,
            Entry::File {
                path,
                size,
                sha256,
            } => {
                let bytes = read(&gui.join("dist/chat").join(path), MAX_FILE)?;
                if bytes.len() as u64 != *size || digest(&bytes) != *sha256 {
                    return refuse("GUI artifact changed during staging; prior output retained");
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
        }
    }
    if inventory(&stage, false)? != manifest.artifacts
        || inventory(&gui.join("dist/chat"), false)? != manifest.artifacts
        || inputs(gui, engine, manifest.inputs.toolchain.clone())? != manifest.inputs
    {
        return refuse("GUI inputs/artifacts changed before publication; prior output retained");
    }
    let marker = encoded(&Owner {
        schema: 1,
        engine_sha256: digest(engine.as_os_str().as_encoded_bytes()),
        artifacts: manifest.artifacts.clone(),
    })?;
    let mut owner = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join(OWNER))?;
    owner.write_all(&marker)?;
    owner.sync_all()?;
    drop(owner);
    revalidate()?;
    if inventory(&stage, false)?
        .into_iter()
        .filter(|e| e.path() != OWNER)
        .collect::<Vec<_>>()
        != manifest.artifacts
    {
        return refuse("GUI stage changed during final authorization; prior output retained");
    }
    check_directory(&parent, &parent_identity)?;
    check_directory(&stage, &stage_identity)?;
    if let Some(prior_identity) = prior {
        validate_existing(&destination, engine, allow_legacy)?;
        check_directory(&destination, &prior_identity)?;
        replace(
            &stage,
            &destination,
            &retained.join("previous"),
            &parent_identity,
            &stage_identity,
            &prior_identity,
        )?;
        println!(
            "cargo:warning=GUI published; prior assets retained outside embed tree at {}",
            retained.display()
        );
    } else {
        rename_noreplace(&stage, &destination)?;
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema: u32,
    engine_sha256: String,
    artifacts: Vec<Entry>,
}

fn validate_existing(destination: &Path, engine: &Path, allow_legacy: bool) -> Result<()> {
    real_directory(destination)?;
    let marker = destination.join(OWNER);
    match fs::symlink_metadata(&marker) {
        Ok(_) => {
            let owner: Owner = serde_json::from_slice(&read(&marker, MAX_JSON)?)?;
            let entries: Vec<_> = inventory(destination, false)?
                .into_iter()
                .filter(|e| e.path() != OWNER)
                .collect();
            if owner.schema != 1
                || owner.engine_sha256 != digest(engine.as_os_str().as_encoded_bytes())
                || entries != owner.artifacts
            {
                return refuse("Existing GUI output ownership/content mismatch; preserved");
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !allow_legacy {
                return refuse("Existing GUI output is unowned; preserved. An explicit normal local build may retain/migrate legacy assets");
            }
            inventory(destination, false)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn replace(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    parent_identity: &DirectoryIdentity,
    stage_identity: &DirectoryIdentity,
    prior_identity: &DirectoryIdentity,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        rename_linux(stage, destination, libc::RENAME_EXCHANGE)?;
        let _ = (backup, parent_identity, stage_identity, prior_identity);
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        rename_noreplace(destination, backup)?;
        let publication = (|| {
            check_directory(backup, prior_identity)?;
            check_directory(stage, stage_identity)?;
            check_directory(
                destination
                    .parent()
                    .ok_or("Missing GUI destination parent")?,
                parent_identity,
            )?;
            rename_noreplace(stage, destination)
        })();
        if let Err(error) = publication {
            check_directory(backup, prior_identity)?;
            check_directory(
                destination
                    .parent()
                    .ok_or("Missing GUI destination parent")?,
                parent_identity,
            )?;
            return match rename_noreplace(backup, destination) {
                Ok(()) => Err(error),
                Err(rollback) => refuse(&format!("GUI publish failed: {error}; rollback failed: {rollback}; prior output retained at {}", backup.display())),
            };
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn rename_linux(from: &Path, to: &Path, flags: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            flags,
        )
    } != 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        rename_linux(from, to, libc::RENAME_NOREPLACE)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (from, to);
        refuse("GUI no-replace publication unsupported on this platform")
    }
}
