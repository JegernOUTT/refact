use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::daemon::state::{now_ms, DaemonState};
use crate::daemon::supervisor::WorkerInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default = "default_true")]
    pub ast: bool,
    #[serde(default = "default_true")]
    pub vecdb: bool,
    #[serde(default = "default_ast_max_files")]
    pub ast_max_files: usize,
    #[serde(default = "default_vecdb_max_files")]
    pub vecdb_max_files: usize,
}

fn default_true() -> bool {
    true
}
fn default_ast_max_files() -> usize {
    50000
}
fn default_vecdb_max_files() -> usize {
    15000
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            ast: true,
            vecdb: true,
            ast_max_files: 50000,
            vecdb_max_files: 15000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub id: String,
    pub slug: String,
    pub root: PathBuf,
    pub pinned: bool,
    pub last_active_ms: u64,
    pub settings: ProjectSettings,
}

pub struct ProjectRegistry {
    entries: HashMap<String, ProjectEntry>,
    path: PathBuf,
}

impl ProjectRegistry {
    pub fn empty(path: PathBuf) -> Self {
        Self {
            entries: HashMap::new(),
            path,
        }
    }

    pub async fn load(path: PathBuf) -> Self {
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => match serde_json::from_str::<HashMap<String, ProjectEntry>>(&content) {
                Ok(entries) => return Self { entries, path },
                Err(error) => {
                    tracing::error!("failed to parse {}: {error}", path.display());
                    let bad = path.with_extension("json.bad");
                    let _ = tokio::fs::rename(&path, &bad).await;
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::error!("failed to read {}: {error}", path.display());
            }
        }
        Self {
            entries: HashMap::new(),
            path,
        }
    }

    async fn save_entries(&self, entries: &HashMap<String, ProjectEntry>) -> Result<(), String> {
        crate::daemon::state::write_daemon_state_json_atomic(&self.path, entries, "projects").await
    }

    pub async fn open(&mut self, root: PathBuf) -> Result<ProjectEntry, String> {
        self.open_with_settings(root, None).await
    }

    pub async fn open_with_settings(
        &mut self,
        root: PathBuf,
        settings: Option<ProjectSettings>,
    ) -> Result<ProjectEntry, String> {
        let id = self
            .entry_id_for_root(&root)
            .unwrap_or_else(|| project_id(&root));
        let now = now_ms();
        if let Some(existing) = self.entries.get(&id) {
            if existing.root != root {
                return Err("project id collision detected".to_string());
            }
            let mut entries = self.entries.clone();
            let entry = entries
                .get_mut(&id)
                .expect("existing project entry missing");
            entry.last_active_ms = now;
            if let Some(settings) = settings {
                entry.settings = settings;
            }
            let entry = entry.clone();
            self.save_entries(&entries).await?;
            self.entries = entries;
            return Ok(entry);
        }
        let slug = make_slug(&root, self.entries.values().map(|e| e.slug.as_str()));
        let entry = ProjectEntry {
            id: id.clone(),
            slug,
            root,
            pinned: false,
            last_active_ms: now,
            settings: settings.unwrap_or_default(),
        };
        let mut entries = self.entries.clone();
        entries.insert(id.clone(), entry.clone());
        self.save_entries(&entries).await?;
        self.entries = entries;
        Ok(entry)
    }

    pub fn list(&self) -> Vec<ProjectEntry> {
        self.entries.values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<&ProjectEntry> {
        self.entries.get(id)
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub async fn forget(&mut self, id: &str) -> Result<bool, String> {
        if self.entries.contains_key(id) {
            let mut entries = self.entries.clone();
            entries.remove(id);
            self.save_entries(&entries).await?;
            self.entries = entries;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn set_pinned(
        &mut self,
        id: &str,
        pinned: bool,
    ) -> Result<Option<ProjectEntry>, String> {
        if self.entries.contains_key(id) {
            let mut entries = self.entries.clone();
            let entry = entries.get_mut(id).expect("existing project entry missing");
            entry.pinned = pinned;
            let entry = entry.clone();
            self.save_entries(&entries).await?;
            self.entries = entries;
            return Ok(Some(entry));
        }
        Ok(None)
    }

    fn entry_id_for_root(&self, root: &Path) -> Option<String> {
        self.entries
            .iter()
            .find_map(|(id, entry)| (entry.root == root).then(|| id.clone()))
    }
}

fn project_id(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

fn make_slug<'a>(root: &Path, existing: impl Iterator<Item = &'a str>) -> String {
    let existing: std::collections::HashSet<String> = existing.map(|s| s.to_string()).collect();
    let raw = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_lowercase();
    let base: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() {
        "project".to_string()
    } else {
        base
    };
    if !existing.contains(&base) {
        return base;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn canonicalize_root(root_str: &str) -> Result<PathBuf, String> {
    let path = crate::files_correction::canonical_path(root_str.to_string());
    if !path.exists() {
        return Err(format!("path does not exist: {root_str}"));
    }
    if !path.is_dir() {
        return Err(format!("path is not a directory: {}", path.display()));
    }
    Ok(path)
}

#[derive(Debug, Serialize)]
struct OpenResponse {
    project_id: String,
    slug: String,
    root: PathBuf,
    pinned: bool,
    worker: Option<WorkerInfo>,
    cron_pending: Option<u64>,
}

#[derive(Deserialize)]
pub struct OpenRequest {
    root: String,
    #[serde(default)]
    pub client_kind: Option<String>,
    #[serde(default)]
    pub settings: Option<ProjectSettings>,
}

#[derive(Deserialize)]
pub struct PinRequest {
    pub pinned: bool,
}

pub async fn open_project(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<OpenRequest>,
) -> impl IntoResponse {
    let root = match canonicalize_root(&request.root) {
        Ok(p) => p,
        Err(message) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response();
        }
    };
    let entry = {
        let mut registry = state.projects.write().await;
        match registry
            .open_with_settings(root, request.settings.clone())
            .await
        {
            Ok(entry) => entry,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": error})),
                )
                    .into_response();
            }
        }
    };
    state.sync_project_liveness(&entry).await;
    let worker = match state.supervisor.ensure_ready_worker(&entry).await {
        Ok(worker) => worker,
        Err(error) => {
            return (StatusCode::BAD_GATEWAY, Json(json!({"error": error}))).into_response();
        }
    };
    let cron_pending = state.cron_pending(&entry.id).await;
    let _ = state
        .events
        .emit(
            "project_opened",
            Some(entry.id.clone()),
            json!({"root": entry.root.to_string_lossy()}),
        )
        .await;
    Json(OpenResponse {
        project_id: entry.id,
        slug: entry.slug,
        root: entry.root,
        pinned: entry.pinned,
        worker: Some(worker),
        cron_pending,
    })
    .into_response()
}

pub async fn list_projects(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<Vec<ProjectEntry>> {
    let registry = state.projects.read().await;
    Json(registry.list())
}

pub async fn get_project(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    let registry = state.projects.read().await;
    match registry.get(&id) {
        Some(entry) => Json(entry.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
    }
}

pub async fn restart_project_worker(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    let entry = {
        let registry = state.projects.read().await;
        match registry.get(&id) {
            Some(entry) => entry.clone(),
            None => {
                return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
                    .into_response();
            }
        }
    };
    match state.supervisor.restart_worker(&entry).await {
        Ok(worker) => Json(worker).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

pub async fn stop_project_worker(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    {
        let registry = state.projects.read().await;
        if registry.get(&id).is_none() {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
        }
    }
    match state.supervisor.stop_worker(&id).await {
        Ok(Some(worker)) => Json(worker).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "worker not found"})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

pub async fn forget_project(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    {
        let registry = state.projects.read().await;
        if registry.get(&id).is_none() {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
        }
    }
    if let Err(error) = state.supervisor.stop_worker(&id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response();
    }
    let result = {
        let mut registry = state.projects.write().await;
        registry.forget(&id).await
    };
    match result {
        Ok(true) => {
            if let Err(error) = state.purge_project_runtime(&id).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": error})),
                )
                    .into_response();
            }
            let _ = state
                .events
                .emit("project_forgotten", Some(id), json!({}))
                .await;
            Json(json!({"success": true})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

pub async fn pin_project(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<PinRequest>,
) -> impl IntoResponse {
    let result = {
        let mut registry = state.projects.write().await;
        registry.set_pinned(&id, request.pinned).await
    };
    match result {
        Ok(Some(entry)) => {
            state.sync_project_liveness(&entry).await;
            let _ = state
                .events
                .emit(
                    "project_pinned",
                    Some(entry.id.clone()),
                    json!({"pinned": entry.pinned}),
                )
                .await;
            Json(entry).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn fake_worker() -> Option<Self> {
            let python = "python3";
            if std::process::Command::new(python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![
                (
                    "REFACT_DAEMON_WORKER_CMD",
                    std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                ),
                (
                    "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                ),
                ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
            ];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python, script.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::remove_var("FAKE_WORKER_CRASH");
            Some(Self { keys })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn make_registry(dir: &tempfile::TempDir) -> ProjectRegistry {
        ProjectRegistry::empty(dir.path().join("projects.json"))
    }

    #[tokio::test]
    async fn project_id_stable_across_dotdot_spelling() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("api");
        std::fs::create_dir_all(&sub).unwrap();

        let canon = dunce::simplified(&sub.canonicalize().unwrap()).to_path_buf();
        let id1 = project_id(&canon);

        let dotdot = sub.join("..").join("api");
        let canon2 = dunce::simplified(&dotdot.canonicalize().unwrap()).to_path_buf();
        let id2 = project_id(&canon2);

        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn project_id_uses_full_sha256_and_distinguishes_roots() {
        let dir = tempdir().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();

        let id_a = project_id(&root_a);
        let id_b = project_id(&root_b);

        assert_eq!(id_a.len(), 64);
        assert_eq!(id_b.len(), 64);
        assert_ne!(id_a, id_b);
    }

    #[tokio::test]
    async fn project_id_collision_is_rejected() {
        let dir = tempdir().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let id = project_id(&root_a);
        let mut reg = make_registry(&dir);
        reg.entries.insert(
            id.clone(),
            ProjectEntry {
                id,
                slug: "b".to_string(),
                root: root_b,
                pinned: false,
                last_active_ms: 0,
                settings: ProjectSettings::default(),
            },
        );

        let error = reg.open(root_a).await.unwrap_err();

        assert!(error.contains("collision"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_id_stable_via_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        symlink(&real, &link).unwrap();

        let id_real = project_id(&dunce::simplified(&real.canonicalize().unwrap()).to_path_buf());
        let id_link = project_id(&dunce::simplified(&link.canonicalize().unwrap()).to_path_buf());
        assert_eq!(id_real, id_link);
    }

    #[test]
    fn slug_dedup_adds_numeric_suffix() {
        let api1 = PathBuf::from("/tmp/api");
        let api2 = PathBuf::from("/tmp/other/api");

        let slug1 = make_slug(&api1, std::iter::empty());
        assert_eq!(slug1, "api");
        let slug2 = make_slug(&api2, std::iter::once(slug1.as_str()));
        assert_eq!(slug2, "api-2");
    }

    #[tokio::test]
    async fn persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();

        let path = dir.path().join("projects.json");
        {
            let mut reg = ProjectRegistry::empty(path.clone());
            reg.open(root.clone()).await.unwrap();
        }
        let reg2 = ProjectRegistry::load(path).await;
        let list = reg2.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].root, root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn persistence_sets_permissions_on_create_and_overwrite() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let path = dir.path().join("projects.json");
        let mut reg = ProjectRegistry::empty(path.clone());

        reg.open(root.clone()).await.unwrap();
        assert_eq!(file_mode(&path), 0o600);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        reg.open(root).await.unwrap();

        assert_eq!(file_mode(&path), 0o600);
    }

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn corrupt_file_recovery() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("projects.json");
        tokio::fs::write(&path, b"not valid json{{{").await.unwrap();

        let reg = ProjectRegistry::load(path.clone()).await;
        assert!(reg.list().is_empty());
        assert!(dir.path().join("projects.json.bad").exists());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn open_idempotency() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();

        let mut reg = make_registry(&dir);
        let e1 = reg.open(root.clone()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let e2 = reg.open(root.clone()).await.unwrap();

        assert_eq!(e1.id, e2.id);
        assert_eq!(e1.slug, e2.slug);
        assert!(e2.last_active_ms >= e1.last_active_ms);
        assert_eq!(reg.list().len(), 1);
    }

    #[tokio::test]
    async fn open_with_settings_overrides_existing_entry() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let settings = ProjectSettings {
            ast: false,
            vecdb: false,
            ast_max_files: 7,
            vecdb_max_files: 8,
        };

        let mut reg = make_registry(&dir);
        let original = reg.open(root.clone()).await.unwrap();
        assert_eq!(original.settings, ProjectSettings::default());
        let updated = reg
            .open_with_settings(root, Some(settings.clone()))
            .await
            .unwrap();

        assert_eq!(updated.id, original.id);
        assert_eq!(updated.settings, settings);
        assert_eq!(reg.list()[0].settings, settings);
    }

    #[tokio::test]
    async fn save_failure_leaves_registry_memory_unchanged() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();

        let mut new_reg = ProjectRegistry::empty(blocked.join("projects.json"));
        assert!(new_reg.open(root.clone()).await.is_err());
        assert!(new_reg.list().is_empty());

        let mut reg = make_registry(&dir);
        let entry = reg.open(root.clone()).await.unwrap();
        let original = reg.list();
        reg.path = blocked.join("projects.json");

        assert!(reg.set_pinned(&entry.id, true).await.is_err());
        assert_eq!(reg.list(), original);

        let settings = ProjectSettings {
            ast: false,
            vecdb: false,
            ast_max_files: 1,
            vecdb_max_files: 2,
        };
        assert!(reg.open_with_settings(root, Some(settings)).await.is_err());
        assert_eq!(reg.list(), original);

        assert!(reg.forget(&entry.id).await.is_err());
        assert_eq!(reg.list(), original);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn open_list_get_pin_forget_flow() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        use crate::daemon::{config::DaemonConfig, events::EventBus, state::DaemonState};
        use axum::body::Body;
        use hyper::{Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempdir().unwrap();
        let proj = dir.path().join("myproject");
        std::fs::create_dir_all(&proj).unwrap();

        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;

        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let body =
            serde_json::to_vec(&serde_json::json!({"root": proj.to_str().unwrap()})).unwrap();
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/projects/open")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
        let open_resp: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let project_id = open_resp["project_id"].as_str().unwrap().to_string();
        assert_eq!(open_resp["worker"]["state"], "ready");
        assert!(open_resp["worker"]["pid"].as_u64().is_some());

        let list_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/daemon/v1/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_resp.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(list_resp.into_body()).await.unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list.len(), 1);

        let get_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/daemon/v1/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        let pin_body = serde_json::to_vec(&serde_json::json!({"pinned": true})).unwrap();
        let pin_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/daemon/v1/projects/{project_id}/pin"))
                    .header("content-type", "application/json")
                    .body(Body::from(pin_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pin_resp.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(pin_resp.into_body()).await.unwrap();
        let pinned_entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(pinned_entry["pinned"], true);

        let restart_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/daemon/v1/projects/{project_id}/restart"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restart_resp.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(restart_resp.into_body())
            .await
            .unwrap();
        let restarted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restarted["state"], "ready");

        let stop_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/daemon/v1/projects/{project_id}/stop"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stop_resp.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(stop_resp.into_body()).await.unwrap();
        let stopped: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stopped["state"], "stopped");

        let forget_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/daemon/v1/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forget_resp.status(), StatusCode::OK);
        assert!(state.supervisor.worker_info(&project_id).await.is_none());

        let gone_resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/daemon/v1/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gone_resp.status(), StatusCode::NOT_FOUND);
        state.supervisor.stop_all().await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn forget_project_stops_running_worker() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        use crate::daemon::{config::DaemonConfig, events::EventBus, state::DaemonState};
        use axum::body::Body;
        use hyper::{Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempdir().unwrap();
        let project_root = dir.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        state.supervisor.ensure_worker(&entry).await.unwrap();
        assert!(state.supervisor.worker_info(&entry.id).await.is_some());
        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/daemon/v1/projects/{}", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.supervisor.worker_info(&entry.id).await.is_none());
        assert!(state.projects.read().await.get(&entry.id).is_none());
        state.supervisor.stop_all().await;
    }

    #[tokio::test]
    async fn open_missing_path_returns_400() {
        use crate::daemon::{config::DaemonConfig, events::EventBus, state::DaemonState};
        use axum::body::Body;
        use hyper::{Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let router = crate::daemon::server::make_router(state, 8488);

        let body =
            serde_json::to_vec(&serde_json::json!({"root": "/definitely/does/not/exist/at/all"}))
                .unwrap();
        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/projects/open")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
