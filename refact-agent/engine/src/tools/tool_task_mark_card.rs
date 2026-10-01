use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use chrono::Utc;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::{FinalizeDecision, StatusUpdate};

const FINALIZE_REFUSAL_PREFIX: &str = "finalize_refusal:";

async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

async fn authorize_finalizer(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    task_id: &str,
    card_id: &str,
) -> Result<(), String> {
    let (gcx, principal) = {
        let ccx_lock = ccx.lock().await;
        (ccx_lock.app.gcx.clone(), ccx_lock.chat_id.clone())
    };
    let allow_self_review = storage::board_allows_self_review(gcx.clone(), task_id).await?;
    storage::load_board(gcx, task_id)
        .await?
        .authorize_finalizer(card_id, &principal, allow_self_review)
        .map_err(storage::violation_error)
}

pub struct ToolTaskMarkCardDone;
pub struct ToolTaskMarkCardFailed;
pub struct ToolTaskFinalizeCard;

impl ToolTaskMarkCardDone {
    pub fn new() -> Self {
        Self
    }
}

impl ToolTaskMarkCardFailed {
    pub fn new() -> Self {
        Self
    }
}

impl ToolTaskFinalizeCard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMarkCardDone {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mark_done".to_string(),
            display_name: "Task Mark Card Done".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manually mark a card as done. Use this if an agent completed work but forgot to call agent_finish(), or to finalize a card after reviewing the agent's work.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID to mark as done"), ("report", "string", "Summary/report for the completed card")], &["card_id", "report"]),
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
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let report = args
            .get("report")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'report'")?;
        authorize_finalizer(&ccx, &task_id, card_id).await?;

        let gcx = ccx.lock().await.app.gcx.clone();

        let card_title = {
            let card_id_owned = card_id.to_string();
            let report_owned = report.to_string();

            let (board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;

                if card.column == "done" {
                    return Err(format!("Card {} is already done", card_id_owned));
                }

                card.final_report = Some(report_owned.clone());
                card.final_report_structured = None;
                card.column = "done".to_string();
                card.completed_at = Some(Utc::now().to_rfc3339());
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "Manually marked as done by planner".to_string(),
                });
                Ok(())
            })
            .await?;

            storage::update_task_stats(gcx.clone(), &task_id).await?;
            board
                .get_card(card_id)
                .map(|c| c.title.clone())
                .unwrap_or_default()
        };

        let result = format!(
            "✅ **Card marked as done:** {}\n\n**Report:**\n{}",
            card_title, report
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
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

#[async_trait]
impl Tool for ToolTaskFinalizeCard {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "finalize_card".to_string(),
            display_name: "Task Finalize Card".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Finalize a done card after checking its review authority and required outcome evidence. Returns every unmet requirement when finalization is refused.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID to finalize"),
                    ("task_id", "string", "Task UUID (optional if in task context)"),
                ],
                &["card_id"],
            ),
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
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(Value::as_str)
            .ok_or("Missing 'card_id'")?;
        let (gcx, principal, is_planner) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock
                    .task_meta
                    .as_ref()
                    .is_some_and(|meta| meta.role == "planner"),
            )
        };
        if !is_planner {
            return Err(
                "finalize_card can only be called by the task planner. Switch to the planner chat to finalize cards."
                    .to_string(),
            );
        }
        let allow_self_review = storage::board_allows_self_review(gcx.clone(), &task_id).await?;
        let card_id_owned = card_id.to_string();
        let principal_owned = principal.clone();
        let finalized_at = Utc::now().to_rfc3339();
        let result = storage::update_board_atomic(gcx, &task_id, move |board| {
            match board.finalize_card(
                &card_id_owned,
                &principal_owned,
                allow_self_review,
                finalized_at,
            ) {
                Ok(()) => Ok(()),
                Err(decision) => {
                    let decision =
                        serde_json::to_string(&decision).map_err(|error| error.to_string())?;
                    Err(format!("{FINALIZE_REFUSAL_PREFIX}{decision}"))
                }
            }
        })
        .await;
        let (finalized, decision) = match result {
            Ok(_) => (true, FinalizeDecision::default()),
            Err(error) => match error.strip_prefix(FINALIZE_REFUSAL_PREFIX) {
                Some(decision) => (
                    false,
                    serde_json::from_str(decision).map_err(|e| e.to_string())?,
                ),
                None => return Err(error),
            },
        };
        let output = serde_json::json!({
            "type": "task_card_finalize",
            "task_id": task_id,
            "card_id": card_id,
            "finalized": finalized,
            "decision": decision,
        })
        .to_string();
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
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
    use crate::global_context::GlobalContext;
    use crate::tasks::types::{
        BoardCard, FragmentReview, OutcomeFragment, OutcomeRequirement, TaskBoard, TaskMeta,
        TaskStatus, VerifierReport, VerifierReportClassification,
    };
    use refact_chat_api::TaskMeta as ThreadTaskMeta;
    use serde_json::json;

    fn card(id: &str) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: id.to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
        }
    }

    async fn setup_task() -> (Arc<GlobalContext>, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        tokio::fs::create_dir_all(temp.path().join(".refact").join("tasks").join("task-1"))
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        storage::save_task_meta(
            gcx.clone(),
            "task-1",
            &TaskMeta {
                schema_version: 1,
                id: "task-1".to_string(),
                name: "Task".to_string(),
                status: TaskStatus::Active,
                created_at: now.clone(),
                updated_at: now,
                cards_total: 0,
                cards_done: 0,
                cards_failed: 0,
                agents_active: 0,
                base_branch: None,
                base_commit: None,
                default_agent_model: None,
                is_name_generated: false,
                last_agents_summary_at: None,
                planner_session_state: None,
            },
        )
        .await
        .unwrap();
        storage::create_board(
            gcx.clone(),
            "task-1",
            &TaskBoard {
                cards: vec![card("T-1")],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        (gcx, temp)
    }

    async fn chat_ccx(gcx: Arc<GlobalContext>, chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                Vec::new(),
                chat_id.to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(chat_id.to_string()),
                }),
                None,
            )
            .await,
        ))
    }

    fn done_args() -> HashMap<String, Value> {
        HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("report".to_string(), json!("finished")),
        ])
    }

    fn tool_text(contexts: Vec<ContextEnum>) -> String {
        match contexts
            .into_iter()
            .next()
            .expect("tool produced no message")
        {
            ContextEnum::ChatMessage(message) => message.content.content_text_only(),
            _ => panic!("expected a tool chat message"),
        }
    }

    #[tokio::test]
    async fn legacy_board_without_an_owner_still_finalises() {
        let (gcx, _temp) = setup_task().await;
        let ccx = chat_ccx(gcx.clone(), "any-chat").await;

        ToolTaskMarkCardDone::new()
            .tool_execute(ccx, &"call".to_string(), &done_args())
            .await
            .unwrap();

        assert_eq!(
            storage::load_board(gcx, "task-1")
                .await
                .unwrap()
                .get_card("T-1")
                .unwrap()
                .column,
            "done"
        );
    }

    #[tokio::test]
    async fn finalising_refuses_a_principal_without_a_board_role() {
        let (gcx, _temp) = setup_task().await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            Ok(())
        })
        .await
        .unwrap();
        let stranger = chat_ccx(gcx.clone(), "stranger-chat").await;

        let denied = ToolTaskMarkCardDone::new()
            .tool_execute(stranger, &"call".to_string(), &done_args())
            .await
            .unwrap_err();

        let decoded: Value = serde_json::from_str(&denied).unwrap();
        assert_eq!(decoded["code"], "scope_denied");
        assert!(decoded["message"]
            .as_str()
            .unwrap()
            .contains("board owner or an assigned reviewer"));
        assert_eq!(
            storage::load_board(gcx, "task-1")
                .await
                .unwrap()
                .get_card("T-1")
                .unwrap()
                .column,
            "doing"
        );
    }

    #[tokio::test]
    async fn finalising_refuses_self_review_by_the_claimant() {
        let (gcx, _temp) = setup_task().await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            board.reviewer_chat_ids = vec!["agent-chat".to_string()];
            Ok(())
        })
        .await
        .unwrap();
        storage::claim_card(gcx.clone(), "task-1", "T-1", "agent-chat", "key")
            .await
            .unwrap();

        let claimant = chat_ccx(gcx.clone(), "agent-chat").await;
        let denied = ToolTaskMarkCardDone::new()
            .tool_execute(claimant, &"call".to_string(), &done_args())
            .await
            .unwrap_err();
        assert!(denied.contains("self-review is refused"));

        let owner = chat_ccx(gcx.clone(), "planner-chat").await;
        ToolTaskMarkCardDone::new()
            .tool_execute(owner, &"call".to_string(), &done_args())
            .await
            .unwrap();

        assert_eq!(
            storage::load_board(gcx, "task-1")
                .await
                .unwrap()
                .get_card("T-1")
                .unwrap()
                .column,
            "done"
        );
    }

    #[tokio::test]
    async fn finalize_card_tool_reaches_finalize_gate_and_records_finalization() {
        let (gcx, _temp) = setup_task().await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.cards[0].column = "done".to_string();
            Ok(())
        })
        .await
        .unwrap();
        let ccx = chat_ccx(gcx.clone(), "planner-chat").await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let output = ToolTaskFinalizeCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(&tool_text(output.1)).unwrap();

        assert_eq!(response["finalized"], json!(true));
        assert_eq!(response["decision"]["blocks"], json!([]));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert!(board.card_is_finalized("T-1"));
        assert_eq!(board.get_card("T-1").unwrap().column, "done");
    }

    #[tokio::test]
    async fn finalize_card_refusal_lists_all_blocks_and_leaves_done_card_unchanged() {
        let (gcx, _temp) = setup_task().await;
        let report = VerifierReport {
            passed: true,
            recommendation: "merge".to_string(),
            classification: VerifierReportClassification::Passed,
            ..Default::default()
        };
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            board.reviewer_chat_ids = vec!["reviewer-chat".to_string()];
            board.cards[0].column = "done".to_string();
            board.cards[0].verifier_report = Some(report.clone());
            board
                .attest_review(
                    "T-1",
                    "reviewer-chat",
                    &report,
                    "2026-01-01T00:00:00Z".to_string(),
                    false,
                )
                .map_err(|error| error.to_string())?;
            board.record_card_outcome(
                "T-1",
                vec![
                    OutcomeRequirement {
                        fragment: "summary".to_string(),
                        requires_evidence: false,
                    },
                    OutcomeRequirement {
                        fragment: "verification".to_string(),
                        requires_evidence: false,
                    },
                ],
                vec![
                    OutcomeFragment::new(
                        "summary",
                        "Implemented the change",
                        Some("agent-1".into()),
                    ),
                    OutcomeFragment::new("verification", "Tests failed", Some("agent-1".into())),
                ],
            );
            board
                .card_outcomes
                .get_mut("T-1")
                .unwrap()
                .record_review(
                    "summary",
                    FragmentReview::Rejected,
                    "reviewer-chat",
                    "2026-01-01T00:00:00Z".to_string(),
                    Some("summary is unsupported".to_string()),
                )
                .map_err(|error| error.to_string())?;
            board
                .card_outcomes
                .get_mut("T-1")
                .unwrap()
                .record_review(
                    "verification",
                    FragmentReview::Contradicted,
                    "reviewer-chat",
                    "2026-01-01T00:00:00Z".to_string(),
                    Some("the report records a failing test".to_string()),
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        })
        .await
        .unwrap();
        let revision = storage::load_board(gcx.clone(), "task-1")
            .await
            .unwrap()
            .rev;
        let ccx = chat_ccx(gcx.clone(), "planner-chat").await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let output = ToolTaskFinalizeCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(&tool_text(output.1)).unwrap();

        assert_eq!(response["finalized"], json!(false));
        let blocks = response["decision"]["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["kind"], json!("rejected_fragment"));
        assert_eq!(blocks[1]["kind"], json!("contradicted_fragment"));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.rev, revision);
        assert_eq!(board.get_card("T-1").unwrap().column, "done");
        assert!(!board.card_is_finalized("T-1"));
    }

    #[tokio::test]
    async fn finalize_card_refuses_self_review_by_default_and_setting_can_allow_it() {
        let (gcx, temp) = setup_task().await;
        let report = VerifierReport {
            passed: true,
            recommendation: "merge".to_string(),
            classification: VerifierReportClassification::Passed,
            ..Default::default()
        };
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            board.reviewer_chat_ids = vec!["reviewer-chat".to_string()];
            board.cards[0].column = "done".to_string();
            board.cards[0].verifier_report = Some(report.clone());
            board
                .attest_review(
                    "T-1",
                    "reviewer-chat",
                    &report,
                    "2026-01-01T00:00:00Z".to_string(),
                    false,
                )
                .map_err(|error| error.to_string())?;
            board
                .claim_card(
                    "T-1",
                    "planner-chat",
                    "claim",
                    "2026-01-01T00:00:00Z".into(),
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        })
        .await
        .unwrap();
        let ccx = chat_ccx(gcx.clone(), "planner-chat").await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let refused = ToolTaskFinalizeCard::new()
            .tool_execute(ccx.clone(), &"call".to_string(), &args)
            .await
            .unwrap();
        let refused: Value = serde_json::from_str(&tool_text(refused.1)).unwrap();
        assert_eq!(refused["finalized"], json!(false));
        assert!(refused["decision"]["blocks"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("self-review is refused"));
        assert!(!storage::load_board(gcx.clone(), "task-1")
            .await
            .unwrap()
            .card_is_finalized("T-1"));

        tokio::fs::write(
            temp.path().join(".refact").join("coordination.yaml"),
            "board_allow_self_review: true\n",
        )
        .await
        .unwrap();
        let allowed = ToolTaskFinalizeCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        let allowed: Value = serde_json::from_str(&tool_text(allowed.1)).unwrap();
        assert_eq!(allowed["finalized"], json!(true));
        assert!(storage::load_board(gcx, "task-1")
            .await
            .unwrap()
            .card_is_finalized("T-1"));
    }

    #[tokio::test]
    async fn finalize_card_tool_is_planner_only() {
        let (gcx, _temp) = setup_task().await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.cards[0].column = "done".to_string();
            Ok(())
        })
        .await
        .unwrap();
        let ccx = chat_ccx(gcx.clone(), "agent-chat").await;
        ccx.lock().await.task_meta.as_mut().unwrap().role = "agent".to_string();

        let denied = ToolTaskFinalizeCard::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .unwrap_err();

        assert!(denied.contains("only be called by the task planner"));
        assert!(!storage::load_board(gcx, "task-1")
            .await
            .unwrap()
            .card_is_finalized("T-1"));
    }
}

#[async_trait]
impl Tool for ToolTaskMarkCardFailed {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mark_failed".to_string(),
            display_name: "Task Mark Card Failed".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manually mark a card as failed. Use this to resolve stuck agents, mark cards that cannot be completed, or when an agent errored without calling agent_finish().".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID to mark as failed"), ("reason", "string", "Reason for failure")], &["card_id", "reason"]),
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
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'reason'")?;
        authorize_finalizer(&ccx, &task_id, card_id).await?;

        let gcx = ccx.lock().await.app.gcx.clone();

        let card_title = {
            let card_id_owned = card_id.to_string();
            let reason_owned = reason.to_string();

            let (board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;

                if card.column == "failed" {
                    return Err(format!("Card {} is already failed", card_id_owned));
                }

                card.final_report = Some(format!("FAILED: {}", reason_owned));
                card.final_report_structured = None;
                card.column = "failed".to_string();
                card.completed_at = Some(Utc::now().to_rfc3339());
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: format!("Manually marked as failed: {}", reason_owned),
                });
                Ok(())
            })
            .await?;

            storage::update_task_stats(gcx.clone(), &task_id).await?;
            board
                .get_card(card_id)
                .map(|c| c.title.clone())
                .unwrap_or_default()
        };

        let result = format!(
            "❌ **Card marked as failed:** {}\n\n**Reason:**\n{}",
            card_title, reason
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
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
