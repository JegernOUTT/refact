use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use refact_chat_api::WindowBounds;

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

pub const TRANSPORT_READ_TIMEOUT: Duration = Duration::from_secs(86_400);

pub const MANDATORY_CHROME_ARGS: [&str; 7] = [
    "--no-restore-last-session",
    "--no-first-run",
    "--no-startup-window",
    "--disable-blink-features=AutomationControlled",
    "--no-default-browser-check",
    "--disable-search-engine-choice-screen",
    "--disable-back-forward-cache",
];

pub const IGNORED_CHROME_DEFAULT_ARGS: [&str; 1] = ["--enable-automation"];

pub const SOFTWARE_GPU_CHROME_ARGS: [&str; 3] = [
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
];

pub const HARDWARE_GPU_CHROME_ARGS: [&str; 1] = ["--ignore-gpu-blocklist"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserGpuMode {
    #[default]
    Disabled,
    Software,
    Hardware,
}

impl BrowserGpuMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "disabled" | "off" | "false" | "none" => Ok(Self::Disabled),
            "software" | "swiftshader" | "angle" => Ok(Self::Software),
            "hardware" | "on" | "true" | "gpu" => Ok(Self::Hardware),
            other => Err(format!(
                "unsupported gpu mode `{other}`; expected disabled, software or hardware"
            )),
        }
    }

    pub fn enables_gpu(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Software => "software",
            Self::Hardware => "hardware",
        }
    }

    fn chrome_args(self) -> &'static [&'static str] {
        match self {
            Self::Disabled => &[],
            Self::Software => &SOFTWARE_GPU_CHROME_ARGS,
            Self::Hardware => &HARDWARE_GPU_CHROME_ARGS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProxyOptions {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserLaunchOptions {
    pub headless: bool,
    pub window_bounds: Option<WindowBounds>,
    pub chrome_path: Option<PathBuf>,
    pub idle_timeout: Option<Duration>,
    pub mask_passwords: bool,
    pub extra_args: Vec<String>,
    pub chromium_sandbox: bool,
    pub proxy: Option<BrowserProxyOptions>,
    pub downloads_dir: Option<PathBuf>,
    pub ignore_https_errors: bool,
    pub evict_idle_attached: bool,
    pub gpu: BrowserGpuMode,
}

impl Default for BrowserLaunchOptions {
    fn default() -> Self {
        Self {
            headless: false,
            window_bounds: None,
            chrome_path: None,
            idle_timeout: None,
            mask_passwords: true,
            extra_args: Vec::new(),
            chromium_sandbox: true,
            proxy: None,
            downloads_dir: None,
            ignore_https_errors: true,
            evict_idle_attached: false,
            gpu: BrowserGpuMode::Disabled,
        }
    }
}

impl BrowserLaunchOptions {
    pub fn headless(headless: bool) -> Self {
        Self {
            headless,
            ..Self::default()
        }
    }

    pub fn idle_timeout_or_default(&self) -> Duration {
        self.idle_timeout.unwrap_or(DEFAULT_IDLE_TIMEOUT)
    }

    pub fn transport_read_timeout(&self) -> Duration {
        TRANSPORT_READ_TIMEOUT
    }

    pub fn window_size(&self) -> Option<(u32, u32)> {
        self.window_bounds
            .as_ref()
            .map(|bounds| (bounds.width, bounds.height))
    }

    pub fn chrome_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = MANDATORY_CHROME_ARGS.iter().map(OsString::from).collect();
        if let Some(bounds) = &self.window_bounds {
            args.push(OsString::from(format!(
                "--window-position={},{}",
                bounds.x, bounds.y
            )));
        }
        if let Some(bypass) = self.proxy.as_ref().and_then(|proxy| proxy.bypass.as_ref()) {
            args.push(OsString::from(format!("--proxy-bypass-list={bypass}")));
        }
        args.extend(self.gpu.chrome_args().iter().map(OsString::from));
        args.extend(self.extra_args.iter().map(OsString::from));
        args
    }

    pub fn ignored_default_args(&self) -> Vec<OsString> {
        IGNORED_CHROME_DEFAULT_ARGS
            .iter()
            .map(OsString::from)
            .collect()
    }

    pub fn mode_label(&self) -> &'static str {
        if self.headless {
            "headless"
        } else {
            "headed"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(options: &BrowserLaunchOptions) -> Vec<String> {
        options
            .chrome_args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn defaults_preserve_todays_launch_behaviour() {
        let options = BrowserLaunchOptions::default();

        assert!(options.chromium_sandbox);
        assert!(options.ignore_https_errors);
        assert!(options.mask_passwords);
        assert!(!options.headless);
        assert!(options.proxy.is_none());
        assert!(options.extra_args.is_empty());
        assert!(options.downloads_dir.is_none());
        assert_eq!(options.idle_timeout_or_default(), DEFAULT_IDLE_TIMEOUT);
        assert_eq!(options.transport_read_timeout(), TRANSPORT_READ_TIMEOUT);
        assert_eq!(options.window_size(), None);
        assert_eq!(args_of(&options), MANDATORY_CHROME_ARGS.to_vec());
    }
}
