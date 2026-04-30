use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    append_scope_warnings, await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    normalize_line_endings, parse_path_for_create, parse_string_arg, restore_line_endings,
    scope_warnings_to_tool_message, sync_documents_ast, write_file,
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

pub struct ToolCreateTextDoc {
    pub config_path: String,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(PathBuf, String, bool, Vec<String>), String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let resolved = parse_path_for_create(gcx.clone(), args, privacy, execution_scope).await?;
    let path = resolved.path;

    let has_crlf = if path.exists() {
        let existing = get_file_text_from_memory_or_disk(gcx, &path)
            .await
            .unwrap_or_default();
        existing.contains("\r\n")
    } else {
        false
    };

    let mut content = parse_string_arg(args, "content", "Provide the file content")?;
    content = normalize_line_endings(&content);
    if !content.ends_with('\n') {
        content.push('\n');
    }
    let content = restore_line_endings(&content, has_crlf);

    Ok((path, content, has_crlf, resolved.warnings))
}

pub async fn tool_create_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let (path, content, _, scope_warnings) = parse_args(gcx.clone(), args, execution_scope).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) = write_file(gcx.clone(), &path, &content, dry, None).await?;
    sync_documents_ast(gcx.clone(), &path).await?;
    let chunks = convert_edit_to_diffchunks(path.clone(), &before, &after)?;
    let summary = if before.is_empty() {
        format!(
            "✅ Created {:?}: {} lines",
            path.file_name().unwrap_or_default(),
            after.lines().count()
        )
    } else {
        edit_result_summary(&before, &after, &path)
    };
    Ok((
        before,
        after,
        chunks,
        append_scope_warnings(summary, &scope_warnings),
    ))
}

#[async_trait]
impl Tool for ToolCreateTextDoc {
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
            tool_create_text_doc_exec(gcx.clone(), args, false, execution_scope.as_ref()).await?;

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
                command: "create_textdoc".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "create_textdoc".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("create_textdoc".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["create_textdoc*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "create_textdoc".to_string(),
            display_name: "Create Text Document".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Creates a new text document or code or completely replaces the content of an existing document. Avoid trailing spaces and tabs.".to_string(),
            input_schema: json_schema_from_params(&[("path", "string", "Absolute path to new file."), ("content", "string", "The initial text or code.")], &["path", "content"]),
            output_schema: None,
            annotations: None,
        }
    }
}
