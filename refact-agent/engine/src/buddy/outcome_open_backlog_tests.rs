use std::path::Path;
use std::time::Instant;

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
        project_id: "open-backlog-fixture".into(),
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

fn start(
    project: &ProjectIdentity,
    run_id: &str,
    workflow: &str,
    coverage: &str,
    started_at: &str,
) -> WorkflowOutcomeStart {
    WorkflowOutcomeStart {
        run_id: run_id.into(),
        work_id: String::new(),
        parent_run_id: None,
        project: project.clone(),
        worker: worker(),
        origin: "buddy_workflow".into(),
        workflow: workflow.into(),
        input_fingerprint: run_id.into(),
        coverage: coverage.into(),
        started_at: started_at.into(),
    }
}

fn seed(
    store: &mut super::super::operator_store::OperatorStore,
    project: &ProjectIdentity,
    run_id: &str,
    workflow: &str,
    coverage: &str,
    started_at: &str,
    terminal: WorkflowTerminal,
) -> WorkItem {
    let start = start(project, run_id, workflow, coverage, started_at);
    let subject = SubjectKey::new(project, "workflow", workflow, coverage, run_id).unwrap();
    let mut outcome = build_workflow_outcome(
        start,
        WorkflowOutcomeCheckpoint::from_evidence([]),
        terminal,
        started_at.into(),
    );
    outcome.work_id = subject.work_item_id();
    publish_report_in_store(
        store,
        outcome,
        subject,
        "Workflow report".into(),
        "A readable workflow result.".into(),
    )
    .unwrap()
    .value
}

fn seed_backlog(
    root: &TempDir,
    group_count: usize,
    rows_per_group: usize,
) -> (ProjectIdentity, Vec<Vec<String>>) {
    let project = identity(root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let mut ids = Vec::new();
    for group in 0..group_count {
        let workflow = format!("workflow_{group:02}");
        let mut group_ids = Vec::new();
        for row in 0..rows_per_group {
            let day = row + 1;
            let terminal = if row < 2 {
                WorkflowTerminal::Failed {
                    failure_class: Some("provider_transient".into()),
                }
            } else {
                WorkflowTerminal::Inconclusive
            };
            let run_id = format!("g{group:02}-r{row:02}");
            let item = super::super::supersede::with_publication_bypass(|| {
                seed(
                    &mut store,
                    &project,
                    &run_id,
                    &workflow,
                    "whole project",
                    &format!("2026-09-{day:02}T00:00:00Z"),
                    terminal,
                )
            });
            let mut current = store.get_work_item(&item.id).unwrap().unwrap();
            current.value.state = if row < 2 {
                WorkItemState::Blocked
            } else {
                WorkItemState::NeedsAttention
            };
            store
                .upsert_work_item(&current.value, Some(current.revision))
                .unwrap();
            group_ids.push(item.id);
        }
        ids.push(group_ids);
    }
    (project, ids)
}

fn open_count(items: &[super::super::operator_store::Stored<WorkItem>]) -> usize {
    items
        .iter()
        .filter(|row| {
            matches!(
                row.value.state,
                WorkItemState::NeedsAttention | WorkItemState::Watching | WorkItemState::Blocked
            )
        })
        .count()
}

async fn sequence(runtime: &OutcomeRuntime) -> u64 {
    runtime
        .with_store(|store| {
            Ok(store
                .diagnostic_summary()?
                .growth
                .max_seq
                .unwrap_or_default())
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn open_collapses_backlog_retains_history_and_is_idempotent() {
    let root = TempDir::new().unwrap();
    let (project, groups) = seed_backlog(&root, 1, 15);
    let runtime = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    let rows = runtime.list_work_items(None, 100).await.unwrap().entries;
    assert_eq!(open_count(&rows), 1);
    let newest = rows
        .iter()
        .find(|row| row.value.id == groups[0][14])
        .unwrap();
    assert_eq!(newest.value.state, WorkItemState::NeedsAttention);
    for id in &groups[0][..14] {
        let row = runtime.get_work_item(id.clone()).await.unwrap().unwrap();
        assert_eq!(row.value.state, WorkItemState::Cancelled);
        let outcomes = runtime
            .list_work_outcomes(id.clone(), None, 10)
            .await
            .unwrap();
        assert_eq!(outcomes.entries.len(), 1);
        assert!(matches!(
            outcomes.entries[0].value.status,
            RunStatus::Inconclusive | RunStatus::Failed
        ));
        assert!(runtime
            .get_artifact_revision(row.value.current_artifact_id.unwrap(), Some(1))
            .await
            .unwrap()
            .is_some());
    }
    let before = sequence(&runtime).await;
    runtime.shutdown().await;
    let reopened = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    assert_eq!(sequence(&reopened).await, before);
    reopened.shutdown().await;
}

#[tokio::test]
async fn disabled_open_pass_leaves_the_seeded_backlog_open() {
    let root = TempDir::new().unwrap();
    let (project, _) = seed_backlog(&root, 1, 15);
    let runtime = super::super::supersede::with_open_pass_disabled(|| {
        OutcomeRuntime::open(root.path(), project, worker()).unwrap()
    });
    let rows = runtime.list_work_items(None, 100).await.unwrap().entries;
    assert_eq!(
        open_count(&rows),
        15,
        "with the open pass disabled the seeded backlog must stay open"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn interrupted_group_commit_is_visible_and_next_open_completes_once() {
    let root = TempDir::new().unwrap();
    let (project, groups) = seed_backlog(&root, 4, 4);
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let original_max_seq = store.diagnostic_summary().unwrap().growth.max_seq.unwrap();
    let interrupted = super::super::supersede::with_interruption_after_groups(1, || {
        store.retire_superseded_backlog()
    });
    assert!(interrupted.is_err());
    drop(store);

    let partial = super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let first_group_seq: Vec<_> = groups[0][..3]
        .iter()
        .map(|id| {
            let row = partial.get_work_item(id).unwrap().unwrap();
            assert_eq!(row.value.state, WorkItemState::Cancelled);
            row.change_seq
        })
        .collect();
    for group in &groups[1..] {
        for id in group {
            assert!(matches!(
                partial.get_work_item(id).unwrap().unwrap().value.state,
                WorkItemState::NeedsAttention | WorkItemState::Blocked
            ));
        }
    }
    assert!(
        partial
            .diagnostic_summary()
            .unwrap()
            .growth
            .max_seq
            .unwrap()
            > original_max_seq
    );
    drop(partial);

    let retry = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    assert_eq!(
        open_count(&retry.list_work_items(None, 100).await.unwrap().entries),
        4
    );
    for (id, previous_seq) in groups[0][..3].iter().zip(first_group_seq) {
        assert_eq!(
            retry
                .get_work_item(id.clone())
                .await
                .unwrap()
                .unwrap()
                .change_seq,
            previous_seq
        );
    }
    let before_third_open = sequence(&retry).await;
    retry.shutdown().await;
    let third = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    assert_eq!(sequence(&third).await, before_third_open);
    assert_eq!(
        third
            .with_store(|store| store.retire_superseded_backlog())
            .await
            .unwrap(),
        0
    );
    third.shutdown().await;
}

#[tokio::test]
async fn advanced_states_and_other_coverage_are_untouched() {
    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let working = super::super::supersede::with_publication_bypass(|| {
        seed(
            &mut store,
            &project,
            "working",
            "model_cost_optimizer",
            "coverage-a",
            "2026-09-01T00:00:00Z",
            WorkflowTerminal::Inconclusive,
        )
    });
    let working_id = working.id.clone();
    let mut stored = store.get_work_item(&working_id).unwrap().unwrap();
    stored.value.state = WorkItemState::Working;
    store
        .upsert_work_item(&stored.value, Some(stored.revision))
        .unwrap();
    let other_coverage = super::super::supersede::with_publication_bypass(|| {
        seed(
            &mut store,
            &project,
            "other-coverage",
            "model_cost_optimizer",
            "coverage-b",
            "2026-09-02T00:00:00Z",
            WorkflowTerminal::Inconclusive,
        )
    });
    let newer = super::super::supersede::with_publication_bypass(|| {
        seed(
            &mut store,
            &project,
            "newer-same-coverage",
            "model_cost_optimizer",
            "coverage-a",
            "2026-09-03T00:00:00Z",
            WorkflowTerminal::Inconclusive,
        )
    });
    for id in [&other_coverage.id, &newer.id] {
        let mut item = store.get_work_item(id).unwrap().unwrap();
        item.value.state = WorkItemState::NeedsAttention;
        store
            .upsert_work_item(&item.value, Some(item.revision))
            .unwrap();
    }
    store.retire_superseded_backlog().unwrap();
    assert_eq!(
        store
            .get_work_item(&working_id)
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::Working
    );
    assert_eq!(
        store
            .get_work_item(&other_coverage.id)
            .unwrap()
            .unwrap()
            .value
            .state,
        WorkItemState::NeedsAttention
    );
    assert_eq!(
        store.get_work_item(&newer.id).unwrap().unwrap().value.state,
        WorkItemState::NeedsAttention
    );
}

#[tokio::test]
async fn open_pass_groups_security_whisperer_coverage_counts_and_keeps_history() {
    let root = TempDir::new().unwrap();
    let project = identity(root.path());
    let mut store =
        super::super::operator_store::OperatorStore::open(root.path(), &project).unwrap();
    let rows = [
        (5, "2026-09-14T23:47:16Z"),
        (5, "2026-09-15T00:25:18Z"),
        (10, "2026-09-15T02:44:22Z"),
        (10, "2026-09-15T03:14:51Z"),
        (11, "2026-09-15T03:50:11Z"),
        (11, "2026-09-15T04:54:27Z"),
        (11, "2026-09-15T05:24:50Z"),
        (11, "2026-09-15T05:54:51Z"),
        (11, "2026-09-15T07:56:13Z"),
        (13, "2026-09-16T02:43:12Z"),
        (0, "2026-09-25T03:03:14Z"),
        (6, "2026-09-30T03:42:21Z"),
        (7, "2026-09-30T10:34:01Z"),
        (8, "2026-09-30T13:15:23Z"),
    ];
    let ids: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, (count, started_at))| {
            let coverage = format!(
                "Secret-pattern scan of staged and working-tree contents of every changed path under [REDACTED_PATH] (text files up to 65536 bytes), plus {count} recent diagnostics"
            );
            let item = super::super::supersede::with_publication_bypass(|| {
                seed(
                    &mut store,
                    &project,
                    &format!("security-whisperer-{index}"),
                    "security_whisperer",
                    &coverage,
                    started_at,
                    WorkflowTerminal::Partial {
                        failure_class: Some("partial_source_coverage".into()),
                    },
                )
            });
            item.id
        })
        .collect();
    drop(store);

    let runtime = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    let open: Vec<_> = runtime
        .list_work_items(None, 100)
        .await
        .unwrap()
        .entries
        .into_iter()
        .filter(|row| {
            matches!(
                row.value.state,
                WorkItemState::NeedsAttention | WorkItemState::Watching | WorkItemState::Blocked
            )
        })
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].value.id, ids[13]);
    for id in &ids[..13] {
        let retired = runtime.get_work_item(id.clone()).await.unwrap().unwrap();
        assert_eq!(retired.value.state, WorkItemState::Cancelled);
        assert_eq!(
            runtime
                .list_work_outcomes(id.clone(), None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        assert!(runtime
            .get_artifact_revision(retired.value.current_artifact_id.unwrap(), Some(1))
            .await
            .unwrap()
            .is_some());
    }
    assert_eq!(
        runtime
            .with_store(|store| store.retire_superseded_backlog())
            .await
            .unwrap(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn backlog_pass_cost_for_110_rows_across_11_groups() {
    let pass_root = TempDir::new().unwrap();
    let (pass_project, _) = seed_backlog(&pass_root, 11, 10);
    let mut pass_store =
        super::super::operator_store::OperatorStore::open(pass_root.path(), &pass_project).unwrap();
    let pass_started = Instant::now();
    let retired = pass_store.retire_superseded_backlog().unwrap();
    let pass_elapsed = pass_started.elapsed();
    assert_eq!(retired, 99);
    drop(pass_store);
    let baseline_root = TempDir::new().unwrap();
    let (baseline_project, _) = seed_backlog(&baseline_root, 11, 10);
    let baseline_started = Instant::now();
    let baseline = super::super::supersede::with_open_pass_disabled(|| {
        OutcomeRuntime::open(baseline_root.path(), baseline_project, worker()).unwrap()
    });
    let baseline_elapsed = baseline_started.elapsed();
    baseline.shutdown().await;
    let open_root = TempDir::new().unwrap();
    let (open_project, _) = seed_backlog(&open_root, 11, 10);
    let open_started = Instant::now();
    let with_pass = OutcomeRuntime::open(open_root.path(), open_project, worker()).unwrap();
    let with_pass_elapsed = open_started.elapsed();
    assert_eq!(
        open_count(&with_pass.list_work_items(None, 200).await.unwrap().entries),
        11
    );
    eprintln!(
        "R-16 cost: 110 rows / 11 groups pass={pass_elapsed:?}; open without pass={baseline_elapsed:?}; open with pass={with_pass_elapsed:?}"
    );
    with_pass.shutdown().await;
}
