use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::agents::spawn::{
    NotifyParent, SpawnHandle, SpawnRequest, emit_background_agent_update, spawn_and_wait,
    spawn_background_agent,
};
use crate::agents::types::{BackgroundAgent, BgAgentKind};
use crate::at_commands::at_commands::{AtCommandsContext, MAX_SUBCHAT_DEPTH};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Clone)]
pub struct ToolDelegate {
    pub config_path: String,
}

#[derive(Clone)]
struct DelegateArgs {
    description: String,
    prompt: String,
    expected_result: String,
    target_files: Vec<String>,
    max_steps: usize,
    wait: bool,
    notify_parent: NotifyParent,
}

#[async_trait]
impl Tool for ToolDelegate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "delegate".to_string(),
            display_name: "Delegate".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Spawn an editable delegate that implements a focused, scoped change in your current workspace. Background by default; pass wait=true to block until it finishes. Multiple delegates may run concurrently in the same workspace — assign non-overlapping target_files when possible. Delegates are LIGHTWEIGHT: they do NOT run tests, compilation, or any verification — YOU verify after they finish.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Short human-readable label, 3-7 words. Example: 'fix auth retry parsing'."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Exact implementation instructions. Include the spec, acceptance criteria, and what NOT to touch."
                    },
                    "expected_result": {
                        "type": "string",
                        "description": "What a successful change looks like. The delegate uses this to know when it is done."
                    },
                    "target_files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Files the delegate is expected to edit. Used for concurrency-overlap warnings."
                    },
                    "max_steps": {
                        "type": "integer",
                        "description": "Step budget (default 25, max 50)."
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "If true, block until the delegate finishes. Default false (background)."
                    },
                    "notify_parent": {
                        "type": "string",
                        "enum": ["auto", "silent"],
                        "description": "auto = push completion back to this chat (default). silent = do not push; you must poll with agent_result."
                    }
                },
                "required": ["description", "prompt", "expected_result"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args = parse_delegate_args(args)?;
        let (
            app,
            parent_chat_id,
            parent_root_chat_id,
            parent_subchat_tx,
            subchat_depth,
            parent_task_meta,
            parent_worktree,
            current_model,
        ) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.root_chat_id.clone(),
                ccx_lock.subchat_tx.clone(),
                ccx_lock.subchat_depth,
                ccx_lock.task_meta.clone(),
                ccx_lock.execution_scope_worktree(),
                ccx_lock.current_model.clone(),
            )
        };

        if subchat_depth >= MAX_SUBCHAT_DEPTH {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!(
                        "Error: Maximum delegate recursion depth ({}) exceeded",
                        MAX_SUBCHAT_DEPTH
                    )),
                    tool_call_id: tool_call_id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                })],
            ));
        }

        let overlap = app
            .agents
            .overlap_warning(&parent_chat_id, &args.target_files)
            .await;
        let prompt = build_delegate_prompt(
            &args.prompt,
            &args.expected_result,
            &args.target_files,
            args.max_steps,
            overlap.as_deref(),
        );
        let req = SpawnRequest {
            kind: BgAgentKind::Delegate,
            parent_chat_id,
            parent_root_chat_id: Some(parent_root_chat_id),
            parent_tool_call_id: Some(tool_call_id.clone()),
            config_name: "delegate_with_editing".to_string(),
            title: args.description.clone(),
            prompt,
            tools: None,
            target_files: args.target_files.clone(),
            max_steps: args.max_steps,
            model: current_model,
            parent_subchat_tx: Some(parent_subchat_tx),
            parent_worktree,
            parent_task_meta,
            subchat_depth,
            notify_parent: args.notify_parent,
        };

        if args.wait {
            let req_silent = SpawnRequest {
                notify_parent: NotifyParent::Silent,
                ..req
            };
            let record =
                call_spawn_and_wait(app.clone(), req_silent, Some(Duration::from_secs(60 * 60)))
                    .await?;
            let record = attach_overlap_warning(app, record, overlap.as_deref()).await?;
            Ok((
                false,
                vec![build_foreground_result_msg(
                    &record,
                    tool_call_id,
                    overlap.as_deref(),
                )],
            ))
        } else {
            let handle = call_spawn_background_agent(app.clone(), req).await?;
            if let Some(warning) = overlap.as_deref() {
                let record = app
                    .agents
                    .update_progress(
                        &handle.agent_id,
                        warning.to_string(),
                        0,
                        Some("overlap_warning".to_string()),
                    )
                    .await?;
                emit_background_agent_update(app, &record).await;
            }
            Ok((
                false,
                vec![build_background_start_msg(
                    &handle,
                    &args.description,
                    &args.target_files,
                    overlap.as_deref(),
                    tool_call_id,
                )],
            ))
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn parse_delegate_args(args: &HashMap<String, Value>) -> Result<DelegateArgs, String> {
    let description = parse_required_string(args, "description")?;
    let prompt = parse_required_string(args, "prompt")?;
    let expected_result = parse_required_string(args, "expected_result")?;
    let target_files = parse_optional_string_array(args, "target_files")?;
    let max_steps = clamp_max_steps(parse_optional_usize(args, "max_steps", 25)?);
    let wait = parse_optional_bool(args, "wait", false)?;
    let notify_parent = parse_optional_string(args, "notify_parent")?
        .map(|s| match s.as_str() {
            "silent" => NotifyParent::Silent,
            _ => NotifyParent::Auto,
        })
        .unwrap_or(NotifyParent::Auto);
    Ok(DelegateArgs {
        description,
        prompt,
        expected_result,
        target_files,
        max_steps,
        wait,
        notify_parent,
    })
}

fn parse_required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(format!("argument `{key}` is not a string: {v:?}")),
        None => Err(format!("Missing argument `{key}`")),
    }
}

fn parse_optional_string(
    args: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(v) => Err(format!("argument `{key}` is not a string: {v:?}")),
        None => Ok(None),
    }
}

fn parse_optional_string_array(
    args: &HashMap<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    match args.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                other => Err(format!(
                    "argument `{key}` contains a non-string value: {other:?}"
                )),
            })
            .collect(),
        Some(v) => Err(format!("argument `{key}` is not a string array: {v:?}")),
        None => Ok(Vec::new()),
    }
}

fn parse_optional_usize(
    args: &HashMap<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize, String> {
    match args.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|value| value as usize)
            .ok_or_else(|| format!("argument `{key}` is not a positive integer: {n:?}")),
        Some(Value::String(s)) => s
            .parse::<usize>()
            .map_err(|_| format!("argument `{key}` is not a positive integer: {s:?}")),
        Some(v) => Err(format!("argument `{key}` is not an integer: {v:?}")),
        None => Ok(default),
    }
}

fn parse_optional_bool(
    args: &HashMap<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(v) => Err(format!("argument `{key}` is not a boolean: {v:?}")),
        None => Ok(default),
    }
}

fn clamp_max_steps(max_steps: usize) -> usize {
    max_steps.min(50).max(1)
}

fn build_delegate_prompt(
    prompt: &str,
    expected_result: &str,
    target_files: &[String],
    max_steps: usize,
    overlap_warning: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Your Task\n{prompt}\n\n"));
    out.push_str(&format!("# Expected Result\n{expected_result}\n\n"));
    if !target_files.is_empty() {
        out.push_str("# Target Files (ONLY edit files in this list)\n");
        for f in target_files {
            out.push_str(&format!("- {f}\n"));
        }
        out.push('\n');
    } else {
        out.push_str("# Target Files\n(no specific files listed; keep your edits minimal and scoped to the task)\n\n");
    }
    if let Some(w) = overlap_warning {
        out.push_str(&format!("# ⚠ Concurrency Warning\n{w}\n\nIf another delegate has already modified a file you need, leave their changes intact and only complete the parts you own. Mention the situation in your final report.\n\n"));
    }
    out.push_str(&format!("# Constraints\n- Maximum steps: {max_steps}\n- Do NOT run tests, compilation, lint, or any verification — the parent agent will verify after you finish.\n- Do NOT touch files outside your target_files.\n- Use `tasks_set` to publish progress.\n- Keep changes minimal.\n\n"));
    out.push_str("# Final Report\nEnd with the Status block described in your system prompt.\n");
    out
}

fn build_background_start_msg(
    handle: &SpawnHandle,
    description: &str,
    target_files: &[String],
    overlap: Option<&str>,
    tool_call_id: &String,
) -> ContextEnum {
    let mut lines = vec![
        format!("✓ Started background delegate: {description}"),
        format!("- agent_id: {}", handle.agent_id),
        "- status: running".to_string(),
        format!("- target_files: {}", format_target_files(target_files)),
        format!("- child_chat_id: {}", handle.child_chat_id),
        String::new(),
        format!(
            "Open the child trajectory: [view](refact://chat/{})",
            handle.child_chat_id
        ),
        String::new(),
        "The completion will be pushed back into this chat automatically. Use agent_status, agent_wait, or agent_result if you need to follow up sooner.".to_string(),
    ];
    if let Some(warning) = overlap {
        lines.push(String::new());
        lines.push(warning.to_string());
    }
    tool_message(
        tool_call_id,
        lines.join("\n"),
        build_extra(
            &handle.agent_id,
            Some(&handle.child_chat_id),
            "running",
            target_files,
            overlap,
        ),
    )
}

fn build_foreground_result_msg(
    record: &BackgroundAgent,
    tool_call_id: &String,
    overlap: Option<&str>,
) -> ContextEnum {
    let child_chat_id = record.child_chat_id.as_deref();
    let mut lines = vec![
        format!("✓ Delegate finished: {}", record.title),
        format!("- agent_id: {}", record.agent_id),
        format!("- status: {}", record.status.as_str()),
        format!(
            "- target_files: {}",
            format_target_files(&record.target_files)
        ),
        format!("- child_chat_id: {}", child_chat_id.unwrap_or("")),
        String::new(),
    ];
    if let Some(child_chat_id) = child_chat_id {
        lines.push(format!(
            "Open the child trajectory: [view](refact://chat/{child_chat_id})"
        ));
        lines.push(String::new());
    }
    lines.push("## Result".to_string());
    lines.push(result_text(record));
    if let Some(warning) = overlap {
        lines.push(String::new());
        lines.push(warning.to_string());
    }
    tool_message(
        tool_call_id,
        lines.join("\n"),
        build_extra(
            &record.agent_id,
            child_chat_id,
            record.status.as_str(),
            &record.target_files,
            overlap,
        ),
    )
}

fn format_target_files(target_files: &[String]) -> String {
    if target_files.is_empty() {
        "(not specified)".to_string()
    } else {
        target_files.join(", ")
    }
}

fn result_text(record: &BackgroundAgent) -> String {
    record
        .result_summary
        .clone()
        .or_else(|| record.error.clone())
        .or_else(|| record.progress.clone())
        .unwrap_or_else(|| "Delegate finished without a result summary.".to_string())
}

fn build_extra(
    agent_id: &str,
    child_chat_id: Option<&str>,
    status: &str,
    target_files: &[String],
    overlap: Option<&str>,
) -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "background_agent_id".to_string(),
        Value::String(agent_id.to_string()),
    );
    extra.insert(
        "background_agent_kind".to_string(),
        Value::String("delegate".to_string()),
    );
    extra.insert(
        "child_chat_id".to_string(),
        child_chat_id
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    extra.insert(
        "background_agent_status".to_string(),
        Value::String(status.to_string()),
    );
    extra.insert(
        "target_files".to_string(),
        Value::Array(
            target_files
                .iter()
                .map(|file| Value::String(file.clone()))
                .collect(),
        ),
    );
    extra.insert(
        "overlap_warning".to_string(),
        overlap
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    extra
}

fn tool_message(
    tool_call_id: &String,
    content: String,
    extra: serde_json::Map<String, Value>,
) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        preserve: Some(true),
        extra,
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    })
}

async fn attach_overlap_warning(
    app: crate::app_state::AppState,
    record: BackgroundAgent,
    overlap: Option<&str>,
) -> Result<BackgroundAgent, String> {
    let Some(warning) = overlap else {
        return Ok(record);
    };
    let updated = app
        .agents
        .update_progress(
            &record.agent_id,
            warning.to_string(),
            record.step_count,
            Some("overlap_warning".to_string()),
        )
        .await?;
    emit_background_agent_update(app, &updated).await;
    Ok(updated)
}

async fn call_spawn_background_agent(
    app: crate::app_state::AppState,
    req: SpawnRequest,
) -> Result<SpawnHandle, String> {
    #[cfg(test)]
    if let Some(log) = active_test_spawn_log(&req.parent_chat_id) {
        return fake_background_spawn(app, req, log).await;
    }
    spawn_background_agent(app, req).await
}

async fn call_spawn_and_wait(
    app: crate::app_state::AppState,
    req: SpawnRequest,
    timeout: Option<Duration>,
) -> Result<BackgroundAgent, String> {
    #[cfg(test)]
    if let Some(log) = active_test_spawn_log(&req.parent_chat_id) {
        return fake_wait_spawn(app, req, timeout, log).await;
    }
    spawn_and_wait(app, req, timeout).await
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoggedSpawnKind {
    Background,
    Wait,
}

#[cfg(test)]
#[derive(Clone)]
struct LoggedSpawnCall {
    kind: LoggedSpawnKind,
    req: SpawnRequest,
    timeout: Option<Duration>,
}

#[cfg(test)]
type TestSpawnLog = Arc<std::sync::Mutex<Vec<LoggedSpawnCall>>>;

#[cfg(test)]
static TEST_SPAWN_LOGS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, TestSpawnLog>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
struct TestSpawnLogGuard {
    parent_chat_id: String,
}

#[cfg(test)]
impl Drop for TestSpawnLogGuard {
    fn drop(&mut self) {
        if let Some(logs) = TEST_SPAWN_LOGS.get() {
            logs.lock().unwrap().remove(&self.parent_chat_id);
        }
    }
}

#[cfg(test)]
fn install_test_spawn_log(parent_chat_id: &str) -> (TestSpawnLog, TestSpawnLogGuard) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    TEST_SPAWN_LOGS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(parent_chat_id.to_string(), calls.clone());
    (
        calls,
        TestSpawnLogGuard {
            parent_chat_id: parent_chat_id.to_string(),
        },
    )
}

#[cfg(test)]
fn active_test_spawn_log(parent_chat_id: &str) -> Option<TestSpawnLog> {
    TEST_SPAWN_LOGS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .get(parent_chat_id)
        .cloned()
}

#[cfg(test)]
async fn fake_background_spawn(
    app: crate::app_state::AppState,
    req: SpawnRequest,
    log: TestSpawnLog,
) -> Result<SpawnHandle, String> {
    log.lock().unwrap().push(LoggedSpawnCall {
        kind: LoggedSpawnKind::Background,
        req: req.clone(),
        timeout: None,
    });
    let (record, _, _) = app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: req.parent_chat_id.clone(),
            parent_root_chat_id: req.parent_root_chat_id.clone(),
            parent_tool_call_id: req.parent_tool_call_id.clone(),
            kind: req.kind,
            config_name: req.config_name.clone(),
            title: req.title.clone(),
            prompt: req.prompt.clone(),
            target_files: req.target_files.clone(),
            model: req.model.clone(),
        })
        .await?;
    let child_chat_id = "subchat-test".to_string();
    app.agents
        .mark_running(&record.agent_id, child_chat_id.clone())
        .await?;
    let (_tx, rx) = tokio::sync::oneshot::channel();
    Ok(SpawnHandle {
        agent_id: record.agent_id,
        child_chat_id,
        completion_rx: rx,
    })
}

#[cfg(test)]
async fn fake_wait_spawn(
    app: crate::app_state::AppState,
    req: SpawnRequest,
    timeout: Option<Duration>,
    log: TestSpawnLog,
) -> Result<BackgroundAgent, String> {
    log.lock().unwrap().push(LoggedSpawnCall {
        kind: LoggedSpawnKind::Wait,
        req: req.clone(),
        timeout,
    });
    let (record, _, _) = app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: req.parent_chat_id.clone(),
            parent_root_chat_id: req.parent_root_chat_id.clone(),
            parent_tool_call_id: req.parent_tool_call_id.clone(),
            kind: req.kind,
            config_name: req.config_name.clone(),
            title: req.title.clone(),
            prompt: req.prompt.clone(),
            target_files: req.target_files.clone(),
            model: req.model.clone(),
        })
        .await?;
    app.agents
        .mark_running(&record.agent_id, "subchat-test".to_string())
        .await?;
    app.agents
        .mark_completed(
            &record.agent_id,
            crate::agents::types::AgentCompletion {
                result_summary: "delegate done".to_string(),
                edited_files: req.target_files.clone(),
                diff_summary: None,
                conflict_summary: None,
                child_chat_id: Some("subchat-test".to_string()),
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_args() -> HashMap<String, Value> {
        HashMap::from([
            ("description".to_string(), json!("fix retry parsing")),
            ("prompt".to_string(), json!("Edit retry parsing.")),
            ("expected_result".to_string(), json!("Retry parsing works.")),
        ])
    }

    async fn delegate_ccx(parent_chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                parent_chat_id.to_string(),
                Some("root-chat".to_string()),
                "test-model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn message_text(message: ContextEnum) -> (String, serde_json::Map<String, Value>) {
        match message {
            ContextEnum::ChatMessage(message) => {
                (message.content.content_text_only(), message.extra)
            }
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn missing_required_fields_return_clear_errors() {
        for key in ["description", "prompt", "expected_result"] {
            let mut args = base_args();
            args.remove(key);
            match parse_delegate_args(&args) {
                Ok(_) => panic!("expected missing field error"),
                Err(err) => assert_eq!(err, format!("Missing argument `{key}`")),
            }
        }
    }

    #[test]
    fn target_files_parse_string_array() {
        let mut args = base_args();
        args.insert(
            "target_files".to_string(),
            json!(["src/auth/retry.ts", "src/auth/retry_test.ts"]),
        );
        let parsed = parse_delegate_args(&args).unwrap();
        assert_eq!(
            parsed.target_files,
            vec![
                "src/auth/retry.ts".to_string(),
                "src/auth/retry_test.ts".to_string(),
            ]
        );
    }

    #[test]
    fn max_steps_clamps_to_range() {
        let mut low = base_args();
        low.insert("max_steps".to_string(), json!(0));
        assert_eq!(parse_delegate_args(&low).unwrap().max_steps, 1);

        let mut high = base_args();
        high.insert("max_steps".to_string(), json!(500));
        assert_eq!(parse_delegate_args(&high).unwrap().max_steps, 50);
    }

    #[tokio::test]
    async fn wait_true_calls_spawn_and_wait() {
        let (calls, _guard) = install_test_spawn_log("parent-wait");
        let ccx = delegate_ccx("parent-wait").await;
        let mut args = base_args();
        args.insert("wait".to_string(), json!(true));
        let mut tool = ToolDelegate {
            config_path: "builtin".to_string(),
        };

        let (_, messages) = tool
            .tool_execute(ccx, &"call-wait".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        let (text, _) = message_text(messages.into_iter().next().unwrap());
        assert!(text.contains(
            "Open the child trajectory: [view](refact://chat/subchat-test)"
        ));
        let logged = calls.lock().unwrap().clone();
        assert_eq!(logged.len(), 1);
        assert!(matches!(logged[0].kind, LoggedSpawnKind::Wait));
        assert!(logged[0].timeout.is_some());
        assert!(matches!(logged[0].req.notify_parent, NotifyParent::Silent));
    }

    #[tokio::test]
    async fn wait_false_defaults_to_background_and_returns_quickly() {
        let (calls, _guard) = install_test_spawn_log("parent-background");
        let ccx = delegate_ccx("parent-background").await;
        let mut tool = ToolDelegate {
            config_path: "builtin".to_string(),
        };

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            tool.tool_execute(ccx, &"call-background".to_string(), &base_args()),
        )
        .await
        .expect("delegate returned within 100ms")
        .unwrap();

        assert_eq!(result.1.len(), 1);
        let (text, _) = message_text(result.1.into_iter().next().unwrap());
        assert!(text.contains(
            "Open the child trajectory: [view](refact://chat/subchat-test)"
        ));
        let logged = calls.lock().unwrap().clone();
        assert_eq!(logged.len(), 1);
        assert!(matches!(logged[0].kind, LoggedSpawnKind::Background));
    }

    #[tokio::test]
    async fn notify_parent_silent_propagates_to_spawn_request() {
        let (calls, _guard) = install_test_spawn_log("parent-silent");
        let ccx = delegate_ccx("parent-silent").await;
        let mut args = base_args();
        args.insert("notify_parent".to_string(), json!("silent"));
        let mut tool = ToolDelegate {
            config_path: "builtin".to_string(),
        };

        tool.tool_execute(ccx, &"call-silent".to_string(), &args)
            .await
            .unwrap();

        let logged = calls.lock().unwrap().clone();
        assert!(matches!(logged[0].req.notify_parent, NotifyParent::Silent));
    }

    #[tokio::test]
    async fn overlap_warning_is_in_prompt_and_extra() {
        let (calls, _guard) = install_test_spawn_log("parent-overlap");
        let ccx = delegate_ccx("parent-overlap").await;
        let app = { ccx.lock().await.app.clone() };
        let (existing, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "parent-overlap".to_string(),
                parent_root_chat_id: Some("root-chat".to_string()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Delegate,
                config_name: "delegate_with_editing".to_string(),
                title: "existing delegate".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec!["src/auth/retry.ts".to_string()],
                model: "test-model".to_string(),
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&existing.agent_id, "subchat-existing".to_string())
            .await
            .unwrap();
        let mut args = base_args();
        args.insert("target_files".to_string(), json!(["src/auth/retry.ts"]));
        let mut tool = ToolDelegate {
            config_path: "builtin".to_string(),
        };

        let (_, messages) = tool
            .tool_execute(ccx, &"call-overlap".to_string(), &args)
            .await
            .unwrap();

        let logged = calls.lock().unwrap().clone();
        assert!(logged[0].req.prompt.contains("# ⚠ Concurrency Warning"));
        assert!(logged[0].req.prompt.contains(&existing.agent_id));
        let (text, extra) = message_text(messages.into_iter().next().unwrap());
        assert!(text.contains("Running delegate target file overlap detected"));
        assert_eq!(
            extra.get("overlap_warning").and_then(Value::as_str),
            Some(
                logged[0]
                    .req
                    .prompt
                    .split("# ⚠ Concurrency Warning\n")
                    .nth(1)
                    .unwrap()
                    .split("\n\nIf another delegate")
                    .next()
                    .unwrap()
            )
        );
    }

    #[test]
    fn prompt_builder_includes_target_files_and_constraints() {
        let prompt = build_delegate_prompt(
            "Implement retry parsing.",
            "Retry parsing works.",
            &["src/auth/retry.ts".to_string()],
            17,
            Some("overlap warning"),
        );

        assert!(prompt.contains("# Your Task\nImplement retry parsing."));
        assert!(prompt.contains("# Expected Result\nRetry parsing works."));
        assert!(prompt.contains("# Target Files (ONLY edit files in this list)"));
        assert!(prompt.contains("- src/auth/retry.ts"));
        assert!(prompt.contains("# ⚠ Concurrency Warning\noverlap warning"));
        assert!(prompt.contains("- Maximum steps: 17"));
        assert!(prompt.contains("Do NOT run tests, compilation, lint, or any verification"));
        assert!(prompt.contains("Use `tasks_set` to publish progress"));
    }
}
