use std::time::Instant;

use refact_buddy_core::operator_types::{
    ArtifactRevision, ProjectIdentity, RunOutcome, WorkItem, WorkerIdentity,
};

use super::*;
use crate::buddy::{outcome_runtime::OutcomeRuntime, operator_store::OperatorStore};

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
        canonical_root: root.canonicalize().unwrap().to_string_lossy().into_owned(),
    }
}

#[test]
fn parser_validates_raw_count_and_empty_segments_then_deduplicates_in_order() {
    assert_eq!(parse_work_ids(None), Err(INVALID_BATCH_IDS));
    assert_eq!(parse_work_ids(Some("")), Err(INVALID_BATCH_IDS));
    assert_eq!(parse_work_ids(Some("one,,two")), Err(INVALID_BATCH_IDS));
    assert_eq!(parse_work_ids(Some("one,two,")), Err(INVALID_BATCH_IDS));
    assert_eq!(
        parse_work_ids(Some(&vec!["id"; 101].join(","))),
        Err(INVALID_BATCH_IDS)
    );
    assert_eq!(
        parse_work_ids(Some("first,second,first,third,second")).unwrap(),
        ["first", "second", "third"]
    );
}

#[test]
#[should_panic]
fn parity_negative_control_empty_actions_fails() {
    let expected = serde_json::json!([{"action_id":"registered"}]);
    let broken_batch = serde_json::json!([]);
    assert_eq!(broken_batch, expected);
}

#[tokio::test]
async fn batch_matches_row_actions_errors_and_measures_fifty_ids() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let identity = project(&root_path, "batch-project");
    let mut store = OperatorStore::open(&root_path, &identity).unwrap();
    let fixture = fixture();
    let mut ids = Vec::new();
    for index in 0..8 {
        let id = format!("batch-work-{index}");
        let mut work: WorkItem = serde_json::from_value(fixture["work_item"].clone()).unwrap();
        work.id = id.clone();
        work.project = identity.clone();
        work.subject.resource_id = format!("batch-subject-{index}");
        if index % 2 == 0 {
            let artifact_id = format!("{id}-artifact");
            let run_id = format!("{id}-run");
            work.current_artifact_id = Some(artifact_id.clone());
            work.current_artifact_revision = Some(1);
            let mut artifact: ArtifactRevision =
                serde_json::from_value(fixture["artifact_revision"].clone()).unwrap();
            artifact.artifact_id = artifact_id;
            artifact.revision = 1;
            artifact.project = identity.clone();
            artifact.source_run_ids = vec![run_id.clone()];
            artifact.provenance.source_run_ids = vec![run_id.clone()];
            let mut outcome: RunOutcome =
                serde_json::from_value(fixture["run_outcome"].clone()).unwrap();
            outcome.run_id = run_id;
            outcome.work_id = id.clone();
            outcome.project = identity.clone();
            outcome.worker = worker();
            store
                .append_outcome_work_item_artifact(&outcome, &work, None, Some(&artifact))
                .unwrap();
        } else {
            work.current_artifact_id = None;
            work.current_artifact_revision = None;
            store.upsert_work_item(&work, None).unwrap();
        }
        ids.push(id);
    }
    drop(store);
    let runtime = OutcomeRuntime::open(&root_path, identity.clone(), worker()).unwrap();
    let per_row =
        futures::future::join_all(ids.iter().map(|id| runtime.desk_actions(id.clone()))).await;
    let started = Instant::now();
    let batch = runtime.desk_actions_batch(ids.clone()).await.unwrap();
    eprintln!(
        "R-21 batch actions measurement: ids={}, elapsed_ms={}",
        ids.len(),
        started.elapsed().as_millis()
    );
    for ((id, row), entry) in ids.iter().zip(per_row).zip(&batch) {
        assert_eq!(&entry.work_id, id);
        assert_eq!(entry.error, None);
        assert_eq!(
            serde_json::to_value(&entry.actions).unwrap(),
            serde_json::to_value(row.unwrap()).unwrap()
        );
    }
    assert!(batch[0]
        .actions
        .iter()
        .any(|action| action.spec.action_id == crate::buddy::desk_actions::DISMISS_NOT_A_PROBLEM));
    assert_eq!(
        parse_work_ids(Some(&format!("{},{},{}", ids[0], ids[1], ids[0]))).unwrap(),
        [ids[0].clone(), ids[1].clone()]
    );
    let mut fifty_ids = ids.clone();
    fifty_ids.resize_with(50, || "missing-work".into());
    let started = Instant::now();
    let fifty = runtime.desk_actions_batch(fifty_ids).await.unwrap();
    eprintln!(
        "R-21 batch actions measurement: ids=50, elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(fifty.len(), 50);
    assert!(fifty[8..]
        .iter()
        .all(|entry| entry.error.as_deref() == Some("work item not found")));
    runtime.shutdown().await;
    let database =
        rusqlite::Connection::open(root_path.join(".refact/buddy/operator.sqlite")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload FROM work_items WHERE id=?1",
            [&ids[0]],
            |row| row.get(0),
        )
        .unwrap();
    let mut work: serde_json::Value = serde_json::from_str(&payload).unwrap();
    work["project"]["project_id"] = serde_json::json!("foreign-project");
    database
        .execute(
            "UPDATE work_items SET payload=?1 WHERE id=?2",
            rusqlite::params![work.to_string(), ids[0]],
        )
        .unwrap();
    drop(database);
    let runtime = OutcomeRuntime::open(&root_path, identity, worker()).unwrap();
    let mixed = runtime
        .desk_actions_batch(vec!["missing-work".into(), ids[0].clone(), ids[1].clone()])
        .await
        .unwrap();
    assert_eq!(mixed[0].error.as_deref(), Some("work item not found"));
    assert_eq!(
        mixed[1].error.as_deref(),
        Some("operator store belongs to a different project")
    );
    assert!(mixed[2].error.is_none());
    runtime.shutdown().await;
}
