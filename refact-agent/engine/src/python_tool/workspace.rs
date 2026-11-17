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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateMeta {
    pub schema: u32,
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
}

#[derive(Clone, Debug)]
pub struct VerifiedState {
    pub path: PathBuf,
    pub sha256: String,
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
            let parent = path.parent().unwrap_or(Path::new("."));
            let storage = if path.starts_with(&app.paths.config_dir) {
                app.paths.config_dir.clone()
            } else {
                parent
                    .ancestors()
                    .find(|p| {
                        p.file_name()
                            .is_some_and(|n| n == "trajectories" || n == "conversations")
                    })
                    .unwrap_or(parent)
                    .to_path_buf()
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
    pub fn state_path(&self) -> PathBuf {
        self.dir.join("state.monty")
    }
    pub async fn ensure_dirs(&self) -> Result<(), String> {
        for path in [self.dir.clone(), self.artifacts_dir(), self.runs_dir()] {
            crate::chat::trajectories::ensure_real_dir_tree(&path).await?;
        }
        Ok(())
    }
    pub async fn read_state_meta(&self) -> Option<StateMeta> {
        serde_json::from_slice(&fs::read(self.dir.join("state.json")).await.ok()?).ok()
    }
    pub async fn verify_state(&self) -> Result<VerifiedState, ResetReason> {
        let fail = |s: &str| ResetReason(s.to_owned());
        let meta = self
            .read_state_meta()
            .await
            .ok_or_else(|| fail("missing or invalid Python state metadata"))?;
        if meta.schema != 1 {
            return Err(fail("unsupported Python state schema"));
        }
        let key = self.key().await.map_err(ResetReason)?;
        if !snapshot::verify_with_taint(
            &key,
            &meta.sha256,
            &self.chat_id,
            &taint_digest(&meta.taint),
            &meta.hmac,
        ) {
            return Err(fail("Python state authentication failed"));
        }
        let data = fs::read(self.state_path())
            .await
            .map_err(|_| fail("Python snapshot is missing"))?;
        if data.len() as u64 != meta.bytes || format!("{:x}", Sha256::digest(&data)) != meta.sha256
        {
            return Err(fail("Python snapshot digest mismatch"));
        }
        Ok(VerifiedState {
            path: self.state_path(),
            sha256: meta.sha256,
        })
    }
    pub async fn record_snapshot(
        &self,
        sha256: String,
        bytes: u64,
        last_run_id: String,
        variables: Vec<VariableSummary>,
        taint: Vec<Value>,
        max_state_bytes: u64,
    ) -> Result<PersistOutcome, String> {
        if bytes > max_state_bytes {
            self.clear_state().await?;
            return Ok(PersistOutcome {
                persisted: false,
                bytes,
            });
        }
        self.ensure_dirs().await?;
        let data = fs::read(self.state_path())
            .await
            .map_err(|e| e.to_string())?;
        if data.len() as u64 != bytes || format!("{:x}", Sha256::digest(&data)) != sha256 {
            return Err("Snapshot bytes do not match worker digest".into());
        }
        let key = self.key().await?;
        let meta = StateMeta {
            schema: 1,
            dump_version: "monty-1.6.0".into(),
            saved_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            bytes,
            hmac: snapshot::sign_with_taint(&key, &sha256, &self.chat_id, &taint_digest(&taint)),
            sha256,
            last_run_id,
            variables,
            variables_truncated: false,
            variables_total: None,
            taint,
        };
        atomic_write(
            &self.dir.join("state.json"),
            &serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?,
        )
        .await?;
        if let Some((root, quota)) = &self.project_quota {
            enforce_project_quota(root, *quota).await?;
        }
        Ok(PersistOutcome {
            persisted: true,
            bytes,
        })
    }
    pub async fn set_variables_truncated(&self, truncated: bool) -> Result<(), String> {
        self.set_variables_info(truncated, None).await
    }
    pub async fn set_variables_info(
        &self,
        truncated: bool,
        total: Option<u64>,
    ) -> Result<(), String> {
        if let Some(mut meta) = self.read_state_meta().await {
            meta.variables_truncated = truncated;
            meta.variables_total = total;
            atomic_write(
                &self.dir.join("state.json"),
                &serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?,
            )
            .await?;
        }
        Ok(())
    }
    pub async fn clear_state(&self) -> Result<(), String> {
        for path in [self.state_path(), self.dir.join("state.json")] {
            remove_file(&path).await?;
        }
        Ok(())
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
    pub async fn fork(app: &AppState, src_chat_id: &str, dst_chat_id: &str) -> Result<(), String> {
        let src = Self::for_chat(app, src_chat_id).await?;
        let dst = Self::for_chat(app, dst_chat_id).await?;
        src.fork_to(&dst).await
    }
    pub(crate) async fn fork_to(&self, dst: &Self) -> Result<(), String> {
        if !fs::try_exists(&self.dir).await.map_err(|e| e.to_string())? {
            return Ok(());
        }
        dst.ensure_dirs().await?;
        if let Ok(verified) = self.verify_state().await {
            let mut meta = self
                .read_state_meta()
                .await
                .ok_or("Missing state metadata")?;
            atomic_write(
                &dst.state_path(),
                &fs::read(verified.path).await.map_err(|e| e.to_string())?,
            )
            .await?;
            meta.hmac = snapshot::sign_with_taint(
                &dst.key().await?,
                &meta.sha256,
                &dst.chat_id,
                &taint_digest(&meta.taint),
            );
            atomic_write(
                &dst.dir.join("state.json"),
                &serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?,
            )
            .await?;
        }
        let ledger = self.dir.join("privacy_ledger.json");
        if let Ok(data) = fs::read(ledger).await {
            let mut entries: Vec<(PathBuf, crate::privacy::records::DerivedZone)> =
                serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            for (path, _) in &mut entries {
                if let Ok(relative) = path.strip_prefix(self.artifacts_dir()) {
                    *path = dst.artifacts_dir().join(relative);
                }
            }
            atomic_write(
                &dst.dir.join("privacy_ledger.json"),
                &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
            )
            .await?;
        }
        copy_tree(&self.artifacts_dir(), &dst.artifacts_dir()).await
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
            .iter()
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
async fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::symlink_metadata(src)
        .await
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err("Symlinked artifacts root refused".into());
    }
    let mut pending = vec![(src.to_owned(), dst.to_owned())];
    while let Some((src, dst)) = pending.pop() {
        fs::create_dir_all(&dst).await.map_err(|e| e.to_string())?;
        for path in list(&src, usize::MAX).await? {
            let target = dst.join(path.file_name().ok_or("Invalid artifact path")?);
            let meta = fs::symlink_metadata(&path)
                .await
                .map_err(|e| e.to_string())?;
            if meta.is_dir() {
                pending.push((path, target));
            } else if meta.is_file() {
                atomic_write(&target, &fs::read(path).await.map_err(|e| e.to_string())?).await?;
            }
        }
    }
    Ok(())
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

static ACTIVE_RUN_PATHS: OnceLock<Mutex<std::collections::HashSet<PathBuf>>> = OnceLock::new();

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
            temp.path()
                .join(".refact/buddy/chats/conversations/python/unsaved-buddy")
        );
    }

    #[tokio::test]
    async fn artifacts_unsaved_task_session_lookup_matches_creating_resolver() {
        let temp = tempfile::tempdir().unwrap();
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
            task.join("trajectories/agents/worker/python/unsaved-task")
        );
    }

    #[tokio::test]
    async fn artifacts_unsaved_session_lookup_matches_creating_resolver() {
        let temp = tempfile::tempdir().unwrap();
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
            temp.path()
                .join(".refact/trajectories/unsaved-root/python/unsaved-child")
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
            .insert(path.clone());
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
