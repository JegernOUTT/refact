use std::path::Path;

use refact_buddy_core::operator_types::{ProjectIdentity, WorkItem, WorkItemState, WorkerIdentity};

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
