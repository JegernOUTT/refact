use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;

use crate::app_state::AppState;
use crate::attention::AttentionQueue;
use crate::coordination::error::WorkError;

/// Read-only: what is waiting on the human, ranked, and what settled without
/// them. Nothing here answers a question.
pub async fn queue(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Result<Json<AttentionQueue>, WorkError> {
    super::work::authorize(&app, &headers, peer.map(|p| p.0))?;
    Ok(Json(app.attention().queue(chrono::Utc::now())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attention::{AttentionRequest, AttentionScope};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};

    use refact_chat_api::QuestionKind;
    use tower::ServiceExt;

    async fn app(token: Option<&str>) -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx)
            .unwrap()
            .cmdline
            .daemon_auth_token = token.map(str::to_string);
        std::sync::Arc::get_mut(&mut gcx).unwrap().cmdline.http_host = Some("127.0.0.1".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        (AppState::from_gcx(gcx).await, dir)
    }

    async fn request(app: AppState, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().uri("/attention");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let mut req = builder.body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(ConnectInfo("127.0.0.1:7".parse::<SocketAddr>().unwrap()));
        let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
        let response = router.oneshot(req).await.unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn ask(text: &str) -> AttentionRequest {
        AttentionRequest::new(
            AttentionScope::new("/project", "AllowToSendAnywhere"),
            QuestionKind::UserInput,
            "agent",
            "user",
            text,
        )
    }

    #[tokio::test]
    async fn the_attention_queue_requires_authorization_before_it_reveals_anything() {
        let (app, _dir) = app(Some("attention-token")).await;
        for token in [None, Some("wrong")] {
            let (status, body) = request(app.clone(), token).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body.get("waiting").is_none());
        }
        let (status, body) = request(app, Some("attention-token")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["waiting"].as_array().unwrap().len(), 0);
        assert_eq!(body["settled"].as_array().unwrap().len(), 0);
    }
}
