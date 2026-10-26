use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
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
    runtime: OutcomeRuntime,
    worktrees: WorktreeService,
    drafts: Arc<dyn DraftRegistry<LandingDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

impl LandingAdapter {
    pub fn new(
        runtime: OutcomeRuntime,
        worktrees: WorktreeService,
        drafts: Arc<dyn DraftRegistry<LandingDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
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
            .with_store(move |store| {
                let candidate = super::candidate(store, &checked.candidate_id)?.value;
                let candidate = activation_gate(
                    store,
                    &checked.candidate_id,
                    &digest(&candidate)?,
                    &checked.proof_id,
                )?;
                validate_action_binding(store, &candidate, &action)?;
                if candidate.activation_action.as_ref() != Some(&action)
                    || candidate.candidate.worktree_ref.as_deref()
                        != Some(checked.worktree_id.as_str())
                    || candidate.commit_identity.as_deref()
                        != Some(checked.exact.source_head.as_str())
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
            })
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
        let draft = self.bound(spec).await?;
        let previous_material_reference = self
            .materials
            .put(&serde_json::to_vec(&draft).map_err(store_error)?)?;
        let merged = self
            .worktrees
            .merge_worktree_checked(
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
                    // Landing an evaluated candidate must refuse a dirty
                    // target checkout rather than absorb unrelated work.
                    acknowledge_dirty_baseline: false,
                },
                draft.exact,
            )
            .await
            .map_err(store_error)?;
        if !merged.merged || merged.conflict.is_some() {
            return Err(ResourceError::ValidationFailed(format!(
                "Native landing did not apply: {} {:?}",
                merged.status, merged.conflict
            )));
        }
        Ok(ResourceApplyReceipt {
            resource_id: draft.worktree_id,
            previous_material_reference,
            expected_post_version: merged.merge_commit.ok_or_else(|| {
                store_error("native landing head unavailable; inspect before any further effect")
            })?,
            state: ResourceApplyState::Applied,
        })
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
            if head.shorthand() != Some(draft.exact.target_branch.as_str())
                || !repo.statuses(None).map_err(store_error)?.is_empty()
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
        self.worktrees
            .recover_checked_landing(
                &draft.worktree_id,
                draft.exact,
                recovery.expected_post_version.clone(),
            )
            .await
            .map_err(store_error)?;
        Ok(ResourceApplyState::Applied)
    }
}
