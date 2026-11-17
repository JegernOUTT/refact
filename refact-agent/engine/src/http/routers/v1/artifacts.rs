use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::time::UNIX_EPOCH;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::python_tool::workspace::{artifact_path_checked, PythonWorkspace};

#[derive(Deserialize)]
pub struct ArtifactQuery {
    chat_id: String,
    path: String,
}

#[derive(Debug, Serialize)]
pub struct ArtifactResolution {
    chat_id: String,
    rel_path: String,
    path: String,
    artifacts_dir: String,
    exists: bool,
    bytes: Option<u64>,
    modified_at_ms: Option<u64>,
}

fn validate(query: &ArtifactQuery) -> Result<(), String> {
    if query.chat_id.is_empty()
        || query.chat_id.len() > 128
        || !query
            .chat_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(
            "chat_id must contain 1–128 ASCII letters, digits, underscores or hyphens".into(),
        );
    }
    crate::chat::trajectories::validate_trajectory_id(&query.chat_id).map_err(|e| e.message)?;
    let drive_prefix = query.path.as_bytes().get(1) == Some(&b':')
        && query.path.as_bytes()[0].is_ascii_alphabetic();
    if query.path.len() > 1024
        || query.path.contains('\\')
        || drive_prefix
        || query.path.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
        || query
            .path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("path must be a relative artifact path of at most 1024 bytes with non-empty, non-traversing segments and no drive prefix, backslashes or control characters".into());
    }
    Ok(())
}

pub async fn handle_v1_artifacts_resolve(
    State(app): State<AppState>,
    Query(query): Query<ArtifactQuery>,
) -> Result<Json<ArtifactResolution>, ScratchError> {
    let bad = |message| ScratchError::new(StatusCode::BAD_REQUEST, message);
    validate(&query).map_err(bad)?;
    let workspace = PythonWorkspace::existing_for_chat(&app, &query.chat_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "Chat has no resolvable artifacts location".into(),
            )
        })?;
    let root = workspace.artifacts_dir();
    let path = root.join(&query.path);
    artifact_path_checked(&root, &path).await.map_err(bad)?;
    let meta = match tokio::fs::symlink_metadata(&path).await {
        Ok(meta) if meta.is_file() => Some(meta),
        Ok(_) => None,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            None
        }
        Err(e) => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            ))
        }
    };
    Ok(Json(ArtifactResolution {
        chat_id: query.chat_id,
        rel_path: query.path,
        path: path.to_string_lossy().into_owned(),
        artifacts_dir: root.to_string_lossy().into_owned(),
        exists: meta.is_some(),
        bytes: meta.as_ref().map(|m| m.len()),
        modified_at_ms: meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn artifacts_invalid_path_classes() {
        for path in [
            "", "/a", "a/", "a//b", ".", "..", "a/../b", "a/./b", "C:foo", "a\\b", "a\0b",
            "a\u{1f}b", "a\u{7f}b",
        ] {
            assert!(
                validate(&ArtifactQuery {
                    chat_id: "chat".into(),
                    path: path.into()
                })
                .is_err(),
                "{path:?}"
            );
        }
        assert!(validate(&ArtifactQuery {
            chat_id: "chat".into(),
            path: "x".repeat(1025)
        })
        .is_err());
        for id in ["", "../chat", "a.b", "chat/child", "a b"] {
            assert!(validate(&ArtifactQuery {
                chat_id: id.into(),
                path: "file".into()
            })
            .is_err());
        }
        assert!(validate(&ArtifactQuery {
            chat_id: "chat".into(),
            path: "folder/report ü.html".into()
        })
        .is_ok());
    }

    #[tokio::test]
    async fn artifacts_resolve_existing_missing_and_unknown_without_creating() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let dir = temp.path().join(".refact/trajectories");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("artifact-chat.json"),
            br#"{"id":"artifact-chat","messages":[]}"#,
        )
        .await
        .unwrap();
        let app = AppState::from_gcx(gcx).await;
        let workspace = PythonWorkspace::existing_for_chat(&app, "artifact-chat")
            .await
            .unwrap()
            .unwrap();
        let root = workspace.artifacts_dir();
        assert!(!root.exists());
        let router = crate::http::routers::make_refact_http_server(app.clone());
        async fn get(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
            let response = router
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let value =
                serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                    .unwrap();
            (status, value)
        }
        let (status, value) = get(
            router.clone(),
            "/v1/artifacts/resolve?chat_id=artifact-chat&path=report.html",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["exists"], false);
        assert_eq!(value["bytes"], serde_json::Value::Null);
        assert!(!root.exists());
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(root.join("report.html"), "hello")
            .await
            .unwrap();
        let (status, value) = get(
            router.clone(),
            "/v1/artifacts/resolve?chat_id=artifact-chat&path=report.html",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["exists"], true);
        assert_eq!(value["bytes"], 5);
        assert!(value["modified_at_ms"].is_u64());
        assert_eq!(
            value["path"],
            root.join("report.html").to_string_lossy().as_ref()
        );
        assert_eq!(
            get(
                router.clone(),
                "/v1/artifacts/resolve?chat_id=artifact-chat&path=missing"
            )
            .await
            .1["exists"],
            false
        );
        let before = std::fs::read_dir(&dir).unwrap().count();
        assert!(
            PythonWorkspace::existing_for_chat(&app, "unknown-artifact-chat")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            get(
                router.clone(),
                "/v1/artifacts/resolve?chat_id=unknown-artifact-chat&path=file"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            before,
            "entries: {:?}",
            std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<Vec<_>>()
        );
        for path in [
            "%2Ffile",
            "a%2F..%2Fb",
            "a%5Cb",
            "C%3Afile",
            "a%00b",
            "a%7Fb",
            "a%2F%2Fb",
        ] {
            assert_eq!(
                get(
                    router.clone(),
                    &format!("/v1/artifacts/resolve?chat_id=artifact-chat&path={path}")
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path(), root.join("escape")).unwrap();
            assert_eq!(
                get(
                    router.clone(),
                    "/v1/artifacts/resolve?chat_id=artifact-chat&path=escape/file"
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
            tokio::fs::remove_dir_all(&root).await.unwrap();
            std::os::unix::fs::symlink(temp.path(), &root).unwrap();
            assert_eq!(
                get(
                    router,
                    "/v1/artifacts/resolve?chat_id=artifact-chat&path=file"
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
    }
}
