//! Test-only production AppState/router worker and HTTP acceptance controller.
//! Both run inside the shell launcher's default-deny OS network namespace.
use std::{path::Path, sync::Arc, time::Duration};

use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
use refact_lsp::{
    app_state::AppState,
    buddy::{actor::BuddyService, outcome_runtime::OutcomeRuntime},
    global_context::{create_global_context, CommandLine},
};
use serde_json::{json, Value};
use tokio::{io::AsyncReadExt, process::Command};

const ROOT: &str = "/scratch/project";
const BASE: &str = "http://127.0.0.1:39191/v1";
const TOKEN: &str = "scratch-only-not-a-secret";
const QUESTION: &str = "/buddy/decisions/questions/b02-native/b02-question";

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn require_namespace(
    host: Option<&str>,
    current: &str,
    interfaces: &[String],
) -> Result<(), String> {
    if host.is_none_or(|host| host.is_empty() || host == current) {
        return Err("B02 REFUSED: whole-worker network namespace was not established".into());
    }
    if interfaces != ["lo"] {
        return Err("B02 REFUSED: non-loopback interface inside worker namespace".into());
    }
    Ok(())
}

fn confinement() -> TestResult {
    let current = std::fs::read_link("/proc/self/ns/net")?;
    let interfaces = std::fs::read_to_string("/proc/net/dev")?
        .lines()
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
        .collect::<Vec<_>>();
    require_namespace(
        std::env::var("B02_HOST_NET").ok().as_deref(),
        &current.to_string_lossy(),
        &interfaces,
    )?;
    if std::env::var("HOME")? != "/scratch/home"
        || Path::new("/home").exists()
        || Path::new("/run").exists()
    {
        return Err("B02 REFUSED: host filesystem/config is visible".into());
    }
    let status = std::fs::read_to_string("/proc/self/status")?;
    if !status
        .lines()
        .any(|line| line == "CapEff:\t0000000000000000")
    {
        return Err("B02 REFUSED: namespace process retained effective capabilities".into());
    }
    Ok(())
}

pub async fn entry() -> TestResult {
    match std::env::var("B02_MODE").as_deref() {
        Ok("controller") => {
            confinement()?;
            controller().await
        }
        Ok("worker") => {
            confinement()?;
            serve().await
        }
        _ => Err("B02 REFUSED: launch via tools/dev/buddy-operator-test.sh".into()),
    }
}

async fn serve() -> TestResult {
    let root = Path::new(ROOT);
    let mut cmd = CommandLine::from_iter_safe([
        "operator-worker",
        "--no-scheduler",
        "--workspace-folder",
        ROOT,
        "--logs-stderr",
    ])?;
    cmd.daemon_auth_token = Some(TOKEN.into());
    let (gcx, _shutdown_receiver) =
        create_global_context("/scratch/cache".into(), "/scratch/config".into(), cmd).await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.into()];
    let app = AppState::from_gcx(gcx.clone()).await;
    let runtime = OutcomeRuntime::open(
        root,
        project(),
        WorkerIdentity {
            worker_id: "b02-worker".into(),
            engine_instance_id: uuid::Uuid::new_v4().to_string(),
            epoch: 1,
        },
    )?;
    let (tx, _) = tokio::sync::broadcast::channel(32);
    let mut buddy = BuddyService::new(
        root.into(),
        refact_lsp::buddy::state::default_buddy_state(),
        Default::default(),
        vec![],
        Default::default(),
        tx,
        None,
    );
    buddy.operator_features.storage = true;
    buddy.operator_features.actions = true;
    assert!(!buddy.operator_features.supervision);
    assert!(!buddy.operator_features.improvement);
    assert!(!buddy.operator_features.editorial);
    buddy.outcome_runtime = Some(runtime.clone());
    *app.buddy.buddy.lock().await = Some(buddy);
    let restored = refact_lsp::chat::try_restore_session_if_trajectory_exists(
        app.clone(),
        &app.chat.sessions,
        "b02-native",
    )
    .await;
    if !restored {
        let mut session = refact_lsp::chat::types::ChatSession::new("b02-native".into());
        session.register_pending_questions(refact_chat_api::PendingQuestionSet {
            tool_call_id: "b02-question".into(),
            asked_at_ms: 7,
            questions: vec![refact_chat_api::PendingQuestion {
                id: "continue".into(),
                question_type: "yes_no".into(),
                text: "Continue scratch work?".into(),
                options: None,
            }],
            status: refact_chat_api::PendingQuestionStatus::Pending,
            answers: vec![],
            resolution_id: None,
        })?;
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("b02-native".into(), session.clone());
        if !refact_lsp::chat::trajectories::try_save_trajectory(app.clone(), session).await? {
            return Err("native fixture question was not durably saved".into());
        }
    }
    let router = refact_lsp::http::routers::make_refact_http_server(app);
    let listener = std::net::TcpListener::bind("127.0.0.1:39191")?;
    listener.set_nonblocking(true)?;
    axum::Server::from_tcp(listener)?
        .serve(router.into_make_service())
        .with_graceful_shutdown(async {
            let mut byte = [0];
            let _ = tokio::io::stdin().read(&mut byte).await;
        })
        .await?;
    gcx.shutdown_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    runtime.shutdown().await;
    Ok(())
}

fn project() -> ProjectIdentity {
    ProjectIdentity {
        project_id: "b02-scratch".into(),
        canonical_root: ROOT.into(),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn request(
    client: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> TestResult<Value> {
    let mut request = client
        .request(method, format!("{BASE}{path}"))
        .bearer_auth(TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(format!("{path}: {status}: {text}").into());
    }
    Ok(serde_json::from_str(&text)?)
}

async fn post(client: &reqwest::Client, path: &str, body: Value) -> TestResult<Value> {
    request(client, reqwest::Method::POST, path, Some(body)).await
}

async fn get(client: &reqwest::Client, path: &str) -> TestResult<Value> {
    request(client, reqwest::Method::GET, path, None).await
}

// Native answer handling persists the decision before resuming generation, which
// may dirty the trajectory again. Read the durable view, not that transient 503.
async fn durable_question(client: &reqwest::Client, path: &str) -> TestResult<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let response = client
            .get(format!("{BASE}{path}"))
            .bearer_auth(TOKEN)
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if status.is_success() && value["question"]["status"] == "answered" {
            return Ok(value);
        }
        let pending = status.is_success() && value["question"]["status"] == "pending";
        let saving = status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            && value["detail"] == "native decision is awaiting durable acknowledgment";
        if !pending && !saving {
            return Err(format!("{path}: {status}: {value}").into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("native command did not durably resolve question".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn ready(
    child: &mut tokio::process::Child,
    client: &reqwest::Client,
    url: &str,
) -> TestResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(format!("worker exited before HTTP readiness: {status}").into());
        }
        if let Ok(response) = client.get(url).bearer_auth(TOKEN).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("worker HTTP startup deadline exceeded".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn start(client: &reqwest::Client) -> TestResult<tokio::process::Child> {
    let mut child = Command::new("/scratch/bin/operator-test")
        .args([
            "--exact",
            "buddy_operator::whole_worker_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("B02_MODE", "worker")
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    ready(
        &mut child,
        client,
        &format!("{BASE}/buddy/decisions/current"),
    )
    .await?;
    Ok(child)
}

async fn stop(mut child: tokio::process::Child) -> TestResult {
    drop(child.stdin.take());
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait()).await??;
    if !status.success() {
        return Err(format!("worker shutdown failed: {status}").into());
    }
    Ok(())
}

// All lifecycle IPC is private scratch filesystem state, not an added production API.
async fn browser_controller(client: &reqwest::Client) -> TestResult {
    use refact_buddy_core::operator_types::{ArtifactRevision, WorkItem};
    use refact_lsp::buddy::{
        desk_actions::{attach_resource_action, WorkActionOrigin},
        operator_store::OperatorStore,
    };
    let worker = start(client).await?;
    let spec = json!({"schema_version":1,"action_id":"b02-browser-command","kind":"markdown_project_apply","adapter":"extensions","project":project(),"resource":{"resource_kind":"native_markdown","resource_id":"commands/b02-browser","version":"absent"},"expected_version":"absent","parameter_ref":"b02-browser-draft","availability":"available","confirmation":"user_confirmation_required","authority_class":"mutating","capabilities":{"can_preview":true,"can_execute":true,"can_verify":true,"can_recover":true,"can_cancel":false}});
    post(client, "/buddy/actions/draft", json!({"spec":spec,"draft":{"project":project(),"kind":"command","name":"b02-browser","description":"Scratch browser acceptance","body":"# Browser scratch effect\n"}})).await?;
    let prepared = post(client, "/buddy/actions/prepare", json!({"spec":spec})).await?;
    stop(worker).await?;
    {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))?;
        let mut work: WorkItem = serde_json::from_value(fixture["work_item"].clone())?;
        let mut artifact: ArtifactRevision =
            serde_json::from_value(fixture["artifact_revision"].clone())?;
        work.project = project();
        work.title = "Browser scratch change".into();
        artifact.project = project();
        artifact.revision = 1;
        artifact.title = "Browser scratch report".into();
        artifact.conclusion = "Review an isolated command-file change.".into();
        work.current_artifact_revision = Some(1);
        let mut store = OperatorStore::open(Path::new(ROOT), &project())?;
        store.append_artifact_revision(&artifact)?;
        let stored = store.upsert_work_item(&work, None)?;
        attach_resource_action(
            &mut store,
            WorkActionOrigin {
                schema_version: 1,
                project: project(),
                work_id: work.id,
                work_revision: stored.revision,
                artifact_id: artifact.artifact_id,
                artifact_revision: 1,
                spec_hash: prepared["data"]["spec_hash"]
                    .as_str()
                    .ok_or("missing prepared hash")?
                    .into(),
            },
        )?;
    }
    let mut worker = start(client).await?;
    let mut browser = Command::new("/usr/bin/node")
        .args([
            "node_modules/@playwright/test/cli.js",
            "test",
            "--config=playwright.operator.config.ts",
        ])
        .current_dir("/scratch/gui")
        .kill_on_drop(true)
        .spawn()?;
    let effect = Path::new(ROOT).join(".refact/commands/b02-browser.md");
    let mut restarts = 0;
    loop {
        if let Some(status) = browser.try_wait()? {
            if !status.success() {
                return Err(format!("B02 browser acceptance failed: {status}; Chromium sandbox must work, no fallback allowed").into());
            }
            break;
        }
        let request_path = format!("/scratch/restart-{}", restarts + 1);
        if Path::new(&request_path).exists() {
            let bytes = std::fs::read(&effect)?;
            assert!(String::from_utf8_lossy(&bytes).contains("Browser scratch effect"));
            let evidence: Value =
                serde_json::from_slice(&std::fs::read("/scratch/browser-evidence.json")?)?;
            let execute = evidence["execute"].clone();
            let id = execute["execution_id"]
                .as_str()
                .ok_or("browser execution ID missing")?;
            let path = format!("/buddy/executions/{id}?project_id=b02-scratch");
            let receipt = get(client, &path).await?;
            assert_eq!(receipt["data"]["value"]["execution_id"], id);
            stop(worker).await?;
            worker = start(client).await?;
            assert_eq!(get(client, &path).await?, receipt);
            assert_eq!(std::fs::read(&effect)?, bytes);
            let duplicate = post(client, "/buddy/actions/execute", execute).await?;
            assert!(duplicate["data"].get("Duplicate").is_some(), "{duplicate}");
            assert_eq!(std::fs::read(&effect)?, bytes);
            restarts += 1;
            std::fs::write(
                format!("/scratch/restarted-{restarts}"),
                "durable receipt and no duplicate effect verified",
            )?;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        restarts, 2,
        "browser must span unanswered and answered restarts"
    );
    stop(worker).await?;
    println!("B02 BROWSER PASS: production GUI controls to real scratch router; actual command effect, durable receipt, two native clients, stale conflicts and two process restarts without replay; intelligence off. IDE host parity remains untested.");
    Ok(())
}

async fn controller() -> TestResult {
    let client = client();
    let mut fixtures = super::fixtures::FixtureEndpoints::start(ROOT.into());
    // Fixtures and arbitrary child tools share the same kernel network boundary.
    let output = Command::new("/bin/sh").args(["-c", "printf 'fixture process effect\\n' > process-effect.txt; test ! -e /home; test ! -e /run"]).current_dir(ROOT).output().await?;
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(Path::new(ROOT).join("process-effect.txt"))?,
        "fixture process effect\n"
    );
    let catalog: Value = client
        .get(fixtures.url("/v1/models"))
        .bearer_auth(TOKEN)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(catalog["data"][0]["id"], "fixture-model");
    let completion: Value = client.post(fixtures.url("/v1/chat/completions")).bearer_auth(TOKEN).json(&json!({"model":"fixture-model","messages":[{"role":"user","content":"scratch command"}]})).send().await?.error_for_status()?.json().await?;
    let mcp: Value = client.post(fixtures.url("/mcp")).json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write_scratch","arguments":{}}})).send().await?.error_for_status()?.json().await?;
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        std::fs::read_to_string(Path::new(ROOT).join("mcp-effect.txt"))?,
        "fixture MCP effect\n"
    );
    std::fs::write(
        "/scratch/config/fixture-provider.json",
        json!({"endpoint":fixtures.url("/v1"),"api_key":TOKEN}).to_string(),
    )?;

    // Exercise the actual compiled production executable, without enabling operator rollout.
    std::fs::create_dir_all("/scratch/lsp-home")?;
    std::fs::create_dir_all("/scratch/lsp-project")?;
    let mut lsp = Command::new("/scratch/bin/refact-lsp")
        .args([
            "--no-scheduler",
            "--http-port",
            "39192",
            "--http-host",
            "127.0.0.1",
            "--workspace-folder",
            "/scratch/lsp-project",
            "--logs-stderr",
            "--ping-message",
            "b02-isolated",
        ])
        .env("HOME", "/scratch/lsp-home")
        .env("REFACT_DAEMON_NO_UPGRADE", "1")
        .env("REFACT_DAEMON_AUTH_TOKEN", TOKEN)
        .kill_on_drop(true)
        .spawn()?;
    ready(&mut lsp, &client, "http://127.0.0.1:39192/v1/ping").await?;
    let ping = client
        .get("http://127.0.0.1:39192/v1/ping")
        .bearer_auth(TOKEN)
        .send()
        .await?
        .text()
        .await?;
    assert!(ping.contains("b02-isolated"));
    lsp.kill().await?;
    lsp.wait().await?;

    if std::env::var("B02_BROWSER").as_deref() == Ok("1") {
        browser_controller(&client).await?;
        fixtures.shutdown().await;
        return Ok(());
    }
    let worker = start(&client).await?;
    let spec = json!({"schema_version":1,"action_id":"b02-command","kind":"markdown_project_apply","adapter":"extensions","project":project(),"resource":{"resource_kind":"native_markdown","resource_id":"commands/b02","version":"absent"},"expected_version":"absent","parameter_ref":"b02-draft","availability":"available","confirmation":"user_confirmation_required","authority_class":"mutating","capabilities":{"can_preview":true,"can_execute":true,"can_verify":true,"can_recover":true,"can_cancel":false}});
    let body = completion["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("fixture returned no content")?;
    post(&client, "/buddy/actions/draft", json!({"spec":spec,"draft":{"project":project(),"kind":"command","name":"b02","description":"Scratch HTTP acceptance","body":body}})).await?;
    post(&client, "/buddy/actions/preview", json!({"spec":spec})).await?;
    let effect = Path::new(ROOT).join(".refact/commands/b02.md");
    assert!(!effect.exists(), "preview must not write resource");
    let prepared = post(&client, "/buddy/actions/prepare", json!({"spec":spec})).await?;
    let rights = post(&client, "/buddy/actions/rights", json!({"spec":spec})).await?;
    let hash = &prepared["data"]["spec_hash"];
    let execute = json!({"project_id":"b02-scratch","spec_hash":hash,"grant_id":"missing","execution_id":"b02-apply","idempotency_key":"b02-once"});
    assert!(post(&client, "/buddy/actions/execute", execute.clone())
        .await
        .is_err());
    assert!(!effect.exists(), "missing grant cannot write");
    let grant = post(&client, "/buddy/actions/grant", json!({"project_id":"b02-scratch","spec_hash":hash,"expires_in_seconds":3600,"recovery_execution_id":null,"native_rights":rights["data"]})).await?;
    let mut execute = execute;
    execute["grant_id"] = grant["data"]["decision"]["native"]["source_id"].clone();
    let applied = post(&client, "/buddy/actions/execute", execute.clone()).await?;
    assert!(applied["data"].get("Finished").is_some(), "{applied}");
    let bytes = std::fs::read(&effect)?;
    assert!(String::from_utf8_lossy(&bytes).contains("fixture-skill"));
    let receipt = get(
        &client,
        "/buddy/executions/b02-apply?project_id=b02-scratch",
    )
    .await?;
    assert_eq!(receipt["data"]["value"]["execution_id"], "b02-apply");
    let duplicate = post(&client, "/buddy/actions/execute", execute.clone()).await?;
    assert!(duplicate["data"].get("Duplicate").is_some(), "{duplicate}");
    assert_eq!(std::fs::read(&effect)?, bytes);

    let second_client = self::client();
    let before = get(&client, "/buddy/decisions/current").await?;
    assert_eq!(
        before["questions"]
            .as_array()
            .ok_or("questions missing")?
            .len(),
        1
    );
    assert_eq!(
        get(&second_client, "/buddy/decisions/current").await?,
        before
    );
    stop(worker).await?;
    let worker = start(&client).await?;
    assert_eq!(
        get(&client, "/buddy/decisions/current").await?,
        before,
        "unanswered question survives process restart"
    );
    assert_eq!(std::fs::read(&effect)?, bytes);
    assert_eq!(
        get(
            &client,
            "/buddy/executions/b02-apply?project_id=b02-scratch"
        )
        .await?,
        receipt
    );
    let duplicate = post(&client, "/buddy/actions/execute", execute).await?;
    assert!(duplicate["data"].get("Duplicate").is_some(), "{duplicate}");
    let answer = |id: &str| json!({"project_root":ROOT,"asked_at_ms":7,"client_request_id":id,"answers":[{"id":"continue","answer":true}]});
    post(&client, QUESTION, answer("client-one")).await?;
    let question_path = format!("{QUESTION}?project_root={ROOT}");
    let resolved = durable_question(&client, &question_path).await?;
    assert_eq!(resolved["question"]["resolution_id"], "client-one");
    let stale = second_client
        .post(format!("{BASE}{QUESTION}"))
        .bearer_auth(TOKEN)
        .json(&answer("client-two"))
        .send()
        .await?;
    assert_eq!(stale.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(
        durable_question(&second_client, &question_path).await?,
        resolved
    );
    stop(worker).await?;
    let worker = start(&client).await?;
    assert_eq!(
        durable_question(&second_client, &question_path).await?,
        resolved
    );
    let stale = second_client
        .post(format!("{BASE}{QUESTION}"))
        .bearer_auth(TOKEN)
        .json(&answer("stale-after-restart"))
        .send()
        .await?;
    assert_eq!(stale.status(), reqwest::StatusCode::CONFLICT);
    stop(worker).await?;
    fixtures.shutdown().await;
    println!("B02 PASS: OS-confined production binary scratch startup/ping; test-only production AppState/router HTTP preview/grant/effect/receipt/duplicate/restart; fixture provider/MCP/process effects; native two-client resolution and stale conflicts before/after restart. Live browser/IDE parity is not covered.");
    Ok(())
}
