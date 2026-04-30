use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent};
use crate::files_correction::correct_to_nearest_filename;
use crate::global_context::GlobalContext;
use crate::subchat::{run_subchat, run_subchat_once_with_parent, resolve_subchat_config_with_parent};
use crate::tools::tool_helpers::{load_code_subagent_config, CodeSubagentConfig};

pub const DEFAULT_MAX_FILES: usize = 30;
pub const DEFAULT_GATHER_MAX_STEPS: usize = 10;

pub static DEFAULT_GATHER_FILES_TOOLS: &[&str] = &[
    "tree",
    "cat",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "knowledge",
];

pub static DEFAULT_GATHER_RETRY_PROMPT: &str = r#"Your response was not in the required format. Please output the list of relevant files in this EXACT format:

RELEVANT_FILES:
path/to/file1.ext
path/to/file2.ext
END_FILES

Include only the files you found during your investigation."#;

pub async fn send_files_gathered_message(
    subchat_tx: &Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: &str,
    files: &[PathBuf],
) {
    let file_names: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let files_preview = if file_names.len() <= 3 {
        file_names.join(", ")
    } else {
        format!("{}, …", file_names[..3].join(", "))
    };
    let message_text = format!("📁 {} files: {}", file_names.len(), files_preview);
    let msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": message_text,
        "add_message": {
            "role": "assistant",
            "content": message_text
        }
    });
    let _ = subchat_tx.lock().await.send(msg);
}

pub fn parse_relevant_files(response: &str, max_files: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut in_files_block = false;

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed == "RELEVANT_FILES:" {
            in_files_block = true;
            continue;
        }
        if trimmed == "END_FILES" {
            break;
        }
        if in_files_block
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("//")
        {
            files.push(trimmed.to_string());
        }
    }

    files.truncate(max_files);
    files
}

pub fn get_last_assistant_content(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.to_text_with_image_placeholders())
        .unwrap_or_default()
}

pub struct GatherFilesParams<'a> {
    pub default_subagent_id: &'a str,
    pub title: &'a str,
    pub default_system_prompt: &'a str,
    pub user_instruction: &'a str,
}

pub async fn gather_files_phase(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
    main_config: &CodeSubagentConfig,
    params: &GatherFilesParams<'_>,
) -> Result<Vec<PathBuf>, String> {
    let (
        parent_chat_id,
        parent_root_chat_id,
        parent_subchat_tx,
        parent_abort_flag,
        current_depth,
        parent_task_meta,
        parent_worktree,
    ) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.chat_id.clone(),
            ccx_lock.root_chat_id.clone(),
            ccx_lock.subchat_tx.clone(),
            ccx_lock.abort_flag.clone(),
            ccx_lock.subchat_depth,
            ccx_lock.task_meta.clone(),
            ccx_lock.execution_scope_worktree(),
        )
    };

    let gather_subagent_id = main_config
        .gather_subagent
        .as_deref()
        .unwrap_or(params.default_subagent_id);

    let gather_config = load_code_subagent_config(gcx.clone(), gather_subagent_id, None)
        .await
        .ok();

    let tools: Vec<String> = gather_config
        .as_ref()
        .and_then(|c| c.gather_tools.clone())
        .unwrap_or_else(|| {
            DEFAULT_GATHER_FILES_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

    let system_prompt = gather_config
        .as_ref()
        .and_then(|c| c.gather_system_prompt.clone())
        .or_else(|| {
            if params.default_system_prompt.is_empty() {
                None
            } else {
                Some(params.default_system_prompt.to_string())
            }
        })
        .ok_or_else(|| {
            format!(
                "gather_system_prompt not configured for {}",
                gather_subagent_id
            )
        })?;

    let retry_prompt = gather_config
        .as_ref()
        .and_then(|c| c.gather_retry_prompt.clone())
        .unwrap_or_else(|| DEFAULT_GATHER_RETRY_PROMPT.to_string());

    let max_steps = main_config
        .gather_max_steps
        .or_else(|| gather_config.as_ref().and_then(|c| c.max_steps))
        .unwrap_or(DEFAULT_GATHER_MAX_STEPS);

    let max_files = main_config.max_files.unwrap_or(DEFAULT_MAX_FILES);

    let subchat_config = resolve_subchat_config_with_parent(
        gcx.clone(),
        gather_subagent_id,
        true,
        None,
        Some(params.title.to_string()),
        Some(parent_chat_id),
        Some("gather_files".to_string()),
        Some(parent_root_chat_id),
        Some(tools),
        max_steps,
        false,
        None,
        "agent".to_string(),
        parent_task_meta.clone(),
        parent_worktree.clone(),
        Some(tool_call_id.clone()),
        Some(parent_subchat_tx.clone()),
        Some(parent_abort_flag.clone()),
        current_depth + 1,
    )
    .await?;

    let mut messages = vec![ChatMessage {
        role: "system".to_string(),
        content: ChatContent::SimpleText(system_prompt),
        ..Default::default()
    }];

    for msg in external_messages.iter() {
        if msg.role == "user" || msg.role == "assistant" || msg.role == "tool" {
            messages.push(msg.clone());
        }
    }

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(params.user_instruction.to_string()),
        ..Default::default()
    });

    tracing::info!("{}: starting file-gathering subagent", gather_subagent_id);
    let result = run_subchat(gcx.clone(), messages.clone(), subchat_config).await?;

    let response = get_last_assistant_content(&result.messages);
    let mut files = parse_relevant_files(&response, max_files);

    if files.is_empty() {
        tracing::info!(
            "{}: file list not properly formatted, requesting retry",
            gather_subagent_id
        );
        let mut retry_messages = result.messages.clone();
        retry_messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(retry_prompt),
            ..Default::default()
        });

        let retry_result = run_subchat_once_with_parent(
            gcx.clone(),
            gather_subagent_id,
            retry_messages,
            tool_call_id.clone(),
            parent_subchat_tx.clone(),
            parent_abort_flag.clone(),
            current_depth,
            parent_task_meta.clone(),
            parent_worktree.clone(),
        )
        .await?;
        let retry_response = get_last_assistant_content(&retry_result.messages);
        files = parse_relevant_files(&retry_response, max_files);

        if files.is_empty() {
            return Err("File-gathering subagent failed to provide a valid file list".to_string());
        }
    }

    tracing::info!("{}: gathered {} files", gather_subagent_id, files.len());

    let mut valid_paths = Vec::new();
    let mut seen = HashSet::new();
    for file_str in files {
        let candidates = correct_to_nearest_filename(gcx.clone(), &file_str, false, 1).await;
        if let Some(corrected) = candidates.first() {
            let path = PathBuf::from(corrected);
            if !seen.contains(&path) {
                seen.insert(path.clone());
                valid_paths.push(path);
            }
        } else {
            tracing::warn!(
                "{}: skipping invalid path: {}",
                gather_subagent_id,
                file_str
            );
        }
    }

    if valid_paths.is_empty() {
        return Err("No valid files found from the gathered list".to_string());
    }

    send_files_gathered_message(&parent_subchat_tx, &tool_call_id, &valid_paths).await;

    Ok(valid_paths)
}
