use std::path::Path;
use std::sync::Arc;

use refact_work_core::store::records::{
    now_ms, DisagreementPosition, DisagreementRecord, DisagreementStatus, EvidenceKind, EvidenceRef,
};
use refact_work_core::{DisagreementFiling, ProjectKey, WorkKey};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::chat::goal_verifier::{run_goal_verification, GoalVerdict};
use crate::chat::types::{ChatSession, GoalBudget};
use crate::coordination::disagreement::ledger_for;

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
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
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

async fn test_context(
    enabled: bool,
    blocks_completion: bool,
    record_id: &str,
) -> (
    tempfile::TempDir,
    AppState,
    Arc<AMutex<ChatSession>>,
    crate::tools::settings_guard::SettingsGuard,
) {
    crate::runtime_settings::reset_for_test();
    let settings = override_settings(enabled, blocks_completion);
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let root = dir.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let app = app_for(&root, &cache).await;
    if !record_id.is_empty() {
        file_disagreement(&app, record_id).await;
    }
    let mut session = ChatSession::new("goal-disagreement-test".into());
    session.install_goal("agent", "complete the task", true, GoalBudget::default());
    (dir, app, Arc::new(AMutex::new(session)), settings)
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn open_disagreement_blocks_goal_without_the_judge() {
    let (_dir, app, session, _settings) = test_context(true, true, "goal-open").await;

    let reply = run_goal_verification(app, session).await.unwrap();

    let GoalVerdict::Unmet(gaps) = reply.verdict else {
        panic!("expected disagreement to leave the goal unmet");
    };
    assert!(gaps.iter().any(|gap| gap.contains("goal-open")), "{gaps:?}");
    assert_eq!(reply.tokens, 0);
}
