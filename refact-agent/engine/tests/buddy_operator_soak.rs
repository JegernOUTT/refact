//! Deterministic scratch-only growth/restart soak. No network or wall-clock assertions.
use refact_lsp::buddy::operator_observability::CapacityPolicy;
use refact_lsp::buddy::operator_store::OperatorStore;
use refact_lsp::buddy::outcome_runtime::{OutcomeRuntime, WorkflowTerminal};
use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity, RunStatus};

fn identity(root: &std::path::Path) -> ProjectIdentity {
    ProjectIdentity {
        project_id: "soak".into(),
        canonical_root: root.canonicalize().unwrap().to_string_lossy().into_owned(),
    }
}
fn worker() -> WorkerIdentity {
    WorkerIdentity {
        worker_id: "soak-worker".into(),
        engine_instance_id: "scratch".into(),
        epoch: 1,
    }
}

fn install_document_policy_fixture(
    root: &std::path::Path,
    policy: &refact_lsp::buddy::operator_observability::DocumentGrowthPolicy,
) {
    let connection =
        rusqlite::Connection::open(root.join(".refact/buddy/operator.sqlite")).unwrap();
    let payload = serde_json::to_string(&Some(policy)).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    // This fixture installs a current schema-7 policy directly, so it must
    // create exactly the accounting baseline that the native policy setter
    // establishes before it adds policy metadata.  Leaving these records out
    // models an incomplete current database, which must fail closed on reopen.
    connection
        .execute_batch(
            "INSERT INTO meta(key,value)
                 SELECT 'document_recovery_credit:legacy',
                     CAST((SELECT count(*) FROM (SELECT DISTINCT namespace,id FROM operator_documents)) * 324 +
                          (SELECT count(*) FROM executions) * 4 AS TEXT);
             INSERT INTO meta(key,value)
                 VALUES('document_global_cutoff',(SELECT COALESCE(max(seq),0) FROM changes));
             INSERT INTO meta(key,value)
                 SELECT 'document_global_baseline', CAST(
                     (SELECT count(*) FROM operator_documents) +
                     (SELECT count(*) FROM meta) +
                     (SELECT count(*) + COALESCE(sum(remaining),0) +
                         COALESCE(sum(CASE WHEN producer='supervision_completion' THEN consumed ELSE 0 END),0)
                      FROM terminal_reservations) +
                     (SELECT COALESCE(sum(CAST(value AS INTEGER)),0)
                      FROM meta WHERE key LIKE 'document_recovery_credit:%') +
                     COALESCE((SELECT sum((json_extract(receipt.value,'$.speech_id') IS NULL) +
                                          (json_extract(receipt.value,'$.claimed_surface') IS NULL) +
                                          (json_extract(receipt.value,'$.feedback') IS NULL))
                               FROM json_each((SELECT payload FROM operator_documents
                                               WHERE namespace='editorial' AND id='attention-v1'
                                               ORDER BY revision DESC LIMIT 1),'$.receipts') AS receipt),0) + 1
                     AS TEXT);",
        )
        .unwrap();
    if policy.replace_editorial_aggregate {
        let revision: u64 = connection
            .query_row(
                "SELECT COALESCE(max(revision),0) FROM operator_documents WHERE namespace='editorial' AND id='attention-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES('editorial_frozen_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [revision.to_string()],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('document_policy_history:0','null')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO meta(key,value) VALUES('document_policy_history:1',?1)",
            [&payload],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO meta(key,value) VALUES('document_policy_epoch','1') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO meta(key,value) VALUES('document_growth_policy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&payload],
        )
        .unwrap();
    connection.execute_batch("COMMIT").unwrap();
}

#[test]
fn editorial_counter_churn_preserves_delivery_history_and_resolves_at_capacity() {
    use refact_lsp::buddy::editorial::{self, Candidate, Decision, Surface};
    use refact_lsp::buddy::operator_observability::DocumentGrowthPolicy;
    let root = tempfile::tempdir().unwrap();
    let project = identity(root.path());
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    assert!(store
        .diagnostic_summary()
        .unwrap()
        .document_growth
        .policy
        .is_none());
    let candidate = Candidate::new("win", "topic", "evidence", 100);
    editorial::with_state(&mut store, |s| {
        assert_eq!(s.reserve(candidate.clone(), 100), Decision::Eligible);
        Ok(())
    })
    .unwrap();
    drop(store);
    install_document_policy_fixture(
        root.path(),
        &DocumentGrowthPolicy {
            max_rows_per_namespace: 2,
            replace_editorial_aggregate: true,
        },
    );
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    for _ in 0..32 {
        editorial::with_state(&mut store, |s| {
            s.counters.generated += 1;
            Ok(())
        })
        .unwrap();
    }
    let docs = |store: &OperatorStore| {
        store
            .diagnostic_summary()
            .unwrap()
            .tables
            .into_iter()
            .find(|t| t.table == "operator_documents")
            .unwrap()
            .sampled_rows
    };
    assert_eq!(docs(&store), 2);
    assert!(store
        .get_document_revision::<editorial::EditorialState>("editorial", "attention-v1", 1)
        .unwrap()
        .is_some());
    assert!(editorial::with_state(&mut store, |s| {
        s.reserve(Candidate::new("win", "other", "other", 101), 101);
        Ok(())
    })
    .is_err());
    editorial::with_state(&mut store, |s| s.publish(&candidate.id, "speech", 101)).unwrap();
    let published = store
        .get_document::<editorial::EditorialState>("editorial", "attention-v1")
        .unwrap()
        .unwrap();
    editorial::with_state(&mut store, |s| s.claim("speech", Surface::Panel, 102)).unwrap();
    for _ in 0..32 {
        editorial::with_state(&mut store, |s| {
            s.counters.generated += 1;
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(docs(&store), 5);
    let history = store
        .get_document_revision::<editorial::EditorialState>(
            "editorial",
            "attention-v1",
            published.revision,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        history.value.receipts[0].speech_id.as_deref(),
        Some("speech")
    );
    assert!(history.value.receipts[0].claimed_surface.is_none());
    drop(store);
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    assert!(
        !editorial::with_state(&mut store, |s| s.claim("speech", Surface::Avatar, 103)).unwrap()
    );
    assert_eq!(docs(&store), 5);
}

#[test]
fn fixed_spec_owner_admission_is_bounded_and_duplicate_safe() {
    use refact_lsp::buddy::improvement_evaluation::{register_spec, Criterion, EvaluationSpec};
    use refact_lsp::buddy::operator_observability::DocumentGrowthPolicy;
    let root = tempfile::tempdir().unwrap();
    let project = identity(root.path());
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
    ))
    .unwrap();
    let authority =
        serde_json::from_value(fixture["action_execution"]["requester"].clone()).unwrap();
    let mut spec = EvaluationSpec {
        schema_version: 1,
        project: project.clone(),
        id: "spec-0".into(),
        criteria: vec![Criterion {
            id: "check".into(),
            command_or_protocol: "manual inspection".into(),
            expected_result: "pass".into(),
            external_review: true,
        }],
        corpus_digest: "a".repeat(64),
        privacy_scope: "scratch".into(),
        authority,
        max_age_seconds: 60,
        execution: None,
    };
    drop(store);
    install_document_policy_fixture(
        root.path(),
        &DocumentGrowthPolicy {
            max_rows_per_namespace: 3,
            replace_editorial_aggregate: false,
        },
    );
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    for n in 0..12 {
        spec.id = format!("spec-{n}");
        assert_eq!(
            register_spec(&mut store, &spec, &spec.authority).is_ok(),
            n < 3
        );
    }
    spec.id = "spec-0".into();
    assert_eq!(
        register_spec(&mut store, &spec, &spec.authority)
            .unwrap()
            .revision,
        1
    );
    assert!(store
        .get_document_revision::<EvaluationSpec>("improvement_specs", "spec-0", 1)
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .diagnostic_summary()
            .unwrap()
            .document_growth
            .global_document_bound,
        Some(197)
    );
}

#[tokio::test]
async fn repeated_material_runs_restart_partial_failure_and_paginated_history() {
    let root = tempfile::tempdir().unwrap();
    let project = identity(root.path());
    let mut owner = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    let mut last = None;
    for cycle in 0..6 {
        for index in 0..40 {
            let mut start = owner.start_workflow("soak", "2026-01-01T00:00:00Z".into());
            start.run_id = format!("{cycle}-{index}");
            let work = owner
                .publish_assessment(
                    start.clone(),
                    "Report".into(),
                    format!("material-{cycle}"),
                    vec![],
                )
                .await
                .unwrap();
            assert_eq!(work.value.current_artifact_revision, Some(cycle + 1));
            // Duplicate publication does not allocate rows or change a material revision.
            let duplicate = owner
                .publish_assessment(start, "Report".into(), format!("material-{cycle}"), vec![])
                .await
                .unwrap();
            assert_eq!(work, duplicate);
            last = Some(work);
        }
        owner.shutdown().await;
        owner = OutcomeRuntime::open(root.path(), project.clone(), worker()).unwrap();
    }
    let work = last.unwrap();
    let mut cursor = None;
    let mut ids = std::collections::BTreeSet::new();
    loop {
        let page = owner
            .list_work_outcomes(work.value.id.clone(), cursor, 17)
            .await
            .unwrap();
        for entry in page.entries {
            assert!(ids.insert(entry.value.run_id));
        }
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 240);
    assert_eq!(
        owner
            .list_artifact_revisions(work.value.current_artifact_id.unwrap(), None, 200)
            .await
            .unwrap()
            .entries
            .len(),
        6
    );
    let mut interrupted = owner.start_workflow("interrupted", "2026-01-01T00:00:00Z".into());
    interrupted.run_id = "interrupted".into();
    owner.begin_workflow(interrupted.clone()).await.unwrap();
    let evidence = refact_buddy_core::operator_types::EvidenceRef {
        evidence_id: "checkpoint".into(),
        source: refact_buddy_core::operator_types::NativeSourceIdentity {
            source_kind: refact_buddy_core::operator_types::NativeSourceKind::Tool,
            source_id: "scratch".into(),
            source_version: "1".into(),
        },
        locator: "tool://scratch".into(),
        observed_at: "2026-01-01T00:00:00Z".into(),
        digest: None,
    };
    owner
        .checkpoint_workflow(
            interrupted.clone(),
            refact_lsp::buddy::outcome_runtime::WorkflowOutcomeCheckpoint::from_evidence([
                evidence.clone(),
            ]),
        )
        .await
        .unwrap();
    let mut cancelled = owner.start_workflow("cancelled", "2026-01-01T00:00:00Z".into());
    cancelled.run_id = "cancelled".into();
    owner.begin_workflow(cancelled.clone()).await.unwrap();
    owner
        .publish_workflow_result(
            cancelled,
            "Cancelled before effect".into(),
            WorkflowTerminal::Cancelled {
                failure_class: Some("cancelled".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        owner
            .get_run_outcome("cancelled".into())
            .await
            .unwrap()
            .unwrap()
            .value
            .status,
        RunStatus::Cancelled
    );
    let mut cancelled = owner.start_workflow("cancelled", "2026-01-01T00:00:00Z".into());
    cancelled.run_id = "cancelled".into();
    assert!(owner.begin_workflow(cancelled).await.is_err());
    owner
        .checkpoint_workflow(
            interrupted.clone(),
            refact_lsp::buddy::outcome_runtime::WorkflowOutcomeCheckpoint::from_evidence([
                evidence.clone(),
            ]),
        )
        .await
        .unwrap();
    // Inject durable SQLite write failure with a scratch-only trigger, not real disk exhaustion.
    let db = rusqlite::Connection::open(root.path().join(".refact/buddy/operator.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER injected_failure BEFORE INSERT ON run_outcomes BEGIN SELECT RAISE(ABORT,'simulated disk failure'); END;").unwrap();
    assert!(owner
        .publish_workflow_result(
            interrupted,
            "Failed write".into(),
            WorkflowTerminal::Completed
        )
        .await
        .is_err());
    db.execute_batch("DROP TRIGGER injected_failure;").unwrap();
    drop(db);
    // Admit no new work, but allow active-run restart reconciliation and native history.
    owner
        .with_store(|s| {
            s.set_capacity_policy(Some(CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
        })
        .await
        .unwrap();
    owner.shutdown().await;
    owner = OutcomeRuntime::open(root.path(), project, worker()).unwrap();
    let recovered = owner
        .get_run_outcome("interrupted".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.value.status, RunStatus::Partial);
    assert_eq!(recovered.value.evidence, vec![evidence]);
    assert_eq!(
        owner
            .with_store(|s| s.diagnostic_summary())
            .await
            .unwrap()
            .active_runs,
        0
    );
    let new = owner.start_workflow("rejected", "2026-01-01T00:00:00Z".into());
    assert!(owner.begin_workflow(new).await.is_err());
    let before = owner.with_store(|s| s.diagnostic_summary()).await.unwrap();
    owner
        .with_store(|s| {
            let prepared = s.prepare_sequence_compaction()?;
            s.apply_sequence_compaction(prepared)
        })
        .await
        .unwrap();
    let after = owner.with_store(|s| s.diagnostic_summary()).await.unwrap();
    assert_eq!(
        before.protected_payload_rows_lower_bound,
        after.protected_payload_rows_lower_bound
    );
    assert_eq!(
        after
            .tables
            .iter()
            .find(|t| t.table == "changes")
            .unwrap()
            .sampled_rows,
        1
    );
    assert!(owner.get_work_item(work.value.id).await.unwrap().is_some());
    owner.shutdown().await;
}

/// Real daemon router, scratch database only. No socket, worker launch or live grant.
#[tokio::test]
async fn policy_rescue_router_saturation_exact_consent_atomic_receipt_and_reopen() {
    use base64::Engine;
    use hyper::{Body, Request, StatusCode};
    use refact_lsp::daemon::{config::DaemonConfig, events::EventBus, state::DaemonState};
    use tower::ServiceExt;
    async fn post(
        router: axum::Router,
        uri: &str,
        auth: &str,
        value: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("authorization", auth)
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
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
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let mut config = DaemonConfig::default();
    config.auth.enabled = true;
    config.auth.username = Some("policy-user".into());
    config.auth.password = Some("scratch-password".into());
    let state = DaemonState::new_with_daemon_dir(
        config,
        EventBus::new(temp.path().join("events.jsonl")),
        Some("worker-bearer".into()),
        temp.path().join("daemon"),
        0,
    );
    state.load_projects(temp.path().join("projects.json")).await;
    let entry = state.projects.write().await.open(root).await.unwrap();
    let project = ProjectIdentity {
        project_id: entry.id.clone(),
        canonical_root: entry.root.to_string_lossy().into_owned(),
    };
    drop(OperatorStore::open(&entry.root, &project).unwrap());
    let router = refact_lsp::daemon::server::make_router(state.clone(), 0);
    let preview_uri = format!("/daemon/v1/projects/{}/operator/policy/preview", entry.id);
    let confirm_uri = format!("/daemon/v1/projects/{}/operator/policy/confirm", entry.id);
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("policy-user:scratch-password")
    );
    let proposal = |cap| serde_json::json!({"project_id":entry.id,"policy":{"max_rows_per_namespace":cap,"replace_editorial_aggregate":false}});
    let confirmation = |preview: &serde_json::Value, key: &str| serde_json::json!({"consent":preview["consent"],"expected_digest":preview["digest"],"idempotency_key":key,"confirm":true});
    let (status, preview) = post(router.clone(), &preview_uri, &auth, proposal(1)).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let (status, initial) = post(
        router.clone(),
        &confirm_uri,
        &auth,
        confirmation(&preview, "initial"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    let mut store = OperatorStore::open(&entry.root, &project).unwrap();
    let mut count = 0;
    loop {
        let value = serde_json::json!({"schema_version":1,"project":project,"n":count});
        if store
            .put_document(
                &format!("abandoned_policy_preparation_{count}"),
                &format!("p-{count}"),
                &value,
                None,
            )
            .is_err()
        {
            break;
        }
        count += 1;
        assert!(
            count <= 128,
            "global budget must bound unclassified preparations"
        );
    }
    assert!(count > 0);
    drop(store);
    let db = rusqlite::Connection::open(entry.root.join(".refact/buddy/operator.sqlite")).unwrap();
    let counts = || -> (u64, u64) {
        (
            db.query_row("SELECT count(*) FROM operator_documents", [], |r| r.get(0))
                .unwrap(),
            db.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))
                .unwrap(),
        )
    };
    let before = counts();
    let (status, preview) = post(router.clone(), &preview_uri, &auth, proposal(4)).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        counts(),
        before,
        "preview must not allocate abandoned records"
    );
    let request = confirmation(&preview, "rescue");
    assert_eq!(
        post(
            router.clone(),
            &confirm_uri,
            "Bearer worker-bearer",
            request.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut wrong = request.clone();
    wrong["expected_digest"] = serde_json::json!("wrong");
    assert_eq!(
        post(router.clone(), &confirm_uri, &auth, wrong).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), before);
    let mut unconfirmed = request.clone();
    unconfirmed["confirm"] = serde_json::json!(false);
    assert_eq!(
        post(router.clone(), &confirm_uri, &auth, unconfirmed)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut changed = request.clone();
    changed["consent"]["proposed_policy"]["max_rows_per_namespace"] = serde_json::json!(100);
    assert_eq!(
        post(router.clone(), &confirm_uri, &auth, changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), before);
    db.execute_batch("CREATE TRIGGER policy_audit_failure BEFORE INSERT ON operator_documents WHEN NEW.namespace='policy_control_audit' BEGIN SELECT RAISE(ABORT,'scratch audit failure'); END;").unwrap();
    assert_eq!(
        post(router.clone(), &confirm_uri, &auth, request.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), before);
    db.execute_batch("DROP TRIGGER policy_audit_failure")
        .unwrap();
    let (status, receipt) = post(router.clone(), &confirm_uri, &auth, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["after"]["epoch"], 2);
    assert_eq!(receipt["audit"]["spec_hash"], preview["digest"]);
    let after = counts();
    let (status, replay) = post(router.clone(), &confirm_uri, &auth, request).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(receipt, replay);
    assert_eq!(counts(), after);
    assert_eq!(
        post(
            router.clone(),
            &confirm_uri,
            &auth,
            confirmation(&preview, "different-replay-key")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), after);
    let reopened = OperatorStore::open(&entry.root, &project).unwrap();
    assert_eq!(reopened.policy_snapshot().unwrap().epoch, 2);
    assert!(reopened
        .get_document::<serde_json::Value>("policy_control_audit", "document-policy:2")
        .unwrap()
        .is_some());
    drop(reopened);
    // A charge change without an epoch change invalidates otherwise exact consent.
    let (_, stale) = post(router.clone(), &preview_uri, &auth, proposal(8)).await;
    let mut writer = OperatorStore::open(&entry.root, &project).unwrap();
    writer
        .put_document(
            "abandoned_policy_preparation",
            "after-preview",
            &serde_json::json!({"schema_version":1,"project":project}),
            None,
        )
        .unwrap();
    drop(writer);
    let after = counts();
    assert_eq!(
        post(
            router.clone(),
            &confirm_uri,
            &auth,
            confirmation(&stale, "stale-charge")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), after);
    let (_, lower) = post(router.clone(), &preview_uri, &auth, proposal(1)).await;
    assert_eq!(
        post(
            router.clone(),
            &confirm_uri,
            &auth,
            confirmation(&lower, "lower")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(counts(), after);
}

#[test]
fn explicit_cleanup_preserves_documents_and_sequence_and_rejects_stale_prepare() {
    let root = tempfile::tempdir().unwrap();
    let project = identity(root.path());
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    let doc = serde_json::json!({"schema_version":1,"project":project,"pinned":true,"decision":"secret fixture value"});
    for n in 0..300 {
        store
            .put_document("legacy_receipt", &format!("receipt-{n}"), &doc, None)
            .unwrap();
    }
    let before = store.retention_preview().unwrap();
    assert_eq!(before.protected_payload_rows_lower_bound, 300);
    assert!(!before.evidence_deletion_supported);
    let stale = store.prepare_sequence_compaction().unwrap();
    let latest = store
        .put_document("editorial", "latest", &doc, None)
        .unwrap();
    assert!(store.apply_sequence_compaction(stale).is_err());
    let prepared = store.prepare_sequence_compaction().unwrap();
    assert_eq!(store.apply_sequence_compaction(prepared).unwrap(), 300);
    let prepared = store.prepare_sequence_compaction().unwrap();
    assert_eq!(store.apply_sequence_compaction(prepared).unwrap(), 0);
    let next = store.put_document("editorial", "next", &doc, None).unwrap();
    assert_eq!(next.change_seq, latest.change_seq + 1);
    let summary = serde_json::to_string(&store.diagnostic_summary().unwrap()).unwrap();
    assert!(!summary.contains("secret fixture"));
    drop(store);
    let store = OperatorStore::open(root.path(), &project).unwrap();
    assert_eq!(
        store
            .get_document::<serde_json::Value>("legacy_receipt", "receipt-0")
            .unwrap()
            .unwrap()
            .value,
        doc
    );
}
