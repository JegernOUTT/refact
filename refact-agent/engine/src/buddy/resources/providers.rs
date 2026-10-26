use std::sync::Arc;
use async_trait::async_trait;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::providers::config_store;

#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderDraft {
    pub project: ProjectIdentity,
    pub instance_id: String,
    pub model_id: String,
}

#[derive(Serialize, Deserialize)]
struct Preimage {
    project: ProjectIdentity,
    instance_id: String,
    post_version: String,
    config: serde_yaml::Value,
}

pub struct ProvidersAdapter {
    app: AppState,
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<ProviderDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

fn invalid() -> ResourceError {
    ResourceError::ValidationFailed("provider model not available for this action".into())
}
fn encode(value: &serde_yaml::Value) -> Result<Vec<u8>, ResourceError> {
    serde_yaml::to_string(value)
        .map(String::into_bytes)
        .map_err(|_| invalid())
}
fn config_version(value: &serde_yaml::Value) -> Result<String, ResourceError> {
    Ok(super::settings::version(&Some(encode(value)?)))
}
fn enable_model(
    mut config: serde_yaml::Value,
    model: &str,
) -> Result<serde_yaml::Value, ResourceError> {
    let map = config.as_mapping_mut().ok_or_else(invalid)?;
    let enabled_key = serde_yaml::Value::String("enabled_models".into());
    let enabled = map
        .entry(enabled_key)
        .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(invalid)?;
    let model_value = serde_yaml::Value::String(model.into());
    if !enabled.contains(&model_value) {
        enabled.push(model_value.clone());
    }
    if let Some(disabled) = map.get_mut(&serde_yaml::Value::String("disabled_models".into())) {
        disabled
            .as_sequence_mut()
            .ok_or_else(invalid)?
            .retain(|value| value != &model_value);
    }
    Ok(config)
}

impl ProvidersAdapter {
    pub fn new(
        app: AppState,
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<ProviderDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
            app,
            project,
            drafts,
            materials,
        }
    }
    fn check(&self, spec: &ActionSpec) -> Result<(), ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "providers"
            || spec.kind != "provider_enable_custom_model"
            || spec.resource.resource_kind != "provider"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        crate::providers::identity::validate_provider_instance_id(&spec.resource.resource_id)
            .map_err(|_| ResourceError::InvalidResourceId)?;
        Ok(())
    }
    async fn snapshot(&self, instance: &str) -> Result<serde_yaml::Value, ResourceError> {
        let path = config_store::provider_config_path(&self.app.paths.config_dir, instance);
        for directory in [
            self.app.paths.config_dir.clone(),
            self.app.paths.config_dir.join("providers.d"),
        ] {
            if tokio::fs::symlink_metadata(directory)
                .await?
                .file_type()
                .is_symlink()
            {
                return Err(ResourceError::SymlinkNotAllowed);
            }
        }
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if metadata.file_type().is_symlink() {
            return Err(ResourceError::SymlinkNotAllowed);
        }
        let bytes = tokio::fs::read(path).await?;
        serde_yaml::from_slice(&bytes).map_err(|_| invalid())
    }
    async fn prepare(
        &self,
        spec: &ActionSpec,
    ) -> Result<(serde_yaml::Value, serde_yaml::Value), ResourceError> {
        self.check(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project || draft.instance_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = self.snapshot(&draft.instance_id).await?;
        if config_version(&before)? != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        let registry = self.app.gcx.providers.read().await;
        let provider = registry
            .get(&draft.instance_id)
            .ok_or(ResourceError::ResourceMissing)?;
        if provider.is_readonly() {
            return Err(ResourceError::UnsupportedResource);
        }
        let mut candidate = provider.clone_box();
        candidate
            .provider_settings_apply(before.clone())
            .map_err(|_| invalid())?;
        if !candidate
            .get_custom_models_only()
            .iter()
            .any(|model| model.id == draft.model_id && model.n_ctx > 0)
        {
            return Err(invalid());
        }
        let after = enable_model(before.clone(), &draft.model_id)?;
        candidate
            .provider_settings_apply(after.clone())
            .map_err(|_| invalid())?;
        if !candidate.enabled_models().contains(&draft.model_id) {
            return Err(invalid());
        }
        Ok((before, after))
    }
}

#[async_trait]
impl AsyncResourceAdapter for ProvidersAdapter {
    fn adapter_id(&self) -> &str {
        "providers"
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
        let (_, after) = self.prepare(spec).await?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(),
            proposed_version: config_version(&after)?, reload_needed: true,
            diff: Some("Enable one existing custom catalog model; preserve credentials, unknown configuration and all current selections. Provider reload required; no default model switch or network probe.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let (before, after) = self.prepare(spec).await?;
        let post = config_version(&after)?;
        let preimage = Preimage {
            project: self.project.clone(),
            instance_id: spec.resource.resource_id.clone(),
            post_version: post.clone(),
            config: before.clone(),
        };
        let reference = self
            .materials
            .put(&serde_json::to_vec(&preimage).map_err(|_| ResourceError::MaterialUnavailable)?)?;
        config_store::update_provider_config_with(
            &self.app.paths.config_dir,
            &spec.resource.resource_id,
            |_| ResourceError::Io("provider persistence failed".into()),
            |existing| {
                if existing.as_ref() != Some(&before) {
                    return Err(ResourceError::VersionMismatch);
                }
                Ok(after)
            },
        )
        .await?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: post,
            state: ResourceApplyState::DiskDoneReloadNeeded,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.check(spec)?;
        Ok(config_version(&self.snapshot(&spec.resource.resource_id).await?)? == post)
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        self.check(spec)?;
        let material: Preimage =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
        if material.project != self.project
            || material.instance_id != spec.resource.resource_id
            || material.post_version != recovery.expected_post_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        let current = self.snapshot(&spec.resource.resource_id).await?;
        if config_version(&current)? != recovery.expected_post_version
            || config_version(&material.config)? != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        config_store::update_provider_config_with(
            &self.app.paths.config_dir,
            &spec.resource.resource_id,
            |_| ResourceError::Io("provider recovery failed".into()),
            |existing| {
                let existing = existing.ok_or(ResourceError::ResourceMissing)?;
                if config_version(&existing)? != recovery.expected_post_version {
                    return Err(ResourceError::VersionMismatch);
                }
                Ok(material.config)
            },
        )
        .await?;
        Ok(ResourceApplyState::DiskDoneReloadNeeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owning_store_cas_preserves_unknowns_and_rejects_concurrent_update() {
        let root = tempfile::tempdir().unwrap();
        let before: serde_yaml::Value =
            serde_yaml::from_str("enabled_models: [selected]\nfuture: {secret: private}\n")
                .unwrap();
        config_store::write_provider_config(root.path(), "custom", before.clone())
            .await
            .unwrap();
        let after = enable_model(before.clone(), "extra").unwrap();
        let result = config_store::update_provider_config_with(
            root.path(),
            "custom",
            |_| invalid(),
            |current| {
                assert_eq!(current.as_ref(), Some(&before));
                Ok(after.clone())
            },
        )
        .await
        .unwrap();
        assert_eq!(result["future"], before["future"]);
        let stale = config_store::update_provider_config_with(
            root.path(),
            "custom",
            |_| invalid(),
            |current| {
                if current.as_ref() != Some(&before) {
                    return Err(ResourceError::VersionMismatch);
                }
                Ok(before.clone())
            },
        )
        .await;
        assert_eq!(stale, Err(ResourceError::VersionMismatch));
        let actual: serde_yaml::Value = serde_yaml::from_slice(
            &tokio::fs::read(config_store::provider_config_path(root.path(), "custom"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(actual, after);
    }

    #[test]
    fn invalid_sequences_are_not_silently_replaced() {
        let before = serde_yaml::from_str("enabled_models: malformed").unwrap();
        assert!(enable_model(before, "custom").is_err());
    }
}
