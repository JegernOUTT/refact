use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use sha2::{Digest, Sha256};

use super::{
    DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore, ResourceAdapter,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};

pub const ADAPTER_ID: &str = "project_customization_file";
pub const RESOURCE_KIND: &str = "project_customization_file";
pub const ACTION_KIND: &str = "customization_file_apply";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomizationDraft {
    pub kind: String,
    pub id: String,
    pub config: serde_json::Value,
}

/// Owns only a canonical project root and typed draft/material providers; it holds no GCX lock.
pub struct CustomizationFileAdapter<D, M> {
    project: ProjectIdentity,
    root: PathBuf,
    drafts: Arc<D>,
    materials: Arc<M>,
}

impl<D, M> CustomizationFileAdapter<D, M>
where
    D: DraftRegistry<CustomizationDraft>,
    M: RecoveryMaterialStore,
{
    pub fn new(
        project: ProjectIdentity,
        drafts: Arc<D>,
        materials: Arc<M>,
    ) -> Result<Self, ResourceError> {
        let root = dunce::canonicalize(&project.canonical_root)?;
        if !root.is_dir() || root.to_string_lossy() != project.canonical_root {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        Ok(Self {
            project,
            root,
            drafts,
            materials,
        })
    }

    fn binding(&self, spec: &ActionSpec) -> Result<Binding, ResourceError> {
        if spec.adapter != ADAPTER_ID
            || spec.kind != ACTION_KIND
            || spec.resource.resource_kind != RESOURCE_KIND
        {
            return Err(ResourceError::UnsupportedResource);
        }
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let (kind, id) = parse_resource_id(&spec.resource.resource_id)?;
        // Canonical editor validation is deliberately reused directly, with no HTTP recursion.
        crate::yaml_configs::customization_service::validate_kind(&kind)
            .map_err(ResourceError::ValidationFailed)?;
        crate::yaml_configs::customization_service::validate_id(&id)
            .map_err(ResourceError::ValidationFailed)?;
        let path = self.checked_target(&kind, &id)?;
        Ok(Binding {
            kind,
            id,
            path,
        })
    }

    fn checked_target(&self, kind: &str, id: &str) -> Result<PathBuf, ResourceError> {
        let target = self
            .root
            .join(".refact")
            .join(kind)
            .join(format!("{id}.yaml"));
        if !target.starts_with(&self.root) {
            return Err(ResourceError::PathEscapesProject);
        }
        // Existing path components must be ordinary directories; never follow a symlink under root.
        let mut current = self.root.clone();
        for component in [".refact", kind] {
            current.push(component);
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(ResourceError::SymlinkNotAllowed);
                }
            }
        }
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ResourceError::SymlinkNotAllowed);
            }
        }
        Ok(target)
    }

    fn current_bytes(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        fs::read(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ResourceError::ResourceMissing,
            _ => error.into(),
        })
    }

    fn require_current_version(
        &self,
        spec: &ActionSpec,
        path: &Path,
    ) -> Result<Vec<u8>, ResourceError> {
        let bytes = self.current_bytes(path)?;
        let actual = digest(&bytes);
        if spec.expected_version != actual || spec.resource.version != actual {
            return Err(ResourceError::VersionMismatch);
        }
        Ok(bytes)
    }

    fn draft(&self, spec: &ActionSpec, binding: &Binding) -> Result<Vec<u8>, ResourceError> {
        let reference = spec
            .parameter_ref
            .as_deref()
            .ok_or(ResourceError::ParameterReferenceMissing)?;
        let draft = self.drafts.get(reference)?;
        if draft.kind != binding.kind || draft.id != binding.id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let original: serde_json::Value =
            serde_yaml::from_slice(&self.current_bytes(&binding.path)?)
                .map_err(|error| ResourceError::ValidationFailed(error.to_string()))?;
        let config = preserve_unmentioned_fields(original.clone(), &draft.config)?;
        crate::yaml_configs::customization_service::reject_authority_widening(&original, &config)
            .map_err(ResourceError::ValidationFailed)?;
        crate::yaml_configs::customization_service::validate_config(
            &binding.kind,
            &config,
            &binding.id,
        )
        .map_err(ResourceError::ValidationFailed)?;
        serde_yaml::to_string(&config)
            .map(|text| text.into_bytes())
            .map_err(|error| ResourceError::ValidationFailed(error.to_string()))
    }

    fn atomic_replace(
        &self,
        target: &Path,
        expected_current_version: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.checked_target_for_path(target)?;
        crate::yaml_configs::customization_service::replace_checked(
            target,
            Some(expected_current_version),
            bytes,
        )
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ResourceError::VersionMismatch
            } else {
                ResourceError::Io(error.to_string())
            }
        })
    }

    fn checked_target_for_path(&self, target: &Path) -> Result<(), ResourceError> {
        let relative = target
            .strip_prefix(&self.root)
            .map_err(|_| ResourceError::PathEscapesProject)?;
        if relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(ResourceError::PathEscapesProject);
        }
        let mut current = self.root.clone();
        for part in relative.components() {
            current.push(part.as_os_str());
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                if metadata.file_type().is_symlink() {
                    return Err(ResourceError::SymlinkNotAllowed);
                }
            }
        }
        Ok(())
    }
}

impl<D, M> ResourceAdapter for CustomizationFileAdapter<D, M>
where
    D: DraftRegistry<CustomizationDraft>,
    M: RecoveryMaterialStore,
{
    fn adapter_id(&self) -> &str {
        ADAPTER_ID
    }

    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: true,
        }
    }

    fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let binding = self.binding(spec)?;
        self.require_current_version(spec, &binding.path)?;
        let bytes = self.draft(spec, &binding)?;
        Ok(ResourcePreview {
            resource_id: spec.resource.resource_id.clone(),
            expected_version: spec.expected_version.clone(),
            proposed_version: digest(&bytes),
            reload_needed: true,
            diff: Some(format!(
                "--- current\n{}\n+++ proposed\n{}",
                String::from_utf8_lossy(&self.current_bytes(&binding.path)?),
                String::from_utf8_lossy(&bytes)
            )),
        })
    }

    fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let binding = self.binding(spec)?;
        let previous = self.require_current_version(spec, &binding.path)?;
        let next = self.draft(spec, &binding)?;
        let material = self.materials.put(&previous)?;
        self.atomic_replace(&binding.path, &spec.expected_version, &next)?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: material,
            expected_post_version: digest(&next),
            state: ResourceApplyState::DiskDoneReloadNeeded,
        })
    }

    fn verify(
        &self,
        spec: &ActionSpec,
        expected_post_version: &str,
    ) -> Result<bool, ResourceError> {
        let binding = self.binding(spec)?;
        Ok(digest(&self.current_bytes(&binding.path)?) == expected_post_version)
    }

    fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let binding = self.binding(spec)?;
        if digest(&self.current_bytes(&binding.path)?) != recovery.expected_post_version {
            return Err(ResourceError::VersionMismatch);
        }
        let previous = self
            .materials
            .get(&recovery.previous_material_reference)
            .map_err(|_| ResourceError::MaterialUnavailable)?;
        if digest(&previous) != spec.expected_version {
            return Err(ResourceError::MaterialUnavailable);
        }
        self.atomic_replace(&binding.path, &recovery.expected_post_version, &previous)?;
        Ok(ResourceApplyState::DiskDoneReloadNeeded)
    }
}

struct Binding {
    kind: String,
    id: String,
    path: PathBuf,
}

fn preserve_unmentioned_fields(
    original: serde_json::Value,
    proposal: &serde_json::Value,
) -> Result<serde_json::Value, ResourceError> {
    crate::yaml_configs::customization_service::merge_preserving_unknown(&original, proposal)
        .map_err(ResourceError::ValidationFailed)
}

fn parse_resource_id(resource_id: &str) -> Result<(String, String), ResourceError> {
    let mut parts = resource_id.split('/');
    let kind = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or(ResourceError::InvalidResourceId)?;
    let id = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or(ResourceError::InvalidResourceId)?;
    if parts.next().is_some() {
        return Err(ResourceError::InvalidResourceId);
    }
    Ok((kind.to_owned(), id.to_owned()))
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{
        ActionAvailability, ActionCapabilities, AuthorityClass, ConfirmationClass, ResourceRef,
        OPERATOR_SCHEMA_VERSION_V1,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct Drafts(Mutex<HashMap<String, CustomizationDraft>>);
    impl DraftRegistry<CustomizationDraft> for Drafts {
        fn get(&self, reference: &str) -> Result<CustomizationDraft, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .ok_or(ResourceError::ParameterReferenceMissing)
        }
    }
    #[derive(Default)]
    struct Materials(Mutex<HashMap<String, Vec<u8>>>);
    impl RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            let key = format!("sealed-{}", digest(bytes));
            self.0.lock().unwrap().insert(key.clone(), bytes.to_vec());
            Ok(key)
        }
        fn get(&self, reference: &str) -> Result<Vec<u8>, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .ok_or(ResourceError::MaterialUnavailable)
        }
    }
    fn setup() -> (
        TempDir,
        CustomizationFileAdapter<Drafts, Materials>,
        Arc<Drafts>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let drafts = Arc::new(Drafts::default());
        let materials = Arc::new(Materials::default());
        let adapter =
            CustomizationFileAdapter::new(project, Arc::clone(&drafts), materials).unwrap();
        (temp, adapter, drafts)
    }
    fn spec(root: &Path, id: &str, version: String) -> ActionSpec {
        ActionSpec {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            action_id: "a".into(),
            kind: ACTION_KIND.into(),
            adapter: ADAPTER_ID.into(),
            project: ProjectIdentity {
                project_id: "p".into(),
                canonical_root: dunce::canonicalize(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            },
            resource: ResourceRef {
                resource_kind: RESOURCE_KIND.into(),
                resource_id: id.into(),
                version: version.clone(),
            },
            expected_version: version,
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
        }
    }
    fn valid_draft() -> CustomizationDraft {
        CustomizationDraft {
            kind: "modes".into(),
            id: "demo".into(),
            config: serde_json::json!({
                "schema_version": 1,
                "id": "demo",
                "title": "Demo",
                "description": "",
                "specific": false,
                "prompt": "",
                "plan_template": "",
                "tools": []
            }),
        }
    }
    #[test]
    fn preview_has_no_effect_and_apply_is_recoverable() {
        let (temp, adapter, drafts) = setup();
        let target = temp.path().join(".refact/modes/demo.yaml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old: true\n").unwrap();
        drafts
            .0
            .lock()
            .unwrap()
            .insert("draft".into(), valid_draft());
        let original = fs::read(&target).unwrap();
        let action = spec(temp.path(), "modes/demo", digest(&original));
        assert!(adapter.preview(&action).is_ok());
        assert_eq!(fs::read(&target).unwrap(), original);
        let receipt = adapter.apply(&action).unwrap();
        assert_eq!(receipt.state, ResourceApplyState::DiskDoneReloadNeeded);
        assert_eq!(
            digest(&fs::read(&target).unwrap()),
            receipt.expected_post_version
        );
        assert!(adapter
            .verify(&action, &receipt.expected_post_version)
            .unwrap());
        assert_eq!(
            adapter
                .recover(
                    &action,
                    &RecoveryDescriptor {
                        previous_material_reference: receipt.previous_material_reference,
                        expected_post_version: receipt.expected_post_version
                    }
                )
                .unwrap(),
            ResourceApplyState::DiskDoneReloadNeeded
        );
        assert_eq!(fs::read(&target).unwrap(), original);
    }
    #[test]
    fn traversal_symlink_stale_and_malformed_drafts_reject_without_changes() {
        let (temp, adapter, drafts) = setup();
        let target = temp.path().join(".refact/modes/demo.yaml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old: true\n").unwrap();
        let original = fs::read(&target).unwrap();
        let action = spec(temp.path(), "modes/demo", digest(&original));
        drafts.0.lock().unwrap().insert(
            "draft".into(),
            CustomizationDraft {
                config: serde_json::json!({"id":"demo"}),
                ..valid_draft()
            },
        );
        assert!(matches!(
            adapter.preview(&action),
            Err(ResourceError::ValidationFailed(_))
        ));
        let traversal = spec(temp.path(), "modes/../escape", digest(&original));
        assert!(matches!(
            adapter.preview(&traversal),
            Err(ResourceError::InvalidResourceId)
        ));
        assert_eq!(fs::read(&target).unwrap(), original);
        let stale = spec(temp.path(), "modes/demo", "not-current".into());
        assert!(matches!(
            adapter.apply(&stale),
            Err(ResourceError::VersionMismatch)
        ));
        assert_eq!(fs::read(&target).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = temp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::remove_file(&target).unwrap();
            fs::remove_dir(target.parent().unwrap()).unwrap();
            symlink(&outside, temp.path().join(".refact/modes")).unwrap();
            let symlinked = spec(temp.path(), "modes/demo", digest(&[]));
            assert!(matches!(
                adapter.preview(&symlinked),
                Err(ResourceError::SymlinkNotAllowed)
            ));
        }
    }
    #[test]
    fn recovery_rejects_intervening_edit() {
        let (temp, adapter, drafts) = setup();
        let target = temp.path().join(".refact/modes/demo.yaml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old: true\n").unwrap();
        drafts
            .0
            .lock()
            .unwrap()
            .insert("draft".into(), valid_draft());
        let action = spec(
            temp.path(),
            "modes/demo",
            digest(&fs::read(&target).unwrap()),
        );
        let receipt = adapter.apply(&action).unwrap();
        fs::write(&target, "intervening\n").unwrap();
        assert!(matches!(
            adapter.recover(
                &action,
                &RecoveryDescriptor {
                    previous_material_reference: receipt.previous_material_reference,
                    expected_post_version: receipt.expected_post_version
                }
            ),
            Err(ResourceError::VersionMismatch)
        ));
    }

    #[test]
    fn proposals_preserve_unknown_nested_fields_and_recovery_refuses_wrong_material() {
        let merged = preserve_unmentioned_fields(serde_json::json!({
            "unknown": { "keep": true }, "subchat": { "unknown": "retained", "model": "selected" }
        }), &serde_json::json!({ "subchat": { "model": "proposed" } })).unwrap();
        assert_eq!(merged["unknown"]["keep"], true);
        assert_eq!(merged["subchat"]["unknown"], "retained");
        assert_eq!(merged["subchat"]["model"], "proposed");
        let (temp, adapter, drafts) = setup();
        let target = temp.path().join(".refact/modes/demo.yaml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old: true\n").unwrap();
        drafts
            .0
            .lock()
            .unwrap()
            .insert("draft".into(), valid_draft());
        let action = spec(
            temp.path(),
            "modes/demo",
            digest(&fs::read(&target).unwrap()),
        );
        let receipt = adapter.apply(&action).unwrap();
        let post = fs::read(&target).unwrap();
        let wrong = adapter.materials.put(b"foreign: true\n").unwrap();
        assert!(matches!(
            adapter.recover(
                &action,
                &RecoveryDescriptor {
                    previous_material_reference: wrong,
                    expected_post_version: receipt.expected_post_version,
                }
            ),
            Err(ResourceError::MaterialUnavailable)
        ));
        assert_eq!(fs::read(&target).unwrap(), post);
    }
}

pub struct ActiveCustomizationAdapter<D, M> {
    files: Arc<CustomizationFileAdapter<D, M>>,
    gcx: Arc<crate::global_context::GlobalContext>,
    mutation: tokio::sync::Mutex<()>,
}

impl<D: DraftRegistry<CustomizationDraft>, M: RecoveryMaterialStore>
    ActiveCustomizationAdapter<D, M>
{
    pub fn new(
        files: CustomizationFileAdapter<D, M>,
        gcx: Arc<crate::global_context::GlobalContext>,
    ) -> Self {
        Self {
            files: Arc::new(files),
            gcx,
            mutation: tokio::sync::Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl<D: DraftRegistry<CustomizationDraft> + 'static, M: RecoveryMaterialStore + 'static>
    super::AsyncResourceAdapter for ActiveCustomizationAdapter<D, M>
{
    fn adapter_id(&self) -> &str {
        ADAPTER_ID
    }
    fn capabilities(&self) -> ResourceCapabilities {
        self.files.capabilities()
    }
    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let files = self.files.clone();
        let spec = spec.clone();
        tokio::task::spawn_blocking(move || files.preview(&spec))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))?
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let _guard = self.mutation.lock().await;
        let files = self.files.clone();
        let spec = spec.clone();
        let mut receipt = tokio::task::spawn_blocking(move || files.apply(&spec))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))??;
        if crate::yaml_configs::customization_service::reload_project(
            self.gcx.clone(),
            &self.files.root,
        )
        .await
        .is_ok()
        {
            receipt.state = ResourceApplyState::Applied;
        }
        Ok(receipt)
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let files = self.files.clone();
        let spec = spec.clone();
        let post = post.to_owned();
        if !tokio::task::spawn_blocking(move || files.verify(&spec, &post))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))??
        {
            return Ok(false);
        }
        Ok(crate::yaml_configs::customization_service::reload_project(
            self.gcx.clone(),
            &self.files.root,
        )
        .await
        .is_ok())
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let _guard = self.mutation.lock().await;
        let files = self.files.clone();
        let spec = spec.clone();
        let recovery = recovery.clone();
        tokio::task::spawn_blocking(move || files.recover(&spec, &recovery))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))??;
        Ok(
            if crate::yaml_configs::customization_service::reload_project(
                self.gcx.clone(),
                &self.files.root,
            )
            .await
            .is_ok()
            {
                ResourceApplyState::Applied
            } else {
                ResourceApplyState::DiskDoneReloadNeeded
            },
        )
    }
}
