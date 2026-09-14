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

fn authorize(
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

pub async fn list(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<crate::coordination::types::WorkOverview>, WorkError> {
    authorize(&app, &headers, peer.map(|p| p.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid Work query"))?;
    let caller = caller(&app, &query).await?;
    Ok(Json(
        service::overview(&app, &caller, overview_request(&query)?).await?,
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
        let (app, _dir) = app(Some("work-test-token")).await;
        for token in [None, Some("wrong")] {
            let (status, body) = request(app.clone(), "/work", token, "127.0.0.1:7").await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["code"], "unauthorized");
            assert!(body.get("rows").is_none());
        }
        let (status, body) =
            request(app.clone(), "/work", Some("work-test-token"), "127.0.0.1:7").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["coverage"], "partial");
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
