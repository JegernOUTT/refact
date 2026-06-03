use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::TaskMeta;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::tasks::storage;

pub(crate) fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

#[allow(dead_code)]
pub(crate) fn optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let take = max.saturating_sub(1);
    format!("{}…", s.chars().take(take).collect::<String>())
}

pub(crate) fn human_age_at(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(ts).num_seconds().max(0);
    if seconds == 0 {
        "now".to_string()
    } else if seconds < 60 {
        format!("{}s ago", seconds)
    } else if seconds < 60 * 60 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h ago", seconds / (60 * 60))
    } else {
        format!("{}d ago", seconds / (60 * 60 * 24))
    }
}

pub(crate) fn human_age(ts: DateTime<Utc>) -> String {
    human_age_at(ts, Utc::now())
}

pub(crate) async fn require_bound_planner_task(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    let requested_task_id = match args.get("task_id") {
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| "task_id must be a string".to_string())?
                .trim()
                .to_string(),
        ),
        None => None,
    };

    let ccx_lock = ccx.lock().await;
    if let Some(meta) = &ccx_lock.task_meta {
        if meta.role != "planner" {
            return Err(
                "task observability tools can only be called by the task planner.".to_string(),
            );
        }
        if requested_task_id
            .as_deref()
            .is_some_and(|task_id| task_id != meta.task_id)
        {
            return Err("task_id override is not allowed from this planner chat".to_string());
        }
        return Ok(meta.task_id.clone());
    }

    let inferred_task_id = storage::infer_task_id_from_chat_id(&ccx_lock.chat_id);
    match (requested_task_id, inferred_task_id) {
        (Some(requested), Some(inferred)) if requested == inferred => Ok(inferred),
        (Some(_), _) => Err("task_id override is not allowed from this planner chat".to_string()),
        (None, Some(inferred)) => Ok(inferred),
        (None, None) => Err("Missing 'task_id' (and chat is not bound to a task)".to_string()),
    }
}

#[allow(dead_code)]
pub(crate) async fn require_agent_task_meta(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> Result<TaskMeta, String> {
    let ccx_lock = ccx.lock().await;
    let meta = ccx_lock
        .task_meta
        .clone()
        .ok_or_else(|| "task agent context is required".to_string())?;
    if !matches!(meta.role.as_str(), "agent" | "agents") {
        return Err("task agent context is required".to_string());
    }
    Ok(meta)
}

pub(crate) async fn preflight_agent_model(
    gcx: Arc<GlobalContext>,
    model_name: &str,
) -> Result<(), String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| format!("Cannot spawn agent: failed to check model availability: {e}"))?;
    if caps.chat_models.contains_key(model_name) {
        return Ok(());
    }
    let available: Vec<&str> = caps
        .chat_models
        .keys()
        .map(|s| s.as_str())
        .take(5)
        .collect();
    let alternatives = if available.is_empty() {
        "no chat models configured".to_string()
    } else {
        available.join(", ")
    };
    Err(format!(
        "Cannot spawn agent: model '{model_name}' is not configured for chat. \
        Available chat models: {alternatives}. \
        Configure `task_planner_agent_model` in default models or pass a different model."
    ))
}

pub(crate) async fn wait_for_agent_abort(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return Ok(());
    };
    let processor_running = {
        let mut session = session_arc.lock().await;
        session.abort_stream();
        session.close_event_channel();
        session.queue_notify.notify_waiters();
        session.queue_processor_running.clone()
    };

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let state = {
            let session = session_arc.lock().await;
            session.runtime.state
        };
        let state_stopped = !matches!(
            state,
            refact_runtime_api::SessionState::Generating
                | refact_runtime_api::SessionState::ExecutingTools
                | refact_runtime_api::SessionState::Paused
                | refact_runtime_api::SessionState::WaitingIde
                | refact_runtime_api::SessionState::WaitingUserInput
        );
        if state_stopped && !processor_running.load(Ordering::SeqCst) {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for agent {} to stop after abort",
                chat_id
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

pub(crate) const AGENT_ABORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use serde_json::json;

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    async fn ccx_with_meta(
        meta: Option<TaskMeta>,
        chat_id: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                meta,
                None,
            )
            .await,
        ))
    }

    fn task_meta(role: &str) -> TaskMeta {
        TaskMeta {
            task_id: "task-1".to_string(),
            role: role.to_string(),
            agent_id: (role == "agents").then(|| "agent-1".to_string()),
            card_id: (role == "agents").then(|| "T-1".to_string()),
            planner_chat_id: Some("planner-chat".to_string()),
        }
    }

    #[test]
    fn task_tool_helpers_parse_required_and_optional_strings() {
        let args = args(&[
            ("name", json!("  card-1  ")),
            ("empty", json!("   ")),
            ("null", Value::Null),
        ]);

        assert_eq!(required_string(&args, "name").unwrap(), "card-1");
        assert_eq!(optional_string(&args, "name"), Some("card-1".to_string()));
        assert_eq!(optional_string(&args, "empty"), None);
        assert_eq!(optional_string(&args, "missing"), None);
        assert_eq!(
            required_string(&args, "empty").unwrap_err(),
            "Missing 'empty'"
        );
        assert_eq!(
            required_string(&args, "null").unwrap_err(),
            "Missing 'null'"
        );
    }

    #[test]
    fn task_tool_helpers_truncate_chars_is_unicode_safe() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("ab😀cd", 4), "ab😀…");
        assert_eq!(truncate_chars("abc", 0), "");
    }

    #[test]
    fn task_tool_helpers_human_age_clamps_future_timestamps() {
        assert_eq!(human_age(Utc::now() + chrono::Duration::seconds(5)), "now");
    }

    #[test]
    fn human_age_at_is_deterministic() {
        let now = Utc::now();
        assert_eq!(
            human_age_at(now - chrono::Duration::seconds(30), now),
            "30s ago"
        );
        assert_eq!(
            human_age_at(now - chrono::Duration::seconds(30), now),
            "30s ago"
        );
        assert_eq!(
            human_age_at(now - chrono::Duration::minutes(5), now),
            "5m ago"
        );
        assert_eq!(
            human_age_at(now - chrono::Duration::hours(2), now),
            "2h ago"
        );
        assert_eq!(human_age_at(now - chrono::Duration::days(3), now), "3d ago");
        assert_eq!(
            human_age_at(now + chrono::Duration::seconds(10), now),
            "now"
        );
    }

    #[tokio::test]
    async fn task_tool_helpers_require_bound_planner_task_accepts_bound_task() {
        let ccx = ccx_with_meta(Some(task_meta("planner")), "planner-chat").await;

        let task_id = require_bound_planner_task(&ccx, &args(&[("task_id", json!("task-1"))]))
            .await
            .unwrap();

        assert_eq!(task_id, "task-1");
    }

    #[tokio::test]
    async fn task_tool_helpers_require_bound_planner_task_rejects_override() {
        let ccx = ccx_with_meta(Some(task_meta("planner")), "planner-chat").await;

        let err = require_bound_planner_task(&ccx, &args(&[("task_id", json!("task-2"))]))
            .await
            .unwrap_err();

        assert_eq!(
            err,
            "task_id override is not allowed from this planner chat"
        );
    }

    #[tokio::test]
    async fn task_tool_helpers_require_bound_planner_task_requires_planner_role() {
        let ccx = ccx_with_meta(Some(task_meta("agents")), "agent-chat").await;

        let err = require_bound_planner_task(&ccx, &HashMap::new())
            .await
            .unwrap_err();

        assert!(err.contains("task planner"));
    }

    #[tokio::test]
    async fn task_tool_helpers_require_agent_task_meta_requires_agent_role() {
        let ccx = ccx_with_meta(Some(task_meta("agents")), "agent-chat").await;
        let meta = require_agent_task_meta(&ccx).await.unwrap();
        assert_eq!(meta.task_id, "task-1");
        assert_eq!(meta.card_id.as_deref(), Some("T-1"));

        let planner_ccx = ccx_with_meta(Some(task_meta("planner")), "planner-chat").await;
        let err = require_agent_task_meta(&planner_ccx).await.unwrap_err();
        assert_eq!(err, "task agent context is required");
    }

    async fn gcx_with_chat_models(
        model_names: &[&str],
    ) -> Arc<crate::global_context::GlobalContext> {
        use crate::caps::{ChatModelRecord, CodeAssistantCaps};
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut caps = CodeAssistantCaps::default();
        for &name in model_names {
            caps.chat_models
                .insert(name.to_string(), Arc::new(ChatModelRecord::default()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = now;
        }
        gcx
    }

    #[tokio::test]
    async fn preflight_passes_for_configured_chat_model() {
        let gcx = gcx_with_chat_models(&["provider/test-model"]).await;
        preflight_agent_model(gcx, "provider/test-model")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn preflight_fails_with_helpful_message_for_unknown_model() {
        let gcx = gcx_with_chat_models(&["provider/available-model"]).await;
        let err = preflight_agent_model(gcx, "provider/missing-model")
            .await
            .unwrap_err();
        assert!(
            err.contains("'provider/missing-model'"),
            "error should name the model: {err}"
        );
        assert!(
            err.contains("provider/available-model"),
            "error should list alternatives: {err}"
        );
    }

    #[tokio::test]
    async fn spawn_agent_fails_before_worktree_when_model_unavailable() {
        let gcx = gcx_with_chat_models(&[]).await;
        let cache_dir = gcx.cache_dir.clone();
        let err = preflight_agent_model(gcx, "unavailable/model")
            .await
            .unwrap_err();
        assert!(
            err.contains("'unavailable/model'"),
            "error should name the model: {err}"
        );
        assert!(
            err.contains("no chat models configured"),
            "error should indicate no models: {err}"
        );
        assert!(
            !cache_dir.join("worktrees").exists(),
            "no worktree directory should be created before preflight passes"
        );
    }

    #[tokio::test]
    async fn restart_agent_fails_before_worktree_when_model_unavailable() {
        let gcx = gcx_with_chat_models(&[]).await;
        let cache_dir = gcx.cache_dir.clone();
        let err = preflight_agent_model(gcx, "unavailable/model")
            .await
            .unwrap_err();
        assert!(
            err.contains("'unavailable/model'"),
            "error should name the model: {err}"
        );
        assert!(
            !cache_dir.join("worktrees").exists(),
            "no worktree directory should be created before preflight passes"
        );
    }
}
