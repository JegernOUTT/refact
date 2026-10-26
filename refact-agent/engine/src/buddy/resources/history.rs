use std::path::{Path, PathBuf};
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::memory::fingerprint;
use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::buddy::outcome_runtime::OutcomeRuntime;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryDraft {
    pub project: ProjectIdentity,
    pub trajectory_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ArchivePreimage {
    project: ProjectIdentity,
    trajectory_id: String,
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

pub struct HistoryAdapter {
    app: AppState,
    runtime: OutcomeRuntime,
    drafts: Arc<dyn DraftRegistry<HistoryDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
    gate: tokio::sync::Mutex<()>,
}

impl HistoryAdapter {
    pub fn new(
        app: AppState,
        runtime: OutcomeRuntime,
        drafts: Arc<dyn DraftRegistry<HistoryDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
            app,
            runtime,
            drafts,
            materials,
            gate: tokio::sync::Mutex::new(()),
        }
    }

    fn validate(&self, spec: &ActionSpec) -> Result<HistoryDraft, ResourceError> {
        if spec.project != self.runtime.project() {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != self.adapter_id()
            || spec.kind != "export_backup"
            || spec.resource.resource_kind != "trajectory"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        crate::chat::trajectories::validate_trajectory_id(&spec.resource.resource_id)
            .map_err(|_| ResourceError::InvalidResourceId)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project || draft.trajectory_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        Ok(draft)
    }

    async fn protected(&self, id: &str) -> Result<bool, ResourceError> {
        let id = id.to_owned();
        self.runtime
            .with_store(move |store| store.trajectory_is_referenced(&id))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))
    }

    async fn prepare(&self, spec: &ActionSpec) -> Result<ArchivePreimage, ResourceError> {
        let draft = self.validate(spec)?;
        if self
            .app
            .chat
            .sessions
            .read()
            .await
            .contains_key(&draft.trajectory_id)
            || self.protected(&draft.trajectory_id).await?
        {
            return Err(ResourceError::ValidationFailed(
                "trajectory is active or referenced by durable work evidence".into(),
            ));
        }
        let root = tokio::fs::canonicalize(&spec.project.canonical_root).await?;
        if root.to_string_lossy() != spec.project.canonical_root {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let path = crate::chat::trajectories::find_trajectory_path(
            self.app.gcx.clone(),
            &draft.trajectory_id,
        )
        .await
        .ok_or(ResourceError::ResourceMissing)?;
        safe_path(&root, &path).await?;
        if !path.starts_with(root.join(".refact/trajectories")) {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if tokio::fs::metadata(&path).await?.len() > 32 * 1024 * 1024 {
            return Err(ResourceError::ValidationFailed(
                "trajectory exceeds bounded archive size".into(),
            ));
        }
        let bytes = tokio::fs::read(&path).await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        if value.get("id").and_then(Value::as_str) != Some(draft.trajectory_id.as_str())
            || protected_metadata(&value)
        {
            return Err(ResourceError::ValidationFailed(
                "trajectory identity or protected metadata prevents archive".into(),
            ));
        }
        if fingerprint(&bytes)? != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        Ok(ArchivePreimage {
            project: spec.project.clone(),
            trajectory_id: draft.trajectory_id,
            relative_path: path
                .strip_prefix(&root)
                .map_err(|_| ResourceError::PathEscapesProject)?
                .to_path_buf(),
            bytes,
        })
    }
}

async fn safe_path(root: &Path, path: &Path) -> Result<(), ResourceError> {
    if !path.starts_with(root) {
        return Err(ResourceError::PathEscapesProject);
    }
    let mut current = root.to_path_buf();
    for component in path
        .strip_prefix(root)
        .map_err(|_| ResourceError::PathEscapesProject)?
        .components()
    {
        match component {
            std::path::Component::Normal(part) => current.push(part),
            _ => return Err(ResourceError::PathEscapesProject),
        }
        if tokio::fs::symlink_metadata(&current)
            .await?
            .file_type()
            .is_symlink()
        {
            return Err(ResourceError::SymlinkNotAllowed);
        }
    }
    Ok(())
}

fn protected_metadata(value: &Value) -> bool {
    crate::chat::trajectory_archive::protected_metadata(value)
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for HistoryAdapter {
    fn adapter_id(&self) -> &str {
        "history"
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
        let preimage = self.prepare(spec).await?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: crate::chat::trajectory_archive::digest(&preimage.bytes), reload_needed: false, diff: Some(format!("Export backup of {} ({} bytes); retain the original trajectory and exact evidence bytes. No archive removal, pruning, descendant removal, or legacy deletion.", preimage.relative_path.display(), preimage.bytes.len())) })
    }

    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        crate::chat::trajectory_archive::with_trajectory_mutation_lock(async {
            let _guard = self.gate.lock().await;
            let preimage = self.prepare(spec).await?;
            let root = Path::new(&spec.project.canonical_root);
            let (identity, current_bytes) = crate::chat::trajectory_archive::prepare(
                root,
                &preimage.relative_path,
                &preimage.trajectory_id,
            )
            .map_err(ResourceError::ValidationFailed)?;
            if current_bytes != preimage.bytes {
                return Err(ResourceError::VersionMismatch);
            }
            let previous_material_reference = self.materials.put(
                &serde_json::to_vec(&(spec.project.clone(), identity.clone()))
                    .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
            )?;
            let sessions = self.app.chat.sessions.read().await;
            let protected = sessions.contains_key(&identity.trajectory_id)
                || self.protected(&identity.trajectory_id).await?;
            crate::chat::trajectory_archive::archive(root, &identity, protected)
                .map_err(ResourceError::ValidationFailed)?;
            Ok(ResourceApplyReceipt {
                resource_id: spec.resource.resource_id.clone(),
                previous_material_reference,
                expected_post_version: identity.digest.clone(),
                state: ResourceApplyState::Applied,
            })
        })
        .await
    }

    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.validate(spec)?;
        // Verify the immutable archive identity rather than the mutable original.
        if post.len() != 64 || !post.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ResourceError::VersionMismatch);
        }
        let root = Path::new(&spec.project.canonical_root);
        let path = root
            .join(".refact/trajectory_archive")
            .join(format!("{}-{post}.json", spec.resource.resource_id));
        safe_path(root, &path).await?;
        let bytes = tokio::fs::read(path).await?;
        Ok(crate::chat::trajectory_archive::digest(&bytes) == post
            && fingerprint(&bytes)? == spec.expected_version)
    }

    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        self.validate(spec)?;
        let (project, identity): (
            ProjectIdentity,
            crate::chat::trajectory_archive::ArchiveIdentity,
        ) = serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
            .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        if project != spec.project
            || identity.trajectory_id != spec.resource.resource_id
            || identity.digest.clone() != recovery.expected_post_version
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        crate::chat::trajectory_archive::with_trajectory_mutation_lock(async {
            let sessions = self.app.chat.sessions.read().await;
            if sessions.contains_key(&identity.trajectory_id) {
                return Err(ResourceError::VersionMismatch);
            }
            crate::chat::trajectory_archive::recover(Path::new(&project.canonical_root), &identity)
                .map_err(ResourceError::ValidationFailed)?;
            Ok(ResourceApplyState::Applied)
        })
        .await
    }
}
