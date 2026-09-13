use std::sync::Arc;
use async_trait::async_trait;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::providers::operator_defaults::{self as owner, DefaultSlot, DefaultsScope};
use super::settings::version;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultModelDraft {
    pub project: ProjectIdentity,
    pub scope: DefaultsScope,
    pub slot: DefaultSlot,
    pub expected_selected: Option<String>,
    pub selected_model: String,
}
#[derive(Serialize, Deserialize)]
struct Preimage {
    project: ProjectIdentity,
    resource_id: String,
    post_version: String,
    bytes: Option<Vec<u8>>,
}
pub struct DefaultModelsAdapter {
    app: AppState,
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<DefaultModelDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}
fn error(e: String) -> ResourceError {
    if e.contains("mismatch") {
        ResourceError::VersionMismatch
    } else {
        ResourceError::ValidationFailed(e)
    }
}
impl DefaultModelsAdapter {
    pub fn new(
        app: AppState,
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<DefaultModelDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
            app,
            project,
            drafts,
            materials,
        }
    }
    fn check(&self, spec: &ActionSpec) -> Result<DefaultsScope, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "default_models"
            || spec.kind != "default_model_select"
            || spec.resource.resource_kind != "default_models"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        match spec.resource.resource_id.as_str() {
            "global/default_models" => Ok(DefaultsScope::Global),
            "project/default_models" => Ok(DefaultsScope::Project),
            _ => Err(ResourceError::InvalidResourceId),
        }
    }
    async fn prepare(
        &self,
        spec: &ActionSpec,
    ) -> Result<(std::path::PathBuf, DefaultsScope, Option<Vec<u8>>, Vec<u8>), ResourceError> {
        let scope = self.check(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project || draft.scope != scope {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let root = owner::scope_root(&self.app, &self.project, scope)
            .await
            .map_err(error)?;
        let before = owner::defaults_snapshot(&root, scope)
            .await
            .map_err(error)?;
        if version(&before) != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        owner::validate_candidate(&self.app, &draft.selected_model)
            .await
            .map_err(error)?;
        let after = owner::patch(
            &before,
            draft.slot,
            &draft.expected_selected,
            &draft.selected_model,
        )
        .map_err(error)?;
        Ok((root, scope, before, after))
    }
}
#[async_trait]
impl AsyncResourceAdapter for DefaultModelsAdapter {
    fn adapter_id(&self) -> &str {
        "default_models"
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
        let (_, _, _, after) = self.prepare(spec).await?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: version(&Some(after)), reload_needed: true, diff: Some("Change only the explicitly selected default slot/model. Project overrides retain precedence; existing chat selections and all other defaults remain unchanged. Persist only; capability reload required. Authentication/quota unverified; no network probe.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let (root, scope, before, after) = self.prepare(spec).await?;
        let post = version(&Some(after.clone()));
        let material = Preimage {
            project: self.project.clone(),
            resource_id: spec.resource.resource_id.clone(),
            post_version: post.clone(),
            bytes: before.clone(),
        };
        let reference = self
            .materials
            .put(&serde_json::to_vec(&material).map_err(|_| ResourceError::MaterialUnavailable)?)?;
        owner::defaults_compare_exchange(&root, scope, &before, &Some(after))
            .await
            .map_err(error)?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: post,
            state: ResourceApplyState::DiskDoneReloadNeeded,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let scope = self.check(spec)?;
        let root = owner::scope_root(&self.app, &self.project, scope)
            .await
            .map_err(error)?;
        Ok(version(
            &owner::defaults_snapshot(&root, scope)
                .await
                .map_err(error)?,
        ) == post)
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let scope = self.check(spec)?;
        let m: Preimage =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
        if m.project != self.project
            || m.resource_id != spec.resource.resource_id
            || m.post_version != recovery.expected_post_version
            || version(&m.bytes) != spec.expected_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        let root = owner::scope_root(&self.app, &self.project, scope)
            .await
            .map_err(error)?;
        let current = owner::defaults_snapshot(&root, scope)
            .await
            .map_err(error)?;
        if version(&current) != recovery.expected_post_version {
            return Err(ResourceError::VersionMismatch);
        }
        owner::defaults_compare_exchange(&root, scope, &current, &m.bytes)
            .await
            .map_err(error)?;
        Ok(ResourceApplyState::DiskDoneReloadNeeded)
    }
}
