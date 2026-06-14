use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::verifier::{verify_card, VerifyCardRequest};
use crate::tools::task_tool_helpers::require_bound_planner_task;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolTaskVerifyCard;

impl ToolTaskVerifyCard {
    pub fn new() -> Self {
        Self
    }
}

fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

fn verify_card_task_scope_error(error: String) -> String {
    if error == "task_id override is not allowed from this planner chat" {
        "task_id does not match bound task_id".to_string()
    } else {
        error
    }
}

#[async_trait]
impl Tool for ToolTaskVerifyCard {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_verify_card".to_string(),
            display_name: "Task Verify Card".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manually re-run verifier for a completed task card and store verifier_report on the card.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "card_id": {"type": "string", "description": "Card ID to verify"},
                    "task_id": {"type": "string", "description": "Task ID (optional if in task context)"}
                },
                "required": ["card_id"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;
        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|meta| meta.role == "planner")
            .unwrap_or(false);
        if !is_planner {
            return Err("task_verify_card can only be called by the task planner.".to_string());
        }
        drop(ccx_lock);

        let card_id = required_string(args, "card_id")?;
        let task_id = require_bound_planner_task(&ccx, args)
            .await
            .map_err(verify_card_task_scope_error)?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let report = verify_card(
            gcx,
            VerifyCardRequest {
                task_id,
                card_id: card_id.clone(),
                expected_state: None,
            },
        )
        .await?;
        let concerns = if report.concerns.is_empty() {
            "none".to_string()
        } else {
            report.concerns.join("\n- ")
        };
        let content = format!(
            "# Verifier Report\n\n**Card:** {}\n**Passed:** {}\n**Recommendation:** {}\n\n## Concerns\n- {}",
            card_id, report.passed, report.recommendation, concerns
        );
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(content),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::global_context::GlobalContext;
    use crate::tasks::storage;
    use crate::tasks::types::{TaskBoard, TaskMeta as StoredTaskMeta, TaskStatus};
    use std::path::Path;

    fn task_meta() -> StoredTaskMeta {
        let now = chrono::Utc::now().to_rfc3339();
        StoredTaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 0,
            base_branch: Some("main".to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn write_task(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        storage::save_task_meta(gcx.clone(), "task-1", &task_meta())
            .await
            .unwrap();
        storage::save_board(gcx.clone(), "task-1", &TaskBoard::default())
            .await
            .unwrap();
        gcx
    }

    fn thread_task_meta(role: &str) -> ThreadTaskMeta {
        ThreadTaskMeta {
            task_id: "task-1".to_string(),
            role: role.to_string(),
            agent_id: (role == "agents").then(|| "agent-1".to_string()),
            card_id: (role == "agents").then(|| "T-1".to_string()),
            planner_chat_id: Some("planner-task-1-1".to_string()),
        }
    }

    async fn task_ccx(
        gcx: Arc<GlobalContext>,
        task_meta: Option<ThreadTaskMeta>,
        chat_id: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
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
                task_meta,
                None,
            )
            .await,
        ))
    }

    #[tokio::test]
    async fn task_verify_card_rejects_mismatched_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = task_ccx(gcx, Some(thread_task_meta("planner")), "planner-task-1-1").await;
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("task_id".to_string(), json!("task-2")),
        ]);

        let err = ToolTaskVerifyCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("does not match bound task_id"));
    }

    #[tokio::test]
    async fn task_verify_card_uses_bound_task_when_no_args() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path()).await;
        let ccx = task_ccx(gcx, Some(thread_task_meta("planner")), "planner-task-1-1").await;
        let args = HashMap::from([("card_id".to_string(), json!("missing-card"))]);

        let err = ToolTaskVerifyCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "Card missing-card not found");
    }

    #[tokio::test]
    async fn task_verify_card_planner_without_metadata_rejected() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = task_ccx(gcx, None, "unbound-chat").await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let err = ToolTaskVerifyCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("task planner"));
    }

    #[tokio::test]
    async fn task_verify_card_non_planner_role_rejected() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = task_ccx(gcx, Some(thread_task_meta("agents")), "agent-chat").await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let err = ToolTaskVerifyCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("task planner"));
    }
}
