use std::fs;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_DIR_ENTRIES: usize = 500;

#[derive(Debug, Deserialize)]
pub(crate) struct BrowseRequest {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct BrowseDir {
    name: String,
    has_git: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct BrowseResponse {
    path: String,
    parent: Option<String>,
    dirs: Vec<BrowseDir>,
    can_open: bool,
    truncated: bool,
}

pub(crate) async fn browse(Json(request): Json<BrowseRequest>) -> Response {
    let requested = request.path.map(PathBuf::from);
    let result = tokio::task::spawn_blocking(move || {
        let home = requested.is_none().then(resolve_home_dir).flatten();
        browse_requested(requested.as_deref(), home.as_deref(), MAX_DIR_ENTRIES)
    })
    .await;

    match result {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(detail)) => {
            (StatusCode::BAD_REQUEST, Json(json!({"detail": detail}))).into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": format!("folder browse task failed: {error}")})),
        )
            .into_response(),
    }
}

fn resolve_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn browse_requested(
    requested: Option<&Path>,
    home_dir: Option<&Path>,
    limit: usize,
) -> Result<BrowseResponse, String> {
    let path = requested
        .or(home_dir)
        .ok_or_else(|| "home directory is unavailable".to_string())?;
    browse_at(path, limit)
}

fn browse_at(path: &Path, limit: usize) -> Result<BrowseResponse, String> {
    let canonical = fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|error| format!("failed to open '{}': {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("path is not a directory: {}", canonical.display()));
    }

    let entries = fs::read_dir(&canonical)
        .map_err(|error| format!("failed to read '{}': {error}", canonical.display()))?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let child = entry.path();
        let metadata = match fs::symlink_metadata(&child) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let has_git = fs::symlink_metadata(child.join(".git"))
            .map(|metadata| !metadata.file_type().is_symlink())
            .unwrap_or(false);
        dirs.push(BrowseDir { name, has_git });
    }

    dirs.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    let truncated = dirs.len() > limit;
    dirs.truncate(limit);

    Ok(BrowseResponse {
        path: canonical.to_string_lossy().into_owned(),
        parent: canonical
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        dirs,
        can_open: true,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(path: &Path) -> String {
        dunce::simplified(&fs::canonicalize(path).unwrap())
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn default_path_uses_home_directory() {
        let temp = tempfile::tempdir().unwrap();

        let response = browse_requested(None, Some(temp.path()), MAX_DIR_ENTRIES).unwrap();

        assert_eq!(response.path, canonical(temp.path()));
        assert!(response.can_open);
    }

    #[tokio::test]
    async fn lists_immediate_visible_directories_sorted_with_git_markers() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("zulu")).unwrap();
        fs::create_dir(temp.path().join("Alpha")).unwrap();
        fs::create_dir(temp.path().join("beta")).unwrap();
        fs::create_dir(temp.path().join("beta").join(".git")).unwrap();
        fs::create_dir(temp.path().join(".hidden")).unwrap();
        fs::create_dir(temp.path().join("node_modules")).unwrap();
        fs::write(temp.path().join("file.txt"), b"not a directory").unwrap();

        let response = browse_at(temp.path(), MAX_DIR_ENTRIES).unwrap();

        assert_eq!(
            response.dirs,
            vec![
                BrowseDir {
                    name: "Alpha".to_string(),
                    has_git: false,
                },
                BrowseDir {
                    name: "beta".to_string(),
                    has_git: true,
                },
                BrowseDir {
                    name: "node_modules".to_string(),
                    has_git: false,
                },
                BrowseDir {
                    name: "zulu".to_string(),
                    has_git: false,
                },
            ]
        );
        assert!(!response.truncated);
    }

    #[tokio::test]
    async fn rejects_missing_and_file_paths() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let file = temp.path().join("file.txt");
        fs::write(&file, b"file").unwrap();

        assert!(browse_at(&missing, MAX_DIR_ENTRIES)
            .unwrap_err()
            .contains("failed to open"));
        assert!(browse_at(&file, MAX_DIR_ENTRIES)
            .unwrap_err()
            .contains("not a directory"));
    }

    #[tokio::test]
    async fn caps_alphabetically_sorted_directories() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["charlie", "Alpha", "bravo"] {
            fs::create_dir(temp.path().join(name)).unwrap();
        }

        let response = browse_at(temp.path(), 2).unwrap();

        assert_eq!(
            response
                .dirs
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "bravo"]
        );
        assert!(response.truncated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_child_symlinks_and_does_not_follow_git_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("project")).unwrap();
        symlink(outside.path(), temp.path().join("linked-project")).unwrap();
        symlink(outside.path(), temp.path().join("project").join(".git")).unwrap();

        let response = browse_at(temp.path(), MAX_DIR_ENTRIES).unwrap();

        assert_eq!(
            response.dirs,
            vec![BrowseDir {
                name: "project".to_string(),
                has_git: false,
            }]
        );
    }

    #[tokio::test]
    async fn router_keeps_folder_browse_behind_control_auth() {
        use crate::daemon::config::{AuthConfig, DaemonConfig};
        use crate::daemon::events::EventBus;
        use crate::daemon::state::DaemonState;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let temp = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(temp.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let router = crate::daemon::server::make_router(state, 8488);
        let make_request = |authorized: bool| {
            let body = serde_json::to_vec(&json!({"path": temp.path()})).unwrap();
            let mut builder = Request::builder()
                .method("POST")
                .uri("/daemon/v1/fs/browse")
                .header("content-type", "application/json");
            if authorized {
                builder = builder.header("authorization", "Bearer secret");
            }
            let mut request = builder.body(Body::from(body)).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
            request
        };

        let unauthorized = router.clone().oneshot(make_request(false)).await.unwrap();
        let authorized = router.oneshot(make_request(true)).await.unwrap();

        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(authorized.status(), StatusCode::OK);
    }
}
