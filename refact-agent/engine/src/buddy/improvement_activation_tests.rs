use super::*;
use crate::buddy::action_execution::{ActionRequest, ActionResult, AsyncActionRepository};
use crate::buddy::improvement_evaluation::{
    CheckObservation, CheckResult, Criterion, EvaluationProof, EvaluationSpec, register_spec,
};
use crate::buddy::resource_execution::{self as resources, RuntimeActionRepository};
use refact_buddy_core::operator_types::*;
use std::path::Path;

fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

struct LandingFixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    project: ProjectIdentity,
    runtime: OutcomeRuntime,
    app: crate::app_state::AppState,
    service: WorktreeService,
    created: crate::worktrees::types::CreateWorktreeResponse,
    exact: CheckedMergeIdentity,
    draft: LandingDraft,
    action: ActionSpec,
    prepared: PreparedAction,
    headers: axum::http::HeaderMap,
    grant: resources::ExactActionGrant,
    request: ActionRequest,
}

async fn landing_fixture() -> LandingFixture {
    use crate::http::routers::v1::buddy_operator::{GrantResourceRequest, handle_resource_grant};
    use axum::{extract::State, http::HeaderMap, Json};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(root.join(".gitignore"), ".refact/\n").unwrap();
    std::fs::write(root.join("file.txt"), "known-good\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "known good"]);
    let root = dunce::canonicalize(&root).unwrap();
    let project = crate::buddy::operator_runtime::project_identity(&root).unwrap();
    let worker = crate::buddy::operator_runtime::engine_worker_identity();
    let runtime = OutcomeRuntime::open(&root, project.clone(), worker.clone()).unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        temp.path().join("cache"),
        temp.path().join("config"),
    )
    .await;
    Arc::get_mut(&mut gcx).unwrap().cmdline.daemon_auth_token = Some("fixture-secret".into());
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let (tx, _) = tokio::sync::broadcast::channel(16);
    let mut buddy = crate::buddy::actor::BuddyService::new(
        root.clone(),
        crate::buddy::state::default_buddy_state(),
        Default::default(),
        vec![],
        crate::buddy::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );
    buddy.operator_features.storage = true;
    buddy.operator_features.actions = true;
    buddy.operator_features.improvement = true;
    buddy.outcome_runtime = Some(runtime.clone());
    *app.buddy.buddy.lock().await = Some(buddy);
    let service = WorktreeService::new(app.paths.cache_dir.clone(), root.clone()).unwrap();
    let created = service
        .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
            branch: Some("refact/chat/reviewed".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    std::fs::write(created.worktree.meta.root.join("file.txt"), "reviewed\n").unwrap();
    git(&created.worktree.meta.root, &["add", "file.txt"]);
    git(
        &created.worktree.meta.root,
        &["commit", "-m", "reviewed candidate"],
    );
    let diff = service
        .diff_worktree(&created.worktree.meta.id)
        .await
        .unwrap();
    let exact = CheckedMergeIdentity {
        source_head: diff.status.head_commit.clone().unwrap(),
        target_head: created.worktree.meta.base_commit.clone().unwrap(),
        target_branch: "main".into(),
        patch_digest: digest(&diff).unwrap(),
        target_files: vec!["file.txt".into()],
    };
    let draft = LandingDraft {
        project: project.clone(),
        candidate_id: "candidate".into(),
        proof_id: "proof".into(),
        worktree_id: created.worktree.meta.id.clone(),
        exact: exact.clone(),
    };
    let action = ActionSpec {
        schema_version: 1,
        action_id: "reviewed-landing".into(),
        kind: ACTION.into(),
        adapter: ADAPTER.into(),
        project: project.clone(),
        resource: ResourceRef {
            resource_kind: "worktree".into(),
            resource_id: draft.worktree_id.clone(),
            version: digest(&exact).unwrap(),
        },
        expected_version: digest(&exact).unwrap(),
        parameter_ref: Some("draft".into()),
        availability: ActionAvailability::Available,
        confirmation: ConfirmationClass::UserConfirmationRequired,
        authority_class: AuthorityClass::Mutating,
        capabilities: ActionCapabilities {
            can_preview: true,
            can_execute: true,
            can_verify: true,
            can_recover: true,
            can_cancel: false,
        },
    };
    resources::persist_draft(&runtime, "draft".into(), draft.clone())
        .await
        .unwrap();
    let identity = crate::buddy::improvement_runner::inspect_source(
        &service,
        &draft.worktree_id,
        &exact.target_files,
    )
    .await
    .unwrap();
    let project_copy = project.clone();
    let action_copy = action.clone();
    let draft_copy = draft.clone();
    runtime
        .with_store(move |store| {
            let contract: serde_json::Value = serde_json::from_str(include_str!(
                "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
            ))
            .unwrap();
            let mut original: RunOutcome =
                serde_json::from_value(contract["run_outcome"].clone()).unwrap();
            original.project = project_copy.clone();
            original.run_id = "problem".into();
            store.append_run_outcome(&original)?;
            let evaluator = NativeSourceIdentity {
                source_kind: NativeSourceKind::Tool,
                source_id: "independent-reviewer".into(),
                source_version: "1".into(),
            };
            let evaluation = EvaluationSpec {
                schema_version: 1,
                project: project_copy.clone(),
                id: "evaluation".into(),
                criteria: vec![Criterion {
                    id: "review".into(),
                    command_or_protocol: "fixed independent source review v1".into(),
                    expected_result: "approved".into(),
                    external_review: true,
                }],
                corpus_digest: "a".repeat(64),
                privacy_scope: "scratch fixture only".into(),
                authority: evaluator.clone(),
                max_age_seconds: 3600,
                execution: None,
            };
            register_spec(store, &evaluation, &evaluator)?;
            let draft_document: Stored<serde_json::Value> =
                store.get_document("resource_draft", "draft")?.unwrap();
            let candidate = CandidateRecord {
                schema_version: 1,
                project: project_copy.clone(),
                candidate: ImprovementCandidate {
                    schema_version: 1,
                    candidate_id: "candidate".into(),
                    project: project_copy.clone(),
                    target: "source_candidate".into(),
                    worktree_ref: Some(draft_copy.worktree_id.clone()),
                    config_draft_ref: None,
                    patch_digest: Some(draft_copy.exact.patch_digest.clone()),
                    build_identity: None,
                    evaluation_spec_version: evaluation.id.clone(),
                    proof_refs: vec![],
                    activation_decision: None,
                    previous_known_good_identity: Some(draft_copy.exact.target_head.clone()),
                },
                target: DeploymentTarget::SourceCandidate,
                target_files: draft_copy.exact.target_files.clone(),
                content_digest: draft_copy.exact.patch_digest.clone(),
                evaluation_digest: digest(&evaluation)?,
                source_identity: identity.source_digest,
                commit_identity: Some(draft_copy.exact.source_head.clone()),
                engine_build: None,
                gui_build: None,
                plugin_package: None,
                installed_version: None,
                running_host: None,
                original_run_id: original.run_id.clone(),
                original_problem: original.evidence.clone(),
                activation_action: Some(action_copy),
                activation_draft_digest: Some(digest(&draft_document.value)?),
                submitted_by: evaluator.clone(),
            };
            super::super::register_candidate(store, &candidate, &evaluator)?;
            let mut result = original;
            result.run_id = "review-result".into();
            result.input_fingerprint = digest(&candidate)?;
            result.status = RunStatus::Completed;
            result.effects.clear();
            result.evidence[0].source = evaluator.clone();
            result.evidence[0].digest = Some("b".repeat(64));
            result.evidence[0].observed_at = chrono::Utc::now().to_rfc3339();
            store.append_run_outcome(&result)?;
            let proof = EvaluationProof {
                schema_version: 1,
                project: project_copy,
                id: "proof".into(),
                candidate_digest: digest(&candidate)?,
                evaluation_digest: digest(&evaluation)?,
                authority: evaluator.clone(),
                observed_at: chrono::Utc::now().to_rfc3339(),
                checks: vec![CheckObservation {
                    criterion_id: "review".into(),
                    command_or_protocol: evaluation.criteria[0].command_or_protocol.clone(),
                    result: CheckResult::Passed,
                    observed_result: "retained fixture attestation, not live review".into(),
                    evidence: result.evidence[0].clone(),
                    run_id: result.run_id,
                }],
            };
            super::super::record_proof(store, "candidate", &proof, &evaluator)?;
            Ok(())
        })
        .await
        .unwrap();
    let catalog = resources::registered_catalog(app.clone(), &runtime, &action)
        .await
        .unwrap();
    let registry_before = std::fs::read(service.registry_path()).unwrap();
    catalog.preview(&action).await.unwrap();
    assert_eq!(
        std::fs::read(service.registry_path()).unwrap(),
        registry_before
    );
    assert!(!root.join(".refact/operator-recovery").exists());
    let prepared = PreparedAction {
        spec: action.clone(),
        bound_parameter_ref: action.parameter_ref.clone(),
        spec_hash: action_spec_hash(&action, &action.parameter_ref),
    };
    resources::bind_prepared_draft(&runtime, &prepared)
        .await
        .unwrap();
    RuntimeActionRepository(runtime.clone())
        .prepare(prepared.clone())
        .await
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
    let grant = handle_resource_grant(
        State(app.clone()),
        headers.clone(),
        Json(GrantResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            expires_in_seconds: 3600,
            recovery_execution_id: None,
            native_rights: None,
        }),
    )
    .await
    .unwrap()
    .0
    .data;
    let request = ActionRequest {
        execution_id: "apply".into(),
        idempotency_key: "apply-key".into(),
        spec: action.clone(),
        bound_parameter_ref: action.parameter_ref.clone(),
        requester: grant.requester.clone(),
        decision: grant.decision.clone(),
        worker: worker.clone(),
    };
    LandingFixture {
        _temp: temp,
        root,
        project,
        runtime,
        app,
        service,
        created,
        exact,
        draft,
        action,
        prepared,
        headers,
        grant,
        request,
    }
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn registered_reviewed_landing_revalidates_owner_grant_lease_and_recovery() {
    use crate::http::routers::v1::buddy_operator::{
        ExecuteResourceRequest, GrantResourceRequest, handle_resource_grant,
        handle_resource_recover,
    };
    use axum::{
        extract::{Path as RoutePath, State},
        Json,
    };
    let LandingFixture {
        _temp,
        root,
        project,
        runtime,
        app,
        service,
        created,
        exact,
        draft,
        action,
        prepared,
        headers,
        grant,
        request,
    } = landing_fixture().await;
    let adapter = LandingAdapter::new(
        app.clone(),
        runtime.clone(),
        service.clone(),
        Arc::new(resources::BoundDraft::new("draft".into(), draft.clone())),
        Arc::new(resources::PrivateRecoveryMaterials::open(&project).unwrap()),
    );
    assert_eq!(
        adapter.apply(&action).await,
        Err(ResourceError::NotApplied(
            "ValidationFailed(\"current resource execution authority unavailable\")".into()
        )),
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), exact.target_head);
    assert!(!root.join(".refact/operator-recovery").exists());
    let mut probe = request.clone();
    probe.execution_id = "phase-probe".into();
    probe.idempotency_key = "phase-probe-key".into();
    let claimed = crate::buddy::action_execution::ExecutionRecord {
        execution: crate::buddy::action_execution::execution_for(
            &probe,
            &prepared,
            ActionExecutionState::Claimed,
        ),
        effects: vec![],
        verification_evidence: vec![],
        recovery: None,
    };
    let repository = RuntimeActionRepository(runtime.clone());
    assert_eq!(
        repository
            .claim_prepared(claimed.clone(), &prepared)
            .await
            .unwrap(),
        crate::buddy::action_execution::ClaimResult::Claimed
    );
    repository
        .transition(
            "phase-probe",
            ActionExecutionState::Claimed,
            ActionExecutionState::Running,
        )
        .await
        .unwrap();
    let context = ResourceExecutionContext {
        runtime: runtime.clone(),
        execution: runtime
            .get_execution("phase-probe".into())
            .await
            .unwrap()
            .unwrap(),
        recovery: None,
    };
    let (guarded, mut renewal) = context
        .clone()
        .scope(adapter.guarded_service(&action, &draft, None))
        .await
        .unwrap();
    assert!(renewal.is_some());
    guarded.checkpoint_merge_phase().await.unwrap();
    let grant_id = grant.decision.as_ref().unwrap().native.source_id.clone();
    let id = grant_id.clone();
    runtime
        .with_store(move |store| {
            let stored = store
                .get_document::<ResourceDocument<resources::ExactActionGrant>>("action_grant", &id)?
                .unwrap();
            let mut changed = stored.value;
            changed.payload.expires_at = "2000-01-01T00:00:00Z".into();
            store.put_document("action_grant", &id, &changed, Some(stored.revision))?;
            Ok(())
        })
        .await
        .unwrap();
    let merge_request = MergeWorktreeRequest {
        strategy: WorktreeMergeStrategy::Squash,
        delete_after_merge: false,
        include_uncommitted: false,
        target_branch: Some("main".into()),
        acknowledge_dirty_baseline: false,
        ..Default::default()
    };
    assert!(guarded
        .merge_worktree_checked(&draft.worktree_id, merge_request.clone(), exact.clone())
        .await
        .is_err());
    assert!(context
        .clone()
        .scope(adapter.guarded_service(&action, &draft, None))
        .await
        .is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), exact.target_head);
    renewal.as_mut().unwrap().shutdown().await.unwrap();
    drop(guarded);
    let id = grant_id.clone();
    let valid = grant.clone();
    runtime
        .with_store(move |store| {
            let stored = store
                .get_document::<ResourceDocument<resources::ExactActionGrant>>("action_grant", &id)?
                .unwrap();
            store.put_document(
                "action_grant",
                &id,
                &ResourceDocument {
                    schema_version: 1,
                    project: stored.value.project,
                    payload: valid,
                },
                Some(stored.revision),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (guarded, mut renewal) = context
        .clone()
        .scope(adapter.guarded_service(&action, &draft, None))
        .await
        .unwrap();
    let id = grant_id;
    let action_copy = action.clone();
    runtime
        .with_store(move |store| {
            let stored = store
                .get_document::<ResourceDocument<resources::ExactActionGrant>>("action_grant", &id)?
                .unwrap();
            let previous_revision = stored.revision;
            let mut changed = stored.value;
            changed.payload.expires_at =
                (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
            assert!(changed.payload.authorize(
                &action_copy,
                &changed.payload.requester,
                changed.payload.decision.as_ref()
            ));
            let revised =
                store.put_document("action_grant", &id, &changed, Some(previous_revision))?;
            assert!(revised.revision > previous_revision);
            Ok(())
        })
        .await
        .unwrap();
    assert!(guarded
        .merge_worktree_checked(&draft.worktree_id, merge_request.clone(), exact.clone())
        .await
        .is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), exact.target_head);
    renewal.as_mut().unwrap().shutdown().await.unwrap();
    drop(guarded);
    let (guarded, mut renewal) = context
        .clone()
        .scope(adapter.guarded_service(&action, &draft, None))
        .await
        .unwrap();
    guarded.checkpoint_merge_phase().await.unwrap();
    renewal.as_mut().unwrap().shutdown().await.unwrap();
    assert!(guarded
        .merge_worktree_checked(&draft.worktree_id, merge_request, exact.clone(),)
        .await
        .is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), exact.target_head);
    drop(guarded);
    let mut finished = claimed;
    finished.execution.status = ActionExecutionState::Failed;
    repository.finish(finished).await.unwrap();
    let landed = match resources::execute_registered(app.clone(), runtime.clone(), request.clone())
        .await
        .unwrap()
    {
        ActionResult::Finished(value) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(landed.execution.status, ActionExecutionState::Completed);
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "reviewed\n"
    );
    assert!(created.worktree.meta.root.exists());
    assert!(matches!(
        resources::execute_registered(app.clone(), runtime.clone(), request.clone())
            .await
            .unwrap(),
        ActionResult::Duplicate(_)
    ));
    let recovery_grant = handle_resource_grant(
        State(app.clone()),
        headers.clone(),
        Json(GrantResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            expires_in_seconds: 3600,
            recovery_execution_id: Some(request.execution_id.clone()),
            native_rights: None,
        }),
    )
    .await
    .unwrap()
    .0
    .data;
    let recover = || ExecuteResourceRequest {
        project_id: project.project_id.clone(),
        spec_hash: prepared.spec_hash.clone(),
        grant_id: recovery_grant
            .decision
            .as_ref()
            .unwrap()
            .native
            .source_id
            .clone(),
        execution_id: "restore".into(),
        idempotency_key: "restore-key".into(),
    };
    std::fs::write(root.join("user.txt"), "retain\n").unwrap();
    assert!(handle_resource_recover(
        State(app.clone()),
        RoutePath("apply".into()),
        headers.clone(),
        Json(recover())
    )
    .await
    .is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("user.txt")).unwrap(),
        "retain\n"
    );
    assert!(runtime
        .get_execution("restore".into())
        .await
        .unwrap()
        .is_none());
    std::fs::remove_file(root.join("user.txt")).unwrap();
    let restored = handle_resource_recover(
        State(app.clone()),
        RoutePath("apply".into()),
        headers.clone(),
        Json(recover()),
    )
    .await
    .unwrap()
    .0
    .data;
    assert_eq!(restored.execution.status, ActionExecutionState::Completed);
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "known-good\n"
    );
    assert!(created.worktree.meta.root.exists());
    runtime.shutdown().await;
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn known_landing_survives_post_commit_revocation_and_release() {
    use crate::http::routers::v1::buddy_operator::{
        ExecuteResourceRequest, GrantResourceRequest, handle_resource_grant,
        handle_resource_recover,
    };
    use axum::{
        extract::{Path as RoutePath, State},
        Json,
    };
    let LandingFixture {
        _temp,
        root,
        project,
        runtime,
        app,
        service,
        created,
        exact,
        draft,
        action,
        prepared,
        headers,
        grant,
        request,
    } = landing_fixture().await;
    let tripped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let registry_before_landing = std::fs::read(service.registry_path()).unwrap();
    let result = {
        let target = root.clone();
        let base = exact.target_head.clone();
        let grant_id = grant.decision.as_ref().unwrap().native.source_id.clone();
        let owner = runtime.clone();
        let handle = tokio::runtime::Handle::current();
        let tripped = tripped.clone();
        let settings = crate::file_activity::settings();
        let coordinator = refact_work_core::CoordinationStore::open(
            &refact_work_core::default_store_path(&app.paths.cache_dir),
            &settings,
        )
        .await
        .unwrap();
        let manager = crate::coordination::leases::merge_leases_for(&app, &root)
            .await
            .unwrap()
            .unwrap();
        let project_key = refact_work_core::identity::ProjectKey::new(manager.project());
        let lease_key = crate::coordination::merge_gate::merge_lease_key(
            &crate::coordination::merge_gate::checkout_key(&root),
        );
        let observed = service.with_merge_authority(Arc::new(move || {
            let repo = git2::Repository::open(&target).unwrap();
            let head = repo
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string();
            if head != base && !tripped.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let grant_id = grant_id.clone();
                handle.block_on(async {
                    owner
                        .with_store(move |store| {
                            let stored = store
                                .get_document::<ResourceDocument<resources::ExactActionGrant>>(
                                    "action_grant",
                                    &grant_id,
                                )?
                                .unwrap();
                            let mut revoked = stored.value;
                            revoked.payload.revoked = true;
                            store.put_document(
                                "action_grant",
                                &grant_id,
                                &revoked,
                                Some(stored.revision),
                            )?;
                            Ok(())
                        })
                        .await
                        .unwrap();
                    let access = coordinator.authorize(project_key.clone());
                    let held = coordinator
                        .get_lease(&access, &lease_key)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(coordinator
                        .release_lease(&access, &held.authority())
                        .await
                        .unwrap());
                });
            }
            Ok(())
        }));
        let mut catalog = resources::ResourceCatalog::default();
        catalog
            .register(
                ACTION.into(),
                Arc::new(LandingAdapter::new(
                    app.clone(),
                    runtime.clone(),
                    observed,
                    Arc::new(resources::BoundDraft::new("draft".into(), draft.clone())),
                    Arc::new(resources::PrivateRecoveryMaterials::open(&project).unwrap()),
                )),
            )
            .unwrap();
        catalog
            .service(runtime.clone(), Arc::new(grant.clone()))
            .unwrap()
            .execute_async(&request)
            .await
    };
    let landed = match result {
        ActionResult::Finished(value) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(landed.execution.status, ActionExecutionState::Partial);
    let post = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(post.len(), 40);
    assert_ne!(post, exact.target_head);
    assert_eq!(git(&root, &["rev-parse", "HEAD^"]), exact.target_head);
    assert_eq!(
        git(&root, &["rev-parse", "HEAD^{tree}"]),
        git(&created.worktree.meta.root, &["rev-parse", "HEAD^{tree}"])
    );
    let receipt = resources::load_resource_receipt(&runtime, request.execution_id.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.expected_post_version, post);
    assert_eq!(receipt.resource_id, draft.worktree_id);
    assert_eq!(receipt.state, ResourceApplyState::Applied);
    let material: LandingDraft = serde_json::from_slice(
        &RecoveryMaterialStore::get(
            &resources::PrivateRecoveryMaterials::open(&project).unwrap(),
            &receipt.previous_material_reference,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(material, draft);
    assert_eq!(landed.effects[0].resource.version, post);
    let recovery = landed.recovery.as_ref().unwrap();
    assert!(recovery.available);
    assert_eq!(recovery.recovery_id, request.execution_id);
    assert_eq!(recovery.expected_post_version, post);
    assert_eq!(
        runtime
            .get_execution(request.execution_id.clone())
            .await
            .unwrap()
            .unwrap()
            .value,
        landed.execution
    );
    assert!(tripped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(landed.verification_evidence.is_empty());
    assert!(
        resources::load_grant(
            &runtime,
            grant.decision.as_ref().unwrap().native.source_id.clone()
        )
        .await
        .unwrap()
        .unwrap()
        .revoked
    );
    assert!(crate::coordination::leases::merge_leases_for(&app, &root)
        .await
        .unwrap()
        .unwrap()
        .inspect(
            &crate::coordination::merge_gate::merge_lease_key(
                &crate::coordination::merge_gate::checkout_key(&root),
            ),
            refact_work_core::store::records::now_ms(),
        )
        .await
        .unwrap()
        .is_none());
    assert!(landed.effects[0]
        .evidence_refs
        .iter()
        .any(|e| e.contains("authority lost")));
    assert_eq!(
        std::fs::read(service.registry_path()).unwrap(),
        registry_before_landing
    );
    assert!(created.worktree.meta.root.exists());
    assert!(
        resources::execute_registered(app.clone(), runtime.clone(), request.clone())
            .await
            .is_err()
    );
    assert!(handle_resource_recover(
        State(app.clone()),
        RoutePath("apply".into()),
        headers.clone(),
        Json(ExecuteResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            grant_id: grant.decision.as_ref().unwrap().native.source_id.clone(),
            execution_id: "revoked-restore".into(),
            idempotency_key: "revoked-restore-key".into(),
        }),
    )
    .await
    .is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), post);
    assert!(runtime
        .get_execution("revoked-restore".into())
        .await
        .unwrap()
        .is_none());
    let recovery_grant = handle_resource_grant(
        State(app.clone()),
        headers.clone(),
        Json(GrantResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            expires_in_seconds: 3600,
            recovery_execution_id: Some(request.execution_id.clone()),
            native_rights: None,
        }),
    )
    .await
    .unwrap()
    .0
    .data;
    let recover = || ExecuteResourceRequest {
        project_id: project.project_id.clone(),
        spec_hash: prepared.spec_hash.clone(),
        grant_id: recovery_grant
            .decision
            .as_ref()
            .unwrap()
            .native
            .source_id
            .clone(),
        execution_id: "restore".into(),
        idempotency_key: "restore-key".into(),
    };
    git(
        &root,
        &["commit", "--allow-empty", "-m", "later user commit"],
    );
    let later = git(&root, &["rev-parse", "HEAD"]);
    assert_ne!(later, post);
    assert!(handle_resource_recover(
        State(app.clone()),
        RoutePath("apply".into()),
        headers.clone(),
        Json(recover())
    )
    .await
    .is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), later);
    assert!(runtime
        .get_execution("restore".into())
        .await
        .unwrap()
        .is_none());
    git(&root, &["reset", "--hard", &post]);
    let restored = handle_resource_recover(
        State(app.clone()),
        RoutePath("apply".into()),
        headers,
        Json(recover()),
    )
    .await
    .unwrap()
    .0
    .data;
    assert_eq!(restored.execution.status, ActionExecutionState::Completed);
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "known-good\n"
    );
    assert!(created.worktree.meta.root.exists());
    runtime.shutdown().await;
}
