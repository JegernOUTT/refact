use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary, parse_bool_arg,
    parse_path_for_update, parse_string_arg, str_replace, sync_documents_ast,
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

pub struct ToolUpdateTextDoc {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    old_str: String,
    replacement: String,
    multiple: bool,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<PathBuf>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_update(gcx, args, privacy, code_workdir).await?;
    let old_str = parse_string_arg(args, "old_str", "Use cat() to find exact text to replace")?;
    let replacement = parse_string_arg(args, "replacement", "Provide the new text")?;
    let multiple = parse_bool_arg(args, "multiple", false)?;
    Ok(Args {
        path,
        old_str,
        replacement,
        multiple,
    })
}

pub async fn tool_update_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    code_workdir: &Option<PathBuf>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, code_workdir).await?;
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
    let summary = edit_result_summary(&before, &after, &a.path);
    Ok((before, after, chunks, summary))
}

#[async_trait]
impl Tool for ToolUpdateTextDoc {
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
        let (_, _, chunks, _) = tool_update_text_doc_exec(gcx, args, false, &None).await?;
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
            agentic: false,
            experimental: false,
            description: "Updates an existing document by replacing specific text, use this if file already exists. Optimized for large files or small changes where simple string replacement is sufficient. Avoid trailing spaces and tabs.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to the file to change.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "old_str".to_string(),
                    description: "The exact text that needs to be updated. Use update_textdoc_regex if you need pattern matching (is not preferred for common editing).".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "replacement".to_string(),
                    description: "The new text that will replace the old text.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "multiple".to_string(),
                    description: "If true, applies the replacement to all occurrences; if false, only the first occurrence is replaced.".to_string(),
                    param_type: "boolean".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string(), "old_str".to_string(), "replacement".to_string()],
        }
    }
}
