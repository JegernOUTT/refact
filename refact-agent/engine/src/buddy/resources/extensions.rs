//! Native extension adapters share versioned receipts, not an alternate resource store.
use std::sync::Arc;
use async_trait::async_trait;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::ext::operator_service::{self as owner, ResourceRights};

pub struct PreparedResource {
    pub relative: String,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
    pub rights: serde_json::Value,
}
pub trait ResourcePreparation: Send + Sync {
    fn adapter_id(&self) -> &str;
    fn project(&self) -> &ProjectIdentity;
    fn relative(&self, spec: &ActionSpec) -> Result<String, ResourceError>;
    fn prepare(&self, spec: &ActionSpec) -> Result<PreparedResource, ResourceError>;
}
pub struct ManagedResource<P> {
    preparation: Arc<P>,
    materials: Arc<dyn RecoveryMaterialStore>,
    rights: Arc<dyn ResourceRights>,
}
impl<P: ResourcePreparation> ManagedResource<P> {
    pub fn new(
        preparation: P,
        materials: Arc<dyn RecoveryMaterialStore>,
        rights: Arc<dyn ResourceRights>,
    ) -> Self {
        Self {
            preparation: Arc::new(preparation),
            materials,
            rights,
        }
    }
}
#[async_trait]
impl<P: ResourcePreparation + 'static> AsyncResourceAdapter for ManagedResource<P> {
    fn adapter_id(&self) -> &str {
        self.preparation.adapter_id()
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
        let p = self.preparation.clone();
        let s = spec.clone();
        tokio::task::spawn_blocking(move || {
            let prepared = p.prepare(&s)?;
            check_version(&s, &prepared.before)?;
            Ok(ResourcePreview { resource_id: s.resource.resource_id, expected_version: s.expected_version,
                proposed_version: owner::version(&Some(prepared.after)), reload_needed: true,
                diff: Some("Validated native resource revision. Apply requires exact independently issued resource rights. No probe, command execution, trust expansion or automatic activation.".into()) })
        }).await.map_err(|e| ResourceError::Io(e.to_string()))?
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let p = self.preparation.clone();
        let s = spec.clone();
        let materials = self.materials.clone();
        let rights = self.rights.clone();
        tokio::task::spawn_blocking(move || {
            let prepared = p.prepare(&s)?;
            check_version(&s, &prepared.before)?;
            let post = owner::version(&Some(prepared.after.clone()));
            rights.authorize(&s, &post, &prepared.rights)?;
            let material = serde_json::to_vec(&prepared.before)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
            let reference = materials.put(&material)?;
            owner::replace(
                p.project(),
                &prepared.relative,
                &s.expected_version,
                &Some(prepared.after),
            )?;
            Ok(ResourceApplyReceipt {
                resource_id: s.resource.resource_id,
                previous_material_reference: reference,
                expected_post_version: post,
                state: ResourceApplyState::DiskDoneReloadNeeded,
            })
        })
        .await
        .map_err(|e| ResourceError::Io(e.to_string()))?
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let p = self.preparation.clone();
        let s = spec.clone();
        let post = post.to_owned();
        tokio::task::spawn_blocking(move || {
            Ok(owner::version(&owner::snapshot(p.project(), &p.relative(&s)?)?) == post)
        })
        .await
        .map_err(|e| ResourceError::Io(e.to_string()))?
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let p = self.preparation.clone();
        let s = spec.clone();
        let r = recovery.clone();
        let materials = self.materials.clone();
        tokio::task::spawn_blocking(move || {
            let relative = p.relative(&s)?;
            let before: Option<Vec<u8>> =
                serde_json::from_slice(&materials.get(&r.previous_material_reference)?)
                    .map_err(|_| ResourceError::MaterialUnavailable)?;
            if owner::version(&before) != s.expected_version
                || s.resource.version != s.expected_version
            {
                return Err(ResourceError::MaterialUnavailable);
            }
            // Recovery is version-bound compensation only; no draft is loaded or reinterpreted.
            owner::replace(p.project(), &relative, &r.expected_post_version, &before)?;
            Ok(ResourceApplyState::DiskDoneReloadNeeded)
        })
        .await
        .map_err(|e| ResourceError::Io(e.to_string()))?
    }
}
fn check_version(spec: &ActionSpec, before: &Option<Vec<u8>>) -> Result<(), ResourceError> {
    if owner::version(before) != spec.expected_version
        || spec.resource.version != spec.expected_version
    {
        return Err(ResourceError::VersionMismatch);
    }
    Ok(())
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownKind {
    Skill,
    Command,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct MarkdownDraft {
    pub project: ProjectIdentity,
    pub kind: MarkdownKind,
    pub name: String,
    pub description: Option<String>,
    pub body: String,
}
pub struct MarkdownPreparation {
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<MarkdownDraft>>,
}
pub type ExtensionsAdapter = ManagedResource<MarkdownPreparation>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use refact_buddy_core::operator_types::{
        ActionAvailability, ActionCapabilities, AuthorityClass, ConfirmationClass, ResourceRef,
    };
    struct Draft(MarkdownDraft);
    impl DraftRegistry<MarkdownDraft> for Draft {
        fn get(&self, _: &str) -> Result<MarkdownDraft, ResourceError> {
            Ok(self.0.clone())
        }
    }
    #[derive(Default)]
    struct Materials(Mutex<HashMap<String, Vec<u8>>>);
    impl RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            let id = uuid::Uuid::new_v4().to_string();
            self.0.lock().unwrap().insert(id.clone(), bytes.to_vec());
            Ok(id)
        }
        fn get(&self, id: &str) -> Result<Vec<u8>, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or(ResourceError::MaterialUnavailable)
        }
    }
    struct FixtureRights {
        action: ActionSpec,
        post: String,
    }
    impl ResourceRights for FixtureRights {
        fn authorize(
            &self,
            spec: &ActionSpec,
            post: &str,
            _: &serde_json::Value,
        ) -> Result<(), ResourceError> {
            if spec == &self.action && post == self.post {
                Ok(())
            } else {
                Err(ResourceError::VersionMismatch)
            }
        }
    }
    #[tokio::test]
    async fn owner_effect_is_exact_recoverable_and_denied_without_grant() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: temp
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let path = temp.path().join(".refact/commands/example.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = b"---\nunknown:\n  retained: true\nallowed-tools: [read]\n---\nOriginal";
        std::fs::write(&path, original).unwrap();
        let expected = owner::version(&Some(original.to_vec()));
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "fixture".into(),
            adapter: "extensions".into(),
            kind: "markdown_project_apply".into(),
            project: project.clone(),
            resource: ResourceRef {
                resource_kind: "native_markdown".into(),
                resource_id: "commands/example".into(),
                version: expected.clone(),
            },
            expected_version: expected,
            parameter_ref: Some("private-draft".into()),
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
        let drafts = Arc::new(Draft(MarkdownDraft {
            project: project.clone(),
            kind: MarkdownKind::Command,
            name: "example".into(),
            description: Some("Updated".into()),
            body: "Updated body".into(),
        }));
        let materials = Arc::new(Materials::default());
        let denied = ExtensionsAdapter::new(
            MarkdownPreparation::new(project.clone(), drafts.clone()),
            materials.clone(),
            Arc::new(owner::DenyResourceRights),
        );
        let preview = denied.preview(&spec).await.unwrap();
        assert!(denied.apply(&spec).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let adapter = ExtensionsAdapter::new(
            MarkdownPreparation::new(project, drafts),
            materials,
            Arc::new(FixtureRights {
                action: spec.clone(),
                post: preview.proposed_version,
            }),
        );
        let receipt = adapter.apply(&spec).await.unwrap();
        assert!(adapter
            .verify(&spec, &receipt.expected_post_version)
            .await
            .unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("retained: true"));
        assert!(text.contains("allowed-tools"));
        assert!(text.contains("Updated body"));
        let recovery = RecoveryDescriptor {
            previous_material_reference: receipt.previous_material_reference,
            expected_post_version: receipt.expected_post_version,
        };
        adapter.recover(&spec, &recovery).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            adapter.recover(&spec, &recovery).await,
            Err(ResourceError::VersionMismatch)
        );
        struct FixtureProbe(std::sync::atomic::AtomicUsize);
        impl owner::HookProbe for FixtureProbe {
            fn trigger(
                &self,
                _: &crate::ext::hooks::HookEvent,
                tool: &str,
                reference: &str,
            ) -> Result<(), ResourceError> {
                assert_eq!(tool, "read");
                assert_eq!(reference, "fixture-ref");
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        let mut spec = spec.clone();
        spec.adapter = "hooks".into();
        spec.kind = "hook_project_apply".into();
        spec.resource.resource_kind = "hooks".into();
        spec.resource.resource_id = "project/hooks".into();
        let hook = crate::buddy::resources::hooks::HookDraft {
            project: spec.project.clone(),
            event: crate::ext::hooks::HookEvent::PreToolUse,
            command: "fixture-only".into(),
            command_reference: "fixture-ref".into(),
            matcher: "^(read)$".into(),
            timeout_seconds: 1,
            tool_access_scope: vec!["read".into()],
        };
        let probe = FixtureProbe(std::sync::atomic::AtomicUsize::new(0));
        assert!(owner::probe_hook(
            &probe,
            &owner::DenyResourceRights,
            &spec,
            "fixture-post",
            &hook,
            "read"
        )
        .is_err());
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        let grant = FixtureRights {
            action: spec.clone(),
            post: "fixture-post".into(),
        };
        assert!(owner::probe_hook(&probe, &grant, &spec, "fixture-post", &hook, "write").is_err());
        owner::probe_hook(&probe, &grant, &spec, "fixture-post", &hook, "read").unwrap();
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

impl MarkdownPreparation {
    pub fn new(project: ProjectIdentity, drafts: Arc<dyn DraftRegistry<MarkdownDraft>>) -> Self {
        Self { project, drafts }
    }
}
impl ResourcePreparation for MarkdownPreparation {
    fn adapter_id(&self) -> &str {
        "extensions"
    }
    fn project(&self) -> &ProjectIdentity {
        &self.project
    }
    fn relative(&self, spec: &ActionSpec) -> Result<String, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "extensions"
            || spec.kind != "markdown_project_apply"
            || spec.resource.resource_kind != "native_markdown"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let (kind, name) = spec
            .resource
            .resource_id
            .split_once('/')
            .ok_or(ResourceError::InvalidResourceId)?;
        owner::validate_name(name).map_err(ResourceError::ValidationFailed)?;
        match kind {
            "skills" => Ok(format!("skills/{name}/SKILL.md")),
            "commands" => Ok(format!("commands/{name}.md")),
            _ => Err(ResourceError::UnsupportedResource),
        }
    }
    fn prepare(&self, spec: &ActionSpec) -> Result<PreparedResource, ResourceError> {
        let relative = self.relative(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        let kind = match draft.kind {
            MarkdownKind::Skill => "skills",
            MarkdownKind::Command => "commands",
        };
        if draft.project != self.project
            || format!("{kind}/{}", draft.name) != spec.resource.resource_id
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = owner::snapshot(&self.project, &relative)?;
        let after = owner::prepare_markdown(
            before.as_deref(),
            kind,
            &draft.name,
            draft.description.as_deref(),
            &draft.body,
        )?;
        Ok(PreparedResource {
            relative,
            before,
            after,
            rights: serde_json::json!({"operation":"native_markdown_write", "kind":kind, "name":draft.name}),
        })
    }
}
