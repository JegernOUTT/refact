use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{sse::Event, IntoResponse, Response, Sse};
use axum::Json;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::coordination::error::WorkError;
use crate::coordination::{scope, service};
use crate::coordination::types::{OverviewScope, WorkOverviewRequest, WorkStateTag};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkQuery {
    pub chat_id: Option<String>,
    pub scope: Option<OverviewScope>,
    pub path: Option<String>,
    pub states: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl IntoResponse for WorkError {
    fn into_response(self) -> Response {
        let status = match self.code {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "scope_denied" => StatusCode::FORBIDDEN,
            "not_found_or_not_visible" => StatusCode::NOT_FOUND,
            "invalid_argument" | "invalid_cursor" => StatusCode::BAD_REQUEST,
            "cursor_expired" => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(self)).into_response()
    }
}

pub(super) fn authorize(
    app: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<(), WorkError> {
    let denied = || {
        WorkError::new(
            "unauthorized",
            "daemon authorization or trusted local transport required",
            false,
        )
    };
    if let Some(expected) = app.gcx.cmdline.daemon_auth_token.as_deref() {
        let token = crate::daemon::auth::bearer_token_from_headers(headers).ok_or_else(denied)?;
        if !expected.is_empty() && crate::daemon::auth::token_matches(token, expected) {
            return Ok(());
        }
        return Err(denied());
    }
    let bind = crate::http::resolve_http_bind_addr(
        app.gcx.cmdline.http_host.as_deref(),
        app.gcx.cmdline.inside_container,
        app.gcx.cmdline.http_port,
    )
    .map_err(|_| denied())?;
    if !bind.ip().is_loopback() || !peer.is_some_and(|peer| peer.ip().is_loopback()) {
        return Err(denied());
    }
    if headers.get(header::ORIGIN).is_some_and(|origin| {
        origin
            .to_str()
            .ok()
            .is_none_or(|origin| !crate::daemon::auth::origin_allowed_for_loopback_or_ide(origin))
    }) {
        return Err(denied());
    }
    Ok(())
}

async fn caller(app: &AppState, query: &WorkQuery) -> Result<scope::WorkCaller, WorkError> {
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

fn overview_request(query: &WorkQuery) -> Result<WorkOverviewRequest, WorkError> {
    let states = query
        .states
        .as_ref()
        .map(|states| {
            states
                .split(',')
                .map(|state| {
                    WorkStateTag::parse(state)
                        .ok_or_else(|| WorkError::invalid("invalid execution state"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(WorkOverviewRequest {
        scope: query.scope.unwrap_or_default(),
        path: query.path.clone(),
        states,
        rows: query.limit,
        cursor: query.cursor.clone(),
        root: None,
    })
}

type ExtractedQuery = Result<Query<WorkQuery>, axum::extract::rejection::QueryRejection>;

#[derive(serde::Serialize)]
pub struct WorkListResponse {
    #[serde(flatten)]
    overview: crate::coordination::types::WorkOverview,
    #[serde(skip_serializing_if = "Option::is_none")]
    board_index_health: Option<crate::coordination::board_index::BoardIndexHealth>,
}

pub async fn list(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<WorkListResponse>, WorkError> {
    authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    let caller = caller(&app, &query).await?;
    let overview = service::overview(&app, &caller, overview_request(&query)?).await?;
    let board_index_health = if query.chat_id.is_none() {
        Some(app.work_boards.snapshot(&caller.source_root).await.health)
    } else {
        None
    };
    Ok(Json(WorkListResponse {
        overview,
        board_index_health,
    }))
}

/// Deletion control for recorded file provenance: forget one authorized path,
/// or everything this project recorded when no path is given.
pub async fn forget_provenance(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<serde_json::Value>, WorkError> {
    authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    if query.states.is_some() || query.scope.is_some() || query.cursor.is_some() {
        return Err(WorkError::invalid(
            "forgetting provenance only accepts chat_id and path",
        ));
    }
    let caller = caller(&app, &query).await?;
    let file_key = match &query.path {
        Some(path) => {
            Some(crate::file_activity::resolve_key(&scope::resolve_path(path, &caller)?).0)
        }
        None => None,
    };
    let removed = crate::coordination::file_provenance::forget(file_key.as_deref())
        .await
        .map_err(|_| WorkError::unavailable())?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

/// Read-only view of who is waiting on whom, filtered to the runs this caller
/// may already see.
pub async fn wait_graph(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<service::Inspection>, WorkError> {
    authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    if query.path.is_some() || query.states.is_some() || query.scope.is_some() {
        return Err(WorkError::invalid(
            "the wait graph only accepts chat_id, limit and cursor",
        ));
    }
    let caller = caller(&app, &query).await?;
    Ok(Json(
        service::inspect(
            &app,
            &caller,
            service::InspectRequest {
                selector: service::InspectSelector::WaitGraph {},
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await?,
    ))
}

pub async fn inspect(
    State(app): State<AppState>,
    Path(id): Path<String>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<service::Inspection>, WorkError> {
    authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    if query.path.is_some() || query.states.is_some() || query.scope.is_some() {
        return Err(WorkError::invalid(
            "detail requests only accept chat_id, limit and cursor",
        ));
    }
    let caller = caller(&app, &query).await?;
    Ok(Json(
        service::inspect(
            &app,
            &caller,
            service::InspectRequest {
                selector: service::InspectSelector::Work { work_id: id },
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await?,
    ))
}

pub async fn subscribe(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Response, WorkError> {
    let peer = peer.map(|p| p.0);
    authorize(&app, &headers, peer)?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    if query.cursor.is_some() {
        return Err(WorkError::invalid("subscribe starts with a fresh snapshot"));
    }
    let request = overview_request(&query)?;
    let first = service::overview(&app, &caller(&app, &query).await?, request.clone()).await?;
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
                authorize(&app, &headers, peer)?;
                service::overview(&app, &caller(&app, &query).await?, request.clone()).await
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
    use axum::{body::Body, http::Request};
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

    #[tokio::test]
    async fn auth_and_trusted_scope_are_checked_before_discovery() {
        let (app, dir) = app(Some("work-test-token")).await;
        let board_path = dir.path().join(".refact/tasks/private-task/board.yaml");
        std::fs::create_dir_all(board_path.parent().unwrap()).unwrap();
        std::fs::write(&board_path, "cards: [").unwrap();
        for token in [None, Some("wrong")] {
            let (status, body) = request(app.clone(), "/work", token, "127.0.0.1:7").await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["code"], "unauthorized");
            assert!(body.get("rows").is_none());
            assert!(body.get("board_index_health").is_none());
        }
        let (status, body) =
            request(app.clone(), "/work", Some("work-test-token"), "127.0.0.1:7").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["coverage"], "partial");
        let health = &body["board_index_health"];
        assert_eq!(health["coverage"], "partial");
        assert_eq!(health["unavailable"], true);
        assert_eq!(health["reason"], "parse");
        assert_eq!(health["retry"], "backoff_or_source_change");
        assert!(health["retry_after_ms"].as_u64().unwrap() <= 1000);
        assert_eq!(health.as_object().unwrap().len(), 5);
        let health = health.to_string();
        assert!(!health.contains("private-task"));
        assert!(!health.contains(dir.path().to_str().unwrap()));
        for uri in [
            "/work?source_workspace_root=/private",
            "/work?root=foreign",
            "/work?limit=0",
            "/work?states=invalid",
        ] {
            assert_eq!(
                request(app.clone(), uri, Some("work-test-token"), "127.0.0.1:7")
                    .await
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            request(
                app,
                "/work?chat_id=guessed",
                Some("work-test-token"),
                "127.0.0.1:7"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn gui_route_and_model_tool_agree_on_the_same_projection() {
        let (app, dir) = app(Some("work-test-token")).await;
        let board_path = dir
            .path()
            .join(".refact/tasks/hidden-board-source/board.yaml");
        std::fs::create_dir_all(board_path.parent().unwrap()).unwrap();
        std::fs::write(&board_path, "cards: [").unwrap();
        assert_eq!(
            app.work_boards.snapshot(dir.path()).await.health.reason,
            Some(crate::coordination::board_index::BoardIndexFailure::Parse)
        );
        {
            let (project_root, _) =
                crate::caps::project_model_defaults_state(app.gcx.clone()).await;
            let mut caps = crate::caps::CodeAssistantCaps::default();
            caps.chat_models.insert(
                "trusted/model".into(),
                std::sync::Arc::new(crate::caps::ChatModelRecord {
                    base: crate::caps::BaseModelRecord {
                        id: "trusted/model".into(),
                        ..Default::default()
                    },
                    supports_tools: true,
                    supports_agent: true,
                    ..Default::default()
                }),
            );
            let mut state = app.gcx.caps_state.write().await;
            state.caps = Some(std::sync::Arc::new(caps));
            state.loaded_project_root = Some(project_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        for id in ["caller", "peer-one", "peer-two"] {
            let mut session = crate::chat::types::ChatSession::new(id.into());
            session.work_project_root = Some(dir.path().into());
            session.thread.title = format!("title {id}");
            session.thread.model = "trusted/model".into();
            app.chat.sessions.write().await.insert(
                id.into(),
                std::sync::Arc::new(tokio::sync::Mutex::new(session)),
            );
        }
        let (status, gui) = request(
            app.clone(),
            "/work?chat_id=caller",
            Some("work-test-token"),
            "127.0.0.1:7",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{gui}");
        let ccx = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                Vec::new(),
                "caller".into(),
                None,
                "trusted/model".into(),
                None,
                None,
            )
            .await,
        ));
        let mut tool = crate::tools::tool_work_overview::ToolWorkOverview {
            config_path: String::new(),
        };
        let (_, output) = crate::tools::tools_description::Tool::tool_execute(
            &mut tool,
            ccx,
            &"overview".into(),
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();
        let crate::call_validation::ContextEnum::ChatMessage(message) = &output[0] else {
            panic!("tool message");
        };
        let tool: serde_json::Value =
            serde_json::from_str(&message.content.content_text_only()).unwrap();
        assert!(gui.get("board_index_health").is_none());
        assert!(tool.get("board_index_health").is_none());
        assert!(gui.get("degraded").is_none());
        assert!(tool.get("degraded").is_none());
        assert!(!gui.to_string().contains("hidden-board-source"));
        assert!(!tool.to_string().contains("hidden-board-source"));
        let ids = |page: &serde_json::Value| {
            let mut ids: Vec<String> = page["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    (
                        row["key"]["id"].as_str().unwrap().to_string(),
                        row["state"].clone(),
                        row["title"].clone(),
                        row["freshness"].clone(),
                    )
                })
                .map(|row| serde_json::to_string(&row).unwrap())
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(gui["total"], 3);
        assert_eq!(gui["total"], tool["total"]);
        assert_eq!(gui["coverage"], tool["coverage"]);
        assert_eq!(ids(&gui), ids(&tool));
        assert_eq!(ids(&gui).len(), 3);
    }

    async fn next_snapshot(
        body: &mut axum::body::BoxBody,
        pending: &mut String,
        raw: &mut String,
    ) -> serde_json::Value {
        use hyper::body::HttpBody;
        loop {
            if let Some(end) = pending.find("\n\n") {
                let frame: String = pending.drain(..end + 2).collect();
                if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data:")) {
                    return serde_json::from_str(data.trim()).unwrap();
                }
                continue;
            }
            let chunk = body.data().await.expect("stream ended").unwrap();
            let text = String::from_utf8(chunk.to_vec()).unwrap();
            raw.push_str(&text);
            pending.push_str(&text);
        }
    }

    async fn retitle(app: &AppState, chat_id: &str, title: &str) {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        session.lock().await.thread.title = title.into();
    }

    #[tokio::test]
    async fn subscription_frames_hide_a_denied_peer_until_the_model_switch_re_evaluates_it() {
        let (app, dir) = app(Some("work-test-token")).await;
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
        let guarded = dir.path().join("denied-peer-secret.rs");
        std::fs::write(&guarded, "denied peer source").unwrap();
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
        for (id, title) in [
            ("sse-caller", "sse caller title"),
            ("sse-denied", "denied peer title"),
            ("sse-visible", "visible peer title"),
        ] {
            let mut session = crate::chat::types::ChatSession::new(id.into());
            session.work_project_root = Some(dir.path().into());
            session.thread.title = title.into();
            session.thread.model = "other/model".into();
            app.chat.sessions.write().await.insert(
                id.into(),
                std::sync::Arc::new(tokio::sync::Mutex::new(session)),
            );
        }
        crate::file_activity::record_read(
            Some("sse-denied"),
            &guarded,
            "denied peer source",
            1,
            1,
            1,
            None,
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer work-test-token".parse().unwrap(),
        );

        tokio::time::pause();
        let response = subscribe(
            State(app.clone()),
            Some(ConnectInfo("127.0.0.1:7".parse().unwrap())),
            headers,
            Ok(Query(WorkQuery {
                chat_id: Some("sse-caller".into()),
                ..Default::default()
            })),
        )
        .await
        .unwrap();
        let mut body = response.into_body();
        let mut pending = String::new();
        let mut denied_bytes = String::new();

        let first = next_snapshot(&mut body, &mut pending, &mut denied_bytes).await;
        assert_eq!(first["total"], 2);
        assert_eq!(first["returned"], 2);
        assert_eq!(first["rows"].as_array().unwrap().len(), 2);

        retitle(&app, "sse-visible", "visible peer retitled").await;
        let second = next_snapshot(&mut body, &mut pending, &mut denied_bytes).await;
        assert_eq!(second["total"], 2);
        assert_eq!(second["rows"].as_array().unwrap().len(), 2);
        assert_ne!(second["seq"], first["seq"]);
        assert!(denied_bytes.contains("visible peer retitled"));
        for secret in ["denied peer title", "sse-denied", guarded_path.as_str()] {
            assert!(
                !denied_bytes.contains(secret),
                "'{secret}' reached the stream: {denied_bytes}"
            );
        }

        let caller = app
            .chat
            .sessions
            .read()
            .await
            .get("sse-caller")
            .cloned()
            .unwrap();
        caller.lock().await.thread.model = "trusted/model".into();
        let mut disclosed_bytes = String::new();
        let third = next_snapshot(&mut body, &mut pending, &mut disclosed_bytes).await;
        assert_eq!(third["total"], 3);
        assert_eq!(third["rows"].as_array().unwrap().len(), 3);
        assert_ne!(third["snapshot_id"], first["snapshot_id"]);
        assert!(disclosed_bytes.contains("denied peer title"));
        assert!(disclosed_bytes.contains(guarded_path.as_str()));
        tokio::time::resume();
    }

    #[tokio::test]
    async fn no_token_mode_requires_real_loopback_transport() {
        let (app, _dir) = app(None).await;
        assert_eq!(
            request(app.clone(), "/work", None, "192.168.1.4:7").await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(app.clone(), "/work", None, "127.0.0.1:7").await.0,
            StatusCode::OK
        );
        assert!(authorize(&app, &HeaderMap::new(), None).is_err());
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(authorize(&app, &headers, Some("127.0.0.1:7".parse().unwrap())).is_err());
    }
}
