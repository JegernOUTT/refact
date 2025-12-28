use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    parse_path_for_create, parse_string_arg, sync_documents_ast, write_file,
};
use crate::tools::tools_description::{MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolCreateTextDoc {
    pub config_path: String,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
) -> Result<(PathBuf, String), String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_create(gcx, args, privacy).await?;
    let mut content = parse_string_arg(args, "content", "Provide the file content")?;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    Ok((path, content))
}

pub async fn tool_create_text_doc_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let (path, content) = parse_args(gcx.clone(), args).await?;
    await_ast_indexing(gcx.clone()).await?;
    let (before, after) = write_file(gcx.clone(), &path, &content, dry).await?;
    sync_documents_ast(gcx.clone(), &path).await?;
    let chunks = convert_edit_to_diffchunks(path.clone(), &before, &after)?;
    let summary = if before.is_empty() {
        format!("✅ Created {:?}: {} lines", path.file_name().unwrap_or_default(), after.lines().count())
    } else {
        edit_result_summary(&before, &after, &path)
    };
    Ok((before, after, chunks, summary))
}

#[async_trait]
impl Tool for ToolCreateTextDoc {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();
        let (_, _, chunks, _summary) = tool_create_text_doc_exec(gcx, args, false).await?;
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
            agentic: false,
            experimental: false,
            description: "Creates a new text document or code or completely replaces the content of an existing document. Avoid trailing spaces and tabs.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to new file.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "content".to_string(),
                    description: "The initial text or code.".to_string(),
                    param_type: "string".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string(), "content".to_string()],
        }
    }
}
