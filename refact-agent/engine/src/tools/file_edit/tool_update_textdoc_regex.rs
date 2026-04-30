use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    append_scope_warnings, await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    normalize_line_endings, parse_bool_arg, parse_path_for_update, parse_string_arg,
    scope_warnings_to_tool_message, str_replace_regex, sync_documents_ast,
};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::worktrees::scope::ExecutionScope;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use crate::knowledge_index::format_related_memories_section;

pub struct ToolUpdateTextDocRegex {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    pattern: Regex,
    replacement: String,
    multiple: bool,
    expected_matches: Option<usize>,
    literal: bool,
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
    let pattern_str = parse_string_arg(args, "pattern", "Provide pattern to match")?;
    let literal = parse_bool_arg(args, "literal", true)?;
    // Normalize CRLF in the pattern so it matches LF-normalized file content
    let pattern_str_normalized = normalize_line_endings(&pattern_str);
    let pattern = if literal {
        Regex::new(&regex::escape(&pattern_str_normalized))
            .map_err(|e| format!("⚠️ Pattern too complex: {}. 💡 Use shorter pattern", e))?
    } else {
        Regex::new(&pattern_str_normalized).map_err(|e| {
            format!(
                "⚠️ Invalid regex: {}. 💡 Check syntax, or set literal:true",
                e
            )
        })?
    };
    let replacement = parse_string_arg(args, "replacement", "Provide the new text")?;
    let multiple = parse_bool_arg(args, "multiple", false)?;
    let expected_matches = match args.get("expected_matches") {
        Some(Value::Number(n)) => n.as_u64().map(|v| v as usize),
        Some(Value::String(s)) => s.parse::<usize>().ok(),
        _ => None,
    };
    Ok(Args {
        path,
        pattern,
        replacement,
        multiple,
        expected_matches,
        literal,
        scope_warnings: resolved.warnings,
    })
}

pub async fn tool_update_text_doc_regex_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, execution_scope).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) = str_replace_regex(
        gcx.clone(),
        &a.path,
        &a.pattern,
        &a.replacement,
        a.multiple,
        a.expected_matches,
        a.literal,
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
impl Tool for ToolUpdateTextDocRegex {
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
            tool_update_text_doc_regex_exec(gcx.clone(), args, false, execution_scope.as_ref())
                .await?;

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
                command: "update_textdoc_regex".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "update_textdoc_regex".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("update_textdoc_regex".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["update_textdoc_regex*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "update_textdoc_regex".to_string(),
            display_name: "Update Text Document with Regex".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Updates an existing document using pattern matching. By default treats pattern as literal text (literal:true). Set literal:false for regex.".to_string(),
            input_schema: json_schema_from_params(&[("path", "string", "Absolute path to the file to change."), ("pattern", "string", "Pattern to match. Treated as literal text by default, or regex if literal:false."), ("replacement", "string", "The new text that will replace the matched pattern."), ("literal", "boolean", "If true (default), pattern is treated as literal text. If false, pattern is a regex."), ("multiple", "boolean", "If true, replaces all occurrences; if false (default), only the first."), ("expected_matches", "integer", "If provided, fails if actual match count differs (safety check).")], &["path", "pattern", "replacement"]),
            output_schema: None,
            annotations: None,
        }
    }
}
