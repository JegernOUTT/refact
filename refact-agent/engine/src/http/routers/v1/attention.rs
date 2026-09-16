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
    use crate::attention::{AttentionEntryState, AttentionPriority, AttentionRequest, AttentionScope};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chrono::{Duration as ChronoDuration, Utc};
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

    #[tokio::test]
    async fn a_silent_default_stays_in_the_queue_surface_after_it_settles() {
        let (app, _dir) = app(None).await;
        let now = Utc::now();
        app.attention()
            .join(
                ask("Rename the branch?")
                    .with_priority(AttentionPriority::Low)
                    .with_default_answer(Some("keep the name".into())),
                "agent-one",
                now,
            )
            .unwrap();
        let timeout = app.attention().limits().default_timeout_secs as i64;
        let later = now + ChronoDuration::seconds(timeout + 1);
        let queue = app.attention().queue(later);
        assert_eq!(queue.silent_defaults, 1);
        assert_eq!(queue.settled[0].state, AttentionEntryState::Defaulted);

        let (status, body) = request(app, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["settled"][0]["state"], "defaulted");
        assert_eq!(body["settled"][0]["default_used"], "keep the name");
    }

    #[tokio::test]
    async fn the_queue_surface_reports_rank_and_what_was_merged_into_a_prompt() {
        let (app, _dir) = app(None).await;
        let now = Utc::now();
        app.attention()
            .join(ask("Lonely?"), "agent-one", now)
            .unwrap();
        for waiter in ["agent-two", "agent-three"] {
            app.attention().join(ask("Crowded?"), waiter, now).unwrap();
        }
        let (status, body) = request(app, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let waiting = body["waiting"].as_array().unwrap();
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0]["rank"], 1);
        assert_eq!(waiting[0]["blocked_runs"], 2);
        assert_eq!(waiting[0]["asked_by"], "agent-two");
        assert_eq!(waiting[0]["merged_waiters"][0], "agent-three");
        assert_eq!(waiting[0]["merged_waiters"].as_array().unwrap().len(), 1);
        assert!(waiting[0]["rank_reason"]
            .as_str()
            .unwrap()
            .contains("2 runs blocked"));
        assert_eq!(waiting[1]["rank"], 2);
        assert_eq!(waiting[1]["state"], "waiting");
    }
}
