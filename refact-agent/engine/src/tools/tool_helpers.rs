use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::get_subagent_config;

pub struct CodeSubagentConfig {
    pub gather_system_prompt: Option<String>,
    pub gather_retry_prompt: Option<String>,
    pub solver_prompt: Option<String>,
    pub reviewer_prompt: Option<String>,
    pub guardrails_prompt: Option<String>,
    pub gather_tools: Option<Vec<String>>,
    pub gather_subagent: Option<String>,
    pub gather_max_steps: Option<usize>,
    pub max_files: Option<usize>,
    pub max_steps: Option<usize>,
    pub subchat_model_type: Option<String>,
    pub subchat_n_ctx: Option<usize>,
    pub subchat_max_new_tokens: Option<usize>,
    pub subchat_tokens_for_rag: Option<usize>,
    pub subchat_reasoning_effort: Option<String>,
}

impl Default for CodeSubagentConfig {
    fn default() -> Self {
        Self {
            gather_system_prompt: None,
            gather_retry_prompt: None,
            solver_prompt: None,
            reviewer_prompt: None,
            guardrails_prompt: None,
            gather_tools: None,
            gather_subagent: None,
            gather_max_steps: None,
            max_files: None,
            max_steps: None,
            subchat_model_type: None,
            subchat_n_ctx: None,
            subchat_max_new_tokens: None,
            subchat_tokens_for_rag: None,
            subchat_reasoning_effort: None,
        }
    }
}

pub async fn load_code_subagent_config(
    gcx: Arc<ARwLock<GlobalContext>>,
    subagent_id: &str,
    model_id: Option<&str>,
) -> Result<CodeSubagentConfig, String> {
    let mut config = CodeSubagentConfig::default();

    let subagent_config = get_subagent_config(gcx.clone(), subagent_id, model_id)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", subagent_id))?;

    config.gather_system_prompt = subagent_config
        .messages
        .system_prompt
        .clone()
        .or_else(|| subagent_config.prompts.gather_system.clone());
    config.gather_retry_prompt = subagent_config.prompts.gather_retry.clone();
    config.solver_prompt = subagent_config.prompts.solver.clone();
    config.reviewer_prompt = subagent_config.prompts.reviewer.clone();
    config.guardrails_prompt = subagent_config.prompts.guardrails.clone();

    if !subagent_config.tools.is_empty() {
        config.gather_tools = Some(subagent_config.tools.clone());
    }

    config.subchat_model_type = subagent_config.subchat.model_type.clone();
    config.subchat_n_ctx = subagent_config.subchat.n_ctx;
    config.subchat_max_new_tokens = subagent_config.subchat.max_new_tokens;
    config.subchat_tokens_for_rag = subagent_config.subchat.tokens_for_rag;
    config.subchat_reasoning_effort = subagent_config.subchat.reasoning_effort.clone();
    config.max_steps = subagent_config.subchat.max_steps;
    config.gather_subagent = subagent_config.gather_files.subagent.clone();
    config.gather_max_steps = subagent_config.gather_files.max_steps;
    config.max_files = subagent_config.gather_files.max_files;

    Ok(config)
}
