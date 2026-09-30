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

async fn set_state(runtime: &OutcomeRuntime, id: &str, state: WorkItemState) {
    let id = id.to_owned();
    runtime
        .with_store(move |store| {
            let mut item = store.get_work_item(&id)?.unwrap();
            item.value.state = state;
            store.upsert_work_item(&item.value, Some(item.revision))?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn force_all_open(runtime: &OutcomeRuntime, ids: &[String]) {
    for id in ids {
        set_state(runtime, id, WorkItemState::NeedsAttention).await;
    }
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
async fn newer_assessment_retires_thirteen_open_rows_and_keeps_their_history() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let mut old_ids = Vec::new();
    for index in 0..13 {
        let item = publish_at(
            &runtime,
            &format!("old-{index}"),
            "security_whisperer",
            "full project",
            &format!("2026-09-{:02}T00:00:00Z", index + 1),
            WorkflowTerminal::Inconclusive,
        )
        .await;
        old_ids.push(item.id);
    }
    force_all_open(&runtime, &old_ids).await;
    let artifacts = items(&runtime).await;
    let artifact_ids: Vec<_> = artifacts
        .iter()
        .filter(|item| old_ids.contains(&item.id))
        .map(|item| item.current_artifact_id.clone().unwrap())
        .collect();
    let newest = publish_at(
        &runtime,
        "newest",
        "security_whisperer",
        "full project",
        "2026-09-14T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;

    let items = items(&runtime).await;
    assert_eq!(
        items.iter().filter(|item| open_state(&item.state)).count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| open_state(&item.state))
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![newest.id.as_str()]
    );
    for id in &old_ids {
        let old = runtime.get_work_item(id.clone()).await.unwrap().unwrap();
        assert_eq!(old.value.state, WorkItemState::Cancelled);
        assert!(runtime
            .list_work_outcomes(id.clone(), None, 20)
            .await
            .unwrap()
            .entries
            .iter()
            .any(|outcome| outcome.value.status == RunStatus::Inconclusive));
    }
    for artifact_id in artifact_ids {
        assert!(runtime
            .get_artifact_revision(artifact_id, Some(1))
            .await
            .unwrap()
            .is_some());
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn failure_keeps_assessment_visible_until_a_later_assessment_supersedes_both() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let assessment = publish_at(
        &runtime,
        "assessment",
        "dependency_radar",
        "manifest files",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let failure = publish_at(
        &runtime,
        "failure",
        "dependency_radar",
        "manifest files",
        "2026-09-02T00:00:00Z",
        WorkflowTerminal::Failed {
            failure_class: Some("provider_transient".into()),
        },
    )
    .await;
    assert_eq!(assessment.state, WorkItemState::NeedsAttention);
    assert_eq!(failure.state, WorkItemState::Blocked);
    assert_eq!(
        items(&runtime)
            .await
            .iter()
            .filter(|item| open_state(&item.state))
            .count(),
        2
    );

    let success = publish_at(
        &runtime,
        "success",
        "dependency_radar",
        "manifest files",
        "2026-09-03T00:00:00Z",
        WorkflowTerminal::Completed,
    )
    .await;
    assert_eq!(success.state, WorkItemState::Delivered);
    assert_eq!(
        runtime
            .get_work_item(assessment.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    assert_eq!(
        runtime
            .get_work_item(failure.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn consecutive_failures_leave_only_the_latest_failure_open() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let first = publish_at(
        &runtime,
        "failed-first",
        "docs_gardener",
        "documentation",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Failed {
            failure_class: Some("tool_failed".into()),
        },
    )
    .await;
    let second = publish_at(
        &runtime,
        "failed-second",
        "docs_gardener",
        "documentation",
        "2026-09-02T00:00:00Z",
        WorkflowTerminal::Blocked {
            failure_class: Some("tool_unavailable".into()),
        },
    )
    .await;
    assert_eq!(
        runtime
            .get_work_item(first.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    assert_eq!(second.state, WorkItemState::Blocked);
    assert_eq!(
        items(&runtime)
            .await
            .iter()
            .filter(|item| open_state(&item.state))
            .count(),
        1
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn working_delivered_and_cancelled_siblings_are_untouched() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let mut ids = Vec::new();
    for (index, state) in [
        WorkItemState::Working,
        WorkItemState::Delivered,
        WorkItemState::Cancelled,
    ]
    .into_iter()
    .enumerate()
    {
        let item = publish(
            &runtime,
            &format!("preserved-{index}"),
            "architecture_drift",
            "source tree",
            WorkflowTerminal::Inconclusive,
        )
        .await;
        set_state(&runtime, &item.id, state.clone()).await;
        ids.push((item.id, state));
    }
    let current = publish_at(
        &runtime,
        "current-assessment",
        "architecture_drift",
        "source tree",
        "2026-09-02T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    for (id, state) in ids {
        assert_eq!(
            runtime
                .get_work_item(id)
                .await
                .unwrap()
                .unwrap()
                .value
                .state,
            state
        );
    }
    assert_eq!(current.state, WorkItemState::NeedsAttention);
    runtime.shutdown().await;
}

#[tokio::test]
async fn different_coverage_or_workflow_is_not_a_sibling() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let different_coverage = publish_at(
        &runtime,
        "other-coverage",
        "memory_gardener",
        "knowledge base",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let different_workflow = publish_at(
        &runtime,
        "other-workflow",
        "error_detective",
        "source tree",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let old = publish_at(
        &runtime,
        "same-scope-old",
        "memory_gardener",
        "source tree",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let new = publish_at(
        &runtime,
        "same-scope-new",
        "memory_gardener",
        "source tree",
        "2026-09-02T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    assert_eq!(
        runtime
            .get_work_item(different_coverage.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::NeedsAttention
    );
    assert_eq!(
        runtime
            .get_work_item(different_workflow.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::NeedsAttention
    );
    assert_eq!(
        runtime
            .get_work_item(old.id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Cancelled
    );
    assert_eq!(new.state, WorkItemState::NeedsAttention);
    runtime.shutdown().await;
}

#[tokio::test]
async fn numeric_coverage_changes_supersede_but_text_changes_and_failures_do_not() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let older = publish_at(
        &runtime,
        "diagnostics-five",
        "security_whisperer",
        "Secret-pattern scan under /project (text files up to 65536 bytes), plus 5 recent diagnostics",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let newer_failure = publish_at(
        &runtime,
        "diagnostics-eight-failure",
        "security_whisperer",
        "Secret-pattern scan under /project (text files up to 65536 bytes), plus 8 recent diagnostics",
        "2026-09-02T00:00:00Z",
        WorkflowTerminal::Failed {
            failure_class: Some("provider_transient".into()),
        },
    )
    .await;
    assert_eq!(older.state, WorkItemState::NeedsAttention);
    assert_eq!(newer_failure.state, WorkItemState::Blocked);

    let newer_assessment = publish_at(
        &runtime,
        "diagnostics-eight-assessment",
        "security_whisperer",
        "Secret-pattern scan under /project (text files up to 65536 bytes), plus 8 recent diagnostics",
        "2026-09-03T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    assert_eq!(newer_assessment.state, WorkItemState::NeedsAttention);
    let newer_assessment_id = newer_assessment.id.clone();
    for id in [older.id, newer_failure.id] {
        assert_eq!(
            runtime
                .get_work_item(id)
                .await
                .unwrap()
                .unwrap()
                .value
                .state,
            WorkItemState::Cancelled
        );
    }

    let other_root = publish_at(
        &runtime,
        "other-root",
        "security_whisperer",
        "Secret-pattern scan under /other/root (text files up to 65536 bytes), plus 9 recent diagnostics",
        "2026-09-04T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    assert_eq!(other_root.state, WorkItemState::NeedsAttention);
    assert_eq!(
        runtime
            .get_work_item(newer_assessment_id)
            .await
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::NeedsAttention
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn equal_observation_time_uses_lexicographically_larger_work_id_as_newer() {
    let root = TempDir::new().unwrap();
    let runtime = runtime(&root);
    let left = publish_at(
        &runtime,
        "tie-left",
        "tie_break_workflow",
        "same scope",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let right = publish_at(
        &runtime,
        "tie-right",
        "tie_break_workflow",
        "same scope",
        "2026-09-01T00:00:00Z",
        WorkflowTerminal::Inconclusive,
    )
    .await;
    let expected_newest = left.id.max(right.id);
    let rows = items(&runtime).await;
    assert_eq!(rows.iter().filter(|row| open_state(&row.state)).count(), 1);
    assert_eq!(
        rows.iter().find(|row| open_state(&row.state)).unwrap().id,
        expected_newest
    );
    runtime.shutdown().await;
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

#[tokio::test]
async fn assessment_failure_assessment_result_is_independent_of_publication_order() {
    let scenarios = [
        ("assessment-1", "2026-01-01T00:00:00Z", false),
        ("failure-2", "2026-01-02T00:00:00Z", true),
        ("assessment-3", "2026-01-03T00:00:00Z", false),
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let root = TempDir::new().unwrap();
        let runtime = runtime(&root);
        let mut ids = Vec::new();
        for index in order {
            let (run_id, started_at, failed) = scenarios[index];
            let terminal = if failed {
                WorkflowTerminal::Failed {
                    failure_class: Some("provider_transient".into()),
                }
            } else {
                WorkflowTerminal::Inconclusive
            };
            let item = publish_at(
                &runtime,
                run_id,
                "order_test",
                "same scope",
                started_at,
                terminal,
            )
            .await;
            ids.push(item.id);
        }
        let final_items = items(&runtime).await;
        let open: Vec<_> = final_items
            .iter()
            .filter(|item| open_state(&item.state))
            .map(|item| item.title.clone())
            .collect();
        assert_eq!(open.len(), 1);
        assert!(open[0].contains("assessment"));
        for id in ids {
            assert!(runtime.get_work_item(id.clone()).await.unwrap().is_some());
            assert_eq!(
                runtime
                    .list_work_outcomes(id.clone(), None, 10)
                    .await
                    .unwrap()
                    .entries
                    .len(),
                1
            );
            let work = runtime.get_work_item(id).await.unwrap().unwrap();
            assert!(work.value.current_artifact_id.is_some());
        }
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn newer_failure_does_not_supersede_assessment_in_either_order() {
    for order in [[0, 1], [1, 0]] {
        let root = TempDir::new().unwrap();
        let runtime = runtime(&root);
        let mut ids = Vec::new();
        for index in order {
            let (run_id, started_at, terminal) = if index == 0 {
                (
                    "assessment-old",
                    "2026-01-01T00:00:00Z",
                    WorkflowTerminal::Inconclusive,
                )
            } else {
                (
                    "failure-new",
                    "2026-01-02T00:00:00Z",
                    WorkflowTerminal::Failed {
                        failure_class: Some("provider_transient".into()),
                    },
                )
            };
            ids.push(
                publish_at(
                    &runtime,
                    run_id,
                    "failure_test",
                    "same scope",
                    started_at,
                    terminal,
                )
                .await
                .id,
            );
        }
        let open_items: Vec<_> = items(&runtime)
            .await
            .into_iter()
            .filter(|item| open_state(&item.state))
            .map(|item| item.id)
            .collect();
        assert_eq!(open_items.len(), 2);
        assert!(open_items.iter().all(|id| ids.contains(id)));
        for id in ids {
            assert!(runtime.get_work_item(id).await.unwrap().is_some());
        }
        runtime.shutdown().await;
    }
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
