use std::sync::Arc;

use serde::Deserialize;

use crate::call_validation::{ChatModelType, SubchatParameters};
use crate::caps::resolve_chat_model;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::llm::params::CacheControl;
use crate::subchat::ExplicitSubchatSpec;
use crate::tools::review_types::ReviewDepth;

pub const SWARM_SUBAGENT_ID: &str = "review_agents";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSlot {
    Chat,
    Chat2,
    Thinking,
    Light,
}

impl ModelSlot {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Chat2 => "chat2",
            Self::Thinking => "thinking",
            Self::Light => "light",
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StaticSection {
    pub enabled: bool,
}

impl Default for StaticSection {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DeadCodeSection {
    pub enabled: bool,
    pub min_confidence: f64,
}

impl Default for DeadCodeSection {
    fn default() -> Self {
        Self {
            enabled: true,
            min_confidence: 0.7,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GitSection {
    pub enabled: bool,
    pub max_commits: usize,
}

impl Default for GitSection {
    fn default() -> Self {
        Self {
            enabled: true,
            max_commits: 1000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OneshotSection {
    pub enabled: bool,
    pub ensemble: Vec<ModelSlot>,
    pub prompt: Option<String>,
    pub n_ctx: Option<usize>,
    pub max_new_tokens: Option<usize>,
    pub tokens_for_rag: Option<usize>,
    pub temperature: Option<f32>,
}

impl Default for OneshotSection {
    fn default() -> Self {
        Self {
            enabled: true,
            ensemble: vec![ModelSlot::Chat, ModelSlot::Chat2, ModelSlot::Thinking],
            prompt: None,
            n_ctx: None,
            max_new_tokens: None,
            tokens_for_rag: None,
            temperature: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgenticSection {
    pub enabled: bool,
    pub model_slot: ModelSlot,
    pub max_steps: usize,
    pub tools: Vec<String>,
    pub prompt: Option<String>,
    pub n_ctx: Option<usize>,
    pub max_new_tokens: Option<usize>,
}

impl Default for AgenticSection {
    fn default() -> Self {
        Self {
            enabled: true,
            model_slot: ModelSlot::Chat,
            max_steps: 25,
            tools: vec![],
            prompt: None,
            n_ctx: None,
            max_new_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExecSection {
    #[serde(flatten)]
    pub base: AgenticSection,
    #[serde(default = "default_true")]
    pub allow_execution: bool,
    pub mutation_probe_cap: Option<usize>,
}

impl Default for ExecSection {
    fn default() -> Self {
        Self {
            base: AgenticSection {
                model_slot: ModelSlot::Thinking,
                max_steps: 50,
                ..Default::default()
            },
            allow_execution: true,
            mutation_probe_cap: Some(6),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BrowserSection {
    #[serde(flatten)]
    pub base: AgenticSection,
    pub app_url: Option<String>,
    pub dev_server_command: Option<String>,
    #[serde(default = "default_true")]
    pub allow_dev_server_boot: bool,
}

impl Default for BrowserSection {
    fn default() -> Self {
        Self {
            base: AgenticSection {
                model_slot: ModelSlot::Thinking,
                max_steps: 50,
                ..Default::default()
            },
            app_url: None,
            dev_server_command: None,
            allow_dev_server_boot: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VerifierSection {
    pub model_slot: ModelSlot,
    pub prompt: Option<String>,
    pub n_ctx: usize,
    pub max_new_tokens: usize,
    pub temperature: f32,
}

impl Default for VerifierSection {
    fn default() -> Self {
        Self {
            model_slot: ModelSlot::Thinking,
            prompt: None,
            n_ctx: 64000,
            max_new_tokens: 4000,
            temperature: 0.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GatherSection {
    pub model_slot: ModelSlot,
    pub system_prompt: Option<String>,
    pub retry_prompt: Option<String>,
    pub tools: Vec<String>,
    pub max_steps: usize,
    pub max_files: usize,
    pub n_ctx: usize,
    pub max_new_tokens: usize,
    pub temperature: f32,
}

impl Default for GatherSection {
    fn default() -> Self {
        Self {
            model_slot: ModelSlot::Light,
            system_prompt: None,
            retry_prompt: None,
            tools: crate::tools::subagent_phases::DEFAULT_GATHER_FILES_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            max_steps: 18,
            max_files: 60,
            n_ctx: 200000,
            max_new_tokens: 16000,
            temperature: 0.2,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReviewSwarmConfig {
    pub default_depth: String,
    pub max_parallel: usize,
    pub oneshot_timeout_secs: u64,
    pub agentic_timeout_secs: u64,
    pub exec_timeout_secs: u64,
    pub browser_timeout_secs: u64,
    pub gather: GatherSection,
    pub verifier: VerifierSection,
    pub s1_security: StaticSection,
    pub s2_dead_code: DeadCodeSection,
    pub s3_duplication: StaticSection,
    pub s4_test_integrity: StaticSection,
    pub s5_dependencies: StaticSection,
    pub s6_git_enrichment: GitSection,
    pub l1_diff: OneshotSection,
    pub l2_simplicity: OneshotSection,
    pub l3_spec: OneshotSection,
    pub a1_repo_context: AgenticSection,
    pub a2_research: AgenticSection,
    pub a3_execution: ExecSection,
    pub a4_browser: BrowserSection,
}

impl Default for ReviewSwarmConfig {
    fn default() -> Self {
        Self {
            default_depth: "quick".to_string(),
            max_parallel: 10,
            oneshot_timeout_secs: 300,
            agentic_timeout_secs: 600,
            exec_timeout_secs: 900,
            browser_timeout_secs: 900,
            gather: Default::default(),
            verifier: Default::default(),
            s1_security: Default::default(),
            s2_dead_code: Default::default(),
            s3_duplication: Default::default(),
            s4_test_integrity: Default::default(),
            s5_dependencies: Default::default(),
            s6_git_enrichment: Default::default(),
            l1_diff: Default::default(),
            l2_simplicity: Default::default(),
            l3_spec: Default::default(),
            a1_repo_context: AgenticSection {
                max_steps: 25,
                ..Default::default()
            },
            a2_research: AgenticSection {
                max_steps: 25,
                ..Default::default()
            },
            a3_execution: Default::default(),
            a4_browser: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewAgentsConfig {
    pub swarm: ReviewSwarmConfig,
    pub reviewer_prompt: String,
    pub guardrails_prompt: String,
    pub base_params: SubchatParameters,
}

impl ReviewAgentsConfig {
    pub fn depth_or_default(&self, requested: Option<ReviewDepth>) -> ReviewDepth {
        requested
            .or_else(|| ReviewDepth::parse(&self.swarm.default_depth))
            .unwrap_or(ReviewDepth::Quick)
    }
}

pub async fn load_review_agents_config(
    gcx: Arc<GlobalContext>,
) -> Result<ReviewAgentsConfig, String> {
    let subagent = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx.clone(),
        SWARM_SUBAGENT_ID,
        None,
    )
    .await
    .ok_or_else(|| format!("subagent config '{}' not found", SWARM_SUBAGENT_ID))?;

    let swarm = match subagent.extra.get("review_swarm") {
        Some(value) => serde_yaml::from_value::<ReviewSwarmConfig>(value.clone())
            .map_err(|e| format!("invalid review_swarm config: {e}"))?,
        None => ReviewSwarmConfig::default(),
    };

    let reviewer_prompt = subagent
        .prompts
        .reviewer
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or("reviewer prompt not configured for review_agents")?;
    let guardrails_prompt = subagent
        .prompts
        .guardrails
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or("guardrails prompt not configured for review_agents")?;

    let base_params = crate::subchat::resolve_subchat_params(gcx, SWARM_SUBAGENT_ID).await?;

    Ok(ReviewAgentsConfig {
        swarm,
        reviewer_prompt,
        guardrails_prompt,
        base_params,
    })
}

pub async fn slot_model_id(gcx: Arc<GlobalContext>, slot: ModelSlot) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;
    let defaults = &caps.defaults;
    let candidates: Vec<&String> = match slot {
        ModelSlot::Chat => vec![&defaults.chat_default_model],
        ModelSlot::Chat2 => vec![
            &defaults.chat_model_2,
            &defaults.chat_light_model,
            &defaults.chat_default_model,
        ],
        ModelSlot::Thinking => vec![&defaults.chat_thinking_model, &defaults.chat_default_model],
        ModelSlot::Light => vec![&defaults.chat_light_model, &defaults.chat_default_model],
    };
    for candidate in candidates {
        if candidate.trim().is_empty() {
            continue;
        }
        if let Ok(record) = resolve_chat_model(caps.clone(), candidate) {
            return Ok(record.base.id.clone());
        }
    }
    Err(format!(
        "no usable model configured for slot '{}'; set Default model settings",
        slot.label()
    ))
}

fn spec_from_params(params: SubchatParameters, model: String) -> ExplicitSubchatSpec {
    ExplicitSubchatSpec {
        params,
        model,
        autonomous_no_confirm: true,
    }
}

pub fn oneshot_spec(
    base: &SubchatParameters,
    section: &OneshotSection,
    model: String,
) -> ExplicitSubchatSpec {
    let mut params = base.clone();
    params.subchat_model = model.clone();
    if let Some(n_ctx) = section.n_ctx {
        params.subchat_n_ctx = n_ctx;
    }
    if let Some(max_new_tokens) = section.max_new_tokens {
        params.subchat_max_new_tokens = max_new_tokens;
    }
    if let Some(tokens_for_rag) = section.tokens_for_rag {
        params.subchat_tokens_for_rag = tokens_for_rag;
    }
    if let Some(temperature) = section.temperature {
        params.subchat_temperature = Some(temperature);
    }
    spec_from_params(params, model)
}

pub fn agentic_spec(
    base: &SubchatParameters,
    section: &AgenticSection,
    model: String,
) -> ExplicitSubchatSpec {
    let mut params = base.clone();
    params.subchat_model = model.clone();
    params.subchat_tokens_for_rag = 0;
    params.subchat_cache_control = CacheControl::Ephemeral;
    if let Some(n_ctx) = section.n_ctx {
        params.subchat_n_ctx = n_ctx;
    }
    if let Some(max_new_tokens) = section.max_new_tokens {
        params.subchat_max_new_tokens = max_new_tokens;
    }
    spec_from_params(params, model)
}

pub fn verifier_spec(section: &VerifierSection, model: String) -> ExplicitSubchatSpec {
    let params = SubchatParameters {
        subchat_model_type: ChatModelType::Thinking,
        subchat_model: model.clone(),
        subchat_n_ctx: section.n_ctx,
        subchat_max_new_tokens: section.max_new_tokens,
        subchat_temperature: Some(section.temperature),
        subchat_tokens_for_rag: 0,
        subchat_reasoning_effort: None,
        subchat_cache_control: CacheControl::Off,
    };
    spec_from_params(params, model)
}

pub fn gather_spec(section: &GatherSection, model: String) -> ExplicitSubchatSpec {
    let params = SubchatParameters {
        subchat_model_type: ChatModelType::Light,
        subchat_model: model.clone(),
        subchat_n_ctx: section.n_ctx,
        subchat_max_new_tokens: section.max_new_tokens,
        subchat_temperature: Some(section.temperature),
        subchat_tokens_for_rag: 0,
        subchat_reasoning_effort: None,
        subchat_cache_control: CacheControl::Ephemeral,
    };
    spec_from_params(params, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_review_swarm_config_defaults_are_sane() {
        let cfg = ReviewSwarmConfig::default();
        assert_eq!(cfg.default_depth, "quick");
        assert_eq!(cfg.max_parallel, 10);
        assert_eq!(cfg.l1_diff.ensemble.len(), 3);
        assert!(cfg.a3_execution.allow_execution);
        assert_eq!(cfg.a3_execution.base.max_steps, 50);
        assert_eq!(cfg.a1_repo_context.max_steps, 25);
        assert_eq!(cfg.verifier.n_ctx, 64000);
        assert_eq!(cfg.gather.max_files, 60);
    }

    #[test]
    fn tool_review_swarm_config_parses_from_yaml_extra() {
        let yaml = r#"
default_depth: standard
max_parallel: 4
l1_diff:
  ensemble: [thinking]
  temperature: 0.3
a3_execution:
  enabled: false
  allow_execution: false
  max_steps: 12
a4_browser:
  app_url: "http://localhost:5173"
s2_dead_code:
  min_confidence: 0.9
"#;
        let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let cfg: ReviewSwarmConfig = serde_yaml::from_value(value).unwrap();
        assert_eq!(cfg.default_depth, "standard");
        assert_eq!(cfg.max_parallel, 4);
        assert_eq!(cfg.l1_diff.ensemble, vec![ModelSlot::Thinking]);
        assert_eq!(cfg.l1_diff.temperature, Some(0.3));
        assert!(!cfg.a3_execution.base.enabled);
        assert!(!cfg.a3_execution.allow_execution);
        assert_eq!(cfg.a3_execution.base.max_steps, 12);
        assert_eq!(
            cfg.a4_browser.app_url.as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(cfg.s2_dead_code.min_confidence, 0.9);
        assert!(cfg.l2_simplicity.enabled);
    }

    #[test]
    fn tool_review_model_slots_deserialize_lowercase() {
        let slots: Vec<ModelSlot> = serde_yaml::from_str("[chat, chat2, thinking, light]").unwrap();
        assert_eq!(
            slots,
            vec![
                ModelSlot::Chat,
                ModelSlot::Chat2,
                ModelSlot::Thinking,
                ModelSlot::Light
            ]
        );
    }
}
