use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use refact_work_core::store::records::{
    now_ms, DisagreementPosition, DisagreementRecord, DisagreementStatus, EvidenceKind, EvidenceRef,
};
use refact_work_core::{DisagreementFiling, ProjectKey, WorkKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::disagreement::{completion_gate_blocks, ledger_for};
use crate::app_state::AppState;
use crate::chat::evidence::EvidenceVerdict;
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::chat::verifier::{verify_card_as_planner, VerifyCardRequest};
use crate::llm::WireFormat;
use crate::tasks::storage;
use crate::tasks::types::{
    BoardCard, FinalReport, ScopeGuardMode, TaskBoard, TaskMeta, TaskStatus,
    VerifierReportClassification, VerificationOutcome, VerificationResult,
};

const UNAVAILABLE_REASON: &str =
    "disagreement ledger unavailable; cannot confirm there is no open dissent";
const TASK_ID: &str = "task-disagreement-gate";
const CARD_ID: &str = "T-disagreement";

fn override_settings(
    enabled: bool,
    blocks_completion: bool,
) -> crate::tools::settings_guard::SettingsGuard {
    crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.enabled = enabled;
        settings.coordination.disagreement_blocks_completion = blocks_completion;
    })
}

async fn app_for(root: &Path, cache_dir: &Path) -> AppState {
    let config_dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        cache_dir.to_path_buf(),
        config_dir.path().to_path_buf(),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() =
        vec![dunce::canonicalize(root).unwrap()];
    AppState::from_gcx(gcx).await
}

fn position(position_id: &str, author: &str, statement: &str) -> DisagreementPosition {
    DisagreementPosition {
        position_id: position_id.to_string(),
        author: WorkKey::background_agent(author),
        run_generation: 1,
        statement: statement.to_string(),
        evidence: vec![EvidenceRef::new(EvidenceKind::File, "src/lib.rs:1")],
        asserted_at_ms: now_ms(),
        subject_revision: "revision-1".to_string(),
    }
}

fn disagreement(app: &AppState, record_id: &str) -> DisagreementRecord {
    DisagreementRecord {
        project: ProjectKey::new(crate::coordination::roots::project_identity(
            &app.gcx.documents_state.workspace_folders.lock().unwrap()[0],
        )),
        record_id: record_id.to_string(),
        claim: "the operation keeps the lock across await".to_string(),
        subject: "src/lib.rs".to_string(),
        positions: vec![
            position("kept", "agent-a", "the lock remains held"),
            position("dropped", "agent-b", "the lock is dropped"),
        ],
        status: DisagreementStatus::Open,
        adjudication: None,
        open_reason: None,
        filed_at_ms: now_ms(),
        updated_at_ms: now_ms(),
    }
}

async fn file_disagreement(app: &AppState, record_id: &str) -> DisagreementRecord {
    let ledger = ledger_for(app)
        .await
        .expect("coordination ledger available");
    let record = disagreement(app, record_id);
    assert!(matches!(
        ledger.file(&record).await.unwrap(),
        DisagreementFiling::Filed(_)
    ));
    record
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn policy_on_unavailable_ledger_fails_closed() {
    crate::runtime_settings::reset_for_test();
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let app = app_for(&root, &cache).await;
    std::fs::write(cache.join("coordination"), "not a directory").unwrap();
    let _settings = override_settings(true, true);

    let blocks = completion_gate_blocks(&app).await;

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].spec_id, "disagreement:unavailable");
    assert_eq!(blocks[0].verdict, EvidenceVerdict::Unavailable);
    assert_eq!(blocks[0].reason, UNAVAILABLE_REASON);
}

fn card(worktree: &Path, base_branch: &str) -> BoardCard {
    let verify_command = "git --version";
    BoardCard {
        id: CARD_ID.to_string(),
        title: "Disagreement gated card".to_string(),
        column: "done".to_string(),
        priority: "P1".to_string(),
        depends_on: Vec::new(),
        instructions: format!("## Acceptance Criteria\n- Verify: `{verify_command}`"),
        assignee: None,
        agent_chat_id: Some("agent-disagreement".to_string()),
        status_updates: Vec::new(),
        comments: Vec::new(),
        final_report: Some("done".to_string()),
        final_report_structured: Some(FinalReport {
            success: true,
            verification: vec![VerificationResult {
                command: verify_command.to_string(),
                exit_code: Some(0),
                passed: true,
                outcome: VerificationOutcome::Passed,
                ..Default::default()
            }],
            ..Default::default()
        }),
        verifier_report: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        started_at: None,
        last_heartbeat_at: None,
        paused_at: None,
        paused_by: None,
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        agent_branch: None,
        agent_worktree: Some(worktree.to_string_lossy().to_string()),
        agent_worktree_name: None,
        base_branch: Some(base_branch.to_string()),
        base_commit: None,
        ab_variants: None,
        team_members: Vec::new(),
        target_files: Vec::new(),
        scope_guard_mode: ScopeGuardMode::Off,
        attention: None,
        deliverable: None,
        verification: Vec::new(),
    }
}

fn init_git(root: &Path) {
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init", "-b", "main"]);
    run(&["config", "core.autocrlf", "false"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
    run(&["add", "src.rs"]);
    run(&["commit", "-m", "initial"]);
}

async fn start_review_server() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let read = socket.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "request closed before headers arrived");
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        server_hits.fetch_add(1, Ordering::SeqCst);
        let response_body = b"data: {\"choices\":[{\"delta\":{\"content\":\"PASS\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len()
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.write_all(response_body).await.unwrap();
    });
    (
        format!("http://{address}/v1/chat/completions"),
        hits,
        server,
    )
}

async fn prepare_verify_card(root: &Path, cache: &Path, endpoint: &str) -> AppState {
    let app = app_for(root, cache).await;
    *app.gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
    let task_dir = root.join(".refact/tasks").join(TASK_ID);
    tokio::fs::create_dir_all(&task_dir).await.unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let meta = TaskMeta {
        schema_version: 1,
        id: TASK_ID.to_string(),
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
    };
    storage::save_task_meta(app.gcx.clone(), TASK_ID, &meta)
        .await
        .unwrap();
    storage::save_board(
        app.gcx.clone(),
        TASK_ID,
        &TaskBoard {
            cards: vec![card(root, "main")],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let model_id = "test/disagreement-verifier";
    let model = ChatModelRecord {
        base: BaseModelRecord {
            id: model_id.to_string(),
            name: "disagreement-verifier".to_string(),
            endpoint: endpoint.to_string(),
            wire_format: WireFormat::OpenaiChatCompletions,
            tokenizer: "fake".to_string(),
            n_ctx: 8192,
            ..Default::default()
        },
        supports_tools: false,
        supports_temperature: false,
        ..Default::default()
    };
    let mut caps = CodeAssistantCaps::default();
    caps.chat_models = IndexMap::new();
    caps.chat_models
        .insert(model_id.to_string(), Arc::new(model));
    caps.defaults.chat_light_model = model_id.to_string();
    {
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let policy_path = crate::tools::shell_gate::policy_path(root);
    tokio::fs::create_dir_all(policy_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&policy_path, "mode: yolo\ndeny: []\n")
        .await
        .unwrap();
    app
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn verify_card_with_open_disagreement_does_not_pass() {
    crate::runtime_settings::reset_for_test();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&root).unwrap();
    let root = dunce::canonicalize(&root).unwrap();
    init_git(&root);

    let (endpoint, hits, server) = start_review_server().await;
    let app = prepare_verify_card(&root, &cache, &endpoint).await;
    file_disagreement(&app, "verify-open").await;
    let _strict = override_settings(true, true);
    let strict = verify_card_as_planner(
        crate::chat::verifier::tests::register_candidate(
            app.gcx.clone(),
            TASK_ID,
            CARD_ID,
            "test/disagreement-verifier",
        )
        .await,
        VerifyCardRequest {
            task_id: TASK_ID.to_string(),
            card_id: CARD_ID.to_string(),
            expected_state: None,
        },
    )
    .await
    .unwrap();
    assert!(!strict.passed);
    assert!(strict
        .concerns
        .iter()
        .any(|concern| concern.contains("verify-open")));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "blocked gates skip the judge"
    );
    let stored = storage::load_board(app.gcx.clone(), TASK_ID)
        .await
        .unwrap()
        .get_card(CARD_ID)
        .unwrap()
        .verifier_report
        .clone()
        .unwrap();
    assert!(!stored.passed);
    assert!(stored
        .concerns
        .iter()
        .any(|concern| concern.contains("verify-open")));
    assert_eq!(
        stored.classification,
        VerifierReportClassification::HumanReview
    );
    server.abort();
    let _ = server.await;

    drop(_strict);
    let (endpoint, hits, server) = start_review_server().await;
    let app = prepare_verify_card(&root, &cache, &endpoint).await;
    let _permissive = override_settings(true, false);
    let permissive = verify_card_as_planner(
        crate::chat::verifier::tests::register_candidate(
            app.gcx.clone(),
            TASK_ID,
            CARD_ID,
            "test/disagreement-verifier",
        )
        .await,
        VerifyCardRequest {
            task_id: TASK_ID.to_string(),
            card_id: CARD_ID.to_string(),
            expected_state: None,
        },
    )
    .await
    .unwrap();
    assert!(
        permissive.passed,
        "policy off preserves the prior pass path: {permissive:?}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let stored = storage::load_board(app.gcx.clone(), TASK_ID)
        .await
        .unwrap()
        .get_card(CARD_ID)
        .unwrap()
        .verifier_report
        .clone()
        .unwrap();
    assert!(stored.passed);
    server.abort();
    let _ = server.await;
}
