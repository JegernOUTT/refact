use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionExecutionState, ActionSpec, ProjectIdentity};

use crate::buddy::action_execution::{action_spec_hash, Authorizer, PreparedAction};
use crate::buddy::resource_execution::{ResourceDocument, ResourceExecutionContext};
use crate::buddy::operator_store::Stored;
use serde::{Deserialize, Serialize};

use super::{activation_gate, CandidateRecord, DeploymentTarget, validate_draft_binding};
use crate::buddy::improvement_evaluation::digest;
use crate::buddy::operator_store::{OperatorStore, OperatorStoreError};
use crate::buddy::outcome_runtime::OutcomeRuntime;
use crate::buddy::resources::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{CheckedMergeIdentity, MergeWorktreeRequest, WorktreeMergeStrategy};

pub const ADAPTER: &str = "candidate_landing";
pub const ACTION: &str = "land_reviewed_source";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LandingDraft {
    pub project: ProjectIdentity,
    pub candidate_id: String,
    pub proof_id: String,
    pub worktree_id: String,
    pub exact: CheckedMergeIdentity,
}

pub fn validate_action_binding(
    store: &OperatorStore,
    candidate: &CandidateRecord,
    action: &ActionSpec,
) -> Result<(), OperatorStoreError> {
    validate_draft_binding(store, candidate, action)?;
    match candidate.target {
        DeploymentTarget::Configuration => {
            if !matches!((action.adapter.as_str(), action.kind.as_str()),
                ("settings", "settings_trajectory_patch") | ("buddy_settings", "buddy_settings_patch") |
                ("default_models", "default_model_select") | ("providers", "provider_enable_custom_model") |
                ("hooks", "hook_project_apply") | ("integrations", "mcp_project_apply") |
                ("extensions", "markdown_project_apply") | ("project_customization_file", "customization_file_apply")) {
                return Err(OperatorStoreError::Conflict("configuration target requires a registered configuration action"));
            }
        }
        DeploymentTarget::SourceCandidate | DeploymentTarget::ReviewedCommit => {
            if action.adapter != ADAPTER || action.kind != ACTION {
                return Err(OperatorStoreError::Conflict("source activation requires native reviewed landing, not a checkpoint"));
            }
        }
        _ => return Err(OperatorStoreError::Conflict("artifact installation and host replacement require a release owner; no installed or running identity inferred")),
    }
    Ok(())
}

pub struct LandingAdapter {
    app: crate::app_state::AppState,
    runtime: OutcomeRuntime,
    worktrees: WorktreeService,
    drafts: Arc<dyn DraftRegistry<LandingDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

impl LandingAdapter {
    pub fn new(
        app: crate::app_state::AppState,
        runtime: OutcomeRuntime,
        worktrees: WorktreeService,
        drafts: Arc<dyn DraftRegistry<LandingDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        let worktrees = crate::worktrees::service::fenced_service(
            worktrees,
            app.runtime.tool_jobs.scopes.clone(),
        );
        Self {
            app,
            runtime,
            worktrees,
            drafts,
            materials,
        }
    }

    async fn bound(&self, spec: &ActionSpec) -> Result<LandingDraft, ResourceError> {
        if spec.project != self.runtime.project()
            || self.worktrees.source_workspace_root().to_string_lossy()
                != spec.project.canonical_root
        {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != ADAPTER
            || spec.kind != ACTION
            || spec.resource.resource_kind != "worktree"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project
            || draft.worktree_id != spec.resource.resource_id
            || digest(&draft.exact).map_err(store_error)? != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let checked = draft.clone();
        let action = spec.clone();
        let candidate = self
            .runtime
            .with_store(move |store| checked_candidate(store, &checked, &action))
            .await
            .map_err(store_error)?;
        let inspected = crate::buddy::improvement_runner::inspect_source(
            &self.worktrees,
            &draft.worktree_id,
            &candidate.target_files,
        )
        .await
        .map_err(store_error)?;
        if inspected.source_digest != candidate.source_identity
            || inspected.patch_digest != candidate.content_digest
        {
            return Err(store_error(
                "evaluated source snapshot changed before landing",
            ));
        }
        Ok(draft)
    }
    async fn guarded_service(
        &self,
        spec: &ActionSpec,
        draft: &LandingDraft,
        recovery: Option<&RecoveryDescriptor>,
    ) -> Result<
        (
            WorktreeService,
            Option<crate::coordination::leases::LeaseRenewal>,
        ),
        ResourceError,
    > {
        let context = crate::buddy::resource_execution::current_resource_execution()?;
        if !context.runtime.same_owner(&self.runtime)
            || context.recovery.is_some() != recovery.is_some()
        {
            return Err(store_error("landing execution owner or operation changed"));
        }
        let authority = LandingAuthority::capture(self, spec, draft, context, recovery).await?;
        authority.validate().await.map_err(store_error)?;
        let target = Some(draft.exact.target_branch.as_str());
        let service = if recovery.is_some() {
            self.worktrees
                .fence_recovery_worktree(&draft.worktree_id, target)
                .await
        } else {
            self.worktrees
                .fence_merge_worktree(&draft.worktree_id, target)
                .await
        }
        .map_err(store_error)?;
        let service = service.acquire_merge_permit().await.map_err(store_error)?;
        let service = service
            .for_merge_target(&draft.exact.target_branch)
            .await
            .map_err(store_error)?;
        if service.merge_checkout_root() != self.worktrees.source_workspace_root() {
            return Err(store_error(
                "reviewed landing requires its exact canonical target checkout",
            ));
        }
        let runtime = tokio::runtime::Handle::current();
        let service =
            service.with_merge_authority(Arc::new(move || runtime.block_on(authority.validate())));
        let leases = crate::coordination::leases::merge_leases_for(
            &self.app,
            self.worktrees.source_workspace_root(),
        )
        .await
        .map_err(store_error)?;
        let (service, renewal) = if let Some(leases) = leases {
            let gate = crate::coordination::merge_gate::MergeGate::new(
                leases.clone(),
                &crate::file_activity::settings(),
            );
            let slot = gate
                .acquire(&crate::coordination::merge_gate::checkout_key(
                    service.merge_checkout_root(),
                ))
                .await
                .map_err(store_error)?;
            let service =
                service.with_merge_authority(leases.phase_validator(slot.holder().clone()));
            let renewal =
                crate::coordination::leases::LeaseRenewal::spawn(leases, slot.holder().clone());
            (service, Some(renewal))
        } else {
            (service, None)
        };
        Ok((service, renewal))
    }
}

#[derive(Clone)]
struct LandingAuthority {
    app: crate::app_state::AppState,
    context: ResourceExecutionContext,
    spec: ActionSpec,
    draft: LandingDraft,
    grant: Stored<ResourceDocument<crate::buddy::resource_execution::ExactActionGrant>>,
    source: crate::worktrees::types::WorktreeMeta,
    source_digest: String,
    recovery: Option<RecoveryDescriptor>,
}

impl LandingAuthority {
    async fn capture(
        adapter: &LandingAdapter,
        spec: &ActionSpec,
        draft: &LandingDraft,
        context: ResourceExecutionContext,
        recovery: Option<&RecoveryDescriptor>,
    ) -> Result<Self, ResourceError> {
        let decision = context
            .execution
            .value
            .decision
            .as_ref()
            .ok_or_else(|| store_error("persisted exact landing decision missing"))?;
        let id = decision.native.source_id.clone();
        let grant = context
            .runtime
            .with_store(move |store| {
                store
                    .get_document("action_grant", &id)?
                    .ok_or(OperatorStoreError::Conflict("landing grant missing"))
            })
            .await
            .map_err(store_error)?;
        let source = adapter
            .worktrees
            .get_worktree(&draft.worktree_id)
            .await
            .map_err(store_error)?
            .meta;
        let candidate_id = draft.candidate_id.clone();
        let source_digest = context
            .runtime
            .with_store(move |store| {
                Ok(super::candidate(store, &candidate_id)?
                    .value
                    .source_identity)
            })
            .await
            .map_err(store_error)?;
        Ok(Self {
            app: adapter.app.clone(),
            context,
            spec: spec.clone(),
            draft: draft.clone(),
            grant,
            source,
            source_digest,
            recovery: recovery.cloned(),
        })
    }

    async fn validate(&self) -> Result<(), String> {
        let owner = crate::buddy::operator_runtime::runtime(
            &self.app,
            crate::buddy::operator_runtime::OperatorCapability::Improvement,
        )
        .await?;
        if !owner.same_owner(&self.context.runtime)
            || owner.is_closed()
            || owner.project() != self.spec.project
            || owner.worker() != self.context.execution.value.worker
        {
            return Err("current Buddy landing owner changed".into());
        }
        let actions = crate::buddy::operator_runtime::runtime(
            &self.app,
            crate::buddy::operator_runtime::OperatorCapability::Actions,
        )
        .await?;
        if !actions.same_owner(&owner) {
            return Err("current Buddy action owner changed".into());
        }
        let binding = self.clone();
        owner
            .with_store(move |store| binding.validate_store(store))
            .await
            .map_err(|e| e.to_string())?;
        let service = WorktreeService::new_async(
            self.app.paths.cache_dir.clone(),
            self.spec.project.canonical_root.clone().into(),
        )
        .await?;
        let view = service.get_worktree(&self.draft.worktree_id).await?;
        if view.meta != self.source
            || view.status.dirty
            || view.status.conflicted
            || view.status.error.is_some()
            || view.status.head_commit.as_deref() != Some(self.draft.exact.source_head.as_str())
        {
            return Err("reviewed source registration or generation changed".into());
        }
        let inspected = crate::buddy::improvement_runner::inspect_source(
            &service,
            &self.draft.worktree_id,
            &self.draft.exact.target_files,
        )
        .await?;
        if inspected.patch_digest != self.draft.exact.patch_digest
            || inspected.source_digest != self.source_digest
        {
            return Err("reviewed source manifest changed".into());
        }
        let root = service.source_workspace_root().to_path_buf();
        let exact = self.draft.exact.clone();
        let recovery = self.recovery.clone();
        tokio::task::spawn_blocking(move || {
            let target = &exact.target_branch;
            crate::worktrees::git::validate_merge_checkout(&root, target)?;
            let repo = git2::Repository::open(&root).map_err(|e| e.to_string())?;
            if dunce::canonicalize(&root).map_err(|e| e.to_string())? != root
                || repo.head().map_err(|e| e.to_string())?.shorthand() != Some(target.as_str())
            {
                return Err("canonical reviewed landing target moved".into());
            }
            let current = repo
                .head()
                .and_then(|head| head.peel_to_commit())
                .map_err(|e| e.to_string())?;
            let good = repo
                .find_commit(git2::Oid::from_str(&exact.target_head).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let source = repo
                .find_commit(git2::Oid::from_str(&exact.source_head).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let valid = if let Some(recovery) = &recovery {
                current.id().to_string() == recovery.expected_post_version
                    || (current.parent_count() == 1
                        && current.parent_id(0).map_err(|e| e.to_string())?.to_string()
                            == recovery.expected_post_version
                        && current.tree_id() == good.tree_id())
            } else {
                current.id() == good.id()
                    || (current.parent_count() == 1
                        && current.parent_id(0).map_err(|e| e.to_string())? == good.id()
                        && current.tree_id() == source.tree_id())
            };
            if !valid {
                return Err("reviewed landing target generation changed".into());
            }
            let mut options = git2::StatusOptions::new();
            options.include_untracked(true).recurse_untracked_dirs(true);
            let status = repo
                .statuses(Some(&mut options))
                .map_err(|e| e.to_string())?;
            if !status.is_empty() {
                let staged_candidate = recovery.is_none()
                    && current.id() == good.id()
                    && status.iter().all(|entry| {
                        !entry.status().intersects(
                            git2::Status::WT_NEW
                                | git2::Status::WT_MODIFIED
                                | git2::Status::WT_DELETED
                                | git2::Status::WT_RENAMED
                                | git2::Status::WT_TYPECHANGE
                                | git2::Status::CONFLICTED,
                        )
                    })
                    && repo
                        .diff_tree_to_index(
                            Some(&source.tree().map_err(|e| e.to_string())?),
                            None,
                            None,
                        )
                        .map_err(|e| e.to_string())?
                        .deltas()
                        .len()
                        == 0;
                if !staged_candidate {
                    return Err(
                        "reviewed landing target has unrelated edits; retain all state".into(),
                    );
                }
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
        let current = crate::buddy::operator_runtime::runtime(
            &self.app,
            crate::buddy::operator_runtime::OperatorCapability::Improvement,
        )
        .await?;
        if !current.same_owner(&owner) {
            return Err("current Buddy landing owner changed during phase validation".into());
        }
        let binding = self.clone();
        current
            .with_store(move |store| binding.validate_store(store))
            .await
            .map_err(|e| e.to_string())
    }

    fn validate_store(&self, store: &OperatorStore) -> Result<(), OperatorStoreError> {
        let conflict = || {
            OperatorStoreError::Conflict("landing execution, grant or prepared resource changed")
        };
        let execution = &self.context.execution;
        let decision = execution.value.decision.as_ref().ok_or_else(conflict)?;
        let grant = store
            .get_document::<ResourceDocument<crate::buddy::resource_execution::ExactActionGrant>>(
                "action_grant",
                &decision.native.source_id,
            )?
            .ok_or_else(conflict)?;
        let hash = action_spec_hash(&self.spec, &self.spec.parameter_ref);
        let prepared = store
            .get_document::<ResourceDocument<PreparedAction>>("prepared_action", &hash)?
            .ok_or_else(conflict)?;
        let bound = store
            .get_document::<ResourceDocument<LandingDraft>>("prepared_draft", &hash)?
            .ok_or_else(conflict)?;
        if store.identity() != &self.spec.project
            || self.spec.adapter != ADAPTER
            || self.spec.kind != ACTION
            || self.spec.resource.resource_kind != "worktree"
            || self.spec.resource.resource_id != self.draft.worktree_id
            || self.spec.expected_version != digest(&self.draft.exact)?
            || self.spec.resource.version != self.spec.expected_version
            || self.draft.project != self.spec.project
            || execution.value.project != self.spec.project
            || execution.value.status != ActionExecutionState::Running
            || store.get_execution(&execution.value.execution_id)?.as_ref() != Some(execution)
            || decision.project != self.spec.project
            || decision.worker != execution.value.worker
            || grant.revision != self.grant.revision
            || grant.change_seq != self.grant.change_seq
            || digest(&grant.value)? != digest(&self.grant.value)?
            || grant.value.project != self.spec.project
            || !grant.value.payload.authorize(
                &self.spec,
                &execution.value.requester,
                Some(decision),
            )
            || prepared.value.project != self.spec.project
            || prepared.value.payload.spec != self.spec
            || prepared.value.payload.spec_hash != hash
            || prepared.value.payload.bound_parameter_ref != self.spec.parameter_ref
            || bound.value.project != self.spec.project
            || bound.value.payload != self.draft
        {
            return Err(conflict());
        }
        if let Some(request) = &self.context.recovery {
            let original = store
                .get_execution(&request.original_execution_id)?
                .ok_or_else(conflict)?;
            let reference =
                crate::buddy::action_recovery::verified_recovery(&original.value, request)
                    .map_err(|_| conflict())?;
            let mut expected =
                crate::buddy::action_recovery::recovery_execution(request, &original.value);
            expected.status = ActionExecutionState::Running;
            expected.started_at = execution.value.started_at.clone();
            let receipt_id = if reference.recovery_id == original.value.execution_id {
                original.value.execution_id.clone()
            } else if store.is_legacy_resource_execution(&original.value.execution_id)? {
                original.value.spec_hash.clone()
            } else {
                return Err(conflict());
            };
            let receipt = store
                .get_document::<ResourceDocument<ResourceApplyReceipt>>(
                    "resource_receipt",
                    &receipt_id,
                )?
                .ok_or_else(conflict)?;
            let recovery = self.recovery.as_ref().ok_or_else(conflict)?;
            if grant.value.payload.recovery_execution_id.as_deref()
                != Some(request.original_execution_id.as_str())
                || request.original_spec != self.spec
                || request.original_bound_parameter_ref != self.spec.parameter_ref
                || original.value.spec_hash != hash
                || execution.value != expected
                || receipt.value.project != self.spec.project
                || receipt.value.payload.resource_id != self.draft.worktree_id
                || receipt.value.payload.expected_post_version != recovery.expected_post_version
                || reference.expected_post_version != recovery.expected_post_version
                || receipt.value.payload.previous_material_reference
                    != recovery.previous_material_reference
            {
                return Err(conflict());
            }
        } else {
            if grant.value.payload.recovery_execution_id.is_some()
                || execution.value.action_id != self.spec.action_id
                || execution.value.spec_hash != hash
            {
                return Err(conflict());
            }
            let current = store
                .get_document::<ResourceDocument<LandingDraft>>(
                    "resource_draft",
                    self.spec.parameter_ref.as_deref().ok_or_else(conflict)?,
                )?
                .ok_or_else(conflict)?;
            if current.value.payload != self.draft || current.value.project != self.spec.project {
                return Err(conflict());
            }
            checked_candidate(store, &self.draft, &self.spec)?;
            crate::buddy::desk_actions::validate_resource_origin(store, &hash)?;
        }
        Ok(())
    }
}

fn checked_candidate(
    store: &OperatorStore,
    checked: &LandingDraft,
    action: &ActionSpec,
) -> Result<CandidateRecord, OperatorStoreError> {
    let candidate = super::candidate(store, &checked.candidate_id)?.value;
    let candidate = activation_gate(
        store,
        &checked.candidate_id,
        &digest(&candidate)?,
        &checked.proof_id,
    )?;
    validate_action_binding(store, &candidate, action)?;
    if candidate.activation_action.as_ref() != Some(action)
        || candidate.candidate.worktree_ref.as_deref() != Some(checked.worktree_id.as_str())
        || candidate.commit_identity.as_deref() != Some(checked.exact.source_head.as_str())
        || candidate.content_digest != checked.exact.patch_digest
        || candidate.target_files != checked.exact.target_files
        || candidate.candidate.previous_known_good_identity.as_deref()
            != Some(checked.exact.target_head.as_str())
    {
        return Err(OperatorStoreError::Conflict(
            "landing draft differs from evaluated candidate or known-good target",
        ));
    }
    Ok(candidate)
}

fn store_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::ValidationFailed(error.to_string())
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for LandingAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER
    }
    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: true,
        }
    }
    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let draft = self.bound(spec).await?;
        self.worktrees
            .validate_checked_merge(&draft.worktree_id, &draft.exact)
            .await
            .map_err(store_error)?;
        Ok(ResourcePreview {
            resource_id: draft.worktree_id.clone(),
            expected_version: spec.expected_version.clone(),
            proposed_version: draft.exact.source_head,
            reload_needed: false,
            diff: Some(
                self.worktrees
                    .diff_worktree(&draft.worktree_id)
                    .await
                    .map_err(store_error)?
                    .patch,
            ),
        })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let refused = |error: ResourceError| {
            ResourceError::NotApplied(format!("{error:?}").chars().take(512).collect())
        };
        let draft = self.bound(spec).await.map_err(refused)?;
        let context =
            crate::buddy::resource_execution::current_resource_execution().map_err(refused)?;
        let (service, mut renewal) = self
            .guarded_service(spec, &draft, None)
            .await
            .map_err(refused)?;
        let known = Arc::new(std::sync::Mutex::new(None::<ResourceApplyReceipt>));
        let result: Result<_, ResourceError> = async {
            service
                .checkpoint_merge_phase()
                .await
                .map_err(|error| ResourceError::NotApplied(error.chars().take(512).collect()))?;
            let previous_material_reference = self
                .materials
                .put(&serde_json::to_vec(&draft).map_err(store_error)?)
                .map_err(refused)?;
            let observed = known.clone();
            let binding = draft.clone();
            let action = spec.clone();
            let handle = tokio::runtime::Handle::current();
            let merged = service
                .merge_worktree_checked_observed(
                    &draft.worktree_id,
                    MergeWorktreeRequest {
                        strategy: WorktreeMergeStrategy::Squash,
                        delete_after_merge: false,
                        include_uncommitted: false,
                        target_branch: Some(draft.exact.target_branch.clone()),
                        commit_message: Some(format!(
                            "Land evaluated candidate {}",
                            draft.candidate_id
                        )),
                        generate_commit_message: false,
                        acknowledge_dirty_baseline: false,
                    },
                    draft.exact.clone(),
                    Arc::new(move |landed| {
                        if landed.exact != binding.exact {
                            return Err("landing commit observation binding changed".into());
                        }
                        let receipt = ResourceApplyReceipt {
                            resource_id: binding.worktree_id.clone(),
                            previous_material_reference: previous_material_reference.clone(),
                            expected_post_version: landed.commit,
                            state: ResourceApplyState::Applied,
                        };
                        *observed.lock().map_err(|_| "landing observation lock poisoned")? = Some(receipt.clone());
                        handle.block_on(context.publish_receipt(&action, &receipt)).map_err(|e| format!("known landing receipt publication failed; inspect without replay: {e}"))
                    }),
                )
                .await
                .map_err(store_error)?;
            if !merged.merged || merged.conflict.is_some() {
                return Err(store_error(format!("Native landing did not apply: {} {:?}", merged.status, merged.conflict)));
            }
            service
                .checkpoint_merge_phase()
                .await
                .map_err(store_error)?;
            service.cleanup_merge_target().await.map_err(store_error)?;
            Ok(())
        }
        .await;
        let released = match &mut renewal {
            Some(renewal) => match renewal.shutdown().await {
                Ok(true) => Ok(()),
                Ok(false) => Err(store_error("landing lease was not held at release")),
                Err(error) => Err(store_error(error)),
            },
            None => Ok(()),
        };
        let receipt = known
            .lock()
            .map_err(|_| store_error("landing observation lock poisoned"))?
            .clone();
        let error = match (result, released) {
            (Ok(()), Ok(())) => {
                return receipt.ok_or_else(|| {
                    store_error("native landing commit unavailable; inspect without replay")
                })
            }
            (Err(error), Ok(())) | (Ok(()), Err(error)) => error,
            (Err(error), Err(release)) => store_error(format!("{error:?}; release: {release:?}")),
        };
        match receipt {
            Some(receipt) => Err(ResourceError::KnownPartial {
                receipt,
                reason: format!("{error:?}").chars().take(512).collect(),
            }),
            None => Err(error),
        }
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        if spec.project != self.runtime.project() || spec.adapter != ADAPTER || spec.kind != ACTION
        {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project
            || draft.worktree_id != spec.resource.resource_id
            || digest(&draft.exact).map_err(store_error)? != spec.expected_version
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let root = self.worktrees.source_workspace_root().to_path_buf();
        let expected = post.to_string();
        let restoring = post == spec.expected_version;
        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(root).map_err(store_error)?;
            let head = repo.head().map_err(store_error)?;
            let mut options = git2::StatusOptions::new();
            options
                .include_ignored(false)
                .include_untracked(true)
                .recurse_untracked_dirs(true);
            if head.shorthand() != Some(draft.exact.target_branch.as_str())
                || !repo
                    .statuses(Some(&mut options))
                    .map_err(store_error)?
                    .is_empty()
            {
                return Ok(false);
            }
            let current = head.peel_to_commit().map_err(store_error)?;
            let good = repo
                .find_commit(git2::Oid::from_str(&draft.exact.target_head).map_err(store_error)?)
                .map_err(store_error)?;
            let source = repo
                .find_commit(git2::Oid::from_str(&draft.exact.source_head).map_err(store_error)?)
                .map_err(store_error)?;
            if restoring {
                Ok(current.tree_id() == good.tree_id()
                    && current.parent_count() == 1
                    && current.parent(0).map_err(store_error)?.tree_id() == source.tree_id())
            } else {
                Ok(current.id().to_string() == expected
                    && current.tree_id() == source.tree_id()
                    && current.parent_count() == 1
                    && current.parent_id(0).map_err(store_error)? == good.id())
            }
        })
        .await
        .map_err(store_error)?
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        if spec.project != self.runtime.project() || spec.adapter != ADAPTER || spec.kind != ACTION
        {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let draft: LandingDraft =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(store_error)?;
        if draft.project != spec.project
            || draft.worktree_id != spec.resource.resource_id
            || digest(&draft.exact).map_err(store_error)? != spec.expected_version
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let (service, mut renewal) = self.guarded_service(spec, &draft, Some(recovery)).await?;
        let result = service
            .recover_checked_landing(
                &draft.worktree_id,
                draft.exact,
                recovery.expected_post_version.clone(),
            )
            .await
            .map_err(store_error);
        if let Some(renewal) = &mut renewal {
            if !renewal.shutdown().await.map_err(store_error)? {
                return Err(store_error(
                    "landing recovery lease was not held at release",
                ));
            }
        }
        result?;
        Ok(ResourceApplyState::Applied)
    }
}

#[cfg(test)]
#[path = "improvement_activation_tests.rs"]
mod tests;
