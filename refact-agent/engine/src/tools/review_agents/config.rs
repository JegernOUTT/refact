use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::call_validation::SubchatParameters;
use crate::caps::resolve_chat_model;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::llm::params::CacheControl;
use crate::subchat::ExplicitSubchatSpec;
use crate::tools::review_types::ReviewDepth;

pub const REVIEW_SUBAGENT_ID: &str = "review_agents";

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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StageOverride {
    pub enabled: Option<bool>,
    pub model_slot: Option<ModelSlot>,
    pub budget_minutes: Option<u64>,
    pub max_steps: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReviewSettings {
    pub default_depth: String,
    pub parallel_depth: usize,
    pub variants: usize,
    pub stage_budget_minutes: u64,
    pub writes_stage_budget_minutes: u64,
    pub deadline_secs: u64,
    pub max_steps: usize,
    pub max_files: usize,
    pub model_slot: ModelSlot,
    pub variant_slots: Vec<ModelSlot>,
    pub n_ctx: Option<usize>,
    pub max_new_tokens: Option<usize>,
    pub stages: HashMap<String, StageOverride>,
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            default_depth: "normal".to_string(),
            parallel_depth: 4,
            variants: 1,
            stage_budget_minutes: 6,
            writes_stage_budget_minutes: 20,
            deadline_secs: 1800,
            max_steps: 40,
            max_files: 60,
            model_slot: ModelSlot::Chat,
            variant_slots: vec![ModelSlot::Thinking, ModelSlot::Chat2],
            n_ctx: None,
            max_new_tokens: None,
            stages: HashMap::new(),
        }
    }
}

impl ReviewSettings {
    pub fn depth_or_default(&self, requested: Option<ReviewDepth>) -> ReviewDepth {
        requested
            .or_else(|| ReviewDepth::parse(&self.default_depth))
            .unwrap_or(ReviewDepth::Normal)
    }

    pub fn slots_for_variants(&self, variants: usize) -> Vec<ModelSlot> {
        let mut slots = vec![self.model_slot];
        for slot in &self.variant_slots {
            if slots.len() >= variants.max(1) {
                break;
            }
            if !slots.contains(slot) {
                slots.push(*slot);
            }
        }
        slots
    }

    pub fn stage_override(&self, stage_id: &str) -> StageOverride {
        self.stages.get(stage_id).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ReviewConfig {
    pub settings: ReviewSettings,
    pub guardrails_prompt: String,
    pub base_params: SubchatParameters,
}

pub async fn load_review_config(gcx: Arc<GlobalContext>) -> Result<ReviewConfig, String> {
    let subagent = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx.clone(),
        REVIEW_SUBAGENT_ID,
        None,
    )
    .await
    .ok_or_else(|| format!("subagent config '{REVIEW_SUBAGENT_ID}' not found"))?;

    let settings = match subagent.extra.get("review") {
        Some(value) => serde_yaml::from_value::<ReviewSettings>(value.clone())
            .map_err(|error| format!("invalid review config: {error}"))?,
        None => ReviewSettings::default(),
    };
    let guardrails_prompt = subagent
        .prompts
        .guardrails
        .clone()
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or("guardrails prompt not configured for review_agents")?;
    let base_params = crate::subchat::resolve_subchat_params(gcx, REVIEW_SUBAGENT_ID).await?;

    Ok(ReviewConfig {
        settings,
        guardrails_prompt,
        base_params,
    })
}

pub async fn slot_model_id(gcx: Arc<GlobalContext>, slot: ModelSlot) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|error| format!("failed to load caps: {error:?}"))?;
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

pub fn stage_subchat_spec(
    base: &SubchatParameters,
    settings: &ReviewSettings,
    model: String,
) -> ExplicitSubchatSpec {
    let mut params = base.clone();
    params.subchat_model = model.clone();
    params.subchat_tokens_for_rag = 0;
    params.subchat_cache_control = CacheControl::Ephemeral;
    if let Some(n_ctx) = settings.n_ctx {
        params.subchat_n_ctx = n_ctx;
    }
    if let Some(max_new_tokens) = settings.max_new_tokens {
        params.subchat_max_new_tokens = max_new_tokens;
    }
    ExplicitSubchatSpec {
        params,
        model,
        autonomous_no_confirm: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_settings_defaults_match_the_documented_arguments() {
        let settings = ReviewSettings::default();

        assert_eq!(settings.default_depth, "normal");
        assert_eq!(settings.parallel_depth, 4);
        assert_eq!(settings.variants, 1);
        assert_eq!(settings.stage_budget_minutes, 6);
        assert_eq!(settings.writes_stage_budget_minutes, 20);
        assert_eq!(settings.deadline_secs, 1800);
        assert_eq!(settings.model_slot, ModelSlot::Chat);
    }

    #[test]
    fn review_settings_variants_pick_distinct_slots_in_order() {
        let settings = ReviewSettings::default();

        assert_eq!(settings.slots_for_variants(1), vec![ModelSlot::Chat]);
        assert_eq!(
            settings.slots_for_variants(2),
            vec![ModelSlot::Chat, ModelSlot::Thinking]
        );
        assert_eq!(
            settings.slots_for_variants(3),
            vec![ModelSlot::Chat, ModelSlot::Thinking, ModelSlot::Chat2]
        );
    }

    #[test]
    fn review_settings_parse_from_yaml_with_stage_overrides() {
        let yaml = r#"
default_depth: deep
parallel_depth: 8
variants: 2
deadline_secs: 900
model_slot: thinking
stages:
  execution:
    enabled: false
  mechanical:
    budget_minutes: 15
    model_slot: light
"#;
        let settings: ReviewSettings = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(settings.default_depth, "deep");
        assert_eq!(settings.parallel_depth, 8);
        assert_eq!(settings.deadline_secs, 900);
        assert_eq!(settings.model_slot, ModelSlot::Thinking);
        assert_eq!(settings.stage_override("execution").enabled, Some(false));
        assert_eq!(
            settings.stage_override("mechanical").budget_minutes,
            Some(15)
        );
        assert_eq!(
            settings.stage_override("mechanical").model_slot,
            Some(ModelSlot::Light)
        );
        assert!(settings.stage_override("diff").enabled.is_none());
        assert_eq!(settings.stage_budget_minutes, 6);
    }

    #[test]
    fn review_stage_spec_is_always_autonomous() {
        let base = SubchatParameters {
            subchat_model_type: crate::call_validation::ChatModelType::Default,
            subchat_model: String::new(),
            subchat_n_ctx: 100000,
            subchat_max_new_tokens: 8000,
            subchat_temperature: None,
            subchat_tokens_for_rag: 5000,
            subchat_reasoning_effort: None,
            subchat_cache_control: CacheControl::Off,
        };

        let spec = stage_subchat_spec(&base, &ReviewSettings::default(), "m".to_string());

        assert!(spec.autonomous_no_confirm);
        assert_eq!(spec.params.subchat_tokens_for_rag, 0);
        assert_eq!(spec.model, "m");
    }
}
