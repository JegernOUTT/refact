use std::path::Path;

use refact_buddy_core::operator_types::{ProjectIdentity, WorkItem, WorkItemState, WorkerIdentity};

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
