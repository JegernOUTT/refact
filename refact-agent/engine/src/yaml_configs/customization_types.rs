use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub specific: bool,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub llm_defaults: LlmDefaults,
    #[serde(default)]
    pub tool_confirm: ToolConfirmConfig,
    #[serde(default)]
    pub thread_defaults: ModeThreadDefaults,
    #[serde(default)]
    pub ui: ModeUi,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub match_models: Option<Vec<String>>,
    #[serde(default, rename = "override")]
    pub override_config: Option<ModeOverride>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeThreadDefaults {
    #[serde(default)]
    pub include_project_info: Option<bool>,
    #[serde(default)]
    pub checkpoints_enabled: Option<bool>,
    #[serde(default)]
    pub automatic_patch: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeUi {
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmDefaults {
    #[serde(default)]
    pub max_new_tokens: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub boost_reasoning: Option<bool>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub tool_choice: Option<String>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolConfirmConfig {
    #[serde(default)]
    pub rules: Vec<ToolConfirmRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfirmRule {
    #[serde(rename = "match")]
    pub match_pattern: String,
    pub action: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeOverride {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub tools_replace: Option<Vec<String>>,
    #[serde(default)]
    pub tools_add: Option<Vec<String>>,
    #[serde(default)]
    pub tools_remove: Option<Vec<String>>,
    #[serde(default)]
    pub llm_defaults: Option<LlmDefaults>,
    #[serde(default)]
    pub tool_confirm: Option<ToolConfirmConfig>,
    #[serde(default)]
    pub thread_defaults: Option<ModeThreadDefaults>,
}

impl ModeConfig {
    pub fn apply_override(&self, override_config: &ModeOverride) -> ModeConfig {
        let mut result = self.clone();
        if let Some(prompt) = &override_config.prompt {
            result.prompt = prompt.clone();
        }
        if let Some(tools) = &override_config.tools_replace {
            result.tools = tools.clone();
        } else {
            if let Some(add) = &override_config.tools_add {
                for tool in add {
                    if !result.tools.contains(tool) {
                        result.tools.push(tool.clone());
                    }
                }
            }
            if let Some(remove) = &override_config.tools_remove {
                result.tools.retain(|t| !remove.contains(t));
            }
        }
        if let Some(llm) = &override_config.llm_defaults {
            if let Some(v) = llm.max_new_tokens { result.llm_defaults.max_new_tokens = Some(v); }
            if let Some(v) = llm.temperature { result.llm_defaults.temperature = Some(v); }
            if let Some(v) = llm.top_p { result.llm_defaults.top_p = Some(v); }
            if let Some(v) = llm.boost_reasoning { result.llm_defaults.boost_reasoning = Some(v); }
            if let Some(v) = &llm.reasoning_effort { result.llm_defaults.reasoning_effort = Some(v.clone()); }
            if let Some(v) = &llm.tool_choice { result.llm_defaults.tool_choice = Some(v.clone()); }
            if let Some(v) = llm.parallel_tool_calls { result.llm_defaults.parallel_tool_calls = Some(v); }
        }
        if let Some(confirm) = &override_config.tool_confirm {
            result.tool_confirm = confirm.clone();
        }
        if let Some(td) = &override_config.thread_defaults {
            if let Some(v) = td.include_project_info { result.thread_defaults.include_project_info = Some(v); }
            if let Some(v) = td.checkpoints_enabled { result.thread_defaults.checkpoints_enabled = Some(v); }
            if let Some(v) = td.automatic_patch { result.thread_defaults.automatic_patch = Some(v); }
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentConfig {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub specific: bool,
    #[serde(default)]
    pub expose_as_tool: bool,
    #[serde(default)]
    pub has_code: bool,
    #[serde(default)]
    pub tool: Option<SubagentToolSchema>,
    #[serde(default)]
    pub subchat: SubchatConfig,
    #[serde(default)]
    pub messages: SubagentMessages,
    #[serde(default)]
    pub prompts: SubagentPrompts,
    #[serde(default)]
    pub gather_files: GatherFilesConfig,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub match_models: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

impl SubagentConfig {
    pub fn apply_override(&self, ovr: &SubagentConfig) -> SubagentConfig {
        let mut result = self.clone();
        if !ovr.title.is_empty() { result.title = ovr.title.clone(); }
        if !ovr.description.is_empty() { result.description = ovr.description.clone(); }
        if ovr.expose_as_tool { result.expose_as_tool = true; }
        if ovr.has_code { result.has_code = true; }
        if ovr.tool.is_some() { result.tool = ovr.tool.clone(); }
        if ovr.subchat.context_mode != "bare" { result.subchat.context_mode = ovr.subchat.context_mode.clone(); }
        if ovr.subchat.stateful { result.subchat.stateful = true; }
        if ovr.subchat.model.is_some() { result.subchat.model = ovr.subchat.model.clone(); }
        if ovr.subchat.model_type.is_some() { result.subchat.model_type = ovr.subchat.model_type.clone(); }
        if ovr.subchat.n_ctx.is_some() { result.subchat.n_ctx = ovr.subchat.n_ctx; }
        if ovr.subchat.max_new_tokens.is_some() { result.subchat.max_new_tokens = ovr.subchat.max_new_tokens; }
        if ovr.subchat.max_steps.is_some() { result.subchat.max_steps = ovr.subchat.max_steps; }
        if ovr.subchat.temperature.is_some() { result.subchat.temperature = ovr.subchat.temperature; }
        if ovr.subchat.reasoning_effort.is_some() { result.subchat.reasoning_effort = ovr.subchat.reasoning_effort.clone(); }
        if ovr.subchat.tokens_for_rag.is_some() { result.subchat.tokens_for_rag = ovr.subchat.tokens_for_rag; }
        if ovr.messages.system_prompt.is_some() { result.messages.system_prompt = ovr.messages.system_prompt.clone(); }
        if ovr.messages.user_template.is_some() { result.messages.user_template = ovr.messages.user_template.clone(); }
        if !ovr.messages.pre_messages.is_empty() { result.messages.pre_messages = ovr.messages.pre_messages.clone(); }
        if !ovr.messages.post_messages.is_empty() { result.messages.post_messages = ovr.messages.post_messages.clone(); }
        if ovr.prompts.solver.is_some() { result.prompts.solver = ovr.prompts.solver.clone(); }
        if ovr.prompts.reviewer.is_some() { result.prompts.reviewer = ovr.prompts.reviewer.clone(); }
        if ovr.prompts.guardrails.is_some() { result.prompts.guardrails = ovr.prompts.guardrails.clone(); }
        if ovr.prompts.gather_system.is_some() { result.prompts.gather_system = ovr.prompts.gather_system.clone(); }
        if ovr.prompts.gather_retry.is_some() { result.prompts.gather_retry = ovr.prompts.gather_retry.clone(); }
        if ovr.gather_files.subagent.is_some() { result.gather_files.subagent = ovr.gather_files.subagent.clone(); }
        if ovr.gather_files.max_files.is_some() { result.gather_files.max_files = ovr.gather_files.max_files; }
        if ovr.gather_files.max_steps.is_some() { result.gather_files.max_steps = ovr.gather_files.max_steps; }
        if !ovr.tools.is_empty() { result.tools = ovr.tools.clone(); }
        for (k, v) in &ovr.extra {
            result.extra.insert(k.clone(), v.clone());
        }
        result
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentToolSchema {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub agentic: bool,
    #[serde(default)]
    pub parameters: Vec<ToolParameter>,
    #[serde(default)]
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubchatConfig {
    #[serde(default = "default_context_mode")]
    pub context_mode: String,
    #[serde(default)]
    pub stateful: bool,
    #[serde(default)]
    pub max_steps: Option<usize>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_type: Option<String>,
    #[serde(default)]
    pub n_ctx: Option<usize>,
    #[serde(default)]
    pub max_new_tokens: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub tokens_for_rag: Option<usize>,
}

fn default_context_mode() -> String {
    "bare".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentMessages {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub user_template: Option<String>,
    #[serde(default)]
    pub pre_messages: Vec<MessageTemplate>,
    #[serde(default)]
    pub post_messages: Vec<MessageTemplate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentPrompts {
    #[serde(default)]
    pub solver: Option<String>,
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub guardrails: Option<String>,
    #[serde(default)]
    pub gather_system: Option<String>,
    #[serde(default)]
    pub gather_retry: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatherFilesConfig {
    #[serde(default)]
    pub subagent: Option<String>,
    #[serde(default)]
    pub max_files: Option<usize>,
    #[serde(default)]
    pub max_steps: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTemplate {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolboxCommandConfig {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub selection_needed: Option<(usize, usize)>,
    #[serde(default)]
    pub selection_unwanted: bool,
    #[serde(default)]
    pub insert_at_cursor: bool,
    #[serde(default)]
    pub messages: Vec<MessageTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeLensConfig {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub new_tab: bool,
    #[serde(default)]
    pub messages: Vec<MessageTemplate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryError {
    pub file_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectRegistry {
    pub modes: HashMap<String, ModeConfig>,
    pub mode_overrides: Vec<ModeConfig>,
    pub subagents: HashMap<String, SubagentConfig>,
    pub subagent_overrides: Vec<SubagentConfig>,
    pub toolbox_commands: HashMap<String, ToolboxCommandConfig>,
    pub code_lens: HashMap<String, CodeLensConfig>,
    pub errors: Vec<RegistryError>,
}
