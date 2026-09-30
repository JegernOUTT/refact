//! Scratch-root helpers for Buddy operator-effect tests.

#[path = "../../tests/it/buddy_operator_common/fixtures.rs"]
mod isolated_fixtures;

#[tokio::test]
async fn fixture_provider_report_is_served_by_real_operator_routes_after_runtime_restart() {
    use isolated_fixtures::{FixtureEndpoints, FixtureTransport};
    use super::outcome_runtime::WorkflowTerminal;
    let harness = BuddyOperatorHarness::new();
    let mut endpoints = FixtureEndpoints::start(harness.project_root().to_path_buf());
    let transport = FixtureTransport::new([endpoints.address]);
    let completion: serde_json::Value = transport.request(reqwest::Method::POST,
        &endpoints.url("/v1/chat/completions"), Some(serde_json::json!({
            "model":"fixture-model", "messages":[{"role":"user","content":"Draft scratch skill"}]
        }))).await.unwrap().json().await.unwrap();
    let runtime = harness.open_operator_runtime();
    let mut run = runtime.start_workflow("fixture_provider", "2026-09-05T00:00:00Z".into());
    run.coverage = "Isolated HTTP fixture response, not production provider health".into();
    run.input_fingerprint = "fixture-provider-v1".into();
    runtime.begin_workflow(run.clone()).await.unwrap();
    let work = runtime
        .publish_report(
            run,
            "Provider fixture result".into(),
            completion["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .into(),
            vec![],
            WorkflowTerminal::Completed,
        )
        .await
        .unwrap();
    runtime.shutdown().await;
    endpoints.shutdown().await;
    assert_eq!(endpoints.calls(), ["completion"]);
    assert!(transport.denied().is_empty());
    let reopened = harness.open_operator_runtime();
    let router = harness.operator_router(Some(reopened.clone())).await;
    let (status, report) = get_json(
        router,
        &format!(
            "/v1/buddy/artifacts/{}",
            work.value.current_artifact_id.unwrap()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(report["data"]["value"]["body"]
        .as_str()
        .unwrap()
        .contains("fixture-skill"));
    reopened.shutdown().await;
}

use std::path::Path;

use axum::{
    routing::{get, post},
    Router,
};
use refact_buddy_core::operator_types::{
    ActionExecution, ArtifactRevision, ProjectIdentity, RunOutcome, WorkerIdentity, WorkItem,
};
use crate::buddy::{
    actor::BuddyService, operator_store::OperatorStore, outcome_runtime::OutcomeRuntime,
    runtime_queue::RuntimeQueue, settings::BuddySettings, state::default_buddy_state,
    storage::load_drafts,
};
use crate::{app_state::AppState, global_context::tests::make_test_gcx_with_dirs};
use tempfile::TempDir;
use tokio::sync::broadcast;

#[tokio::test]
async fn batch_http_endpoint_is_static_and_keeps_row_action_parity_for_fifty_items() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let worker = harness.worker_identity();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
    ))
    .unwrap();
    let mut store = OperatorStore::open(harness.project_root(), &project).unwrap();
    let mut ids = Vec::new();
    for index in 0..50 {
        let mut work: WorkItem = serde_json::from_value(fixture["work_item"].clone()).unwrap();
        work.id = format!("batch-http-{index}");
        work.project = project.clone();
        work.subject.resource_id = format!("batch-http-subject-{index}");
        work.current_artifact_id = None;
        work.current_artifact_revision = None;
        store.upsert_work_item(&work, None).unwrap();
        ids.push(work.id);
    }
    drop(store);
    let runtime = OutcomeRuntime::open(harness.project_root(), project.clone(), worker).unwrap();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let started = std::time::Instant::now();
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/buddy/work/actions?project_id={}&ids={}",
                    project.project_id,
                    ids.join(",")
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    eprintln!(
        "R-21 batch HTTP measurement: ids=50, elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(response.status(), hyper::StatusCode::OK);
    let batch: serde_json::Value =
        serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
            .unwrap();
    assert_eq!(batch["data"].as_array().unwrap().len(), 50);
    for query in [
        format!(
            "/v1/buddy/work/actions?project_id={}&ids=",
            project.project_id
        ),
        format!(
            "/v1/buddy/work/actions?project_id={}&ids={}",
            project.project_id,
            std::iter::repeat_n("too-many", 101)
                .collect::<Vec<_>>()
                .join(",")
        ),
    ] {
        let invalid = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(query)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), hyper::StatusCode::BAD_REQUEST);
    }
    let item = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/buddy/work/{}?project_id={}",
                    ids[0], project.project_id
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(item.status(), hyper::StatusCode::OK);
    let row = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/buddy/work/{}/actions?project_id={}",
                    ids[0], project.project_id
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(row.status(), hyper::StatusCode::OK);
    let row: serde_json::Value =
        serde_json::from_slice(&hyper::body::to_bytes(row.into_body()).await.unwrap()).unwrap();
    assert_eq!(batch["data"][0]["actions"], row["data"]);
    runtime.shutdown().await;
}

pub struct BuddyOperatorHarness {
    project: TempDir,
    user_config: TempDir,
    shutdown: bool,
}

impl BuddyOperatorHarness {
    pub fn new() -> Self {
        let project = tempfile::tempdir().expect("create scratch project");
        let user_config = tempfile::tempdir().expect("create scratch user config");
        std::fs::write(
            project.path().join("README.md"),
            "# Buddy operator scratch\n",
        )
        .expect("seed scratch project");
        Self {
            project,
            user_config,
            shutdown: false,
        }
    }

    pub fn project_root(&self) -> &Path {
        self.project.path()
    }

    pub fn user_config_root(&self) -> &Path {
        self.user_config.path()
    }

    pub fn service(&self) -> BuddyService {
        let (events_tx, _) = broadcast::channel(16);
        BuddyService::new(
            self.project_root().to_path_buf(),
            default_buddy_state(),
            BuddySettings::default(),
            Vec::new(),
            RuntimeQueue::new(),
            events_tx,
            None,
        )
    }

    pub async fn restored_service(&self) -> BuddyService {
        let mut service = self.service();
        service
            .draft_store
            .restore(load_drafts(self.project_root()).await);
        service
    }

    pub fn shutdown(&mut self) {
        self.shutdown = true;
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    pub fn project_identity(&self) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "buddy-operator-scratch".to_string(),
            canonical_root: std::fs::canonicalize(self.project_root())
                .expect("canonical scratch root")
                .to_string_lossy()
                .into_owned(),
        }
    }

    pub fn worker_identity(&self) -> WorkerIdentity {
        WorkerIdentity {
            worker_id: "buddy-operator-test-worker".to_string(),
            engine_instance_id: "buddy-operator-test-engine".to_string(),
            epoch: 1,
        }
    }

    pub fn seed_operator_records(
        &self,
        outcome: &RunOutcome,
        work: &WorkItem,
        artifact: &ArtifactRevision,
        execution: &ActionExecution,
    ) {
        let project = self.project_identity();
        let mut store = OperatorStore::open(self.project_root(), &project)
            .expect("open isolated operator store for seeding");
        store
            .append_outcome_work_item_artifact(outcome, work, None, Some(artifact))
            .expect("seed linked operator outcome, work item, and artifact atomically");
        store
            .claim_execution(execution)
            .expect("seed operator execution metadata");
        drop(store);
    }

    pub async fn operator_router(&self, runtime: Option<OutcomeRuntime>) -> Router {
        let cache_dir = self.user_config_root().join("cache");
        let config_dir = self.user_config_root().join("config");
        let gcx = make_test_gcx_with_dirs(cache_dir, config_dir).await;
        let app = AppState::from_gcx(gcx).await;
        let mut service = self.service();
        service.outcome_runtime = runtime;
        *app.buddy.buddy.lock().await = Some(service);
        Router::new()
            .route(
                "/v1/buddy/work",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_work_list),
            )
            .route(
                "/v1/buddy/work/actions",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_actions_batch),
            )
            .route(
                "/v1/buddy/work/:id",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_work_get),
            )
            .route(
                "/v1/buddy/work/:id/actions",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_actions),
            )
            .route(
                "/v1/buddy/work/:id/outcomes",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_work_outcomes),
            )
            .route(
                "/v1/buddy/work/:id/read",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_read_get)
                    .post(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_read_mark),
            )
            .route(
                "/v1/buddy/work/:id/snooze",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_snooze_get)
                    .post(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_snooze_set),
            )
            .route(
                "/v1/buddy/work/:id/actions/:action_id/execute",
                post(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_action_execute),
            )
            .route(
                "/v1/buddy/artifacts/:id",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_artifact_get),
            )
            .route(
                "/v1/buddy/artifacts/:id/revisions",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_artifact_history),
            )
            .route(
                "/v1/buddy/outcomes/:id",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_outcome_get),
            )
            .route(
                "/v1/buddy/executions/:id",
                get(crate::http::routers::v1::buddy_operator::handle_v1_buddy_operator_execution_get),
            )
            .with_state(app)
    }

    pub fn open_operator_runtime(&self) -> OutcomeRuntime {
        OutcomeRuntime::open(
            self.project_root(),
            self.project_identity(),
            self.worker_identity(),
        )
        .expect("reopen isolated operator runtime after seed store closes")
    }
}

use axum::body::Body;
use hyper::{Request, StatusCode};
use refact_buddy_core::operator_types::{
    ActionExecutionState, ArtifactProvenance, EffectState, NativeSourceIdentity, NativeSourceKind,
    ResourceEffect, ResourceRef, RunStatus, WorkItemState, OPERATOR_SCHEMA_VERSION_V1,
};
use tower::ServiceExt;

fn source(kind: NativeSourceKind, id: &str) -> NativeSourceIdentity {
    NativeSourceIdentity {
        source_kind: kind,
        source_id: id.to_string(),
        source_version: "1".to_string(),
    }
}

fn scratch_records(
    project: ProjectIdentity,
    worker: refact_buddy_core::operator_types::WorkerIdentity,
) -> (RunOutcome, WorkItem, ArtifactRevision, ActionExecution) {
    let resource = ResourceRef {
        resource_kind: "scratch".to_string(),
        resource_id: "scratch-resource".to_string(),
        version: "1".to_string(),
    };
    let effect = ResourceEffect {
        resource: resource.clone(),
        state: EffectState::Unknown,
        evidence_refs: Vec::new(),
    };
    let work = WorkItem {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        id: "scratch-work".to_string(),
        project: project.clone(),
        subject: resource.clone(),
        kind: "scratch_check".to_string(),
        title: "Scratch work item".to_string(),
        state: WorkItemState::NeedsAttention,
        native_sources: vec![source(NativeSourceKind::Goal, "scratch-goal")],
        current_artifact_id: Some("scratch-artifact".to_string()),
        current_artifact_revision: Some(1),
        last_material_change_at: "2026-09-05T00:00:00Z".to_string(),
        severity: None,
        problem_summary: None,
        scope: None,
    };
    let outcome = RunOutcome {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        run_id: "scratch-run".to_string(),
        work_id: work.id.clone(),
        parent_run_id: None,
        project: project.clone(),
        worker: worker.clone(),
        origin: "operator-test".to_string(),
        workflow: Some("scratch_check".to_string()),
        input_fingerprint: "scratch-input".to_string(),
        coverage: "isolated router read coverage".to_string(),
        started_at: "2026-09-05T00:00:00Z".to_string(),
        ended_at: Some("2026-09-05T00:01:00Z".to_string()),
        status: RunStatus::Inconclusive,
        evidence: Vec::new(),
        effects: vec![effect.clone()],
        failure_class: None,
    };
    let artifact = ArtifactRevision {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        artifact_id: "scratch-artifact".to_string(),
        revision: 1,
        project: project.clone(),
        source_run_ids: vec![outcome.run_id.clone()],
        title: "Scratch artifact".to_string(),
        conclusion: "The scratch store was read through HTTP.".to_string(),
        delta: "Initial scratch observation.".to_string(),
        body: "This is local test data only.".to_string(),
        uncertainty: None,
        proposed_action_ids: vec!["scratch-action".to_string()],
        evidence: Vec::new(),
        provenance: ArtifactProvenance {
            source_run_ids: vec![outcome.run_id.clone()],
            evidence_refs: Vec::new(),
            contains_untrusted_source: false,
        },
    };
    let execution = ActionExecution {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        execution_id: "scratch-execution".to_string(),
        project,
        worker,
        idempotency_key: "scratch-request".to_string(),
        action_id: "scratch-action".to_string(),
        spec_hash: "scratch-spec".to_string(),
        requester: source(NativeSourceKind::Chat, "scratch-chat"),
        decision: None,
        status: ActionExecutionState::Reconciling,
        started_at: Some("2026-09-05T00:02:00Z".to_string()),
        ended_at: None,
        effects: vec![effect],
        verification_evidence: Vec::new(),
        recovery: None,
    };
    (outcome, work, artifact, execution)
}

async fn get_json(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("in-process operator route responds");
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body())
        .await
        .expect("read operator response body");
    let json = serde_json::from_slice(&body).expect("operator response is JSON");
    (status, json)
}

#[tokio::test]
async fn completed_report_and_interrupted_run_are_readable_through_real_routes_after_restart() {
    use super::outcome_runtime::{WorkflowOutcomeCheckpoint, WorkflowTerminal};
    use refact_buddy_core::operator_types::EvidenceRef;
    let harness = BuddyOperatorHarness::new();
    let runtime = harness.open_operator_runtime();
    let mut completed = runtime.start_workflow("scratch_check", "2026-09-05T00:00:00Z".into());
    completed.coverage = "Scratch README metadata only; no network or provider check".into();
    completed.input_fingerprint = "scratch-revision-a".into();
    runtime.begin_workflow(completed.clone()).await.unwrap();
    let work = runtime
        .publish_report(
            completed.clone(),
            "Scratch diagnosis".into(),
            "The scratch README is present. No resources were changed.".into(),
            vec![],
            WorkflowTerminal::Completed,
        )
        .await
        .unwrap();
    let mut interrupted = completed.clone();
    interrupted.run_id = "interrupted-scratch-check".into();
    interrupted.input_fingerprint = "scratch-revision-b".into();
    runtime.begin_workflow(interrupted.clone()).await.unwrap();
    runtime
        .checkpoint_workflow(
            interrupted.clone(),
            WorkflowOutcomeCheckpoint::from_evidence([EvidenceRef {
                evidence_id: "scratch-readme-observation".into(),
                source: source(NativeSourceKind::Check, "scratch-check"),
                locator: "check://scratch-readme".into(),
                observed_at: "2026-09-05T00:00:01Z".into(),
                digest: Some("scratch-revision-b".into()),
            }]),
        )
        .await
        .unwrap();
    runtime.shutdown().await;

    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (status, report) = get_json(
        router.clone(),
        &format!(
            "/v1/buddy/artifacts/{}",
            work.value.current_artifact_id.unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["data"]["value"]["title"], "Scratch diagnosis");
    assert!(report["data"]["value"]["body"]
        .as_str()
        .unwrap()
        .contains("No resources were changed"));
    let (status, saved) = get_json(
        router.clone(),
        &format!("/v1/buddy/outcomes/{}", completed.run_id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["data"]["value"]["status"], "completed");
    let (status, interrupted_outcome) = get_json(
        router.clone(),
        "/v1/buddy/outcomes/interrupted-scratch-check",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let outcome = &interrupted_outcome["data"]["value"];
    assert_eq!(outcome["status"], "partial");
    assert_eq!(outcome["failure_class"], "engine_interrupted");
    assert_eq!(
        outcome["evidence"][0]["evidence_id"],
        "scratch-readme-observation"
    );
    assert!(outcome["effects"].as_array().unwrap().is_empty());
    let (status, recovered_work) = get_json(
        router.clone(),
        &format!("/v1/buddy/work/{}", outcome["work_id"].as_str().unwrap(),),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered_work["data"]["value"]["state"], "needs_attention");
    let (_, recovered_report) = get_json(
        router,
        &format!(
            "/v1/buddy/artifacts/{}",
            recovered_work["data"]["value"]["current_artifact_id"]
                .as_str()
                .unwrap(),
        ),
    )
    .await;
    assert!(recovered_report["data"]["value"]["body"]
        .as_str()
        .unwrap()
        .contains("No effects were replayed"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn twelve_scoped_unchanged_checks_fold_but_a_blocked_check_is_a_new_unread_report() {
    use super::outcome_runtime::WorkflowTerminal;
    let harness = BuddyOperatorHarness::new();
    let runtime = harness.open_operator_runtime();
    let mut first = runtime.start_workflow("scratch_check", "2026-09-05T00:00:00Z".into());
    first.coverage = "Scratch resource digest at revision one".into();
    first.input_fingerprint = "scratch:one".into();
    let work = runtime
        .publish_workflow_result(
            first.clone(),
            "Observed digest is unchanged.".into(),
            WorkflowTerminal::NoChange,
        )
        .await
        .unwrap();
    let artifact_id = work.value.current_artifact_id.clone().unwrap();
    runtime
        .mark_work_read(work.value.id.clone(), artifact_id.clone(), 1)
        .await
        .unwrap();
    for index in 1..12 {
        let mut check = first.clone();
        check.run_id = format!("unchanged-{index}");
        check.started_at = format!("2026-09-05T00:{index:02}:00Z");
        runtime.begin_workflow(check.clone()).await.unwrap();
        let repeated = runtime
            .publish_workflow_result(
                check,
                "Observed digest is unchanged.".into(),
                WorkflowTerminal::NoChange,
            )
            .await
            .unwrap();
        assert_eq!(repeated.value.current_artifact_revision, Some(1));
        assert_eq!(
            repeated.value.last_material_change_at,
            work.value.last_material_change_at
        );
    }
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (_, unread) = get_json(router.clone(), "/v1/buddy/work?attention=unread").await;
    assert!(unread["data"].as_array().unwrap().is_empty());
    let (_, runs) = get_json(
        router.clone(),
        &format!("/v1/buddy/work/{}/outcomes", work.value.id),
    )
    .await;
    assert_eq!(runs["data"].as_array().unwrap().len(), 12);
    let mut blocked = first;
    blocked.run_id = "blocked-check".into();
    runtime.begin_workflow(blocked.clone()).await.unwrap();
    let blocked_work = runtime
        .publish_workflow_result(
            blocked,
            "Permission unavailable; resource was not observed.".into(),
            WorkflowTerminal::Blocked {
                failure_class: Some("permission".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(blocked_work.value.id, work.value.id);
    assert_eq!(blocked_work.value.state, WorkItemState::Blocked);
    assert_eq!(blocked_work.value.current_artifact_revision, Some(2));
    let (_, unread) = get_json(router.clone(), "/v1/buddy/work?attention=unread").await;
    assert_eq!(unread["data"].as_array().unwrap().len(), 1);
    let (_, original) = get_json(
        router,
        &format!("/v1/buddy/artifacts/{artifact_id}?revision=1"),
    )
    .await;
    assert_eq!(
        original["data"]["value"]["body"],
        "Observed digest is unchanged."
    );
    assert_eq!(
        runtime
            .get_read_state(work.value.id)
            .await
            .unwrap()
            .unwrap()
            .last_read_revision,
        1
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn isolated_operator_read_routes_return_persisted_runtime_records_after_reopen() {
    let mut harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);

    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let encoded_project = &project.project_id;

    let (status, listed) = get_json(
        router.clone(),
        &format!("/v1/buddy/work?project_id={encoded_project}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["schema_version"], OPERATOR_SCHEMA_VERSION_V1);
    assert_eq!(listed["project"]["project_id"], project.project_id);
    assert_eq!(listed["data"][0]["value"]["id"], work.id);
    assert_eq!(listed["data"][0]["revision"], 1);

    let (status, fetched_work) = get_json(
        router.clone(),
        &format!("/v1/buddy/work/{}?project_id={encoded_project}", work.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched_work["data"]["value"]["title"], work.title);
    assert_eq!(fetched_work["data"]["change_seq"], 1);

    let (status, fetched_artifact) = get_json(
        router.clone(),
        &format!(
            "/v1/buddy/artifacts/{}?project_id={encoded_project}&revision={}",
            artifact.artifact_id, artifact.revision
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fetched_artifact["data"]["value"]["conclusion"],
        artifact.conclusion
    );
    assert_eq!(fetched_artifact["data"]["revision"], artifact.revision);

    let (status, fetched_execution) = get_json(
        router,
        &format!(
            "/v1/buddy/executions/{}?project_id={encoded_project}",
            execution.execution_id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fetched_execution["data"]["value"]["status"],
        "outcome_unknown"
    );
    assert_eq!(fetched_execution["data"]["revision"], 2);
    assert!(fetched_execution["data"]["value"]["verification_evidence"]
        .to_string()
        .contains("not replayed"));

    runtime.shutdown().await;
    harness.shutdown();
}

#[tokio::test]
async fn isolated_snooze_routes_preserve_unread_work_and_reject_foreign_or_stale_requests() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let body = serde_json::json!({ "project_id": project.project_id, "canonical_root": project.canonical_root,
        "artifact_id": artifact.artifact_id, "artifact_revision": artifact.revision, "minutes": 60 });
    for (field, value, expected) in [
        (
            "canonical_root",
            serde_json::json!("/foreign"),
            StatusCode::CONFLICT,
        ),
        (
            "artifact_revision",
            serde_json::json!(2),
            StatusCode::CONFLICT,
        ),
        ("minutes", serde_json::json!(10081), StatusCode::BAD_REQUEST),
        ("minutes", serde_json::json!(60), StatusCode::OK),
    ] {
        let mut request = body.clone();
        request[field] = value;
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/buddy/work/scratch-work/snooze")
                    .header("content-type", "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let (_, snooze) = get_json(router.clone(), "/v1/buddy/work/scratch-work/snooze").await;
    assert_eq!(snooze["data"]["work_id"], work.id);
    let (_, unread) = get_json(router.clone(), "/v1/buddy/work?attention=unread").await;
    assert_eq!(unread["data"].as_array().unwrap().len(), 1);
    let (_, snoozed) = get_json(router.clone(), "/v1/buddy/work?attention=snoozed").await;
    assert_eq!(snoozed["data"].as_array().unwrap().len(), 1);
    let (_, read) = get_json(router, "/v1/buddy/work/scratch-work/read").await;
    assert!(read["data"].is_null());
    assert_eq!(
        runtime
            .get_work_item(work.id.clone())
            .await
            .unwrap()
            .unwrap()
            .value,
        work
    );
    runtime.shutdown().await;
    let reopened = harness.open_operator_runtime();
    assert!(reopened.get_work_snooze(work.id).await.unwrap().is_some());
    reopened.shutdown().await;
}

#[tokio::test]
async fn isolated_operator_read_routes_reject_wrong_project_and_report_missing_runtime() {
    let mut harness = BuddyOperatorHarness::new();
    let unavailable = harness.operator_router(None).await;
    let (status, body) = get_json(unavailable, "/v1/buddy/work").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.to_string().contains("runtime is unavailable"));

    let project = harness.project_identity();
    let (outcome, work, artifact, execution) = scratch_records(project, harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (status, body) = get_json(
        router,
        "/v1/buddy/work/scratch-work?project_id=another-project",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("does not match"));

    runtime.shutdown().await;
    harness.shutdown();
}

async fn post_action(
    router: Router,
    action: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/buddy/work/scratch-work/actions/{action}/execute"
                ))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn action_body(action: &serde_json::Value, project: &ProjectIdentity) -> serde_json::Value {
    serde_json::json!({
        "project_id": project.project_id, "work_revision": action["work_revision"],
        "artifact_id": action["artifact_id"], "artifact_revision": action["artifact_revision"],
        "spec_hash": action["spec_hash"], "idempotency_key": "desk-request-1"
    })
}

#[tokio::test]
async fn isolated_registered_desk_action_persists_partial_receipt_without_changing_work() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (status, actions) = get_json(router.clone(), "/v1/buddy/work/scratch-work/actions").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(actions["data"].as_array().unwrap().len(), 2);
    assert_eq!(actions["data"][0]["spec"]["authority_class"], "read_only");
    assert!(actions["data"][0]["latest_execution"].is_null());
    assert_eq!(
        actions["data"][1]["spec"]["action_id"],
        crate::buddy::desk_actions::DISMISS_NOT_A_PROBLEM
    );
    let body = action_body(&actions["data"][0], &project);
    let (status, response) =
        post_action(router.clone(), "check_saved_evidence", body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["data"]["value"]["status"], "partial");
    assert_eq!(response["data"]["value"]["effects"], serde_json::json!([]));
    assert!(response["data"]["value"]["started_at"].is_string());
    assert!(response["data"]["value"]["ended_at"].is_string());
    assert!(response["data"]["value"]["verification_evidence"]
        .to_string()
        .contains("not independently verified"));
    let (_, duplicate) = post_action(router.clone(), "check_saved_evidence", body).await;
    assert_eq!(duplicate, response);
    assert_eq!(
        runtime
            .get_work_item(work.id.clone())
            .await
            .unwrap()
            .unwrap()
            .value,
        work
    );
    runtime.shutdown().await;
    let reopened = harness.open_operator_runtime();
    let router = harness.operator_router(Some(reopened.clone())).await;
    let (_, actions) = get_json(router.clone(), "/v1/buddy/work/scratch-work/actions").await;
    assert_eq!(actions["data"][0]["latest_execution"], response["data"]);
    assert!(actions["data"][1]["latest_execution"].is_null());
    let (_, fetched) = get_json(
        router,
        &format!(
            "/v1/buddy/executions/{}",
            response["data"]["value"]["execution_id"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(fetched, response);
    reopened.shutdown().await;
}

#[tokio::test]
async fn isolated_desk_action_rejects_forged_stale_and_foreign_requests_before_claim() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (_, actions) = get_json(router.clone(), "/v1/buddy/work/scratch-work/actions").await;
    let body = action_body(&actions["data"][0], &project);
    for (field, value, status) in [
        (
            "spec_hash",
            serde_json::json!("forged"),
            StatusCode::CONFLICT,
        ),
        ("work_revision", serde_json::json!(2), StatusCode::CONFLICT),
        (
            "artifact_revision",
            serde_json::json!(2),
            StatusCode::CONFLICT,
        ),
        (
            "project_id",
            serde_json::json!("foreign"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "authority_class",
            serde_json::json!("mutating"),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "idempotency_key",
            serde_json::json!("../invalid"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut invalid = body.clone();
        invalid[field] = value;
        assert_eq!(
            post_action(router.clone(), "check_saved_evidence", invalid)
                .await
                .0,
            status,
            "{field}"
        );
    }
    assert_eq!(
        post_action(router.clone(), "apply_config_patch", body)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (_, after) = get_json(router, "/v1/buddy/work/scratch-work/actions").await;
    assert!(after["data"][0]["latest_execution"].is_null());
    runtime.shutdown().await;
}

#[tokio::test]
async fn isolated_desk_action_checks_exact_evidence_metadata_without_opening_sources() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (mut outcome, work, mut artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    let evidence = refact_buddy_core::operator_types::EvidenceRef {
        evidence_id: "retained-evidence".into(),
        source: source(NativeSourceKind::External, "untrusted-source"),
        locator: "https://must-not-be-opened.invalid/secret".into(),
        observed_at: "2026-09-05T00:00:00Z".into(),
        digest: Some("recorded-digest".into()),
    };
    outcome.evidence = vec![evidence.clone()];
    artifact.evidence = vec![evidence.clone()];
    artifact.provenance.evidence_refs = vec![evidence.evidence_id.clone()];
    artifact.provenance.contains_untrusted_source = true;
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (_, actions) = get_json(router.clone(), "/v1/buddy/work/scratch-work/actions").await;
    let body = action_body(&actions["data"][0], &project);
    let (a, b) = tokio::join!(
        post_action(router.clone(), "check_saved_evidence", body.clone()),
        post_action(router, "check_saved_evidence", body)
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(a, b);
    assert_eq!(a.1["data"]["value"]["status"], "completed");
    assert!(a.1["data"]["value"]["verification_evidence"]
        .to_string()
        .contains("1/1"));
    assert_eq!(
        runtime
            .get_artifact_revision(artifact.artifact_id.clone(), Some(1))
            .await
            .unwrap()
            .unwrap()
            .value,
        artifact
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn isolated_report_read_ack_survives_reopen_without_resolving_work() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (_, before) = get_json(router.clone(), "/v1/buddy/work/scratch-work").await;
    let (_, unread) = get_json(router.clone(), "/v1/buddy/work/scratch-work/read").await;
    assert!(unread["data"].is_null());
    for (project_id, revision, expected) in [
        ("foreign", 1, StatusCode::BAD_REQUEST),
        (project.project_id.as_str(), 2, StatusCode::CONFLICT),
        (project.project_id.as_str(), 1, StatusCode::OK),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/buddy/work/scratch-work/read")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"project_id":project_id,
                "artifact_id":artifact.artifact_id,"artifact_revision":revision})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let (_, after) = get_json(router.clone(), "/v1/buddy/work/scratch-work").await;
    assert_eq!(before, after);
    let (_, read) = get_json(router, "/v1/buddy/work/scratch-work/read").await;
    assert_eq!(read["data"]["last_read_revision"], 1);
    runtime.shutdown().await;
    let reopened = harness.open_operator_runtime();
    let router = harness.operator_router(Some(reopened.clone())).await;
    assert_eq!(
        get_json(router, "/v1/buddy/work/scratch-work/read").await.1,
        read
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn isolated_history_and_outcome_routes_return_only_saved_evidence() {
    let harness = BuddyOperatorHarness::new();
    let project = harness.project_identity();
    let (outcome, work, artifact, execution) =
        scratch_records(project.clone(), harness.worker_identity());
    harness.seed_operator_records(&outcome, &work, &artifact, &execution);
    let runtime = harness.open_operator_runtime();
    let router = harness.operator_router(Some(runtime.clone())).await;
    let (status, history) = get_json(
        router.clone(),
        "/v1/buddy/artifacts/scratch-artifact/revisions?limit=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        history["data"][0]["value"],
        serde_json::to_value(&artifact).unwrap()
    );
    let (status, run) = get_json(router.clone(), "/v1/buddy/outcomes/scratch-run").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        run["data"]["value"],
        serde_json::to_value(&outcome).unwrap()
    );
    assert_eq!(
        get_json(
            router.clone(),
            "/v1/buddy/outcomes/scratch-run?project_id=foreign"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get_json(router, "/v1/buddy/outcomes/absent").await.0,
        StatusCode::NOT_FOUND
    );
    runtime.shutdown().await;
}
