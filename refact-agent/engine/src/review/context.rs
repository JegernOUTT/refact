use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use refact_review_core::*;
use crate::{
    at_commands::at_commands::AtCommandsContext,
    chat::types::{ChatSession, SessionState, ChatEvent, TrajectoryCommitIntent},
    worktrees::{service::WorktreeService, scope::ExecutionScope, types::WorktreeMeta},
};
use crate::chat::worktree_lifecycle::{
    transition_lock, is_ordinary_worktree_lineage, check_ordinary_worktree_ancestry,
    check_unchanged_attachment,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetContextRequest {
    pub repository: String,
    pub base_ref: String,
    pub candidate_ref: String,
    #[serde(default)]
    pub policy: CapturePolicy,
    #[serde(default)]
    pub target_reference: String,
    #[serde(default)]
    pub title: String,
    pub review_id: Option<String>,
    pub expected_revision: Option<u64>,
    pub request_key: String,
}
#[derive(Serialize)]
pub struct ContextReceipt {
    pub review: Review,
    pub candidate_id: String,
    pub imported_material_id: String,
    pub investigation_material_id: String,
    pub execution_commit: String,
    pub worktree: WorktreeMeta,
    pub origin_records: Vec<refact_privacy::FileRecord>,
}
fn check_caller(
    context: &AtCommandsContext,
    session: &ChatSession,
) -> std::result::Result<(), String> {
    if !is_ordinary_worktree_lineage(&context.chat_id, &context.root_chat_id, &session.thread)
        || context.subchat_depth != 0
        || context.background_agent_id.is_some()
        || context.task_meta.is_some()
        || session.closed
        || session.chat_id != context.chat_id
        || session.runtime.state != SessionState::ExecutingTools
        || !Arc::ptr_eq(&session.abort_flag, &context.abort_flag)
        || context.abort_flag.load(Ordering::Relaxed)
        || context.execution_scope_worktree() != session.thread.worktree
    {
        return Err(
            "Review context requires an ordinary live root tool execution with unchanged scope"
                .into(),
        );
    }
    Ok(())
}

pub(crate) async fn install_origin_labels(
    app: &crate::app_state::AppState,
    session: &Arc<Mutex<ChatSession>>,
    candidate: &Candidate,
    worktree: &WorktreeMeta,
) -> std::result::Result<(), String> {
    let labels = derive_origin_labels(app, candidate, worktree).await?;
    let derived = session.lock().await.derived_privacy_zones.clone();
    let mut installed = derived.write().map_err(|_| "Privacy label lock poisoned")?;
    installed.extend(labels);
    Ok(())
}

/// Construct a complete label batch without changing the live session. The
/// batch is provenance, not permission to publish or use Review context.
pub(crate) async fn derive_origin_labels(
    app: &crate::app_state::AppState,
    candidate: &Candidate,
    worktree: &WorktreeMeta,
) -> std::result::Result<Vec<(PathBuf, crate::privacy::records::DerivedZone)>, String> {
    let started = std::time::Instant::now();
    let origins = candidate
        .files
        .iter()
        .map(|f| PathBuf::from(&candidate.repository).join(&f.path))
        .collect::<Vec<_>>();
    let read_context = crate::files_in_workspace::prepare_file_read_context(app.gcx.clone()).await;
    tracing::debug!(
        phase = "read_context",
        files = origins.len(),
        elapsed_us = started.elapsed().as_micros() as u64,
        "Review origin preparation"
    );
    let checked = std::time::Instant::now();
    for path in &origins {
        crate::files_in_workspace::check_file_privacy_for_read_preparation_with_context(
            &read_context,
            path,
        )?;
    }
    tracing::debug!(
        phase = "origin_checks",
        files = origins.len(),
        elapsed_us = checked.elapsed().as_micros() as u64,
        "Review origin preparation"
    );
    let recorded = std::time::Instant::now();
    let records = crate::privacy::records::declared_file_records(&app.gcx, origins)?;
    tracing::debug!(
        phase = "origin_records",
        files = records.len(),
        elapsed_us = recorded.elapsed().as_micros() as u64,
        "Review origin preparation"
    );
    let mut records_by_path = HashMap::with_capacity(records.len());
    for record in &records {
        records_by_path
            .entry(PathBuf::from(&record.path))
            .or_insert(record);
    }
    let mut labels = Vec::with_capacity(candidate.files.len());
    for file in &candidate.files {
        let origin = PathBuf::from(&candidate.repository).join(&file.path);
        if let Some(record) = records_by_path.get(&origin) {
            labels.push((
                worktree.root.join(&file.path),
                crate::privacy::records::DerivedZone {
                    zone: record.zone.clone(),
                    origin: Some(record.path.clone()),
                },
            ));
        }
    }
    tracing::debug!(
        phase = "complete",
        files = candidate.files.len(),
        elapsed_us = started.elapsed().as_micros() as u64,
        "Review origin preparation"
    );
    Ok(labels)
}

/// Private binding intent. Resolving and validating this value has no session effects.
#[derive(Clone)]
pub(crate) struct PreparedBinding {
    pub candidate: Candidate,
    pub worktree: WorktreeMeta,
    pub previous: Option<WorktreeMeta>,
    pub review_id: String,
}

impl PreparedBinding {
    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.review_id == other.review_id
            && self.candidate == other.candidate
            && self.worktree == other.worktree
            && self.previous == other.previous
    }
}

pub(crate) async fn resolve_binding(
    app: &crate::app_state::AppState,
    chat_id: &str,
) -> std::result::Result<Option<PreparedBinding>, String> {
    let root = super::service::trusted_root(app, chat_id).await?;
    let Some(review) = super::service::get_for_chat(root.clone(), chat_id.into())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let Some(candidate_id) = &review.current_candidate_id else {
        return Ok(None);
    };
    let material = review
        .materials
        .iter()
        .rev()
        .find(|m| {
            m.id == format!("activation:{candidate_id}")
                && m.kind == MaterialKind::Investigation
                && &m.candidate_id == candidate_id
        })
        .ok_or("Active review has no investigation binding")?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("Chat unavailable")?;
    let previous = session.lock().await.thread.worktree.clone();
    let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), root).await?;
    let worktree = service
        .get_worktree(
            material
                .worktree_id
                .as_deref()
                .ok_or("Activation checkout missing")?,
        )
        .await?
        .meta;
    if worktree.kind != "review"
        || material.worktree_root.as_deref() != worktree.root.to_str()
        || service.validate_worktree_meta_strict(&worktree).await? != worktree
    {
        return Err("Review registry binding changed".into());
    }
    if previous.as_ref() != Some(&worktree) {
        if crate::agents::seal::seal_reason(app, chat_id)
            .await
            .is_some()
            || app
                .agents
                .find_agent_id_by_child_chat_id(chat_id)
                .await
                .is_some()
        {
            return Err("Review recovery forbidden for sealed/background session".into());
        }
        if let Some(old) = &previous {
            let owner = review.owner_chat_id.clone();
            let old = old.clone();
            let prior =
                super::service::with_store(worktree.source_workspace_root.clone(), move |store| {
                    Ok(store.list_for_chat(&owner)?.iter().any(|r| {
                        r.materials.iter().any(|m| {
                            m.id.starts_with("activation:")
                                && m.worktree_id.as_deref() == Some(&old.id)
                                && m.worktree_root.as_deref() == old.root.to_str()
                        })
                    }))
                })
                .await
                .map_err(|e| e.to_string())?;
            if !prior {
                return Err("Conflicting non-review attachment; explicit recovery required".into());
            }
        }
    }
    let candidate = review
        .candidates
        .iter()
        .find(|c| &c.id == candidate_id)
        .ok_or("Active candidate missing")?
        .clone();
    Ok(Some(PreparedBinding {
        candidate,
        worktree,
        previous,
        review_id: review.id,
    }))
}

/// Publish the complete batch and optional recovery attachment in one session critical section.
/// The caller holds the lifecycle lease and must validate turn and policy immediately before this call.
pub(crate) async fn publish_binding(
    app: &crate::app_state::AppState,
    chat_id: &str,
    binding: &PreparedBinding,
    labels: Vec<(PathBuf, crate::privacy::records::DerivedZone)>,
) -> std::result::Result<(), String> {
    publish_binding_checked(app, chat_id, binding, labels, None).await
}

pub(crate) async fn publish_binding_checked(
    app: &crate::app_state::AppState,
    chat_id: &str,
    binding: &PreparedBinding,
    labels: Vec<(PathBuf, crate::privacy::records::DerivedZone)>,
    identity: Option<&super::readiness::AdmissionIdentity>,
) -> std::result::Result<(), String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("Chat unavailable")?;
    let recovered = {
        let mut s = session.lock().await;
        if identity.is_some_and(|identity| !identity.matches(&s))
            || s.closed
            || s.abort_flag.load(Ordering::Acquire)
            || s.thread.worktree != binding.previous
            || (binding.previous.as_ref() != Some(&binding.worktree)
                && s.thread.task_meta.is_some())
            || s.work_project_root
                .as_ref()
                .and_then(|root| dunce::canonicalize(root).ok())
                != Some(binding.worktree.source_workspace_root.clone())
        {
            return Err("Review publication session identity changed".into());
        }
        let derived = s.derived_privacy_zones.clone();
        // Obtain the only fallible publication lock before changing either labels or attachment.
        let mut installed = derived.write().map_err(|_| "Privacy label lock poisoned")?;
        installed.extend(labels);
        let recovered = s.thread.worktree.as_ref() != Some(&binding.worktree);
        if recovered {
            s.thread.worktree = Some(binding.worktree.clone());
            s.release_turn_only_state();
            s.emit(ChatEvent::ThreadUpdated {
                params: serde_json::json!({"worktree":binding.worktree}),
            });
            s.increment_version();
            s.touch();
        }
        recovered
    };
    if recovered {
        crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session,
            TrajectoryCommitIntent::Required,
        )
        .await?;
    }
    Ok(())
}

/// Legacy direct callers still perform fresh checks; model preparation uses readiness instead.
pub async fn verify_binding_for_chat(
    app: &crate::app_state::AppState,
    chat_id: &str,
) -> std::result::Result<(), String> {
    let Some(binding) = resolve_binding(app, chat_id).await? else {
        return Ok(());
    };
    let labels = derive_origin_labels(app, &binding.candidate, &binding.worktree).await?;
    let current = resolve_binding(app, chat_id)
        .await?
        .ok_or("Review binding disappeared")?;
    if !binding.same_identity(&current) {
        return Err("Review binding changed during preparation".into());
    }
    publish_binding(app, chat_id, &binding, labels).await
}

/// Carry actual runtime source provenance, never model-supplied privacy claims.
pub(crate) async fn chat_source_privacy(
    app: &crate::app_state::AppState,
    chat_id: &str,
) -> std::result::Result<Vec<refact_privacy::FileRecord>, String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("Owning chat unavailable")?;
    let session = session.lock().await;
    crate::privacy::records::records_to_carry(&session.messages).map_err(|e| e.to_string())
}

pub async fn set_context(
    ccx: Arc<Mutex<AtCommandsContext>>,
    request: SetContextRequest,
) -> std::result::Result<ContextReceipt, String> {
    if request.request_key.is_empty()
        || request.repository.len() > 4096
        || request.base_ref.len() > 1024
        || request.candidate_ref.len() > 1024
    {
        return Err("Invalid review capture request".into());
    }
    let (app, chat_id, root_chat_id) = {
        let c = ccx.lock().await;
        (c.app.clone(), c.chat_id.clone(), c.root_chat_id.clone())
    };
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or("Current session unavailable")?;
    let _transition = transition_lock(&session).await;
    if let Some(reason) = crate::agents::seal::seal_reason(&app, &chat_id).await {
        return Err(reason.message());
    }
    if app
        .agents
        .find_agent_id_by_child_chat_id(&chat_id)
        .await
        .is_some()
    {
        return Err("Background review scope changes forbidden".into());
    }
    let (before, source) = {
        let c = ccx.lock().await;
        let s = session.lock().await;
        check_caller(&c, &s)?;
        (
            s.thread.clone(),
            s.work_project_root
                .clone()
                .ok_or("Trusted project unavailable")?,
        )
    };
    check_ordinary_worktree_ancestry(&app, &chat_id, &root_chat_id, &before).await?;
    let source = dunce::canonicalize(source).map_err(|error| error.to_string())?;
    let gate_owner = chat_id.clone();
    let confirmed = super::service::with_store(source.clone(), move |store| {
        store.stack_scope_confirmed(&gate_owner)
    })
    .await
    .map_err(|e| e.to_string())?;
    if !confirmed {
        return Err("Current stack scope choice has not been confirmed".into());
    }
    let fingerprint = content_hash(&serde_json::to_vec(&request).map_err(|e| e.to_string())?);
    if let Some(review) = super::service::get_for_chat(source.clone(), chat_id.clone())
        .await
        .map_err(|e| e.to_string())?
    {
        if let Some(candidate) = review
            .current_candidate_id
            .as_ref()
            .and_then(|id| review.candidates.iter().find(|c| &c.id == id))
        {
            if let Some(material) = review.materials.iter().find(|m| {
                m.id == format!("activation:{}", candidate.id)
                    && m.input_digests.get("context_request") == Some(&fingerprint)
            }) {
                let service =
                    WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone()).await?;
                let worktree = service
                    .get_worktree(
                        material
                            .worktree_id
                            .as_deref()
                            .ok_or("Activation intent missing checkout")?,
                    )
                    .await?
                    .meta;
                if material.worktree_root.as_deref() != worktree.root.to_str()
                    || worktree.kind != "review"
                {
                    return Err("Activation intent checkout mismatch".into());
                }
                install_origin_labels(&app, &session, candidate, &worktree).await?;
                let origin_records = crate::privacy::records::declared_file_records(
                    &app.gcx,
                    candidate
                        .files
                        .iter()
                        .map(|f| PathBuf::from(&candidate.repository).join(&f.path)),
                )?;
                {
                    let mut c = ccx.lock().await;
                    let mut s = session.lock().await;
                    check_caller(&c, &s)?;
                    check_unchanged_attachment(&s, &before)?;
                    s.thread.worktree = Some(worktree.clone());
                    c.execution_scope = Some(ExecutionScope::from_worktree(&worktree));
                    s.release_turn_only_state();
                    s.emit(ChatEvent::ThreadUpdated {
                        params: serde_json::json!({"worktree":worktree}),
                    });
                    s.increment_version();
                    s.touch();
                }
                crate::chat::trajectories::try_save_trajectory_with_intent(
                    app.clone(),
                    session.clone(),
                    TrajectoryCommitIntent::Required,
                )
                .await?;
                let imported_material_id = review
                    .materials
                    .iter()
                    .find(|m| m.kind == MaterialKind::Imported && m.candidate_id == candidate.id)
                    .ok_or("Imported material missing")?
                    .id
                    .clone();
                let candidate_id = candidate.id.clone();
                let investigation_material_id = material.id.clone();
                let execution_commit = material
                    .input_digests
                    .get("execution_commit")
                    .cloned()
                    .ok_or("Execution commit missing")?;
                return Ok(ContextReceipt {
                    review,
                    candidate_id,
                    imported_material_id,
                    investigation_material_id,
                    execution_commit,
                    worktree,
                    origin_records,
                });
            }
        }
    }
    let explicit_refresh_capture = if let Some(id) = &request.review_id {
        let id = id.clone();
        let owner = chat_id.clone();
        let attached = before.worktree.clone();
        super::service::with_store(source.clone(), move |store| {
            let review = store.load(&id)?;
            Ok(review.owner_chat_id == owner
                && store.stack_scope_confirmed(&owner)?
                && store.get_stack_for_chat(&owner)?.is_some_and(|stack| {
                    stack
                        .members
                        .iter()
                        .any(|member| member.review_id.as_deref() == Some(id.as_str()))
                })
                && review
                    .current_candidate_id
                    .as_ref()
                    .is_some_and(|candidate| {
                        !review
                            .materials
                            .iter()
                            .any(|material| material.id == format!("activation:{candidate}"))
                    })
                && store
                    .get_for_chat(&owner)?
                    .is_some_and(|active| active.id == review.id)
                && review.materials.iter().any(|material| {
                    material.kind == MaterialKind::Investigation
                        && material.id == format!("activation:{}", material.candidate_id)
                        && attached.as_ref().is_some_and(|worktree| {
                            worktree.kind == "review"
                                && material.worktree_id.as_deref() == Some(worktree.id.as_str())
                                && material.worktree_root.as_deref() == worktree.root.to_str()
                        })
                }))
        })
        .await
        .map_err(|e| e.to_string())?
    } else {
        false
    };
    if explicit_refresh_capture {
        let attached = before
            .worktree
            .as_ref()
            .ok_or("Prior investigation checkout missing")?;
        let registry =
            WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone()).await?;
        if registry.validate_worktree_meta_strict(attached).await? != *attached {
            return Err("Prior investigation registry binding changed".into());
        }
    } else {
        verify_binding_for_chat(&app, &chat_id).await?;
    }
    let repository = dunce::canonicalize(&request.repository).map_err(|e| e.to_string())?;
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &repository)
        .await?;
    let owner = chat_id.clone();
    let req = request.clone();
    let capture_repository = repository.clone();
    let privacy = chat_source_privacy(&app, &chat_id).await?;
    let mut expected_active = super::service::get_for_chat(source.clone(), chat_id.clone())
        .await
        .map_err(|e| e.to_string())?
        .map(|r| r.id);
    let (mut review, candidate, files, created_singleton) = super::service::with_store(source.clone(), move |store| {
        if !store.stack_scope_confirmed(&owner)? {
            return Err(Error::Invalid("A known stack is awaiting the user's exact current scope choice; capture requires a fresh answered receipt and stack_import".into()));
        }
        if let Some(stack) = store.get_stack_for_chat(&owner)? {
            let member = stack.members.iter().find(|member| member.review_id.as_deref() == req.review_id.as_deref() && req.review_id.is_some()).ok_or_else(|| Error::Invalid("Known stack: select a confirmed member via stack_capture before reviewing; implicit singleton capture is forbidden".into()))?;
            if !stack.scope_choice.confirmed || member.state != StackMemberState::Ready || (stack.scope_choice.scope == StackScope::Single && stack.scope_choice.selected_member_id != member.id)
                || req.policy != CapturePolicy::default() || member.resolved_base_oid.as_deref() != Some(req.base_ref.as_str()) || member.head_oid.as_deref() != Some(req.candidate_ref.as_str()) || stack.repository != req.repository {
                return Err(Error::Invalid("Capture differs from confirmed pinned stack member; explicitly refresh the manifest first".into()));
            }
        }
        let selected = match &req.review_id {
            Some(id) => { let review = store.load(id)?; if review.owner_chat_id != owner { return Err(Error::NotFound); } Some(review) },
            None => store.get_for_chat(&owner)?,
        };
        let created_singleton = selected.is_none();
        let review = match selected {
            Some(r) => {
                if req.review_id.as_deref().is_some_and(|id| id != r.id) {
                    return Err(Error::NotFound);
                }
                if req.expected_revision != Some(r.revision) {
                    return Err(Error::Invalid(
                        "Expected current review revision required".into(),
                    ));
                }
                r
            }
            None => {
                if req.review_id.is_some() || req.expected_revision.is_some_and(|r| r != 0) {
                    return Err(Error::NotFound);
                }
                store.create_with_privacy(&owner, &req.title, Origin::Imported, &req.request_key, privacy)?
            }
        };
        let repo = git2::Repository::open(&capture_repository)?;
        let base_oid = repo
            .revparse_single(&req.base_ref)?
            .peel_to_commit()?
            .id()
            .to_string();
        let head_oid = repo
            .revparse_single(&req.candidate_ref)?
            .peel_to_commit()?
            .id()
            .to_string();
        let candidate = store.capture(
            &capture_repository,
            CaptureRequest {
                base_oid,
                head_oid,
                policy: req.policy,
                origin: Origin::Imported,
            },
        )?;
        let files = candidate
            .files
            .iter()
            .filter_map(|f| f.head.as_ref().map(|blob| (f, blob)))
            .map(|(f, blob)| {
                Ok(refact_worktrees::review::SnapshotFile {
                    path: f.path.clone(),
                    bytes: store.read_blob(blob)?,
                    mode: f
                        .head_mode
                        .ok_or_else(|| Error::Invalid("Missing file mode".into()))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((review, candidate, files, created_singleton))
    })
    .await
    .map_err(|e| e.to_string())?;
    if created_singleton && expected_active.is_none() {
        expected_active = Some(review.id.clone());
    }
    let mut origins = vec![repository.clone()];
    for file in &candidate.files {
        let path = repository.join(&file.path);
        crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &path)
            .await?;
        origins.push(path);
    }
    let origin_records = crate::privacy::records::declared_file_records(&app.gcx, origins)?;
    let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone()).await?;
    {
        let c = ccx.lock().await;
        let s = session.lock().await;
        check_caller(&c, &s)?;
        check_unchanged_attachment(&s, &before)?;
    }
    if let Some(current) = review
        .current_candidate_id
        .as_ref()
        .and_then(|id| review.candidates.iter().find(|c| &c.id == id))
    {
        if expected_active.as_deref() == Some(review.id.as_str())
            && current.base_oid == candidate.base_oid
            && current.head_oid == candidate.head_oid
            && current.content_digest == candidate.content_digest
            && current.policy == candidate.policy
            && current.repository == candidate.repository
            && current.origin == candidate.origin
            && review.target_reference == request.target_reference
            && review
                .materials
                .iter()
                .any(|m| m.id == format!("activation:{}", current.id))
        {
            let material = review
                .materials
                .iter()
                .find(|m| m.id == format!("activation:{}", current.id))
                .ok_or("Current activation missing")?;
            let worktree = service
                .get_worktree(
                    material
                        .worktree_id
                        .as_deref()
                        .ok_or("Current checkout missing")?,
                )
                .await?
                .meta;
            let imported_material_id = review
                .materials
                .iter()
                .find(|m| m.kind == MaterialKind::Imported && m.candidate_id == current.id)
                .ok_or("Current imported material missing")?
                .id
                .clone();
            let candidate_id = current.id.clone();
            let investigation_material_id = material.id.clone();
            let execution_commit = material
                .input_digests
                .get("execution_commit")
                .cloned()
                .ok_or("Execution commit missing")?;
            return Ok(ContextReceipt {
                review,
                candidate_id,
                imported_material_id,
                investigation_material_id,
                execution_commit,
                worktree,
                origin_records,
            });
        }
    }
    let worktree = service
        .create_review_snapshot(&chat_id, &review.id, files)
        .await?
        .worktree
        .meta;
    install_origin_labels(&app, &session, &candidate, &worktree).await?;
    let execution_commit = worktree
        .base_commit
        .clone()
        .ok_or("Snapshot execution commit missing")?;
    let imported_material_id = uuid::Uuid::new_v4().to_string();
    let investigation_material_id = format!("activation:{}", candidate.id);
    let mut input_digests = BTreeMap::new();
    input_digests.insert("context_request".into(), fingerprint);
    input_digests.insert(
        "comparison_content".into(),
        candidate.content_digest.clone(),
    );
    input_digests.insert("execution_commit".into(), execution_commit.clone());
    input_digests.insert("comparison_base".into(), candidate.base_oid.clone());
    input_digests.insert("comparison_head".into(), candidate.head_oid.clone());
    let imported = Material {
        id: imported_material_id.clone(),
        kind: MaterialKind::Imported,
        candidate_id: candidate.id.clone(),
        agent_id: None,
        description: "Immutable imported comparison; not the execution commit".into(),
        content_digest: candidate.content_digest.clone(),
        input_digests: input_digests.clone(),
        environment: None,
        worktree_id: None,
        worktree_root: None,
        artifacts: vec![],
    };
    let investigation = Material {
        id: investigation_material_id.clone(),
        kind: MaterialKind::Investigation,
        candidate_id: candidate.id.clone(),
        agent_id: None,
        description: "Durable activation intent: exact registered execution checkout".into(),
        content_digest: candidate.content_digest.clone(),
        input_digests,
        environment: None,
        worktree_id: Some(worktree.id.clone()),
        worktree_root: Some(worktree.root.to_string_lossy().into()),
        artifacts: vec![],
    };
    {
        let c = ccx.lock().await;
        let s = session.lock().await;
        check_caller(&c, &s)?;
        check_unchanged_attachment(&s, &before)?;
    }
    let mut privacy = chat_source_privacy(&app, &chat_id).await?;
    privacy.extend(origin_records.clone());
    review = super::service::activate_owned(
        source.clone(),
        chat_id.clone(),
        review.id.clone(),
        expected_active,
        Mutation {
            expected_revision: review.revision,
            request_key: request.request_key,
            actor: Actor {
                kind: ActorKind::Agent,
                id: chat_id.clone(),
            },
            at_ms: chrono::Utc::now().timestamp_millis(),
            operation: Operation::ActivateSnapshot {
                candidate: candidate.clone(),
                imported,
                investigation,
                target_reference: request.target_reference,
            },
        },
        privacy,
    )
    .await
    .map_err(|e| e.to_string())?;
    {
        let mut c = ccx.lock().await;
        let mut s = session.lock().await;
        check_caller(&c, &s)?;
        check_unchanged_attachment(&s, &before)?;
        if s.work_project_root
            .as_ref()
            .and_then(|root| dunce::canonicalize(root).ok())
            != Some(source.clone())
        {
            return Err("Trusted source changed; review binding must be recovered".into());
        }
        s.thread.worktree = Some(worktree.clone());
        c.execution_scope = Some(ExecutionScope::from_worktree(&worktree));
        s.release_turn_only_state();
        s.emit(ChatEvent::ThreadUpdated {
            params: serde_json::json!({"worktree":worktree}),
        });
        let mut message = crate::chat::queue::worktree_activation_message(&worktree);
        crate::privacy::records::merge_records(
            &mut message,
            origin_records
                .iter()
                .filter(|record| record.zone != "normal")
                .cloned(),
        );
        s.add_message(message);
        s.increment_version();
        s.touch();
    }
    crate::chat::trajectories::try_save_trajectory_with_intent(app, session, TrajectoryCommitIntent::Required).await.map_err(|e| format!("Review activated but trajectory publication failed; binding verification must fail closed on reload: {e}"))?;
    Ok(ContextReceipt {
        review,
        candidate_id: candidate.id,
        imported_material_id,
        investigation_material_id,
        execution_commit,
        worktree,
        origin_records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn origin_labels_match_records_and_recheck_changed_policy_and_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let repo = git2::Repository::init(&source).unwrap();
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let signature = git2::Signature::now("Test", "test@localhost").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial",
            &repo.find_tree(tree_oid).unwrap(),
            &[],
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        let source = dunce::canonicalize(source).unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let service = WorktreeService::new_async(gcx.cache_dir.clone(), source.clone())
            .await
            .unwrap();
        let worktree = service
            .create_review_snapshot("labels-root", "labels-review", vec![])
            .await
            .unwrap()
            .worktree
            .meta;
        let candidate = Candidate {
            id: "labels-candidate".into(),
            base_oid: String::new(),
            head_oid: String::new(),
            content_digest: String::new(),
            snapshot_scope: String::new(),
            created_at_ms: 0,
            repository: source.to_string_lossy().into_owned(),
            origin: Origin::Imported,
            policy: CapturePolicy::default(),
            files: (0..64)
                .map(|index| CandidateFile {
                    path: format!("file-{index}.rs"),
                    base: None,
                    head: None,
                    base_mode: None,
                    head_mode: None,
                })
                .collect(),
        };
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let session = Arc::new(Mutex::new(ChatSession::new("labels-root".into())));
        let derived = session.lock().await.derived_privacy_zones.clone();
        let batch = derive_origin_labels(&app, &candidate, &worktree)
            .await
            .unwrap();
        assert_eq!(batch.len(), candidate.files.len());
        assert!(
            derived.read().unwrap().is_empty(),
            "derivation must not publish labels"
        );
        install_origin_labels(&app, &session, &candidate, &worktree)
            .await
            .unwrap();
        let expected = crate::privacy::records::declared_file_records(
            &gcx,
            candidate.files.iter().map(|file| source.join(&file.path)),
        )
        .unwrap();
        let derived = session.lock().await.derived_privacy_zones.clone();
        {
            let labels = derived.read().unwrap();
            assert_eq!(labels.len(), candidate.files.len());
            for file in &candidate.files {
                let origin = source.join(&file.path);
                let record = expected
                    .iter()
                    .find(|r| PathBuf::from(&r.path) == origin)
                    .unwrap();
                let label = labels.get(&worktree.root.join(&file.path)).unwrap();
                assert_eq!(label.zone, record.zone);
                assert_eq!(label.origin.as_deref(), Some(record.path.as_str()));
            }
        }
        for blocked in [source.join("file-63.rs"), worktree.root.join("file-63.rs")] {
            let current = gcx.privacy_settings.read().unwrap().clone();
            let settings = refact_core::privacy_types::PrivacySettings {
                loaded_ts: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                privacy_rules: refact_core::privacy_types::FilePrivacySettings {
                    only_send_to_servers_I_control: current
                        .privacy_rules
                        .only_send_to_servers_I_control
                        .clone(),
                    blocked: vec![blocked.to_string_lossy().into_owned()],
                },
            };
            *gcx.privacy_settings.write().unwrap() = Arc::new(settings);
            let before = serde_json::to_value(&*derived.read().unwrap()).unwrap();
            let error = install_origin_labels(&app, &session, &candidate, &worktree)
                .await
                .unwrap_err();
            assert!(error.contains("Blocked"), "{error}");
            assert_eq!(
                serde_json::to_value(&*derived.read().unwrap()).unwrap(),
                before,
                "a denied batch must not partially install labels"
            );
        }
    }

    #[tokio::test]
    async fn context_activation_update_preserves_old_edits_and_recovers_exact_request() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let repo = git2::Repository::init(&source).unwrap();
        let blob = repo.blob(b"original\n").unwrap();
        let mut tree = repo.treebuilder(None).unwrap();
        tree.insert("tracked", blob, 0o100644).unwrap();
        let tree_oid = tree.write().unwrap();
        let signature = git2::Signature::now("Test", "test@localhost").unwrap();
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &repo.find_tree(tree_oid).unwrap(),
                &[],
            )
            .unwrap();
        repo.checkout_head(None).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        // Readiness validates the captured on-disk policy, not just the test
        // context's default in-memory settings. Keep its source explicit here.
        crate::privacy::save_privacy_policy(gcx.clone(), crate::privacy::PrivacyPolicy::default())
            .await
            .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let mut live = ChatSession::new("review-context-root".into());
        live.work_project_root = Some(source.clone());
        live.set_runtime_state(SessionState::ExecutingTools, None);
        let abort = live.abort_flag.clone();
        let session = Arc::new(Mutex::new(live));
        app.chat
            .sessions
            .write()
            .await
            .insert("review-context-root".into(), session.clone());
        let context = AtCommandsContext::new_with_abort(
            app.clone(),
            4096,
            10,
            false,
            vec![],
            "review-context-root".into(),
            None,
            String::new(),
            None,
            None,
            Some(abort.clone()),
        )
        .await;
        let ccx = Arc::new(Mutex::new(context));
        let request = SetContextRequest {
            repository: source.to_string_lossy().into(),
            base_ref: oid.to_string(),
            candidate_ref: oid.to_string(),
            policy: CapturePolicy::default(),
            title: "Review".into(),
            target_reference: "local".into(),
            review_id: None,
            expected_revision: None,
            request_key: "first".into(),
        };
        abort.store(true, Ordering::Relaxed);
        assert!(set_context(ccx.clone(), request.clone()).await.is_err());
        assert!(
            !source.join(".refact/reviews").exists(),
            "cancelled caller must have no durable capture side effect"
        );
        abort.store(false, Ordering::Relaxed);
        let mut source_message = crate::call_validation::ChatMessage::new(
            "tool".into(),
            "Actually retrieved source context".into(),
        );
        let source_record: refact_privacy::FileRecord = serde_json::from_value(serde_json::json!({
            "path":source.join("design.md"), "zone":"normal", "attribution":"declared"
        }))
        .unwrap();
        crate::privacy::records::merge_records(&mut source_message, vec![source_record.clone()]);
        session.lock().await.add_message(source_message);
        let first = set_context(ccx.clone(), request.clone()).await.unwrap();
        assert!(
            first.review.privacy.contains(&source_record),
            "direct capture must retain actual chat source provenance"
        );
        assert_eq!(
            std::fs::read(first.worktree.root.join("tracked")).unwrap(),
            b"original\n"
        );
        assert_ne!(first.execution_commit, oid.to_string());
        // The model-facing path always returns a receipt, then admits its private
        // batch once on the preserved attempt nonce. A receipt is not context.
        let receipt = super::super::readiness::prepare_readiness(
            &app,
            "review-context-root",
            "first-attempt",
        )
        .await
        .unwrap()
        .expect("first preflight must park");
        super::super::readiness::wait_for_readiness(
            &app,
            "review-context-root",
            &receipt,
            "first-attempt",
        )
        .await
        .unwrap();
        assert!(super::super::readiness::prepare_readiness(
            &app,
            "review-context-root",
            "first-attempt"
        )
        .await
        .unwrap()
        .is_none());
        assert!(super::super::readiness::prepare_readiness(
            &app,
            "review-context-root",
            "first-attempt"
        )
        .await
        .unwrap_err()
        .contains("already consumed"));
        verify_binding_for_chat(&app, "review-context-root")
            .await
            .unwrap();
        let receipt = super::super::readiness::prepare_readiness(
            &app,
            "review-context-root",
            "drift-attempt",
        )
        .await
        .unwrap()
        .unwrap();
        super::super::readiness::wait_for_readiness(
            &app,
            "review-context-root",
            &receipt,
            "drift-attempt",
        )
        .await
        .unwrap();
        let saved_settings = app.gcx.privacy_settings.read().unwrap().clone();
        let mut blocked = saved_settings.privacy_rules.blocked.clone();
        blocked.push(source.join("tracked").to_string_lossy().into_owned());
        *app.gcx.privacy_settings.write().unwrap() =
            Arc::new(refact_core::privacy_types::PrivacySettings {
                loaded_ts: saved_settings.loaded_ts,
                privacy_rules: refact_core::privacy_types::FilePrivacySettings {
                    blocked,
                    only_send_to_servers_I_control: saved_settings
                        .privacy_rules
                        .only_send_to_servers_I_control
                        .clone(),
                },
            });
        // A completed old-policy batch is discarded and rebuilt, not admitted.
        let replacement = super::super::readiness::prepare_readiness(
            &app,
            "review-context-root",
            "drift-attempt",
        )
        .await
        .unwrap()
        .expect("policy drift must redo preflight");
        assert_ne!(replacement.job_id, receipt.job_id);
        assert!(super::super::readiness::wait_for_readiness(
            &app,
            "review-context-root",
            &replacement,
            "drift-attempt"
        )
        .await
        .is_err());
        *app.gcx.privacy_settings.write().unwrap() = saved_settings;
        std::fs::write(
            first.worktree.root.join("tracked"),
            "old investigation edit\n",
        )
        .unwrap();
        // Simulate restart loading the old trajectory after durable candidate activation.
        session.lock().await.thread.worktree = None;
        ccx.lock().await.execution_scope = None;
        verify_binding_for_chat(&app, "review-context-root")
            .await
            .unwrap();
        assert_eq!(
            session.lock().await.thread.worktree.as_ref(),
            Some(&first.worktree)
        );
        ccx.lock().await.execution_scope = Some(ExecutionScope::from_worktree(&first.worktree));
        let recovered = set_context(ccx.clone(), request).await.unwrap();
        assert_eq!(recovered.worktree, first.worktree);
        let identical = SetContextRequest {
            repository: source.to_string_lossy().into(),
            base_ref: oid.to_string(),
            candidate_ref: oid.to_string(),
            policy: CapturePolicy::default(),
            title: "Review".into(),
            target_reference: "local".into(),
            review_id: Some(recovered.review.id.clone()),
            expected_revision: Some(recovered.review.revision),
            request_key: "identical".into(),
        };
        let same = set_context(ccx.clone(), identical).await.unwrap();
        assert_eq!(same.worktree, first.worktree);
        assert_eq!(same.review.candidates.len(), 1);
        assert_eq!(same.review.revision, recovered.review.revision);
        tree.insert("tracked", repo.blob(b"updated\n").unwrap(), 0o100644)
            .unwrap();
        let updated_oid = repo
            .commit(
                None,
                &signature,
                &signature,
                "update",
                &repo.find_tree(tree.write().unwrap()).unwrap(),
                &[&repo.find_commit(oid).unwrap()],
            )
            .unwrap();
        let second_request = SetContextRequest {
            repository: source.to_string_lossy().into(),
            base_ref: oid.to_string(),
            candidate_ref: updated_oid.to_string(),
            policy: CapturePolicy::default(),
            title: "Review".into(),
            target_reference: "local".into(),
            review_id: Some(recovered.review.id.clone()),
            expected_revision: Some(recovered.review.revision),
            request_key: "update".into(),
        };
        let second = set_context(ccx.clone(), second_request).await.unwrap();
        assert_ne!(first.worktree.id, second.worktree.id);
        assert_eq!(
            std::fs::read(first.worktree.root.join("tracked")).unwrap(),
            b"old investigation edit\n"
        );
        assert_eq!(
            std::fs::read(second.worktree.root.join("tracked")).unwrap(),
            b"updated\n"
        );
        assert_eq!(second.review.candidates.len(), 2);
        assert!(
            second.review.privacy.contains(&source_record),
            "refresh must not erase earlier source provenance"
        );
        verify_binding_for_chat(&app, "review-context-root")
            .await
            .unwrap();
        let member_id = uuid::Uuid::new_v4().to_string();
        repo.reference("refs/heads/stack-member", updated_oid, true, "fixture")
            .unwrap();
        let store = ReviewStore::open(&source).unwrap();
        let stack = StackManifest {
            schema_version: SCHEMA_VERSION,
            id: "context-stack".into(),
            owner_chat_id: "review-context-root".into(),
            revision: 0,
            title: "Context stack".into(),
            repository: source.to_str().unwrap().into(),
            source: StackSource {
                kind: "provided_manifest".into(),
                reference: Some("fixture".into()),
            },
            root_base_ref: oid.to_string(),
            root_base_oid: Some(oid.to_string()),
            scope_choice: StackScopeChoice {
                scope: StackScope::Whole,
                selected_member_id: member_id.clone(),
                confirmed: true,
                explicit_request: true,
            },
            members: vec![StackMember {
                id: member_id.clone(),
                branch_ref: "refs/heads/stack-member".into(),
                pr_identity: None,
                parent_member_id: None,
                declared_base_ref: oid.to_string(),
                resolved_base_oid: Some(oid.to_string()),
                head_oid: Some(updated_oid.to_string()),
                review_id: None,
                state: StackMemberState::Ready,
                reason: None,
            }],
        };
        store.create_stack(stack, "context-stack").unwrap();
        store
            .create_member_review(
                "context-stack",
                &member_id,
                0,
                "Member",
                Origin::Imported,
                "member",
            )
            .unwrap();
        let selected =
            super::super::stacks::capture(ccx.clone(), member_id.clone(), "select-stack".into())
                .await
                .unwrap();
        std::fs::write(
            selected.worktree.root.join("tracked"),
            "retained old investigation edit\n",
        )
        .unwrap();
        tree.insert("tracked", repo.blob(b"refreshed\n").unwrap(), 0o100644)
            .unwrap();
        let refreshed_oid = repo
            .commit(
                None,
                &signature,
                &signature,
                "refreshed",
                &repo.find_tree(tree.write().unwrap()).unwrap(),
                &[&repo.find_commit(updated_oid).unwrap()],
            )
            .unwrap();
        repo.reference(
            "refs/heads/stack-member",
            refreshed_oid,
            true,
            "refresh fixture",
        )
        .unwrap();
        let before_refresh = store.load_stack("context-stack").unwrap();
        let refreshed = super::super::stacks::refresh(
            &app,
            "review-context-root",
            "refresh-context".into(),
            before_refresh.revision,
        )
        .await
        .unwrap();
        assert!(verify_binding_for_chat(&app, "review-context-root")
            .await
            .is_err());
        let recaptured = super::super::stacks::capture(ccx, member_id, "recapture-stack".into())
            .await
            .unwrap();
        assert_ne!(recaptured.worktree.id, selected.worktree.id);
        assert_eq!(
            std::fs::read(selected.worktree.root.join("tracked")).unwrap(),
            b"retained old investigation edit\n"
        );
        assert_eq!(
            std::fs::read(recaptured.worktree.root.join("tracked")).unwrap(),
            b"refreshed\n"
        );
        assert_eq!(
            recaptured.review.candidates.last().unwrap().head_oid,
            refreshed.members[0].head_oid.clone().unwrap()
        );
        verify_binding_for_chat(&app, "review-context-root")
            .await
            .unwrap();
        let root = super::super::service::trusted_root(&app, "review-context-root")
            .await
            .unwrap();
        let bound = resolve_binding(&app, "review-context-root")
            .await
            .unwrap()
            .unwrap();
        let active =
            super::super::service::get_for_chat(root.clone(), "review-context-root".into())
                .await
                .unwrap()
                .unwrap();
        let renamed = super::super::service::user_mutate(
            root,
            "review-context-root".into(),
            active.id.clone(),
            Mutation {
                expected_revision: active.revision,
                request_key: "rename-while-bound".into(),
                actor: Actor {
                    kind: ActorKind::User,
                    id: "user".into(),
                },
                at_ms: 0,
                operation: Operation::Rename {
                    title: "Renamed while bound".into(),
                },
            },
        )
        .await
        .unwrap();
        assert!(renamed.revision > active.revision);
        let rebound = resolve_binding(&app, "review-context-root")
            .await
            .unwrap()
            .unwrap();
        assert!(
            bound.same_identity(&rebound),
            "an ordinary review edit must not invalidate the checkout binding"
        );
    }
}
