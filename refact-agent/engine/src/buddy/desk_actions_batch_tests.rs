use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
    ))
    .unwrap()
}

fn worker() -> WorkerIdentity {
    WorkerIdentity {
        worker_id: "batch-worker".into(),
        engine_instance_id: "batch-engine".into(),
        epoch: 1,
    }
}

fn project(root: &std::path::Path, id: &str) -> ProjectIdentity {
    ProjectIdentity {
        project_id: id.into(),
        canonical_root: dunce::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    }
}
