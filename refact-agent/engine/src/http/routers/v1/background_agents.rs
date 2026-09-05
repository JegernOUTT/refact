use axum::extract::{Path, Query, State};
use axum::response::Json;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use refact_core::chat_types::{PendingDelivery, PushMode};
use crate::agents::types::{BackgroundAgent, BackgroundAgentSummary};
use crate::app_state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

#[derive(Debug, Deserialize)]
pub struct BackgroundAgentsQuery {
    pub chat_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default = "default_subtree")]
    pub subtree: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    #[serde(default)]
    pub chat_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub push: PushMode,
}

fn default_subtree() -> bool {
    true
}

fn api_error(status: StatusCode, error: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "error": error.into(),
            "code": match status {
                StatusCode::NOT_FOUND => "not_found",
                StatusCode::CONFLICT => "conflict",
                StatusCode::BAD_REQUEST => "bad_request",
                _ => "background_agent_error",
            },
        })),
    )
}

fn belongs_to_chat(record: &BackgroundAgent, chat_id: &str) -> bool {
    record.parent_chat_id == chat_id || record.parent_root_chat_id.as_deref() == Some(chat_id)
}

fn verify_optional_chat_ownership(
    record: &BackgroundAgent,
    chat_id: Option<&str>,
) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.trim().is_empty()) else {
        // TODO: require chat_id after GUI T-15 sends it for every request.
        tracing::warn!(agent_id = %record.agent_id, "background agent request omitted chat_id");
        return Ok(());
    };
    if belongs_to_chat(record, chat_id) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "Background agent does not belong to this chat",
        ))
    }
}

async fn root_chat_id(app: &AppState, chat_id: &str) -> String {
    app.agents
        .list_all()
        .await
        .into_iter()
        .find(|record| record.child_chat_id.as_deref() == Some(chat_id))
        .and_then(|record| record.parent_root_chat_id)
        .unwrap_or_else(|| chat_id.to_string())
}

fn summary(record: &BackgroundAgent) -> BackgroundAgentSummary {
    BackgroundAgentSummary::from(record)
}

pub async fn handle_v1_background_agents_get(
    State(app): State<AppState>,
    Query(query): Query<BackgroundAgentsQuery>,
) -> Json<Vec<BackgroundAgentSummary>> {
    let root_chat_id = root_chat_id(&app, &query.chat_id).await;
    let records = crate::tools::tool_agent_interact::records_for_root(&app, &root_chat_id).await;
    Json(
        records
            .into_iter()
            .map(|(record, _)| summary(&record))
            .collect(),
    )
}

pub async fn handle_v1_background_agents_cancel(
    State(app): State<AppState>,
    Path(agent_id): Path<String>,
    Json(request): Json<CancelRequest>,
) -> ApiResult<Value> {
    let record = app
        .agents
        .get_any(&agent_id)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "Background agent not found"))?;
    verify_optional_chat_ownership(&record, request.chat_id.as_deref())?;
    let cancelled = app
        .agents
        .cancel_subtree(
            &record.parent_chat_id,
            &agent_id,
            request.subtree,
            request.reason,
        )
        .await
        .map_err(|error| api_error(StatusCode::CONFLICT, error))?;
    for record in &cancelled {
        crate::agents::spawn::emit_background_agent_update(app.clone(), record).await;
    }
    let cancelled_agent_ids = cancelled
        .iter()
        .map(|record| record.agent_id.clone())
        .collect::<Vec<_>>();
    let agents = cancelled.iter().map(summary).collect::<Vec<_>>();
    Ok(Json(
        json!({ "cancelled_agent_ids": cancelled_agent_ids, "agents": agents }),
    ))
}

pub async fn handle_v1_background_agents_message(
    State(app): State<AppState>,
    Path(agent_id): Path<String>,
    Json(request): Json<MessageRequest>,
) -> ApiResult<Value> {
    let text = request.text.trim();
    if text.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Message text must not be empty",
        ));
    }
    let record = app
        .agents
        .get_any(&agent_id)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "Background agent not found"))?;
    verify_optional_chat_ownership(&record, request.chat_id.as_deref())?;
    if record.status.is_terminal() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "Background agent is not running",
        ));
    }
    let outcome = crate::agents::delivery::deliver_to_agent(
        app.clone(),
        &agent_id,
        PendingDelivery::new(
            vec![crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::SystemNotice,
                "agents.message",
                json!({"from": "user"}),
                format!("[message from user]\n{text}"),
            )],
            request.push,
            "agents.message".to_string(),
            true,
        ),
    )
    .await
    .map_err(|error| api_error(StatusCode::CONFLICT, error))?;
    crate::agents::spawn::emit_background_agent_update(app, &record).await;
    Ok(Json(
        json!({ "agent": summary(&record), "queued": true, "outcome": outcome }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    use crate::agents::types::{BgAgentKind, CreateAgentRequest};

    fn create_request(parent_chat_id: &str, title: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: Some("root".to_string()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".to_string(),
            title: title.to_string(),
            prompt: title.to_string(),
            target_files: vec![],
            model: "test/model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    async fn app() -> AppState {
        AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await
    }

    async fn request(app: AppState, method: &str, uri: String, body: Value) -> (StatusCode, Value) {
        let response = crate::http::routers::make_refact_http_server(app)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body()).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    #[test]
    fn message_push_defaults_and_validates_modes() {
        let request: MessageRequest = serde_json::from_value(json!({"text": "hello"})).unwrap();
        assert_eq!(request.push, PushMode::Append);
        for mode in [PushMode::Append, PushMode::Preempt, PushMode::WhenIdle] {
            let request: MessageRequest =
                serde_json::from_value(json!({"text": "hello", "push": mode})).unwrap();
            assert_eq!(request.push, mode);
        }
        assert!(
            serde_json::from_value::<MessageRequest>(json!({"text": "hello", "push": true}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn message_preserves_explicit_push_for_queued_agent() {
        let app = app().await;
        let (agent, _, _) = app
            .agents
            .create(create_request("root", "child"))
            .await
            .unwrap();
        let (status, _) = request(
            app.clone(),
            "POST",
            format!("/v1/background-agents/{}/message", agent.agent_id),
            json!({"chat_id": "root", "text": "hello", "push": "when_idle"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let record = app.agents.get_any(&agent.agent_id).await.unwrap();
        assert_eq!(record.pending_deliveries.len(), 1);
        assert_eq!(record.pending_deliveries[0].push, PushMode::WhenIdle);
    }

    #[tokio::test]
    async fn message_returns_not_found_and_terminal_conflict() {
        let app = app().await;
        let (status, _) = request(
            app.clone(),
            "POST",
            "/v1/background-agents/missing/message".to_string(),
            json!({ "text": "hello" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (agent, _, _) = app
            .agents
            .create(create_request("root", "done"))
            .await
            .unwrap();
        app.agents
            .mark_cancelled(&agent.agent_id, Some("done".to_string()))
            .await
            .unwrap();
        let (status, _) = request(
            app,
            "POST",
            format!("/v1/background-agents/{}/message", agent.agent_id),
            json!({ "text": "hello" }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn get_tree_and_cancel_subtree_return_summaries() {
        let app = app().await;
        let (child, child_abort, _) = app
            .agents
            .create(create_request("root", "child"))
            .await
            .unwrap();
        app.agents
            .mark_running(&child.agent_id, "child-chat".to_string())
            .await
            .unwrap();
        let (grandchild, grandchild_abort, _) = app
            .agents
            .create(create_request("child-chat", "grandchild"))
            .await
            .unwrap();
        app.agents
            .mark_running(&grandchild.agent_id, "grandchild-chat".to_string())
            .await
            .unwrap();

        let (status, tree) = request(
            app.clone(),
            "GET",
            "/v1/background-agents?chat_id=child-chat".to_string(),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tree.as_array().unwrap().len(), 2);

        let (status, cancelled) = request(
            app,
            "POST",
            format!("/v1/background-agents/{}/cancel", child.agent_id),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            cancelled["cancelled_agent_ids"].as_array().unwrap().len(),
            2
        );
        assert!(child_abort.load(std::sync::atomic::Ordering::SeqCst));
        assert!(grandchild_abort.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn message_and_cancel_enforce_present_chat_ownership() {
        let app = app().await;
        let (agent, _, _) = app
            .agents
            .create(create_request("root", "child"))
            .await
            .unwrap();

        let (status, _) = request(
            app.clone(),
            "POST",
            format!("/v1/background-agents/{}/message", agent.agent_id),
            json!({ "chat_id": "other", "text": "hello" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = request(
            app.clone(),
            "POST",
            format!("/v1/background-agents/{}/cancel", agent.agent_id),
            json!({ "chat_id": "other" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = request(
            app,
            "POST",
            format!("/v1/background-agents/{}/message", agent.agent_id),
            json!({ "chat_id": "root", "text": "hello" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
