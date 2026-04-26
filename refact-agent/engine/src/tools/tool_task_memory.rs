use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Local, Utc};
use serde_json::Value;
use tokio::fs;
use tokio::sync::Mutex as AMutex;
use tracing::info;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage::find_task_dir;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use tokio::sync::RwLock as ARwLock;

const MEMORIES_DIR: &str = "memories";
const MAX_MEMORIES_CHARS: usize = 120_000;

pub async fn get_task_memories_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<PathBuf, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    Ok(task_dir.join(MEMORIES_DIR))
}

fn generate_memory_filename(title: Option<&str>, content: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let short_uuid = &Uuid::new_v4().to_string()[..8];

    let slug = title
        .or_else(|| content.lines().next())
        .unwrap_or("memory")
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .take(40)
        .collect::<String>();

    if slug.is_empty() {
        format!("{}_{}_{}.md", timestamp, short_uuid, "memory")
    } else {
        format!("{}_{}_{}.md", timestamp, short_uuid, slug)
    }
}

pub struct ToolTaskMemorySave;

impl ToolTaskMemorySave {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMemorySave {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_memory_save".to_string(),
            display_name: "Save Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Saves a memory/note for the current task. Use this to record decisions, assumptions, API quirks, investigation results, or any useful information that should be shared with other agents and future planner iterations. Memories are automatically injected into all task chats.".to_string(),
            input_schema: json_schema_from_params(&[("content", "string", "The content to save. Can be markdown formatted."), ("title", "string", "Optional title for the memory (used in filename)."), ("tags", "string", "Optional comma-separated tags for categorization.")], &["content"]),
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
        let (gcx, task_meta) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.task_meta.clone(),
            )
        };

        let task_id = task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .ok_or("task_memory_save requires task context (task_id missing). This tool only works within task planner/agent chats.")?;

        let content = match args.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `content` is not a string: {:?}", v)),
            None => return Err("argument `content` is required".to_string()),
        };

        if content.trim().is_empty() {
            return Err("content cannot be empty".to_string());
        }

        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let memories_dir = get_task_memories_dir(gcx.clone(), &task_id).await?;
        fs::create_dir_all(&memories_dir)
            .await
            .map_err(|e| format!("Failed to create memories directory: {}", e))?;

        let filename = generate_memory_filename(title.as_deref(), &content);
        let file_path = memories_dir.join(&filename);

        let role = task_meta
            .as_ref()
            .map(|m| m.role.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let agent_id = task_meta.as_ref().and_then(|m| m.agent_id.clone());
        let card_id = task_meta.as_ref().and_then(|m| m.card_id.clone());

        let mut frontmatter = String::from("---\n");
        frontmatter.push_str(&format!("created_at: {}\n", Utc::now().to_rfc3339()));
        frontmatter.push_str(&format!("task_id: {}\n", task_id));
        frontmatter.push_str(&format!("role: {}\n", role));
        if let Some(aid) = &agent_id {
            frontmatter.push_str(&format!("agent_id: {}\n", aid));
        }
        if let Some(cid) = &card_id {
            frontmatter.push_str(&format!("card_id: {}\n", cid));
        }
        if let Some(t) = &title {
            frontmatter.push_str(&format!("title: {}\n", t));
        }
        if !tags.is_empty() {
            frontmatter.push_str(&format!("tags: [{}]\n", tags.join(", ")));
        }
        frontmatter.push_str("---\n\n");

        let full_content = if let Some(t) = &title {
            format!("{}# {}\n\n{}", frontmatter, t, content)
        } else {
            format!("{}{}", frontmatter, content)
        };

        fs::write(&file_path, &full_content)
            .await
            .map_err(|e| format!("Failed to write memory file: {}", e))?;

        info!("Task memory saved: {}", file_path.display());

        let result = format!(
            "Memory saved successfully.\nFile: {}\nTask: {}\nRole: {}",
            file_path.display(),
            task_id,
            role
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub struct ToolTaskMemoriesGet;

impl ToolTaskMemoriesGet {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMemoriesGet {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_memories_get".to_string(),
            display_name: "Get Task Memories".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Retrieves all saved memories for the current task. Returns the content of all memory files from the task's memories folder.".to_string(),
            input_schema: json_schema_from_params(&[("format", "string", "Output format: 'full' (default) returns all content, 'titles' returns only titles/filenames, 'paths' returns only file paths.")], &[]),
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
        let (gcx, task_meta) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.task_meta.clone(),
            )
        };

        let task_id = task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .ok_or("task_memories_get requires task context (task_id missing). This tool only works within task planner/agent chats.")?;

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("full");

        let memories_dir = get_task_memories_dir(gcx.clone(), &task_id).await?;

        if !memories_dir.exists() {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("No task memories found.".to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let mut memories: Vec<(PathBuf, String)> = Vec::new();

        for entry in WalkDir::new(&memories_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }

            match fs::read_to_string(path).await {
                Ok(content) => memories.push((path.to_path_buf(), content)),
                Err(e) => {
                    tracing::warn!("Failed to read memory file {:?}: {}", path, e);
                }
            }
        }

        memories.sort_by(|a, b| b.0.cmp(&a.0));

        if memories.is_empty() {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("No task memories found.".to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let result = match format {
            "paths" => {
                let paths: Vec<String> = memories
                    .iter()
                    .map(|(p, _)| p.display().to_string())
                    .collect();
                format!("## Task Memories ({})\n\n{}", paths.len(), paths.join("\n"))
            }
            "titles" => {
                let titles: Vec<String> = memories
                    .iter()
                    .map(|(p, content)| {
                        let title = content
                            .lines()
                            .find(|l| l.starts_with("# ") || l.starts_with("title:"))
                            .map(|l| {
                                l.trim_start_matches("# ")
                                    .trim_start_matches("title:")
                                    .trim()
                            })
                            .unwrap_or_else(|| {
                                p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown")
                            });
                        format!(
                            "- {} ({})",
                            title,
                            p.file_name().unwrap_or_default().to_string_lossy()
                        )
                    })
                    .collect();
                format!(
                    "## Task Memories ({})\n\n{}",
                    titles.len(),
                    titles.join("\n")
                )
            }
            _ => {
                let mut output = format!("## Task Memories ({})\n\n", memories.len());
                let mut total_chars = output.len();

                for (path, content) in &memories {
                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                    let entry = format!("--- file: {} ---\n{}\n\n", filename, content);

                    if total_chars + entry.len() > MAX_MEMORIES_CHARS {
                        output.push_str(&format!(
                            "\n[TRUNCATED: {} more memories not shown. Use format='paths' to see all.]\n",
                            memories.len() - memories.iter().position(|(p, _)| p == path).unwrap_or(0)
                        ));
                        break;
                    }

                    output.push_str(&entry);
                    total_chars += entry.len();
                }

                output
            }
        };

        info!(
            "Task memories retrieved: {} files for task {}",
            memories.len(),
            task_id
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub async fn load_task_memories(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<Vec<(PathBuf, String)>, String> {
    let memories_dir = get_task_memories_dir(gcx, task_id).await?;

    if !memories_dir.exists() {
        return Ok(vec![]);
    }

    let mut memories: Vec<(PathBuf, String)> = Vec::new();

    for entry in WalkDir::new(&memories_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" && ext != "mdx" {
            continue;
        }

        match fs::read_to_string(path).await {
            Ok(content) => memories.push((path.to_path_buf(), content)),
            Err(e) => {
                tracing::warn!("Failed to read task memory file {:?}: {}", path, e);
            }
        }
    }

    memories.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(memories)
}
