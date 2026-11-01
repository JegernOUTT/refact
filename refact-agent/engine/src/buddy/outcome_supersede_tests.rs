use std::path::Path;

use refact_buddy_core::operator_types::{
    ProjectIdentity, RunStatus, WorkItem, WorkItemState, WorkerIdentity,
};
use tempfile::TempDir;

use super::{
    build_workflow_outcome, publish_report_in_store, OutcomeRuntime, SubjectKey,
    WorkflowOutcomeCheckpoint, WorkflowOutcomeStart, WorkflowTerminal,
};

fn identity(root: &Path) -> ProjectIdentity {
    ProjectIdentity {
        project_id: "supersede-fixture".into(),
        canonical_root: dunce::canonicalize(root)
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

fn runtime(root: &TempDir) -> OutcomeRuntime {
    OutcomeRuntime::open(root.path(), identity(root.path()), worker()).unwrap()
}

fn start(
    runtime: &OutcomeRuntime,
    run_id: &str,
    workflow: &str,
    coverage: &str,
) -> WorkflowOutcomeStart {
    start_at(runtime, run_id, workflow, coverage, "2026-09-01T00:00:00Z")
}

fn start_at(
    runtime: &OutcomeRuntime,
    run_id: &str,
    workflow: &str,
    coverage: &str,
    started_at: &str,
) -> WorkflowOutcomeStart {
    let mut start = runtime.start_workflow(workflow, "2026-09-01T00:00:00Z".into());
    start.run_id = run_id.into();
    start.coverage = coverage.into();
    start.input_fingerprint = run_id.into();
    start.started_at = started_at.into();
    start
}

async fn publish(
    runtime: &OutcomeRuntime,
    run_id: &str,
    workflow: &str,
    coverage: &str,
    terminal: WorkflowTerminal,
) -> WorkItem {
    publish_at(
        runtime,
        run_id,
        workflow,
        coverage,
        "2026-09-01T00:00:00Z",
        terminal,
    )
    .await
}

async fn publish_at(
    runtime: &OutcomeRuntime,
    run_id: &str,
    workflow: &str,
    coverage: &str,
    started_at: &str,
    terminal: WorkflowTerminal,
) -> WorkItem {
    runtime
        .publish_workflow_result(
            start_at(runtime, run_id, workflow, coverage, started_at),
            format!("Readable report for {run_id}."),
            terminal,
        )
        .await
        .unwrap()
        .value
}

fn open_state(state: &WorkItemState) -> bool {
    matches!(
        state,
        WorkItemState::NeedsAttention | WorkItemState::Watching | WorkItemState::Blocked
    )
}

async fn items(runtime: &OutcomeRuntime) -> Vec<WorkItem> {
    runtime
        .list_work_items(None, 200)
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|stored| stored.value)
        .collect()
}

fn publish_at_store(
    store: &mut super::super::operator_store::OperatorStore,
    start: WorkflowOutcomeStart,
    terminal: WorkflowTerminal,
    ended_at: &str,
) -> Result<super::super::operator_store::Stored<WorkItem>, String> {
    let subject = SubjectKey::new(
        &start.project,
        "workflow",
        &start.workflow,
        &start.coverage,
        &start.input_fingerprint,
    )
    .map_err(|error| error.to_string())?;
    let mut outcome = build_workflow_outcome(
        start,
        WorkflowOutcomeCheckpoint::from_evidence([]),
        terminal,
        ended_at.into(),
    );
    outcome.work_id = subject.work_item_id();
    publish_report_in_store(
        store,
        outcome,
        subject,
        "Workflow report".into(),
        "A readable workflow result.".into(),
    )
}

#[tokio::test]
async fn duplicate_and_out_of_order_publications_do_not_change_newer_rows() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let same = start(&runtime, "same-run", "commit_msg", "commit range");
    let first = runtime
        .publish_assessment(
            same.clone(),
            "Same report".into(),
            "Same report body".into(),
            vec![],
        )
        .await
        .unwrap();
    let before = runtime
        .with_store(|store| {
            Ok(store
                .diagnostic_summary()?
                .growth
                .max_seq
                .unwrap_or_default())
        })
        .await
        .unwrap();
    let duplicate = runtime
        .publish_assessment(
            same,
            "Same report".into(),
            "Same report body".into(),
            vec![],
        )
        .await
        .unwrap();
    let after = runtime
        .with_store(|store| {
            Ok(store
                .diagnostic_summary()?
                .growth
                .max_seq
                .unwrap_or_default())
        })
        .await
        .unwrap();
    assert_eq!(first, duplicate);
    assert_eq!(before, after);

    let future_start = start_at(
        &runtime,
        "future-run",
        "commit_msg",
        "commit range",
        "2030-01-01T00:00:00Z",
    );
    runtime
        .with_store(move |store| {
            publish_at_store(
                store,
                future_start,
                WorkflowTerminal::Inconclusive,
                "2030-01-01T00:00:00Z",
            )
            .map(|_| ())
            .map_err(|_| {
                super::super::operator_store::OperatorStoreError::Conflict("publication failed")
            })
        })
        .await
        .unwrap();
    let older_start = start_at(
        &runtime,
        "late-old-run",
        "commit_msg",
        "commit range",
        "2020-01-01T00:00:00Z",
    );
    runtime
        .with_store(move |store| {
            publish_at_store(
                store,
                older_start,
                WorkflowTerminal::Inconclusive,
                "2020-01-01T00:00:00Z",
            )
            .map(|_| ())
            .map_err(|_| {
                super::super::operator_store::OperatorStoreError::Conflict("publication failed")
            })
        })
        .await
        .unwrap();
    let all = items(&runtime).await;
    assert_eq!(all.iter().filter(|item| open_state(&item.state)).count(), 1);
    let future = runtime
        .get_work_item(
            SubjectKey::new(
                &identity(root.path()),
                "workflow",
                "commit_msg",
                "commit range",
                "future-run",
            )
            .unwrap()
            .work_item_id(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(future.value.state, WorkItemState::NeedsAttention);
    let late = runtime
        .get_work_item(
            SubjectKey::new(
                &identity(root.path()),
                "workflow",
                "commit_msg",
                "commit range",
                "late-old-run",
            )
            .unwrap()
            .work_item_id(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(late.value.state, WorkItemState::Cancelled);
    runtime.shutdown().await;
}

#[test]
fn interrupted_run_recovery_retires_older_open_failures() {
    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let worker = worker();
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let old_terminal = WorkflowTerminal::Failed {
        failure_class: Some("provider_transient".into()),
    };
    for run_id in ["recovery-old-1", "recovery-old-2"] {
        let mut active = WorkflowOutcomeStart {
            run_id: run_id.into(),
            work_id: String::new(),
            parent_run_id: None,
            project: project.clone(),
            worker: worker.clone(),
            origin: "buddy_workflow".into(),
            workflow: "dependency_radar".into(),
            input_fingerprint: run_id.into(),
            coverage: "manifest files".into(),
            started_at: "2026-09-01T00:00:00Z".into(),
        };
        let subject = SubjectKey::new(
            &project,
            "workflow",
            &active.workflow,
            &active.coverage,
            &active.input_fingerprint,
        )
        .unwrap();
        active.work_id = subject.work_item_id();
        publish_at_store(
            &mut store,
            active,
            old_terminal.clone(),
            "2026-09-01T00:01:00Z",
        )
        .unwrap();
    }
    for run_id in ["recovery-old-1", "recovery-old-2"] {
        let id = SubjectKey::new(
            &project,
            "workflow",
            "dependency_radar",
            "manifest files",
            run_id,
        )
        .unwrap()
        .work_item_id();
        let mut item = store.get_work_item(&id).unwrap().unwrap();
        item.value.state = WorkItemState::Blocked;
        store
            .upsert_work_item(&item.value, Some(item.revision))
            .unwrap();
    }
    let active = WorkflowOutcomeStart {
        run_id: "recovery-current".into(),
        work_id: SubjectKey::new(
            &project,
            "workflow",
            "dependency_radar",
            "manifest files",
            "recovery-current",
        )
        .unwrap()
        .work_item_id(),
        parent_run_id: None,
        project: project.clone(),
        worker: worker.clone(),
        origin: "buddy_workflow".into(),
        workflow: "dependency_radar".into(),
        input_fingerprint: "recovery-current".into(),
        coverage: "manifest files".into(),
        started_at: "2026-09-02T00:00:00Z".into(),
    };
    store.begin_workflow(&active).unwrap();
    drop(store);

    let store = super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let current = store.get_run_outcome("recovery-current").unwrap().unwrap();
    assert_eq!(current.value.status, RunStatus::Blocked);
    for run_id in ["recovery-old-1", "recovery-old-2"] {
        let id = SubjectKey::new(
            &project,
            "workflow",
            "dependency_radar",
            "manifest files",
            run_id,
        )
        .unwrap()
        .work_item_id();
        assert_eq!(
            store.get_work_item(&id).unwrap().unwrap().value.state,
            WorkItemState::Cancelled
        );
        assert!(store
            .get_run_outcome(run_id)
            .unwrap()
            .is_some_and(|outcome| outcome.value.status == RunStatus::Failed));
        assert!(store
            .get_artifact_revision(
                store
                    .get_work_item(&id)
                    .unwrap()
                    .unwrap()
                    .value
                    .current_artifact_id
                    .as_deref()
                    .unwrap(),
                Some(1),
            )
            .unwrap()
            .is_some());
    }
}
