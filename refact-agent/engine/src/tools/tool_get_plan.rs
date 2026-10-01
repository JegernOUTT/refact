use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::plan_role;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolGetPlan {
    pub config_path: String,
}

impl ToolGetPlan {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

fn plan_value(
    session: &ChatSession,
    archive: &[ChatMessage],
    message: &ChatMessage,
    include_history: bool,
) -> Result<Value, String> {
    let meta = message
        .extra
        .get("plan")
        .ok_or_else(|| "current plan is missing plan metadata".to_string())?;
    let mode = meta
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| "current plan is missing mode".to_string())?;
    let version = meta
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current plan is missing version".to_string())?;
    let created_at_ms = meta
        .get("created_at_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current plan is missing created_at_ms".to_string())?;
    let projection =
        plan_role::project_current_plan(session, crate::runtime_settings::plan_projection_limits())
            .ok_or_else(|| "current plan could not be synthesized".to_string())?;

    let mut value = json!({
        "content": projection.text,
        "mode": mode,
        "version": version,
        "created_at_ms": created_at_ms,
        "delta_count": projection.delta_count,
        "checkpoint": projection.checkpoint,
    });
    if include_history {
        value["history"] = Value::Array(
            super::role_history::archived_delta_history(
                archive,
                &session.messages,
                &message.message_id,
                "plan_delta",
            )
            .into_iter()
            .map(|(revision, note)| json!({"seq": revision, "content": note}))
            .collect(),
        );
    }
    Ok(value)
}

fn output_message(tool_call_id: &str, value: Value) -> Result<ContextEnum, String> {
    let content = serde_json::to_string(&value)
        .map_err(|error| format!("failed to serialize get_plan output: {error}"))?;
    Ok(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    }))
}

#[async_trait]
impl Tool for ToolGetPlan {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "get_plan".to_string(),
            display_name: "Get Plan".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read the current plan installed on this chat. Returns the bounded current projection, mode, base version, creation timestamp, delta count, and checkpoint provenance when older updates were folded.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "include_history": {
                        "type": "boolean",
                        "description": "Return the complete append-only delta history instead of only the bounded projection.",
                    }
                },
                "required": [],
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
        let include_history = args
            .get("include_history")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (sessions, chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.chat.sessions.clone(), ccx.chat_id.clone())
        };

        let session_arc = sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;
        let live = session_arc.lock().await;
        let session = live.try_accepted_control_projection()?;
        let plan = match plan_role::try_current_base_plan(&session)? {
            Some(message) => plan_value(&session, &live.messages, &message, include_history)?,
            None => Value::Null,
        };
        Ok((
            false,
            vec![output_message(tool_call_id, json!({ "plan": plan }))?],
        ))
    }
}

#[cfg(test)]
fn session_from_snapshot(
    chat_id: String,
    thread: crate::chat::types::ThreadParams,
    messages: Vec<ChatMessage>,
) -> ChatSession {
    let mut session = ChatSession::new(chat_id);
    session.thread = thread;
    session.messages = messages;
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles;
    use crate::chat::trajectories::{
        load_trajectory_for_chat, save_trajectory_snapshot, TrajectorySnapshot,
    };
    use crate::tools::tool_set_plan::ToolSetPlan;
    use crate::tools::tool_update_plan::ToolUpdatePlan;
    use refact_chat_api::role_projection::ProjectionLimits;
    use serial_test::serial;

    const APPROVED_BASE: &str =
        "# Approved plan\n\nShip M5.\n\nNon-goal: do not compact ordinary chat messages.";

    async fn ccx(app: AppState, chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    async fn insert_session(app: &AppState, session: ChatSession) {
        app.chat
            .sessions
            .write()
            .await
            .insert(session.chat_id.clone(), Arc::new(AMutex::new(session)));
    }

    fn result_json(result: (bool, Vec<ContextEnum>)) -> Value {
        assert!(!result.0);
        match result.1.into_iter().next().expect("tool output") {
            ContextEnum::ChatMessage(message) => {
                serde_json::from_str(&message.content.content_text_only()).unwrap()
            }
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    async fn drain_plan_side_effects(app: &AppState, chat_id: &str) {
        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        session_arc.lock().await.drain_post_tool_side_effects();
    }

    async fn session_messages(app: &AppState, chat_id: &str) -> Vec<ChatMessage> {
        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        session.messages.clone()
    }

    #[tokio::test]
    async fn plan_lifecycle_smoke() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat_id = "plan-lifecycle-smoke";
        let mut session = ChatSession::new(chat_id.to_string());
        session.thread.mode = "agent".to_string();
        session.thread.tool_use = "agent".to_string();
        session.thread.model = "model".to_string();
        session.thread.title = "Plan Lifecycle Smoke".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        insert_session(&app, session).await;

        let mut set_tool = ToolSetPlan {
            config_path: String::new(),
        };
        let set_result = result_json(
            set_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"set-plan".to_string(),
                    &HashMap::from([("content".to_string(), json!("## Plan\n- base"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set_result, json!({"version": 1, "supersedes": null}));
        drain_plan_side_effects(&app, chat_id).await;

        let messages = session_messages(&app, chat_id).await;
        let plans: Vec<_> = messages
            .iter()
            .filter(|message| message.role == internal_roles::PLAN_ROLE)
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].extra["plan"]["version"], json!(1));
        assert_eq!(plans[0].content.content_text_only(), "## Plan\n- base");

        let duplicate_err = set_tool
            .tool_execute(
                ccx(app.clone(), chat_id).await,
                &"set-plan-duplicate".to_string(),
                &HashMap::from([("content".to_string(), json!("second"))]),
            )
            .await
            .unwrap_err();
        assert_eq!(
            duplicate_err,
            "a plan already exists; use update_plan to change it"
        );

        let mut update_tool = ToolUpdatePlan {
            config_path: String::new(),
        };
        let update_result = result_json(
            update_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"update-plan".to_string(),
                    &HashMap::from([("note".to_string(), json!("- changed"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(update_result, json!({"seq": 1, "truncated": false}));
        drain_plan_side_effects(&app, chat_id).await;

        let mut get_tool = ToolGetPlan::new(String::new());
        let get_result = result_json(
            get_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"get-plan".to_string(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(get_result["plan"]["version"], json!(1));
        assert_eq!(get_result["plan"]["delta_count"], json!(1));
        assert_eq!(get_result["plan"]["mode"], json!("agent"));
        assert_eq!(
            get_result["plan"]["content"],
            json!("## Plan\n- base\n\n---\n\n## Plan updates\n\n- changed")
        );

        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        let snapshot = {
            let session = session_arc.lock().await;
            TrajectorySnapshot::from_thread_parts(
                chat_id.to_string(),
                &session.thread,
                session.messages.clone(),
                session.created_at.clone(),
                session.trajectory_version,
            )
        };
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let loaded_session =
            session_from_snapshot(chat_id.to_string(), loaded.thread, loaded.messages);
        let loaded_plans: Vec<_> = loaded_session
            .messages
            .iter()
            .filter(|message| message.role == internal_roles::PLAN_ROLE)
            .collect();
        assert_eq!(loaded_plans.len(), 1);
        assert_eq!(loaded_plans[0].extra["plan"]["version"], json!(1));
        assert_eq!(
            loaded_plans[0].content.content_text_only(),
            "## Plan\n- base"
        );
        let loaded_deltas = plan_role::plan_delta_events(&loaded_session);
        assert_eq!(loaded_deltas.len(), 1);
        assert_eq!(
            loaded_deltas[0].extra["event"]["subkind"],
            json!("plan_delta")
        );
        assert_eq!(loaded_deltas[0].extra["event"]["payload"]["seq"], json!(1));
        assert_eq!(loaded_deltas[0].content.content_text_only(), "- changed");
        assert_eq!(
            plan_role::synthesize_current_plan(&loaded_session).unwrap(),
            "## Plan\n- base\n\n---\n\n## Plan updates\n\n- changed"
        );
    }

    #[tokio::test]
    async fn no_plan_returns_null() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-no-plan";
        insert_session(&app, ChatSession::new(chat_id.to_string())).await;
        let mut tool = ToolGetPlan::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(output, json!({ "plan": null }));
    }

    #[tokio::test]
    async fn with_plan_returns_synthesized_plan() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-with-plan";
        let mut session = ChatSession::new(chat_id.to_string());
        session.install_plan("agent", "base plan");
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": 2}),
            "second update",
        ));
        let created_at_ms = session.messages[0].extra["plan"]["created_at_ms"]
            .as_u64()
            .unwrap();
        insert_session(&app, session).await;
        let mut tool = ToolGetPlan::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(
            output["plan"]["content"],
            json!("base plan\n\n---\n\n## Plan updates\n\nfirst update\n\nsecond update")
        );
        assert_eq!(output["plan"]["mode"], json!("agent"));
        assert_eq!(output["plan"]["version"], json!(1));
        assert_eq!(output["plan"]["created_at_ms"], json!(created_at_ms));
        assert_eq!(output["plan"]["delta_count"], json!(2));
    }
    fn plan_session_with_deltas(chat_id: &str, count: usize) -> ChatSession {
        let mut session = ChatSession::new(chat_id.to_string());
        session.install_plan("task_planner", APPROVED_BASE);
        for seq in 1..=count {
            session.add_message(internal_roles::plan_delta(
                "tool.update_plan",
                json!({"seq": seq}),
                format!("- update {seq}"),
            ));
        }
        session
    }

    #[tokio::test]
    #[serial(runtime_settings)]
    async fn get_plan_checkpoints_two_hundred_deltas_without_losing_approved_constraints() {
        crate::runtime_settings::reset_for_test();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-checkpoint";
        insert_session(&app, plan_session_with_deltas(chat_id, 200)).await;
        let limits = crate::runtime_settings::plan_projection_limits();

        let output = result_json(
            ToolGetPlan::new(String::new())
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"tc".to_string(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );

        let content = output["plan"]["content"].as_str().unwrap();
        assert_eq!(output["plan"]["delta_count"], json!(200));
        assert!(content.chars().count() <= limits.max_chars);
        assert!(content.starts_with(APPROVED_BASE));
        assert!(content.contains("Non-goal: do not compact ordinary chat messages."));
        assert!(content.ends_with("- update 200"));
        assert_eq!(
            output["plan"]["checkpoint"]["folded_deltas"],
            json!(200 - limits.checkpoint_after_deltas)
        );
        assert_eq!(output["plan"]["checkpoint"]["approved_version"], json!(1));
        assert_eq!(
            output["plan"]["checkpoint"]["approved_by"],
            json!("task_planner")
        );
        assert!(output["plan"].get("history").is_none());

        let messages = session_messages(&app, chat_id).await;
        assert_eq!(messages[0].content.content_text_only(), APPROVED_BASE);
        assert!(messages[0].extra["plan"].get("truncated").is_none());
        assert_eq!(messages.iter().filter(|m| m.role == "event").count(), 200);
    }

    #[tokio::test]
    #[serial(runtime_settings)]
    async fn get_plan_history_recovers_the_full_audit_trail_after_checkpointing() {
        crate::runtime_settings::reset_for_test();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-history";
        insert_session(&app, plan_session_with_deltas(chat_id, 200)).await;

        let output = result_json(
            ToolGetPlan::new(String::new())
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"tc".to_string(),
                    &HashMap::from([("include_history".to_string(), json!(true))]),
                )
                .await
                .unwrap(),
        );

        let history = output["plan"]["history"].as_array().unwrap();
        assert!(
            output["plan"]["checkpoint"]["folded_deltas"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(history.len(), 200);
        assert_eq!(history[0], json!({"seq": 1, "content": "- update 1"}));
        assert_eq!(history[199], json!({"seq": 200, "content": "- update 200"}));
    }

    #[tokio::test]
    async fn legacy_plan_trajectory_without_checkpoint_loads_and_projects() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let mut messages = vec![json!({
            "role": "plan",
            "content": "legacy base\n\nNon-goal: stay legacy.",
            "plan": {"mode": "agent", "version": 1, "created_at_ms": 7}
        })];
        for index in 1..=100 {
            messages.push(json!({
                "role": "event",
                "content": format!("- legacy update {index}"),
                "event": {"subkind": "plan_delta", "source": "tool.update_plan", "payload": {}}
            }));
        }
        tokio::fs::write(
            trajectories_dir.join("legacy-plan.json"),
            json!({
                "id": "legacy-plan",
                "title": "Legacy",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": messages,
                "include_project_info": true,
                "checkpoints_enabled": true
            })
            .to_string(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "legacy-plan").await.unwrap();
        let session =
            session_from_snapshot("legacy-plan".to_string(), loaded.thread, loaded.messages);

        let projection = plan_role::project_current_plan(
            &session,
            ProjectionLimits {
                checkpoint_after_deltas: 10,
                max_chars: 4_096,
            },
        )
        .unwrap();
        let checkpoint = projection.checkpoint.expect("checkpoint");

        assert_eq!(projection.delta_count, 100);
        assert_eq!(checkpoint.folded_deltas, 90);
        assert_eq!(checkpoint.revision, 90);
        assert_eq!(checkpoint.approved_version, 1);
        assert_eq!(checkpoint.approved_at_ms, 7);
        assert!(projection
            .text
            .starts_with("legacy base\n\nNon-goal: stay legacy."));
        assert!(projection.text.ends_with("- legacy update 100"));
        assert_eq!(plan_role::plan_delta_history(&session).len(), 100);
    }

    #[tokio::test]
    async fn accepted_control_pending_plan_is_read_without_delivery() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-pending";
        let mut session = ChatSession::new(chat_id.to_string());
        session.turn_depth = 1;
        let base = internal_roles::plan("agent", 1, "pending base", None);
        let delta = internal_roles::plan_delta("test", json!({"seq": 1}), "pending delta");
        session
            .enqueue_delivery(refact_core::chat_types::PendingDelivery::new(
                vec![base, delta],
                refact_core::chat_types::PushMode::WhenIdle,
                "test",
                false,
            ))
            .unwrap();
        insert_session(&app, session).await;
        let mut tool = ToolGetPlan::new(String::new());
        let output = result_json(
            tool.tool_execute(
                ccx(app.clone(), chat_id).await,
                &"read".to_string(),
                &HashMap::new(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(output["plan"]["delta_count"], json!(1));
        assert!(output["plan"]["content"]
            .as_str()
            .unwrap()
            .ends_with("pending delta"));
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        let session = session.lock().await;
        assert!(session.messages.is_empty());
        assert_eq!(session.pending_deliveries.len(), 1);
    }
}
