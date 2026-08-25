use std::collections::HashMap;

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelSettingsCapabilities {
    reasoning_effort_options: Option<Vec<String>>,
    supports_thinking_budget: Option<bool>,
    supports_adaptive_thinking_budget: Option<bool>,
    max_thinking_tokens: Option<u64>,
    supports_temperature: Option<bool>,
    supports_parallel_tools: Option<bool>,
    chatgpt_backend: bool,
}

impl ModelSettingsCapabilities {
    fn supports_reasoning(&self) -> bool {
        self.reasoning_effort_options
            .as_ref()
            .is_some_and(|options| !options.is_empty())
            || self.supports_thinking_budget == Some(true)
            || self.supports_adaptive_thinking_budget == Some(true)
    }

    fn supports_thinking_budget(&self) -> bool {
        self.supports_thinking_budget == Some(true)
            || self.supports_adaptive_thinking_budget == Some(true)
    }

    fn merge_from(&mut self, other: &Self) {
        if other.reasoning_effort_options.is_some() {
            self.reasoning_effort_options = other.reasoning_effort_options.clone();
        }
        if other.supports_thinking_budget.is_some() {
            self.supports_thinking_budget = other.supports_thinking_budget;
        }
        if other.supports_adaptive_thinking_budget.is_some() {
            self.supports_adaptive_thinking_budget = other.supports_adaptive_thinking_budget;
        }
        if other.max_thinking_tokens.is_some() {
            self.max_thinking_tokens = other.max_thinking_tokens;
        }
        if other.supports_temperature.is_some() {
            self.supports_temperature = other.supports_temperature;
        }
        if other.supports_parallel_tools.is_some() {
            self.supports_parallel_tools = other.supports_parallel_tools;
        }
        self.chatgpt_backend |= other.chatgpt_backend;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingControl {
    ThinkingBudget,
    Temperature,
    FrequencyPenalty,
    MaxTokens,
    ParallelToolCalls,
    ToolUse,
    ContextTokensCap,
    AutoCompressionCap,
    IncludeProjectInfo,
    CheckpointsEnabled,
    AutoEnrichmentEnabled,
    AutoApproveEditingTools,
    AutoApproveDangerousCommands,
    TopP,
    Stop,
}

impl SettingControl {
    fn key(self) -> &'static str {
        match self {
            Self::ThinkingBudget => "thinking_budget",
            Self::Temperature => "temperature",
            Self::FrequencyPenalty => "frequency_penalty",
            Self::MaxTokens => "max_tokens",
            Self::ParallelToolCalls => "parallel_tool_calls",
            Self::ToolUse => "tool_use",
            Self::ContextTokensCap => "context_tokens_cap",
            Self::AutoCompressionCap => "auto_compression_cap",
            Self::IncludeProjectInfo => "include_project_info",
            Self::CheckpointsEnabled => "checkpoints_enabled",
            Self::AutoEnrichmentEnabled => "auto_enrichment_enabled",
            Self::AutoApproveEditingTools => "auto_approve_editing_tools",
            Self::AutoApproveDangerousCommands => "auto_approve_dangerous_commands",
            Self::TopP => "top_p",
            Self::Stop => "stop",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::ThinkingBudget => "thinking budget",
            Self::Temperature => "temperature",
            Self::FrequencyPenalty => "frequency penalty",
            Self::MaxTokens => "max tokens",
            Self::ParallelToolCalls => "parallel tool calls",
            Self::ToolUse => "tool use",
            Self::ContextTokensCap => "context token cap",
            Self::AutoCompressionCap => "auto-compression cap",
            Self::IncludeProjectInfo => "include project info",
            Self::CheckpointsEnabled => "checkpoints enabled",
            Self::AutoEnrichmentEnabled => "auto enrichment",
            Self::AutoApproveEditingTools => "! auto-approve editing tools",
            Self::AutoApproveDangerousCommands => "! auto-approve dangerous commands",
            Self::TopP => "top p",
            Self::Stop => "stop sequences",
        }
    }

    fn section(self) -> &'static str {
        match self {
            Self::ThinkingBudget
            | Self::Temperature
            | Self::FrequencyPenalty
            | Self::MaxTokens
            | Self::ParallelToolCalls
            | Self::ToolUse
            | Self::TopP
            | Self::Stop => "Model controls",
            Self::ContextTokensCap
            | Self::AutoCompressionCap
            | Self::IncludeProjectInfo
            | Self::CheckpointsEnabled
            | Self::AutoEnrichmentEnabled => "Chat controls",
            Self::AutoApproveEditingTools | Self::AutoApproveDangerousCommands => {
                "Dangerous per-chat toggles"
            }
        }
    }

    fn is_boolean(self) -> bool {
        matches!(
            self,
            Self::ParallelToolCalls
                | Self::IncludeProjectInfo
                | Self::CheckpointsEnabled
                | Self::AutoEnrichmentEnabled
                | Self::AutoApproveEditingTools
                | Self::AutoApproveDangerousCommands
        )
    }

    fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::ThinkingBudget
                | Self::Temperature
                | Self::FrequencyPenalty
                | Self::MaxTokens
                | Self::ContextTokensCap
                | Self::AutoCompressionCap
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsRow {
    pub(crate) section: &'static str,
    pub(crate) key: &'static str,
    pub(crate) title: &'static str,
    pub(crate) value: String,
    pub(crate) unavailable_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SettingsState {
    controls: Vec<SettingControl>,
    params: Map<String, Value>,
    caps: ModelSettingsCapabilities,
    selected: usize,
    editing: bool,
    draft: String,
}

impl SettingsState {
    pub(crate) fn new(params: &Value, caps: ModelSettingsCapabilities) -> Self {
        Self {
            controls: vec![
                SettingControl::ThinkingBudget,
                SettingControl::Temperature,
                SettingControl::FrequencyPenalty,
                SettingControl::MaxTokens,
                SettingControl::ParallelToolCalls,
                SettingControl::ToolUse,
                SettingControl::TopP,
                SettingControl::Stop,
                SettingControl::ContextTokensCap,
                SettingControl::AutoCompressionCap,
                SettingControl::IncludeProjectInfo,
                SettingControl::CheckpointsEnabled,
                SettingControl::AutoEnrichmentEnabled,
                SettingControl::AutoApproveEditingTools,
                SettingControl::AutoApproveDangerousCommands,
            ],
            params: params.as_object().cloned().unwrap_or_default(),
            caps,
            selected: 0,
            editing: false,
            draft: String::new(),
        }
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn rows(&self) -> Vec<SettingsRow> {
        self.controls
            .iter()
            .copied()
            .map(|control| SettingsRow {
                section: control.section(),
                key: control.key(),
                title: control.title(),
                value: self.display_value(control),
                unavailable_reason: self.unavailable_reason(control),
            })
            .collect()
    }

    pub(crate) fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.clear_draft();
    }

    pub(crate) fn select_next(&mut self) {
        self.selected = (self.selected + 1).min(self.controls.len().saturating_sub(1));
        self.clear_draft();
    }

    pub(crate) fn select_first(&mut self) {
        self.selected = 0;
        self.clear_draft();
    }

    pub(crate) fn select_last(&mut self) {
        self.selected = self.controls.len().saturating_sub(1);
        self.clear_draft();
    }

    pub(crate) fn push_text(&mut self, text: &str) {
        let Some(control) = self.controls.get(self.selected).copied() else {
            return;
        };
        if control.is_numeric() && self.unavailable_reason(control).is_none() {
            if !self.editing {
                self.draft.clear();
                self.editing = true;
            }
            self.draft.push_str(text);
        }
    }

    pub(crate) fn pop_text(&mut self) {
        if self.editing {
            self.draft.pop();
        }
    }

    pub(crate) fn activate(&mut self) -> Option<Value> {
        let control = *self.controls.get(self.selected)?;
        if self.unavailable_reason(control).is_some() {
            return None;
        }
        let value = if control.is_boolean() {
            Value::Bool(!self.boolean_value(control))
        } else if control == SettingControl::ToolUse {
            Value::String(next_tool_use(self.string_value(control)))
        } else if control.is_numeric() {
            if !self.editing {
                self.editing = true;
                self.draft.clear();
                return None;
            }
            parse_numeric_value(control, &self.draft)?
        } else {
            return None;
        };
        self.params.insert(control.key().to_string(), value.clone());
        self.clear_draft();
        Some(json!({control.key(): value}))
    }

    pub(crate) fn refresh(&mut self, params: &Value, caps: ModelSettingsCapabilities) {
        self.params = params.as_object().cloned().unwrap_or_default();
        self.caps = caps;
        self.selected = self.selected.min(self.controls.len().saturating_sub(1));
        self.clear_draft();
    }

    fn display_value(&self, control: SettingControl) -> String {
        if self.editing && self.controls.get(self.selected) == Some(&control) {
            return format!("draft {}", self.draft);
        }
        if control.is_boolean() {
            return if self.boolean_value(control) {
                "ON".to_string()
            } else {
                "OFF".to_string()
            };
        }
        match self.params.get(control.key()) {
            Some(Value::String(value)) if !value.is_empty() => value.clone(),
            Some(Value::Number(value)) => value.to_string(),
            Some(Value::Bool(value)) => value.to_string(),
            _ => "default".to_string(),
        }
    }

    fn unavailable_reason(&self, control: SettingControl) -> Option<String> {
        if self.caps.chatgpt_backend
            && matches!(
                control,
                SettingControl::Temperature
                    | SettingControl::MaxTokens
                    | SettingControl::TopP
                    | SettingControl::FrequencyPenalty
                    | SettingControl::Stop
            )
        {
            return Some(chatgpt_rejection_reason(control).to_string());
        }
        match control {
            SettingControl::ThinkingBudget if !self.caps.supports_thinking_budget() => {
                Some("unavailable: this model does not support thinking budgets".to_string())
            }
            SettingControl::Temperature if self.caps.supports_reasoning() => {
                Some("unavailable: this model drops it when reasoning is on".to_string())
            }
            SettingControl::Temperature if self.caps.supports_temperature == Some(false) => {
                Some("unavailable: this model does not support temperature".to_string())
            }
            SettingControl::ParallelToolCalls
                if self.caps.supports_parallel_tools == Some(false) =>
            {
                Some("unavailable: this model does not support parallel tools".to_string())
            }
            SettingControl::TopP | SettingControl::Stop => {
                Some("unavailable: this TUI cannot patch this thread parameter".to_string())
            }
            _ => None,
        }
    }

    fn boolean_value(&self, control: SettingControl) -> bool {
        self.params
            .get(control.key())
            .and_then(Value::as_bool)
            .unwrap_or_else(|| match control {
                SettingControl::IncludeProjectInfo | SettingControl::CheckpointsEnabled => true,
                _ => false,
            })
    }

    fn string_value(&self, control: SettingControl) -> &str {
        self.params
            .get(control.key())
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("agent")
    }

    fn clear_draft(&mut self) {
        self.editing = false;
        self.draft.clear();
    }
}

pub(crate) fn model_settings_caps(caps: &Value) -> HashMap<String, ModelSettingsCapabilities> {
    let mut result = HashMap::new();
    for models in [
        caps.get("available_models"),
        caps.get("models").and_then(|models| models.get("chat")),
        caps.get("chat_models"),
    ] {
        collect_model_settings_caps(models, &mut result);
    }
    result
}

pub(crate) fn settings_caps_for_model(
    caps: &HashMap<String, ModelSettingsCapabilities>,
    model: Option<&str>,
) -> ModelSettingsCapabilities {
    let Some(model) = model.filter(|model| !model.trim().is_empty()) else {
        return ModelSettingsCapabilities::default();
    };
    caps.get(model)
        .cloned()
        .or_else(|| {
            let mut matches = caps
                .iter()
                .filter(|(id, _)| id.rsplit('/').next().is_some_and(|suffix| suffix == model));
            let (_, matched) = matches.next()?;
            matches.next().is_none().then(|| matched.clone())
        })
        .unwrap_or_default()
}

fn collect_model_settings_caps(
    models: Option<&Value>,
    result: &mut HashMap<String, ModelSettingsCapabilities>,
) {
    match models {
        Some(Value::Object(models)) => {
            for (id, model) in models {
                insert_model_settings_caps(result, id, model);
            }
        }
        Some(Value::Array(models)) => {
            for model in models {
                if let Some(id) = model.get("id").and_then(Value::as_str) {
                    insert_model_settings_caps(result, id, model);
                }
            }
        }
        _ => {}
    }
}

fn insert_model_settings_caps(
    result: &mut HashMap<String, ModelSettingsCapabilities>,
    id: &str,
    model: &Value,
) {
    let caps = ModelSettingsCapabilities {
        reasoning_effort_options: model
            .get("reasoning_effort_options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            }),
        supports_thinking_budget: optional_bool(model, "supports_thinking_budget"),
        supports_adaptive_thinking_budget: optional_bool(
            model,
            "supports_adaptive_thinking_budget",
        ),
        max_thinking_tokens: model.get("max_thinking_tokens").and_then(Value::as_u64),
        supports_temperature: optional_bool(model, "supports_temperature"),
        supports_parallel_tools: optional_bool(model, "supports_parallel_tools"),
        chatgpt_backend: model
            .get("chatgpt_backend")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || model
                .get("endpoint")
                .and_then(Value::as_str)
                .is_some_and(|endpoint| endpoint.contains("chatgpt.com/backend-api"))
            || model
                .get("selected_provider")
                .and_then(Value::as_str)
                .is_some_and(is_openai_codex_provider),
    };
    merge_caps(result, id, &caps);
    if let Some(model_id) = model.get("id").and_then(Value::as_str) {
        merge_caps(result, model_id, &caps);
    }
}

fn optional_bool(model: &Value, key: &str) -> Option<bool> {
    model.get(key).and_then(Value::as_bool)
}

fn is_openai_codex_provider(provider: &str) -> bool {
    provider
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .starts_with("openai_codex")
}

fn merge_caps(
    result: &mut HashMap<String, ModelSettingsCapabilities>,
    id: &str,
    caps: &ModelSettingsCapabilities,
) {
    if id.trim().is_empty() {
        return;
    }
    result.entry(id.to_string()).or_default().merge_from(caps);
}

fn next_tool_use(current: &str) -> String {
    match current {
        "agent" => "explore".to_string(),
        "explore" => "disabled".to_string(),
        _ => "agent".to_string(),
    }
}

fn chatgpt_rejection_reason(control: SettingControl) -> &'static str {
    match control {
        SettingControl::Temperature => "unavailable: ChatGPT backend rejects temperature",
        SettingControl::MaxTokens => "unavailable: ChatGPT backend rejects max tokens",
        SettingControl::TopP => "unavailable: ChatGPT backend rejects top p",
        SettingControl::FrequencyPenalty => {
            "unavailable: ChatGPT backend rejects frequency penalty"
        }
        SettingControl::Stop => "unavailable: ChatGPT backend rejects stop sequences",
        _ => "unavailable: ChatGPT backend rejects this parameter",
    }
}

fn parse_numeric_value(control: SettingControl, draft: &str) -> Option<Value> {
    let draft = draft.trim();
    if draft.is_empty() || draft.eq_ignore_ascii_case("default") {
        return Some(Value::Null);
    }
    match control {
        SettingControl::Temperature | SettingControl::FrequencyPenalty => {
            draft.parse::<f64>().ok().map(|value| json!(value))
        }
        _ => draft.parse::<u64>().ok().map(|value| json!(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_caps() -> Value {
        json!({
            "chat_models": {
                "model": {
                    "supports_thinking_budget": true,
                    "max_thinking_tokens": 32000,
                    "supports_parallel_tools": true,
                    "supports_temperature": true,
                },
            },
        })
    }

    fn settings() -> SettingsState {
        SettingsState::new(
            &json!({"model": "model", "tool_use": "agent"}),
            ModelSettingsCapabilities {
                supports_parallel_tools: Some(true),
                supports_temperature: Some(true),
                ..Default::default()
            },
        )
    }

    fn activate_numeric(state: &mut SettingsState, value: &str) -> Value {
        state.push_text(value);
        state.activate().expect("numeric patch")
    }

    #[test]
    fn all_settable_thread_params_dispatch_their_own_patch() {
        let mut state = settings();
        let expected = [
            ("temperature", "1.25"),
            ("frequency_penalty", "0.5"),
            ("max_tokens", "2000"),
        ];
        for (key, value) in expected {
            let index = state.rows().iter().position(|row| row.key == key).unwrap();
            state.selected = index;
            assert_eq!(
                activate_numeric(&mut state, value)[key].as_f64(),
                Some(value.parse::<f64>().unwrap())
            );
        }

        let caps = model_settings_caps(&all_caps());
        state = SettingsState::new(
            &json!({"model": "model", "tool_use": "agent"}),
            caps["model"].clone(),
        );
        let index = state
            .rows()
            .iter()
            .position(|row| row.key == "thinking_budget")
            .unwrap();
        state.selected = index;
        assert_eq!(
            activate_numeric(&mut state, "1000")["thinking_budget"],
            1000
        );

        let expected = [
            "parallel_tool_calls",
            "tool_use",
            "context_tokens_cap",
            "auto_compression_cap",
            "include_project_info",
            "checkpoints_enabled",
            "auto_enrichment_enabled",
        ];
        for key in expected {
            let index = state.rows().iter().position(|row| row.key == key).unwrap();
            state.selected = index;
            let patch = if matches!(key, "context_tokens_cap" | "auto_compression_cap") {
                activate_numeric(&mut state, "4096")
            } else {
                state.activate().expect("patch")
            };
            assert!(patch.get(key).is_some(), "{key}");
            assert_eq!(patch.as_object().unwrap().len(), 1, "{key}");
        }
    }

    #[test]
    fn reasoning_model_keeps_temperature_visible_with_reason() {
        let caps = model_settings_caps(&json!({
            "chat_models": {"reasoning": {"reasoning_effort_options": ["low"]}},
        }));
        let state = SettingsState::new(&json!({"model": "reasoning"}), caps["reasoning"].clone());
        let temperature = state
            .rows()
            .into_iter()
            .find(|row| row.key == "temperature")
            .unwrap();

        assert_eq!(
            temperature.unavailable_reason.as_deref(),
            Some("unavailable: this model drops it when reasoning is on")
        );
    }

    #[test]
    fn chatgpt_backend_explains_every_rejected_parameter() {
        let caps = model_settings_caps(&json!({
            "chat_models": {"codex": {"selected_provider": "openai_codex"}},
        }));
        let state = SettingsState::new(&json!({"model": "codex"}), caps["codex"].clone());

        for key in [
            "temperature",
            "max_tokens",
            "top_p",
            "frequency_penalty",
            "stop",
        ] {
            let row = state.rows().into_iter().find(|row| row.key == key).unwrap();
            assert!(
                row.unavailable_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("ChatGPT backend rejects")),
                "{key}"
            );
        }
    }

    #[test]
    fn dangerous_toggles_render_explicit_off_labels() {
        let state = settings();
        let rows = state.rows();
        for key in [
            "auto_approve_editing_tools",
            "auto_approve_dangerous_commands",
        ] {
            let row = rows.iter().find(|row| row.key == key).unwrap();
            assert_eq!(row.section, "Dangerous per-chat toggles");
            assert!(row.title.starts_with("! "));
            assert_eq!(row.value, "OFF");
        }
    }
}
