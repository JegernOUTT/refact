use refact_core::claude_backend::{ClaudeAuthType, ClaudeBackend, ClaudeTransportConfig};
use crate::claude_code::ClaudeCodeProvider;

pub const SECRET_FIELDS: &[&str] = &[
    "api_key",
    "auth_token",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "foundry_api_key",
    "foundry_auth_token",
];

pub fn resolve(value: &Option<String>, env: &[&str]) -> Option<String> {
    resolve_with(value, env, |name| std::env::var(name).ok())
}

pub(crate) fn resolve_with(
    value: &Option<String>,
    env: &[&str],
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match value.as_deref().filter(|s| !s.is_empty()) {
        Some(s) if s.starts_with('$') => lookup(&s[1..]).filter(|s| !s.is_empty()),
        Some(s) => Some(s.to_string()),
        None => env
            .iter()
            .find_map(|name| lookup(name).filter(|s| !s.is_empty())),
    }
}

impl ClaudeCodeProvider {
    pub fn is_subscription(&self) -> bool {
        self.transport.backend == ClaudeBackend::Oauth
    }

    pub fn supports_live_discovery(&self) -> bool {
        matches!(
            self.transport.backend,
            ClaudeBackend::Oauth | ClaudeBackend::Anthropic
        )
    }

    pub fn resolved_transport(&self) -> ClaudeTransportConfig {
        let t = &self.transport;
        let mut result = ClaudeTransportConfig {
            backend: t.backend,
            auth_type: t.auth_type,
            ..Default::default()
        };
        match t.backend {
            ClaudeBackend::Oauth => {}
            ClaudeBackend::Anthropic => {
                result.api_key = resolve(&t.api_key, &["ANTHROPIC_API_KEY"]);
                result.base_url = resolve(&t.base_url, &["ANTHROPIC_BASE_URL"]);
            }
            ClaudeBackend::Custom => {
                result.base_url = resolve(&t.base_url, &["ANTHROPIC_BASE_URL"]);
                match t.auth_type {
                    ClaudeAuthType::ApiKey => {
                        result.api_key = resolve(&t.api_key, &["ANTHROPIC_API_KEY"])
                    }
                    ClaudeAuthType::Bearer => {
                        result.auth_token = resolve(&t.auth_token, &["ANTHROPIC_AUTH_TOKEN"])
                    }
                    ClaudeAuthType::None => {}
                }
            }
            ClaudeBackend::Bedrock => {
                result.auth_token = resolve(&None, &["AWS_BEARER_TOKEN_BEDROCK"]);
                result.aws_region = resolve(&t.aws_region, &["AWS_REGION", "AWS_DEFAULT_REGION"]);
                result.aws_profile = resolve(&t.aws_profile, &["AWS_PROFILE"]);
                result.aws_access_key_id = resolve(&t.aws_access_key_id, &[]);
                result.aws_secret_access_key = resolve(&t.aws_secret_access_key, &[]);
                result.aws_session_token = resolve(&t.aws_session_token, &[]);
            }
            ClaudeBackend::Vertex => {
                result.gcp_project = resolve(
                    &t.gcp_project,
                    &[
                        "ANTHROPIC_VERTEX_PROJECT_ID",
                        "GOOGLE_CLOUD_PROJECT",
                        "GCLOUD_PROJECT",
                    ],
                );
                result.gcp_region =
                    resolve(&t.gcp_region, &["CLOUD_ML_REGION", "GOOGLE_CLOUD_LOCATION"]);
            }
            ClaudeBackend::Foundry => {
                result.azure_resource = resolve(&t.azure_resource, &["ANTHROPIC_FOUNDRY_RESOURCE"]);
                result.api_key = resolve(&t.foundry_api_key, &["ANTHROPIC_FOUNDRY_API_KEY"]);
                result.auth_token =
                    resolve(&t.foundry_auth_token, &["ANTHROPIC_FOUNDRY_AUTH_TOKEN"]);
                result.base_url = resolve(&t.foundry_base_url, &["ANTHROPIC_FOUNDRY_BASE_URL"]);
            }
        }
        result
    }

    pub fn apply_shared_settings(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        let patch = yaml
            .as_mapping()
            .ok_or("Provider settings must be a mapping")?;
        let mut current =
            serde_yaml::to_value(&*self).map_err(|_| "Cannot serialize Claude settings")?;
        let map = current.as_mapping_mut().ok_or("Invalid Claude settings")?;
        for (key, value) in patch {
            let name = key.as_str().ok_or("Setting names must be strings")?;
            if ["auth_status", "oauth_connected"].contains(&name) {
                continue;
            }
            if [
                "backend",
                "auth_type",
                "api_key",
                "auth_token",
                "base_url",
                "aws_region",
                "aws_profile",
                "aws_access_key_id",
                "aws_secret_access_key",
                "aws_session_token",
                "gcp_project",
                "gcp_region",
                "azure_resource",
                "foundry_api_key",
                "foundry_auth_token",
                "foundry_base_url",
            ]
            .contains(&name)
                && !value.is_string()
                && !value.is_null()
            {
                return Err(format!("{name} must be a string"));
            }
            if name == "enabled_models"
                && !value
                    .as_sequence()
                    .is_some_and(|items| items.iter().all(serde_yaml::Value::is_string))
            {
                return Err("enabled_models must be a list of strings".into());
            }
            if SECRET_FIELDS.contains(&name) && value.as_str() == Some("***") {
                continue;
            }
            if name == "oauth_tokens" {
                let tokens = value.as_mapping().ok_or("oauth_tokens must be a mapping")?;
                let old = map
                    .entry(key.clone())
                    .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
                let old = old.as_mapping_mut().ok_or("Invalid OAuth tokens")?;
                for (k, v) in tokens {
                    if v.as_str() != Some("***") {
                        old.insert(k.clone(), v.clone());
                    }
                }
            } else {
                map.insert(key.clone(), value.clone());
            }
        }
        let candidate: Self = serde_yaml::from_value(current)
            .map_err(|_| "Malformed Claude provider setting".to_string())?;
        candidate.validate_settings()?;
        *self = candidate;
        Ok(())
    }

    pub fn validate_settings(&self) -> Result<(), String> {
        let value = serde_json::to_value(&self.transport).map_err(|_| "Invalid backend config")?;
        for (field, v) in value.as_object().ok_or("Invalid backend config")? {
            if let Some(s) = v.as_str() {
                if s.chars().any(char::is_control) {
                    return Err(format!("{field} contains control characters"));
                }
                if let Some(name) = s.strip_prefix('$') {
                    if name.is_empty()
                        || !name.chars().enumerate().all(|(i, c)| {
                            c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                        })
                    {
                        return Err(format!("{field} has an invalid environment reference"));
                    }
                }
            }
        }
        for url in [&self.transport.base_url, &self.transport.foundry_base_url]
            .into_iter()
            .filter_map(|value| value.as_deref())
            .filter(|value| !value.is_empty() && !value.starts_with('$'))
        {
            let parsed =
                reqwest::Url::parse(url).map_err(|_| "base_url must be an absolute HTTP(S) URL")?;
            if !matches!(parsed.scheme(), "https" | "http")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(
                    "base_url must be an HTTP(S) URL without credentials, query or fragment".into(),
                );
            }
        }
        for (name, value) in [
            ("aws_region", &self.transport.aws_region),
            ("gcp_region", &self.transport.gcp_region),
            ("azure_resource", &self.transport.azure_resource),
        ] {
            if value.as_deref().is_some_and(|s| {
                !s.is_empty()
                    && !s.starts_with('$')
                    && !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }) {
                return Err(format!(
                    "{name} must contain only letters, digits and hyphens"
                ));
            }
        }
        Ok(())
    }
}

pub const SCHEMA: &str = r#"
fields:
  backend:
    f_type: select
    f_label: "Backend"
    f_default: oauth
    f_options:
      - {value: oauth, label: "Claude subscription"}
      - {value: anthropic, label: "Anthropic API key"}
      - {value: bedrock, label: "Amazon Bedrock"}
      - {value: vertex, label: "Google Vertex AI"}
      - {value: foundry, label: "Microsoft Foundry / Azure"}
      - {value: custom, label: "Custom endpoint"}
    f_desc: "OAuth login and usage apply only to Claude subscriptions. Cloud/custom backends use explicit custom model IDs."
  auth_type:
    f_type: select
    f_label: "Custom authentication"
    f_default: api_key
    f_options:
      - {value: api_key, label: "API key"}
      - {value: bearer, label: "Bearer token"}
      - {value: none, label: "No authentication"}
    f_visible_when: {backend: [custom]}
  api_key:
    f_type: string_long
    f_label: "API Key"
    f_secret: true
    f_visible_when_any:
      - {backend: [anthropic]}
      - {backend: [custom], auth_type: [api_key]}
    f_desc: "Literal or $ENV reference. Empty uses ANTHROPIC_API_KEY."
    f_placeholder: "sk-ant-... or $MY_API_KEY"
  auth_token:
    f_type: string_long
    f_label: "Bearer Token"
    f_secret: true
    f_visible_when_any:
      - {backend: [custom], auth_type: [bearer]}
    f_desc: "Literal or $ENV reference. Empty uses ANTHROPIC_AUTH_TOKEN."
    f_placeholder: "$MY_AUTH_TOKEN"
  base_url:
    f_type: string
    f_label: "Base URL"
    f_visible_when: {backend: [anthropic, custom]}
    f_desc: "Required for custom endpoints. HTTP(S) API root, optionally ending in /v1. Accepts $ENV. Empty uses ANTHROPIC_BASE_URL."
    f_placeholder: "https://api.anthropic.com"
  aws_region:
    f_type: string
    f_label: "AWS Region"
    f_visible_when: {backend: [bedrock]}
    f_desc: "Literal or $ENV. Empty uses AWS_REGION / AWS_DEFAULT_REGION and native AWS profile region defaults."
    f_placeholder: "us-east-1"
  aws_profile:
    f_type: string
    f_label: "AWS Profile"
    f_extra: true
    f_visible_when: {backend: [bedrock]}
    f_desc: "Optional named profile or $ENV. Empty uses AWS_PROFILE and the native AWS credential chain (profiles, SSO, roles)."
    f_placeholder: "default"
  aws_access_key_id:
    f_type: string_long
    f_label: "AWS Access Key ID"
    f_secret: true
    f_extra: true
    f_visible_when: {backend: [bedrock]}
    f_desc: "Optional explicit credentials or $ENV; set together with secret access key. Empty uses the native AWS credential chain."
    f_placeholder: "$AWS_ACCESS_KEY_ID"
  aws_secret_access_key:
    f_type: string_long
    f_label: "AWS Secret Access Key"
    f_secret: true
    f_extra: true
    f_visible_when: {backend: [bedrock]}
    f_desc: "Optional explicit secret or $ENV. Empty uses the native AWS credential chain."
    f_placeholder: "$AWS_SECRET_ACCESS_KEY"
  aws_session_token:
    f_type: string_long
    f_label: "AWS Session Token"
    f_secret: true
    f_extra: true
    f_visible_when: {backend: [bedrock]}
    f_desc: "Optional temporary credential token or $ENV. Empty uses the native AWS credential chain."
    f_placeholder: "$AWS_SESSION_TOKEN"
  gcp_project:
    f_type: string
    f_label: "Google Cloud Project"
    f_visible_when: {backend: [vertex]}
    f_desc: "Literal or $ENV. Empty uses ANTHROPIC_VERTEX_PROJECT_ID / GOOGLE_CLOUD_PROJECT / GCLOUD_PROJECT. Authentication uses application default credentials."
    f_placeholder: "my-project-id"
  gcp_region:
    f_type: string
    f_label: "Google Cloud Region"
    f_visible_when: {backend: [vertex]}
    f_desc: "Literal or $ENV. Empty uses CLOUD_ML_REGION / GOOGLE_CLOUD_LOCATION."
    f_placeholder: "us-east5"
  azure_resource:
    f_type: string
    f_label: "Azure Resource"
    f_visible_when: {backend: [foundry]}
    f_desc: "Foundry resource name or $ENV. Empty uses ANTHROPIC_FOUNDRY_RESOURCE. A custom Foundry base URL may be supplied instead."
    f_placeholder: "my-resource"
  foundry_api_key:
    f_type: string_long
    f_label: "Foundry API Key"
    f_secret: true
    f_visible_when: {backend: [foundry]}
    f_desc: "Optional key or $ENV. Empty uses ANTHROPIC_FOUNDRY_API_KEY, otherwise native Azure credentials."
  foundry_auth_token:
    f_type: string_long
    f_label: "Foundry Bearer Token"
    f_secret: true
    f_extra: true
    f_visible_when: {backend: [foundry]}
    f_desc: "Optional token or $ENV. Empty uses ANTHROPIC_FOUNDRY_AUTH_TOKEN. A token takes precedence over the Foundry API key."
  foundry_base_url:
    f_type: string
    f_label: "Foundry Base URL"
    f_visible_when: {backend: [foundry]}
    f_desc: "Optional HTTPS endpoint ending in /anthropic. Accepts $ENV. Empty uses ANTHROPIC_FOUNDRY_BASE_URL or the Azure resource name."
    f_placeholder: "https://my-resource.services.ai.azure.com/anthropic"
oauth:
  supported: true
  methods:
    - id: max
      label: "Claude Pro/Max"
      description: "Login with your Claude Pro or Max subscription"
description: |
  Use Claude subscriptions, the Anthropic API, cloud platforms, or a custom Messages endpoint. Credentials are verified on request, not on save.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#;
