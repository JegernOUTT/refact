use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    parse_bool_arg, parse_path_for_update, parse_string_arg, str_replace_regex, sync_documents_ast,
};
use crate::tools::tools_description::{MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolUpdateTextDocRegex {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    pattern: Regex,
    replacement: String,
    multiple: bool,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_update(gcx, args, privacy).await?;
    let pattern_str = parse_string_arg(args, "pattern", "Provide regex pattern to match")?;
    let pattern = Regex::new(&pattern_str)
        .map_err(|e| format!("⚠️ Invalid regex: {}. 💡 Check syntax, escape special chars with \\", e))?;
    let replacement = parse_string_arg(args, "replacement", "Provide the new text")?;
    let multiple = parse_bool_arg(args, "multiple", false)?;
    Ok(Args { path, pattern, replacement, multiple })
}

pub async fn tool_update_text_doc_regex_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) = str_replace_regex(gcx.clone(), &a.path, &a.pattern, &a.replacement, a.multiple, dry).await?;
    sync_documents_ast(gcx.clone(), &a.path).await?;
    let chunks = convert_edit_to_diffchunks(a.path.clone(), &before, &after)?;
    let summary = edit_result_summary(&before, &after, &a.path);
    Ok((before, after, chunks, summary))
}

#[async_trait]
impl Tool for ToolUpdateTextDocRegex {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();
        let (_, _, chunks, _summary) = tool_update_text_doc_regex_exec(gcx, args, false).await?;
        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText(json!(chunks).to_string()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let gcx = ccx.lock().await.global_context.clone();
        let can_exec = parse_args(gcx, args).await.is_ok();
        let msgs_len = ccx.lock().await.messages.len();
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
            agentic: false,
            experimental: false,
            description: "Updates an existing document using regex pattern matching. Ideal when changes can be expressed as a regular expression or when you need to match variable text patterns. Avoid trailing spaces and tabs.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to the file to change.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "pattern".to_string(),
                    description: "A regex pattern to match the text that needs to be updated. Prefer simpler regexes for better performance.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "replacement".to_string(),
                    description: "The new text that will replace the matched pattern.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "multiple".to_string(),
                    description: "If true, applies the replacement to all occurrences; if false, only the first occurrence is replaced.".to_string(),
                    param_type: "boolean".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string(), "pattern".to_string(), "replacement".to_string()],
        }
    }
}
