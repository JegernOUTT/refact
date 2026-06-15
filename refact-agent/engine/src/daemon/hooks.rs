use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::daemon::config::HookKind;
use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::DaemonState;
use crate::daemon::supervisor::WorkerInfo;

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct HookBody {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    deliver: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WorkerHookFire {
    kind: HookKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deliver: Option<Value>,
}

#[derive(Debug)]
enum HookRoute {
    Wake,
    Agent,
    Named(String),
}

#[derive(Debug)]
struct HookError {
    status: StatusCode,
    message: String,
}

impl HookError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

pub(crate) async fn wake(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(body): Json<HookBody>,
) -> Response {
    dispatch(state, HookRoute::Wake, body).await
}

pub(crate) async fn agent(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(body): Json<HookBody>,
) -> Response {
    dispatch(state, HookRoute::Agent, body).await
}

pub(crate) async fn named(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<HookBody>,
) -> Response {
    dispatch(state, HookRoute::Named(name), body).await
}

pub(crate) async fn bare(State((state, _)): State<(Arc<DaemonState>, u16)>) -> Response {
    if !state.config.hooks.enabled {
        return HookError::new(StatusCode::NOT_FOUND, "hooks are disabled").into_response();
    }
    HookError::new(StatusCode::BAD_REQUEST, "missing hook name").into_response()
}

async fn dispatch(state: Arc<DaemonState>, route: HookRoute, body: HookBody) -> Response {
    if !state.config.hooks.enabled {
        return HookError::new(StatusCode::NOT_FOUND, "hooks are disabled").into_response();
    }
    match build_fire_payload(&state, route, body).await {
        Ok((entry, payload)) => match ensure_ready_worker(&state, &entry).await {
            Ok(worker) => match forward_to_worker(&state, &worker, &payload).await {
                Ok(worker_response) => Json(json!({
                    "success": true,
                    "project_id": entry.id,
                    "worker": worker_response,
                }))
                .into_response(),
                Err(error) => error.into_response(),
            },
            Err(error) => error.into_response(),
        },
        Err(error) => error.into_response(),
    }
}

async fn build_fire_payload(
    state: &Arc<DaemonState>,
    route: HookRoute,
    body: HookBody,
) -> Result<(ProjectEntry, WorkerHookFire), HookError> {
    let hooks = &state.config.hooks;
    let (kind, hook_id, mapping) = match route {
        HookRoute::Wake => (HookKind::Wake, None, None),
        HookRoute::Agent => (HookKind::Agent, None, None),
        HookRoute::Named(name) => {
            let mapping = hooks.mappings.get(&name).ok_or_else(|| {
                HookError::new(StatusCode::NOT_FOUND, format!("hook not found: {name}"))
            })?;
            (mapping.kind.clone(), Some(name), Some(mapping))
        }
    };
    let project = body
        .project
        .as_deref()
        .or_else(|| mapping.and_then(|mapping| mapping.project.as_deref()))
        .or(hooks.default_project.as_deref());
    let entry = resolve_project(state, project).await?;
    enforce_allowed_project(hooks.allowed_projects.as_deref(), &entry)?;
    let payload = match kind {
        HookKind::Wake => WorkerHookFire {
            kind,
            hook_id,
            text: Some(required_string(body.text, "text")?),
            message: None,
            mode: None,
            model: None,
            deliver: None,
        },
        HookKind::Agent => WorkerHookFire {
            kind,
            hook_id,
            text: None,
            message: Some(required_string(body.message, "message")?),
            mode: mapping
                .and_then(|mapping| mapping.mode.clone())
                .or(body.mode),
            model: mapping
                .and_then(|mapping| mapping.model.clone())
                .or(body.model),
            deliver: mapping
                .and_then(|mapping| mapping.deliver.clone())
                .or(body.deliver),
        },
    };
    Ok((entry, payload))
}

fn required_string(value: Option<String>, field: &str) -> Result<String, HookError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| HookError::new(StatusCode::BAD_REQUEST, format!("missing {field}")))
}

async fn resolve_project(
    state: &Arc<DaemonState>,
    project: Option<&str>,
) -> Result<ProjectEntry, HookError> {
    let project = project
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HookError::new(StatusCode::BAD_REQUEST, "missing project"))?;
    let registry = state.projects.read().await;
    if let Some(entry) = registry.get(project) {
        return Ok(entry.clone());
    }
    let by_slug = registry
        .list()
        .into_iter()
        .find(|entry| entry.slug == project);
    if let Some(entry) = by_slug {
        return Ok(entry);
    }
    let root = crate::files_correction::canonical_path(project.to_string());
    registry
        .list()
        .into_iter()
        .find(|entry| entry.root == root)
        .ok_or_else(|| HookError::new(StatusCode::NOT_FOUND, "project not found"))
}

fn enforce_allowed_project(
    allowed_projects: Option<&[String]>,
    entry: &ProjectEntry,
) -> Result<(), HookError> {
    let Some(allowed_projects) = allowed_projects else {
        return Ok(());
    };
    let root = entry.root.to_string_lossy();
    if allowed_projects
        .iter()
        .any(|project| project == &entry.id || project == &entry.slug || project == root.as_ref())
    {
        Ok(())
    } else {
        Err(HookError::new(
            StatusCode::FORBIDDEN,
            "project is not allowed for hooks",
        ))
    }
}

async fn ensure_ready_worker(
    state: &Arc<DaemonState>,
    entry: &ProjectEntry,
) -> Result<WorkerInfo, HookError> {
    let worker = state
        .supervisor
        .ensure_ready_worker(entry)
        .await
        .map_err(|error| {
            HookError::new(
                StatusCode::BAD_GATEWAY,
                format!("failed to wake worker: {error}"),
            )
        })?;
    Ok(worker)
}

async fn forward_to_worker(
    state: &Arc<DaemonState>,
    worker: &WorkerInfo,
    payload: &WorkerHookFire,
) -> Result<Value, HookError> {
    let url = format!("http://127.0.0.1:{}/v1/hooks/fire", worker.http_port);
    let mut request = state.proxy_client.post(url).json(payload);
    if let Some(token) = worker_forward_token(state) {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|error| {
        HookError::new(
            StatusCode::BAD_GATEWAY,
            crate::daemon::auth::redact_daemon_token(&format!("failed to forward hook: {error}")),
        )
    })?;
    let status = response.status();
    let text = response.text().await.map_err(|error| {
        HookError::new(
            StatusCode::BAD_GATEWAY,
            crate::daemon::auth::redact_daemon_token(&format!(
                "failed to read hook response: {error}"
            )),
        )
    })?;
    if !status.is_success() {
        return Err(HookError::new(
            StatusCode::BAD_GATEWAY,
            crate::daemon::auth::redact_daemon_token(&format!(
                "worker hook failed with status {}: {}",
                status.as_u16(),
                text
            )),
        ));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|error| {
        HookError::new(
            StatusCode::BAD_GATEWAY,
            format!("worker returned invalid JSON: {error}"),
        )
    })
}

fn worker_forward_token(state: &DaemonState) -> Option<&str> {
    state.auth_token.as_deref().or_else(|| {
        if state.config.hooks.enabled {
            state
                .config
                .hooks
                .token
                .as_deref()
                .filter(|token| !token.is_empty())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::{AuthConfig, DaemonConfig, HookMapping, HooksConfig};
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectRegistry;
    use hyper::{Body as HyperBody, Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn fake_worker() -> Option<Self> {
            let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
            if std::process::Command::new(&python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![(
                "REFACT_DAEMON_WORKER_CMD",
                std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
            )];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python.as_str(), script.to_string_lossy().as_ref()]),
            );
            Some(Self { keys })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    async fn test_state(config: DaemonConfig, dir: &tempfile::TempDir) -> Arc<DaemonState> {
        let auth_token = config
            .auth
            .enabled
            .then(|| config.auth.token.clone())
            .flatten();
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            auth_token,
        );
        *state.projects.write().await = ProjectRegistry::empty(dir.path().join("projects.json"));
        state
    }

    async fn add_project(state: &Arc<DaemonState>, root: std::path::PathBuf) -> ProjectEntry {
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        entry
    }

    fn hook_config(enabled: bool) -> DaemonConfig {
        DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("daemon-secret".to_string()),
            },
            hooks: HooksConfig {
                enabled,
                token: Some("hook-secret".to_string()),
                ..HooksConfig::default()
            },
            ..DaemonConfig::default()
        }
    }

    async fn request_json(
        router: axum::Router,
        uri: &str,
        token: Option<&str>,
        header_name: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = if header_name == "authorization" {
                builder.header("Authorization", format!("Bearer {token}"))
            } else {
                builder.header("x-refact-token", token)
            };
        }
        let response = router
            .oneshot(builder.body(HyperBody::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    #[tokio::test]
    async fn hook_auth_rejects_missing_invalid_and_query_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(hook_config(true), &dir).await;
        let router = crate::daemon::server::make_router(state, 8488);
        let body = json!({"project":"missing","text":"hello"});

        let (status, _) = request_json(
            router.clone(),
            "/hooks/wake",
            None,
            "authorization",
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = request_json(
            router.clone(),
            "/hooks/wake",
            Some("wrong"),
            "authorization",
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = request_json(
            router,
            "/hooks/wake?daemon_token=hook-secret",
            Some("hook-secret"),
            "authorization",
            body,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn hook_auth_rejects_encoded_query_token_even_with_header() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(hook_config(true), &dir).await;
        let router = crate::daemon::server::make_router(state, 8488);

        let (status, _) = request_json(
            router,
            "/hooks/wake?d%61emon_token=hook-secret",
            Some("hook-secret"),
            "authorization",
            json!({"project":"missing","text":"hello"}),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn disabled_hooks_are_blocked_after_auth() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(hook_config(false), &dir).await;
        let router = crate::daemon::server::make_router(state, 8488);

        let (status, value) = request_json(
            router,
            "/hooks/wake",
            Some("hook-secret"),
            "authorization",
            json!({"project":"missing","text":"hello"}),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(value["error"], "hooks are disabled");
    }

    #[tokio::test]
    async fn bare_hooks_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(hook_config(true), &dir).await;
        let router = crate::daemon::server::make_router(state, 8488);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hooks")
                    .header("Authorization", "Bearer hook-secret")
                    .body(HyperBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn wake_resolves_body_project_wakes_worker_and_forwards() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let state = test_state(hook_config(true), &dir).await;
        let entry = add_project(&state, project_root).await;
        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let (status, value) = request_json(
            router,
            "/hooks/wake",
            Some("hook-secret"),
            "authorization",
            json!({"project": entry.id, "text": "wake up"}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["success"], true);
        assert_eq!(value["worker"]["path"], "/v1/hooks/fire");
        assert_eq!(
            value["worker"]["headers"]["authorization"],
            "Bearer daemon-secret"
        );
        let forwarded: Value =
            serde_json::from_str(value["worker"]["body_text"].as_str().unwrap()).unwrap();
        assert_eq!(forwarded["kind"], "wake");
        assert_eq!(forwarded["text"], "wake up");
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn named_agent_uses_mapping_defaults_and_x_refact_token() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let mut config = hook_config(true);
        let temporary_state = test_state(config.clone(), &dir).await;
        let entry = add_project(&temporary_state, project_root.clone()).await;
        config.hooks.mappings.insert(
            "deploy".to_string(),
            HookMapping {
                project: Some(entry.id.clone()),
                kind: HookKind::Agent,
                mode: Some("agent".to_string()),
                model: Some("test-model".to_string()),
                deliver: Some(json!({"type":"chat"})),
            },
        );
        let state = test_state(config, &dir).await;
        let reopened = add_project(&state, project_root).await;
        assert_eq!(reopened.id, entry.id);
        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let (status, value) = request_json(
            router,
            "/hooks/deploy",
            Some("hook-secret"),
            "x-refact-token",
            json!({"message": "ship it", "mode": "ignored"}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let forwarded: Value =
            serde_json::from_str(value["worker"]["body_text"].as_str().unwrap()).unwrap();
        assert_eq!(forwarded["kind"], "agent");
        assert_eq!(forwarded["hook_id"], "deploy");
        assert_eq!(forwarded["message"], "ship it");
        assert_eq!(forwarded["mode"], "agent");
        assert_eq!(forwarded["model"], "test-model");
        assert_eq!(forwarded["deliver"], json!({"type":"chat"}));
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn agent_uses_default_project_when_body_omits_project() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let mut config = hook_config(true);
        let temporary_state = test_state(config.clone(), &dir).await;
        let entry = add_project(&temporary_state, project_root.clone()).await;
        config.hooks.default_project = Some(entry.id.clone());
        let state = test_state(config, &dir).await;
        let entry = add_project(&state, project_root).await;
        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let (status, value) = request_json(
            router,
            "/hooks/agent",
            Some("hook-secret"),
            "authorization",
            json!({"message": "hello"}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["project_id"], entry.id);
        state.supervisor.stop_all().await;
    }

    #[tokio::test]
    async fn allowed_projects_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("project");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let mut config = hook_config(true);
        config.hooks.allowed_projects = Some(vec!["other".to_string()]);
        let state = test_state(config, &dir).await;
        let entry = add_project(&state, project_root).await;
        let router = crate::daemon::server::make_router(state.clone(), 8488);

        let (status, value) = request_json(
            router,
            "/hooks/agent",
            Some("hook-secret"),
            "authorization",
            json!({"project": entry.id, "message": "hello"}),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(value["error"], "project is not allowed for hooks");
        assert_eq!(state.supervisor.worker_count().await, 0);
    }

    #[test]
    fn worker_forward_token_prefers_daemon_token() {
        let config = hook_config(true);
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("daemon-secret".to_string()),
        );

        assert_eq!(worker_forward_token(&state), Some("daemon-secret"));
    }

    #[test]
    fn worker_forward_token_falls_back_to_hook_token() {
        let mut config = hook_config(true);
        config.auth.enabled = false;
        config.auth.token = None;
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(config, EventBus::new(dir.path().join("events.jsonl")), None);

        assert_eq!(worker_forward_token(&state), Some("hook-secret"));
    }
}
