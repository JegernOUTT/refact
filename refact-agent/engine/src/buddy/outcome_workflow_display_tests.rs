use std::path::Path;

use refact_buddy_core::operator_types::{
    ProjectIdentity, RunStatus, WorkItem, WorkItemState, WorkerIdentity,
};
use tempfile::TempDir;

use super::{
    build_workflow_outcome, publish_report_in_store, OutcomeRuntime, WorkflowOutcomeCheckpoint,
    SubjectKey, WorkflowOutcomeStart, WorkflowTerminal,
};

fn identity(root: &Path) -> ProjectIdentity {
    ProjectIdentity {
        project_id: "workflow-display-fixture".into(),
        canonical_root: std::fs::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    }
}

fn worker() -> WorkerIdentity {
    WorkerIdentity {
        worker_id: "test-worker".into(),
        engine_instance_id: "test-engine".into(),
        epoch: 1,
    }
}

fn publish_seed(
    store: &mut super::super::operator_store::OperatorStore,
    project: &ProjectIdentity,
    run_id: &str,
    started_at: &str,
    workflow: &str,
    coverage: &str,
) -> WorkItem {
    let start = WorkflowOutcomeStart {
        run_id: run_id.into(),
        work_id: String::new(),
        parent_run_id: None,
        project: project.clone(),
        worker: worker(),
        origin: "buddy_workflow".into(),
        workflow: workflow.into(),
        input_fingerprint: format!("legacy:{run_id}"),
        coverage: coverage.into(),
        started_at: started_at.into(),
    };
    let subject = SubjectKey::new(
        project,
        "workflow",
        workflow,
        coverage,
        &start.input_fingerprint,
    )
    .unwrap();
    let mut outcome = build_workflow_outcome(
        start,
        WorkflowOutcomeCheckpoint::from_evidence([]),
        WorkflowTerminal::Inconclusive,
        started_at.into(),
    );
    outcome.work_id = subject.work_item_id();
    super::super::supersede::with_publication_bypass(|| {
        publish_report_in_store(
            store,
            outcome,
            subject,
            "Dependency Radar assessment".into(),
            format!("Retained report for {run_id}."),
        )
    })
    .unwrap()
    .value
}

fn mark_legacy_scope(store: &mut super::super::operator_store::OperatorStore, id: &str) {
    let mut item = store.get_work_item(id).unwrap().unwrap();
    item.value.scope = Some("workflow invocation and terminal result".into());
    store
        .upsert_work_item(&item.value, Some(item.revision))
        .unwrap();
}

#[tokio::test]
async fn interrupted_publication_has_display_title_truthful_conclusion_and_original_state() {
    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let runtime = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    let mut start = runtime.start_workflow("memo_extraction", "2026-09-01T00:00:00Z".into());
    start.run_id = "interrupted-run".into();
    start.coverage = "workflow invocation and terminal result".into();
    start.input_fingerprint = "interrupted-run".into();
    runtime
        .with_store(move |store| {
            store.begin_workflow(&start)?;
            Ok(())
        })
        .await
        .unwrap();
    runtime.shutdown().await;

    let interrupted = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    let outcome = interrupted
        .get_run_outcome("interrupted-run".into())
        .await
        .unwrap()
        .unwrap();
    let item = interrupted
        .get_work_item(outcome.value.work_id.clone())
        .await
        .unwrap()
        .unwrap();
    let artifact = interrupted
        .get_artifact_revision(
            item.value.current_artifact_id.clone().unwrap(),
            item.value.current_artifact_revision,
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(outcome.value.status, RunStatus::Blocked);
    assert_eq!(item.value.state, WorkItemState::Blocked);
    assert_eq!(item.value.title, "Memo extraction — interrupted");
    assert_eq!(
        item.value.scope.as_deref(),
        Some("Latest run of Memo extraction")
    );
    assert_eq!(
        artifact.value.conclusion,
        "The engine stopped before this run finished; nothing it was checking was confirmed."
    );
    assert!(!artifact.value.conclusion.contains("blocked"));
    interrupted.shutdown().await;
}

#[tokio::test]
async fn old_coverage_rows_group_with_display_scope_on_publication_and_open() {
    let publication_root = TempDir::new().unwrap();
    let project = identity(publication_root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(publication_root.path(), &project)
            .unwrap();
    let old = publish_seed(
        &mut store,
        &project,
        "publication-old",
        "2026-09-01T00:00:00Z",
        "buddy_dependency_radar",
        "workflow invocation and terminal result",
    );
    mark_legacy_scope(&mut store, &old.id);
    let runtime = OutcomeRuntime::from_store(store, project.clone(), worker());
    let mut newest =
        runtime.start_workflow("buddy_dependency_radar", "2026-09-02T00:00:00Z".into());
    newest.run_id = "publication-new".into();
    newest.coverage = "workflow invocation and terminal result".into();
    newest.input_fingerprint = newest.run_id.clone();
    let new = runtime
        .publish_workflow_result(
            newest,
            "New assessment".into(),
            WorkflowTerminal::Inconclusive,
        )
        .await
        .unwrap();
    assert_eq!(
        new.value.scope.as_deref(),
        Some("Latest run of Dependency Radar")
    );
    assert_eq!(
        runtime
            .get_work_item(old.id.clone())
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    assert_eq!(
        runtime
            .get_run_outcome("publication-new".into())
            .await
            .unwrap()
            .unwrap()
            .value
            .coverage,
        "workflow invocation and terminal result"
    );
    runtime.shutdown().await;

    let backlog_root = TempDir::new().unwrap();
    let project = identity(backlog_root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(backlog_root.path(), &project).unwrap();
    let old = publish_seed(
        &mut store,
        &project,
        "backlog-old",
        "2026-09-01T00:00:00Z",
        "buddy_dependency_radar",
        "workflow invocation and terminal result",
    );
    let newer = publish_seed(
        &mut store,
        &project,
        "backlog-new",
        "2026-09-02T00:00:00Z",
        "buddy_dependency_radar",
        "workflow invocation and terminal result",
    );
    mark_legacy_scope(&mut store, &old.id);
    mark_legacy_scope(&mut store, &newer.id);
    drop(store);

    let reopened = OutcomeRuntime::open(backlog_root.path(), project, worker()).unwrap();
    assert_eq!(
        reopened
            .get_work_item(old.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    assert_eq!(
        reopened
            .get_work_item(newer.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::NeedsAttention
    );
    reopened.shutdown().await;
}

#[test]
fn display_scope_does_not_change_coverage_identity() {
    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let workflow = "buddy_dependency_radar";
    let coverage = "workflow invocation and terminal result";
    let input_fingerprint = "workflow:buddy_dependency_radar";
    let subject =
        SubjectKey::new(&project, "workflow", workflow, coverage, input_fingerprint).unwrap();
    assert_eq!(
        super::super::workflow_display::workflow_display_scope(workflow),
        "Latest run of Dependency Radar"
    );
    assert_eq!(
        SubjectKey::new(
            &project,
            "workflow",
            workflow,
            "workflow invocation and terminal result",
            input_fingerprint,
        )
        .unwrap()
        .work_item_id(),
        subject.work_item_id()
    );
}

fn set_work_fields(
    store: &mut super::super::operator_store::OperatorStore,
    id: &str,
    title: &str,
    state: WorkItemState,
    scope: Option<&str>,
) {
    let mut stored = store.get_work_item(id).unwrap().unwrap();
    stored.value.title = title.into();
    stored.value.state = state;
    stored.value.scope = scope.map(str::to_owned);
    store
        .upsert_work_item(&stored.value, Some(stored.revision))
        .unwrap();
}

#[tokio::test]
async fn stored_workflow_display_fields_backfill_once_without_touching_identity_or_evidence() {
    use refact_buddy_core::operator_types::WorkItemSeverity;
    use super::super::work_items::WorkItemDraft;

    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let security_coverage = "Secret-pattern scan of the workspace under [REDACTED_PATH] (text files up to 65536 bytes), plus 11 recent diagnostics";
    let security = publish_seed(
        &mut store,
        &project,
        "security-whisperer-old",
        "2026-09-01T00:00:00Z",
        "security_whisperer",
        security_coverage,
    );
    set_work_fields(
        &mut store,
        &security.id,
        &security.title,
        security.state.clone(),
        Some(security_coverage),
    );
    let optimizer = publish_seed(
        &mut store,
        &project,
        "model-cost-optimizer-old",
        "2026-09-02T00:00:00Z",
        "buddy_model_cost_optimizer",
        "workflow invocation and terminal result",
    );
    set_work_fields(
        &mut store,
        &optimizer.id,
        &optimizer.title,
        optimizer.state.clone(),
        Some("Assessment of workflow signal buddy_model_cost_optimizer"),
    );
    let delivered = publish_seed(
        &mut store,
        &project,
        "delivered-workflow-old",
        "2026-09-03T00:00:00Z",
        "buddy_dependency_radar",
        "delivered workflow coverage",
    );
    set_work_fields(
        &mut store,
        &delivered.id,
        &delivered.title,
        WorkItemState::Delivered,
        None,
    );
    let interrupted = publish_seed(
        &mut store,
        &project,
        "memo-extraction-old",
        "2026-09-04T00:00:00Z",
        "memo_extraction",
        "interrupted workflow coverage",
    );
    set_work_fields(
        &mut store,
        &interrupted.id,
        "memo_extraction — interrupted run",
        WorkItemState::Blocked,
        Some("Assessment of workflow signal memo_extraction"),
    );

    let legacy_subject = SubjectKey::new(
        &project,
        "legacy_opportunity",
        "legacy-display-fixture",
        "legacy-import-v1",
        "1",
    )
    .unwrap();
    let legacy = WorkItemDraft {
        project: project.clone(),
        subject: legacy_subject,
        kind: "legacy_opportunity".into(),
        title: "Legacy proposal title".into(),
        state: WorkItemState::Watching,
        native_sources: vec![],
        last_material_change_at: "2026-09-05T00:00:00Z".into(),
        severity: Some(WorkItemSeverity::Unknown),
        problem_summary: None,
        scope: Some("Legacy scope".into()),
    }
    .build()
    .unwrap();
    let legacy = store.upsert_work_item(&legacy, None).unwrap();

    let ids = [security.id, optimizer.id, delivered.id, interrupted.id];
    let before = ids
        .iter()
        .map(|id| {
            let work = store.get_work_item(id).unwrap().unwrap();
            let artifact = store
                .get_artifact_revision(
                    work.value.current_artifact_id.as_deref().unwrap(),
                    work.value.current_artifact_revision,
                )
                .unwrap()
                .unwrap();
            (work, artifact)
        })
        .collect::<Vec<_>>();
    drop(store);

    let opened = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    let expected_workflows = [
        "security_whisperer",
        "buddy_model_cost_optimizer",
        "buddy_dependency_radar",
        "memo_extraction",
    ];
    let mut after_first_open = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let current = opened.get_work_item(id.clone()).await.unwrap().unwrap();
        assert_eq!(
            current.value.scope.as_deref(),
            Some(
                super::super::workflow_display::workflow_display_scope(expected_workflows[index])
                    .as_str()
            )
        );
        assert_eq!(
            current.value.title,
            if index == 3 {
                "Memo extraction — interrupted"
            } else {
                before[index].0.value.title.as_str()
            }
        );
        assert_eq!(current.value.state, before[index].0.value.state);
        assert_eq!(
            current.value.last_material_change_at,
            before[index].0.value.last_material_change_at
        );
        assert_eq!(current.value.subject, before[index].0.value.subject);
        assert_eq!(current.value.severity, before[index].0.value.severity);
        assert_eq!(
            current.value.problem_summary,
            before[index].0.value.problem_summary
        );
        assert_eq!(
            current.value.current_artifact_id,
            before[index].0.value.current_artifact_id
        );
        assert_eq!(
            current.value.current_artifact_revision,
            before[index].0.value.current_artifact_revision
        );
        let artifact = opened
            .get_artifact_revision(
                current.value.current_artifact_id.clone().unwrap(),
                current.value.current_artifact_revision,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(artifact, before[index].1);
        after_first_open.push(current);
    }
    assert_eq!(
        opened
            .get_work_item(legacy.value.id.clone())
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    opened.shutdown().await;

    let reopened = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    for (id, previous) in ids.iter().zip(after_first_open) {
        assert_eq!(
            reopened.get_work_item(id.clone()).await.unwrap().unwrap(),
            previous
        );
    }
    assert_eq!(
        reopened
            .get_work_item(legacy.value.id.clone())
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    reopened.shutdown().await;
}
