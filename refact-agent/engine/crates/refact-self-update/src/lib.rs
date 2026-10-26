use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

const GITHUB_API_BASE: &str = "https://api.github.com/repos/JegernOUTT/refact/releases";
const GITHUB_DOWNLOAD_BASE: &str = "https://github.com/JegernOUTT/refact/releases/download";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
const API_PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b'/')
    .add(b'?')
    .add(b'#')
    .add(b'[')
    .add(b']')
    .add(b'@')
    .add(b':');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdateOptions {
    pub check: bool,
    pub version: Option<String>,
    pub force: bool,
    pub quiet: bool,
    pub json: bool,
}

#[derive(Debug)]
struct SelfUpdateError {
    pub message: String,
    pub exit_code: i32,
}

impl SelfUpdateError {
    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetUrls {
    pub archive_name: String,
    pub archive_url: String,
    pub sha256_name: String,
    pub sha256_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub releases: Vec<ReleaseInfo>,
}

#[derive(Debug, Clone)]
pub struct UpdateInstallOutcome {
    pub installed_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineReleaseInfo {
    pub version: String,
    pub tag: String,
    assets: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpdateReason {
    NewerVersion,
    Force,
    ExplicitVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateDecision {
    Update {
        version: String,
        reason: UpdateReason,
    },
    NoUpdate,
}

#[derive(Debug, Serialize)]
struct CheckOutput<'a> {
    ok: bool,
    current_version: &'a str,
    latest_version: &'a str,
    target_version: Option<&'a str>,
    update_available: bool,
    reason: Option<UpdateReason>,
    target: &'a str,
    archive_url: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateOutput<'a> {
    ok: bool,
    old_version: &'a str,
    new_version: &'a str,
    target: &'a str,
    binary_path: String,
    restart_hint: &'a str,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

type ReleaseFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SelfUpdateError>> + Send + 'a>>;

trait ReleaseSource: Sync {
    fn latest_release<'a>(&'a self) -> ReleaseFuture<'a, EngineReleaseInfo>;
    fn release_for_version<'a>(&'a self, version: &'a str) -> ReleaseFuture<'a, EngineReleaseInfo>;
    fn download<'a>(&'a self, url: &'a str, label: &'static str) -> ReleaseFuture<'a, Vec<u8>>;
}

struct ReqwestReleaseSource {
    client: reqwest::Client,
}

impl ReqwestReleaseSource {
    fn new(current_version: &str) -> Result<Self, SelfUpdateError> {
        // Public GitHub release downloads carry no credentials and redirect to asset storage.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .user_agent(format!("refact-self-update/{current_version}"))
            .build()
            .map_err(|error| {
                SelfUpdateError::runtime(format!("failed to build HTTP client: {error}"))
            })?;
        Ok(Self { client })
    }

    async fn get_json(&self, url: &str) -> Result<GitHubRelease, SelfUpdateError> {
        let response = self.client.get(url).send().await.map_err(|error| {
            SelfUpdateError::runtime(format!(
                "failed to contact GitHub Releases at {url}: {error}. Check your network connection and try again."
            ))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SelfUpdateError::runtime(format!(
                "GitHub Releases request failed with status {status}: {}",
                truncate_error_body(&body)
            )));
        }
        let release = response.json::<GitHubRelease>().await.map_err(|error| {
            SelfUpdateError::runtime(format!("GitHub Releases returned invalid JSON: {error}"))
        })?;
        Ok(release)
    }

    async fn get_latest_engine_release(&self) -> Result<EngineReleaseInfo, SelfUpdateError> {
        let releases = self.get_releases(20).await?;
        latest_engine_release_from(releases)
    }

    async fn get_releases(&self, per_page: usize) -> Result<Vec<GitHubRelease>, SelfUpdateError> {
        let url = format!("{GITHUB_API_BASE}?per_page={}", per_page.clamp(1, 100));
        let response = self.client.get(&url).send().await.map_err(|error| {
            SelfUpdateError::runtime(format!(
                "failed to contact GitHub Releases at {url}: {error}. Check your network connection and try again."
            ))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SelfUpdateError::runtime(format!(
                "GitHub Releases request failed with status {status}: {}",
                truncate_error_body(&body)
            )));
        }
        let releases = response
            .json::<Vec<GitHubRelease>>()
            .await
            .map_err(|error| {
                SelfUpdateError::runtime(format!("GitHub Releases returned invalid JSON: {error}"))
            })?;
        Ok(releases)
    }

    async fn get_bytes(&self, url: &str, label: &str) -> Result<Vec<u8>, SelfUpdateError> {
        let response = self.client.get(url).send().await.map_err(|error| {
            SelfUpdateError::runtime(format!(
                "failed to download {label} from {url}: {error}. Check your network connection and try again."
            ))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SelfUpdateError::runtime(format!(
                "download failed for {label} with status {status}: {}",
                truncate_error_body(&body)
            )));
        }
        let bytes = response.bytes().await.map_err(|error| {
            SelfUpdateError::runtime(format!("failed to read {label} download: {error}"))
        })?;
        Ok(bytes.to_vec())
    }
}

impl ReleaseSource for ReqwestReleaseSource {
    fn latest_release<'a>(&'a self) -> ReleaseFuture<'a, EngineReleaseInfo> {
        Box::pin(async move { self.get_latest_engine_release().await })
    }

    fn release_for_version<'a>(&'a self, version: &'a str) -> ReleaseFuture<'a, EngineReleaseInfo> {
        Box::pin(async move {
            let version = normalize_version(version).map_err(SelfUpdateError::runtime)?;
            let tag = default_release_tag(&version);
            self.get_json(&format!(
                "{GITHUB_API_BASE}/tags/{}",
                api_path_segment(&tag)
            ))
            .await
            .and_then(release_info_from_github)
        })
    }

    fn download<'a>(&'a self, url: &'a str, label: &'static str) -> ReleaseFuture<'a, Vec<u8>> {
        Box::pin(async move { self.get_bytes(url, label).await })
    }
}

pub fn parse_self_update_args(args: &[OsString]) -> Result<SelfUpdateOptions, String> {
    let mut check = false;
    let mut version = None;
    let mut force = false;
    let mut quiet = false;
    let mut json = false;
    let mut i = 0usize;
    while i < args.len() {
        let arg = os_to_string(&args[i])?;
        match arg.as_str() {
            "--check" => check = true,
            "--version" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--version requires a version".to_string())?;
                version = Some(normalize_version(&os_to_string(value)?)?);
            }
            "--force" => force = true,
            "--quiet" => quiet = true,
            "--json" => json = true,
            value if value.starts_with("--version=") => {
                version = Some(normalize_version(&value["--version=".len()..])?);
            }
            value => return Err(format!("unexpected self-update argument `{value}`")),
        }
        i += 1;
    }
    Ok(SelfUpdateOptions {
        check,
        version,
        force,
        quiet,
        json,
    })
}

pub async fn run(options: SelfUpdateOptions) -> i32 {
    run_for_current_version(options, env!("REFACT_ENGINE_VERSION")).await
}

pub async fn check_update_info(limit: usize) -> Result<UpdateCheckInfo, String> {
    let current_version = env!("REFACT_ENGINE_VERSION");
    let source = ReqwestReleaseSource::new(current_version).map_err(|error| error.message)?;
    let releases = source
        .get_releases(limit.max(1))
        .await
        .map_err(|error| error.message)?;
    Ok(update_check_info_from_releases(
        current_version,
        releases,
        limit,
    ))
}

pub async fn install_update_version(
    version: Option<&str>,
    force: bool,
) -> Result<UpdateInstallOutcome, String> {
    let current_version = env!("REFACT_ENGINE_VERSION");
    let source = ReqwestReleaseSource::new(current_version).map_err(|error| error.message)?;
    install_update_with_source(current_version, &source, version, force, None)
        .await
        .map_err(|error| error.message)
}

async fn run_for_current_version(options: SelfUpdateOptions, current_version: &str) -> i32 {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    run_with_io_for_current_version(options, current_version, &mut stdout, &mut stderr).await
}

async fn run_with_io_for_current_version(
    options: SelfUpdateOptions,
    current_version: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let json_output = options.json;
    let source = match ReqwestReleaseSource::new(current_version) {
        Ok(source) => source,
        Err(error) => return write_error(error, json_output, stdout, stderr),
    };
    match run_with_source_for_current_version(options, current_version, &source, None, stdout).await
    {
        Ok(code) => code,
        Err(error) => write_error(error, json_output, stdout, stderr),
    }
}

async fn run_with_source_for_current_version(
    options: SelfUpdateOptions,
    current_version: &str,
    source: &dyn ReleaseSource,
    exe_path: Option<PathBuf>,
    out: &mut dyn Write,
) -> Result<i32, SelfUpdateError> {
    let target = current_target_triple().ok_or_else(|| {
        SelfUpdateError::runtime(format!(
            "unsupported self-update target: {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let release = match options.version.as_deref() {
        Some(version) => source.release_for_version(version).await?,
        None => source.latest_release().await?,
    };
    let decision = decide_update(
        current_version,
        &release.version,
        options.force,
        options.version.as_deref(),
    );
    let target_version = match &decision {
        UpdateDecision::Update { version, .. } => Some(version.as_str()),
        UpdateDecision::NoUpdate => None,
    };
    let urls = release.asset_urls(target);
    if options.check {
        write_check_output(
            &options,
            out,
            current_version,
            &release,
            &decision,
            target,
            &urls,
        )?;
        return Ok(0);
    }
    let Some(new_version) = target_version else {
        if options.json {
            print_json(
                out,
                &json!({"ok": true, "updated": false, "current_version": current_version, "latest_version": release.version, "target": target}),
            )?;
        } else if !options.quiet {
            writeln!(out, "refact is up to date ({current_version})").map_err(write_error_io)?;
        }
        return Ok(0);
    };
    let exe_path = install_release(source, target, &urls, exe_path).await?;
    if options.json {
        print_json(
            out,
            &UpdateOutput {
                ok: true,
                old_version: current_version,
                new_version,
                target,
                binary_path: exe_path.display().to_string(),
                restart_hint: restart_hint(),
            },
        )?;
    } else if !options.quiet {
        writeln!(out, "refact updated {current_version} -> {new_version}")
            .map_err(write_error_io)?;
        writeln!(out, "{}", restart_hint()).map_err(write_error_io)?;
    }
    Ok(0)
}

async fn install_update_with_source(
    current_version: &str,
    source: &dyn ReleaseSource,
    version: Option<&str>,
    force: bool,
    exe_path: Option<PathBuf>,
) -> Result<UpdateInstallOutcome, SelfUpdateError> {
    let target = current_target_triple().ok_or_else(|| {
        SelfUpdateError::runtime(format!(
            "unsupported self-update target: {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let release = match version {
        Some(version) => source.release_for_version(version).await?,
        None => source.latest_release().await?,
    };
    let decision = decide_update(current_version, &release.version, force, version);
    let UpdateDecision::Update { version, .. } = decision else {
        return Err(SelfUpdateError::runtime(format!(
            "refact is up to date ({current_version})"
        )));
    };
    let urls = release.asset_urls(target);
    install_release(source, target, &urls, exe_path).await?;
    Ok(UpdateInstallOutcome {
        installed_version: version,
    })
}

async fn install_release(
    source: &dyn ReleaseSource,
    target: &str,
    urls: &AssetUrls,
    exe_path: Option<PathBuf>,
) -> Result<PathBuf, SelfUpdateError> {
    let exe_path = match exe_path {
        Some(path) => path,
        None => {
            let path = std::env::current_exe().map_err(|error| {
                SelfUpdateError::runtime(format!(
                    "failed to find current refact binary path: {error}"
                ))
            })?;
            std::fs::canonicalize(&path).unwrap_or(path)
        }
    };
    let archive = source
        .download(&urls.archive_url, "release archive")
        .await?;
    let sha256 = source.download(&urls.sha256_url, "sha256 sidecar").await?;
    verify_sha256(
        &archive,
        std::str::from_utf8(&sha256).map_err(|error| {
            SelfUpdateError::runtime(format!("sha256 sidecar is not valid UTF-8: {error}"))
        })?,
    )?;
    #[cfg(target_os = "linux")]
    {
        let bytes = match archive_kind_for_target(target) {
            ArchiveKind::TarGz => extract_from_targz(&archive, binary_name())?,
            ArchiveKind::Zip => extract_from_zip(&archive, binary_name())?,
        };
        // Both native installation entry points participate in the same pinned
        // directory transaction, including its interrupted-transaction marker.
        candidate_install::Installer::open(&exe_path)
            .and_then(|installer| installer.install_release_bytes(&bytes))
            .map_err(SelfUpdateError::runtime)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let temp_path = temp_binary_path(&exe_path);
        extract_binary_to_path(&archive, archive_kind_for_target(target), &temp_path)?;
        replace_binary(&exe_path, &temp_path)?;
    }
    Ok(exe_path)
}

fn decide_update(
    current_version: &str,
    latest_version: &str,
    force: bool,
    explicit_version: Option<&str>,
) -> UpdateDecision {
    if let Some(version) = explicit_version {
        return UpdateDecision::Update {
            version: normalize_version(version).unwrap_or_else(|_| version.to_string()),
            reason: UpdateReason::ExplicitVersion,
        };
    }
    if refact_core::semver::compare_versions(current_version, latest_version) == Ordering::Less {
        return UpdateDecision::Update {
            version: latest_version.to_string(),
            reason: UpdateReason::NewerVersion,
        };
    }
    if force {
        return UpdateDecision::Update {
            version: latest_version.to_string(),
            reason: UpdateReason::Force,
        };
    }
    UpdateDecision::NoUpdate
}

fn current_target_triple() -> Option<&'static str> {
    target_triple_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(target_env = "msvc") {
            Some("msvc")
        } else {
            None
        },
    )
}

fn target_triple_for(os: &str, arch: &str, env: Option<&str>) -> Option<&'static str> {
    match (os, arch, env) {
        ("windows", "x86_64", Some("msvc")) => Some("x86_64-pc-windows-msvc"),
        ("windows", "x86", Some("msvc")) => Some("i686-pc-windows-msvc"),
        ("windows", "aarch64", Some("msvc")) => Some("aarch64-pc-windows-msvc"),
        ("linux", "x86_64", _) => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64", _) => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64", _) => Some("x86_64-apple-darwin"),
        ("macos", "aarch64", _) => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

#[cfg(test)]
fn asset_urls_for_version(version: &str, target: &str) -> AssetUrls {
    let version = normalize_version(version).unwrap_or_else(|_| version.to_string());
    release_asset_urls(&default_release_tag(&version), &version, target)
}

impl EngineReleaseInfo {
    fn asset_urls(&self, target: &str) -> AssetUrls {
        let mut urls = release_asset_urls(&self.tag, &self.version, target);
        if let Some(url) = self.assets.get(&urls.archive_name) {
            urls.archive_url = url.clone();
        }
        if let Some(url) = self.assets.get(&urls.sha256_name) {
            urls.sha256_url = url.clone();
        }
        urls
    }
}

fn release_info_from_github(release: GitHubRelease) -> Result<EngineReleaseInfo, SelfUpdateError> {
    let version = parse_version_from_tag(&release.tag_name)?;
    let assets = release
        .assets
        .into_iter()
        .map(|asset| (asset.name, asset.browser_download_url))
        .collect::<HashMap<_, _>>();
    Ok(EngineReleaseInfo {
        version,
        tag: release.tag_name,
        assets,
    })
}

fn latest_engine_release_from(
    releases: Vec<GitHubRelease>,
) -> Result<EngineReleaseInfo, SelfUpdateError> {
    releases
        .into_iter()
        .filter(|release| {
            release.tag_name.starts_with("engine/v") && !release.draft && !release.prerelease
        })
        .filter_map(|release| release_info_from_github(release).ok())
        .max_by(|left, right| refact_core::semver::compare_versions(&left.version, &right.version))
        .ok_or_else(|| {
            SelfUpdateError::runtime("GitHub Releases did not include an engine/v* release")
        })
}

fn update_check_info_from_releases(
    current_version: &str,
    releases: Vec<GitHubRelease>,
    limit: usize,
) -> UpdateCheckInfo {
    let mut public_releases = Vec::new();
    let mut latest_version = None::<String>;
    for release in releases {
        if release.draft || !release.tag_name.starts_with("engine/v") {
            continue;
        }
        let Ok(version) = parse_version_from_tag(&release.tag_name) else {
            continue;
        };
        if !release.prerelease {
            latest_version = match latest_version {
                Some(previous)
                    if refact_core::semver::compare_versions(&previous, &version)
                        != Ordering::Less =>
                {
                    Some(previous)
                }
                _ => Some(version.clone()),
            };
        }
        if public_releases.len() < limit {
            public_releases.push(ReleaseInfo {
                version,
                published_at: release.published_at,
                prerelease: release.prerelease,
                url: release.html_url,
            });
        }
    }
    let update_available = latest_version
        .as_deref()
        .map(|version| {
            refact_core::semver::compare_versions(current_version, version) == Ordering::Less
        })
        .unwrap_or(false);
    UpdateCheckInfo {
        current_version: current_version.to_string(),
        latest_version,
        update_available,
        releases: public_releases,
    }
}

fn release_asset_urls(tag: &str, version: &str, target: &str) -> AssetUrls {
    let ext = archive_extension_for_target(target);
    let archive_name = format!("refact-{version}-{target}{ext}");
    let archive_url = format!("{GITHUB_DOWNLOAD_BASE}/{tag}/{archive_name}");
    let sha256_name = format!("{archive_name}.sha256");
    let sha256_url = format!("{archive_url}.sha256");
    AssetUrls {
        archive_name,
        archive_url,
        sha256_name,
        sha256_url,
    }
}

fn archive_extension_for_target(target: &str) -> &'static str {
    if target.contains("windows") {
        ".zip"
    } else {
        ".tar.gz"
    }
}

fn default_release_tag(version: &str) -> String {
    format!(
        "engine/v{}",
        normalize_version(version).unwrap_or_else(|_| version.to_string())
    )
}

fn api_path_segment(value: &str) -> String {
    utf8_percent_encode(value, API_PATH_SEGMENT).to_string()
}

fn parse_version_from_tag(tag: &str) -> Result<String, SelfUpdateError> {
    let value = tag.rsplit('/').next().unwrap_or(tag);
    normalize_version(value).map_err(SelfUpdateError::runtime)
}

fn normalize_version(version: &str) -> Result<String, String> {
    let trimmed = version.trim();
    let tail = trimmed.rsplit('/').next().unwrap_or(trimmed).trim();
    let value = tail.strip_prefix('v').unwrap_or(tail).trim();
    if value.is_empty() {
        return Err("version must not be empty".to_string());
    }
    Ok(value.to_string())
}

fn write_check_output(
    options: &SelfUpdateOptions,
    out: &mut dyn Write,
    current_version: &str,
    release: &EngineReleaseInfo,
    decision: &UpdateDecision,
    target: &str,
    urls: &AssetUrls,
) -> Result<(), SelfUpdateError> {
    let (target_version, reason) = match decision {
        UpdateDecision::Update { version, reason } => (Some(version.as_str()), Some(*reason)),
        UpdateDecision::NoUpdate => (None, None),
    };
    if options.json {
        print_json(
            out,
            &CheckOutput {
                ok: true,
                current_version,
                latest_version: &release.version,
                target_version,
                update_available: target_version.is_some(),
                reason,
                target,
                archive_url: &urls.archive_url,
            },
        )?;
    } else if !options.quiet {
        let status = match target_version {
            Some(version) => format!("update available: {current_version} -> {version}"),
            None => format!("up to date: {current_version}"),
        };
        writeln!(out, "current: {current_version}").map_err(write_error_io)?;
        writeln!(out, "latest:  {}", release.version).map_err(write_error_io)?;
        writeln!(out, "target:  {target}").map_err(write_error_io)?;
        writeln!(out, "{status}").map_err(write_error_io)?;
    }
    Ok(())
}

fn verify_sha256(bytes: &[u8], checksum_text: &str) -> Result<String, SelfUpdateError> {
    let expected = checksum_text
        .split_whitespace()
        .find(|part| part.len() == 64 && part.chars().all(|ch| ch.is_ascii_hexdigit()))
        .ok_or_else(|| {
            SelfUpdateError::runtime("sha256 sidecar did not contain a 64-character hex digest")
        })?
        .to_ascii_lowercase();
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != expected {
        return Err(SelfUpdateError::runtime(format!(
            "sha256 verification failed: expected {expected}, got {actual}"
        )));
    }
    Ok(actual)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    TarGz,
    Zip,
}

fn archive_kind_for_target(target: &str) -> ArchiveKind {
    if target.contains("windows") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

#[cfg(not(target_os = "linux"))]
fn extract_binary_to_path(
    archive_bytes: &[u8],
    kind: ArchiveKind,
    destination: &Path,
) -> Result<(), SelfUpdateError> {
    let binary_name = binary_name();
    let bytes = match kind {
        ArchiveKind::TarGz => extract_from_targz(archive_bytes, binary_name)?,
        ArchiveKind::Zip => extract_from_zip(archive_bytes, binary_name)?,
    };
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            SelfUpdateError::runtime(format!(
                "failed to create update temp directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(destination, bytes).map_err(|error| {
        SelfUpdateError::runtime(format!(
            "failed to write extracted binary to {}: {error}",
            destination.display()
        ))
    })?;
    Ok(())
}

fn extract_from_targz(archive_bytes: &[u8], binary_name: &str) -> Result<Vec<u8>, SelfUpdateError> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(archive_bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|error| {
        SelfUpdateError::runtime(format!("failed to read tar.gz archive: {error}"))
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| {
            SelfUpdateError::runtime(format!("failed to read tar.gz archive entry: {error}"))
        })?;
        let path = entry.path().map_err(|error| {
            SelfUpdateError::runtime(format!("failed to read tar.gz archive path: {error}"))
        })?;
        if path.file_name().and_then(|name| name.to_str()) == Some(binary_name) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|error| {
                SelfUpdateError::runtime(format!(
                    "failed to extract binary from tar.gz archive: {error}"
                ))
            })?;
            return Ok(bytes);
        }
    }
    Err(SelfUpdateError::runtime(format!(
        "release archive did not contain {binary_name}"
    )))
}

fn extract_from_zip(archive_bytes: &[u8], binary_name: &str) -> Result<Vec<u8>, SelfUpdateError> {
    let reader = Cursor::new(archive_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| {
        SelfUpdateError::runtime(format!("failed to read zip archive: {error}"))
    })?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| {
            SelfUpdateError::runtime(format!("failed to read zip archive entry: {error}"))
        })?;
        let Some(path) = file.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        if path.file_name().and_then(|name| name.to_str()) == Some(binary_name) {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|error| {
                SelfUpdateError::runtime(format!(
                    "failed to extract binary from zip archive: {error}"
                ))
            })?;
            return Ok(bytes);
        }
    }
    Err(SelfUpdateError::runtime(format!(
        "release archive did not contain {binary_name}"
    )))
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "refact.exe"
    } else {
        "refact"
    }
}

#[cfg(not(target_os = "linux"))]
fn temp_binary_path(exe_path: &Path) -> PathBuf {
    let file_name = exe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("refact");
    exe_path.with_file_name(format!(".{file_name}.update-{}", std::process::id()))
}

fn replace_binary(current_path: &Path, replacement_path: &Path) -> Result<(), SelfUpdateError> {
    ensure_replaceable(current_path)?;
    preserve_binary_permissions(current_path, replacement_path)?;
    replace_binary_inner(current_path, replacement_path)
}

#[cfg(unix)]
fn preserve_binary_permissions(
    _current_path: &Path,
    replacement_path: &Path,
) -> Result<(), SelfUpdateError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(replacement_path, std::fs::Permissions::from_mode(0o755)).map_err(
        |error| {
            SelfUpdateError::runtime(format!(
                "failed to set executable permissions on {}: {error}",
                replacement_path.display()
            ))
        },
    )?;
    Ok(())
}

#[cfg(not(unix))]
fn preserve_binary_permissions(
    current_path: &Path,
    replacement_path: &Path,
) -> Result<(), SelfUpdateError> {
    let permissions = std::fs::metadata(current_path)
        .map_err(|error| {
            SelfUpdateError::runtime(format!(
                "failed to inspect current binary {}: {error}",
                current_path.display()
            ))
        })?
        .permissions();
    std::fs::set_permissions(replacement_path, permissions).map_err(|error| {
        SelfUpdateError::runtime(format!(
            "failed to preserve permissions on {}: {error}",
            replacement_path.display()
        ))
    })?;
    Ok(())
}

fn ensure_replaceable(current_path: &Path) -> Result<(), SelfUpdateError> {
    let metadata = std::fs::metadata(current_path).map_err(|error| {
        SelfUpdateError::runtime(format!(
            "failed to inspect current binary {}: {error}",
            current_path.display()
        ))
    })?;
    if metadata.permissions().readonly() {
        return Err(not_writable_error(current_path, "the file is read-only"));
    }
    let parent = current_path.parent().ok_or_else(|| {
        SelfUpdateError::runtime(format!(
            "cannot determine parent directory for {}",
            current_path.display()
        ))
    })?;
    let probe = parent.join(format!(".refact-update-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(error) => Err(not_writable_error(
            current_path,
            format!("cannot write to {}: {error}", parent.display()),
        )),
    }
}

#[cfg(not(windows))]
fn replace_binary_inner(
    current_path: &Path,
    replacement_path: &Path,
) -> Result<(), SelfUpdateError> {
    std::fs::rename(replacement_path, current_path).map_err(|error| {
        SelfUpdateError::runtime(format!(
            "failed to replace {} atomically: {error}. Try re-running the installer or rerun with sudo if the install directory is protected.",
            current_path.display()
        ))
    })
}

#[cfg(windows)]
fn replace_binary_inner(
    current_path: &Path,
    replacement_path: &Path,
) -> Result<(), SelfUpdateError> {
    let old_path = current_path.with_file_name(format!(
        "{}.old",
        current_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("refact.exe")
    ));
    let _ = std::fs::remove_file(&old_path);
    std::fs::rename(current_path, &old_path).map_err(|error| {
        SelfUpdateError::runtime(format!(
            "failed to move running binary {} to {}: {error}. Close running refact processes, rerun the installer, or run from an elevated shell if the install directory is protected.",
            current_path.display(),
            old_path.display()
        ))
    })?;
    if let Err(error) = std::fs::rename(replacement_path, current_path) {
        let rollback_error = std::fs::rename(&old_path, current_path).err();
        return Err(windows_install_failure_error(
            current_path,
            replacement_path,
            &old_path,
            &error,
            rollback_error.as_ref(),
        ));
    }
    let _ = std::fs::remove_file(&old_path);
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_install_failure_error(
    current_path: &Path,
    replacement_path: &Path,
    old_path: &Path,
    install_error: &std::io::Error,
    rollback_error: Option<&std::io::Error>,
) -> SelfUpdateError {
    match rollback_error {
        Some(rollback_error) => SelfUpdateError::runtime(format!(
            "failed to install updated binary {} from {}: {install_error}. Rollback also failed while moving {} back to {}: {rollback_error}. The refact binary may be in a broken state. Recover manually by moving {} back to {}, then rerun the installer or use the temp binary at {}.",
            current_path.display(),
            replacement_path.display(),
            old_path.display(),
            current_path.display(),
            old_path.display(),
            current_path.display(),
            replacement_path.display()
        )),
        None => SelfUpdateError::runtime(format!(
            "failed to install updated binary {} from {}: {install_error}. The previous binary was restored.",
            current_path.display(),
            replacement_path.display()
        )),
    }
}

fn not_writable_error(path: &Path, reason: impl Into<String>) -> SelfUpdateError {
    SelfUpdateError::runtime(format!(
        "cannot update {} because it is not writable: {}. Try re-running the installer or rerun with sudo if the install directory is protected.",
        path.display(),
        reason.into()
    ))
}

fn print_json(out: &mut dyn Write, value: &impl Serialize) -> Result<(), SelfUpdateError> {
    serde_json::to_writer(&mut *out, value)
        .map_err(|error| SelfUpdateError::runtime(format!("failed to write JSON: {error}")))?;
    writeln!(out).map_err(write_error_io)
}

fn write_error(
    error: SelfUpdateError,
    json_output: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    if json_output {
        let _ = print_json(
            stdout,
            &json!({"ok": false, "error": error.message, "exit_code": error.exit_code}),
        );
    } else {
        let _ = writeln!(stderr, "{}", error.message);
    }
    error.exit_code
}

fn write_error_io(error: std::io::Error) -> SelfUpdateError {
    SelfUpdateError::runtime(format!("failed to write output: {error}"))
}

fn os_to_string(value: &OsString) -> Result<String, String> {
    value
        .to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| "arguments must be valid UTF-8".to_string())
}

fn truncate_error_body(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() > 500 {
        format!("{}…", body.chars().take(500).collect::<String>())
    } else if body.is_empty() {
        "<empty response>".to_string()
    } else {
        body.to_string()
    }
}

fn restart_hint() -> &'static str {
    "Restart any running daemon with `refact restart --daemon`."
}

/// Candidate installation is a separate, offline native owner. No release lookup,
/// caller-selected command, or version string is used as executable evidence.
#[cfg(target_os = "linux")]
pub mod candidate_install {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    const LIMIT: u64 = 256 * 1024 * 1024;
    // Keep the original marker name so interrupted candidate transactions also
    // block ordinary self-update after upgrading. Never auto-delete stale claims.
    const INSTALL_LOCK: &str = ".refact-candidate-install-lock";
    const O_NOFOLLOW: i32 = 0x20000;
    const O_DIRECTORY: i32 = 0x10000;
    const O_NONBLOCK: i32 = 0x800;
    pub struct Installer {
        directory: std::fs::File,
        executable: PathBuf,
        name: OsString,
    }
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    pub struct ImageReceipt {
        pub before: String,
        pub installed: String,
        pub operation: String,
    }
    fn hash(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }
    fn digest(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    fn read(path: &Path) -> Result<Vec<u8>, String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(path)
            .map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        let uid = std::fs::metadata("/proc/self")
            .map_err(|e| e.to_string())?
            .uid();
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.len() > LIMIT
            || meta.uid() != uid
            || meta.mode() & 0o022 != 0
        {
            return Err("native image owner/mode/type/link/size rejected".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        let named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != meta.len()
            || (meta.dev(), meta.ino()) != (named.dev(), named.ino())
            || (
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ctime(),
                meta.ctime_nsec(),
            ) != (
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            )
        {
            return Err("native image changed while reading".into());
        }
        Ok(bytes)
    }
    impl Installer {
        /// Only the daemon passes its captured native relaunch path.
        /// This is not an HTTP parameter. A directory handle pins all mutations.
        /// Ownership/mode checks exclude other-user writes, not hostile same-UID
        /// processes: those can alter files or remove a cooperative lock. Path
        /// revalidation detects observed moves, but is not an atomic namespace
        /// guarantee against such processes or an attacker controlling ancestors.
        pub fn open(executable: &Path) -> Result<Self, String> {
            if !executable.is_absolute()
                || executable.canonicalize().map_err(|e| e.to_string())? != executable
            {
                return Err("canonical installed executable required".into());
            }
            let parent = executable.parent().ok_or("image parent missing")?;
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(O_DIRECTORY | O_NOFOLLOW)
                .open(parent)
                .map_err(|e| e.to_string())?;
            let owner = Self {
                directory,
                executable: executable.to_owned(),
                name: executable
                    .file_name()
                    .ok_or("image name missing")?
                    .to_owned(),
            };
            owner.validate_path()?;
            read(&owner.path(&owner.name))?;
            Ok(owner)
        }
        fn validate_path(&self) -> Result<(), String> {
            let meta = self.directory.metadata().map_err(|e| e.to_string())?;
            let uid = std::fs::metadata("/proc/self")
                .map_err(|e| e.to_string())?
                .uid();
            let parent = self.executable.parent().ok_or("image parent missing")?;
            let named = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
            if !meta.is_dir()
                || meta.uid() != uid
                || meta.mode() & 0o022 != 0
                || (meta.dev(), meta.ino()) != (named.dev(), named.ino())
                || self.executable.canonicalize().map_err(|e| e.to_string())? != self.executable
            {
                return Err("installer requires an unchanged user-owned non-shared canonical install directory".into());
            }
            Ok(())
        }
        fn claim(&self, operation: &str) -> Result<std::fs::File, String> {
            self.validate_path()?;
            let mut claim = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.path(INSTALL_LOCK))
                .map_err(|e| format!("installer busy or interrupted: {e}"))?;
            claim
                .write_all(operation.as_bytes())
                .map_err(|e| e.to_string())?;
            claim.sync_all().map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
            Ok(claim)
        }
        fn complete(&self, claim: std::fs::File) -> Result<(), String> {
            self.validate_path()?;
            let held = claim.metadata().map_err(|e| e.to_string())?;
            let named =
                std::fs::symlink_metadata(self.path(INSTALL_LOCK)).map_err(|e| e.to_string())?;
            if (held.dev(), held.ino()) != (named.dev(), named.ino()) {
                return Err("installation lock changed; inspect interrupted transaction".into());
            }
            std::fs::remove_file(self.path(INSTALL_LOCK)).map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())
        }
        pub(super) fn install_release_bytes(&self, bytes: &[u8]) -> Result<(), String> {
            if bytes.len() as u64 > LIMIT {
                return Err("native image size rejected".into());
            }
            let before = self.installed_digest()?;
            let after = hash(bytes);
            if before == after {
                // Even an identical release must not bypass an interrupted claim.
                let claim = self.claim("release-identical")?;
                if self.installed_digest()? != before {
                    return Err("installed preimage changed".into());
                }
                return self.complete(claim);
            }
            let operation = hash(format!("release:{before}:{after}").as_bytes());
            self.install(bytes, &before, &after, &operation).map(|_| ())
        }
        fn path(&self, name: impl AsRef<std::ffi::OsStr>) -> PathBuf {
            PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
                .join(name.as_ref())
        }
        pub fn installed_digest(&self) -> Result<String, String> {
            self.validate_path()?;
            Ok(hash(&read(&self.path(&self.name))?))
        }
        /// Known-good bytes are fsynced before replacement. An interrupted lock
        /// is deliberately not replayed or removed by a subsequent request.
        pub fn install(
            &self,
            bytes: &[u8],
            before: &str,
            after: &str,
            operation: &str,
        ) -> Result<ImageReceipt, String> {
            if !digest(before)
                || !digest(after)
                || !digest(operation)
                || before == after
                || bytes.len() as u64 > LIMIT
                || hash(bytes) != after
            {
                return Err("exact image binding rejected".into());
            }
            let old = read(&self.path(&self.name))?;
            if hash(&old) != before {
                return Err("installed preimage changed".into());
            }
            let claim = self.claim(operation)?;
            if self.installed_digest()? != before {
                return Err("installed preimage changed under lock".into());
            }
            let backup = self.path(format!(".refact-known-good-{before}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&backup)
            {
                Ok(mut file) => {
                    file.write_all(&old).map_err(|e| e.to_string())?;
                    file.sync_all().map_err(|e| e.to_string())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if read(&backup)? != old {
                        return Err("known-good material changed".into());
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
            let temporary = self.path(format!(".refact-candidate-{operation}"));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
            if self.installed_digest()? != before || hash(&read(&temporary)?) != after {
                return Err("native image CAS failed; inspect interrupted transaction".into());
            }
            replace_binary(&self.path(&self.name), &temporary).map_err(|e| e.message)?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
            if self.installed_digest()? != after {
                return Err("installed image observation differs".into());
            }
            self.complete(claim)?;
            Ok(ImageReceipt {
                before: before.into(),
                installed: after.into(),
                operation: operation.into(),
            })
        }
        pub fn recover(
            &self,
            receipt: &ImageReceipt,
            operation: &str,
        ) -> Result<ImageReceipt, String> {
            if !digest(&receipt.before) || !digest(&receipt.installed) {
                return Err("invalid recovery identity".into());
            }
            let bytes = read(&self.path(format!(".refact-known-good-{}", receipt.before)))?;
            self.install(&bytes, &receipt.installed, &receipt.before, operation)
        }
    }
    /// Kernel executable handle, not the (possibly already replaced) install path.
    pub fn running_digest() -> Result<String, String> {
        let mut file = std::fs::File::open("/proc/self/exe").map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn scratch_install() -> (tempfile::TempDir, PathBuf) {
            // tempfile and std::fs::write inherit the process umask; 0002 would
            // otherwise create a shared directory (0775) and image (0664).
            let dir = tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()
                .unwrap();
            let path = dir.path().canonicalize().unwrap().join("refact");
            let mut image = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            image.write_all(b"old").unwrap();
            (dir, path)
        }

        #[tokio::test]
        async fn scratch_ordinary_release_honors_native_claim() {
            struct OfflineArchive(Vec<u8>);
            impl ReleaseSource for OfflineArchive {
                fn latest_release<'a>(&'a self) -> ReleaseFuture<'a, EngineReleaseInfo> {
                    Box::pin(async { Err(SelfUpdateError::runtime("unexpected lookup")) })
                }
                fn release_for_version<'a>(
                    &'a self,
                    _: &'a str,
                ) -> ReleaseFuture<'a, EngineReleaseInfo> {
                    self.latest_release()
                }
                fn download<'a>(
                    &'a self,
                    _: &'a str,
                    label: &'static str,
                ) -> ReleaseFuture<'a, Vec<u8>> {
                    Box::pin(async move {
                        Ok(if label == "release archive" {
                            self.0.clone()
                        } else {
                            hash(&self.0).into_bytes()
                        })
                    })
                }
            }
            let mut archive = Vec::new();
            {
                let encoder =
                    flate2::write::GzEncoder::new(&mut archive, flate2::Compression::default());
                let mut builder = tar::Builder::new(encoder);
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o755);
                header.set_cksum();
                builder
                    .append_data(&mut header, binary_name(), Cursor::new(b"new"))
                    .unwrap();
                builder.into_inner().unwrap().finish().unwrap();
            }
            let source = OfflineArchive(archive);
            let (_dir, path) = scratch_install();
            let owner = Installer::open(&path).unwrap();
            let claim = owner.claim("scratch").unwrap();
            let target = "x86_64-unknown-linux-gnu";
            let urls = asset_urls_for_version("99.0.0", target);
            let error = install_release(&source, target, &urls, Some(path.clone()))
                .await
                .unwrap_err();
            assert!(error.message.contains("busy or interrupted"));
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            owner.complete(claim).unwrap();
            install_release(&source, target, &urls, Some(path.clone()))
                .await
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
        }
        #[test]
        fn scratch_shared_claim_blocks_release_and_candidate_without_mutation() {
            let (dir, path) = scratch_install();
            let candidate = Installer::open(&path).unwrap();
            let release = Installer::open(&path).unwrap();
            let claim = candidate.claim(&hash(b"claim")).unwrap();
            assert!(release
                .install_release_bytes(b"new")
                .unwrap_err()
                .contains("busy or interrupted"));
            assert!(release.install_release_bytes(b"old").is_err());
            assert!(release
                .install(b"new", &hash(b"old"), &hash(b"new"), &hash(b"op"))
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            candidate.complete(claim).unwrap();
            release.install_release_bytes(b"new").unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
            assert!(!dir.path().join(INSTALL_LOCK).exists());
            release.install_release_bytes(b"new").unwrap();
        }
        #[test]
        fn scratch_rejects_shared_linked_symlink_and_moved_install_paths() {
            use std::os::unix::fs::{symlink, PermissionsExt};
            let dir = tempfile::tempdir().unwrap();
            let install = dir.path().join("install");
            std::fs::create_dir(&install).unwrap();
            std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o700)).unwrap();
            let path = install.join("refact");
            std::fs::write(&path, b"old").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            // Prove the baseline is valid before introducing each unsafe path.
            assert!(Installer::open(&path).is_ok());
            let alias = install.join("alias");
            symlink(&path, &alias).unwrap();
            assert!(Installer::open(&alias).is_err());
            std::fs::remove_file(&alias).unwrap();
            std::fs::hard_link(&path, &alias).unwrap();
            assert!(Installer::open(&path).is_err());
            std::fs::remove_file(&alias).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert!(Installer::open(&path).is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert!(Installer::open(&path).is_err());
            std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o700)).unwrap();
            let owner = Installer::open(&path).unwrap();
            let moved = dir.path().join("moved");
            std::fs::rename(&install, &moved).unwrap();
            std::fs::create_dir(&install).unwrap();
            std::fs::write(&path, b"decoy").unwrap();
            assert!(owner.installed_digest().is_err());
            assert!(owner.install_release_bytes(b"new").is_err());
            assert_eq!(std::fs::read(moved.join("refact")).unwrap(), b"old");
            assert_eq!(std::fs::read(&path).unwrap(), b"decoy");
        }
        #[test]
        fn scratch_replaced_claim_is_not_removed() {
            let (dir, path) = scratch_install();
            let owner = Installer::open(&path).unwrap();
            let claim = owner.claim("scratch").unwrap();
            let lock = dir.path().join(INSTALL_LOCK);
            std::fs::remove_file(&lock).unwrap();
            std::fs::write(&lock, b"different owner").unwrap();
            assert!(owner.complete(claim).is_err());
            assert_eq!(std::fs::read(&lock).unwrap(), b"different owner");
        }
        #[test]
        fn scratch_installer_exact_cas_retained_recovery_and_interrupted_lock() {
            let (dir, path) = scratch_install();
            let owner = Installer::open(&path).unwrap();
            assert!(owner
                .install(b"new", &hash(b"wrong"), &hash(b"new"), &hash(b"op"))
                .is_err());
            let receipt = owner
                .install(b"new", &hash(b"old"), &hash(b"new"), &hash(b"op"))
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
            assert!(owner
                .install(b"new", &hash(b"old"), &hash(b"new"), &hash(b"op"))
                .is_err());
            owner.recover(&receipt, &hash(b"recovery")).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            std::fs::write(
                dir.path().join(".refact-candidate-install-lock"),
                b"interrupted",
            )
            .unwrap();
            assert!(owner
                .install(b"new", &hash(b"old"), &hash(b"new"), &hash(b"another"))
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_decision_updates_only_when_needed_or_forced() {
        assert_eq!(
            decide_update("1.0.0", "1.1.0", false, None),
            UpdateDecision::Update {
                version: "1.1.0".to_string(),
                reason: UpdateReason::NewerVersion
            }
        );
        assert_eq!(
            decide_update("1.1.0", "1.1.0", false, None),
            UpdateDecision::NoUpdate
        );
        assert_eq!(
            decide_update("1.1.0", "1.1.0", true, None),
            UpdateDecision::Update {
                version: "1.1.0".to_string(),
                reason: UpdateReason::Force
            }
        );
        assert_eq!(
            decide_update("2.0.0", "1.9.0", false, Some("v1.9.0")),
            UpdateDecision::Update {
                version: "1.9.0".to_string(),
                reason: UpdateReason::ExplicitVersion
            }
        );
    }

    #[test]
    fn sha256_sidecar_supports_sha256sum_format() {
        let bytes = b"hello";
        let digest = hex::encode(Sha256::digest(bytes));
        assert_eq!(
            verify_sha256(bytes, &format!("{digest}  refact-8.2.0-target.tar.gz\n")).unwrap(),
            digest
        );
        let error = verify_sha256(bytes, "0".repeat(64).as_str()).unwrap_err();
        assert!(error.message.contains("sha256 verification failed"));
    }

    #[test]
    fn extracts_binary_from_tar_gz_archive() {
        let mut tar_bytes = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, binary_name(), Cursor::new(b"bin"))
                .unwrap();
            builder.finish().unwrap();
        }
        assert_eq!(
            extract_from_targz(&tar_bytes, binary_name()).unwrap(),
            b"bin"
        );
    }

    #[test]
    fn atomic_replace_installs_replacement_and_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join(binary_name());
        let replacement = dir.path().join("replacement");
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(&replacement, b"new").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        replace_binary(&current, &replacement).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert!(!replacement.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&current).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn readonly_binary_refuses_update() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join(binary_name());
        std::fs::write(&current, b"old").unwrap();
        let mut permissions = std::fs::metadata(&current).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&current, permissions).unwrap();
        let error = ensure_replaceable(&current).unwrap_err();
        assert!(error.message.contains("not writable"));
        let mut permissions = std::fs::metadata(&current).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&current, permissions).unwrap();
    }
}
