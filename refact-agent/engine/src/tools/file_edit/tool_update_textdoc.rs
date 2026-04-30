use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    append_scope_warnings, await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    parse_bool_arg, parse_path_for_update, parse_string_arg, scope_warnings_to_tool_message,
    str_replace, sync_documents_ast,
};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::worktrees::scope::ExecutionScope;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use crate::knowledge_index::format_related_memories_section;

pub struct ToolUpdateTextDoc {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    old_str: String,
    replacement: String,
    multiple: bool,
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
    let old_str = parse_string_arg(args, "old_str", "Use cat() to find exact text to replace")?;
    let replacement = parse_string_arg(args, "replacement", "Provide the new text")?;
    let multiple = parse_bool_arg(args, "multiple", false)?;
    Ok(Args {
        path,
        old_str,
        replacement,
        multiple,
        scope_warnings: resolved.warnings,
    })
}

pub async fn tool_update_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, execution_scope).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) = str_replace(
        gcx.clone(),
        &a.path,
        &a.old_str,
        &a.replacement,
        a.multiple,
        dry,
    )
    .await?;
    sync_documents_ast(gcx.clone(), &a.path).await?;
    let chunks = convert_edit_to_diffchunks(a.path.clone(), &before, &after)?;
    let summary = append_scope_warnings(
        edit_result_summary(&before, &after, &a.path),
        &a.scope_warnings,
    );
    Ok((before, after, chunks, summary))
}

#[async_trait]
impl Tool for ToolUpdateTextDoc {
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
            tool_update_text_doc_exec(gcx.clone(), args, false, execution_scope.as_ref()).await?;

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
        let (gcx, execution_scope, msgs_len) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
                ccx_locked.messages.len(),
            )
        };
        let can_exec = parse_args(gcx.clone(), args, execution_scope.as_ref())
            .await
            .is_ok();
        if msgs_len != 0 && !can_exec {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "update_textdoc".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "update_textdoc".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("update_textdoc".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["update_textdoc*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "update_textdoc".to_string(),
            display_name: "Update Text Document".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Updates an existing document by replacing specific text, use this if file already exists. Optimized for large files or small changes where simple string replacement is sufficient. Avoid trailing spaces and tabs.".to_string(),
            input_schema: json_schema_from_params(&[("path", "string", "Absolute path to the file to change."), ("old_str", "string", "The exact text that needs to be updated. Use update_textdoc_regex if you need pattern matching (is not preferred for common editing)."), ("replacement", "string", "The new text that will replace the old text."), ("multiple", "boolean", "If true, applies the replacement to all occurrences; if false, only the first occurrence is replaced.")], &["path", "old_str", "replacement"]),
            output_schema: None,
            annotations: None,
        }
    }
}
