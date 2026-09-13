use std::sync::Arc;
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use super::{DraftRegistry, ResourceError};
use super::extensions::{ManagedResource, PreparedResource, ResourcePreparation};
use crate::ext::operator_service as owner;

#[derive(Clone, Serialize, Deserialize)]
pub struct HookDraft {
    pub project: ProjectIdentity,
    pub event: crate::ext::hooks::HookEvent,
    pub command: String,
    pub command_reference: String,
    pub matcher: String,
    pub timeout_seconds: u64,
    pub tool_access_scope: Vec<String>,
}
pub struct HookPreparation {
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<HookDraft>>,
}
pub type HooksAdapter = ManagedResource<HookPreparation>;
impl HookPreparation {
    pub fn new(project: ProjectIdentity, drafts: Arc<dyn DraftRegistry<HookDraft>>) -> Self {
        Self { project, drafts }
    }
}
impl ResourcePreparation for HookPreparation {
    fn adapter_id(&self) -> &str {
        "hooks"
    }
    fn project(&self) -> &ProjectIdentity {
        &self.project
    }
    fn relative(&self, spec: &ActionSpec) -> Result<String, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != "hooks"
            || spec.kind != "hook_project_apply"
            || spec.resource.resource_kind != "hooks"
            || spec.resource.resource_id != "project/hooks"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        Ok("hooks.yaml".into())
    }
    fn prepare(&self, spec: &ActionSpec) -> Result<PreparedResource, ResourceError> {
        let relative = self.relative(spec)?;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != self.project {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let before = owner::snapshot(&self.project, &relative)?;
        let after = owner::prepare_hook(before.as_deref(), &draft)?;
        // Writing hooks.yaml is itself activation: native runners read on demand.
        let rights = serde_json::json!({"operation":"install_active_hook", "event":draft.event,
            "command":draft.command, "command_reference":draft.command_reference, "matcher":draft.matcher,
            "timeout_seconds":draft.timeout_seconds, "tools":draft.tool_access_scope});
        Ok(PreparedResource {
            relative,
            before,
            after,
            rights,
        })
    }
}
