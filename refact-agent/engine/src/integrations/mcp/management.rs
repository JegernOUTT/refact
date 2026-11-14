use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use hyper::StatusCode;

use crate::{custom_error::ScratchError, global_context::GlobalContext};
use super::integr_mcp_common::{mcp_lifecycle_lock, startup_task_in_flight};
use super::session_mcp::{abort_and_join, cancel_mcp_client, MCPConnectionStatus, SessionMCP};

static RECONNECTS_IN_FLIGHT: StdMutex<Option<HashSet<String>>> = StdMutex::new(None);

/// Reservation for one manual reconnect; released when the owning task finishes.
struct ReconnectReservation(String);

impl ReconnectReservation {
    fn try_acquire(config_path: &str) -> Option<Self> {
        let mut set = RECONNECTS_IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set.get_or_insert_with(HashSet::new)
            .insert(config_path.to_string())
            .then(|| Self(config_path.to_string()))
    }
}

impl Drop for ReconnectReservation {
    fn drop(&mut self) {
        let mut set = RECONNECTS_IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(set) = set.as_mut() {
            set.remove(&self.0);
        }
    }
}

fn busy() -> ScratchError {
    ScratchError::new(
        StatusCode::CONFLICT,
        "MCP server is already connecting or reconnecting".into(),
    )
}

/// Restarts one MCP server from its config. The work runs in an owned task so a dropped
/// caller (client disconnect, tool abort) cannot leave the session half-transitioned.
pub async fn reconnect_mcp_server(
    gcx: Arc<GlobalContext>,
    config_path: &str,
) -> Result<(), ScratchError> {
    let reservation = ReconnectReservation::try_acquire(config_path).ok_or_else(busy)?;
    let config_path = config_path.to_string();
    tokio::spawn(async move {
        let _reservation = reservation;
        reconnect_owned(gcx, &config_path).await
    })
    .await
    .map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MCP reconnect task failed".into(),
        )
    })?
}

async fn reconnect_owned(gcx: Arc<GlobalContext>, config_path: &str) -> Result<(), ScratchError> {
    let session = {
        let _lifecycle = mcp_lifecycle_lock(config_path).await;
        let session = gcx
            .integration_sessions
            .lock()
            .await
            .get(config_path)
            .cloned()
            .ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, "no MCP session".into()))?;
        let (client, logs, tasks, startup_abort) = {
            let mut locked = session.lock().await;
            let mcp = locked
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .ok_or_else(|| {
                    ScratchError::new(StatusCode::BAD_REQUEST, "not an MCP session".into())
                })?;
            if matches!(
                mcp.connection_status,
                MCPConnectionStatus::Reconnecting { .. }
            ) || (matches!(mcp.connection_status, MCPConnectionStatus::Connecting)
                && startup_task_in_flight(mcp))
            {
                return Err(busy());
            }
            mcp.connection_status = MCPConnectionStatus::Reconnecting {
                attempt: 0,
            };
            mcp.launched_cfg = serde_json::Value::Null;
            mcp.lifecycle_epoch = mcp.lifecycle_epoch.wrapping_add(1);
            (
                mcp.mcp_client.take(),
                mcp.logs.clone(),
                [mcp.health_task_handle.take(), mcp.recovery_task.take()],
                mcp.startup_task_handles.take().map(|(_, abort)| abort),
            )
        };
        if let Some(abort) = startup_abort {
            abort.abort();
        }
        for task in tasks {
            abort_and_join(task).await;
        }
        if let Some(client) = client {
            cancel_mcp_client(config_path, client, logs).await;
        }
        {
            let mut locked = session.lock().await;
            if let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() {
                mcp.mcp_tools.clear();
                mcp.mcp_resources.clear();
                mcp.mcp_prompts.clear();
                mcp.server_info = None;
                mcp.connection_status = MCPConnectionStatus::Connecting;
            }
        }
        session
    };
    // Setup takes the lifecycle lock itself, so it must be released before this call.
    let filename = Path::new(config_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let (_, errors) = crate::integrations::running_integrations::load_integrations(
        gcx.clone(),
        &[format!("**/integrations.d/{filename}")],
    )
    .await;

    let current = gcx
        .integration_sessions
        .lock()
        .await
        .get(config_path)
        .cloned()
        .unwrap_or(session);
    let mut locked = current.lock().await;
    let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() else {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "not an MCP session".into(),
        ));
    };
    let installed =
        !mcp.launched_cfg.is_null() || mcp.mcp_client.is_some() || startup_task_in_flight(mcp);
    if installed {
        return Ok(());
    }
    let reason = errors
        .iter()
        .find(|e| e.path == config_path)
        .map(|e| e.error_msg.clone())
        .unwrap_or_else(|| "config is invalid, disabled or not loadable".into());
    let message = format!("MCP server could not be restarted: {reason}");
    mcp.connection_status = MCPConnectionStatus::Failed {
        message: message.clone(),
    };
    Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, message))
}

fn invalid_path() -> ScratchError {
    ScratchError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "integration_path is invalid".into(),
    )
}

/// Checks a config path against the configured `integrations.d` roots. Symlinks above a
/// root are allowed (dotfile-managed `~/.config`, macOS `/var`); the config file itself
/// must not be a symlink and must sit directly inside the canonical root.
pub fn validate_integration_path(path: &Path, roots: &[PathBuf]) -> Result<(), ScratchError> {
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Non-normal integration path refused".into(),
        ));
    }
    crate::integrations::setting_up_integrations::split_path_into_project_and_integration(
        &path.to_path_buf(),
    )
    .map_err(|_| invalid_path())?;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                "integration_path not found".into(),
            ))
        }
        Err(_) => return Err(invalid_path()),
    };
    if meta.file_type().is_symlink() {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Symlinked integration path refused".into(),
        ));
    }
    if !meta.is_file() {
        return Err(invalid_path());
    }
    ensure_inside_roots(path, roots)
}

fn ensure_inside_roots(path: &Path, roots: &[PathBuf]) -> Result<(), ScratchError> {
    let parent = path.parent().and_then(|p| std::fs::canonicalize(p).ok());
    let canonical = std::fs::canonicalize(path).ok();
    let inside = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|root| {
            parent.as_ref() == Some(&root)
                && canonical
                    .as_ref()
                    .map_or(true, |c| c.parent() == Some(root.as_path()))
        });
    if inside {
        Ok(())
    } else {
        Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "integration_path is outside configured integrations.d directories".into(),
        ))
    }
}

/// Save-side check for a config that may not exist yet under `root`.
pub fn validate_config_save_path(path: &Path, root: &Path) -> Result<(), ScratchError> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Symlinked MCP config path refused".into(),
        ));
    }
    if root.exists() {
        ensure_inside_roots(path, &[root.to_path_buf()])?;
    }
    Ok(())
}

pub async fn configured_integration_roots(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let (mut dirs, global) =
        crate::integrations::setting_up_integrations::get_config_dirs(gcx, &None).await;
    dirs.push(global);
    dirs.into_iter().map(|d| d.join("integrations.d")).collect()
}

pub async fn delete_integration_config(
    gcx: Arc<GlobalContext>,
    path: &Path,
) -> Result<(), ScratchError> {
    let roots = configured_integration_roots(gcx.clone()).await;
    validate_integration_path(path, &roots)?;
    std::fs::remove_file(path).map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to delete integration config".into(),
        )
    })?;
    super::integr_mcp_common::remove_mcp_session_for_config(gcx, &path.to_string_lossy()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::Mutex as AMutex;

    async fn fixture(workspace: &Path) -> (tempfile::TempDir, Arc<GlobalContext>) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join(".config/refact"),
        )
        .await;
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(workspace.to_path_buf());
        (dir, gcx)
    }

    fn write_config(root: &Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(".refact/integrations.d").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    async fn insert_detached(gcx: &Arc<GlobalContext>, key: &str) {
        gcx.integration_sessions.lock().await.insert(
            key.to_string(),
            Arc::new(AMutex::new(Box::new(SessionMCP::detached(key)))),
        );
    }

    #[tokio::test]
    async fn reconnect_with_malformed_config_fails_with_422_and_failed_status() {
        let ws = tempfile::tempdir().unwrap();
        let (_dir, gcx) = fixture(ws.path()).await;
        let path = write_config(ws.path(), "mcp_http_broken.yaml", "url: [unterminated\n");
        let key = path.to_string_lossy().into_owned();
        insert_detached(&gcx, &key).await;
        let err = tokio::time::timeout(
            Duration::from_secs(20),
            reconnect_mcp_server(gcx.clone(), &key),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
        let session = gcx
            .integration_sessions
            .lock()
            .await
            .get(&key)
            .cloned()
            .unwrap();
        let mut locked = session.lock().await;
        let mcp = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
        assert!(matches!(
            mcp.connection_status,
            MCPConnectionStatus::Failed { .. }
        ));
        assert!(ReconnectReservation::try_acquire(&key).is_some());
    }

    #[tokio::test]
    async fn concurrent_reconnects_admit_exactly_one_without_deadlock() {
        let ws = tempfile::tempdir().unwrap();
        let (_dir, gcx) = fixture(ws.path()).await;
        let path = write_config(
            ws.path(),
            "mcp_http_race.yaml",
            "url: http://127.0.0.1:9/mcp\navailable:\n  on_your_laptop: false\n",
        );
        let key = path.to_string_lossy().into_owned();
        insert_detached(&gcx, &key).await;
        let held = mcp_lifecycle_lock(&key).await;
        let first = tokio::spawn({
            let (gcx, key) = (gcx.clone(), key.clone());
            async move { reconnect_mcp_server(gcx, &key).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let second = tokio::time::timeout(
            Duration::from_secs(5),
            reconnect_mcp_server(gcx.clone(), &key),
        )
        .await
        .unwrap();
        assert_eq!(second.unwrap_err().status_code, StatusCode::CONFLICT);
        drop(held);
        let first = tokio::time::timeout(Duration::from_secs(20), first)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            first.err().map(|e| e.status_code),
            Some(StatusCode::CONFLICT)
        );
    }

    #[tokio::test]
    async fn dropped_reconnect_caller_does_not_strand_session() {
        let ws = tempfile::tempdir().unwrap();
        let (_dir, gcx) = fixture(ws.path()).await;
        let path = write_config(ws.path(), "mcp_http_drop.yaml", "url: [broken\n");
        let key = path.to_string_lossy().into_owned();
        insert_detached(&gcx, &key).await;
        let held = mcp_lifecycle_lock(&key).await;
        let caller = tokio::spawn({
            let (gcx, key) = (gcx.clone(), key.clone());
            async move { reconnect_mcp_server(gcx, &key).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        caller.abort();
        drop(held);
        tokio::time::timeout(Duration::from_secs(20), async {
            while ReconnectReservation::try_acquire(&key).is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let session = gcx
            .integration_sessions
            .lock()
            .await
            .get(&key)
            .cloned()
            .unwrap();
        let mut locked = session.lock().await;
        let mcp = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
        assert!(!matches!(
            mcp.connection_status,
            MCPConnectionStatus::Reconnecting { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delete_allows_symlinked_ancestor_and_rejects_symlinks_below_root() {
        let real = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let linked_ws = links.path().join("ws");
        std::os::unix::fs::symlink(real.path(), &linked_ws).unwrap();
        let (_dir, gcx) = fixture(&linked_ws).await;

        let path = write_config(&linked_ws, "mcp_http_ok.yaml", "url: x\n");
        delete_integration_config(gcx.clone(), &path).await.unwrap();
        assert!(!path.exists());

        let target = real.path().join("elsewhere.yaml");
        std::fs::write(&target, "url: x\n").unwrap();
        let link = linked_ws.join(".refact/integrations.d/mcp_http_link.yaml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = delete_integration_config(gcx.clone(), &link)
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(target.exists());

        let traversal = linked_ws.join(".refact/integrations.d/../integrations.d/mcp_x.yaml");
        let err = delete_integration_config(gcx.clone(), &traversal)
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);

        let missing = linked_ws.join(".refact/integrations.d/mcp_http_missing.yaml");
        let err = delete_integration_config(gcx, &missing).await.unwrap_err();
        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_of_starting_server_is_bounded() {
        let ws = tempfile::tempdir().unwrap();
        let (_dir, gcx) = fixture(ws.path()).await;
        let path = write_config(ws.path(), "mcp_http_slow.yaml", "url: x\n");
        let key = path.to_string_lossy().into_owned();
        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let startup = tokio::task::spawn_blocking({
            let release = release.clone();
            move || {
                while !release.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let mut session = SessionMCP::detached(&key);
        session.connection_status = MCPConnectionStatus::Connecting;
        let abort = startup.abort_handle();
        session.startup_task_handles = Some((Arc::new(AMutex::new(Some(startup))), abort));
        gcx.integration_sessions
            .lock()
            .await
            .insert(key.clone(), Arc::new(AMutex::new(Box::new(session))));
        let result = tokio::time::timeout(
            Duration::from_secs(9),
            delete_integration_config(gcx.clone(), &path),
        )
        .await;
        release.store(true, std::sync::atomic::Ordering::SeqCst);
        result.unwrap().unwrap();
        assert!(!path.exists());
        assert!(!gcx.integration_sessions.lock().await.contains_key(&key));
    }
}
