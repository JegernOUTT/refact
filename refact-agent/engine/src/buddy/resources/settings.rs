use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::http::routers::v1::settings_config::{
    buddy_trajectory_compare_exchange, buddy_trajectory_snapshot,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct SettingsDraft {
    pub project: ProjectIdentity,
    pub changes: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Serialize, Deserialize)]
struct SettingsPreimage {
    project: ProjectIdentity,
    resource_id: String,
    post_version: String,
    bytes: Option<Vec<u8>>,
}

pub struct SettingsAdapter {
    app: AppState,
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<SettingsDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

pub(crate) fn version(bytes: &Option<Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    hash.update(if bytes.is_some() {
        b"present".as_slice()
    } else {
        b"absent".as_slice()
    });
    if let Some(bytes) = bytes {
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

fn validation() -> ResourceError {
    ResourceError::ValidationFailed("invalid trajectory settings draft".into())
}

fn patched(bytes: &Option<Vec<u8>>, draft: &SettingsDraft) -> Result<Vec<u8>, ResourceError> {
    let mut value = match bytes {
        Some(bytes) => {
            serde_yaml::from_slice::<serde_yaml::Value>(bytes).map_err(|_| validation())?
        }
        None => serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
    };
    let defaults =
        serde_yaml::to_value(crate::runtime_settings::TrajectoryRuntimeSettings::default())
            .map_err(|_| validation())?;
    let known = defaults.as_mapping().ok_or_else(validation)?;
    let map = value.as_mapping_mut().ok_or_else(validation)?;
    if draft.changes.is_empty() {
        return Err(validation());
    }
    for (key, value) in &draft.changes {
        let key = serde_yaml::Value::String(key.clone());
        if !known.contains_key(&key) {
            return Err(validation());
        }
        map.insert(key, value.clone());
    }
    let bytes = serde_yaml::to_string(&value)
        .map(String::into_bytes)
        .map_err(|_| validation())?;
    crate::http::routers::v1::settings_config::buddy_trajectory_parse(&bytes)
        .map_err(|_| validation())?;
    Ok(bytes)
}

impl SettingsAdapter {
    pub fn new(
        app: AppState,
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<SettingsDraft>>,
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
        if spec.adapter != "settings"
            || spec.kind != "settings_trajectory_patch"
            || spec.resource.resource_kind != "settings"
            || spec.resource.resource_id != "global/trajectory"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        Ok(())
    }

    async fn prepare(
        &self,
        spec: &ActionSpec,
    ) -> Result<(Option<Vec<u8>>, Vec<u8>), ResourceError> {
        self.check(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = buddy_trajectory_snapshot(&self.app)
            .await
            .map_err(ResourceError::Io)?;
        if version(&before) != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        let after = patched(&before, &draft)?;
        Ok((before, after))
    }
}

#[async_trait]
impl AsyncResourceAdapter for SettingsAdapter {
    fn adapter_id(&self) -> &str {
        "settings"
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
        Ok(ResourcePreview {
            resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(),
            proposed_version: version(&Some(after)), reload_needed: true,
            diff: Some("Persist trajectory settings. Live fields install immediately; event channel and writer/index/tool-catalog/VecDB rollout switches require restart. Environment rollout overrides retain precedence. Values and unknown fields withheld.".into()),
        })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let (before, after) = self.prepare(spec).await?;
        let post_version = version(&Some(after.clone()));
        let material = SettingsPreimage {
            project: self.project.clone(),
            resource_id: spec.resource.resource_id.clone(),
            post_version: post_version.clone(),
            bytes: before.clone(),
        };
        let reference = self
            .materials
            .put(&serde_json::to_vec(&material).map_err(|_| ResourceError::MaterialUnavailable)?)?;
        buddy_trajectory_compare_exchange(&self.app, &before, &Some(after))
            .await
            .map_err(|error| {
                if error == "settings version mismatch" {
                    ResourceError::VersionMismatch
                } else {
                    ResourceError::Io(error)
                }
            })?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: post_version,
            state: ResourceApplyState::DiskDoneReloadNeeded,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.check(spec)?;
        Ok(version(
            &buddy_trajectory_snapshot(&self.app)
                .await
                .map_err(ResourceError::Io)?,
        ) == post)
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        self.check(spec)?;
        let material: SettingsPreimage =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
        if material.project != self.project
            || material.resource_id != spec.resource.resource_id
            || material.post_version != recovery.expected_post_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        if version(&material.bytes) != spec.expected_version {
            return Err(ResourceError::MaterialUnavailable);
        }
        let current = buddy_trajectory_snapshot(&self.app)
            .await
            .map_err(ResourceError::Io)?;
        if version(&current) != recovery.expected_post_version {
            return Err(ResourceError::VersionMismatch);
        }
        buddy_trajectory_compare_exchange(&self.app, &current, &material.bytes)
            .await
            .map_err(ResourceError::Io)?;
        Ok(ResourceApplyState::DiskDoneReloadNeeded)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuddySettingsDraft {
    pub project: ProjectIdentity,
    pub changes: BTreeMap<String, serde_json::Value>,
}

pub struct BuddySettingsAdapter {
    app: AppState,
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<BuddySettingsDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
}

fn buddy_patch(
    bytes: &Option<Vec<u8>>,
    changes: &BTreeMap<String, serde_json::Value>,
) -> Result<Vec<u8>, ResourceError> {
    use crate::http::routers::v1::buddy::BuddySettingsRequest;
    if changes.is_empty() {
        return Err(validation());
    }
    let allowed = [
        "personality_prompt",
        "humor_enabled",
        "humor_level",
        "quiet_mode",
        "daily_digest_hour",
        "quiet_hours_mode",
        "quiet_hours_start",
        "quiet_hours_end",
        "muted_intents",
        "muted_chat_ids",
    ];
    if changes.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ResourceError::ValidationFailed("authority/budget/observation changes require a separate permission-policy action; settings persistence cannot grant authority".into()));
    }
    let request: BuddySettingsRequest =
        serde_json::from_value(serde_json::to_value(changes).map_err(|_| validation())?)
            .map_err(|_| validation())?;
    request.validate().map_err(|_| validation())?;
    let mut document = match bytes {
        Some(b) => serde_json::from_slice::<serde_json::Value>(b).map_err(|_| validation())?,
        None => serde_json::json!({}),
    };
    let map = document.as_object_mut().ok_or_else(validation)?;
    for (key, value) in changes {
        map.insert(key.clone(), value.clone());
    }
    serde_json::from_value::<crate::buddy::settings::BuddySettings>(document.clone())
        .map_err(|_| validation())?;
    serde_json::to_vec_pretty(&document).map_err(|_| validation())
}

impl BuddySettingsAdapter {
    pub fn new(
        app: AppState,
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<BuddySettingsDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
            app,
            project,
            drafts,
            materials,
        }
    }
    async fn root(&self, spec: &ActionSpec) -> Result<std::path::PathBuf, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "buddy_settings"
            || spec.kind != "buddy_settings_patch"
            || spec.resource.resource_kind != "settings"
            || spec.resource.resource_id != "project/buddy"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let root = std::path::PathBuf::from(&self.project.canonical_root);
        if !crate::files_correction::get_project_dirs(self.app.gcx.clone())
            .await
            .contains(&root)
        {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        Ok(root)
    }
    async fn prepare(
        &self,
        spec: &ActionSpec,
    ) -> Result<(std::path::PathBuf, Option<Vec<u8>>, Vec<u8>), ResourceError> {
        let root = self.root(spec).await?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = crate::buddy::settings::settings_snapshot(&root)
            .await
            .map_err(ResourceError::Io)?;
        if version(&before) != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        let after = buddy_patch(&before, &draft.changes)?;
        Ok((root, before, after))
    }
}
fn buddy_owner_error(e: String) -> ResourceError {
    if e == "settings version mismatch" {
        ResourceError::VersionMismatch
    } else {
        ResourceError::Io(e)
    }
}
#[async_trait]
impl AsyncResourceAdapter for BuddySettingsAdapter {
    fn adapter_id(&self) -> &str {
        "buddy_settings"
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
        let (_, _, after) = self.prepare(spec).await?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: version(&Some(after)), reload_needed: true, diff: Some("Persist exact project Buddy preferences, preserving unknown fields. Running Buddy keeps its current settings until service restart; no global override exists. Personality text is style, never authority. No permission, autonomy, observation, budget or rollout grants.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let (root, before, after) = self.prepare(spec).await?;
        let post = version(&Some(after.clone()));
        let m = SettingsPreimage {
            project: self.project.clone(),
            resource_id: spec.resource.resource_id.clone(),
            post_version: post.clone(),
            bytes: before.clone(),
        };
        let reference = self
            .materials
            .put(&serde_json::to_vec(&m).map_err(|_| ResourceError::MaterialUnavailable)?)?;
        crate::buddy::settings::settings_compare_exchange(&root, &before, &Some(after))
            .await
            .map_err(buddy_owner_error)?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: post,
            state: ResourceApplyState::DiskDoneReloadNeeded,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let root = self.root(spec).await?;
        Ok(version(
            &crate::buddy::settings::settings_snapshot(&root)
                .await
                .map_err(ResourceError::Io)?,
        ) == post)
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let root = self.root(spec).await?;
        let m: SettingsPreimage =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
        if m.project != self.project
            || m.resource_id != spec.resource.resource_id
            || m.post_version != recovery.expected_post_version
            || version(&m.bytes) != spec.expected_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        let current = crate::buddy::settings::settings_snapshot(&root)
            .await
            .map_err(ResourceError::Io)?;
        if version(&current) != recovery.expected_post_version {
            return Err(ResourceError::VersionMismatch);
        }
        crate::buddy::settings::settings_compare_exchange(&root, &current, &m.bytes)
            .await
            .map_err(buddy_owner_error)?;
        Ok(ResourceApplyState::DiskDoneReloadNeeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn buddy_preferences_preserve_unknowns_recover_exactly_and_reject_stale_or_authority() {
        let dir = tempfile::tempdir().unwrap();
        // The persisted schema requires these three authority fields even for legacy files.
        // Keep an exact byte preimage with unknown top-level and nested observer fields.
        let before = Some(
            br#"{"enabled":false,"auto_diagnostics":false,"auto_issue_creation":false,"quiet_mode":false,"future":{"secret":"private"},"observers":{"future":true}}"#
                .to_vec(),
        );
        serde_json::from_slice::<crate::buddy::settings::BuddySettings>(before.as_ref().unwrap())
            .expect("preimage must satisfy the canonical persisted settings schema");
        crate::buddy::settings::settings_compare_exchange(dir.path(), &None, &before)
            .await
            .unwrap();
        let changes = [("quiet_mode".into(), serde_json::json!(true))]
            .into_iter()
            .collect();
        let after = Some(buddy_patch(&before, &changes).unwrap());
        let value: serde_json::Value = serde_json::from_slice(after.as_ref().unwrap()).unwrap();
        assert_eq!(value["future"]["secret"], "private");
        assert_eq!(value["observers"]["future"], true);
        for key in ["enabled", "auto_diagnostics", "auto_issue_creation"] {
            assert_eq!(value[key], false, "preference patch must preserve {key}");
        }
        crate::buddy::settings::settings_compare_exchange(dir.path(), &before, &after)
            .await
            .unwrap();
        assert!(
            crate::buddy::settings::load_settings(dir.path())
                .await
                .quiet_mode
        );
        assert!(
            crate::buddy::settings::settings_compare_exchange(dir.path(), &before, &None)
                .await
                .is_err()
        );
        assert_eq!(
            crate::buddy::settings::settings_snapshot(dir.path())
                .await
                .unwrap(),
            after
        );
        crate::buddy::settings::settings_compare_exchange(dir.path(), &after, &before)
            .await
            .unwrap();
        assert_eq!(
            crate::buddy::settings::settings_snapshot(dir.path())
                .await
                .unwrap(),
            before
        );
        assert!(
            !crate::buddy::settings::load_settings(dir.path())
                .await
                .quiet_mode
        );
        for key in [
            "enabled",
            "autonomy_level",
            "daily_llm_token_budget",
            "auto_issue_creation",
            "observers",
        ] {
            assert!(buddy_patch(
                &before,
                &[(key.into(), serde_json::json!(true))]
                    .into_iter()
                    .collect()
            )
            .is_err());
        }
        assert!(buddy_patch(
            &before,
            &[("daily_digest_hour".into(), serde_json::json!(24))]
                .into_iter()
                .collect()
        )
        .is_err());
    }

    fn draft(key: &str, value: serde_yaml::Value) -> SettingsDraft {
        SettingsDraft {
            project: ProjectIdentity {
                project_id: "p".into(),
                canonical_root: "/p".into(),
            },
            changes: [(key.to_string(), value)].into_iter().collect(),
        }
    }
    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn owner_compare_exchange_restores_exact_preimage_and_refuses_stale_bytes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let path = crate::runtime_settings::settings_path(&app.paths.config_dir);
        let before = Some(b"max_queue_size: 7\nfuture: {credential: private}\n".to_vec());
        tokio::fs::create_dir_all(&app.paths.config_dir)
            .await
            .unwrap();
        tokio::fs::write(&path, before.as_ref().unwrap())
            .await
            .unwrap();
        let after = Some(
            patched(
                &before,
                &draft("max_queue_size", serde_yaml::to_value(9).unwrap()),
            )
            .unwrap(),
        );
        buddy_trajectory_compare_exchange(&app, &before, &after)
            .await
            .unwrap();
        assert_eq!(buddy_trajectory_snapshot(&app).await.unwrap(), after);
        assert_eq!(crate::runtime_settings::current().max_queue_size, 9);
        assert!(buddy_trajectory_compare_exchange(&app, &before, &None)
            .await
            .is_err());
        assert_eq!(buddy_trajectory_snapshot(&app).await.unwrap(), after);
        buddy_trajectory_compare_exchange(&app, &after, &before)
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(path).await.unwrap(), before.unwrap());
        assert_eq!(crate::runtime_settings::current().max_queue_size, 7);
        crate::runtime_settings::reset_for_test();
    }
    #[test]
    fn persisted_parser_rejects_schema_versions_and_malformed_known_values() {
        assert!(crate::runtime_settings::parse_persisted(b"schema_version: 99\n").is_err());
        assert!(crate::runtime_settings::parse_persisted(b"max_queue_size: wrong\n").is_err());
        assert!(crate::runtime_settings::parse_persisted(
            b"future: {anything: true}\nmax_queue_size: 7\n"
        )
        .is_ok());
    }
    #[test]
    fn patch_preserves_unknown_nested_and_secret_fields() {
        let before =
            Some(b"max_queue_size: 7\nfuture:\n  token: secret\n  list: [a, b]\n".to_vec());
        let after = patched(
            &before,
            &draft("max_queue_size", serde_yaml::to_value(9).unwrap()),
        )
        .unwrap();
        let before: serde_yaml::Value = serde_yaml::from_slice(before.as_ref().unwrap()).unwrap();
        let after: serde_yaml::Value = serde_yaml::from_slice(&after).unwrap();
        assert_eq!(before["future"], after["future"]);
        assert_eq!(after["max_queue_size"].as_u64(), Some(9));
    }
    #[test]
    fn patch_rejects_unknown_keys_and_invalid_known_values() {
        assert!(patched(&None, &draft("future", serde_yaml::Value::Null)).is_err());
        assert!(patched(
            &None,
            &draft("max_queue_size", serde_yaml::to_value(0).unwrap())
        )
        .is_err());
        assert_ne!(version(&None), version(&Some(Vec::new())));
    }
}
