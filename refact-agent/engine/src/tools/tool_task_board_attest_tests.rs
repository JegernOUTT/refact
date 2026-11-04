use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ContextEnum};
use crate::chat::verifier::store_verifier_report;
use crate::tasks::storage;
use crate::tasks::types::{
    BoardCard, TaskBoard, TaskMeta, TaskStatus, VerificationOutcome, VerificationResult,
    VerifierReport, VerifierReportClassification,
};
use crate::tools::tool_task_board_authority::ToolTaskBoardAuthority;
use crate::tools::tool_task_mark_card::ToolTaskFinalizeCard;
use crate::tools::tools_description::Tool;

fn card() -> BoardCard {
    BoardCard {
        id: "T-1".to_string(),
        title: "Card".to_string(),
        column: "done".to_string(),
        priority: "P1".to_string(),
        depends_on: vec![],
        instructions: String::new(),
        assignee: None,
        agent_chat_id: None,
        status_updates: vec![],
        comments: vec![],
        final_report: Some("finished".to_string()),
        final_report_structured: None,
        verifier_report: Some(verifier_report(true)),
        created_at: Utc::now().to_rfc3339(),
        started_at: None,
        last_heartbeat_at: None,
        paused_at: None,
        paused_by: None,
        completed_at: Some(Utc::now().to_rfc3339()),
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
        deliverable: None,
        verification: Vec::new(),
    }
}

fn verifier_report(passed: bool) -> VerifierReport {
    VerifierReport {
        notes: Vec::new(),
        passed,
        command_results: vec![VerificationResult {
            command: "cargo test".to_string(),
            exit_code: Some(if passed { 0 } else { 1 }),
            passed,
            output_tail: if passed { "ok" } else { "failed" }.to_string(),
            outcome: if passed {
                VerificationOutcome::Passed
            } else {
                VerificationOutcome::CommandFailed
            },
            from_cache: false,
            executed_at: None,
        }],
        concerns: if passed {
            vec![]
        } else {
            vec!["failed".to_string()]
        },
        recommendation: if passed { "merge" } else { "fix-needed" }.to_string(),
        classification: if passed {
            VerifierReportClassification::Passed
        } else {
            VerifierReportClassification::VerificationFailed
        },
    }
}

async fn setup_task(
    with_report: bool,
    claimant: Option<&str>,
) -> (Arc<crate::global_context::GlobalContext>, tempfile::TempDir) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let temp = tempfile::tempdir().unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    tokio::fs::create_dir_all(temp.path().join(".refact/tasks/task-1"))
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
            cards_total: 1,
            cards_done: 1,
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
    let mut task_card = card();
    if !with_report {
        task_card.verifier_report = None;
    }
    storage::create_board(
        gcx.clone(),
        "task-1",
        &TaskBoard {
            cards: vec![task_card],
            owner_chat_id: Some("planner-owner".to_string()),
            reviewer_chat_ids: vec!["planner-reviewer".to_string()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    if let Some(claimant) = claimant {
        storage::claim_card(gcx.clone(), "task-1", "T-1", claimant, "claim-key")
            .await
            .unwrap();
    }
    for chat_id in ["planner-owner", "planner-reviewer", "planner-unassigned"] {
        crate::chat::trajectories::save_initial_task_chat_trajectory(
            gcx.clone(),
            "task-1",
            chat_id,
            "task_planner",
        )
        .await
        .unwrap();
        let path = crate::chat::trajectories::find_trajectory_path(gcx.clone(), chat_id)
            .await
            .unwrap();
        let mut value: Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        value["model"] = json!("model");
        tokio::fs::write(path, serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
    }
    (gcx, temp)
}

async fn chat_ccx(
    gcx: Arc<crate::global_context::GlobalContext>,
    chat_id: &str,
    role: &str,
) -> Arc<AMutex<AtCommandsContext>> {
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
            Some(refact_chat_api::TaskMeta {
                task_id: "task-1".to_string(),
                role: role.to_string(),
                agent_id: (role == "agents").then(|| "agent-1".to_string()),
                card_id: (role == "agents").then(|| "T-1".to_string()),
                planner_chat_id: Some("planner-owner".to_string()),
            }),
            None,
        )
        .await,
    ))
}

fn args(action: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("action".to_string(), json!(action)),
        ("card_id".to_string(), json!("T-1")),
    ])
}

fn text(contexts: Vec<ContextEnum>) -> String {
    match contexts.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(message) => match message.content {
            ChatContent::SimpleText(text) => text,
            _ => panic!("expected text"),
        },
        _ => panic!("expected tool response"),
    }
}

async fn attest(gcx: Arc<crate::global_context::GlobalContext>) -> Result<(), String> {
    ToolTaskBoardAuthority::new()
        .tool_execute(
            chat_ccx(gcx, "planner-reviewer", "planner").await,
            &"call".to_string(),
            &args("attest"),
        )
        .await
        .map(|_| ())
}

async fn finalize(gcx: Arc<crate::global_context::GlobalContext>) -> Result<(bool, Value), String> {
    let result = ToolTaskFinalizeCard::new()
        .tool_execute(
            chat_ccx(gcx, "planner-owner", "planner").await,
            &"call".to_string(),
            &HashMap::from([("card_id".to_string(), json!("T-1"))]),
        )
        .await?;
    let response: Value = serde_json::from_str(&text(result.1)).unwrap();
    Ok((response["finalized"] == json!(true), response))
}

#[tokio::test]
async fn reviewer_attest_then_owner_finalize_succeeds() {
    let (gcx, _temp) = setup_task(true, None).await;
    attest(gcx.clone()).await.unwrap();

    let (finalized, _) = finalize(gcx.clone()).await.unwrap();

    assert!(finalized);
    assert!(storage::load_board(gcx, "task-1")
        .await
        .unwrap()
        .card_is_finalized("T-1"));
}

#[tokio::test]
async fn finalize_without_attest_is_refused() {
    let (gcx, _temp) = setup_task(true, None).await;

    let (finalized, response) = finalize(gcx).await.unwrap();

    assert!(!finalized);
    assert!(response["decision"]["blocks"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("independent reviewer"));
}

#[tokio::test]
async fn owner_cannot_attest() {
    let (gcx, temp) = setup_task(true, None).await;
    let error = ToolTaskBoardAuthority::new()
        .tool_execute(
            chat_ccx(gcx.clone(), "planner-owner", "planner").await,
            &"call".to_string(),
            &args("attest"),
        )
        .await
        .unwrap_err();

    assert!(error.contains("independent assigned reviewer"));

    tokio::fs::write(
        temp.path().join(".refact/coordination.yaml"),
        "board_allow_self_review: true\n",
    )
    .await
    .unwrap();
    let error = ToolTaskBoardAuthority::new()
        .tool_execute(
            chat_ccx(gcx, "planner-owner", "planner").await,
            &"call".to_string(),
            &args("attest"),
        )
        .await
        .unwrap_err();
    assert!(error.contains("owner cannot attest"), "{error}");
}

#[tokio::test]
async fn unassigned_planner_cannot_attest() {
    let (gcx, _temp) = setup_task(true, None).await;
    let error = ToolTaskBoardAuthority::new()
        .tool_execute(
            chat_ccx(gcx, "planner-unassigned", "planner").await,
            &"call".to_string(),
            &args("attest"),
        )
        .await
        .unwrap_err();

    assert!(error.contains("owner or an assigned reviewer"));
}

#[tokio::test]
async fn failing_or_missing_report_cannot_be_attested() {
    let (gcx, _temp) = setup_task(false, None).await;
    let error = attest(gcx.clone()).await.unwrap_err();
    assert!(error.contains("no verifier report"));

    storage::update_board_atomic(gcx.clone(), "task-1", |board| {
        board.get_card_mut("T-1").unwrap().verifier_report = Some(verifier_report(false));
        Ok(())
    })
    .await
    .unwrap();
    let error = attest(gcx).await.unwrap_err();
    assert!(error.contains("failing verifier report"));
}

#[tokio::test]
async fn verifier_rerun_clears_attestation() {
    let (gcx, _temp) = setup_task(true, None).await;
    attest(gcx.clone()).await.unwrap();

    assert!(store_verifier_report(
        gcx.clone(),
        "task-1",
        "T-1",
        None,
        verifier_report(true),
        Vec::new(),
    )
    .await
    .unwrap());
    assert!(!storage::load_board(gcx.clone(), "task-1")
        .await
        .unwrap()
        .review_attestations
        .contains_key("T-1"));
    let (finalized, response) = finalize(gcx.clone()).await.unwrap();
    assert!(!finalized);
    assert!(response["decision"]["blocks"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("independent reviewer"));

    attest(gcx.clone()).await.unwrap();
    assert!(finalize(gcx).await.unwrap().0);
}

#[tokio::test]
async fn claimant_attest_respects_self_review_setting() {
    let (gcx, temp) = setup_task(true, Some("planner-reviewer")).await;
    let refused = attest(gcx.clone()).await.unwrap_err();
    assert!(refused.contains("claimant"));

    tokio::fs::write(
        temp.path().join(".refact/coordination.yaml"),
        "board_allow_self_review: true\n",
    )
    .await
    .unwrap();
    attest(gcx.clone()).await.unwrap();
    assert_eq!(
        storage::load_board(gcx, "task-1")
            .await
            .unwrap()
            .review_attestations["T-1"]
            .reviewer_chat_id,
        "planner-reviewer"
    );
}
