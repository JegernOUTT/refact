use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use refact_core::claude_backend::{ClaudeBackend, ClaudeTransportConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::claude_code::ClaudeCodeProvider;
use crate::traits::ProviderTrait;

pub const MAX_MODELS: usize = 8;
pub const MAX_PROFILE_BYTES: u64 = 524_288;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub sso_configured: bool,
}

#[derive(Serialize)]
pub struct Profiles {
    pub profiles: Vec<Profile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_region: Option<String>,
}

pub fn valid_profile(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.@+".contains(&b))
}

pub fn valid_region(region: &str) -> bool {
    (5..=64).contains(&region.len())
        && region.contains('-')
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn parse_profiles(text: &str) -> Vec<Profile> {
    let mut profiles = BTreeMap::<String, Profile>::new();
    let mut section = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            section = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| {
                    let name = if s == "default" {
                        s
                    } else {
                        s.strip_prefix("profile ")?.trim()
                    };
                    valid_profile(name).then(|| name.to_string())
                });
            if let Some(name) = &section {
                profiles.entry(name.clone()).or_insert_with(|| Profile {
                    name: name.clone(),
                    region: None,
                    sso_configured: false,
                });
            }
            continue;
        }
        let Some(profile) = section.as_ref().and_then(|name| profiles.get_mut(name)) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "region" if valid_region(value) => profile.region = Some(value.into()),
            "sso_session" | "sso_start_url" if !value.is_empty() => profile.sso_configured = true,
            _ => {}
        }
    }
    profiles.into_values().collect()
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("AWS_CONFIG_FILE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".aws").join("config"))
        })
}

pub struct HostProfiles {
    pub profiles: Vec<Profile>,
    pub fingerprint: String,
}

pub async fn host_profiles() -> Result<HostProfiles, String> {
    tokio::task::spawn_blocking(host_profiles_now)
        .await
        .map_err(|_| "AWS source inspection failed")?
}

// The config-store CAS callback is synchronous. Keep its final check bounded to the
// same two files/bytes as the async metadata snapshot; never invoke the AWS SDK here.
pub fn host_fingerprint_now() -> Result<String, String> {
    Ok(host_profiles_now()?.fingerprint)
}

pub fn host_profiles_now() -> Result<HostProfiles, String> {
    let config = config_path().ok_or("AWS config path unavailable")?;
    let credentials = std::env::var_os("AWS_SHARED_CREDENTIALS_FILE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".aws/credentials"))
        })
        .ok_or("AWS credential source location unavailable")?;
    let env: BTreeMap<_, _> = std::env::vars_os()
        .filter(|(k, _)| {
            k.to_string_lossy().starts_with("AWS_") || k == "HOME" || k == "USERPROFILE"
        })
        .collect();
    host_profiles_from_sources(&config, &credentials, env)
}

fn host_profiles_from_sources(
    config: &std::path::Path,
    credentials: &std::path::Path,
    env: BTreeMap<std::ffi::OsString, std::ffi::OsString>,
) -> Result<HostProfiles, String> {
    use std::io::Read;
    let mut hash = Sha256::new();
    let mut profiles = BTreeMap::new();
    for (key, value) in env {
        hash.update(key.as_encoded_bytes());
        hash.update([0]);
        hash.update(value.as_encoded_bytes());
        hash.update([0]);
    }
    for path in [config, credentials] {
        hash.update(path.as_os_str().as_encoded_bytes());
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        match options.open(path) {
            Ok(file) => {
                if !file
                    .metadata()
                    .map_err(|_| "Cannot inspect AWS credential source")?
                    .is_file()
                {
                    return Err("AWS credential source must be a regular file".into());
                }
                let mut bytes = Vec::new();
                std::io::Read::take(file, MAX_PROFILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Cannot fingerprint AWS credential source")?;
                if bytes.len() as u64 > MAX_PROFILE_BYTES {
                    return Err("AWS credential source exceeds setup size limit".into());
                }
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| "Invalid AWS configuration encoding")?;
                if path == config {
                    for profile in parse_profiles(text) {
                        profiles.insert(profile.name.clone(), profile);
                    }
                } else {
                    for line in text.lines() {
                        if let Some(name) = line
                            .trim()
                            .strip_prefix('[')
                            .and_then(|s| s.strip_suffix(']'))
                        {
                            if valid_profile(name) {
                                profiles.entry(name.to_string()).or_insert_with(|| Profile {
                                    name: name.to_string(),
                                    region: None,
                                    sso_configured: false,
                                });
                            }
                        }
                    }
                }
                hash.update(bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update(b"missing"),
            Err(_) => return Err("Cannot fingerprint AWS credential source".into()),
        }
        hash.update([0]);
    }
    if profiles.len() > 256 {
        return Err("Too many AWS profiles".into());
    }
    Ok(HostProfiles {
        profiles: profiles.into_values().collect(),
        fingerprint: format!("{:x}", hash.finalize()),
    })
}

pub fn suggestions(
    profiles: Vec<Profile>,
    current_profile: Option<String>,
    current_region: Option<String>,
    env_profile: Option<String>,
    env_region: Option<String>,
) -> Profiles {
    let known = |name: &str| profiles.iter().any(|p| p.name == name);
    let suggested_profile = current_profile
        .filter(|s| known(s))
        .or_else(|| env_profile.filter(|s| known(s)))
        .or_else(|| (profiles.len() == 1).then(|| profiles[0].name.clone()))
        .or_else(|| known("default").then(|| "default".into()));
    let suggested_region = current_region
        .filter(|r| valid_region(r))
        .or_else(|| env_region.filter(|r| valid_region(r)))
        .or_else(|| {
            profiles
                .iter()
                .find(|p| Some(&p.name) == suggested_profile.as_ref())
                .and_then(|p| p.region.clone())
        });
    Profiles {
        profiles,
        suggested_profile,
        suggested_region,
    }
}

pub fn selected_transport(
    provider: &ClaudeCodeProvider,
    profile: &str,
    region: &str,
    env_present: impl Fn(&str) -> bool,
) -> Result<ClaudeTransportConfig, String> {
    if provider.transport.backend != ClaudeBackend::Bedrock {
        return Err("This provider is not configured for Amazon Bedrock".into());
    }
    if !valid_profile(profile) || !valid_region(region) {
        return Err("Select a valid AWS profile and region".into());
    }
    if [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_BEARER_TOKEN_BEDROCK",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
        "AWS_ROLE_ARN",
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    ]
    .iter()
    .any(|key| env_present(key))
        || [
            &provider.transport.aws_access_key_id,
            &provider.transport.aws_secret_access_key,
            &provider.transport.aws_session_token,
            &provider.transport.auth_token,
        ]
        .iter()
        .any(|value| value.as_ref().is_some_and(|s| !s.is_empty()))
    {
        return Err("Profile setup cannot verify a different credential override. Clear saved static AWS credentials and AWS bearer/static/web-identity/container environment overrides on the engine host, or use manual configuration.".into());
    }
    Ok(ClaudeTransportConfig {
        backend: ClaudeBackend::Bedrock,
        aws_profile: Some(profile.into()),
        aws_region: Some(region.into()),
        ..Default::default()
    })
}

pub fn validate_models(models: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    if models.is_empty() || models.len() > MAX_MODELS {
        return Err(format!("Select between 1 and {MAX_MODELS} exact model IDs"));
    }
    for model in models {
        if model.is_empty()
            || model.len() > 512
            || !seen.insert(model)
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        {
            return Err(
                "Model IDs must be unique exact Bedrock IDs or ARNs without whitespace".into(),
            );
        }
    }
    Ok(())
}

pub fn fingerprint(value: &impl Serialize) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| "Cannot fingerprint provider configuration")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn known_claude(id: &str) -> bool {
    let id = id.rsplit(":foundation-model/").next().unwrap_or(id);
    let id = ["us.", "eu.", "apac.", "global.", "jp.", "au.", "us-gov."]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id);
    matches!(
        id,
        "anthropic.claude-sonnet-4-6"
            | "anthropic.claude-opus-4-6"
            | "anthropic.claude-opus-4-7"
            | "anthropic.claude-opus-4-8"
            | "anthropic.claude-opus-5"
            | "anthropic.claude-opus-5-5"
            | "anthropic.claude-sonnet-5"
            | "anthropic.claude-haiku-4-5-20251001-v1:0"
            | "anthropic.claude-sonnet-4-5-20250929-v1:0"
            | "anthropic.claude-sonnet-4-20250514-v1:0"
            | "anthropic.claude-opus-4-1-20250805-v1:0"
            | "anthropic.claude-opus-4-20250514-v1:0"
    )
}

pub fn pin_models(
    existing: serde_yaml::Value,
    profile: &str,
    region: &str,
    model_ids: &[String],
    context_1m: bool,
) -> Result<serde_yaml::Value, String> {
    validate_models(model_ids)?;
    let mut settings = existing
        .as_mapping()
        .cloned()
        .ok_or("Provider configuration must be a mapping")?;
    let mut custom = settings
        .get(&serde_yaml::Value::String("custom_models".into()))
        .cloned()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| "Invalid custom models")?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    if let Some(previous) = settings
        .get(&serde_yaml::Value::String(
            "bedrock_context_1m_models".into(),
        ))
        .and_then(serde_yaml::Value::as_sequence)
    {
        for id in previous.iter().filter_map(serde_yaml::Value::as_str) {
            if let Some(metadata) = custom.get_mut(id).and_then(Value::as_object_mut) {
                if metadata.get("n_ctx") == Some(&json!(1_000_000)) {
                    metadata.insert(
                        "n_ctx".into(),
                        json!(if known_claude(id) { 200_000 } else { 32_000 }),
                    );
                }
            }
        }
    }
    for id in model_ids {
        let known = known_claude(id);
        let entry = custom.entry(id.clone()).or_insert_with(|| {
            json!({
                "supports_tools": known, "supports_multimodality": known
            })
        });
        let metadata = entry
            .as_object_mut()
            .ok_or("Invalid custom model metadata")?;
        if context_1m {
            metadata.insert("n_ctx".into(), json!(1_000_000));
        } else {
            // A one-token probe establishes invocation, not a context-window or
            // tool/vision capability guarantee for an opaque application ARN.
            metadata
                .entry("n_ctx".to_string())
                .or_insert(json!(if known { 200_000 } else { 32_000 }));
        }
    }
    for (key, value) in [
        ("aws_profile", json!(profile)),
        ("aws_region", json!(region)),
        ("enabled", json!(true)),
        ("enabled_models", json!(model_ids)),
        ("custom_models", Value::Object(custom)),
        ("disabled_models", json!([])),
        (
            "bedrock_context_1m_models",
            json!(if context_1m { model_ids } else { &[] }),
        ),
    ] {
        settings.insert(
            serde_yaml::Value::String(key.into()),
            serde_yaml::to_value(value).map_err(|_| "Cannot serialize Bedrock selection")?,
        );
    }
    let result = serde_yaml::Value::Mapping(settings);
    let mut provider = ClaudeCodeProvider::default();
    provider.provider_settings_apply(result.clone())?;
    Ok(result)
}

/// Mutable evidence is held under the engine's per-session operation mutex.
#[derive(Default)]
pub struct SessionState {
    successful: HashSet<(String, bool)>,
    consumed: bool,
}

impl SessionState {
    pub fn available(&self) -> Result<(), &'static str> {
        if self.consumed {
            Err("Setup already applied; verify again")
        } else {
            Ok(())
        }
    }
    pub fn begin_probe(&mut self, ids: &[String], context_1m: bool) {
        for id in ids {
            self.successful.remove(&(id.clone(), context_1m));
        }
    }
    pub fn record_probe(&mut self, id: &str, context_1m: bool, working: bool) {
        self.successful.remove(&(id.to_string(), context_1m));
        if working {
            self.successful.insert((id.to_string(), context_1m));
        }
    }
    pub fn require_tested(&self, ids: &[String], context_1m: bool) -> Result<(), &'static str> {
        self.available()?;
        if ids
            .iter()
            .any(|id| !self.successful.contains(&(id.clone(), context_1m)))
        {
            Err("Every selected model and context mode must pass an explicit probe before apply")
        } else {
            Ok(())
        }
    }
    pub fn consume(&mut self) {
        self.consumed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_evidence_invalidates_before_reprobe_and_fences_apply() {
        let operation = std::sync::Arc::new(tokio::sync::Mutex::new(SessionState::default()));
        let ids = vec!["custom-application-id".to_string()];
        let mut probe = operation.clone().try_lock_owned().unwrap();
        probe.record_probe(&ids[0], false, true);
        assert!(probe.require_tested(&ids, false).is_ok());
        assert!(probe.require_tested(&ids, true).is_err());
        probe.begin_probe(&ids, false);
        assert!(
            probe.require_tested(&ids, false).is_err(),
            "old success survives pending probe"
        );
        assert!(
            operation.clone().try_lock_owned().is_err(),
            "concurrent apply entered probe"
        );
        // Dropping an interrupted request's owned guard must release ownership, not restore evidence.
        drop(probe);
        let mut apply = operation.clone().try_lock_owned().unwrap();
        assert!(apply.require_tested(&ids, false).is_err());
        apply.record_probe(&ids[0], false, true);
        assert!(apply.require_tested(&ids, false).is_ok());
        apply.consume();
        drop(apply);
        assert!(
            operation.try_lock().unwrap().available().is_err(),
            "consumed setup replayed"
        );
    }

    #[test]
    fn source_fingerprint_tracks_credentials_even_with_unchanged_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let credentials = dir.path().join("credentials");
        std::fs::write(&config, "[profile team]\nregion=us-east-1\n").unwrap();
        std::fs::write(&credentials, "[team]\naws_secret_access_key=old-fixture\n").unwrap();
        let before = host_profiles_from_sources(&config, &credentials, BTreeMap::new()).unwrap();
        std::fs::write(&credentials, "[team]\naws_secret_access_key=new-fixture\n").unwrap();
        let after = host_profiles_from_sources(&config, &credentials, BTreeMap::new()).unwrap();
        assert_eq!(before.profiles, after.profiles);
        assert_ne!(
            before.fingerprint, after.fingerprint,
            "credential changes retained stale evidence"
        );
        assert_eq!(after.profiles[0].name, "team");
        assert!(!serde_json::to_string(&after.profiles)
            .unwrap()
            .contains("fixture"));
        #[cfg(unix)]
        {
            use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
            let fifo = dir.path().join("credential-pipe");
            let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            assert!(std::fs::metadata(&fifo).unwrap().file_type().is_fifo());
            assert_eq!(
                host_profiles_from_sources(&config, &fifo, BTreeMap::new())
                    .err()
                    .unwrap(),
                "AWS credential source must be a regular file"
            );
        }
    }

    #[test]
    fn profile_metadata_never_returns_config_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aws-config");
        std::fs::write(&path, "[default]\nregion=us-east-1\naws_secret_access_key=fixture-secret\n[profile team]\nregion=eu-west-1\nsso_session=company\n[sso-session company]\nsso_start_url=https://secret.example\n").unwrap();
        let profiles = host_profiles_from_sources(
            &path,
            &dir.path().join("missing-credentials"),
            BTreeMap::new(),
        )
        .unwrap()
        .profiles;
        assert_eq!(
            profiles,
            vec![
                Profile {
                    name: "default".into(),
                    region: Some("us-east-1".into()),
                    sso_configured: false
                },
                Profile {
                    name: "team".into(),
                    region: Some("eu-west-1".into()),
                    sso_configured: true
                }
            ]
        );
        let wire = serde_json::to_value(suggestions(
            profiles,
            Some("team".into()),
            None,
            Some("default".into()),
            None,
        ))
        .unwrap();
        assert_eq!(wire["suggested_profile"], "team");
        assert_eq!(wire["suggested_region"], "eu-west-1");
        assert!(!wire.to_string().contains("fixture-secret"));
        assert!(!wire.to_string().contains("secret.example"));
    }

    #[test]
    fn profile_override_refusal_preserves_manual_credentials() {
        let provider = ClaudeCodeProvider {
            transport: ClaudeTransportConfig {
                backend: ClaudeBackend::Bedrock,
                aws_access_key_id: Some("fixture-key".into()),
                aws_secret_access_key: Some("fixture-secret".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(selected_transport(&provider, "team", "us-east-1", |_| false).is_err());
        assert_eq!(
            provider.resolved_transport().aws_access_key_id.as_deref(),
            Some("fixture-key")
        );
        let provider = ClaudeCodeProvider {
            transport: ClaudeTransportConfig {
                backend: ClaudeBackend::Bedrock,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(selected_transport(&provider, "team", "us-east-1", |key| key
            == "AWS_BEARER_TOKEN_BEDROCK")
        .is_err());
        let selected = selected_transport(&provider, "team", "us-east-1", |_| false).unwrap();
        assert_eq!(selected.aws_profile.as_deref(), Some("team"));
        assert!(selected.auth_token.is_none());
    }

    #[test]
    fn exact_model_pins_preserve_unrelated_settings_and_wire_beta() {
        let original: serde_yaml::Value = serde_yaml::from_str("base_provider: claude_code\nbackend: bedrock\ndisplay_name: Team\napi_key: keep-fixture\nenabled_models: [old]\ncustom_models: {old: {n_ctx: 10}, us.anthropic.claude-sonnet-4-6: {n_ctx: 10, supports_tools: false, custom_note: retain}}\n").unwrap();
        let result = pin_models(
            original,
            "team",
            "eu-west-1",
            &["us.anthropic.claude-sonnet-4-6".into()],
            true,
        )
        .unwrap();
        assert_eq!(
            result["custom_models"]["us.anthropic.claude-sonnet-4-6"]["custom_note"].as_str(),
            Some("retain")
        );
        assert_eq!(
            result["custom_models"]["us.anthropic.claude-sonnet-4-6"]["supports_tools"].as_bool(),
            Some(false)
        );
        assert_eq!(result["api_key"].as_str(), Some("keep-fixture"));
        assert_eq!(result["display_name"].as_str(), Some("Team"));
        assert_eq!(
            result["enabled_models"],
            serde_yaml::to_value(["us.anthropic.claude-sonnet-4-6"]).unwrap()
        );
        assert_eq!(result["custom_models"]["old"]["n_ctx"].as_u64(), Some(10));
        assert_eq!(
            result["custom_models"]["us.anthropic.claude-sonnet-4-6"]["n_ctx"].as_u64(),
            Some(1_000_000)
        );
        let mut provider = ClaudeCodeProvider::default();
        provider.provider_settings_apply(result.clone()).unwrap();
        let runtime = provider.build_runtime().unwrap();
        let transport: ClaudeTransportConfig = serde_json::from_str(
            runtime
                .extra_headers
                .get(refact_core::claude_backend::CLAUDE_TRANSPORT_HEADER)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            transport.bedrock_context_1m_models,
            ["us.anthropic.claude-sonnet-4-6"]
        );
        let mut enabled = result;
        enabled["custom_models"]["manual"] =
            serde_yaml::from_str("n_ctx: 1000000\nsupports_tools: false").unwrap();
        let downgraded = pin_models(
            enabled,
            "team",
            "eu-west-1",
            &["us.anthropic.claude-sonnet-4-6".into(), "manual".into()],
            false,
        )
        .unwrap();
        assert_eq!(
            downgraded["custom_models"]["us.anthropic.claude-sonnet-4-6"]["n_ctx"].as_u64(),
            Some(200_000),
            "wizard context survived downgrade"
        );
        assert_eq!(
            downgraded["custom_models"]["manual"]["n_ctx"].as_u64(),
            Some(1_000_000),
            "manual context was overwritten"
        );
        assert_eq!(
            downgraded["custom_models"]["old"]["n_ctx"].as_u64(),
            Some(10)
        );
        assert_eq!(
            downgraded["custom_models"]["us.anthropic.claude-sonnet-4-6"]["custom_note"].as_str(),
            Some("retain")
        );
        provider.provider_settings_apply(downgraded).unwrap();
        assert!(provider
            .resolved_transport()
            .bedrock_context_1m_models
            .is_empty());
    }
}
