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
    pub allow_integrations: bool,
    #[serde(default)]
    pub allow_mcp: bool,
    #[serde(default)]
    pub allow_subagents: bool,
    #[serde(default)]
    pub model_defaults: ModeModelDefaults,
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
    pub auto_approve_editing_tools: Option<bool>,
    #[serde(default)]
    pub auto_approve_dangerous_commands: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeUi {
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelTypeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_new_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl ModelTypeConfig {
    pub fn merge_from(&mut self, other: &ModelTypeConfig) {
        if other.model.is_some() {
            self.model = other.model.clone();
        }
        if other.max_new_tokens.is_some() {
            self.max_new_tokens = other.max_new_tokens;
        }
        if other.temperature.is_some() {
            self.temperature = other.temperature;
        }
        if other.top_p.is_some() {
            self.top_p = other.top_p;
        }
        if other.boost_reasoning.is_some() {
            self.boost_reasoning = other.boost_reasoning;
        }
        if other.reasoning_effort.is_some() {
            self.reasoning_effort = other.reasoning_effort.clone();
        }
        if other.thinking_budget.is_some() {
            self.thinking_budget = other.thinking_budget;
        }
        if other.tool_choice.is_some() {
            self.tool_choice = other.tool_choice.clone();
        }
        if other.parallel_tool_calls.is_some() {
            self.parallel_tool_calls = other.parallel_tool_calls;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeModelDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ModelTypeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ModelTypeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ModelTypeConfig>,
}

impl ModeModelDefaults {
    pub fn merge_from(&mut self, other: &ModeModelDefaults) {
        if let Some(ref ovr) = other.default {
            self.default
                .get_or_insert_with(ModelTypeConfig::default)
                .merge_from(ovr);
        }
        if let Some(ref ovr) = other.light {
            self.light
                .get_or_insert_with(ModelTypeConfig::default)
                .merge_from(ovr);
        }
        if let Some(ref ovr) = other.thinking {
            self.thinking
                .get_or_insert_with(ModelTypeConfig::default)
                .merge_from(ovr);
        }
    }
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
    pub allow_integrations: Option<bool>,
    #[serde(default)]
    pub allow_mcp: Option<bool>,
    #[serde(default)]
    pub allow_subagents: Option<bool>,
    #[serde(default)]
    pub model_defaults: Option<ModeModelDefaults>,
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
        if let Some(v) = override_config.allow_integrations {
            result.allow_integrations = v;
        }
        if let Some(v) = override_config.allow_mcp {
            result.allow_mcp = v;
        }
        if let Some(v) = override_config.allow_subagents {
            result.allow_subagents = v;
        }
        if let Some(model_defaults) = &override_config.model_defaults {
            result.model_defaults.merge_from(model_defaults);
        }
        if let Some(confirm) = &override_config.tool_confirm {
            result.tool_confirm = confirm.clone();
        }
        if let Some(td) = &override_config.thread_defaults {
            if let Some(v) = td.include_project_info {
                result.thread_defaults.include_project_info = Some(v);
            }
            if let Some(v) = td.checkpoints_enabled {
                result.thread_defaults.checkpoints_enabled = Some(v);
            }
            if let Some(v) = td.auto_approve_editing_tools {
                result.thread_defaults.auto_approve_editing_tools = Some(v);
            }
            if let Some(v) = td.auto_approve_dangerous_commands {
                result.thread_defaults.auto_approve_dangerous_commands = Some(v);
            }
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
        if !ovr.title.is_empty() {
            result.title = ovr.title.clone();
        }
        if !ovr.description.is_empty() {
            result.description = ovr.description.clone();
        }
        if ovr.expose_as_tool {
            result.expose_as_tool = true;
        }
        if ovr.has_code {
            result.has_code = true;
        }
        if ovr.tool.is_some() {
            result.tool = ovr.tool.clone();
        }
        if ovr.subchat.context_mode != "bare" {
            result.subchat.context_mode = ovr.subchat.context_mode.clone();
        }
        if ovr.subchat.stateful {
            result.subchat.stateful = true;
        }
        if ovr.subchat.model.is_some() {
            result.subchat.model = ovr.subchat.model.clone();
        }
        if ovr.subchat.model_type.is_some() {
            result.subchat.model_type = ovr.subchat.model_type.clone();
        }
        if ovr.subchat.n_ctx.is_some() {
            result.subchat.n_ctx = ovr.subchat.n_ctx;
        }
        if ovr.subchat.max_new_tokens.is_some() {
            result.subchat.max_new_tokens = ovr.subchat.max_new_tokens;
        }
        if ovr.subchat.max_steps.is_some() {
            result.subchat.max_steps = ovr.subchat.max_steps;
        }
        if ovr.subchat.temperature.is_some() {
            result.subchat.temperature = ovr.subchat.temperature;
        }
        if ovr.subchat.reasoning_effort.is_some() {
            result.subchat.reasoning_effort = ovr.subchat.reasoning_effort.clone();
        }
        if ovr.subchat.tokens_for_rag.is_some() {
            result.subchat.tokens_for_rag = ovr.subchat.tokens_for_rag;
        }
        if ovr.messages.system_prompt.is_some() {
            result.messages.system_prompt = ovr.messages.system_prompt.clone();
        }
        if ovr.messages.user_template.is_some() {
            result.messages.user_template = ovr.messages.user_template.clone();
        }
        if !ovr.messages.pre_messages.is_empty() {
            result.messages.pre_messages = ovr.messages.pre_messages.clone();
        }
        if !ovr.messages.post_messages.is_empty() {
            result.messages.post_messages = ovr.messages.post_messages.clone();
        }
        if ovr.prompts.solver.is_some() {
            result.prompts.solver = ovr.prompts.solver.clone();
        }
        if ovr.prompts.reviewer.is_some() {
            result.prompts.reviewer = ovr.prompts.reviewer.clone();
        }
        if ovr.prompts.guardrails.is_some() {
            result.prompts.guardrails = ovr.prompts.guardrails.clone();
        }
        if ovr.prompts.gather_system.is_some() {
            result.prompts.gather_system = ovr.prompts.gather_system.clone();
        }
        if ovr.prompts.gather_retry.is_some() {
            result.prompts.gather_retry = ovr.prompts.gather_retry.clone();
        }
        if ovr.gather_files.subagent.is_some() {
            result.gather_files.subagent = ovr.gather_files.subagent.clone();
        }
        if ovr.gather_files.max_files.is_some() {
            result.gather_files.max_files = ovr.gather_files.max_files;
        }
        if ovr.gather_files.max_steps.is_some() {
            result.gather_files.max_steps = ovr.gather_files.max_steps;
        }
        if !ovr.tools.is_empty() {
            result.tools = ovr.tools.clone();
        }
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
    pub allow_parallel: bool,
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
    /// Prompt for generating commit message from diff only
    #[serde(default)]
    pub diff_only: Option<String>,
    /// Prompt for generating commit message when user provides context
    #[serde(default)]
    pub diff_with_user_text: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProjectRegistry {
    pub modes: HashMap<String, ModeConfig>,
    pub mode_overrides: Vec<ModeConfig>,
    pub subagents: HashMap<String, SubagentConfig>,
    pub subagent_overrides: Vec<SubagentConfig>,
    pub toolbox_commands: HashMap<String, ToolboxCommandConfig>,
    pub code_lens: HashMap<String, CodeLensConfig>,
    pub errors: Vec<RegistryError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_confirm_rule_serialization() {
        let rule = ToolConfirmRule {
            match_pattern: "tree".to_string(),
            action: "auto".to_string(),
        };

        let yaml = serde_yaml::to_string(&rule).unwrap();
        assert!(yaml.contains("match:"));
        assert!(!yaml.contains("match_pattern:"));

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"match\":"));
        assert!(!json.contains("\"match_pattern\":"));
    }

    #[test]
    fn test_tool_confirm_rule_deserialization() {
        let yaml = "match: tree\naction: auto";
        let rule: ToolConfirmRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.match_pattern, "tree");
        assert_eq!(rule.action, "auto");

        let json = r#"{"match": "shell", "action": "ask"}"#;
        let rule: ToolConfirmRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.match_pattern, "shell");
        assert_eq!(rule.action, "ask");
    }

    #[test]
    fn test_mode_config_roundtrip() {
        let yaml = r#"
schema_version: 1
id: test_mode
title: Test Mode
description: A test mode
specific: false
prompt: "Test prompt"
tools:
  - tree
  - cat
tool_confirm:
  rules:
    - match: "tree"
      action: auto
    - match: "shell"
      action: ask
"#;
        let config: ModeConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.id, "test_mode");
        assert_eq!(config.tools, vec!["tree", "cat"]);
        assert_eq!(config.tool_confirm.rules.len(), 2);
        assert_eq!(config.tool_confirm.rules[0].match_pattern, "tree");
        assert_eq!(config.tool_confirm.rules[0].action, "auto");

        let serialized = serde_yaml::to_string(&config).unwrap();
        assert!(serialized.contains("match:"));
        assert!(!serialized.contains("match_pattern:"));
    }

    #[test]
    fn test_mode_override_apply() {
        let base = ModeConfig {
            schema_version: 1,
            id: "base".to_string(),
            title: "Base".to_string(),
            description: "".to_string(),
            specific: false,
            prompt: "Base prompt".to_string(),
            tools: vec!["tree".to_string(), "cat".to_string()],
            model_defaults: ModeModelDefaults {
                default: Some(ModelTypeConfig {
                    max_new_tokens: Some(1000),
                    temperature: Some(0.5),
                    ..Default::default()
                }),
                ..Default::default()
            },
            tool_confirm: ToolConfirmConfig::default(),
            thread_defaults: ModeThreadDefaults::default(),
            ui: ModeUi::default(),
            base: None,
            match_models: None,
            override_config: None,
            allow_integrations: false,
            allow_mcp: false,
            allow_subagents: false,
        };

        let override_cfg = ModeOverride {
            prompt: Some("Override prompt".to_string()),
            tools_add: Some(vec!["shell".to_string()]),
            model_defaults: Some(ModeModelDefaults {
                default: Some(ModelTypeConfig {
                    temperature: Some(0.8),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = base.apply_override(&override_cfg);
        assert_eq!(result.prompt, "Override prompt");
        assert_eq!(result.tools, vec!["tree", "cat", "shell"]);
        assert_eq!(
            result
                .model_defaults
                .default
                .as_ref()
                .unwrap()
                .max_new_tokens,
            Some(1000)
        );
        assert_eq!(
            result.model_defaults.default.as_ref().unwrap().temperature,
            Some(0.8)
        );
    }

    #[test]
    fn test_mode_override_tools_replace() {
        let base = ModeConfig {
            schema_version: 1,
            id: "base".to_string(),
            title: "".to_string(),
            description: "".to_string(),
            specific: false,
            prompt: "".to_string(),
            tools: vec!["tree".to_string(), "cat".to_string()],
            model_defaults: ModeModelDefaults::default(),
            tool_confirm: ToolConfirmConfig::default(),
            thread_defaults: ModeThreadDefaults::default(),
            ui: ModeUi::default(),
            base: None,
            match_models: None,
            override_config: None,
            allow_integrations: false,
            allow_mcp: false,
            allow_subagents: false,
        };

        let override_cfg = ModeOverride {
            tools_replace: Some(vec!["shell".to_string()]),
            ..Default::default()
        };

        let result = base.apply_override(&override_cfg);
        assert_eq!(result.tools, vec!["shell"]);
    }

    #[test]
    fn test_mode_override_tools_remove() {
        let base = ModeConfig {
            schema_version: 1,
            id: "base".to_string(),
            title: "".to_string(),
            description: "".to_string(),
            specific: false,
            prompt: "".to_string(),
            tools: vec!["tree".to_string(), "cat".to_string(), "shell".to_string()],
            model_defaults: ModeModelDefaults::default(),
            tool_confirm: ToolConfirmConfig::default(),
            thread_defaults: ModeThreadDefaults::default(),
            ui: ModeUi::default(),
            base: None,
            match_models: None,
            override_config: None,
            allow_integrations: false,
            allow_mcp: false,
            allow_subagents: false,
        };

        let override_cfg = ModeOverride {
            tools_remove: Some(vec!["cat".to_string()]),
            ..Default::default()
        };

        let result = base.apply_override(&override_cfg);
        assert_eq!(result.tools, vec!["tree", "shell"]);
    }

    #[test]
    fn test_subagent_config_extra_fields_preserved() {
        let yaml = r#"
schema_version: 1
id: test_subagent
title: Test
expose_as_tool: true
has_code: false
subchat:
  context_mode: bare
custom_field: custom_value
another_extra: 123
"#;
        let config: SubagentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.id, "test_subagent");
        assert!(config.extra.contains_key("custom_field"));
        assert!(config.extra.contains_key("another_extra"));

        let serialized = serde_yaml::to_string(&config).unwrap();
        assert!(serialized.contains("custom_field:"));
        assert!(serialized.contains("another_extra:"));
    }

    #[test]
    fn test_subagent_apply_override_preserves_extra() {
        let base = SubagentConfig {
            schema_version: 1,
            id: "base".to_string(),
            title: "Base".to_string(),
            description: "".to_string(),
            specific: false,
            expose_as_tool: false,
            has_code: false,
            tool: None,
            subchat: SubchatConfig::default(),
            messages: SubagentMessages::default(),
            prompts: SubagentPrompts::default(),
            gather_files: GatherFilesConfig::default(),
            tools: vec![],
            base: None,
            match_models: None,
            extra: {
                let mut m = HashMap::new();
                m.insert(
                    "base_extra".to_string(),
                    serde_yaml::Value::String("value".to_string()),
                );
                m
            },
        };

        let override_cfg = SubagentConfig {
            schema_version: 1,
            id: "override".to_string(),
            title: "Override".to_string(),
            description: "".to_string(),
            specific: false,
            expose_as_tool: true,
            has_code: false,
            tool: None,
            subchat: SubchatConfig::default(),
            messages: SubagentMessages::default(),
            prompts: SubagentPrompts::default(),
            gather_files: GatherFilesConfig::default(),
            tools: vec![],
            base: Some("base".to_string()),
            match_models: Some(vec!["gpt-*".to_string()]),
            extra: {
                let mut m = HashMap::new();
                m.insert(
                    "override_extra".to_string(),
                    serde_yaml::Value::String("new".to_string()),
                );
                m
            },
        };

        let result = base.apply_override(&override_cfg);
        assert_eq!(result.title, "Override");
        assert!(result.expose_as_tool);
        assert!(result.extra.contains_key("base_extra"));
        assert!(result.extra.contains_key("override_extra"));
    }

    #[test]
    fn test_toolbox_command_selection_needed() {
        let yaml = r#"
schema_version: 1
id: test_cmd
description: Test command
selection_needed: [1, 100]
messages: []
"#;
        let config: ToolboxCommandConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.selection_needed, Some((1, 100)));

        let json = serde_json::to_string(&config).unwrap();
        let parsed: ToolboxCommandConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.selection_needed, Some((1, 100)));
    }

    #[test]
    fn test_code_lens_config() {
        let yaml = r#"
schema_version: 1
id: test_lens
label: Test Lens
auto_submit: true
new_tab: false
messages:
  - role: user
    content: "Test message"
"#;
        let config: CodeLensConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.id, "test_lens");
        assert_eq!(config.label, "Test Lens");
        assert!(config.auto_submit);
        assert!(!config.new_tab);
        assert_eq!(config.messages.len(), 1);
        assert_eq!(config.messages[0].role, "user");
    }
}

#[cfg(test)]
mod ui_defaults_tests {
    use super::*;

    #[test]
    fn test_ui_default_mode_config() {
        let json = r#"{"schema_version":1,"id":"test_mode","title":"test_mode","description":"","specific":false,"prompt":"","tools":[]}"#;
        let result: Result<ModeConfig, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "mode default: {:?}", result.err());
    }

    #[test]
    fn test_ui_default_subagent_config() {
        let json = r#"{"schema_version":1,"id":"test_sub","title":"test_sub","description":"","specific":false,"expose_as_tool":true,"has_code":false,"subchat":{"context_mode":"bare"},"messages":{}}"#;
        let result: Result<SubagentConfig, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "subagent default: {:?}", result.err());
    }

    #[test]
    fn test_ui_default_toolbox_command_config() {
        let json = r#"{"schema_version":1,"id":"test_cmd","description":"","messages":[]}"#;
        let result: Result<ToolboxCommandConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "toolbox_command default: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_ui_default_code_lens_config() {
        let json = r#"{"schema_version":1,"id":"test_lens","label":"test_lens","auto_submit":false,"new_tab":false,"messages":[]}"#;
        let result: Result<CodeLensConfig, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "code_lens default: {:?}", result.err());
    }

    #[test]
    fn test_ui_toolbox_selection_needed_array() {
        // UI sends selection_needed as [1, 10000] - test if tuple deserializes from array
        let json = r#"{"schema_version":1,"id":"test_cmd","description":"","selection_needed":[1,10000],"messages":[]}"#;
        let result: Result<ToolboxCommandConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "toolbox selection_needed [1,10000]: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().selection_needed, Some((1, 10000)));
    }
}
