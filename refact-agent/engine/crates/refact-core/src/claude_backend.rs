use serde::{Deserialize, Serialize};

pub const CLAUDE_TRANSPORT_HEADER: &str = "x-refact-internal-claude-transport";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeBackend {
    #[default]
    Oauth,
    Anthropic,
    Bedrock,
    Vertex,
    Foundry,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthType {
    #[default]
    ApiKey,
    Bearer,
    None,
    Oauth,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClaudeTransportConfig {
    pub backend: ClaudeBackend,
    pub auth_type: ClaudeAuthType,
    pub api_key: Option<String>,
    pub auth_token: Option<String>,
    pub base_url: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_profile: Option<String>,
    pub aws_region: Option<String>,
    pub gcp_project: Option<String>,
    pub gcp_region: Option<String>,
    pub azure_resource: Option<String>,
    pub foundry_api_key: Option<String>,
    pub foundry_auth_token: Option<String>,
    pub foundry_base_url: Option<String>,
    pub oauth_workspace_id: Option<String>,
}

impl std::fmt::Debug for ClaudeTransportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeTransportConfig")
            .field("backend", &self.backend)
            .field("auth_type", &self.auth_type)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_backend_roundtrips() {
        let config: ClaudeTransportConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.backend, ClaudeBackend::Oauth);
        for backend in [
            "oauth",
            "anthropic",
            "bedrock",
            "vertex",
            "foundry",
            "custom",
        ] {
            let value = serde_json::json!({"backend": backend});
            let config: ClaudeTransportConfig = serde_json::from_value(value).unwrap();
            assert_eq!(serde_json::to_value(config).unwrap()["backend"], backend);
        }
        assert!(CLAUDE_TRANSPORT_HEADER.starts_with("x-refact-internal-"));
    }
}
