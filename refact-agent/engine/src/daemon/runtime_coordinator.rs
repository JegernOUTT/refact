//! Bounded host coordinator. Cache observations and plans never authorize activation.
//! Routes own authentication/update gating; this module never starts or drains a process.
use refact_self_update::public_release::{self, PublicTransport, ResolvedPublicRelease};
use refact_self_update::runtime_artifact::ArtifactLimits;
use refact_self_update::runtime_manifest::{Compatibility, Qualification, SourceEvidence, TestResult};
use refact_self_update::runtime_selection::RuntimeSelection;
use refact_self_update::runtime_stage::{StageStore, StagedObservation};
use refact_self_update::runtime_status::{ManagedIdentity, Observation, RuntimeStatus};
use refact_self_update::selection_store::{SelectionSnapshot, SelectionStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectRequest {
    pub expected_revision: u64,
    pub selection: RuntimeSelection,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRequest {
    pub expected_revision: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageKey {
    pub manifest_digest: String,
    pub selection_revision: u64,
}
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: RuntimeStatus,
    pub target: String,
    pub diagnostics: Vec<String>,
    pub activation_supported: bool,
    pub restart_dispatched: bool,
}
#[derive(Debug, Serialize)]
pub struct UpdatePlan {
    pub selection_revision: u64,
    pub identity: ManagedIdentity,
    pub source: SourceEvidence,
    pub manifest_tests: TestResult,
    pub qualification: String,
    pub public_qualification_verified: bool,
    pub together: Vec<String>,
    pub ide_owned_not_updated: Vec<String>,
    pub activation_supported: bool,
    pub restart_dispatched: bool,
}
#[derive(Debug, Serialize)]
pub struct StageInfo {
    pub selection_revision: u64,
    pub manifest_digest: String,
    pub target: String,
    pub runtime_sha256: String,
    pub runtime_size: u64,
}
impl From<StagedObservation> for StageInfo {
    fn from(o: StagedObservation) -> Self {
        Self {
            selection_revision: o.selection_revision(),
            manifest_digest: o.manifest_digest().into(),
            target: o.target().into(),
            runtime_sha256: o.runtime_sha256().into(),
            runtime_size: o.runtime_size(),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct StageResponse {
    pub plan: UpdatePlan,
    pub staged: StageInfo,
}

#[derive(Clone)]
pub struct RuntimeCoordinator {
    selection_path: PathBuf,
    stage_root: PathBuf,
    target: String,
    epochs: (u32, u32, u32),
}
impl RuntimeCoordinator {
    pub fn host() -> Result<Self> {
        let config = super::paths::daemon_config_path();
        Self::new(
            config
                .parent()
                .ok_or("daemon config parent absent")?
                .join("runtime-selection.json"),
            super::paths::daemon_dir().join("runtime-staging"),
        )
    }
    /// Paths are host-owned configuration, never request parameters. Construction is read-only.
    pub fn new(selection_path: PathBuf, stage_root: PathBuf) -> Result<Self> {
        let info: serde_json::Value =
            serde_json::from_str(crate::version::build_info_json()).map_err(|e| e.to_string())?;
        let target = info["target"]
            .as_str()
            .ok_or("compiled target absent")?
            .to_owned();
        Self::with_host_data(selection_path, stage_root, target, compiled_epochs()?)
    }
    fn with_host_data(
        selection_path: PathBuf,
        stage_root: PathBuf,
        target: String,
        epochs: (u32, u32, u32),
    ) -> Result<Self> {
        if !selection_path.is_absolute()
            || !stage_root.is_absolute()
            || !refact_self_update::runtime_manifest::native_target(&target)
        {
            return Err("absolute host paths and supported compiled target required".into());
        }
        Ok(Self {
            selection_path,
            stage_root,
            target,
            epochs,
        })
    }
    fn selections(&self) -> Result<SelectionStore> {
        SelectionStore::new(self.selection_path.clone())
    }
    pub fn load_selection(&self) -> Result<Option<SelectionSnapshot>> {
        let parent = self
            .selection_path
            .parent()
            .ok_or("selection parent absent")?;
        if !inspect_ancestors(parent)? {
            return Ok(None);
        }
        self.selections()?.load()
    }
    /// No mkdir, lock creation, network, process discovery, or daemon startup.
    /// A supplied cache key is informational, and must match the current selection revision.
    pub fn status(&self, key: Option<StageKey>) -> StatusResponse {
        let mut response = StatusResponse {
            status: RuntimeStatus::default(),
            target: self.target.clone(),
            diagnostics: vec![],
            activation_supported: false,
            restart_dispatched: false,
        };
        match self.load_selection() {
            Ok(Some(s)) => {
                response.status.selection_revision = s.revision();
                response.status.selected = Observation::Known(s.selection().clone());
            }
            Ok(None) => response.diagnostics.push("runtime selection absent".into()),
            Err(e) => response
                .diagnostics
                .push(format!("selection unreadable: {e}")),
        }
        match key {
            None => response
                .diagnostics
                .push("staged runtime unknown: no stage key supplied".into()),
            Some(k) => {
                let observed: Result<ManagedIdentity> = (|| {
                    if k.selection_revision == 0
                        || k.selection_revision != response.status.selection_revision
                    {
                        return Err("stage key selection revision differs".into());
                    }
                    if !inspect_ancestors(&self.stage_root)? {
                        return Err("stage root absent".into());
                    }
                    let o = StageStore::open(&self.stage_root)?.observe(
                        &self.target,
                        &k.manifest_digest,
                        k.selection_revision,
                    )?;
                    let current = self
                        .load_selection()?
                        .ok_or("selection disappeared during stage observation")?;
                    if current.revision() != k.selection_revision {
                        return Err("selection changed during stage observation".into());
                    }
                    Ok(o.identity().clone())
                })();
                match observed {
                    Ok(identity) => response.status.staged = Observation::Known(identity),
                    Err(e) => response
                        .diagnostics
                        .push(format!("staged runtime unknown: {e}")),
                }
            }
        }
        response.status.running =
            Observation::Known(refact_self_update::runtime_status::RunningProcess {
                pid: std::process::id(),
                reported_versions: Observation::Unknown,
            });
        response.diagnostics.push(format!("daemon reports runtime version {}; installed identity and full running component versions are unknown", crate::version::VERSION));
        response
    }
    /// Explicit selection mutation alone prepares the host config parent; never chmod existing dirs.
    pub fn select(&self, request: SelectRequest) -> Result<SelectionSnapshot> {
        prepare_directory(
            self.selection_path
                .parent()
                .ok_or("selection parent absent")?,
            false,
        )?;
        self.selections()?
            .compare_and_swap(request.expected_revision, request.selection)
    }
    fn current(&self, revision: u64) -> Result<SelectionSnapshot> {
        let s = self.load_selection()?.ok_or("runtime selection absent")?;
        if revision == 0 || s.revision() != revision {
            return Err("selection revision conflict".into());
        }
        s.selection().require_managed_overwrite()?;
        Ok(s)
    }
    async fn snapshot(&self, revision: u64) -> Result<SelectionSnapshot> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.current(revision))
            .await
            .map_err(|e| format!("selection observation interrupted: {e}"))?
    }
    async fn resolve_snapshot(
        &self,
        transport: &impl PublicTransport,
        snapshot: &SelectionSnapshot,
    ) -> Result<ResolvedPublicRelease> {
        public_release::resolve_public_release(
            transport,
            snapshot.selection(),
            &self.target,
            self.epochs,
        )
        .await
        .map_err(|e| format!("public resolution failed: {e:?}"))
    }
    pub async fn resolve(
        &self,
        transport: &impl PublicTransport,
        request: RevisionRequest,
    ) -> Result<UpdatePlan> {
        let snapshot = self.snapshot(request.expected_revision).await?;
        let resolved = self.resolve_snapshot(transport, &snapshot).await?;
        let current = self.snapshot(request.expected_revision).await?;
        if current.selection() != snapshot.selection() {
            return Err("selection changed".into());
        }
        self.plan(&resolved, request.expected_revision)
    }
    pub async fn stage(
        &self,
        transport: &impl PublicTransport,
        request: RevisionRequest,
    ) -> Result<StageResponse> {
        let snapshot = self.snapshot(request.expected_revision).await?;
        let resolved = self.resolve_snapshot(transport, &snapshot).await?;
        let plan = self.plan(&resolved, request.expected_revision)?;
        let candidate = public_release::fetch_verified_candidate(
            transport,
            resolved,
            snapshot.selection(),
            &self.target,
            self.epochs,
            ArtifactLimits {
                archive_bytes: 256 * 1024 * 1024,
                runtime_bytes: 512 * 1024 * 1024,
            },
        )
        .await
        .map_err(|e| format!("public artifact failed: {e:?}"))?;
        let this = self.clone();
        let observed = tokio::task::spawn_blocking(move || {
            // Recheck after network, then StageStore owns the strict CAS fence through publication.
            this.current(request.expected_revision)?;
            prepare_directory(&this.stage_root, true)?;
            StageStore::open(&this.stage_root)?.stage(
                &this.selections()?,
                request.expected_revision,
                candidate,
            )
        })
        .await
        .map_err(|e| format!("stage completion unknown; inspect cache before retry: {e}"))??;
        Ok(StageResponse {
            plan,
            staged: observed.into(),
        })
    }
    fn plan(&self, resolved: &ResolvedPublicRelease, revision: u64) -> Result<UpdatePlan> {
        Ok(UpdatePlan {
            selection_revision: revision,
            identity: ManagedIdentity::from_manifest(resolved.manifest(), &self.target)?,
            source: resolved.source_evidence().clone(),
            manifest_tests: resolved.manifest().tests(),
            qualification: match resolved.qualification() {
                Qualification::Tested => "tested",
                Qualification::Untested => "untested",
            }
            .into(),
            public_qualification_verified: resolved.public_qualification().is_some(),
            together: ["runtime_cli", "daemon", "workers", "embedded_gui"]
                .map(String::from)
                .to_vec(),
            ide_owned_not_updated: ["vscode", "jetbrains"].map(String::from).to_vec(),
            activation_supported: false,
            restart_dispatched: false,
        })
    }
}

fn compiled_epochs() -> Result<(u32, u32, u32)> {
    #[derive(Deserialize)]
    struct Epochs {
        schema_version: u32,
        daemon_api: refact_self_update::runtime_manifest::Interval,
        worker_api: refact_self_update::runtime_manifest::Interval,
        host_bridge: refact_self_update::runtime_manifest::Interval,
    }
    let e: Epochs = serde_json::from_slice(include_bytes!(
        "../../../../packaging/runtime-compatibility.json"
    ))
    .map_err(|e| e.to_string())?;
    let c = Compatibility {
        daemon_api: e.daemon_api,
        worker_api: e.worker_api,
        host_bridge: e.host_bridge,
    };
    if e.schema_version != 1
        || [&c.daemon_api, &c.worker_api, &c.host_bridge]
            .iter()
            .any(|i| i.min == 0 || i.min != i.max)
    {
        return Err("compiled manifest compatibility must specify positive point epochs".into());
    }
    Ok((c.daemon_api.min, c.worker_api.min, c.host_bridge.min))
}

/// Inspect every existing ancestor even when a leaf is absent; dangling symlinks are not absence.
fn inspect_ancestors(path: &Path) -> Result<bool> {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("normalized absolute directory required".into());
    }
    let mut exists = true;
    for p in path.ancestors() {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let system_sticky = m.uid() == 0 && m.mode() & 0o1000 != 0;
                    if (m.uid() != 0 && m.uid() != unsafe { libc::geteuid() })
                        || (m.mode() & 0o022 != 0 && !system_sticky)
                    {
                        return Err(format!("unsafe ancestor owner/mode: {}", p.display()));
                    }
                }
            }
            Ok(_) => return Err(format!("unsafe directory {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => exists = false,
            Err(e) => return Err(format!("inspect {}: {e}", p.display())),
        }
    }
    Ok(exists)
}
fn validate_directory(path: &Path, private: bool) -> Result<()> {
    if !inspect_ancestors(path)? {
        return Err("directory absent".into());
    }
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o022 != 0
            || (private && m.mode() & 0o7777 != 0o700)
        {
            return Err(format!("unsafe directory owner/mode: {}", path.display()));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (m, private);
        return Err("runtime mutation directory security unsupported".into());
    }
    Ok(())
}
fn prepare_directory(path: &Path, private: bool) -> Result<()> {
    if inspect_ancestors(path)? {
        return validate_directory(path, private);
    }
    let parent = path.parent().ok_or("directory parent absent")?;
    prepare_directory(parent, false)?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    validate_directory(path, private)
}

#[cfg(test)]
#[path = "runtime_coordinator_tests.rs"]
mod tests;
