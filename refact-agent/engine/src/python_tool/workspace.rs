use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use refact_python_kernel::{protocol::VariableSummary, snapshot};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::fs;

use crate::app_state::AppState;

#[derive(Clone, Debug)]
pub struct PythonWorkspace {
    dir: PathBuf,
    chat_id: String,
    key_path: PathBuf,
    project_quota: Option<(PathBuf, u64)>,
}

const MANIFEST: &str = "state.json";
/// The one pending manifest name of the layout before per-attempt names; still recovered.
const PENDING_MANIFEST: &str = "state.json.new";
const LEGACY_GENERATION: &str = "state.monty";
/// A file of unknown origin may be another engine's save in progress until it is this old.
const UNKNOWN_FILE_GRACE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

fn legacy_generation() -> String {
    LEGACY_GENERATION.into()
}
fn is_hex_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
fn is_generation_name(name: &str) -> bool {
    name == LEGACY_GENERATION
        || name
            .strip_prefix("state-")
            .and_then(|rest| rest.strip_suffix(".monty"))
            .is_some_and(is_hex_id)
}
fn is_pending_manifest_name(name: &str) -> bool {
    name == PENDING_MANIFEST
        || name
            .strip_prefix("state.json.")
            .and_then(|rest| rest.strip_suffix(".new"))
            .is_some_and(is_hex_id)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateMeta {
    pub schema: u32,
    /// The generation file this manifest publishes; manifests from before generations name
    /// the one fixed file.
    #[serde(default = "legacy_generation")]
    pub generation: String,
    /// The generation the replaced manifest named; it is kept until the next publish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub dump_version: String,
    pub saved_at_ms: u64,
    pub bytes: u64,
    pub sha256: String,
    pub last_run_id: String,
    pub hmac: String,
    pub taint: Vec<Value>,
    pub variables: Vec<VariableSummary>,
    #[serde(default)]
    pub variables_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables_total: Option<u64>,
    /// The variables could not be listed safely; an empty list then says nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub variables_unknown: bool,
}

/// The published state as one read of its manifest found it.
#[derive(Clone, Debug)]
pub struct PinnedState {
    pub path: PathBuf,
    pub meta: StateMeta,
    /// The manifest's bytes, to tell later whether it is still the current one.
    pub manifest: Vec<u8>,
}

/// What one read of the saved state finds.
pub enum Pin {
    Nothing,
    Pinned(PinnedState),
    /// The current manifest is invalid; its bytes let a clear check that it is still current.
    Invalid(ResetReason, Vec<u8>),
    /// The state could not be read now; nothing shows it is damaged, so it is kept.
    Unavailable(String),
}

enum Check {
    Invalid(ResetReason),
    Unavailable(String),
}

/// A snapshot file the worker wrote, with what the manifest will say about it.
pub struct NewState {
    pub generation: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub last_run_id: String,
    pub taint: Vec<Value>,
    pub variables: Vec<VariableSummary>,
    pub variables_truncated: bool,
    pub variables_total: Option<u64>,
    pub variables_unknown: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetReason(pub String);
impl std::fmt::Display for ResetReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ResetReason {}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistOutcome {
    pub persisted: bool,
    pub bytes: u64,
    /// What to tell about a large state, when it is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why the state was not saved while the worker keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_saved: Option<String>,
}

/// What a fork did not copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForkOutcome {
    /// Why the variables were not copied, when the source has saved state.
    pub variables_not_copied: Option<String>,
    /// Artifacts a running script of the source changed before their privacy labels were saved.
    pub artifacts_skipped: usize,
}
impl ForkOutcome {
    fn notes(&self, src_chat_id: &str) -> Vec<String> {
        let mut notes = vec![];
        if let Some(reason) = &self.variables_not_copied {
            notes.push(format!(
                "variables were not copied from chat {src_chat_id}: {reason}"
            ));
        }
        if self.artifacts_skipped > 0 {
            notes.push(format!(
                "{} artifacts were not copied from chat {src_chat_id}: a script there was still writing them and had not saved their privacy labels",
                self.artifacts_skipped
            ));
        }
        notes
    }
    /// One sentence for the caller of a fork, when something was not copied.
    pub fn summary(&self) -> Option<String> {
        let mut parts = vec![];
        if let Some(reason) = &self.variables_not_copied {
            parts.push(format!("Python variables were not copied ({reason})"));
        }
        if self.artifacts_skipped > 0 {
            parts.push(format!(
                "{} Python artifacts were not copied because a script was still writing them",
                self.artifacts_skipped
            ));
        }
        (!parts.is_empty()).then(|| format!("{}.", parts.join("; ")))
    }
}

/// Test-only crash points of the save routine: an armed step stops the routine there and
/// leaves the files as a crash at that step would.
pub(crate) fn interrupted(dir: &Path, step: &str) -> bool {
    #[cfg(test)]
    {
        let mut armed = SAVE_FAULTS.lock().unwrap();
        if let Some(i) = armed.iter().position(|(d, s)| d == dir && s == step) {
            armed.remove(i);
            return true;
        }
    }
    let _ = (dir, step);
    false
}
#[cfg(test)]
static SAVE_FAULTS: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) fn arm_save_fault(dir: &Path, step: &str) {
    SAVE_FAULTS
        .lock()
        .unwrap()
        .push((dir.to_path_buf(), step.to_owned()));
}

/// Test-only pause points: an armed step waits there until the test releases it.
pub(crate) async fn pause_point(dir: &Path, step: &str) {
    #[cfg(test)]
    {
        let armed = {
            let mut armed = PAUSES.lock().unwrap();
            armed
                .iter()
                .position(|(d, s, _)| d == dir && s == step)
                .map(|i| armed.remove(i).2)
        };
        if let Some(pause) = armed {
            pause.reached.notify_one();
            pause.release.notified().await;
        }
    }
    let _ = (dir, step);
}
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct Pause {
    reached: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}
#[cfg(test)]
impl Pause {
    pub(crate) async fn reached(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(30), self.reached.notified())
            .await
            .expect("pause point not reached");
    }
    pub(crate) fn release(&self) {
        self.release.notify_one();
    }
}
#[cfg(test)]
static PAUSES: Mutex<Vec<(PathBuf, String, Pause)>> = Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) fn arm_pause(dir: &Path, step: &str) -> Pause {
    let pause = Pause::default();
    PAUSES
        .lock()
        .unwrap()
        .push((dir.to_path_buf(), step.to_owned(), pause.clone()));
    pause
}

/// Test-only pause points in synchronous code: the calling thread itself waits.
pub(crate) fn blocking_pause_point(dir: &Path, step: &str) {
    #[cfg(test)]
    {
        let armed = {
            let mut armed = BLOCKING_PAUSES.lock().unwrap();
            armed
                .iter()
                .position(|(d, s, _, _)| d == dir && s == step)
                .map(|i| armed.remove(i))
        };
        if let Some((_, _, reached, release)) = armed {
            let _ = reached.send(());
            let _ = release.recv();
        }
    }
    let _ = (dir, step);
}
#[cfg(test)]
type BlockingPauseEntry = (
    PathBuf,
    String,
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);
#[cfg(test)]
static BLOCKING_PAUSES: Mutex<Vec<BlockingPauseEntry>> = Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) struct BlockingPause {
    reached: std::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}
#[cfg(test)]
impl BlockingPause {
    pub(crate) fn reached(&self) {
        self.reached
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("pause point not reached");
    }
    pub(crate) fn release(&self) {
        let _ = self.release.send(());
    }
}
#[cfg(test)]
pub(crate) fn arm_blocking_pause(dir: &Path, step: &str) -> BlockingPause {
    let (reached_tx, reached) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    BLOCKING_PAUSES.lock().unwrap().push((
        dir.to_path_buf(),
        step.to_owned(),
        reached_tx,
        release_rx,
    ));
    BlockingPause {
        reached,
        release,
    }
}

impl PythonWorkspace {
    pub async fn for_chat(app: &AppState, chat_id: &str) -> Result<Self, String> {
        let path =
            crate::chat::trajectories::python_workspace_trajectory_path(app, chat_id).await?;
        Self::for_trajectory(app, chat_id, path).await
    }

    pub async fn existing_for_chat(app: &AppState, chat_id: &str) -> Result<Option<Self>, String> {
        let Some(path) =
            crate::chat::trajectories::existing_python_workspace_trajectory_path(app, chat_id)
                .await?
        else {
            return Ok(None);
        };
        Self::for_trajectory(app, chat_id, path).await.map(Some)
    }

    async fn for_trajectory(app: &AppState, chat_id: &str, path: PathBuf) -> Result<Self, String> {
        let mut workspace = Self::from_trajectory(
            &path,
            chat_id,
            app.paths.config_dir.join("python_state.key"),
        );
        for root in crate::files_correction::get_project_dirs(app.gcx.clone()).await {
            if path.starts_with(&root) {
                let settings = crate::runtime_settings::coordination_for_project(&root).await?;
                workspace.project_quota =
                    Some((root.join(".refact"), settings.python_disk_quota_bytes));
                break;
            }
        }
        if workspace.project_quota.is_none() {
            let storage = if path.starts_with(&app.paths.config_dir) {
                app.paths.config_dir.clone()
            } else {
                workspace.storage_root()
            };
            let quota = refact_work_core::coordination_settings().python_disk_quota_bytes;
            workspace.project_quota = Some((storage, quota));
        }
        Ok(workspace)
    }

    pub(crate) fn from_trajectory(path: &Path, chat_id: &str, key_path: PathBuf) -> Self {
        Self {
            dir: path
                .parent()
                .unwrap_or(Path::new("."))
                .join("python")
                .join(chat_id),
            chat_id: chat_id.to_owned(),
            key_path,
            project_quota: None,
        }
    }
    pub async fn enforce_quota(&self, fallback_quota: u64) -> Result<(), String> {
        if let Some((root, quota)) = &self.project_quota {
            enforce_project_quota(root, *quota).await
        } else {
            let root = self
                .dir
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&self.dir);
            enforce_project_quota(root, fallback_quota).await
        }
    }
    pub fn storage_root(&self) -> PathBuf {
        if let Some((root, _)) = &self.project_quota {
            return root.clone();
        }
        let parent = self
            .dir
            .parent()
            .and_then(Path::parent)
            .unwrap_or(&self.dir);
        parent
            .ancestors()
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n == "trajectories" || n == "conversations")
            })
            .unwrap_or(parent)
            .to_path_buf()
    }
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn artifacts_dir(&self) -> PathBuf {
        self.dir.join("artifacts")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.dir.join("runs")
    }
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        // Encode untrusted identifiers as one path component.
        let safe: String = run_id
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                    (b as char).to_string()
                } else {
                    format!("%{b:02x}")
                }
            })
            .collect();
        self.runs_dir()
            .join(if safe.is_empty() { "%00" } else { &safe })
    }
    pub fn new_generation_path(&self) -> PathBuf {
        self.dir
            .join(format!("state-{}.monty", uuid::Uuid::new_v4().simple()))
    }
    pub async fn ensure_dirs(&self) -> Result<(), String> {
        for path in [self.dir.clone(), self.artifacts_dir(), self.runs_dir()] {
            crate::chat::trajectories::ensure_real_dir_tree(&path).await?;
        }
        Ok(())
    }
    pub async fn read_state_meta(&self) -> Option<StateMeta> {
        serde_json::from_slice(&fs::read(self.dir.join(MANIFEST)).await.ok()?).ok()
    }
    /// One read of the saved state, checked: schema, signature, name, length and, with
    /// `digest`, the generation's bytes. When the manifest is missing or invalid, a complete
    /// pending manifest left by a replace that did not finish stands in, newest first; `adopt`
    /// installs it, and only after its generation passed the digest.
    pub async fn pin(&self, adopt: bool, digest: bool) -> Pin {
        let manifest = self.dir.join(MANIFEST);
        let read = if interrupted(&self.dir, "manifest_read") {
            Err(std::io::Error::other("injected read failure"))
        } else {
            fs::read(&manifest).await
        };
        let mut invalid = None;
        match read {
            Ok(data) => match self.verified(data.clone(), digest).await {
                Ok(pinned) => return Pin::Pinned(pinned),
                Err(Check::Unavailable(reason)) => return Pin::Unavailable(reason),
                Err(Check::Invalid(reason)) => invalid = Some((reason, data)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Pin::Unavailable(format!("its metadata is unreadable: {e}")),
        }
        for (pending, data) in self.pending_manifests().await {
            match self.verified(data, digest || adopt).await {
                Ok(pinned) => {
                    if adopt {
                        if let Err(e) = install_manifest(&pending, &manifest).await {
                            tracing::warn!("Python state metadata left pending: {e}");
                        }
                    }
                    return Pin::Pinned(pinned);
                }
                Err(Check::Unavailable(reason)) => return Pin::Unavailable(reason),
                Err(Check::Invalid(_)) => {}
            }
        }
        match invalid {
            Some((reason, data)) => Pin::Invalid(reason, data),
            None => Pin::Nothing,
        }
    }
    /// The published state without reading its bytes, unless `adopt` installs a pending one.
    pub async fn pin_state(&self, adopt: bool) -> Result<Option<PinnedState>, ResetReason> {
        match self.pin(adopt, adopt).await {
            Pin::Nothing => Ok(None),
            Pin::Pinned(pinned) => Ok(Some(pinned)),
            Pin::Invalid(reason, _) => Err(reason),
            Pin::Unavailable(reason) => Err(ResetReason(reason)),
        }
    }
    async fn verified(&self, data: Vec<u8>, digest: bool) -> Result<PinnedState, Check> {
        let pinned = self.check_manifest(data).await?;
        if digest {
            verify_digest(&pinned).await?;
        }
        Ok(pinned)
    }
    async fn check_manifest(&self, data: Vec<u8>) -> Result<PinnedState, Check> {
        let invalid = |s: &str| Check::Invalid(ResetReason(s.to_owned()));
        let meta: StateMeta = serde_json::from_slice(&data)
            .map_err(|_| invalid("missing or invalid Python state metadata"))?;
        if meta.schema != 1 {
            return Err(invalid("unsupported Python state schema"));
        }
        let key = self
            .key()
            .await
            .map_err(|e| Check::Unavailable(format!("its signing key is unavailable: {e}")))?;
        if !snapshot::verify_with_taint(
            &key,
            &meta.sha256,
            &self.chat_id,
            &taint_digest(&meta.taint),
            &meta.hmac,
        ) {
            return Err(invalid("Python state authentication failed"));
        }
        if !is_generation_name(&meta.generation) {
            return Err(invalid("invalid Python snapshot name"));
        }
        let path = self.dir.join(&meta.generation);
        match fs::symlink_metadata(&path).await {
            Ok(file) if file.is_file() && file.len() == meta.bytes => Ok(PinnedState {
                path,
                meta,
                manifest: data,
            }),
            Ok(file) if file.is_file() => Err(invalid("Python snapshot digest mismatch")),
            Ok(_) => Err(invalid("Python snapshot is not a regular file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(invalid("Python snapshot is missing"))
            }
            Err(e) => Err(Check::Unavailable(format!(
                "its snapshot is unreadable: {e}"
            ))),
        }
    }
    /// The pending manifests, newest first, with their bytes; unreadable ones are left out.
    async fn pending_manifests(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = vec![];
        for path in self.pending_manifest_paths().await {
            if let Ok(data) = fs::read(&path).await {
                let saved = serde_json::from_slice::<StateMeta>(&data).map_or(0, |m| m.saved_at_ms);
                found.push((saved, path, data));
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        found
            .into_iter()
            .map(|(_, path, data)| (path, data))
            .collect()
    }
    async fn pending_manifest_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![];
        let Ok(mut entries) = fs::read_dir(&self.dir).await else {
            return paths;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_str()
                .is_some_and(is_pending_manifest_name)
                && entry.file_type().await.is_ok_and(|t| t.is_file())
            {
                paths.push(entry.path());
            }
        }
        paths
    }
    /// The generations the current and the pending manifests name.
    async fn named_generations(&self) -> Vec<String> {
        let current = fs::read(self.dir.join(MANIFEST)).await.ok();
        let pending = self.pending_manifests().await;
        current
            .iter()
            .chain(pending.iter().map(|(_, data)| data))
            .filter_map(|data| serde_json::from_slice::<StateMeta>(data).ok())
            .map(|meta| meta.generation)
            .collect()
    }
    /// Whether a manifest is there, current or pending, even one that does not parse.
    async fn has_manifest(&self) -> bool {
        !matches!(
            fs::symlink_metadata(self.dir.join(MANIFEST)).await,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        ) || !self.pending_manifest_paths().await.is_empty()
    }
    /// The pinned state, its bytes checked against the signed digest.
    pub async fn verify_state(&self) -> Result<PinnedState, ResetReason> {
        match self.pin(false, true).await {
            Pin::Pinned(pinned) => Ok(pinned),
            Pin::Nothing => Err(ResetReason(
                "missing or invalid Python state metadata".into(),
            )),
            Pin::Invalid(reason, _) => Err(reason),
            Pin::Unavailable(reason) => Err(ResetReason(reason)),
        }
    }
    /// Clears the state when the manifest is still `expected`; false when it changed meanwhile.
    pub async fn clear_if_current(&self, expected: &[u8]) -> Result<bool, String> {
        match fs::read(self.dir.join(MANIFEST)).await {
            Ok(current) if current == expected => {}
            Ok(_) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.to_string()),
        }
        self.clear_state().await.map(|()| true)
    }
    /// The one commit point of a save: the manifest is replaced by one that names this exact
    /// generation. The generation the replaced manifest named stays until the next publish;
    /// nothing after the commit undoes it. A state over the limit is refused and the published
    /// state stays as it is.
    pub async fn record_snapshot(
        &self,
        state: NewState,
        max_state_bytes: u64,
    ) -> Result<PersistOutcome, String> {
        let name = state
            .generation
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| {
                is_generation_name(name) && state.generation.parent() == Some(self.dir.as_path())
            })
            .ok_or("Invalid Python snapshot name")?
            .to_owned();
        if state.bytes > max_state_bytes {
            remove_file(&state.generation).await?;
            return Ok(PersistOutcome {
                persisted: false,
                bytes: state.bytes,
                ..Default::default()
            });
        }
        match fs::symlink_metadata(&state.generation).await {
            Ok(file) if file.is_file() && file.len() == state.bytes => {}
            _ => return Err("Python snapshot file does not match the worker's report".into()),
        }
        let key = self.key().await?;
        let replaced = self.read_state_meta().await;
        let previous = replaced.as_ref().map(|meta| meta.generation.clone());
        let superseded = replaced.and_then(|meta| meta.previous);
        let meta = StateMeta {
            schema: 1,
            generation: name.clone(),
            previous: previous.clone(),
            dump_version: "monty-1.6.0".into(),
            saved_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            bytes: state.bytes,
            hmac: snapshot::sign_with_taint(
                &key,
                &state.sha256,
                &self.chat_id,
                &taint_digest(&state.taint),
            ),
            sha256: state.sha256,
            last_run_id: state.last_run_id,
            variables: state.variables,
            variables_truncated: state.variables_truncated,
            variables_total: state.variables_total,
            variables_unknown: state.variables_unknown,
            taint: state.taint,
        };
        self.publish_manifest(&meta).await?;
        if interrupted(&self.dir, "published") {
            return Err("interrupted".into());
        }
        let keep: Vec<&str> = [Some(name.as_str()), previous.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let known: Vec<&str> = superseded.as_deref().into_iter().collect();
        if let Err(e) = self.collect_generations(&keep, &known).await {
            tracing::warn!("Python state cleanup failed: {e}");
        }
        if let Some((root, quota)) = &self.project_quota {
            if let Err(e) = enforce_project_quota(root, *quota).await {
                tracing::warn!("Python workspace quota check failed: {e}");
            }
        }
        Ok(PersistOutcome {
            persisted: true,
            bytes: meta.bytes,
            ..Default::default()
        })
    }
    /// The manifest is written under a name of its own, made durable, then renamed over the
    /// current one; a pending manifest of another attempt is never written over.
    async fn publish_manifest(&self, meta: &StateMeta) -> Result<(), String> {
        let pending = self
            .dir
            .join(format!("{MANIFEST}.{}.new", uuid::Uuid::new_v4().simple()));
        let data = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
        if interrupted(&self.dir, "manifest_write_cut") {
            let _ = fs::write(&pending, &data[..data.len() / 2]).await;
            return Err("interrupted".into());
        }
        let written = async {
            use tokio::io::AsyncWriteExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending)
                .await?;
            file.write_all(&data).await?;
            file.sync_all().await?;
            sync_dir(&self.dir).await
        }
        .await;
        if let Err(e) = written {
            let _ = fs::remove_file(&pending).await;
            return Err(format!("Cannot write Python state metadata: {e}"));
        }
        if interrupted(&self.dir, "manifest_written") {
            return Err("interrupted".into());
        }
        let installed = install_manifest(&pending, &self.dir.join(MANIFEST)).await;
        if installed.is_err() {
            let _ = fs::remove_file(&pending).await;
        }
        installed
    }
    /// Removes generation files no manifest needs: `known` ones at once, files of unknown
    /// origin only when no manifest names them and they are older than [`UNKNOWN_FILE_GRACE`],
    /// since another engine may be about to publish them. Pending manifests that old are
    /// leftovers of saves that never finished.
    async fn collect_generations(&self, keep: &[&str], known: &[&str]) -> Result<(), String> {
        let mut entries = match fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let mut candidates = vec![];
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let old = meta
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age > UNKNOWN_FILE_GRACE);
            if (name.starts_with("state.monty.") && name.ends_with(".tmp"))
                || name.starts_with("state.tmp.")
                || (old && is_pending_manifest_name(name))
            {
                remove_file(&entry.path()).await?;
            } else if is_generation_name(name) && !keep.contains(&name) {
                candidates.push((name.to_owned(), entry.path(), old));
            }
        }
        let named = self.named_generations().await;
        for (name, path, old) in candidates {
            if known.contains(&name.as_str()) || (old && !named.contains(&name)) {
                remove_file(&path).await?;
            }
        }
        Ok(())
    }
    /// Removes a generation an attempt wrote and did not publish; one a manifest names stays.
    pub async fn discard_generation(&self, generation: &Path) {
        let named = self.named_generations().await;
        if generation
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| named.iter().any(|n| n == name))
        {
            return;
        }
        if let Err(e) = remove_file(generation).await {
            tracing::warn!("Python state cleanup failed: {e}");
        }
    }
    pub async fn clear_state(&self) -> Result<(), String> {
        let pending = self.pending_manifests().await;
        let current = fs::read(self.dir.join(MANIFEST)).await.ok();
        let mut known = vec![];
        for data in pending.iter().map(|(_, data)| data).chain(current.iter()) {
            if let Ok(meta) = serde_json::from_slice::<StateMeta>(data) {
                known.push(meta.generation);
                known.extend(meta.previous);
            }
        }
        for path in self.pending_manifest_paths().await {
            remove_file(&path).await?;
        }
        remove_file(&self.dir.join(MANIFEST)).await?;
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        self.collect_generations(&[], &known).await
    }
    pub async fn list_artifacts(&self, cap: usize) -> Result<Vec<PathBuf>, String> {
        list(&self.artifacts_dir(), cap).await
    }
    pub async fn list_runs(&self, cap: usize) -> Result<Vec<PathBuf>, String> {
        list(&self.runs_dir(), cap).await
    }
    pub async fn disk_usage(&self) -> Result<u64, String> {
        tree_size(&self.dir).await
    }
    async fn key(&self) -> Result<Vec<u8>, String> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        if let Ok(meta) = fs::symlink_metadata(&self.key_path).await {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("Python state signing key is not a real file".into());
            }
        }
        match fs::read(&self.key_path).await {
            Ok(key) if key.len() == 32 => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&self.key_path, std::fs::Permissions::from_mode(0o600))
                        .await
                        .map_err(|e| e.to_string())?;
                }
                return Ok(key);
            }
            Ok(_) => return Err("Invalid Python state signing key".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        fs::create_dir_all(self.key_path.parent().ok_or("No signing key directory")?)
            .await
            .map_err(|e| e.to_string())?;
        let mut key = vec![0; 32];
        rand::thread_rng().fill_bytes(&mut key);
        let tmp = self
            .key_path
            .with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp).await.map_err(|e| e.to_string())?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&key).await.map_err(|e| e.to_string())?;
        file.sync_all().await.map_err(|e| e.to_string())?;
        drop(file);
        // Hard-link publication avoids replacing a key created by a second engine.
        match fs::hard_link(&tmp, &self.key_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                let _ = fs::remove_file(&tmp).await;
                return Err(e.to_string());
            }
        }
        let _ = fs::remove_file(&tmp).await;
        let key = fs::read(&self.key_path).await.map_err(|e| e.to_string())?;
        if key.len() != 32 {
            return Err("Invalid Python state signing key".into());
        }
        Ok(key)
    }
    pub async fn delete_for_chat(app: &AppState, chat_id: &str) -> Result<(), String> {
        app.python_tool.registry.drop_chat(chat_id).await;
        let workspace = Self::for_chat(app, chat_id).await?;
        remove_dir(workspace.dir()).await
    }
    /// Copies the source's last published state, artifacts and their ledger into a new chat.
    /// The destination is held for the copy, so its first run waits for it and says what was
    /// not copied.
    pub async fn fork(
        app: &AppState,
        src_chat_id: &str,
        dst_chat_id: &str,
    ) -> Result<ForkOutcome, String> {
        let src = Self::for_chat(app, src_chat_id).await?;
        let dst = Self::for_chat(app, dst_chat_id).await?;
        let mut lease = app
            .python_tool
            .registry
            .reserve_turn(dst_chat_id, "fork")
            .acquire()
            .await
            .map_err(|busy| busy.to_string())?;
        let outcome = if lease.has_worker() {
            ForkOutcome {
                variables_not_copied: Some("this chat already has a Python session".into()),
                ..Default::default()
            }
        } else {
            src.fork_to(&dst).await?
        };
        for note in outcome.notes(src_chat_id) {
            lease.note(note);
        }
        Ok(outcome)
    }
    /// While a run of the source is active, its artifacts changed since it started and without
    /// a ledger row yet are not copied, so no copy lacks the label its bytes need.
    pub(crate) async fn fork_to(&self, dst: &Self) -> Result<ForkOutcome, String> {
        let mut outcome = ForkOutcome::default();
        if !fs::try_exists(&self.dir).await.map_err(|e| e.to_string())? {
            return Ok(outcome);
        }
        dst.ensure_dirs().await?;
        if dst.has_manifest().await {
            outcome.variables_not_copied = Some("this chat already has saved Python state".into());
            return Ok(outcome);
        }
        match self.pinned_bytes().await {
            Ok(None) => {}
            Ok(Some((meta, data))) => {
                let generation = dst.new_generation_path();
                write_new_file(&generation, &data).await?;
                dst.record_snapshot(
                    NewState {
                        generation,
                        sha256: meta.sha256,
                        bytes: meta.bytes,
                        last_run_id: meta.last_run_id,
                        taint: meta.taint,
                        variables: meta.variables,
                        variables_truncated: meta.variables_truncated,
                        variables_total: meta.variables_total,
                        variables_unknown: meta.variables_unknown,
                    },
                    u64::MAX,
                )
                .await?;
            }
            Err(reason) => outcome.variables_not_copied = Some(reason),
        }
        let busy_since = active_run_start(&self.runs_dir());
        let artifacts = self.artifacts_dir();
        let canonical = fs::canonicalize(&artifacts).await.ok();
        let mut labelled = std::collections::HashSet::new();
        let ledger = self.dir.join("privacy_ledger.json");
        if let Ok(data) = fs::read(ledger).await {
            let mut entries: Vec<(PathBuf, crate::privacy::records::DerivedZone)> =
                serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            for (path, _) in &mut entries {
                let relative = path
                    .strip_prefix(&artifacts)
                    .ok()
                    .or_else(|| {
                        canonical
                            .as_ref()
                            .and_then(|root| path.strip_prefix(root).ok())
                    })
                    .map(Path::to_path_buf);
                if let Some(relative) = relative {
                    *path = dst.artifacts_dir().join(&relative);
                    labelled.insert(relative);
                }
            }
            atomic_write(
                &dst.dir.join("privacy_ledger.json"),
                &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
            )
            .await?;
        }
        outcome.artifacts_skipped =
            copy_tree(&artifacts, &dst.artifacts_dir(), |relative, meta| {
                busy_since.is_some_and(|since| {
                    !labelled.contains(relative)
                        && meta.modified().map_or(true, |modified| {
                            modified + std::time::Duration::from_secs(2) >= since
                        })
                })
            })
            .await?;
        Ok(outcome)
    }
    /// The published generation's manifest and bytes, read once and checked against each
    /// other; a generation collected meanwhile is pinned again.
    async fn pinned_bytes(&self) -> Result<Option<(StateMeta, Vec<u8>)>, String> {
        for _ in 0..3 {
            let pinned = match self.pin(false, false).await {
                Pin::Nothing => return Ok(None),
                Pin::Pinned(pinned) => pinned,
                Pin::Invalid(reason, _) => return Err(reason.to_string()),
                Pin::Unavailable(reason) => {
                    return Err(format!("its saved state could not be read ({reason})"))
                }
            };
            pause_point(&self.dir, "fork_pinned").await;
            match fs::read(&pinned.path).await {
                Ok(data) if format!("{:x}", Sha256::digest(&data)) == pinned.meta.sha256 => {
                    return Ok(Some((pinned.meta, data)))
                }
                Ok(_) => return Err("Python snapshot digest mismatch".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("its saved state changed while it was copied".into())
    }
}

fn taint_digest(taint: &[Value]) -> Vec<u8> {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let ordered: std::collections::BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
                serde_json::to_value(ordered).expect("JSON object")
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            value => value.clone(),
        }
    }
    Sha256::digest(serde_json::to_vec(&canonical(&serde_json::json!(taint))).expect("JSON taint"))
        .to_vec()
}

/// The pinned generation's bytes, checked against the signed digest in one streaming pass.
async fn verify_digest(pinned: &PinnedState) -> Result<(), Check> {
    let digest = async {
        use tokio::io::AsyncReadExt;
        let mut file = fs::File::open(&pinned.path).await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok::<_, std::io::Error>(format!("{:x}", hasher.finalize()))
    }
    .await;
    match digest {
        Ok(digest) if digest == pinned.meta.sha256 => Ok(()),
        Ok(_) => Err(Check::Invalid(ResetReason(
            "Python snapshot digest mismatch".into(),
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Check::Unavailable(
            "its snapshot vanished while it was read".into(),
        )),
        Err(e) => Err(Check::Unavailable(format!(
            "its snapshot is unreadable: {e}"
        ))),
    }
}

/// Replaces the manifest in one rename. Where the platform cannot replace atomically, the
/// complete pending manifest stays until the rename, so the next start can finish it.
async fn install_manifest(pending: &Path, manifest: &Path) -> Result<(), String> {
    if manifest
        .parent()
        .is_some_and(|dir| interrupted(dir, "install"))
    {
        return Err("Cannot publish Python state metadata: injected failure".into());
    }
    crate::chat::trajectory_archive::with_trajectory_mutation_lock(fs::rename(pending, manifest))
        .await
        .map_err(|e| format!("Cannot publish Python state metadata: {e}"))?;
    if let Some(dir) = manifest.parent() {
        if let Err(e) = sync_dir(dir).await {
            tracing::warn!("Python state metadata may not be durable yet: {e}");
        }
    }
    Ok(())
}

/// A folder whose filesystem cannot sync folders is only warned about.
pub(crate) async fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let synced = if interrupted(dir, "dir_sync_einval") {
            Err(std::io::Error::from_raw_os_error(libc::EINVAL))
        } else if interrupted(dir, "dir_sync_eio") {
            Err(std::io::Error::from_raw_os_error(libc::EIO))
        } else {
            async { fs::File::open(dir).await?.sync_all().await }.await
        };
        if let Err(e) = synced {
            let unsupported = [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBADF];
            if !e
                .raw_os_error()
                .is_some_and(|code| unsupported.contains(&code))
            {
                return Err(e);
            }
            tracing::warn!("Python state folder cannot be synced: {e}");
        }
    }
    let _ = dir;
    Ok(())
}

async fn write_new_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(data).await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())
}

pub(crate) async fn artifact_path_checked(root: &Path, target: &Path) -> Result<(), String> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| "Artifact path is outside artifacts directory")?;
    if relative
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Artifact path contains invalid components".into());
    }
    let workspace = root.parent().ok_or("Invalid artifacts directory")?;
    let python = workspace.parent().ok_or("Invalid Python workspace")?;
    let mut paths = vec![
        python.to_path_buf(),
        workspace.to_path_buf(),
        root.to_path_buf(),
    ];
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        paths.push(current.clone());
    }
    let mut canonical_root = None;
    for path in paths {
        match fs::symlink_metadata(&path).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() || super::run_output::is_reparse_point(&meta) {
                    return Err(format!("Linked artifact path refused: {}", path.display()));
                }
                let canonical = fs::canonicalize(&path).await.map_err(|e| e.to_string())?;
                if path == root {
                    canonical_root = Some(canonical.clone());
                }
                if canonical_root
                    .as_ref()
                    .is_some_and(|root| !canonical.starts_with(root))
                {
                    return Err("Artifact path escapes artifacts directory".into());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                break
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub(crate) async fn is_own_artifact_path(app: &AppState, chat_id: &str, path: &Path) -> bool {
    if !is_python_workspace_path(path)
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return false;
    }
    let Ok(Some(workspace)) = PythonWorkspace::existing_for_chat(app, chat_id).await else {
        return false;
    };
    let root = workspace.artifacts_dir();
    path.is_absolute()
        && path != root
        && path.starts_with(&root)
        && artifact_path_checked(&root, path).await.is_ok()
}

pub async fn parent_artifacts_dir(app: &AppState, chat_id: &str) -> Option<PathBuf> {
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    if let Some(session) = &session {
        let session = session.lock().await;
        if !matches!(
            session.thread.link_type.as_deref(),
            Some("subagent" | "delegate" | "task_agent")
        ) && !session
            .thread
            .task_meta
            .as_ref()
            .is_some_and(|m| matches!(m.role.as_str(), "agent" | "agents"))
        {
            return None;
        }
    } else {
        return None;
    }
    let parent = match session {
        Some(s) => s.lock().await.thread.parent_id.clone(),
        None => crate::chat::trajectories::find_trajectory_parent_id(app.gcx.clone(), chat_id)
            .await
            .flatten(),
    }?;
    Some(
        PythonWorkspace::for_chat(app, &parent)
            .await
            .ok()?
            .artifacts_dir(),
    )
}

pub fn is_python_workspace_path(path: &Path) -> bool {
    let parts: Vec<_> = path.components().collect();
    parts.iter().enumerate().any(|(i, p)| {
        p.as_os_str() == "python"
            && i + 1 < parts.len()
            && parts[..i].iter().any(|p| {
                p.as_os_str() == "trajectories"
                    || p.as_os_str() == "conversations"
                    || p.as_os_str() == "tasks"
            })
    })
}

pub async fn delete_for_trajectory(path: &Path) -> Result<(), String> {
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid trajectory path")?;
    remove_dir(
        &path
            .parent()
            .ok_or("Invalid trajectory parent")?
            .join("python")
            .join(id),
    )
    .await
}

pub async fn enforce_project_quota(
    trajectories_root: &Path,
    quota_bytes: u64,
) -> Result<(), String> {
    static LAST: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    {
        let mut last = LAST
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|e| e.to_string())?;
        if last
            .get(trajectories_root)
            .is_some_and(|t| t.elapsed().as_secs() < 60)
        {
            return Ok(());
        }
        last.insert(trajectories_root.to_owned(), Instant::now());
    }
    // Only owned storage roots are discovery authority, never arbitrary project descendants.
    let roots = if trajectories_root
        .file_name()
        .is_some_and(|n| n == "trajectories" || n == "tasks")
    {
        vec![trajectories_root.to_owned()]
    } else if trajectories_root
        .file_name()
        .is_some_and(|n| n == ".refact")
    {
        vec![
            trajectories_root.join("trajectories"),
            trajectories_root.join("tasks"),
            trajectories_root.join("buddy/chats/conversations"),
        ]
    } else {
        vec![
            trajectories_root.join(".refact/trajectories"),
            trajectories_root.join(".refact/tasks"),
            trajectories_root.join(".refact/buddy/chats/conversations"),
            trajectories_root.join("trajectories"),
            trajectories_root.join("tasks"),
        ]
    };
    // Resolve symlinked ancestors once; components below an owned root must be real.
    let roots: Vec<(PathBuf, PathBuf)> = {
        let mut resolved = Vec::new();
        for root in roots {
            if let Some(owned) = canonical_owned_root(&root).await {
                resolved.push((owned.clone(), owned));
            }
        }
        resolved
    };
    let mut pending: Vec<_> = roots
        .into_iter()
        .map(|(owned, root)| (owned, root, 0usize))
        .collect();
    let mut workspaces = Vec::new();
    let mut visits = 0usize;
    while let Some((owned, dir, depth)) = pending.pop() {
        if depth > 16 || visits >= 100_000 || !real_directories_below(&owned, &dir).await {
            continue;
        }
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            visits += 1;
            if visits > 100_000 {
                break;
            }
            if !entry.file_type().await.map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let path = entry.path();
            if dir.file_name().is_some_and(|s| s == "python") {
                let id = entry.file_name();
                let valid_id = id.to_str().is_some_and(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                });
                if valid_id {
                    if !fs::symlink_metadata(path.join("runs"))
                        .await
                        .is_ok_and(|m| m.is_dir())
                    {
                        continue;
                    }
                    // A real runs directory in an owned tree is sufficient: crashed
                    // runs may never have published metadata at all.
                    workspaces.push(path);
                }
            } else {
                pending.push((owned.clone(), path, depth + 1));
            }
        }
    }
    // A non-terminal run belongs to a live engine unless it outlived the run limit by an hour.
    let stale_running_ms = refact_work_core::coordination_settings()
        .python_max_run_ms
        .saturating_add(3_600_000);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut total = 0u64;
    let mut runs = Vec::new();
    for workspace in workspaces {
        total = total.saturating_add(tree_size(&workspace).await?);
        for run in list(&workspace.join("runs"), usize::MAX).await? {
            let meta = fs::symlink_metadata(&run)
                .await
                .map_err(|e| e.to_string())?;
            if meta.is_dir() {
                let data = match read_run_metadata(&run.join(RUN_SUMMARY_FILE)).await {
                    Some(summary) => Some(summary),
                    None => read_run_metadata(&run.join("run.json")).await,
                };
                let finished = match data {
                    Some(v)
                        if matches!(
                            v["status"].as_str(),
                            Some(
                                "completed"
                                    | "failed"
                                    | "cancelled"
                                    | "timed_out"
                                    | "limit_exceeded"
                            )
                        ) =>
                    {
                        v["finished_at_ms"].as_u64()
                    }
                    Some(v) if v["status"].is_string() => {
                        let started = v["started_at_ms"].as_u64().or_else(|| {
                            meta.modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64)
                        });
                        // Treated as finished only once it can no longer be running.
                        started
                            .filter(|started| now.saturating_sub(*started) > stale_running_ms)
                            .map(|_| 0)
                    }
                    // Missing, malformed or oversized metadata gets the same grace
                    // period, based on the run directory's last modification.
                    _ => meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64),
                };
                if let Some(finished) = finished.filter(|t| now.saturating_sub(*t) > 600_000) {
                    runs.push((UNIX_EPOCH + std::time::Duration::from_millis(finished), run));
                }
            }
        }
    }
    runs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, run) in runs {
        if total <= quota_bytes {
            break;
        }
        let active: Vec<PathBuf> = ACTIVE_RUN_PATHS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        if active
            .iter()
            .any(|path| path == &run || std::fs::canonicalize(path).is_ok_and(|p| p == run))
        {
            continue;
        }
        let size = tree_size(&run).await?;
        remove_dir(&run).await?;
        total = total.saturating_sub(size);
    }
    Ok(())
}

async fn canonical_owned_root(root: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(root).await.ok()?;
    fs::symlink_metadata(&canonical)
        .await
        .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        .then_some(canonical)
}

async fn real_directories_below(owned: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(owned) else {
        return false;
    };
    let mut current = owned.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            match component {
                std::path::Component::Normal(part) => current.push(part),
                _ => return false,
            }
        }
        if !fs::symlink_metadata(&current)
            .await
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}

fn real_run_directory(path: &Path) -> bool {
    // The run directory sits at <workspace>/runs/<id>; the workspace and below must be real.
    let Some(workspace) = path.parent().and_then(Path::parent) else {
        return false;
    };
    let Ok(owned) = std::fs::canonicalize(workspace) else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(workspace) else {
        return false;
    };
    let mut current = owned;
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            match component {
                std::path::Component::Normal(part) => current.push(part),
                _ => return false,
            }
        }
        if !std::fs::symlink_metadata(&current)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}

/// The few fields of `run.json` the workspace view and pruning read, written after the terminal
/// `run.json` so they stay small however many calls and diffs the run has.
pub const RUN_SUMMARY_FILE: &str = "summary.json";

pub fn run_summary(meta: &Value) -> Value {
    let mut summary = serde_json::Map::new();
    for key in [
        "schema",
        "run_id",
        "status",
        "started_at_ms",
        "finished_at_ms",
        "background",
        "calls_total",
        "description",
        "code_preview",
        "displays_total",
        "engine_changes_total",
        "read_only",
    ] {
        if let Some(value) = meta.get(key) {
            summary.insert(key.into(), value.clone());
        }
    }
    if let Some(calls) = meta["llm"].get("calls") {
        summary.insert("llm".into(), serde_json::json!({ "calls": calls }));
    }
    Value::Object(summary)
}

fn run_metadata_json(
    run_id: &str,
    status: &str,
    started_at_ms: u64,
    background: bool,
    labels: &Value,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "schema": 1, "run_id": run_id, "status": status,
        "started_at_ms": started_at_ms, "background": background, "calls_total": 0,
    });
    if let (Some(value), Some(labels)) = (value.as_object_mut(), labels.as_object()) {
        value.extend(labels.clone());
    }
    if status == "failed" {
        value["finished_at_ms"] = serde_json::json!(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64);
        value["error"] = serde_json::json!({"type":"RunAbandoned", "message":"Python run exited before terminal metadata was persisted"});
    }
    value
}

/// Publishes non-terminal metadata so other engines never prune a live run.
pub async fn write_running_run_json(
    run_dir: &Path,
    run_id: &str,
    started_at_ms: u64,
    background: bool,
    labels: &Value,
) -> Result<(), String> {
    atomic_write(
        &run_dir.join("run.json"),
        run_metadata_json(run_id, "running", started_at_ms, background, labels)
            .to_string()
            .as_bytes(),
    )
    .await
}

async fn read_run_metadata(path: &Path) -> Option<Value> {
    use tokio::io::AsyncReadExt;
    let meta = fs::symlink_metadata(path).await.ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .await
        .ok()?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub(crate) async fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
    let result = async {
        use tokio::io::AsyncWriteExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(data).await.map_err(|e| e.to_string())?;
        file.sync_all().await.map_err(|e| e.to_string())?;
        drop(file);
        crate::chat::trajectories::atomic_write_file(&tmp, path).await
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(tmp).await;
    }
    result
}
async fn remove_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
async fn remove_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
async fn list(path: &Path, cap: usize) -> Result<Vec<PathBuf>, String> {
    let mut entries = match fs::read_dir(path).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut paths = Vec::new();
    while let Some(e) = entries.next_entry().await.map_err(|e| e.to_string())? {
        paths.push(e.path());
    }
    paths.sort();
    paths.truncate(cap);
    Ok(paths)
}
async fn tree_size(root: &Path) -> Result<u64, String> {
    let mut pending = vec![root.to_owned()];
    let mut size = 0u64;
    while let Some(path) = pending.pop() {
        let meta = match fs::symlink_metadata(&path).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if meta.is_file() {
            size = size.saturating_add(meta.len());
        } else if meta.is_dir() {
            pending.extend(list(&path, usize::MAX).await?);
        }
    }
    Ok(size)
}
/// Copies the regular files of `src`; returns how many `skip` left out, by relative path.
async fn copy_tree(
    src: &Path,
    dst: &Path,
    skip: impl Fn(&Path, &std::fs::Metadata) -> bool,
) -> Result<usize, String> {
    if fs::symlink_metadata(src)
        .await
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err("Symlinked artifacts root refused".into());
    }
    let mut skipped = 0;
    let mut pending = vec![(src.to_owned(), dst.to_owned(), PathBuf::new())];
    while let Some((src, dst, relative)) = pending.pop() {
        fs::create_dir_all(&dst).await.map_err(|e| e.to_string())?;
        for path in list(&src, usize::MAX).await? {
            let name = path.file_name().ok_or("Invalid artifact path")?;
            let target = dst.join(name);
            let relative = relative.join(name);
            let meta = fs::symlink_metadata(&path)
                .await
                .map_err(|e| e.to_string())?;
            if meta.is_dir() {
                pending.push((path, target, relative));
            } else if meta.is_file() {
                if skip(&relative, &meta) {
                    skipped += 1;
                    continue;
                }
                atomic_write(&target, &fs::read(path).await.map_err(|e| e.to_string())?).await?;
            }
        }
    }
    Ok(skipped)
}

/// Records a failed terminal run when execution exits before normal persistence.
pub struct FailedRunGuard {
    path: PathBuf,
    run_id: String,
    started_at_ms: u64,
    background: bool,
    labels: Value,
    armed: bool,
}
impl FailedRunGuard {
    pub fn new(path: PathBuf, run_id: String, started_at_ms: u64, background: bool) -> Self {
        Self {
            path,
            run_id,
            started_at_ms,
            background,
            labels: Value::Null,
            armed: true,
        }
    }
    pub fn with_labels(mut self, labels: Value) -> Self {
        self.labels = labels;
        self
    }
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for FailedRunGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Drop also runs for cancelled futures. Do not spawn async cleanup that
        // can itself be cancelled or lost while the runtime is shutting down.
        let metadata = run_metadata_json(
            &self.run_id,
            "failed",
            self.started_at_ms,
            self.background,
            &self.labels,
        );
        let result = (|| -> std::io::Result<()> {
            use std::io::Write;
            if !real_run_directory(&self.path) {
                return Ok(());
            }
            let target = self.path.join("run.json");
            let tmp = self.path.join(format!("run.tmp.{}", uuid::Uuid::new_v4()));
            let write = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&tmp)?;
                file.write_all(metadata.to_string().as_bytes())?;
                file.sync_all()?;
                drop(file);
                // Replace only "running" metadata; terminal metadata published by the
                // normal path wins if cancellation lands right after that write.
                loop {
                    match std::fs::hard_link(&tmp, &target) {
                        Ok(()) => return Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                            let running = std::fs::read(&target)
                                .ok()
                                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                                .is_some_and(|v| v["status"] == "running");
                            if !running {
                                return Ok(());
                            }
                            std::fs::remove_file(&target)?;
                        }
                        Err(e) => return Err(e),
                    }
                }
            })();
            let _ = std::fs::remove_file(tmp);
            write
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "Failed to persist abandoned Python run metadata");
        }
    }
}

/// Run directories of this engine whose runs have not ended, with the time each began.
static ACTIVE_RUN_PATHS: OnceLock<Mutex<HashMap<PathBuf, SystemTime>>> = OnceLock::new();

/// When the earliest run below `runs` that has not ended began.
fn active_run_start(runs: &Path) -> Option<SystemTime> {
    ACTIVE_RUN_PATHS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, _)| path.starts_with(runs))
        .map(|(_, began)| *began)
        .min()
}

#[cfg(test)]
mod artifacts_tests {
    use super::*;

    #[tokio::test]
    async fn artifacts_lookup_does_not_create_and_ownership_is_exact() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().into()];
        let app = AppState::from_gcx(gcx).await;
        assert!(PythonWorkspace::existing_for_chat(&app, "missing")
            .await
            .unwrap()
            .is_none());
        assert!(!temp.path().join(".refact").exists());
        let dir = temp.path().join(".refact/trajectories");
        fs::create_dir_all(&dir).await.unwrap();
        for id in ["own", "other"] {
            fs::write(
                dir.join(format!("{id}.json")),
                format!(r#"{{"id":"{id}","messages":[]}}"#),
            )
            .await
            .unwrap();
        }
        let own = PythonWorkspace::existing_for_chat(&app, "own")
            .await
            .unwrap()
            .unwrap();
        assert!(!own.dir().exists());
        assert!(is_own_artifact_path(&app, "own", &own.artifacts_dir().join("new/file")).await);
        assert!(!is_own_artifact_path(&app, "other", &own.artifacts_dir().join("new/file")).await);
        assert!(!is_own_artifact_path(&app, "own", &temp.path().join("random")).await);
        own.ensure_dirs().await.unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path(), own.artifacts_dir().join("escape")).unwrap();
            assert!(
                !is_own_artifact_path(&app, "own", &own.artifacts_dir().join("escape/file")).await
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn artifacts_symlinked_project_parent_is_supported() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir_all(real.join("project/python/chat/artifacts"))
            .await
            .unwrap();
        std::os::unix::fs::symlink(&real, temp.path().join("alias")).unwrap();
        let root = temp.path().join("alias/project/python/chat/artifacts");
        fs::write(root.join("file"), "ok").await.unwrap();
        artifact_path_checked(&root, &root.join("file"))
            .await
            .unwrap();
        artifact_path_checked(&root, &root.join("missing/file"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn artifacts_unsaved_buddy_session_lookup_matches_creating_resolver() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("unsaved-buddy".into());
        session.thread.buddy_meta = Some(crate::chat::types::BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "investigation".into(),
            workflow_id: None,
            source_chat_id: None,
            source_project: None,
            source_created_at: None,
        });
        app.chat.sessions.write().await.insert(
            "unsaved-buddy".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
        let existing = PythonWorkspace::existing_for_chat(&app, "unsaved-buddy")
            .await
            .unwrap()
            .unwrap();
        assert!(!temp.path().join(".refact").exists());
        let creating = PythonWorkspace::for_chat(&app, "unsaved-buddy")
            .await
            .unwrap();
        assert_eq!(existing.dir(), creating.dir());
        assert_eq!(
            existing.dir(),
            root.join(".refact/buddy/chats/conversations/python/unsaved-buddy")
        );
    }

    #[tokio::test]
    async fn artifacts_unsaved_task_session_lookup_matches_creating_resolver() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().into()];
        let task = temp.path().join(".refact/tasks/task");
        fs::create_dir_all(&task).await.unwrap();
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("unsaved-task".into());
        session.thread.task_meta = Some(crate::chat::types::TaskMeta {
            task_id: "task".into(),
            role: "agents".into(),
            agent_id: Some("worker".into()),
            card_id: None,
            planner_chat_id: None,
        });
        app.chat.sessions.write().await.insert(
            "unsaved-task".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
        let existing = PythonWorkspace::existing_for_chat(&app, "unsaved-task")
            .await
            .unwrap()
            .unwrap();
        assert!(!task.join("trajectories").exists());
        let creating = PythonWorkspace::for_chat(&app, "unsaved-task")
            .await
            .unwrap();
        assert_eq!(existing.dir(), creating.dir());
        assert_eq!(
            existing.dir(),
            root.join(".refact/tasks/task/trajectories/agents/worker/python/unsaved-task")
        );
    }

    #[tokio::test]
    async fn artifacts_unsaved_session_lookup_matches_creating_resolver() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("unsaved-child".into());
        session.thread.root_chat_id = Some("unsaved-root".into());
        app.chat.sessions.write().await.insert(
            "unsaved-child".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
        let existing = PythonWorkspace::existing_for_chat(&app, "unsaved-child")
            .await
            .unwrap()
            .unwrap();
        assert!(!temp.path().join(".refact").exists());
        let creating = PythonWorkspace::for_chat(&app, "unsaved-child")
            .await
            .unwrap();
        assert_eq!(existing.dir(), creating.dir());
        assert_eq!(
            existing.dir(),
            root.join(".refact/trajectories/unsaved-root/python/unsaved-child")
        );
    }
}

pub struct ActiveRunPath(PathBuf);
impl ActiveRunPath {
    pub fn new(path: PathBuf) -> Self {
        ACTIVE_RUN_PATHS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(path.clone(), SystemTime::now());
        Self(path)
    }
}
impl Drop for ActiveRunPath {
    fn drop(&mut self) {
        ACTIVE_RUN_PATHS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}
