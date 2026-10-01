use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::agents::tool_authority::ensure_context_tool;
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::tools::{self, ExecuteToolsOptions};
use crate::chat::types::ThreadParams;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{
    get_project_dir_string, revalidate_rewritten_tool_call, run_control_hooks, ControlVerdict,
    HookPayload,
};

use super::hosts::{self, HostOperation};
use super::session::AuthorizedCall;

fn tool_call(index: usize, call: &AuthorizedCall) -> ChatToolCall {
    ChatToolCall {
        id: format!("codemode_{index}_{}", uuid::Uuid::new_v4()),
        index: Some(index),
        function: ChatToolFunction {
            name: call.tool.to_string(),
            arguments: call.arguments.to_string(),
        },
        tool_type: "function".to_string(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }
}

fn operation(call: &AuthorizedCall, index: usize) -> Result<&'static HostOperation, String> {
    hosts::operation(call.op).ok_or_else(|| {
        format!(
            "code mode call {index} ({}) has unknown host operation {}",
            call.tool, call.op
        )
    })
}

pub(crate) async fn dispatch_authorized_calls(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    session_id: &str,
    calls: &[AuthorizedCall],
) -> Result<Vec<ChatMessage>, String> {
    let mut produced = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let host_operation = operation(call, index)?;
        if host_operation.tool != call.tool {
            return Err(format!(
                "code mode call {index} ({}) does not match host operation {}",
                call.tool, call.op
            ));
        }
        host_operation.validate(&call.arguments).map_err(|error| {
            format!("code mode call {index} ({}) is invalid: {error}", call.tool)
        })?;
        ensure_context_tool(&ccx, call.tool).await.map_err(|_| {
            format!(
                "code mode call {index} ({}) is outside the caller's authority",
                call.tool
            )
        })?;
        let hook_authority = { ccx.lock().await.child_tool_ceiling.clone() };

        let mut tc = tool_call(index, call);
        let arguments = call.arguments.clone();
        let project_dir = match &thread.worktree {
            Some(worktree) => worktree.root.to_string_lossy().into_owned(),
            None => get_project_dir_string(app.clone()).await,
        };
        let payload = HookPayload::new(&HookEvent::PreToolUse, session_id.to_string(), project_dir)
            .with_tool(call.tool, Some(arguments));
        match run_control_hooks(app.clone(), HookEvent::PreToolUse, payload).await {
            ControlVerdict::Allow => {}
            ControlVerdict::Deny(reason) => {
                return Err(format!(
                    "code mode call {index} ({}) Blocked by hook: {reason}",
                    call.tool
                ));
            }
            ControlVerdict::Rewrite(value) => {
                let rewritten =
                    revalidate_rewritten_tool_call(call.tool, &value, hook_authority.as_ref())
                        .map_err(|error| {
                            format!(
                                "code mode call {index} ({}) rewrite refused: {error}",
                                call.tool
                            )
                        })?;
                let rewritten = serde_json::Value::Object(rewritten);
                host_operation.validate(&rewritten).map_err(|error| {
                    format!(
                        "code mode call {index} ({}) rewrite refused: {error}",
                        call.tool
                    )
                })?;
                tc.function.arguments = rewritten.to_string();
            }
        }

        let (confirmations, denials) = tools::check_tools_confirmation(
            app.clone(),
            std::slice::from_ref(&tc),
            &[],
            thread,
            None,
            mode_id,
            model_id,
            &[],
            "",
        )
        .await;
        if !confirmations.is_empty() || !denials.is_empty() {
            let reason = denials
                .first()
                .map(|denial| format!(": {}", denial.rule))
                .unwrap_or_default();
            return Err(format!(
                "code mode call {index} ({}) requires confirmation; code mode cannot pause for approval{reason}",
                call.tool,
            ));
        }

        let (messages, _) = tools::execute_tools(
            app.clone(),
            std::slice::from_ref(&tc),
            &[],
            thread,
            mode_id,
            model_id,
            ExecuteToolsOptions {
                execution_context: Some(ccx.clone()),
                ..Default::default()
            },
        )
        .await;
        if let Some(failure) = messages
            .iter()
            .find(|message| message.tool_call_id == tc.id && message.tool_failed == Some(true))
        {
            return Err(format!(
                "code mode call {index} ({}) failed: {}",
                call.tool,
                failure.content.content_text_only()
            ));
        }
        produced.extend(messages);
    }
    Ok(produced)
}
