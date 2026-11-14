use crate::runtime_manifest::{
    digest, native_target, require, ComponentVersions, ContractResult, RuntimeManifest,
};
use crate::runtime_selection::{valid_absolute_path, RuntimeSelection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Observation<T> {
    Unknown,
    Known(T),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityWire {
    manifest_digest: String,
    target: String,
    component_versions: ComponentVersions,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "IdentityWire", into = "IdentityWire")]
pub struct ManagedIdentity(IdentityWire);
impl From<ManagedIdentity> for IdentityWire {
    fn from(i: ManagedIdentity) -> Self {
        i.0
    }
}
impl TryFrom<IdentityWire> for ManagedIdentity {
    type Error = String;
    fn try_from(w: IdentityWire) -> ContractResult<Self> {
        require(
            digest(&w.manifest_digest, 64) && native_target(&w.target),
            "invalid managed identity",
        )?;
        w.component_versions.validate()?;
        Ok(Self(w))
    }
}
impl ManagedIdentity {
    pub fn from_manifest(m: &RuntimeManifest, target: &str) -> ContractResult<Self> {
        require(
            m.artifacts().iter().any(|a| {
                a.component == crate::runtime_manifest::Component::Runtime && a.target == target
            }),
            "runtime target absent",
        )?;
        Self::try_from(IdentityWire {
            manifest_digest: m.digest(),
            target: target.into(),
            component_versions: m.component_versions().clone(),
        })
    }
    pub fn manifest_digest(&self) -> &str {
        &self.0.manifest_digest
    }
    pub fn target(&self) -> &str {
        &self.0.target
    }
    pub fn component_versions(&self) -> &ComponentVersions {
        &self.0.component_versions
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunningProcess {
    pub pid: u32,
    pub reported_versions: Observation<ComponentVersions>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledWire {
    absolute_path: String,
    identity: ManagedIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InstalledWire", into = "InstalledWire")]
pub struct InstalledRuntime(InstalledWire);
impl From<InstalledRuntime> for InstalledWire {
    fn from(i: InstalledRuntime) -> Self {
        i.0
    }
}
impl TryFrom<InstalledWire> for InstalledRuntime {
    type Error = String;
    fn try_from(w: InstalledWire) -> ContractResult<Self> {
        require(
            valid_absolute_path(&w.absolute_path),
            "invalid installed path",
        )?;
        Ok(Self(w))
    }
}
impl InstalledRuntime {
    pub fn new(
        selection: &RuntimeSelection,
        absolute_path: String,
        identity: ManagedIdentity,
    ) -> ContractResult<Self> {
        selection.require_managed_overwrite()?;
        Self::try_from(InstalledWire {
            absolute_path,
            identity,
        })
    }
    pub fn absolute_path(&self) -> &str {
        &self.0.absolute_path
    }
    pub fn identity(&self) -> &ManagedIdentity {
        &self.0.identity
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeStatus {
    pub selection_revision: u64,
    pub selected: Observation<RuntimeSelection>,
    pub installed: Observation<InstalledRuntime>,
    pub staged: Observation<ManagedIdentity>,
    pub running: Observation<RunningProcess>,
}
impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            selection_revision: 0,
            selected: Observation::Unknown,
            installed: Observation::Unknown,
            staged: Observation::Unknown,
            running: Observation::Unknown,
        }
    }
}
