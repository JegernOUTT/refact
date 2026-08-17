use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{Browser, Emulation, Network};
use refact_integrations::browser_models::{
    BrowserContextSummary, BrowserCookie, BrowserCookieSameSite, BrowserStorageItem,
    BrowserStorageKind, BrowserStorageOrigin, BrowserStorageState,
};

#[derive(Clone, Debug, Default)]
pub struct ViewportState {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub is_mobile: bool,
    pub has_touch: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MediaState {
    pub color_scheme: Option<String>,
    pub reduced_motion: Option<String>,
    pub forced_colors: Option<String>,
    pub contrast: Option<String>,
    pub media: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ContextState {
    pub viewport: Option<ViewportState>,
    pub media: MediaState,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub user_agent: Option<(String, Option<String>)>,
    pub geolocation: Option<(f64, f64, f64)>,
    pub offline: bool,
    pub extra_http_headers: BTreeMap<String, String>,
    pub permissions: Vec<String>,
    pub http_credentials: Option<(String, String)>,
}

impl ContextState {
    pub fn apply_to_tab(&self, tab: &Tab) -> Result<(), String> {
        if let Some(viewport) = &self.viewport {
            apply_viewport(tab, viewport)?;
        }
        if self.media.media.is_some()
            || self.media.color_scheme.is_some()
            || self.media.reduced_motion.is_some()
            || self.media.forced_colors.is_some()
            || self.media.contrast.is_some()
        {
            apply_media(tab, &self.media)?;
        }
        if let Some(locale) = &self.locale {
            tab.call_method(Emulation::SetLocaleOverride {
                locale: Some(locale.clone()),
            })
            .map_err(|error| format!("Failed to set locale: {error}"))?;
        }
        if let Some(timezone) = &self.timezone {
            tab.call_method(Emulation::SetTimezoneOverride {
                timezone_id: timezone.clone(),
            })
            .map_err(|error| format!("Failed to set timezone: {error}"))?;
        }
        if let Some((user_agent, accept_language)) = &self.user_agent {
            tab.call_method(Emulation::SetUserAgentOverride {
                user_agent: user_agent.clone(),
                accept_language: accept_language.clone(),
                platform: None,
                user_agent_metadata: None,
            })
            .map_err(|error| format!("Failed to set user agent: {error}"))?;
        }
        if let Some((latitude, longitude, accuracy)) = self.geolocation {
            tab.call_method(Emulation::SetGeolocationOverride {
                latitude: Some(latitude),
                longitude: Some(longitude),
                accuracy: Some(accuracy),
                altitude: None,
                altitude_accuracy: None,
                heading: None,
                speed: None,
            })
            .map_err(|error| format!("Failed to set geolocation: {error}"))?;
        }
        apply_offline(tab, self.offline)?;
        apply_extra_http_headers(tab, &self.extra_http_headers)?;
        if let Some((username, password)) = &self.http_credentials {
            tab.authenticate(Some(username.clone()), Some(password.clone()))
                .map_err(|error| format!("Failed to set HTTP credentials: {error}"))?;
        }
        Ok(())
    }

    pub fn clear_overrides(&mut self, tabs: &[std::sync::Arc<Tab>]) -> Result<(), String> {
        self.viewport = None;
        self.media = MediaState::default();
        self.locale = None;
        self.timezone = None;
        self.user_agent = None;
        self.geolocation = None;
        self.offline = false;
        self.permissions.clear();
        for tab in tabs {
            tab.call_method(Emulation::ClearDeviceMetricsOverride(None))
                .map_err(|error| format!("Failed to clear viewport: {error}"))?;
            tab.call_method(Emulation::SetTouchEmulationEnabled {
                enabled: false,
                max_touch_points: None,
            })
            .map_err(|error| format!("Failed to clear touch emulation: {error}"))?;
            apply_media(tab, &self.media)?;
            tab.call_method(Emulation::SetLocaleOverride { locale: None })
                .map_err(|error| format!("Failed to clear locale: {error}"))?;
            tab.call_method(Emulation::SetTimezoneOverride {
                timezone_id: String::new(),
            })
            .map_err(|error| format!("Failed to clear timezone: {error}"))?;
            tab.call_method(Emulation::SetUserAgentOverride {
                user_agent: String::new(),
                accept_language: None,
                platform: None,
                user_agent_metadata: None,
            })
            .map_err(|error| format!("Failed to clear user agent: {error}"))?;
            tab.call_method(Emulation::ClearGeolocationOverride(None))
                .map_err(|error| format!("Failed to clear geolocation: {error}"))?;
            apply_offline(tab, self.offline)?;
            clear_permissions(tab)?;
        }
        Ok(())
    }

    pub fn summary(
        &self,
        cookie_count: usize,
        local_storage_count: usize,
        session_storage_count: usize,
    ) -> BrowserContextSummary {
        BrowserContextSummary {
            viewport: self.viewport.as_ref().map(|viewport| {
                format!(
                    "{}x{} @{}x{}{}",
                    viewport.width,
                    viewport.height,
                    viewport.device_scale_factor,
                    if viewport.is_mobile { " mobile" } else { "" },
                    if viewport.has_touch { " touch" } else { "" }
                )
            }),
            locale: self.locale.clone(),
            timezone: self.timezone.clone(),
            color_scheme: self.media.color_scheme.clone(),
            permissions: self.permissions.clone(),
            cookie_count,
            local_storage_count,
            session_storage_count,
            offline: self.offline,
            http_credentials: self.http_credentials.is_some(),
        }
    }
}

pub fn apply_viewport(tab: &Tab, viewport: &ViewportState) -> Result<(), String> {
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width: viewport.width,
        height: viewport.height,
        device_scale_factor: viewport.device_scale_factor,
        mobile: viewport.is_mobile,
        scale: None,
        screen_width: None,
        screen_height: None,
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .map_err(|error| format!("Failed to set viewport: {error}"))?;
    tab.call_method(Emulation::SetTouchEmulationEnabled {
        enabled: viewport.has_touch,
        max_touch_points: viewport.has_touch.then_some(1),
    })
    .map_err(|error| format!("Failed to set touch emulation: {error}"))?;
    Ok(())
}

pub fn apply_media(tab: &Tab, media: &MediaState) -> Result<(), String> {
    let features = [
        ("prefers-color-scheme", media.color_scheme.as_ref()),
        ("prefers-reduced-motion", media.reduced_motion.as_ref()),
        ("forced-colors", media.forced_colors.as_ref()),
        ("prefers-contrast", media.contrast.as_ref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|value| Emulation::MediaFeature {
            name: name.to_string(),
            value: value.clone(),
        })
    })
    .collect::<Vec<_>>();
    tab.call_method(Emulation::SetEmulatedMedia {
        media: media.media.clone(),
        features: Some(features),
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to emulate media: {error}"))
}

pub fn apply_offline(tab: &Tab, offline: bool) -> Result<(), String> {
    tab.call_method(Network::EmulateNetworkConditions {
        offline,
        latency: 0.0,
        download_throughput: -1.0,
        upload_throughput: -1.0,
        connection_Type: None,
        packet_loss: None,
        packet_queue_length: None,
        packet_reordering: None,
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to set offline mode: {error}"))
}

pub fn apply_extra_http_headers(
    tab: &Tab,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    tab.call_method(Network::SetExtraHTTPHeaders {
        headers: Network::Headers(Some(
            serde_json::to_value(headers)
                .map_err(|error| format!("Failed to serialize extra HTTP headers: {error}"))?,
        )),
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to set extra HTTP headers: {error}"))
}

pub fn get_cookies(tab: &Tab, urls: Option<Vec<String>>) -> Result<Vec<BrowserCookie>, String> {
    tab.call_method(Network::GetCookies { urls })
        .map_err(|error| format!("Failed to get cookies: {error}"))
        .map(|response| response.cookies.into_iter().map(cookie_from_cdp).collect())
}

pub fn set_cookies(tab: &Tab, cookies: &[BrowserCookie]) -> Result<(), String> {
    tab.call_method(Network::SetCookies {
        cookies: cookies.iter().map(cookie_to_cdp).collect(),
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to set cookies: {error}"))
}

pub fn clear_cookies(
    tab: &Tab,
    name: Option<&str>,
    domain: Option<&str>,
    path: Option<&str>,
) -> Result<usize, String> {
    let cookies = get_cookies(tab, None)?;
    let matching = cookies
        .into_iter()
        .filter(|cookie| name.is_none_or(|name| cookie.name == name))
        .filter(|cookie| domain.is_none_or(|domain| cookie.domain == domain))
        .filter(|cookie| path.is_none_or(|path| cookie.path == path))
        .collect::<Vec<_>>();
    for cookie in &matching {
        tab.call_method(Network::DeleteCookies {
            name: cookie.name.clone(),
            url: None,
            domain: Some(cookie.domain.clone()),
            path: Some(cookie.path.clone()),
            partition_key: None,
        })
        .map_err(|error| format!("Failed to clear cookie {}: {error}", cookie.name))?;
    }
    Ok(matching.len())
}

pub fn get_storage(
    tab: &Tab,
    kind: BrowserStorageKind,
    origin: Option<&str>,
) -> Result<Vec<BrowserStorageItem>, String> {
    with_origin(tab, origin, || {
        let storage = match kind {
            BrowserStorageKind::Local => "localStorage",
            BrowserStorageKind::Session => "sessionStorage",
        };
        let value = tab
            .evaluate(
                &format!(
                    "JSON.stringify(Object.entries({storage}).map(([name,value]) => ({{name,value}})))"
                ),
                false,
            )
            .map_err(|error| format!("Failed to read storage: {error}"))?
            .value
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| "Storage read returned no value".to_string())?;
        serde_json::from_str(&value).map_err(|error| format!("Failed to parse storage: {error}"))
    })
}

pub fn set_storage(
    tab: &Tab,
    kind: BrowserStorageKind,
    items: &[BrowserStorageItem],
) -> Result<(), String> {
    let storage = match kind {
        BrowserStorageKind::Local => "localStorage",
        BrowserStorageKind::Session => "sessionStorage",
    };
    let items = serde_json::to_string(items)
        .map_err(|error| format!("Failed to serialize storage items: {error}"))?;
    tab.evaluate(
        &format!("for (const item of {items}) {storage}.setItem(item.name, item.value)"),
        false,
    )
    .map(|_| ())
    .map_err(|error| format!("Failed to set storage: {error}"))
}

pub fn clear_storage(tab: &Tab, kind: BrowserStorageKind) -> Result<(), String> {
    let storage = match kind {
        BrowserStorageKind::Local => "localStorage",
        BrowserStorageKind::Session => "sessionStorage",
    };
    tab.evaluate(&format!("{storage}.clear()"), false)
        .map(|_| ())
        .map_err(|error| format!("Failed to clear storage: {error}"))
}

pub fn storage_state(tab: &Tab) -> Result<BrowserStorageState, String> {
    let cookies = get_cookies(tab, None)?;
    let origin = current_origin(tab)?;
    let local_storage = get_storage(tab, BrowserStorageKind::Local, None).unwrap_or_default();
    let origins = if origin == "null" {
        Vec::new()
    } else {
        vec![BrowserStorageOrigin {
            origin,
            local_storage,
        }]
    };
    Ok(BrowserStorageState { cookies, origins })
}

#[derive(Clone, Debug, PartialEq)]
pub struct StorageStateArtifact {
    pub path: PathBuf,
    pub bytes: usize,
}

pub fn save_storage_state(
    state: &BrowserStorageState,
    artifacts_dir: &Path,
    save_as: &str,
) -> Result<StorageStateArtifact, String> {
    let file_name = Path::new(save_as)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| *value == save_as && !value.is_empty())
        .ok_or_else(|| {
            "save_as must be a file name inside the runtime artifact directory".to_string()
        })?;
    std::fs::create_dir_all(artifacts_dir).map_err(|error| {
        format!(
            "Failed to create browser artifacts directory {}: {error}",
            artifacts_dir.display()
        )
    })?;
    let path = artifacts_dir.join(file_name);
    let body = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Failed to serialize storage state: {error}"))?;
    std::fs::write(&path, &body).map_err(|error| {
        format!(
            "Failed to save storage state artifact {}: {error}",
            path.display()
        )
    })?;
    Ok(StorageStateArtifact {
        path,
        bytes: body.len(),
    })
}

pub fn set_storage_state(tab: &Tab, state: &BrowserStorageState) -> Result<(), String> {
    set_cookies(tab, &state.cookies)?;
    for origin in &state.origins {
        with_origin(tab, Some(&origin.origin), || {
            clear_storage(tab, BrowserStorageKind::Local)?;
            set_storage(tab, BrowserStorageKind::Local, &origin.local_storage)
        })?;
    }
    Ok(())
}

pub fn grant_permissions(
    tab: &Tab,
    permissions: &[String],
    origin: Option<String>,
) -> Result<(), String> {
    for permission in permissions {
        tab.call_method(Browser::SetPermission {
            permission: Browser::PermissionDescriptor {
                name: permission.clone(),
                sysex: None,
                user_visible_only: None,
                allow_without_sanitization: None,
                allow_without_gesture: None,
                pan_tilt_zoom: None,
            },
            setting: Browser::PermissionSetting::Granted,
            origin: origin.clone(),
            embedding_origin: None,
            browser_context_id: None,
        })
        .map_err(|error| format!("Failed to grant permission {permission}: {error}"))?;
    }
    Ok(())
}

pub fn clear_permissions(tab: &Tab) -> Result<(), String> {
    tab.call_method(Browser::ResetPermissions {
        browser_context_id: None,
    })
    .map(|_| ())
    .map_err(|error| format!("Failed to clear permissions: {error}"))
}

pub fn mask_cookies(cookies: &[BrowserCookie]) -> Vec<BrowserCookie> {
    cookies
        .iter()
        .cloned()
        .map(|mut cookie| {
            cookie.value = "[REDACTED]".to_string();
            cookie
        })
        .collect()
}

pub fn mask_storage_state(state: &BrowserStorageState) -> BrowserStorageState {
    BrowserStorageState {
        cookies: mask_cookies(&state.cookies),
        origins: state
            .origins
            .iter()
            .map(|origin| BrowserStorageOrigin {
                origin: origin.origin.clone(),
                local_storage: origin
                    .local_storage
                    .iter()
                    .map(|item| BrowserStorageItem {
                        name: item.name.clone(),
                        value: "[REDACTED]".to_string(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn current_origin(tab: &Tab) -> Result<String, String> {
    tab.evaluate("location.origin", false)
        .map_err(|error| format!("Failed to read page origin: {error}"))?
        .value
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| "Page origin returned no value".to_string())
}

fn with_origin<T>(
    tab: &Tab,
    origin: Option<&str>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let previous = tab.get_url();
    if let Some(origin) = origin {
        if current_origin(tab).ok().as_deref() != Some(origin) {
            tab.navigate_to(origin)
                .and_then(|tab| tab.wait_until_navigated())
                .map_err(|error| {
                    format!("Failed to navigate to storage origin {origin}: {error}")
                })?;
        }
    }
    let result = operation();
    if origin.is_some() && previous != tab.get_url() {
        let _ = tab
            .navigate_to(&previous)
            .and_then(|tab| tab.wait_until_navigated());
    }
    result
}

fn cookie_from_cdp(cookie: Network::Cookie) -> BrowserCookie {
    BrowserCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: cookie.path,
        expires: (cookie.expires >= 0.0).then_some(cookie.expires),
        http_only: cookie.http_only,
        secure: cookie.secure,
        same_site: cookie.same_site.map(|value| match value {
            Network::CookieSameSite::Strict => BrowserCookieSameSite::Strict,
            Network::CookieSameSite::Lax => BrowserCookieSameSite::Lax,
            Network::CookieSameSite::None => BrowserCookieSameSite::None,
        }),
        url: None,
    }
}

fn cookie_to_cdp(cookie: &BrowserCookie) -> Network::CookieParam {
    Network::CookieParam {
        name: cookie.name.clone(),
        value: cookie.value.clone(),
        url: cookie.url.clone(),
        domain: (!cookie.domain.is_empty()).then(|| cookie.domain.clone()),
        path: Some(cookie.path.clone()),
        secure: Some(cookie.secure),
        http_only: Some(cookie.http_only),
        same_site: cookie.same_site.map(|value| match value {
            BrowserCookieSameSite::Strict => Network::CookieSameSite::Strict,
            BrowserCookieSameSite::Lax => Network::CookieSameSite::Lax,
            BrowserCookieSameSite::None => Network::CookieSameSite::None,
        }),
        expires: cookie.expires,
        priority: None,
        same_party: None,
        source_scheme: None,
        source_port: None,
        partition_key: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> BrowserStorageState {
        BrowserStorageState {
            cookies: vec![BrowserCookie {
                name: "session".to_string(),
                value: "cookie-secret".to_string(),
                domain: "example.test".to_string(),
                path: "/".to_string(),
                expires: None,
                http_only: true,
                secure: true,
                same_site: Some(BrowserCookieSameSite::Lax),
                url: None,
            }],
            origins: vec![BrowserStorageOrigin {
                origin: "https://example.test".to_string(),
                local_storage: vec![BrowserStorageItem {
                    name: "token".to_string(),
                    value: "storage-secret".to_string(),
                }],
            }],
        }
    }

    #[test]
    fn emulation_payload_keeps_viewport_and_media_features() {
        let viewport = ViewportState {
            width: 390,
            height: 844,
            device_scale_factor: 3.0,
            is_mobile: true,
            has_touch: true,
        };
        assert_eq!(viewport.width, 390);
        assert!(viewport.is_mobile && viewport.has_touch);
        let media = MediaState {
            color_scheme: Some("dark".to_string()),
            reduced_motion: Some("reduce".to_string()),
            ..Default::default()
        };
        assert_eq!(media.color_scheme.as_deref(), Some("dark"));
    }

    #[test]
    fn cookie_and_storage_state_are_redacted_for_reports() {
        let state = sample_state();
        let serialized = serde_json::to_string(&mask_storage_state(&state)).unwrap();
        assert!(!serialized.contains("cookie-secret"));
        assert!(!serialized.contains("storage-secret"));
        assert!(serialized.contains("[REDACTED]"));
    }

    #[test]
    fn saved_storage_state_artifact_round_trips_unredacted_while_report_stays_masked() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts_dir = dir.path().join("artifacts");
        let state = sample_state();
        let artifact = save_storage_state(&state, &artifacts_dir, "auth.json").unwrap();

        assert_eq!(artifact.path, artifacts_dir.join("auth.json"));
        let body = std::fs::read(&artifact.path).unwrap();
        assert_eq!(artifact.bytes, body.len());
        let restored: BrowserStorageState = serde_json::from_slice(&body).unwrap();
        assert_eq!(restored, state);
        assert_eq!(restored.cookies[0].value, "cookie-secret");
        assert_eq!(restored.origins[0].local_storage[0].value, "storage-secret");

        let report = serde_json::to_string(&mask_storage_state(&state)).unwrap();
        assert!(!report.contains("cookie-secret"));
        assert!(!report.contains("storage-secret"));
    }

    #[test]
    fn saved_storage_state_rejects_paths_outside_the_artifact_directory() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts_dir = dir.path().join("artifacts");
        let state = sample_state();
        for save_as in ["../x.json", "nested/x.json", "/tmp/x.json", "..", ""] {
            let error = save_storage_state(&state, &artifacts_dir, save_as).unwrap_err();
            assert!(error.contains("save_as must be a file name"), "{error}");
            assert!(!error.contains("cookie-secret"));
            assert!(!error.contains("storage-secret"));
        }
        assert!(!artifacts_dir.join("x.json").exists());
    }
}
