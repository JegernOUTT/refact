use std::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;
use serde_json::Value;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::files_in_workspace::enqueue_all_files_from_workspace_folders;

pub struct ToolAddWorkspaceFolder {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolAddWorkspaceFolder {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "add_workspace_folder".to_string(),
            display_name: "Add Workspace Folder".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Add a folder to the workspace so its files become available for search and editing. Use this when you need to access files in a directory that isn't currently indexed (e.g., submodules, extra_repos, or external directories).".to_string(),
            input_schema: json_schema_from_params(&[("path", "string", "Absolute path to the folder to add to the workspace.")], &["path"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let path_str = match args.get("path") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument 'path' must be a string, got: {:?}", v)),
            None => return Err("Missing required argument 'path'".to_string()),
        };

        let path = PathBuf::from(&path_str);

        if !path.exists() {
            return Err(format!("Path does not exist: {}", path_str));
        }
        if !path.is_dir() {
            return Err(format!("Path is not a directory: {}", path_str));
        }

        let abs_path = path
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path '{}': {}", path_str, e))?;

        let gcx = ccx.lock().await.global_context.clone();

        let already_exists = {
            let gcx_locked = gcx.read().await;
            let workspace_folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
            workspace_folders.contains(&abs_path)
        };

        if already_exists {
            let msg = format!("Folder is already in workspace: {}", abs_path.display());
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(msg),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        {
            let gcx_locked = gcx.read().await;
            let mut workspace_folders =
                gcx_locked.documents_state.workspace_folders.lock().unwrap();
            workspace_folders.push(abs_path.clone());
            tracing::info!(
                "add_workspace_folder: added {} to workspace folders",
                abs_path.display()
            );
        }

        let file_count = enqueue_all_files_from_workspace_folders(gcx.clone(), true, false).await;
        let msg = format!(
            "Successfully added folder to workspace: {}\nWorkspace now contains {} files.",
            abs_path.display(),
            file_count
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(msg),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}
