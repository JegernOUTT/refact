use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};

use super::memory::fingerprint;
use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::global_context::GlobalContext;
use crate::worktrees::service::WorktreeService;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCodeDraft {
    pub project: ProjectIdentity,
    pub worktree_id: String,
    pub chat_id: String,
}

pub struct ProjectCodeAdapter {
    gcx: Arc<GlobalContext>,
    project: ProjectIdentity,
    worktrees: WorktreeService,
    drafts: Arc<dyn DraftRegistry<ProjectCodeDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

impl ProjectCodeAdapter {
    pub fn new(
        gcx: Arc<GlobalContext>,
        project: ProjectIdentity,
        worktrees: WorktreeService,
        drafts: Arc<dyn DraftRegistry<ProjectCodeDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Result<Self, ResourceError> {
        if worktrees.source_workspace_root().to_string_lossy() != project.canonical_root {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        Ok(Self {
            gcx,
            project,
            worktrees,
            drafts,
            materials,
        })
    }

    fn validate(&self, spec: &ActionSpec) -> Result<ProjectCodeDraft, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != self.adapter_id()
            || spec.kind != "checkpoint_isolated_candidate"
            || spec.resource.resource_kind != "worktree"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project || draft.worktree_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        crate::chat::trajectories::validate_trajectory_id(&draft.chat_id)
            .map_err(|_| ResourceError::InvalidResourceId)?;
        Ok(draft)
    }

    pub async fn version(&self, worktree_id: &str) -> Result<String, ResourceError> {
        registered_candidate_version(&self.worktrees, worktree_id).await
    }
}

/// Read-only current-version lookup for an already registered isolated checkout. Callers must
/// establish chat ownership separately; this helper neither registers nor mutates a worktree.
pub async fn registered_candidate_version(
    worktrees: &WorktreeService,
    worktree_id: &str,
) -> Result<String, ResourceError> {
    worktrees
        .validate_registered_worktree_checkout(worktree_id)
        .await
        .map_err(ResourceError::ValidationFailed)?;
    let view = worktrees
        .get_worktree(worktree_id)
        .await
        .map_err(ResourceError::Io)?;
    if view.meta.source_workspace_root != worktrees.source_workspace_root()
        || view.meta.root == view.meta.source_workspace_root
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.conflicted
        || view.status.error.is_some()
    {
        return Err(ResourceError::ValidationFailed(
            "candidate is not a healthy isolated checkout".into(),
        ));
    }
    let diff = worktrees
        .diff_worktree(worktree_id)
        .await
        .map_err(ResourceError::Io)?;
    if diff.patch_truncated {
        return Err(ResourceError::ValidationFailed(
            "candidate diff is truncated".into(),
        ));
    }
    let mut contents = std::collections::BTreeMap::new();
    for file in &diff.files {
        let relative = std::path::Path::new(&file.path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(ResourceError::PathEscapesProject);
        }
        let path = view.meta.root.join(relative);
        let bytes = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() || tokio::fs::canonicalize(&path).await? != path {
                    return Err(ResourceError::SymlinkNotAllowed);
                }
                Some(tokio::fs::read(&path).await?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        contents.insert(file.path.clone(), bytes);
    }
    fingerprint(&(view.meta, view.status, diff, contents))
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for ProjectCodeAdapter {
    fn adapter_id(&self) -> &str {
        "project_code"
    }
    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: false,
        }
    }

    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let draft = self.validate(spec)?;
        let version = self.version(&draft.worktree_id).await?;
        if version != spec.expected_version || spec.resource.version != version {
            return Err(ResourceError::VersionMismatch);
        }
        Ok(ResourcePreview {
            resource_id: draft.worktree_id.clone(),
            expected_version: version.clone(),
            proposed_version: version,
            reload_needed: false,
            diff: Some(
                serde_json::to_string(
                    &self
                        .worktrees
                        .diff_worktree(&draft.worktree_id)
                        .await
                        .map_err(ResourceError::Io)?,
                )
                .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
            ),
        })
    }

    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let _guard = crate::worktrees::service::worktree_merge_lock()
            .lock()
            .await;
        self.preview(spec).await?;
        let draft = self.validate(spec)?;
        let view = self
            .worktrees
            .get_worktree(&draft.worktree_id)
            .await
            .map_err(ResourceError::Io)?;
        if !view
            .references
            .iter()
            .any(|reference| reference.chat_id.as_deref() == Some(draft.chat_id.as_str()))
        {
            return Err(ResourceError::ValidationFailed(
                "chat does not own isolated candidate".into(),
            ));
        }
        let (checkpoint, repository) =
            crate::git::checkpoints::create_workspace_checkpoint_for_root(
                self.gcx.clone(),
                &view.meta.root,
                None,
                &draft.chat_id,
            )
            .await
            .map_err(ResourceError::Io)?;
        drop(repository);
        let post = self.version(&spec.resource.resource_id).await?;
        // A checkpoint is not test or review proof and never authorizes landing.
        // Keep the exact prepared/current binding even if an uncoordinated editor raced.
        let record = serde_json::json!({
            "schema_version": 1, "project": self.project, "draft": draft,
            "checkpoint": checkpoint, "prepared_patch_version": spec.expected_version,
            "observed_patch_version": post, "patch_unchanged": post == spec.expected_version,
            "test_status": "not_run", "review_status": "not_run", "landing_status": "not_authorized",
            "auto_merge": false
        });
        let previous_material_reference = self.materials.put(
            &serde_json::to_vec(&record)
                .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
        )?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference,
            expected_post_version: post.clone(),
            state: if post == spec.expected_version {
                ResourceApplyState::Applied
            } else {
                ResourceApplyState::NotApplied
            },
        })
    }

    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let draft = self.validate(spec)?;
        Ok(self.version(&draft.worktree_id).await? == post)
    }

    async fn recover(
        &self,
        _: &ActionSpec,
        _: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        Err(ResourceError::UnsupportedResource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_draft_is_typed_and_does_not_accept_commands() {
        let value = serde_json::json!({"project":{"project_id":"p","canonical_root":"/p"},"worktree_id":"w","chat_id":"c"});
        let draft: ProjectCodeDraft = serde_json::from_value(value).unwrap();
        assert_eq!(draft.worktree_id, "w");
        assert_eq!(draft.chat_id, "c");
        assert!(serde_json::from_value::<ProjectCodeDraft>(serde_json::json!({"project":{"project_id":"p","canonical_root":"/p"},"worktree_id":"w","chat_id":"c","command":"publish"})).is_err());
    }
}
