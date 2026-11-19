use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::registry::SessionStatus;
use super::workspace::PythonWorkspace;
use crate::app_state::AppState;

#[derive(Debug, Serialize)]
pub struct WorkspaceView {
    pub schema: u32,
    pub chat_id: String,
    pub exists: bool,
    pub workspace_dir: String,
    pub artifacts_dir: String,
    pub session: SessionView,
    pub artifacts: Vec<ArtifactView>,
    pub artifacts_truncated: bool,
    pub runs: Vec<RunView>,
    pub runs_truncated: bool,
    /// Newest displays across the listed runs, newest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<WorkspaceDisplay>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub displays_truncated: bool,
    pub disk: DiskView,
}
#[derive(Debug, Serialize)]
pub struct WorkspaceDisplay {
    #[serde(flatten)]
    pub display: refact_chat_api::PythonDisplay,
    pub run_id: String,
    pub started_at_ms: u64,
}
#[derive(Debug, Default, Serialize)]
pub struct SessionView {
    pub status: String,
    pub live: bool,
    pub state_persisted: bool,
    pub state_bytes: u64,
    pub saved_at_ms: Option<u64>,
    pub last_run_id: Option<String>,
    pub reset_reason: Option<String>,
    pub variables: Vec<VariableView>,
    pub variables_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables_total: Option<u64>,
    /// The variables could not be listed; an empty list then says nothing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub variables_unknown: bool,
    pub last_run_restored: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct VariableView {
    pub name: String,
    pub type_name: String,
    pub preview: String,
}
#[derive(Debug, Serialize)]
pub struct ArtifactView {
    pub path: String,
    pub rel_path: String,
    pub bytes: u64,
    pub modified_at_ms: u64,
}
#[derive(Debug, Serialize)]
pub struct RunView {
    pub run_id: String,
    pub status: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub calls_total: u64,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_preview: Option<String>,
    /// Model calls among `calls_total`; omitted when zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displays_total: Option<u64>,
    /// Applied engine changes; omitted when zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_changes: Option<u64>,
    /// Scripts of this run could write only under ARTIFACTS_DIR; omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}
#[derive(Debug, Default, Serialize)]
pub struct DiskView {
    pub workspace_bytes: u64,
    pub quota_bytes: u64,
}

pub async fn workspace_view(app: &AppState, chat_id: &str) -> Result<WorkspaceView, String> {
    let workspace = PythonWorkspace::for_chat(app, chat_id).await?;
    let mut quota = 0;
    for root in crate::files_correction::get_project_dirs(app.gcx.clone()).await {
        if workspace.dir().starts_with(&root) {
            quota = crate::runtime_settings::coordination_for_project(&root)
                .await?
                .python_disk_quota_bytes;
            break;
        }
    }
    let status = app.python_tool.registry.session_status(chat_id);
    let reset_reason = app.python_tool.registry.reset_reason(chat_id).await;
    let last_run_restored = app.python_tool.registry.last_run_restored(chat_id);
    let active = app.python_tool.active_run_ids(chat_id);
    let live_counts = if let Some(session) = app.chat.sessions.read().await.get(chat_id).cloned() {
        session
            .lock()
            .await
            .runtime
            .python_runs
            .iter()
            .map(|run| {
                (
                    run.run_id.clone(),
                    run.calls
                        .iter()
                        .map(|call| u64::from(call.seq))
                        .max()
                        .unwrap_or(0),
                )
            })
            .collect::<std::collections::HashMap<_, _>>()
    } else {
        Default::default()
    };
    let chat_id = chat_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut view = collect(&workspace, &chat_id, quota, status, &active);
        apply_live_counts(&mut view, &live_counts);
        view.session.reset_reason = reset_reason;
        view.session.last_run_restored = last_run_restored;
        view
    })
    .await
    .map_err(|e| e.to_string())
}

struct Storage {
    root: PathBuf,
    real: PathBuf,
}
impl Storage {
    fn resolve(workspace: &PythonWorkspace) -> Option<Self> {
        let root = workspace.storage_root();
        let real = std::fs::canonicalize(&root).ok()?;
        Some(Self {
            root,
            real,
        })
    }
    /// The path below the checked storage base that `path`, as the workspace spells it, stands
    /// for; every component below the base is checked not to be a link. Reads use this path, so
    /// a storage alias changed after the check is never followed.
    fn real(&self, path: &Path) -> Option<PathBuf> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let mut current = self.real.clone();
        for component in relative.components() {
            let std::path::Component::Normal(part) = component else {
                return None;
            };
            current.push(part);
            let metadata = std::fs::symlink_metadata(&current).ok()?;
            if metadata.file_type().is_symlink() || super::run_output::is_reparse_point(&metadata) {
                return None;
            }
        }
        Some(current)
    }
}
fn read_json(storage: &Storage, path: &Path) -> Option<Value> {
    read_json_bounded(storage, path, 4 * 1024 * 1024)
}
fn read_json_bounded(storage: &Storage, path: &Path, limit: u64) -> Option<Value> {
    read_json_file(&storage.real(path)?, limit)
}
/// Reads a file found below the checked base; a link there is not a file and is not read.
fn read_json_file(real: &Path, limit: u64) -> Option<Value> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(real).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(real)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn children(path: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(10001)
        .map(|e| e.path())
        .collect()
}
fn collect(
    workspace: &PythonWorkspace,
    chat_id: &str,
    quota: u64,
    status: SessionStatus,
    active: &[String],
) -> WorkspaceView {
    collect_in(
        Storage::resolve(workspace),
        workspace,
        chat_id,
        quota,
        status,
        active,
    )
}
/// Every read, listing and walk goes through `storage`, the base checked once.
fn collect_in(
    storage: Option<Storage>,
    workspace: &PythonWorkspace,
    chat_id: &str,
    quota: u64,
    status: SessionStatus,
    active: &[String],
) -> WorkspaceView {
    let real_dir = storage
        .as_ref()
        .and_then(|storage| storage.real(workspace.dir()))
        .filter(|dir| dir.is_dir());
    let exists = real_dir.is_some();
    let mut view = WorkspaceView {
        schema: 1,
        chat_id: chat_id.into(),
        exists,
        workspace_dir: workspace.dir().to_string_lossy().into(),
        artifacts_dir: workspace.artifacts_dir().to_string_lossy().into(),
        session: SessionView {
            status: "none".into(),
            ..Default::default()
        },
        artifacts: vec![],
        artifacts_truncated: false,
        runs: vec![],
        runs_truncated: false,
        displays: vec![],
        displays_truncated: false,
        disk: DiskView::default(),
    };
    let (Some(storage), Some(real_dir)) = (storage, real_dir) else {
        return view;
    };
    view.disk.quota_bytes = quota;
    view.session.live = status.live;
    view.session.status = if status.background {
        "background"
    } else if status.running || !active.is_empty() {
        "running"
    } else {
        "idle"
    }
    .into();
    if let Some(meta) = read_json(&storage, &workspace.dir().join("state.json")) {
        view.session.state_persisted = true;
        view.session.state_bytes = meta["bytes"].as_u64().unwrap_or_default();
        view.session.saved_at_ms = meta["saved_at_ms"].as_u64();
        view.session.last_run_id = meta["last_run_id"].as_str().map(str::to_owned);
        view.session.variables_unknown = meta["variables_unknown"] == true;

        if let Some(vars) = meta["variables"].as_array() {
            view.session.variables_truncated =
                vars.len() > 200 || meta["variables_truncated"] == true;
            view.session.variables_total = meta["variables_total"]
                .as_u64()
                .or_else(|| (vars.len() > 200).then_some(vars.len() as u64));
            view.session.variables = vars
                .iter()
                .take(200)
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
        }
    }
    let artifacts = workspace.artifacts_dir();
    let mut pending = vec![(workspace.dir().to_owned(), real_dir, 0usize)];
    let mut visited = 0usize;
    while let Some((path, real, depth)) = pending.pop() {
        visited += 1;
        if visited > 10000 || depth > 16 {
            view.artifacts_truncated = true;
            view.runs_truncated = true;
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&real) else {
            continue;
        };
        if meta.is_dir() {
            for child in children(&real) {
                if pending.len() + visited >= 10000 {
                    view.artifacts_truncated = true;
                    view.runs_truncated = true;
                    break;
                }
                if let Some(name) = child.file_name() {
                    pending.push((path.join(name), child.clone(), depth + 1));
                }
            }
        } else if meta.is_file() {
            view.disk.workspace_bytes = view.disk.workspace_bytes.saturating_add(meta.len());
            if let Ok(rel) = path.strip_prefix(&artifacts) {
                view.artifacts.push(ArtifactView {
                    path: path.to_string_lossy().into(),
                    rel_path: rel.to_string_lossy().replace('\\', "/"),
                    bytes: meta.len(),
                    modified_at_ms: modified(&meta),
                });
            }
        }
    }
    view.artifacts.sort_by(|a, b| {
        b.modified_at_ms
            .cmp(&a.modified_at_ms)
            .then(a.rel_path.cmp(&b.rel_path))
    });
    view.artifacts_truncated |= view.artifacts.len() > 500;
    view.artifacts.truncate(500);
    if let Some(runs_dir) = storage.real(&workspace.runs_dir()) {
        // Select using directory metadata, before opening any run.json. At
        // most 500 bounded metadata payloads are decoded per view request.
        let mut directories = std::collections::BinaryHeap::new();
        if let Ok(entries) = std::fs::read_dir(&runs_dir) {
            for entry in entries.filter_map(Result::ok) {
                let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                directories.push(std::cmp::Reverse((modified(&meta), entry.path())));
                if directories.len() > 500 {
                    directories.pop();
                    view.runs_truncated = true;
                }
            }
        }
        for std::cmp::Reverse((_, path)) in directories {
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_dir() {
                continue;
            }
            let run_id = path.file_name().unwrap().to_string_lossy().to_string();
            let data = read_json_file(&path.join(super::workspace::RUN_SUMMARY_FILE), 64 * 1024)
                .or_else(|| read_json_file(&path.join("run.json"), 64 * 1024));
            let valid = data.as_ref().filter(|d| d["status"].is_string());
            view.runs.push(RunView {
                status: valid
                    .and_then(|d| d["status"].as_str())
                    .unwrap_or(if active.contains(&run_id) {
                        "running"
                    } else {
                        "failed"
                    })
                    .into(),
                started_at_ms: valid
                    .and_then(|d| d["started_at_ms"].as_u64())
                    .unwrap_or_else(|| modified(&meta)),
                finished_at_ms: valid.and_then(|d| d["finished_at_ms"].as_u64()),
                calls_total: valid
                    .and_then(|d| d["calls_total"].as_u64())
                    .unwrap_or_default(),
                background: valid
                    .and_then(|d| d["background"].as_bool())
                    .unwrap_or(false)
                    || (status.background && active.contains(&run_id)),
                description: valid
                    .and_then(|d| d["description"].as_str())
                    .map(Into::into),
                code_preview: valid
                    .and_then(|d| d["code_preview"].as_str())
                    .map(Into::into),
                llm_calls: valid
                    .and_then(|d| d["llm"]["calls"].as_u64())
                    .filter(|n| *n > 0),
                displays_total: valid
                    .and_then(|d| d["displays_total"].as_u64())
                    .filter(|n| *n > 0),
                engine_changes: valid
                    .and_then(|d| d["engine_changes_total"].as_u64())
                    .filter(|n| *n > 0),
                read_only: valid
                    .and_then(|d| d["read_only"].as_bool())
                    .filter(|read_only| *read_only),
                run_id,
            });
        }
    }
    view.runs.sort_by(|a, b| {
        b.started_at_ms
            .cmp(&a.started_at_ms)
            .then(a.run_id.cmp(&b.run_id))
    });
    view.runs_truncated |= view.runs.len() > 500;
    view.runs.truncate(500);
    collect_displays(workspace, &storage, &mut view);
    view
}

const WORKSPACE_DISPLAYS: usize = 8;
const DISPLAY_FILE_LIMIT: u64 = 16 * 1024 * 1024;

fn collect_displays(workspace: &PythonWorkspace, storage: &Storage, view: &mut WorkspaceView) {
    for run in &view.runs {
        let dir = workspace.run_dir(&run.run_id).join("displays");
        let Some(real_dir) = storage.real(&dir) else {
            continue;
        };
        let mut files: Vec<(u32, std::path::PathBuf)> = children(&real_dir)
            .into_iter()
            .filter_map(|path| {
                let index = path.file_stem()?.to_str()?.parse::<u32>().ok()?;
                (path.extension()? == "json").then_some((index, path))
            })
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        for (index, path) in files {
            let Some((stored, display)) = read_json_file(&path, DISPLAY_FILE_LIMIT)
                .as_ref()
                .and_then(super::display::from_file)
            else {
                continue;
            };
            let Some(path) = path.file_name().map(|name| dir.join(name)) else {
                continue;
            };
            if stored != index {
                continue;
            }
            if view.displays.len() == WORKSPACE_DISPLAYS {
                view.displays_truncated = true;
                return;
            }
            view.displays.push(WorkspaceDisplay {
                display: super::display::inline(&display, index, &path),
                run_id: run.run_id.clone(),
                started_at_ms: run.started_at_ms,
            });
        }
    }
}

#[cfg(test)]
#[path = "workspace_view_tests.rs"]
mod tests;

#[cfg(test)]
mod bounded_run_tests {
    use super::*;

    #[test]
    fn newest_directories_are_selected_before_reading_run_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join("chat.json"),
            "chat",
            temp.path().join("key"),
        );
        for n in 0..501 {
            let dir = ws.run_dir(&format!("r{n:03}"));
            std::fs::create_dir_all(&dir).unwrap();
            // Old directory metadata must not win selection through a forged start time.
            std::fs::write(
                dir.join("run.json"),
                serde_json::json!({"status":"completed","started_at_ms":501-n}).to_string(),
            )
            .unwrap();
            filetime::set_file_mtime(&dir, filetime::FileTime::from_unix_time(n as i64 + 1, 0))
                .unwrap();
        }
        let v = collect(
            &ws,
            "chat",
            0,
            SessionStatus {
                live: false,
                running: false,
                background: false,
            },
            &[],
        );
        assert_eq!(v.runs.len(), 500);
        assert!(v.runs_truncated);
        assert!(!v.runs.iter().any(|r| r.run_id == "r000"));
        assert!(v.runs.iter().any(|r| r.run_id == "r500"));
    }

    #[test]
    fn run_metadata_larger_than_64k_is_not_loaded() {
        let temp = tempfile::tempdir().unwrap();
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join("chat.json"),
            "chat",
            temp.path().join("key"),
        );
        std::fs::create_dir_all(ws.run_dir("large")).unwrap();
        std::fs::write(
            ws.run_dir("large").join("run.json"),
            serde_json::json!({"status":"completed","padding":"x".repeat(65536)}).to_string(),
        )
        .unwrap();
        let v = collect(
            &ws,
            "chat",
            0,
            SessionStatus {
                live: false,
                running: false,
                background: false,
            },
            &[],
        );
        assert_eq!(v.runs[0].status, "failed");
    }

    #[test]
    fn a_finished_run_with_large_metadata_is_listed_from_its_summary() {
        let temp = tempfile::tempdir().unwrap();
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join("chat.json"),
            "chat",
            temp.path().join("key"),
        );
        let dir = ws.run_dir("big");
        std::fs::create_dir_all(&dir).unwrap();
        let calls: Vec<serde_json::Value> = (0..200)
            .map(|i| serde_json::json!({"seq":i,"tool":"cat","args_preview":"a".repeat(1000)}))
            .collect();
        let meta = serde_json::json!({"schema":1,"run_id":"big","status":"completed","started_at_ms":5,"finished_at_ms":9,"background":false,"calls":calls,"calls_total":200,"description":"zone big run","llm":{"calls":3,"input_tokens":10},"displays_total":2,"engine_changes_total":4});
        std::fs::write(dir.join("run.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(std::fs::metadata(dir.join("run.json")).unwrap().len() > 64 * 1024);
        std::fs::write(
            dir.join(super::super::workspace::RUN_SUMMARY_FILE),
            serde_json::to_vec(&super::super::workspace::run_summary(&meta)).unwrap(),
        )
        .unwrap();
        let v = collect(
            &ws,
            "chat",
            0,
            SessionStatus {
                live: false,
                running: false,
                background: false,
            },
            &[],
        );
        let run = &v.runs[0];
        assert_eq!(run.status, "completed");
        assert_eq!(run.calls_total, 200);
        assert_eq!(run.finished_at_ms, Some(9));
        assert_eq!(run.llm_calls, Some(3));
        assert_eq!(run.displays_total, Some(2));
        assert_eq!(run.engine_changes, Some(4));
        assert_eq!(run.description.as_deref(), Some("zone big run"));
    }
}

fn apply_live_counts(view: &mut WorkspaceView, counts: &std::collections::HashMap<String, u64>) {
    for run in &mut view.runs {
        if let Some(count) = counts.get(&run.run_id) {
            run.calls_total = *count;
        }
    }
}
