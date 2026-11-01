use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ContextEnum};
use crate::chat::types::{ChatSession, ThreadParams};
use crate::coordination::causal;
use crate::tools::tool_causal_trace::ToolCausalTrace;
use crate::tools::tools_description::Tool;
use refact_privacy::{PrivacyPolicy, ShellBehavior, Zone};
use refact_work_core::{CausalCause, CausalEventRecord, CausalKind, ProjectKey, WorkKey};

async fn fixture() -> (AppState, Arc<AMutex<AtCommandsContext>>, tempfile::TempDir) {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let project = dir.path().to_path_buf();
    let cache = dir.path().join("cache");
    let config = dir.path().join("config");
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(cache, config).await;
    Arc::get_mut(&mut gcx).unwrap().cmdline.workspace_folder =
        project.to_string_lossy().into_owned();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.clone()];
    std::fs::create_dir_all(&project).unwrap();
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(&gcx.config_dir)
        .await
        .unwrap();

    let model_id = "trusted/causal-trace";
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        model_id.into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: model_id.into(),
                name: model_id.into(),
                n_ctx: 32_000,
                endpoint: "https://example.com/v1/chat/completions".into(),
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    caps.defaults.chat_default_model = model_id.into();
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    let app = AppState::from_gcx(gcx).await;
    let chat_id = format!("causal-trace-{}", uuid::Uuid::new_v4());
    let session = Arc::new(AMutex::new(ChatSession::new(chat_id.clone())));
    {
        let mut session = session.lock().await;
        session.thread = ThreadParams {
            id: chat_id.clone(),
            model: model_id.into(),
            mode: "agent".into(),
            ..Default::default()
        };
        session.work_project_root = Some(project);
    }
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.clone(), session);
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            chat_id.clone(),
            Some(chat_id.clone()),
            model_id.into(),
            None,
            None,
        )
        .await,
    ));
    (app, ccx, dir)
}

fn args(event_id: &str, replay: bool, max_events: usize) -> HashMap<String, Value> {
    HashMap::from([
        ("event_id".into(), json!(event_id)),
        ("replay".into(), json!(replay)),
        ("max_events".into(), json!(max_events)),
    ])
}

async fn execute(
    ccx: Arc<AMutex<AtCommandsContext>>,
    event_id: &str,
    replay: bool,
    max_events: usize,
) -> String {
    let mut tool = ToolCausalTrace {
        config_path: String::new(),
    };
    let (_, output) = tool
        .tool_execute(
            ccx,
            &"causal-trace-test".into(),
            &args(event_id, replay, max_events),
        )
        .await
        .unwrap();
    let ContextEnum::ChatMessage(message) = &output[0] else {
        panic!("expected tool message")
    };
    let ChatContent::SimpleText(text) = &message.content else {
        panic!("expected text output")
    };
    text.clone()
}

fn record_event(
    project_root: &std::path::Path,
    event_id: &str,
    kind: CausalKind,
    scope: &str,
    seq: i64,
    causes: Vec<CausalCause>,
    paths: Vec<String>,
) {
    causal::recorder().push(
        CausalEventRecord {
            project: ProjectKey::new(crate::coordination::roots::project_identity(project_root)),
            event_id: event_id.to_string(),
            seq,
            kind,
            work: WorkKey::trajectory(scope),
            scope: scope.to_string(),
            causes,
            recorded_at_ms: 1_000 + seq,
            paths,
        },
        causal::retention_cap(),
    );
}

fn chain_ids() -> (String, String, String) {
    let suffix = uuid::Uuid::new_v4();
    (
        format!("delivery:{suffix}"),
        format!("spawn:{suffix}"),
        format!("child:{suffix}"),
    )
}

#[tokio::test]
async fn causal_trace_renders_recorded_causes() {
    let (_, ccx, dir) = fixture().await;
    let (delivery, spawn, child_scope) = chain_ids();
    record_event(
        dir.path(),
        &delivery,
        CausalKind::Delivery,
        "parent",
        1,
        vec![CausalCause::Unknown],
        vec![],
    );
    record_event(
        dir.path(),
        &spawn,
        CausalKind::Spawn,
        &child_scope,
        2,
        vec![CausalCause::Event(delivery.clone())],
        vec![],
    );

    let output = execute(ccx, &spawn, false, 50).await;
    assert!(output.contains(&spawn), "{output}");
    assert!(output.contains(&delivery), "{output}");
    assert!(
        output.find(&spawn).unwrap() < output.find(&delivery).unwrap(),
        "{output}"
    );
    assert!(output.contains("spawn in scope"), "{output}");
    assert!(output.contains("delivery in scope"), "{output}");
}

#[tokio::test]
async fn causal_trace_hides_events_not_cleared_for_model() {
    let (app, ccx, dir) = fixture().await;
    let guarded = dir.path().join("guarded.rs");
    let secret = format!("evidence:private-{}", uuid::Uuid::new_v4());
    let root = format!("decision:private-{}", uuid::Uuid::new_v4());
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
        zones: vec![Zone {
            name: "guarded".into(),
            patterns: vec![guarded.to_string_lossy().into_owned()],
            send_to: vec!["other".into()],
            on_shell_read: ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    record_event(
        dir.path(),
        &secret,
        CausalKind::Evidence,
        "private-scope",
        1,
        vec![CausalCause::Unknown],
        vec![guarded.to_string_lossy().into_owned()],
    );
    record_event(
        dir.path(),
        &root,
        CausalKind::Decision,
        "public-scope",
        2,
        vec![CausalCause::Event(secret.clone())],
        vec![],
    );

    let output = execute(ccx, &root, false, 50).await;
    assert!(output.contains(&root), "{output}");
    assert!(!output.contains(&secret), "{output}");
    assert!(
        !output.contains(&guarded.to_string_lossy().to_string()),
        "{output}"
    );
}
