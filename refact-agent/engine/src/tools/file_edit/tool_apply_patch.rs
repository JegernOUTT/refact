use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary,
    parse_path_for_create, parse_path_for_update, restore_line_endings,
    sync_documents_ast, write_file,
};
use crate::tools::file_edit::v4a_patch::{apply_v4a_diff, ApplyDiffMode};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam,
    ToolSource, ToolSourceType,
};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::tools::file_edit::undo_history;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolApplyPatch {
    pub config_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Operation {
    CreateFile { path: String, diff: String },
    UpdateFile { path: String, diff: String },
    DeleteFile { path: String },
}

#[derive(Debug, Deserialize)]
struct ApplyPatchArgs {
    operation: Operation,
}

async fn parse_operation(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<PathBuf>,
) -> Result<(Operation, PathBuf), String> {
    let operation_value = args.get("operation")
        .ok_or("Missing 'operation' parameter")?;
    
    let operation: Operation = serde_json::from_value(operation_value.clone())
        .map_err(|e| format!("Invalid operation format: {}", e))?;
    
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    
    let (path_str, is_create) = match &operation {
        Operation::CreateFile { path, .. } => (path.clone(), true),
        Operation::UpdateFile { path, .. } => (path.clone(), false),
        Operation::DeleteFile { path } => (path.clone(), false),
    };
    
    let mut path_args = HashMap::new();
    path_args.insert("path".to_string(), json!(path_str));
    
    let resolved_path = if is_create {
        parse_path_for_create(gcx, &path_args, privacy, code_workdir).await?
    } else {
        parse_path_for_update(gcx, &path_args, privacy, code_workdir).await?
    };
    
    Ok((operation, resolved_path))
}

pub async fn tool_apply_patch_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    code_workdir: &Option<PathBuf>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let (operation, path) = parse_operation(gcx.clone(), args, code_workdir).await?;
    await_ast_indexing(gcx.clone()).await?;
    
    match operation {
        Operation::CreateFile { diff, .. } => {
            if path.exists() {
                return Err(format!("File already exists: {:?}", path));
            }
            
            let new_content = apply_v4a_diff("", &diff, ApplyDiffMode::Create)?;
            let has_crlf = false;
            let new_file_content = restore_line_endings(&new_content, has_crlf);
            
            write_file(gcx.clone(), &path, &new_file_content, dry).await?;
            sync_documents_ast(gcx.clone(), &path).await?;
            
            let chunks = convert_edit_to_diffchunks(path.clone(), &"".to_string(), &new_file_content)?;
            let summary = format!("Created file: {:?} ({} lines)", path, new_content.lines().count());
            
            Ok(("".to_string(), new_file_content, chunks, summary))
        }
        
        Operation::UpdateFile { diff, .. } => {
            let file_content = get_file_text_from_memory_or_disk(gcx.clone(), &path).await?;
            let has_crlf = file_content.contains("\r\n");
            
            let new_content = apply_v4a_diff(&file_content, &diff, ApplyDiffMode::Update)?;
            let new_file_content = restore_line_endings(&new_content, has_crlf);
            
            write_file(gcx.clone(), &path, &new_file_content, dry).await?;
            sync_documents_ast(gcx.clone(), &path).await?;
            
            let chunks = convert_edit_to_diffchunks(path.clone(), &file_content, &new_file_content)?;
            let summary = edit_result_summary(&file_content, &new_file_content, &path);
            
            Ok((file_content, new_file_content, chunks, summary))
        }
        
        Operation::DeleteFile { .. } => {
            let file_content = get_file_text_from_memory_or_disk(gcx.clone(), &path).await?;
            
            if !dry {
                undo_history::record_before_edit(&path, &file_content);
                
                std::fs::remove_file(&path)
                    .map_err(|e| format!("Failed to delete file: {}", e))?;
                
                let mut gcx_write = gcx.write().await;
                gcx_write.documents_state.memory_document_map.remove(&path);
            }
            
            let chunk = DiffChunk {
                file_name: path.to_string_lossy().to_string(),
                file_action: "remove".to_string(),
                line1: 1,
                line2: file_content.lines().count(),
                lines_remove: file_content.clone(),
                lines_add: String::new(),
                file_name_rename: None,
                is_file: true,
                application_details: format!("Deleted file: {:?}", path),
            };
            
            let summary = format!("Deleted file: {:?}", path);
            Ok((file_content, "".to_string(), vec![chunk], summary))
        }
    }
}

#[async_trait]
impl Tool for ToolApplyPatch {
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
        
        let (_, _, chunks, _) = tool_apply_patch_exec(gcx, args, false, &code_workdir).await?;
        
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
        
        let can_exec = parse_operation(gcx, args, &code_workdir).await.is_ok();
        let msgs_len = ccx.lock().await.messages.len();
        
        if msgs_len != 0 && !can_exec {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "apply_patch".to_string(),
                rule: "".to_string(),
            });
        }
        
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "apply_patch".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("apply_patch".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["apply_patch*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "apply_patch".to_string(),
            display_name: "Apply Patch".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            agentic: false,
            experimental: false,
            description: r#"Apply file operations using OpenAI 2025 V4A diff format.

Operation types:
- create_file: Create new file (fails if exists)
- update_file: Patch existing file (fails if missing)
- delete_file: Remove file (fails if missing)

Diff format (headerless V4A):
@@ optional anchor (class/function name)
 context line (space prefix)
-old line (minus prefix)
+new line (plus prefix)
 context line

Rules:
- Use 3+ lines of context for unique matching
- Add @@ anchors to disambiguate repeated code
- NO line numbers (context-based matching only)
- 0 matches = context not found error
- >1 matches = ambiguous error (add more context)
- Update mode requires context (no pure insertion)"#.to_string(),
            parameters: vec![
                ToolParam {
                    name: "operation".to_string(),
                    description: r#"Operation object with fields:
type: "create_file" | "update_file" | "delete_file"
path: relative file path from repo root
diff: headerless V4A diff (required for create/update)"#.to_string(),
                    param_type: "object".to_string(),
                },
            ],
            parameters_required: vec!["operation".to_string()],
        }
    }
}
