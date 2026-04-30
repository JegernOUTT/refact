use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    append_scope_warnings, convert_edit_to_diffchunks, parse_path_for_update,
    scope_warnings_to_tool_message, sync_documents_ast,
};
use crate::tools::file_edit::undo_history::{get_undo_history, UndoEntry};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::worktrees::scope::ExecutionScope;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use crate::knowledge_index::format_related_memories_section;

pub struct ToolUndoTextDoc {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    steps: usize,
    scope_warnings: Vec<String>,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let resolved = parse_path_for_update(gcx, args, privacy, execution_scope).await?;
    let path = resolved.path;
    let steps = match args.get("steps") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(1) as usize,
        Some(Value::String(s)) => s.parse().unwrap_or(1),
        _ => 1,
    };
    if steps == 0 {
        return Err("⚠️ steps must be >= 1".to_string());
    }
    Ok(Args {
        path,
        steps,
        scope_warnings: resolved.warnings,
    })
}

pub async fn tool_undo_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, execution_scope).await?;

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
    let summary = append_scope_warnings(summary, &a.scope_warnings);
    Ok((current_content, target_content.clone(), chunks, summary))
}

#[async_trait]
impl Tool for ToolUndoTextDoc {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, execution_scope) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
            )
        };
        let (_, _, chunks, summary) =
            tool_undo_text_doc_exec(gcx.clone(), args, execution_scope.as_ref()).await?;

        let related_section = {
            let idx_arc = { gcx.read().await.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let mut paths: Vec<String> = Vec::new();
            for c in chunks.iter() {
                if !c.file_name.is_empty() {
                    paths.push(c.file_name.clone());
                }
                if let Some(rename) = &c.file_name_rename {
                    if !rename.is_empty() {
                        paths.push(rename.clone());
                    }
                }
            }
            paths.sort();
            paths.dedup();
            let mut cards = idx_guard.related_for_files(&paths, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&paths, 8);
            }
            format_related_memories_section(&cards, None)
        };

        let mut out = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText(json!(chunks).to_string()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })];

        if let Some(message) = scope_warnings_to_tool_message(&summary, tool_call_id) {
            out.push(message);
        }

        if !related_section.trim().is_empty() {
            out.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(related_section),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));
        }

        Ok((false, out))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let (gcx, execution_scope) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
            )
        };
        let can_exec = parse_args(gcx, args, execution_scope.as_ref())
            .await
            .is_ok();
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
            experimental: false,
            allow_parallel: false,
            description: "Undo recent file edits from this session. Reverts to previous version."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("path", "string", "Absolute path to the file to undo."),
                    ("steps", "integer", "Number of edits to undo (default: 1)."),
                ],
                &["path"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}
