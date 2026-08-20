use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;

use crate::call_validation::ChatMessage;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::json_utils::extract_json_object;
use crate::subchat::{run_subchat, run_subchat_once, SubchatConfig, ToolsPolicy, TraceParent};
use crate::tools::shell_gate::ShellLlmValidation;

#[derive(Clone, Debug, Deserialize)]
pub struct LlmVerdict {
    pub ask: bool,
    pub reason: String,
}

static VERDICT_CACHE: OnceLock<Mutex<HashMap<String, LlmVerdict>>> = OnceLock::new();

pub async fn validate_command(
    gcx: Arc<GlobalContext>,
    command: &str,
    chat_id: &str,
    cfg: &ShellLlmValidation,
) -> Option<LlmVerdict> {
    let cache_key = format!("{chat_id}:{command}");
    if cfg.cache_per_chat {
        let cache = VERDICT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(cache) = cache.lock() {
            if let Some(verdict) = cache.get(&cache_key) {
                return Some(verdict.clone());
            }
        } else {
            return None;
        }
    }

    let command = command.to_string();
    let cfg = cfg.clone();
    let cache_per_chat = cfg.cache_per_chat;
    let verdict = tokio::time::timeout(
        Duration::from_secs(cfg.timeout_secs),
        validate_command_inner(gcx, command, chat_id, cfg),
    )
    .await
    .ok()
    .flatten()?;

    if cache_per_chat {
        let cache = VERDICT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut cache = cache.lock().ok()?;
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(cache_key, verdict.clone());
    }
    Some(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_ignores_unknown_fields() {
        let verdict: LlmVerdict =
            serde_json::from_str(r#"{"ask":true,"reason":"Review this","confidence":0.9}"#)
                .unwrap();
        assert!(verdict.ask);
        assert_eq!(verdict.reason, "Review this");
    }
}

async fn validate_command_inner(
    gcx: Arc<GlobalContext>,
    command: String,
    chat_id: &str,
    cfg: ShellLlmValidation,
) -> Option<LlmVerdict> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .ok()?;
    let light_model = caps.defaults.chat_light_model.clone();
    let default_model = caps.defaults.chat_default_model.clone();
    let model = if !cfg.model.trim().is_empty() {
        cfg.model.trim().to_string()
    } else if !light_model.is_empty() {
        light_model.clone()
    } else if !default_model.is_empty() {
        default_model
    } else {
        return None;
    };

    let subagent = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx.clone(),
        "shell_command_validator",
        Some(&model),
    )
    .await?;
    let system_prompt = subagent.messages.system_prompt?;
    let messages = vec![ChatMessage::new(
        "user".to_string(),
        format!("{system_prompt}\n\nCommand:\n```shell\n{command}\n```"),
    )];

    let result = if cfg.model.trim().is_empty() && model == light_model {
        run_subchat_once(
            gcx,
            "shell_command_validator",
            messages,
            TraceParent::chat(chat_id),
        )
        .await
        .ok()?
    } else {
        let config = SubchatConfig {
            tool_name: "shell_command_validator".to_string(),
            stateful: false,
            autonomous_no_confirm: true,
            chat_id: None,
            title: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            tools: ToolsPolicy::None,
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: None,
            worktree: None,
            model,
            mode: "agent".to_string(),
            n_ctx: 8192,
            max_new_tokens: 128,
            temperature: Some(0.0),
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: TraceParent::chat(chat_id),
        };
        run_subchat(gcx, messages, config).await.ok()?
    };
    let answer = result
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())?;
    let mut verdict: LlmVerdict = extract_json_object(&answer).ok()?;
    verdict.reason = verdict.reason.trim().to_string();
    if verdict.reason.is_empty() {
        return None;
    }
    Some(verdict)
}
