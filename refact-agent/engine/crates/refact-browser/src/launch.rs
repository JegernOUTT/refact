use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use refact_chat_api::WindowBounds;

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

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
        assert_eq!(options.window_size(), None);
        assert_eq!(args_of(&options), MANDATORY_CHROME_ARGS.to_vec());
    }

    #[test]
    fn extra_args_are_appended_after_the_non_overridable_args() {
        let options = BrowserLaunchOptions {
            extra_args: vec![
                "--lang=de-DE".to_string(),
                "--disable-blink-features=".to_string(),
            ],
            ..BrowserLaunchOptions::default()
        };

        let args = args_of(&options);
        assert_eq!(&args[..MANDATORY_CHROME_ARGS.len()], &MANDATORY_CHROME_ARGS);
        assert_eq!(
            &args[MANDATORY_CHROME_ARGS.len()..],
            &["--lang=de-DE", "--disable-blink-features="]
        );
    }

    #[test]
    fn window_bounds_drive_both_window_size_and_the_position_flag() {
        let options = BrowserLaunchOptions {
            window_bounds: Some(WindowBounds {
                x: 120,
                y: -40,
                width: 1280,
                height: 720,
            }),
            ..BrowserLaunchOptions::default()
        };

        assert_eq!(options.window_size(), Some((1280, 720)));
        assert!(args_of(&options).contains(&"--window-position=120,-40".to_string()));
    }

    #[test]
    fn proxy_bypass_becomes_a_flag_and_ordering_keeps_extra_args_last() {
        let options = BrowserLaunchOptions {
            proxy: Some(BrowserProxyOptions {
                server: "http://proxy.local:3128".to_string(),
                bypass: Some("localhost,127.0.0.1".to_string()),
            }),
            extra_args: vec!["--mute-audio".to_string()],
            ..BrowserLaunchOptions::default()
        };

        let args = args_of(&options);
        let bypass = args
            .iter()
            .position(|arg| arg == "--proxy-bypass-list=localhost,127.0.0.1")
            .expect("bypass flag missing");
        let extra = args
            .iter()
            .position(|arg| arg == "--mute-audio")
            .expect("extra arg missing");
        assert!(bypass > MANDATORY_CHROME_ARGS.len() - 1);
        assert!(extra > bypass);
    }

    #[test]
    fn proxy_without_bypass_emits_no_bypass_flag() {
        let options = BrowserLaunchOptions {
            proxy: Some(BrowserProxyOptions {
                server: "socks5://127.0.0.1:9050".to_string(),
                bypass: None,
            }),
            ..BrowserLaunchOptions::default()
        };

        assert_eq!(args_of(&options), MANDATORY_CHROME_ARGS.to_vec());
    }

    #[test]
    fn automation_banner_default_arg_is_dropped_so_stealth_is_not_self_defeating() {
        let ignored = BrowserLaunchOptions::default()
            .ignored_default_args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(ignored, vec!["--enable-automation".to_string()]);
        assert!(
            MANDATORY_CHROME_ARGS.contains(&"--disable-blink-features=AutomationControlled"),
            "stealth arg must stay mandatory"
        );
    }

    #[test]
    fn mode_label_reports_the_launch_mode() {
        assert_eq!(
            BrowserLaunchOptions::headless(true).mode_label(),
            "headless"
        );
        assert_eq!(BrowserLaunchOptions::headless(false).mode_label(), "headed");
    }
}
