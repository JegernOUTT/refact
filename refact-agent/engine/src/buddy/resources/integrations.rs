use std::sync::Arc;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use super::{DraftRegistry, ResourceError};
use super::extensions::{ManagedResource, PreparedResource, ResourcePreparation};
use crate::ext::operator_service as files;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpEndpoint {
    Stdio {
        command: String,
        args: Vec<String>,
        command_reference: String,
    },
    Http {
        url: String,
        credential_references: Vec<String>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct McpDraft {
    pub project: ProjectIdentity,
    pub instance_id: String,
    pub endpoint: McpEndpoint,
    pub tool_access_scope: Vec<String>,
}
/// Connected native MCP lifecycle, separate from inert disabled-config preparation.
pub struct McpRuntimeAdapter {
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<McpDraft>>,
    rights: Arc<dyn files::ResourceRights>,
    materials: Arc<dyn super::RecoveryMaterialStore>,
    gcx: Arc<crate::global_context::GlobalContext>,
    owner: crate::integrations::operator_service::McpSessionOwner,
    /// Probe receipts are ephemeral: they exist only to let `verify`/`recover` recognise a probe
    /// this adapter instance actually performed, keyed by action id.
    probe_receipts: tokio::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Process-local instance nonce fencing **only** ephemeral probe receipts.
    ///
    /// A capability probe has no durable or session effect, so refusing to recognise a receipt
    /// minted by a different adapter instance (or a previous process) can never block real
    /// compensation. Durable `mcp_enabled_reload` compensation deliberately does not use this:
    /// see `McpRecovery`, which is bound by re-checkable durable facts so that recovery stays
    /// possible across restarts.
    runtime_identity: String,
}
impl McpRuntimeAdapter {
    pub fn new(
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<McpDraft>>,
        rights: Arc<dyn files::ResourceRights>,
        gcx: Arc<crate::global_context::GlobalContext>,
        materials: Arc<dyn super::RecoveryMaterialStore>,
    ) -> Self {
        // A freshly minted nonce, not a pointer: allocator address reuse could otherwise let a
        // receipt from a dropped adapter be mistaken for one this instance produced.
        let runtime_identity = uuid::Uuid::new_v4().to_string();
        Self {
            project,
            drafts,
            rights,
            owner: crate::integrations::operator_service::McpSessionOwner::new(
                gcx.clone(),
                materials.clone(),
            ),
            gcx,
            materials,
            runtime_identity,
            probe_receipts: Default::default(),
        }
    }
    fn draft(&self, spec: &ActionSpec) -> Result<McpDraft, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if !matches!(
            spec.kind.as_str(),
            "mcp_capability_probe" | "mcp_enabled_reload"
        ) {
            return Err(ResourceError::UnsupportedResource);
        }
        self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )
    }
}
#[derive(Serialize, Deserialize)]
struct ProbeReceipt {
    spec: ActionSpec,
    tools: Vec<String>,
    runtime_identity: String,
}
#[async_trait::async_trait]
impl super::AsyncResourceAdapter for McpRuntimeAdapter {
    fn adapter_id(&self) -> &str {
        "integrations"
    }
    fn capabilities(&self) -> super::ResourceCapabilities {
        super::ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: true,
        }
    }
    async fn preview(&self, spec: &ActionSpec) -> Result<super::ResourcePreview, ResourceError> {
        let draft = self.draft(spec)?;
        let prepared = crate::integrations::operator_probe::prepare(spec, &draft)?;
        Ok(super::ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: prepared.post, reload_needed: false, diff: Some("Exact-scoped MCP initialization and tools listing; no invocation. Enabled reload persists and publishes only approved tools.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<super::ResourceApplyReceipt, ResourceError> {
        let draft = self.draft(spec)?;
        if spec.kind == "mcp_enabled_reload" {
            return self.owner.reload(self.rights.as_ref(), spec, &draft).await;
        }
        let tools = crate::integrations::operator_probe::probe_mcp(
            self.gcx.clone(),
            self.rights.as_ref(),
            spec,
            &draft,
        )
        .await?;
        let material = serde_json::to_vec(&ProbeReceipt {
            spec: spec.clone(),
            tools,
            runtime_identity: self.runtime_identity.clone(),
        })
        .map_err(|_| ResourceError::MaterialUnavailable)?;
        let reference = self.materials.put(&material)?;
        self.probe_receipts
            .lock()
            .await
            .insert(spec.action_id.clone(), reference.clone());
        Ok(super::ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: spec.expected_version.clone(),
            state: super::ResourceApplyState::Applied,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.draft(spec)?;
        if spec.kind == "mcp_enabled_reload" {
            return self.owner.verify(spec, post).await;
        }
        let receipts = self.probe_receipts.lock().await;
        let Some(reference) = receipts.get(&spec.action_id) else {
            return Ok(false);
        };
        let receipt: ProbeReceipt = serde_json::from_slice(&self.materials.get(reference)?)
            .map_err(|_| ResourceError::MaterialUnavailable)?;
        if receipt.spec != *spec || receipt.runtime_identity != self.runtime_identity {
            return Ok(false);
        }

        files::validate_name(&spec.resource.resource_id)
            .map_err(ResourceError::ValidationFailed)?;
        let relative = format!("integrations.d/mcp_{}.yaml", spec.resource.resource_id);
        Ok(post == spec.expected_version
            && files::version(&files::snapshot(&self.project, &relative)?) == post)
    }
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &super::RecoveryDescriptor,
    ) -> Result<super::ResourceApplyState, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.kind == "mcp_enabled_reload" {
            return self.owner.recover(spec, recovery).await;
        }
        let receipt: ProbeReceipt =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|_| ResourceError::MaterialUnavailable)?;
        if receipt.spec != *spec
            || receipt.runtime_identity != self.runtime_identity
            || recovery.expected_post_version != spec.expected_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        self.probe_receipts.lock().await.remove(&spec.action_id);
        // A capability probe has no persistent/session effect to compensate.
        Ok(super::ResourceApplyState::NotApplied)
    }
}

pub struct McpPreparation {
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<McpDraft>>,
}
pub type IntegrationsAdapter = ManagedResource<McpPreparation>;
impl McpPreparation {
    pub fn new(project: ProjectIdentity, drafts: Arc<dyn DraftRegistry<McpDraft>>) -> Self {
        Self {
            project,
            drafts,
        }
    }
}
impl ResourcePreparation for McpPreparation {
    fn adapter_id(&self) -> &str {
        "integrations"
    }
    fn project(&self) -> &ProjectIdentity {
        &self.project
    }
    fn relative(&self, spec: &ActionSpec) -> Result<String, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "integrations"
            || spec.kind != "mcp_project_apply"
            || spec.resource.resource_kind != "mcp"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        files::validate_name(&spec.resource.resource_id)
            .map_err(ResourceError::ValidationFailed)?;
        Ok(format!(
            "integrations.d/mcp_{}.yaml",
            spec.resource.resource_id
        ))
    }
    fn prepare(&self, spec: &ActionSpec) -> Result<PreparedResource, ResourceError> {
        let relative = self.relative(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project || draft.instance_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = files::snapshot(&self.project, &relative)?;
        let after = crate::integrations::operator_service::prepare_mcp(before.as_deref(), &draft)?;
        Ok(PreparedResource {
            relative,
            before,
            after,
            rights: serde_json::json!({"operation":"configure_mcp_disabled","draft":draft}),
        })
    }
}
