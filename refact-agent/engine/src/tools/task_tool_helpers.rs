use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::TaskMeta;
use crate::tasks::storage;

pub(crate) fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

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

pub(crate) fn human_age(ts: DateTime<Utc>) -> String {
    let seconds = Utc::now().signed_duration_since(ts).num_seconds().max(0);
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
        assert_eq!(required_string(&args, "empty").unwrap_err(), "Missing 'empty'");
        assert_eq!(required_string(&args, "null").unwrap_err(), "Missing 'null'");
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

        assert_eq!(err, "task_id override is not allowed from this planner chat");
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
}
