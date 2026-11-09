//! Host-only catalog inventory. The command adapter must reject model/tool callers.
//! No trajectory messages, binding state, parameters or provider tokens are projected.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use refact_chat_api::{StoredAppWorkspace, WorktreeMeta};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::chat::trajectories::{is_internal_trace_link_type, validate_trajectory_id};
use crate::chat::trajectory_index::TRAJECTORY_INDEX_FILE;
use super::AppRuntime;

const MAX_ENTRIES: usize = 4096;
const MAX_FILES: usize = 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LIVE: usize = 1024;

#[derive(Default, Serialize)]
struct Coverage {
    complete: bool,
    truncated: bool,
    entries_scanned: usize,
    files_scanned: usize,
    bytes_read: u64,
    live_scanned: usize,
    errors: usize,
    excluded: usize,
}

#[derive(Serialize)]
struct Chat {
    chat_id: String,
    title: String,
    binding_ids: Vec<Uuid>,
}

// Unknown fields are skipped by serde, not collected into a Value. In particular,
// even a large message history is never materialized as messages or token metadata.
#[derive(Deserialize)]
struct Metadata {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    mode: String,
    parent_id: Option<String>,
    link_type: Option<String>,
    buddy_meta: Option<serde::de::IgnoredAny>,
    task_meta: Option<serde::de::IgnoredAny>,
    worktree: Option<WorktreeMeta>,
    app_workspace: Option<StoredAppWorkspace>,
}

fn displayable(mode: &str, _parent: Option<&str>, link: Option<&str>, companion: bool) -> bool {
    !companion && mode != "buddy" && !is_internal_trace_link_type(link)
}

fn chat_metadata(
    id: &str,
    title: &str,
    workspace: Option<&StoredAppWorkspace>,
    app_id: Uuid,
) -> Result<Option<Chat>, String> {
    validate_trajectory_id(id).map_err(|_| "invalid chat id")?;
    let Some(stored) = workspace else {
        return Ok(None);
    };
    let workspace = stored.supported().ok_or("unsupported app workspace")?;
    workspace.validate()?;
    let mut binding_ids: Vec<_> = workspace
        .bindings
        .iter()
        .filter(|binding| binding.app_id == app_id)
        .map(|binding| binding.binding_id)
        .collect();
    binding_ids.sort();
    binding_ids.dedup();
    Ok((!binding_ids.is_empty()).then(|| Chat {
        chat_id: id.to_owned(),
        title: title.chars().take(240).collect(),
        binding_ids,
    }))
}

fn rooted_open(root: &Path, relative: &Path, directory: bool) -> std::io::Result<std::fs::File> {
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    let mut dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    let parts: Vec<_> = relative.components().collect();
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid relative path",
            ));
        };
        if index + 1 < parts.len() || directory {
            dir = dir.open_dir_nofollow(name)?;
        } else {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NONBLOCK);
            }
            return dir.open_with(name, &options).map(|file| file.into_std());
        }
    }
    Ok(dir.into_std_file())
}

fn read_metadata(
    root: &Path,
    relative: &Path,
    coverage: &mut Coverage,
) -> Result<Metadata, String> {
    if coverage.files_scanned >= MAX_FILES {
        coverage.truncated = true;
        return Err("file count limit".into());
    }
    coverage.files_scanned += 1;
    let file = rooted_open(root, relative, false).map_err(|_| "trajectory unreadable or unsafe")?;
    let metadata = file
        .metadata()
        .map_err(|_| "trajectory metadata unavailable")?;
    if !metadata.is_file() {
        return Err("not a regular trajectory".into());
    }
    let remaining = MAX_TOTAL_BYTES.saturating_sub(coverage.bytes_read);
    if metadata.len() > MAX_FILE_BYTES || metadata.len() > remaining {
        coverage.truncated = true;
        return Err("trajectory byte limit".into());
    }
    // take bounds actual reads too, even if a concurrent writer grows the file.
    let limit = MAX_FILE_BYTES.min(remaining);
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "trajectory read failed")?;
    coverage.bytes_read += bytes.len() as u64;
    if bytes.len() as u64 > limit {
        coverage.truncated = true;
        return Err("trajectory grew past byte limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "malformed trajectory metadata".into())
}

fn persisted(root: &Path, app_id: Uuid) -> (BTreeMap<String, Chat>, Coverage) {
    let mut coverage = Coverage::default();
    let mut chats = BTreeMap::new();
    let base = PathBuf::from(".refact/trajectories");
    match rooted_open(root, &base, true) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (chats, coverage),
        Err(_) => {
            coverage.errors += 1;
            return (chats, coverage);
        }
    }
    let mut directories = vec![base];
    while let Some(relative_dir) = directories.pop() {
        if rooted_open(root, &relative_dir, true).is_err() {
            coverage.errors += 1;
            continue;
        }
        let entries = match std::fs::read_dir(root.join(&relative_dir)) {
            Ok(entries) => entries,
            Err(_) => {
                coverage.errors += 1;
                continue;
            }
        };
        for entry in entries {
            if coverage.entries_scanned >= MAX_ENTRIES || coverage.files_scanned >= MAX_FILES {
                coverage.truncated = true;
                return (chats, coverage);
            }
            coverage.entries_scanned += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    coverage.errors += 1;
                    continue;
                }
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                coverage.errors += 1;
                continue;
            };
            if name.starts_with('.')
                || name == TRAJECTORY_INDEX_FILE
                || name == "index.journal.jsonl"
                || name == "internal"
            {
                continue;
            }
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    coverage.errors += 1;
                    continue;
                }
            };
            if kind.is_symlink() {
                coverage.errors += 1;
                continue;
            }
            let relative = relative_dir.join(name);
            if kind.is_dir() {
                // Existing trajectory layout: legacy flat files and one chat-family folder.
                if relative_dir.components().count() == 2 && validate_trajectory_id(name).is_ok() {
                    directories.push(relative);
                }
                continue;
            }
            if !kind.is_file() || !name.ends_with(".json") {
                continue;
            }
            let data = match read_metadata(root, &relative, &mut coverage) {
                Ok(data) => data,
                Err(_) => {
                    coverage.errors += 1;
                    continue;
                }
            };
            if relative.file_stem().and_then(|s| s.to_str()) != Some(data.id.as_str()) {
                coverage.errors += 1;
                continue;
            }
            if !displayable(
                &data.mode,
                data.parent_id.as_deref(),
                data.link_type.as_deref(),
                data.buddy_meta.is_some() || data.task_meta.is_some(),
            ) {
                coverage.excluded += 1;
                continue;
            }
            // A normal trajectory stored here belongs to this checkout unless it explicitly
            // declares a different worktree. Never follow that untrusted path to read data.
            if let Some(worktree) = &data.worktree {
                match std::fs::canonicalize(&worktree.root) {
                    Ok(worktree_root) if worktree_root == root => {}
                    Ok(_) => {
                        coverage.excluded += 1;
                        continue;
                    }
                    Err(_) => {
                        coverage.errors += 1;
                        continue;
                    }
                }
            }
            match chat_metadata(&data.id, &data.title, data.app_workspace.as_ref(), app_id) {
                Ok(Some(chat)) => {
                    chats.insert(chat.chat_id.clone(), chat);
                }
                Ok(None) => {}
                Err(_) => coverage.errors += 1,
            }
        }
    }
    (chats, coverage)
}

pub async fn get(app: &AppState, runtime: &AppRuntime, app_id: Uuid) -> Result<Value, String> {
    let record = runtime.store.get_by_id(app_id).await?;
    let statuses = runtime.automation.snapshot(&app_id.to_string()).await;
    let connectors: Vec<_> = record
        .manifest
        .sources
        .iter()
        .map(|source| {
            // Source status only: cached connector payloads can contain arbitrary data.
            let status = statuses
                .iter()
                .find(|status| status.source_id == source.source_id)
                .cloned()
                .map(|mut status| {
                    status.data = None;
                    status
                });
            json!({"source": source, "runtime": status})
        })
        .collect();
    let root = runtime.root.clone();
    let (mut chats, mut coverage) = tokio::task::spawn_blocking(move || persisted(&root, app_id))
        .await
        .map_err(|_| "details inventory task failed")?;
    // Clone Arcs while holding the registry guard, never hold it across session awaits.
    let sessions = {
        let sessions = app.chat.sessions.read().await;
        if sessions.len() > MAX_LIVE {
            coverage.truncated = true;
        }
        sessions
            .values()
            .take(MAX_LIVE)
            .cloned()
            .collect::<Vec<_>>()
    };
    for session in sessions {
        coverage.live_scanned += 1;
        let (id, title, workspace, root, visible) = {
            let session = session.lock().await;
            (
                session.chat_id.clone(),
                session.thread.title.clone(),
                session.thread.app_workspace.clone(),
                session
                    .thread
                    .worktree
                    .as_ref()
                    .map(|w| w.root.clone())
                    .or_else(|| session.work_project_root.clone()),
                displayable(
                    &session.thread.mode,
                    session.thread.parent_id.as_deref(),
                    session.thread.link_type.as_deref(),
                    session.thread.buddy_meta.is_some() || session.thread.task_meta.is_some(),
                ),
            )
        };
        if !visible {
            chats.remove(&id);
            continue;
        }
        let Some(root) = root else {
            if workspace.is_some() {
                coverage.errors += 1;
            }
            continue;
        };
        let Ok(root) = tokio::fs::canonicalize(root).await else {
            coverage.errors += 1;
            continue;
        };
        if root != runtime.root {
            chats.remove(&id);
            continue;
        }
        chats.remove(&id);
        match chat_metadata(&id, &title, workspace.as_ref(), app_id) {
            Ok(Some(chat)) => {
                chats.insert(id, chat);
            }
            Ok(None) => {}
            Err(_) => coverage.errors += 1,
        }
    }
    coverage.complete = !coverage.truncated && coverage.errors == 0;
    let mut result = json!({"files": record.files.iter().map(|path| json!({"path":path})).collect::<Vec<_>>(), "app": record, "connectors": connectors, "chats": chats.into_values().collect::<Vec<_>>(), "coverage": coverage});
    result["coverage"]["limits"] = json!({"directory_entries":MAX_ENTRIES,"trajectory_files":MAX_FILES,"file_bytes":MAX_FILE_BYTES,"total_bytes":MAX_TOTAL_BYTES,"live_sessions":MAX_LIVE});
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trajectory(app_id: Uuid, binding_id: Uuid) -> Value {
        json!({"id":"closed-chat", "title":"Closed chat", "mode":"agent", "messages":[{"content":"SECRET MESSAGE"}], "provider_token":"SECRET TOKEN", "app_workspace":{
            "schema_version":1,"revision":1,"bindings":[{"binding_id":binding_id,"app_id":app_id,"state":{"private":"SECRET STATE"}}],
            "layout":{"kind":"leaf","tabs":[{"kind":"chat","ref":"self"},{"kind":"app","binding_id":binding_id}],"active":0}
        }})
    }

    fn write(root: &Path, value: &Value) {
        let dir = root.join(".refact/trajectories/closed-chat");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("closed-chat.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn persisted_closed_chat_is_metadata_only_and_workspace_is_validated() {
        let temp = tempfile::tempdir().unwrap();
        let app_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let mut value = trajectory(app_id, binding_id);
        write(temp.path(), &value);
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert_eq!(coverage.errors, 0);
        let projected = serde_json::to_value(chats).unwrap();
        assert_eq!(projected["closed-chat"]["binding_ids"], json!([binding_id]));
        assert!(!projected.to_string().contains("SECRET"));
        value["app_workspace"]["schema_version"] = json!(999);
        write(temp.path(), &value);
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert!(chats.is_empty());
        assert_eq!(coverage.errors, 1);
    }

    #[test]
    fn malformed_and_internal_trajectories_are_not_claimed_complete() {
        let temp = tempfile::tempdir().unwrap();
        let app_id = Uuid::new_v4();
        let mut value = trajectory(app_id, Uuid::new_v4());
        value["link_type"] = json!("internal:tool-trace");
        write(temp.path(), &value);
        assert!(persisted(temp.path(), app_id).0.is_empty());
        std::fs::write(temp.path().join(".refact/trajectories/broken.json"), b"{").unwrap();
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert!(chats.is_empty());
        assert_eq!(coverage.errors, 1);
    }

    #[test]
    fn foreign_worktree_and_companion_are_excluded() {
        assert!(!displayable("buddy", None, None, false));
        assert!(!displayable("agent", None, None, true));
        assert!(displayable(
            "agent",
            Some("parent"),
            Some("subagent"),
            false
        ));
        assert!(displayable("agent", Some("parent"), Some("branch"), false));
        let temp = tempfile::tempdir().unwrap();
        let app_id = Uuid::new_v4();
        let mut value = trajectory(app_id, Uuid::new_v4());
        value["worktree"] = json!({"id":"foreign","kind":"chat","root":"/other-checkout", "source_workspace_root":"/other-checkout","repo_root":"/other-checkout"});
        write(temp.path(), &value);
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert!(chats.is_empty());
        assert!(coverage.excluded > 0 || coverage.errors > 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_files_and_ancestor_directories_are_never_read() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let app_id = Uuid::new_v4();
        write(outside.path(), &trajectory(app_id, Uuid::new_v4()));
        std::fs::create_dir_all(temp.path().join(".refact")).unwrap();
        symlink(
            outside.path().join(".refact/trajectories"),
            temp.path().join(".refact/trajectories"),
        )
        .unwrap();
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert!(chats.is_empty());
        assert_eq!(coverage.bytes_read, 0);
        assert_eq!(coverage.errors, 1);
        std::fs::remove_file(temp.path().join(".refact/trajectories")).unwrap();
        std::fs::create_dir_all(temp.path().join(".refact/trajectories")).unwrap();
        symlink(
            outside
                .path()
                .join(".refact/trajectories/closed-chat/closed-chat.json"),
            temp.path().join(".refact/trajectories/closed-chat.json"),
        )
        .unwrap();
        let (chats, coverage) = persisted(temp.path(), app_id);
        assert!(chats.is_empty());
        assert_eq!(coverage.bytes_read, 0);
        assert_eq!(coverage.errors, 1);
    }
}
