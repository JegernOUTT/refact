use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::code_mode::session::AuthorizedCall;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolCodeMode {
    pub config_path: String,
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}

pub(crate) fn summarize_dispatch(
    calls: &[AuthorizedCall],
    result: &Result<Vec<ChatMessage>, String>,
) -> Vec<String> {
    match result {
        Ok(messages) => calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                let prefix = format!("codemode_{index}_");
                let outcome = match messages
                    .iter()
                    .find(|message| message.tool_call_id.starts_with(&prefix))
                {
                    Some(message) if message.tool_failed == Some(true) => {
                        format!(
                            "err: {}",
                            truncate(&message.content.content_text_only(), 240)
                        )
                    }
                    Some(_) => "ok".to_string(),
                    None => "no result".to_string(),
                };
                format!("- {} ({}) — {outcome}", call.op, call.tool)
            })
            .collect(),
        Err(error) => {
            let failed_index = (|| {
                let rest = error.strip_prefix("code mode call ")?;
                let digits = rest
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>();
                if digits.is_empty() {
                    return None;
                }
                let suffix = &rest[digits.len()..];
                if !suffix.starts_with(" (") {
                    return None;
                }
                digits.parse::<usize>().ok()
            })();
            let error = truncate(error, 240);
            match failed_index {
                Some(failed) if failed < calls.len() => calls
                    .iter()
                    .enumerate()
                    .map(|(index, call)| {
                        let outcome = if index < failed {
                            "ran (output not returned after a later failure)".to_string()
                        } else if index == failed {
                            format!("err: {error}")
                        } else {
                            "not run".to_string()
                        };
                        format!("- {} ({}) — {outcome}", call.op, call.tool)
                    })
                    .collect(),
                _ => {
                    let mut lines = vec![format!("dispatch stopped: {error}")];
                    lines.extend(
                        calls
                            .iter()
                            .map(|call| format!("- {} ({}) — status unknown", call.op, call.tool)),
                    );
                    lines
                }
            }
        }
    }
}

#[async_trait]
impl Tool for ToolCodeMode {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_mode".to_string(),
            display_name: "Code Mode".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Default off. Runs a Python orchestration script only inside an enforceable OS sandbox. It may call only the host operations spawn, claim, wait, and report that your authority permits. It cannot pause for approvals. The user confirms the script before it runs; results are returned per call.".to_string(),
            input_schema: json!({
                "additionalProperties": false,
                "type": "object",
                "properties": {
                    "script": {"type": "string", "minLength": 1}
                },
                "required": ["script"]
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
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .filter(|script| !script.trim().is_empty())
            .ok_or_else(|| "Missing argument `script`".to_string())?
            .to_string();
        let (app, chat_id, abort, child_ceiling, source_root, checkout_root) = {
            let context = ccx.lock().await;
            let worktree = context
                .execution_scope
                .as_ref()
                .map(|scope| scope.worktree());
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.abort_flag.clone(),
                context.child_tool_ceiling.clone(),
                worktree.map(|worktree| worktree.source_workspace_root.clone()),
                context
                    .execution_scope
                    .as_ref()
                    .map(|scope| scope.effective_root().to_path_buf()),
            )
        };
        let project_root = match source_root {
            Some(root) => root,
            None => crate::files_correction::get_project_dirs(app.gcx.clone())
                .await
                .into_iter()
                .next()
                .ok_or_else(|| "code mode requires an active project root".to_string())?,
        };
        let settings = crate::runtime_settings::coordination_for_project(&project_root).await?;
        let settings = match settings.code_mode_enabled {
            true => settings,
            false => {
                return Err(
                    "code mode is unavailable: the `code_mode_enabled` setting is off".to_string(),
                )
            }
        };
        let status =
            crate::code_mode::current_status(&settings, app.gcx.terminal_security_config.mode);
        status.require_available()?;

        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or_else(|| "caller session is unavailable".to_string())?;
        let thread = session.lock().await.thread.clone();
        let mode_id = thread.mode.clone();
        let model_id = thread.model.clone();
        let ceiling = match child_ceiling {
            Some(ceiling) => ceiling,
            None => {
                crate::agents::tool_authority::current_ceiling(&app, &chat_id, &model_id, &mode_id)
                    .await?
            }
        };
        let caps = crate::code_mode::capabilities(&ceiling)?;
        if caps.is_empty() {
            let schemas =
                serde_json::to_string(&caps.schemas()).map_err(|error| error.to_string())?;
            return Err(format!(
                "code mode refused: the assignment authority ceiling grants no host operation. Callable schemas: {schemas}"
            ));
        }

        let req = crate::code_mode::CodeModeRequest {
            run: refact_work_core::WorkKey::trajectory(chat_id.clone()).run(1),
            scope: refact_work_core::WorkScope::new(
                refact_work_core::ProjectKey::new(crate::coordination::roots::project_identity(
                    &project_root,
                )),
                refact_work_core::CheckoutKey::new(
                    checkout_root
                        .unwrap_or_else(|| project_root.clone())
                        .to_string_lossy()
                        .into_owned(),
                ),
            ),
            script,
            capabilities: caps,
            limits: crate::code_mode::CodeModeLimits::from_settings(&settings),
            abort,
        };
        let run = crate::code_mode::execute(
            &app.runtime.exec_registry,
            &app.work_scheduler,
            &status,
            req,
        )
        .await?;
        let dispatched = crate::code_mode::dispatch::dispatch_authorized_calls(
            app.clone(),
            ccx.clone(),
            &thread,
            &mode_id,
            Some(&model_id),
            &chat_id,
            &run.calls,
        )
        .await;
        let result_lines = summarize_dispatch(&run.calls, &dispatched);
        let mut lines = vec![format!(
            "code_mode authorized {} host calls",
            run.calls.len()
        )];
        lines.extend(result_lines);
        lines.push(format!(
            "Intermediate bytes: {} / {}",
            run.intermediate.used(),
            run.intermediate.cap()
        ));
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(lines.join("\n")),
                tool_call_id: tool_call_id.clone(),
                preserve: Some(true),
                ..Default::default()
            })],
        ))
    }
}
