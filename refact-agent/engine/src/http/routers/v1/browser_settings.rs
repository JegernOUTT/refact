use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

pub const BROWSER_SETTINGS_FILE: &str = "browser.yaml";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserLaunchSettings {
    pub chrome_path: String,
    pub headless: bool,
    pub chromium_sandbox: bool,
    pub ignore_https_errors: bool,
    pub mask_passwords: bool,
    pub extra_args: Vec<String>,
    pub downloads_dir: String,
    pub proxy_server: String,
    pub proxy_bypass: String,
}

impl Default for BrowserLaunchSettings {
    fn default() -> Self {
        let defaults = refact_browser::BrowserLaunchOptions::default();
        Self {
            chrome_path: String::new(),
            headless: defaults.headless,
            chromium_sandbox: defaults.chromium_sandbox,
            ignore_https_errors: defaults.ignore_https_errors,
            mask_passwords: defaults.mask_passwords,
            extra_args: Vec::new(),
            downloads_dir: String::new(),
            proxy_server: String::new(),
            proxy_bypass: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserLifecycleSettings {
    pub idle_timeout_secs: u64,
    pub evict_idle_attached: bool,
    pub monitor_interval_secs: u64,
    pub relaunch_settle_ms: u64,
}

impl Default for BrowserLifecycleSettings {
    fn default() -> Self {
        Self {
            idle_timeout_secs: refact_browser::DEFAULT_IDLE_TIMEOUT.as_secs(),
            evict_idle_attached: false,
            monitor_interval_secs: 10,
            relaunch_settle_ms: 800,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserViewportSettings {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

impl Default for BrowserViewportSettings {
    fn default() -> Self {
        Self {
            width: 1440,
            height: 900,
            scale_factor: 2.0,
        }
    }
}

impl BrowserViewportSettings {
    fn mobile_default() -> Self {
        Self {
            width: 390,
            height: 844,
            scale_factor: 3.0,
        }
    }

    fn tablet_default() -> Self {
        Self {
            width: 834,
            height: 1112,
            scale_factor: 2.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserViewportGroup {
    pub desktop: BrowserViewportSettings,
    pub mobile: BrowserViewportSettings,
    pub tablet: BrowserViewportSettings,
}

impl Default for BrowserViewportGroup {
    fn default() -> Self {
        Self {
            desktop: BrowserViewportSettings::default(),
            mobile: BrowserViewportSettings::mobile_default(),
            tablet: BrowserViewportSettings::tablet_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserTimingSettings {
    pub default_wait_timeout_ms: u64,
    pub max_wait_timeout_ms: u64,
    pub default_poll_interval_ms: u64,
}

impl Default for BrowserTimingSettings {
    fn default() -> Self {
        Self {
            default_wait_timeout_ms: 5_000,
            max_wait_timeout_ms: 60_000,
            default_poll_interval_ms: 200,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserCaptureSettings {
    pub default_aria_snapshot_chars: usize,
    pub max_aria_snapshot_chars: usize,
    pub max_dom_snapshot_chars: usize,
    pub max_inline_snapshot_bytes: usize,
    pub max_extract_links: usize,
    pub max_extract_table_rows: usize,
    pub default_all_texts: usize,
}

impl Default for BrowserCaptureSettings {
    fn default() -> Self {
        Self {
            default_aria_snapshot_chars: 20_000,
            max_aria_snapshot_chars: 100_000,
            max_dom_snapshot_chars: 100_000,
            max_inline_snapshot_bytes: 6 * 1024,
            max_extract_links: 500,
            max_extract_table_rows: 100,
            default_all_texts: 50,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserSettings {
    pub launch: BrowserLaunchSettings,
    pub lifecycle: BrowserLifecycleSettings,
    pub viewport: BrowserViewportGroup,
    pub timing: BrowserTimingSettings,
    pub capture: BrowserCaptureSettings,
}

impl BrowserSettings {
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.lifecycle.idle_timeout_secs)
    }

    pub fn monitor_interval(&self) -> Duration {
        Duration::from_secs(self.lifecycle.monitor_interval_secs)
    }

    pub fn relaunch_settle(&self) -> Duration {
        Duration::from_millis(self.lifecycle.relaunch_settle_ms)
    }

    pub fn to_launch_options(&self) -> refact_browser::BrowserLaunchOptions {
        let launch = &self.launch;
        refact_browser::BrowserLaunchOptions {
            headless: launch.headless,
            chrome_path: (!launch.chrome_path.is_empty())
                .then(|| PathBuf::from(launch.chrome_path.clone())),
            idle_timeout: Some(self.idle_timeout()),
            mask_passwords: launch.mask_passwords,
            extra_args: launch.extra_args.clone(),
            chromium_sandbox: launch.chromium_sandbox,
            proxy: (!launch.proxy_server.is_empty()).then(|| {
                refact_browser::BrowserProxyOptions {
                    server: launch.proxy_server.clone(),
                    bypass: (!launch.proxy_bypass.is_empty())
                        .then(|| launch.proxy_bypass.clone()),
                }
            }),
            downloads_dir: (!launch.downloads_dir.is_empty())
                .then(|| PathBuf::from(launch.downloads_dir.clone())),
            ignore_https_errors: launch.ignore_https_errors,
            evict_idle_attached: self.lifecycle.evict_idle_attached,
            ..Default::default()
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.lifecycle.idle_timeout_secs == 0 {
            return Err("lifecycle.idle_timeout_secs must be at least 1".to_string());
        }
        if self.lifecycle.monitor_interval_secs == 0 {
            return Err("lifecycle.monitor_interval_secs must be at least 1".to_string());
        }
        for (label, viewport) in [
            ("desktop", &self.viewport.desktop),
            ("mobile", &self.viewport.mobile),
            ("tablet", &self.viewport.tablet),
        ] {
            if viewport.width == 0 || viewport.height == 0 {
                return Err(format!("viewport.{label} width and height must be at least 1"));
            }
            if !(0.1..=8.0).contains(&viewport.scale_factor) {
                return Err(format!(
                    "viewport.{label}.scale_factor must be between 0.1 and 8.0"
                ));
            }
        }
        if self.timing.default_wait_timeout_ms == 0 || self.timing.max_wait_timeout_ms == 0 {
            return Err("timing values must be at least 1ms".to_string());
        }
        if self.timing.default_wait_timeout_ms > self.timing.max_wait_timeout_ms {
            return Err(
                "timing.default_wait_timeout_ms cannot exceed timing.max_wait_timeout_ms"
                    .to_string(),
            );
        }
        if self.timing.default_poll_interval_ms == 0 {
            return Err("timing.default_poll_interval_ms must be at least 1".to_string());
        }
        if self.capture.default_aria_snapshot_chars > self.capture.max_aria_snapshot_chars {
            return Err(
                "capture.default_aria_snapshot_chars cannot exceed capture.max_aria_snapshot_chars"
                    .to_string(),
            );
        }
        let zero_capture = [
            ("default_aria_snapshot_chars", self.capture.default_aria_snapshot_chars),
            ("max_aria_snapshot_chars", self.capture.max_aria_snapshot_chars),
            ("max_dom_snapshot_chars", self.capture.max_dom_snapshot_chars),
            ("max_inline_snapshot_bytes", self.capture.max_inline_snapshot_bytes),
            ("max_extract_links", self.capture.max_extract_links),
            ("max_extract_table_rows", self.capture.max_extract_table_rows),
            ("default_all_texts", self.capture.default_all_texts),
        ];
        for (label, value) in zero_capture {
            if value == 0 {
                return Err(format!("capture.{label} must be at least 1"));
            }
        }
        Ok(())
    }
}

static CURRENT: OnceLock<RwLock<Arc<BrowserSettings>>> = OnceLock::new();

fn cell() -> &'static RwLock<Arc<BrowserSettings>> {
    CURRENT.get_or_init(|| RwLock::new(Arc::new(BrowserSettings::default())))
}

pub fn current() -> Arc<BrowserSettings> {
    cell()
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| Arc::new(BrowserSettings::default()))
}

fn publish(settings: BrowserSettings) {
    if let Ok(mut guard) = cell().write() {
        *guard = Arc::new(settings);
    }
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(BROWSER_SETTINGS_FILE)
}

pub async fn load_from_disk(config_dir: &Path) -> BrowserSettings {
    let path = settings_path(config_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_yaml::from_str::<BrowserSettings>(&raw) {
            Ok(settings) if settings.validate().is_ok() => settings,
            Ok(_) => {
                tracing::warn!(
                    "browser settings in {} failed validation, using defaults",
                    path.display()
                );
                BrowserSettings::default()
            }
            Err(error) => {
                tracing::warn!(
                    "invalid browser settings YAML in {}: {}, using defaults",
                    path.display(),
                    error
                );
                BrowserSettings::default()
            }
        },
        Err(_) => BrowserSettings::default(),
    }
}

pub async fn refresh_from_disk(config_dir: &Path) -> Arc<BrowserSettings> {
    publish(load_from_disk(config_dir).await);
    current()
}

fn bad_request(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::BAD_REQUEST, message.into())
}

fn server_error(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, message.into())
}

async fn atomic_write(path: &Path, content: &str) -> Result<(), ScratchError> {
    let parent = path
        .parent()
        .ok_or_else(|| server_error(format!("{} has no parent directory", path.display())))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| server_error(format!("cannot create {}: {}", parent.display(), e)))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{BROWSER_SETTINGS_FILE}.{nonce}.tmp"));
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| server_error(format!("cannot create {}: {}", tmp.display(), e)))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|e| server_error(format!("cannot write {}: {}", tmp.display(), e)))?;
    file.flush()
        .await
        .map_err(|e| server_error(format!("cannot flush {}: {}", tmp.display(), e)))?;
    drop(file);
    tokio::fs::rename(&tmp, path).await.map_err(|e| {
        server_error(format!(
            "cannot move {} to {}: {}",
            tmp.display(),
            path.display(),
            e
        ))
    })
}

pub async fn handle_v1_browser_settings_get(
    State(app): State<AppState>,
) -> Result<Json<BrowserSettings>, ScratchError> {
    let config_dir = app.gcx.config_dir.clone();
    let settings = refresh_from_disk(&config_dir).await;
    Ok(Json((*settings).clone()))
}

pub async fn handle_v1_browser_settings_post(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Json<BrowserSettings>, ScratchError> {
    let settings = serde_json::from_slice::<BrowserSettings>(&body).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("invalid browser settings payload: {}", e),
        )
    })?;
    settings.validate().map_err(bad_request)?;
    let config_dir = app.gcx.config_dir.clone();
    let path = settings_path(&config_dir);
    let yaml = serde_yaml::to_string(&settings)
        .map_err(|e| server_error(format!("cannot serialize browser settings: {}", e)))?;
    atomic_write(&path, &yaml).await?;
    publish(settings.clone());
    Ok(Json(settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_yaml() {
        let settings = BrowserSettings::default();
        let yaml = serde_yaml::to_string(&settings).unwrap();
        let parsed: BrowserSettings = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn defaults_mirror_the_launch_option_defaults() {
        let settings = BrowserSettings::default();
        let options = settings.to_launch_options();
        let defaults = refact_browser::BrowserLaunchOptions::default();
        assert_eq!(options.headless, defaults.headless);
        assert_eq!(options.chromium_sandbox, defaults.chromium_sandbox);
        assert_eq!(options.ignore_https_errors, defaults.ignore_https_errors);
        assert_eq!(options.mask_passwords, defaults.mask_passwords);
        assert_eq!(options.idle_timeout, Some(refact_browser::DEFAULT_IDLE_TIMEOUT));
    }

    #[test]
    fn attached_runtimes_are_not_idle_evicted_by_default() {
        assert!(!BrowserSettings::default().lifecycle.evict_idle_attached);
        assert!(!refact_browser::BrowserLaunchOptions::default().evict_idle_attached);
    }

    #[test]
    fn proxy_and_paths_are_only_set_when_non_empty() {
        let mut settings = BrowserSettings::default();
        let options = settings.to_launch_options();
        assert!(options.proxy.is_none());
        assert!(options.chrome_path.is_none());
        assert!(options.downloads_dir.is_none());

        settings.launch.proxy_server = "http://127.0.0.1:8080".to_string();
        settings.launch.proxy_bypass = "localhost".to_string();
        settings.launch.chrome_path = "/usr/bin/chromium".to_string();
        let options = settings.to_launch_options();
        let proxy = options.proxy.expect("proxy configured");
        assert_eq!(proxy.server, "http://127.0.0.1:8080");
        assert_eq!(proxy.bypass.as_deref(), Some("localhost"));
        assert_eq!(options.chrome_path, Some(PathBuf::from("/usr/bin/chromium")));
    }

    #[test]
    fn validation_rejects_inconsistent_values() {
        let mut settings = BrowserSettings::default();
        settings.timing.default_wait_timeout_ms = settings.timing.max_wait_timeout_ms + 1;
        assert!(settings.validate().is_err());

        let mut settings = BrowserSettings::default();
        settings.lifecycle.idle_timeout_secs = 0;
        assert!(settings.validate().is_err());

        let mut settings = BrowserSettings::default();
        settings.viewport.mobile.scale_factor = 0.0;
        assert!(settings.validate().is_err());

        let mut settings = BrowserSettings::default();
        settings.capture.default_all_texts = 0;
        assert!(settings.validate().is_err());

        assert!(BrowserSettings::default().validate().is_ok());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let yaml = "launch:\n  chrome_path: ''\n  nonsense: true\n";
        assert!(serde_yaml::from_str::<BrowserSettings>(yaml).is_err());
    }
}
