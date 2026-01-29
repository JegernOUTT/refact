use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary, parse_path_for_update,
    parse_string_arg, str_replace_lines, sync_documents_ast,
};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolUpdateTextDocByLines {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    content: String,
    ranges: String,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<PathBuf>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_update(gcx, args, privacy, code_workdir).await?;
    let content = parse_string_arg(args, "content", "Provide the new text for the line range")?;
    let ranges = parse_string_arg(args, "ranges", "Format: '10:20' or ':5' or '100:' or '5'")?;
    let ranges = ranges.trim().to_string();
    if ranges.is_empty() {
        return Err(
            "⚠️ 'ranges' cannot be empty. 💡 Format: '10:20' or ':5' or '100:'".to_string(),
        );
    }
    Ok(Args {
        path,
        content,
        ranges,
    })
}

pub async fn tool_update_text_doc_by_lines_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    code_workdir: &Option<PathBuf>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, code_workdir).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) =
        str_replace_lines(gcx.clone(), &a.path, &a.content, &a.ranges, dry).await?;
    sync_documents_ast(gcx.clone(), &a.path).await?;
    let chunks = convert_edit_to_diffchunks(a.path.clone(), &before, &after)?;
    let summary = edit_result_summary(&before, &after, &a.path);
    Ok((before, after, chunks, summary))
}

#[async_trait]
impl Tool for ToolUpdateTextDocByLines {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = {
            let ccx_locked = ccx.lock().await;
            ccx_locked.global_context.clone()
        };
        let (_, _, chunks, _) =
            tool_update_text_doc_by_lines_exec(gcx, args, false, &None).await?;
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
        let gcx = {
            let ccx_locked = ccx.lock().await;
            ccx_locked.global_context.clone()
        };
        let can_exec = parse_args(gcx.clone(), args, &None).await.is_ok();
        let msgs_len = ccx.lock().await.messages.len();
        if msgs_len != 0 && !can_exec {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "update_textdoc_by_lines".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "update_textdoc_by_lines".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("update_textdoc_by_lines".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["update_textdoc_by_lines*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "update_textdoc_by_lines".to_string(),
            display_name: "Update Text Document By Lines".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            agentic: false,
            experimental: false,
            description: "Replaces line ranges in an existing file with new content. Line numbers are 1-based and inclusive. Supports multiple non-overlapping ranges.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to the file to modify.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "content".to_string(),
                    description: "The new text content. For multiple ranges, separate content for each range with '---RANGE_SEPARATOR---'.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "ranges".to_string(),
                    description: "Line ranges to replace. Format: ':3' (lines 1-3), '40:50' (lines 40-50), '100:' (line 100 to end), '5' (just line 5). Combine multiple ranges with commas: ':3,40:50,100:'. Ranges must not overlap.".to_string(),
                    param_type: "string".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string(), "content".to_string(), "ranges".to_string()],
        }
    }
}
