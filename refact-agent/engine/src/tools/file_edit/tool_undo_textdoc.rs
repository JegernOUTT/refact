use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    convert_edit_to_diffchunks, parse_path_for_update, sync_documents_ast,
};
use crate::tools::file_edit::undo_history::{get_undo_history, UndoEntry};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolUndoTextDoc {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    steps: usize,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<PathBuf>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_update(gcx, args, privacy, code_workdir).await?;
    let steps = match args.get("steps") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(1) as usize,
        Some(Value::String(s)) => s.parse().unwrap_or(1),
        _ => 1,
    };
    if steps == 0 {
        return Err("⚠️ steps must be >= 1".to_string());
    }
    Ok(Args { path, steps })
}

pub async fn tool_undo_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<PathBuf>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, code_workdir).await?;

    let history = get_undo_history();
    let entries: Vec<UndoEntry> = {
        let h = history.lock().unwrap();
        h.get(&a.path).cloned().unwrap_or_default()
    };

    if entries.is_empty() {
        return Err(format!(
            "⚠️ No undo history for {:?}. 💡 Only edits from this session can be undone",
            a.path
        ));
    }
    if a.steps > entries.len() {
        return Err(format!(
            "⚠️ Only {} undo steps available, requested {}. 💡 Use steps:{}",
            entries.len(),
            a.steps,
            entries.len()
        ));
    }

    let target_idx = entries.len() - a.steps;
    let target_content = &entries[target_idx].content;

    let current_content = fs::read_to_string(&a.path)
        .map_err(|e| format!("⚠️ Failed to read {:?}: {}", a.path, e))?;

    if target_content.is_empty() {
        fs::remove_file(&a.path).map_err(|e| format!("⚠️ Failed to delete {:?}: {}", a.path, e))?;
    } else {
        fs::write(&a.path, target_content)
            .map_err(|e| format!("⚠️ Failed to write {:?}: {}", a.path, e))?;
    }

    {
        let mut h = history.lock().unwrap();
        if let Some(list) = h.get_mut(&a.path) {
            list.truncate(target_idx + 1);
        }
    }

    gcx.write()
        .await
        .documents_state
        .memory_document_map
        .remove(&a.path);

    let summary = if target_content.is_empty() {
        format!(
            "✅ Undid {} step(s), deleted {:?}",
            a.steps,
            a.path.file_name().unwrap_or_default()
        )
    } else {
        sync_documents_ast(gcx.clone(), &a.path).await?;
        format!(
            "✅ Undid {} step(s) on {:?}",
            a.steps,
            a.path.file_name().unwrap_or_default()
        )
    };

    let chunks = convert_edit_to_diffchunks(a.path.clone(), &current_content, target_content)?;
    Ok((current_content, target_content.clone(), chunks, summary))
}

#[async_trait]
impl Tool for ToolUndoTextDoc {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, code_workdir) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.code_workdir.clone())
        };
        let (_, _, chunks, _) = tool_undo_text_doc_exec(gcx, args, &code_workdir).await?;
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText(json!(chunks).to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let (gcx, code_workdir) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.code_workdir.clone())
        };
        let can_exec = parse_args(gcx, args, &code_workdir).await.is_ok();
        if !can_exec {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "undo_textdoc".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "undo_textdoc".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("undo_textdoc".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["undo_textdoc*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "undo_textdoc".to_string(),
            display_name: "Undo Text Document".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            agentic: false,
            experimental: false,
            description: "Undo recent file edits from this session. Reverts to previous version."
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to the file to undo.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "steps".to_string(),
                    description: "Number of edits to undo (default: 1).".to_string(),
                    param_type: "integer".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string()],
        }
    }
}
