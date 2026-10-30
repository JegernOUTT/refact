//! Typed, project-scoped resource adapter contracts.
//!
//! Adapters receive reviewed action specs and opaque, typed draft/material references. They never
//! accept a caller-selected filesystem path or command string.

use refact_buddy_core::operator_types::ActionSpec;

pub mod customization;
pub mod default_models;
pub mod extensions;
pub mod history;
pub mod hooks;
pub mod integrations;
pub mod memory;
pub mod project_code;
pub mod providers;
pub mod retention;
pub mod settings;
pub mod worktree_housekeeping;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceCapabilities {
    pub preview: bool,
    pub apply: bool,
    pub verify: bool,
    pub recover: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResourceApplyState {
    /// Disk bytes are durable; the owner must explicitly reload its registry.
    DiskDoneReloadNeeded,
    Applied,
    NotApplied,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourcePreview {
    pub resource_id: String,
    pub expected_version: String,
    pub proposed_version: String,
    pub reload_needed: bool,
    pub diff: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceApplyReceipt {
    pub resource_id: String,
    /// Opaque reference only; no potentially sensitive preimage bytes are exposed.
    pub previous_material_reference: String,
    pub expected_post_version: String,
    pub state: ResourceApplyState,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryDescriptor {
    pub previous_material_reference: String,
    pub expected_post_version: String,
}

/// A `parameter_ref` is an opaque lookup key for a typed, previously validated draft.
pub trait DraftRegistry<D>: Send + Sync {
    fn get(&self, parameter_ref: &str) -> Result<D, ResourceError>;
}

/// Stores sensitive recovery preimages outside public action receipts.
pub trait RecoveryMaterialStore: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<String, ResourceError>;
    fn get(&self, reference: &str) -> Result<Vec<u8>, ResourceError>;
}

/// Authorization and explicit-grant processing belong to B08, outside this interface.
pub trait ResourceAdapter: Send + Sync {
    fn adapter_id(&self) -> &str;
    fn capabilities(&self) -> ResourceCapabilities;
    fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError>;
    fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError>;
    fn verify(&self, spec: &ActionSpec, expected_post_version: &str)
        -> Result<bool, ResourceError>;
    fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError>;
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResourceError {
    UnsupportedResource,
    InvalidResourceId,
    ProjectScopeMismatch,
    PathEscapesProject,
    SymlinkNotAllowed,
    ParameterReferenceMissing,
    DraftDoesNotMatchResource,
    VersionMismatch,
    ValidationFailed(String),
    KnownPartial {
        receipt: ResourceApplyReceipt,
        reason: String,
    },
    NotApplied(String),
    MaterialUnavailable,
    ResourceMissing,
    Io(String),
}

impl From<std::io::Error> for ResourceError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[async_trait::async_trait]
pub trait AsyncResourceAdapter: Send + Sync {
    /// Version produced by recovery; monotonic owners must not restore an old epoch.
    async fn recovered_version(
        &self,
        spec: &ActionSpec,
        _recovery: &RecoveryDescriptor,
    ) -> Result<String, ResourceError> {
        Ok(spec.expected_version.clone())
    }

    fn adapter_id(&self) -> &str;
    fn capabilities(&self) -> ResourceCapabilities;
    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError>;
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError>;
    async fn verify(
        &self,
        spec: &ActionSpec,
        expected_post_version: &str,
    ) -> Result<bool, ResourceError>;
    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError>;
}
