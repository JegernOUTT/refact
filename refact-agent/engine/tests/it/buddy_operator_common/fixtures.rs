use std::{
    collections::HashSet,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::{sync::oneshot, task::JoinHandle};

#[derive(Clone)]
struct FixtureState {
    root: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
}

pub struct FixtureEndpoints {
    pub address: SocketAddr,
    calls: Arc<Mutex<Vec<String>>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl FixtureEndpoints {
    pub fn start(root: PathBuf) -> Self {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = FixtureState {
            root,
            calls: calls.clone(),
        };
        let router = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(completion))
            .route("/mcp", post(mcp))
            .route(
                "/redirect",
                get(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", "http://203.0.113.1/forbidden")],
                    )
                }),
            )
            .with_state(state);
        let (stop, stopped) = oneshot::channel();
        let server = axum::Server::from_tcp(listener)
            .unwrap()
            .serve(router.into_make_service());
        let task = tokio::spawn(async move {
            server
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            address,
            calls,
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub async fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.await.unwrap();
        }
    }
}

impl Drop for FixtureEndpoints {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn models(State(state): State<FixtureState>) -> Json<Value> {
    state.calls.lock().unwrap().push("models".into());
    Json(
        json!({"object":"list","data":[{"id":"fixture-model","object":"model","owned_by":"scratch"}]}),
    )
}

async fn completion(
    State(state): State<FixtureState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if body["model"] != "fixture-model" || body["stream"] == true || !body["messages"].is_array() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"message":"unsupported fixture request"}})),
        );
    }
    state.calls.lock().unwrap().push("completion".into());
    (
        StatusCode::OK,
        Json(
            json!({"id":"fixture-completion","object":"chat.completion","created":0,"model":"fixture-model","choices":[{"index":0,"message":{"role":"assistant","content":"---\nname: fixture-skill\n---\n# Fixture skill\n"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
        ),
    )
}

async fn mcp(
    State(state): State<FixtureState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let method = body["method"].as_str().unwrap_or("");
    state.calls.lock().unwrap().push(format!("mcp:{method}"));
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"buddy-scratch","version":"1"}})
        }
        "notifications/initialized" => return (StatusCode::ACCEPTED, Json(Value::Null)),
        "tools/list" => {
            json!({"tools":[{"name":"write_scratch","description":"Write the fixed scratch marker","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}]})
        }
        "tools/call"
            if body["params"]["name"] == "write_scratch"
                && body["params"]["arguments"] == json!({}) =>
        {
            tokio::fs::write(state.root.join("mcp-effect.txt"), b"fixture MCP effect\n")
                .await
                .unwrap();
            json!({"content":[{"type":"text","text":"fixture MCP effect written"}],"isError":false})
        }
        _ => {
            return (
                StatusCode::OK,
                Json(
                    json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32601,"message":"Unsupported fixture method or arguments"}}),
                ),
            )
        }
    };
    (
        StatusCode::OK,
        Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result})),
    )
}

pub fn require_full_process_confinement(
    enforcement: refact_sandbox::Enforcement,
) -> Result<(), String> {
    if enforcement != refact_sandbox::Enforcement::Full {
        return Err(
            "unsupported: full bubblewrap filesystem/network confinement is required".into(),
        );
    }
    Ok(())
}

pub async fn run_confined_process(
    root: &std::path::Path,
    user_config: &std::path::Path,
) -> Result<std::process::Output, String> {
    run_confined_process_with_provider(&refact_sandbox::BwrapProvider, root, user_config).await
}

pub async fn run_confined_process_with_provider(
    provider: &dyn refact_sandbox::SandboxProvider,
    root: &std::path::Path,
    user_config: &std::path::Path,
) -> Result<std::process::Output, String> {
    use refact_sandbox::{ExecSandboxSpec, SandboxMode};
    require_full_process_confinement(provider.probe())?;
    let spec = ExecSandboxSpec {
        mode: SandboxMode::WorkspaceWrite,
        ro_paths: vec![std::path::PathBuf::from("/")],
        rw_paths: vec![root.to_path_buf(), user_config.to_path_buf()],
        allow_network: false,
    };
    let (launcher, args) = provider.confine(&spec, "/bin/sh", &[
        "-c".into(),
        "set -eu; printf 'fixture process effect\\n' > process-effect.txt; printf '{\"effect\":\"process-effect.txt\"}\\n'".into(),
    ]).map_err(|error| error.to_string())?;
    tokio::process::Command::new(launcher)
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("HOME", user_config)
        .env("XDG_CONFIG_HOME", user_config)
        .env("PATH", "/usr/bin:/bin")
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| error.to_string())
}

pub async fn full_confinement_process_acceptance_scenario() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let user_config = tempfile::tempdir().map_err(|error| error.to_string())?;
    let output = run_confined_process(project.path(), user_config.path()).await?;
    if !output.status.success() {
        return Err(format!(
            "confined process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let receipt: Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    if receipt != json!({"effect":"process-effect.txt"}) {
        return Err("confined process returned an unexpected receipt".into());
    }
    let persisted = tokio::fs::read_to_string(project.path().join("process-effect.txt"))
        .await
        .map_err(|error| error.to_string())?;
    if persisted != "fixture process effect\n" {
        return Err("confined process receipt has no matching persisted effect".into());
    }
    Ok(())
}

pub struct FixtureTransport {
    client: reqwest::Client,
    allowed: HashSet<SocketAddr>,
    denied: Mutex<Vec<String>>,
}

impl FixtureTransport {
    pub fn new(allowed: impl IntoIterator<Item = SocketAddr>) -> Self {
        let allowed: HashSet<_> = allowed.into_iter().collect();
        assert!(allowed.iter().all(|address| address.ip().is_loopback()));
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            allowed,
            denied: Mutex::new(Vec::new()),
        }
    }

    pub fn denied(&self) -> Vec<String> {
        self.denied.lock().unwrap().clone()
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response, String> {
        let parsed = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
        let address = parsed
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .zip(parsed.port())
            .map(|(ip, port)| SocketAddr::new(ip, port));
        if parsed.scheme() != "http"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(
                (method.as_str(), parsed.path()),
                ("GET", "/v1/models" | "/redirect") | ("POST", "/v1/chat/completions" | "/mcp")
            )
            || !address.is_some_and(|address| self.allowed.contains(&address))
        {
            self.denied.lock().unwrap().push(url.into());
            return Err("fixture transport denied destination before DNS/connect".into());
        }
        let mut request = self
            .client
            .request(method, parsed)
            .bearer_auth("scratch-only-not-a-secret");
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.map_err(|error| error.to_string())
    }
}
