use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::{sse::Event, IntoResponse, Response, Sse};
use axum::Json;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::coordination::causal_view::{self, CausalPage, CausalRequest};
use crate::coordination::error::WorkError;
use crate::coordination::scope;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalQuery {
    pub chat_id: Option<String>,
    pub root: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

async fn caller(app: &AppState, query: &CausalQuery) -> Result<scope::WorkCaller, WorkError> {
    match &query.chat_id {
        None => scope::operator_caller(app).await,
        Some(id) => {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(id)
                .cloned()
                .ok_or_else(WorkError::not_visible)?;
            let model = session
                .try_lock()
                .map_err(|_| WorkError::unavailable())?
                .thread
                .model
                .clone();
            scope::model_caller(app, id, &model).await
        }
    }
}

fn trace_request(query: &CausalQuery) -> Result<CausalRequest, WorkError> {
    Ok(CausalRequest {
        root: query
            .root
            .clone()
            .ok_or_else(|| WorkError::invalid("a causal trace requires a root event id"))?,
        rows: query.limit,
        cursor: query.cursor.clone(),
    })
}

type ExtractedQuery = Result<Query<CausalQuery>, axum::extract::rejection::QueryRejection>;

pub async fn trace(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<CausalPage>, WorkError> {
    super::work::authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid causal trace query"))?;
    let caller = caller(&app, &query).await?;
    Ok(Json(
        causal_view::page(&app, &caller, trace_request(&query)?).await?,
    ))
}

pub async fn subscribe(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Response, WorkError> {
    let peer = peer.map(|p| p.0);
    super::work::authorize(&app, &headers, peer)?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid causal trace query"))?;
    if query.cursor.is_some() {
        return Err(WorkError::invalid("subscribe starts with a fresh snapshot"));
    }
    let request = trace_request(&query)?;
    let first = causal_view::page(&app, &caller(&app, &query).await?, request.clone()).await?;
    let stream = async_stream::stream! {
        let mut previous = (first.snapshot_id.clone(), first.seq);
        if let Ok(event) = Event::default().event("snapshot").id(format!("{}:{}", previous.0, previous.1)).json_data(first) {
            yield Ok::<Event, Infallible>(event);
        }
        loop {
            if app.runtime.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(crate::file_activity::settings().work_subscription_interval_ms)).await;
            if app.runtime.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) { break; }
            let next = async {
                super::work::authorize(&app, &headers, peer)?;
                causal_view::page(&app, &caller(&app, &query).await?, request.clone()).await
            }.await;
            match next {
                Ok(page) => {
                    let current = (page.snapshot_id.clone(), page.seq);
                    if current != previous {
                        previous = current;
                        if let Ok(event) = Event::default().event("snapshot").id(format!("{}:{}", previous.0, previous.1)).json_data(page) { yield Ok(event); }
                    }
                }
                Err(error) => {
                    if let Ok(event) = Event::default().event("work_error").json_data(error) { yield Ok(event); }
                    break;
                }
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::causal::{self, CausalEvent};
    use axum::http::{header, StatusCode};
    use axum::{body::Body, http::Request};
    use refact_work_core::{CausalCause, CausalKind};
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

    async fn request(
        app: AppState,
        uri: &str,
        token: Option<&str>,
        peer: &str,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder().uri(uri);
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
        let response = router.oneshot(req).await.unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn record_chain(app: &AppState, scope: &str) -> (String, String) {
        let root = format!("delivery:{scope}");
        let leaf = format!("decision:{scope}");
        causal::record(
            app,
            CausalEvent::new(root.clone(), CausalKind::Delivery, scope.to_string()),
        )
        .await;
        causal::record(
            app,
            CausalEvent::new(leaf.clone(), CausalKind::Decision, scope.to_string())
                .caused_by(vec![CausalCause::Event(root.clone())]),
        )
        .await;
        (root, leaf)
    }

    #[tokio::test]
    async fn a_scoped_trace_is_returned_and_authorization_precedes_any_reconstruction() {
        let (app, _dir) = app(Some("causal-token")).await;
        let scope = format!("chat-{}", uuid::Uuid::new_v4());
        let (root, leaf) = record_chain(&app, &scope).await;

        for token in [None, Some("wrong")] {
            let (status, body) = request(
                app.clone(),
                &format!("/work/causal?root={leaf}"),
                token,
                "127.0.0.1:7",
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["code"], "unauthorized");
            assert!(body.get("events").is_none());
        }

        let (status, body) = request(
            app.clone(),
            &format!("/work/causal?root={leaf}"),
            Some("causal-token"),
            "127.0.0.1:7",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["root"], leaf);
        let ids: Vec<&str> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["event_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![leaf.as_str(), root.as_str()]);
        assert_eq!(body["replay"]["reproduces_model_output"], false);
        assert_eq!(body["replay"]["finding_count"], 0);
        assert!(body["replay"]["scope_note"]
            .as_str()
            .unwrap()
            .contains("does not reproduce model output"));
    }

    #[tokio::test]
    async fn a_missing_root_and_a_guessed_chat_are_refused_without_a_trace() {
        let (app, _dir) = app(Some("causal-token")).await;
        assert_eq!(
            request(
                app.clone(),
                "/work/causal",
                Some("causal-token"),
                "127.0.0.1:7"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                app.clone(),
                "/work/causal?root=%20",
                Some("causal-token"),
                "127.0.0.1:7"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                app.clone(),
                "/work/causal?root=x&states=running",
                Some("causal-token"),
                "127.0.0.1:7"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "an unknown filter is refused rather than silently ignored"
        );
        assert_eq!(
            request(
                app,
                "/work/causal?root=x&chat_id=guessed",
                Some("causal-token"),
                "127.0.0.1:7"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_privacy_denied_node_leaks_nothing_and_leaves_a_visible_gap() {
        let (app, dir) = app(Some("causal-token")).await;
        {
            let (project_root, _) =
                crate::caps::project_model_defaults_state(app.gcx.clone()).await;
            let mut caps = crate::caps::CodeAssistantCaps::default();
            for id in ["trusted/model", "other/model"] {
                caps.chat_models.insert(
                    id.into(),
                    std::sync::Arc::new(crate::caps::ChatModelRecord {
                        base: crate::caps::BaseModelRecord {
                            id: id.into(),
                            ..Default::default()
                        },
                        supports_tools: true,
                        supports_agent: true,
                        ..Default::default()
                    }),
                );
            }
            let mut state = app.gcx.caps_state.write().await;
            state.caps = Some(std::sync::Arc::new(caps));
            state.loaded_project_root = Some(project_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let guarded = dir.path().join("causal-secret.rs");
        std::fs::write(&guarded, "secret").unwrap();
        let guarded_path = guarded.to_string_lossy().to_string();
        app.gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "guarded".into(),
                    patterns: vec![guarded_path.clone()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        let scope = format!("chat-{}", uuid::Uuid::new_v4());
        let secret = format!("evidence:{scope}");
        let open = format!("decision:{scope}");
        causal::record(
            &app,
            CausalEvent::new(secret.clone(), CausalKind::Evidence, scope.clone())
                .with_paths(vec![guarded_path.clone()]),
        )
        .await;
        causal::record(
            &app,
            CausalEvent::new(open.clone(), CausalKind::Decision, scope.clone())
                .caused_by(vec![CausalCause::Event(secret.clone())]),
        )
        .await;
        let mut session = crate::chat::types::ChatSession::new("reader".into());
        session.work_project_root = Some(dir.path().into());
        session.thread.model = "other/model".into();
        app.chat.sessions.write().await.insert(
            "reader".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );

        let (status, body) = request(
            app.clone(),
            &format!("/work/causal?root={open}&chat_id=reader"),
            Some("causal-token"),
            "127.0.0.1:7",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["total"], 1, "the withheld ancestor is not returned");
        assert_eq!(body["complete"], false);
        let rendered = body.to_string();
        assert!(
            rendered.contains(&secret),
            "the gap names the missing event id so the viewer can show a gap"
        );
        assert!(
            !rendered.contains(&guarded_path),
            "a guarded path never reaches the response: {rendered}"
        );
        assert_eq!(
            body["replay"]["finding_count"], 0,
            "a withheld row is a gap, not a replay divergence"
        );
    }
}
