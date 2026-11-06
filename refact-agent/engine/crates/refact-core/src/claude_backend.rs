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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bedrock_context_1m_models: Vec<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BedrockContext {
    Standard,
    Beta1m,
    Native1m,
}

#[derive(Clone, Copy, Debug)]
pub struct BedrockModel {
    pub foundation_id: &'static str,
    pub anthropic_id: &'static str,
    pub display_name: &'static str,
    pub context: BedrockContext,
    pub adaptive_thinking: bool,
}

impl BedrockModel {
    pub fn default_n_ctx(self) -> usize {
        if self.context == BedrockContext::Native1m {
            1_000_000
        } else {
            200_000
        }
    }

    pub fn default_caps(self) -> crate::model_caps::ModelCapabilities {
        crate::model_caps::ModelCapabilities {
            n_ctx: self.default_n_ctx(),
            supports_tools: true,
            supports_vision: true,
            supports_thinking_budget: !self.adaptive_thinking,
            supports_adaptive_thinking_budget: self.adaptive_thinking,
            supports_cache_control: true,
            supports_temperature: true,
            tokenizer: crate::model_caps::ANTHROPIC_CLOUD_TOKENIZER.into(),
            ..Default::default()
        }
    }
}

pub fn bedrock_model(model_id: &str) -> Option<BedrockModel> {
    let id = if model_id.starts_with("arn:") {
        let resource = model_id.splitn(6, ':').nth(5)?;
        resource
            .strip_prefix("foundation-model/")
            .or_else(|| resource.strip_prefix("inference-profile/"))?
    } else {
        model_id
    };
    let id = ["us.", "eu.", "apac.", "global.", "jp.", "au.", "us-gov."]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id);
    let (foundation_id, anthropic_id, display_name, context, adaptive_thinking) = match id {
        "anthropic.claude-haiku-4-5-20251001-v1:0" => (
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            "claude-haiku-4-5-20251001",
            "Claude Haiku 4.5",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-sonnet-4-5-20250929-v1:0" => (
            "anthropic.claude-sonnet-4-5-20250929-v1:0",
            "claude-sonnet-4-5-20250929",
            "Claude Sonnet 4.5",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-opus-4-5-20251101-v1:0" => (
            "anthropic.claude-opus-4-5-20251101-v1:0",
            "claude-opus-4-5-20251101",
            "Claude Opus 4.5",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-sonnet-4-20250514-v1:0" => (
            "anthropic.claude-sonnet-4-20250514-v1:0",
            "claude-sonnet-4-20250514",
            "Claude Sonnet 4",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-opus-4-1-20250805-v1:0" => (
            "anthropic.claude-opus-4-1-20250805-v1:0",
            "claude-opus-4-1-20250805",
            "Claude Opus 4.1",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-opus-4-20250514-v1:0" => (
            "anthropic.claude-opus-4-20250514-v1:0",
            "claude-opus-4-20250514",
            "Claude Opus 4",
            BedrockContext::Standard,
            false,
        ),
        "anthropic.claude-opus-4-6" | "anthropic.claude-opus-4-6-v1" => (
            "anthropic.claude-opus-4-6",
            "claude-opus-4-6",
            "Claude Opus 4.6",
            BedrockContext::Beta1m,
            true,
        ),
        "anthropic.claude-sonnet-4-6" | "anthropic.claude-sonnet-4-6-v1" => (
            "anthropic.claude-sonnet-4-6",
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            BedrockContext::Beta1m,
            true,
        ),
        "anthropic.claude-opus-4-7" => (
            "anthropic.claude-opus-4-7",
            "claude-opus-4-7",
            "Claude Opus 4.7",
            BedrockContext::Native1m,
            true,
        ),
        "anthropic.claude-opus-4-8" => (
            "anthropic.claude-opus-4-8",
            "claude-opus-4-8",
            "Claude Opus 4.8",
            BedrockContext::Native1m,
            true,
        ),
        "anthropic.claude-opus-5" => (
            "anthropic.claude-opus-5",
            "claude-opus-5",
            "Claude Opus 5",
            BedrockContext::Native1m,
            true,
        ),
        "anthropic.claude-opus-5-5" => (
            "anthropic.claude-opus-5-5",
            "claude-opus-5-5",
            "Claude Opus 5.5",
            BedrockContext::Native1m,
            true,
        ),
        "anthropic.claude-sonnet-5" => (
            "anthropic.claude-sonnet-5",
            "claude-sonnet-5",
            "Claude Sonnet 5",
            BedrockContext::Native1m,
            true,
        ),
        _ => return None,
    };
    Some(BedrockModel {
        foundation_id,
        anthropic_id,
        display_name,
        context,
        adaptive_thinking,
    })
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
