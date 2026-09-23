use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use refact_chat_api::GoalSnapshot;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_role;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolGetGoal {
    pub config_path: String,
}

impl ToolGetGoal {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

fn goal_value(
    session: &ChatSession,
    message: &ChatMessage,
    live: Option<&GoalSnapshot>,
    include_history: bool,
) -> Result<Value, String> {
    let meta = message
        .extra
        .get("goal")
        .ok_or_else(|| "current goal is missing goal metadata".to_string())?;
    let version = meta
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current goal is missing version".to_string())?;
    let goal = crate::chat::goal_snapshot_from_messages(&session.messages, live)
        .ok_or("current goal projection unavailable")?;
    let latest = goal.attempts.last();
    let projection =
        goal_role::project_current_goal(session, crate::runtime_settings::plan_projection_limits())
            .ok_or_else(|| "current goal could not be synthesized".to_string())?;
    let content = projection.text;
    let delta_count = projection.delta_count;

    let mut value = json!({
        "content": content,
        "checkpoint": projection.checkpoint,
        "status": goal.status,
        "active": goal.active,
        "version": version,
        "delta_count": delta_count,
        "turns_used": goal.progress.turns_used,
        "tokens_used": goal.progress.tokens_used,
        "no_progress_turns": goal.progress.no_progress_turns,
        "cost_used_cents": goal.progress.cost_used_cents,
        "budget": goal.budget,
        "snoozed_until_ms": goal.snoozed_until_ms,
        "wait": goal.wait,
        "latest_verdict": latest.map(|attempt| &attempt.verdict),
        "gaps": latest.map(|attempt| attempt.gaps.as_slice()).unwrap_or_default(),
    });
    if include_history {
        value["history"] = Value::Array(
            goal_role::goal_delta_history(session)
                .into_iter()
                .map(|(revision, note)| json!({"seq": revision, "content": note}))
                .collect(),
        );
    }
    Ok(value)
}

fn output_message(tool_call_id: &str, value: Value) -> Result<ContextEnum, String> {
    let content = serde_json::to_string(&value)
        .map_err(|error| format!("failed to serialize get_goal output: {error}"))?;
    Ok(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    }))
}

#[async_trait]
impl Tool for ToolGetGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "get_goal".to_string(),
            display_name: "Get Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read the current goal installed on this chat. Returns the bounded goal projection, checkpoint provenance, status, version, delta count, budget counters, latest verifier verdict, and gaps.".to_string(),
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
        let goal = match goal_role::current_base_goal(&session) {
            Some(message) => goal_value(&session, &message, live.goal.as_ref(), include_history)?,
            None => Value::Null,
        };
        Ok((
            false,
            vec![output_message(tool_call_id, json!({ "goal": goal }))?],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles;
    use crate::tools::tool_set_goal::ToolSetGoal;
    use crate::tools::tool_update_goal::ToolUpdateGoal;
    use crate::tools::tools_list::get_tools_for_mode;

    #[tokio::test]
    async fn public_goal_read_preserves_live_status_usage_verdict_and_unlimited_budget() {
        for status in [
            refact_chat_api::GoalStatus::Paused,
            refact_chat_api::GoalStatus::Stopped,
            refact_chat_api::GoalStatus::Verifying,
            refact_chat_api::GoalStatus::Completed,
        ] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let app = AppState::from_gcx(gcx).await;
            let chat_id = "get-goal-live";
            let mut session = ChatSession::new(chat_id.into());
            session.install_goal(
                "agent",
                "base",
                true,
                refact_chat_api::GoalBudget::default(),
            );
            session.goal_record_progress_with_cost(987, true, 12);
            session.goal_push_attempt(refact_chat_api::GoalAttempt {
                verdict: "unmet".into(),
                gaps: vec!["missing executed evidence".into()],
                ..Default::default()
            });
            session.goal_set_status(status);
            session.turn_depth = 1;
            session
                .enqueue_delivery(refact_core::chat_types::PendingDelivery::new(
                    vec![internal_roles::goal_delta(
                        "test",
                        json!({"seq":1}),
                        "pending change",
                    )],
                    refact_core::chat_types::PushMode::WhenIdle,
                    "test",
                    false,
                ))
                .unwrap();
            let before = session.goal.clone();
            insert_session(&app, session).await;
            let output = result_json(
                ToolGetGoal::new(String::new())
                    .tool_execute(
                        ccx(app.clone(), chat_id).await,
                        &"read-live".into(),
                        &HashMap::new(),
                    )
                    .await
                    .unwrap(),
            );
            assert_eq!(output["goal"]["status"], json!(status));
            assert_eq!(output["goal"]["turns_used"], 1);
            assert_eq!(output["goal"]["tokens_used"], 987);
            assert_eq!(output["goal"]["cost_used_cents"], 12);
            assert_eq!(output["goal"]["latest_verdict"], "unmet");
            assert_eq!(output["goal"]["gaps"], json!(["missing executed evidence"]));
            assert!(output["goal"]["budget"]["max_turns"].is_null());
            assert!(output["goal"]["content"]
                .as_str()
                .unwrap()
                .ends_with("pending change"));
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(chat_id)
                .cloned()
                .unwrap();
            let session = session.lock().await;
            assert_eq!(session.goal, before);
            assert_eq!(session.pending_deliveries.len(), 1);
        }
    }

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

    #[tokio::test]
    async fn public_goal_read_preserves_restored_ledger_state_without_rearming() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().into()];
        let app = AppState::from_gcx(gcx.clone()).await;
        for (index, status) in [
            refact_chat_api::GoalStatus::Paused,
            refact_chat_api::GoalStatus::Completed,
            refact_chat_api::GoalStatus::BudgetExhausted,
            refact_chat_api::GoalStatus::Transferred,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("restored-goal-{index}");
            let mut session = ChatSession::new(id.clone());
            session.install_goal(
                "agent",
                "preserve persisted state",
                true,
                refact_chat_api::GoalBudget {
                    max_turns: Some(3),
                    max_tokens: Some(0),
                    explicit: true,
                    ..Default::default()
                },
            );
            session.goal_record_progress_with_cost(456, true, 9);
            session.goal_push_attempt(refact_chat_api::GoalAttempt {
                verdict: "blocked".into(),
                gaps: vec!["required evidence unavailable".into()],
                ..Default::default()
            });
            session.goal_set_status(status);
            let before = session.goal.clone().unwrap();
            let ledger = session.goal_ledger.clone();
            let session = Arc::new(AMutex::new(session));
            app.chat
                .sessions
                .write()
                .await
                .insert(id.clone(), session.clone());
            crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                .await
                .unwrap();
            let saved = crate::chat::trajectories::load_trajectory_for_chat(gcx.clone(), &id)
                .await
                .unwrap();
            assert_eq!(saved.goal_ledger, ledger);
            let mut restored = ChatSession::new_with_trajectory(
                id.clone(),
                saved.messages,
                saved.thread,
                saved.created_at,
                saved.wake_up_at,
                saved.waiting_for_card_ids,
                saved.goal,
            );
            restored.set_goal_ledger(saved.goal_ledger);
            insert_session(&app, restored).await;
            let output = result_json(
                ToolGetGoal::new(String::new())
                    .tool_execute(
                        ccx(app.clone(), &id).await,
                        &"restored-read".into(),
                        &HashMap::new(),
                    )
                    .await
                    .unwrap(),
            );
            assert_eq!(output["goal"]["status"], json!(status));
            assert_eq!(output["goal"]["active"], before.active);
            assert_eq!(output["goal"]["tokens_used"], 456);
            assert_eq!(output["goal"]["cost_used_cents"], 9);
            assert_eq!(output["goal"]["budget"]["max_turns"], 3);
            assert_eq!(output["goal"]["budget"]["max_tokens"], 0);
            assert_eq!(output["goal"]["latest_verdict"], "blocked");
            assert_eq!(
                output["goal"]["gaps"],
                json!(["required evidence unavailable"])
            );
            let session = app.chat.sessions.read().await.get(&id).cloned().unwrap();
            let session = session.lock().await;
            assert_eq!(session.goal.as_ref().unwrap(), &before);
            assert_eq!(session.goal_ledger, ledger);
            assert!(session.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn public_goal_read_does_not_attach_old_verdict_to_new_goal_version() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let id = "new-goal-version";
        let mut session = ChatSession::new(id.into());
        session.install_goal("agent", "old", true, refact_chat_api::GoalBudget::default());
        session.goal_record_progress_with_cost(50, true, 2);
        session.goal_push_attempt(refact_chat_api::GoalAttempt {
            verdict: "met".into(),
            ..Default::default()
        });
        session.turn_depth = 1;
        session
            .enqueue_delivery(refact_core::chat_types::PendingDelivery::new(
                vec![internal_roles::goal(
                    "agent",
                    2,
                    "new",
                    None,
                    false,
                    refact_chat_api::GoalBudget::default(),
                )],
                refact_core::chat_types::PushMode::WhenIdle,
                "test",
                false,
            ))
            .unwrap();
        insert_session(&app, session).await;
        let output = result_json(
            ToolGetGoal::new(String::new())
                .tool_execute(
                    ccx(app.clone(), id).await,
                    &"new-version-read".into(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(output["goal"]["content"], "new");
        assert_eq!(output["goal"]["version"], 2);
        assert_eq!(output["goal"]["status"], "paused");
        assert_eq!(output["goal"]["tokens_used"], 0);
        assert!(output["goal"]["latest_verdict"].is_null());
        let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
        assert_eq!(session.lock().await.goal.as_ref().unwrap().version, 1);
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

    async fn drain_goal_side_effects(app: &AppState, chat_id: &str) {
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

    #[tokio::test]
    async fn goal_lifecycle_smoke() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat_id = "goal-lifecycle-smoke";
        let mut session = ChatSession::new(chat_id.to_string());
        session.thread.mode = "agent".to_string();
        session.thread.tool_use = "agent".to_string();
        session.thread.model = "model".to_string();
        insert_session(&app, session).await;

        let mut set_tool = ToolSetGoal {
            config_path: String::new(),
        };
        let set_result = result_json(
            set_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"set-goal".to_string(),
                    &HashMap::from([("content".to_string(), json!("Ship the pond"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set_result, json!({"version": 1, "supersedes": null}));
        drain_goal_side_effects(&app, chat_id).await;

        let duplicate_err = set_tool
            .tool_execute(
                ccx(app.clone(), chat_id).await,
                &"set-goal-duplicate".to_string(),
                &HashMap::from([("content".to_string(), json!("second"))]),
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate_err, "goal already exists; use update_goal");

        let mut update_tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let update_result = result_json(
            update_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"update-goal".to_string(),
                    &HashMap::from([("note".to_string(), json!("Add tests"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(update_result, json!({"seq": 1, "truncated": false}));
        drain_goal_side_effects(&app, chat_id).await;

        let mut get_tool = ToolGetGoal::new(String::new());
        let get_result = result_json(
            get_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"get-goal".to_string(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(get_result["goal"]["version"], json!(1));
        assert_eq!(get_result["goal"]["delta_count"], json!(1));
        assert_eq!(get_result["goal"]["status"], json!("active"));
        assert_eq!(get_result["goal"]["turns_used"], json!(0));
        assert_eq!(get_result["goal"]["tokens_used"], json!(0));
        assert_eq!(get_result["goal"]["latest_verdict"], Value::Null);
        assert_eq!(get_result["goal"]["gaps"], json!([]));
        assert_eq!(
            get_result["goal"]["content"],
            json!("Ship the pond\n\n---\n\n## Goal updates\n\nAdd tests")
        );
    }

    #[tokio::test]
    async fn no_goal_returns_null() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-goal-no-goal";
        insert_session(&app, ChatSession::new(chat_id.to_string())).await;
        let mut tool = ToolGetGoal::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(output, json!({ "goal": null }));
    }

    #[tokio::test]
    async fn with_goal_returns_synthesized_goal() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-goal-with-goal";
        let mut session = ChatSession::new(chat_id.to_string());
        crate::chat::goal_role::install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            refact_chat_api::GoalBudget::default(),
        );
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 2}),
            "second update",
        ));
        insert_session(&app, session).await;
        let mut tool = ToolGetGoal::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(
            output["goal"]["content"],
            json!("base goal\n\n---\n\n## Goal updates\n\nfirst update\n\nsecond update")
        );
        assert_eq!(output["goal"]["status"], json!("active"));
        assert_eq!(output["goal"]["version"], json!(1));
        assert_eq!(output["goal"]["delta_count"], json!(2));
    }

    #[tokio::test]
    async fn available_in_plan_supporting_modes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        for mode in ["agent", "task_agent", "task_planner"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "get_goal");
            assert!(has_tool, "{mode} should expose get_goal");
        }
        for mode in ["NO_TOOLS", "shell", "explore"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "get_goal");
            assert!(!has_tool, "{mode} should not expose get_goal");
        }
    }
    #[tokio::test]
    async fn accepted_control_pending_goal_is_read_without_delivery() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-goal-pending";
        let mut session = ChatSession::new(chat_id.to_string());
        session.turn_depth = 1;
        let base = internal_roles::goal(
            "agent",
            1,
            "pending base",
            None,
            true,
            refact_chat_api::GoalBudget::default(),
        );
        let delta = internal_roles::goal_delta("test", json!({"seq": 1}), "pending delta");
        session
            .enqueue_delivery(refact_core::chat_types::PendingDelivery::new(
                vec![base, delta],
                refact_core::chat_types::PushMode::WhenIdle,
                "test",
                false,
            ))
            .unwrap();
        insert_session(&app, session).await;
        let mut tool = ToolGetGoal::new(String::new());
        let output = result_json(
            tool.tool_execute(
                ccx(app.clone(), chat_id).await,
                &"read".to_string(),
                &HashMap::new(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(output["goal"]["delta_count"], json!(1));
        assert!(output["goal"]["content"]
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
