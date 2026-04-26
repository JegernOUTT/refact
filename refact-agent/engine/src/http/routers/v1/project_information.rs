use axum::Extension;
use axum::response::Result;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::chat::system_context::{
    SystemInfo, find_instruction_files, find_project_configs, gather_git_info, detect_environments,
    generate_compact_project_tree, generate_git_info_prompt, generate_environment_instructions,
};
use crate::memories::load_memories_by_tags;

pub use crate::yaml_configs::project_information::{
    ProjectInformationConfig, load_project_information_config, save_project_information_config,
    to_relative_path, sanitize_overrides,
};

async fn get_project_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    crate::files_correction::get_project_dirs(gcx).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfoBlock {
    pub id: String,
    pub section: String,
    pub title: String,
    pub path: Option<String>,
    pub content: String,
    pub truncated: bool,
    pub enabled: bool,
    pub char_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_char_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationPreviewResponse {
    pub blocks: Vec<ProjectInfoBlock>,
    pub warnings: Vec<String>,
}

pub async fn handle_v1_project_information_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<ProjectInformationConfig>, ScratchError> {
    let config = load_project_information_config(gcx).await;
    Ok(axum::Json(config))
}

pub async fn handle_v1_project_information_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(mut config): axum::Json<ProjectInformationConfig>,
) -> Result<StatusCode, ScratchError> {
    let project_roots = get_project_dirs(gcx.clone()).await;
    config.sections.instruction_files.overrides =
        sanitize_overrides(&config.sections.instruction_files.overrides, &project_roots);
    config.sections.project_configs.overrides =
        sanitize_overrides(&config.sections.project_configs.overrides, &project_roots);
    config.sections.memories.overrides =
        sanitize_overrides(&config.sections.memories.overrides, &project_roots);
    save_project_information_config(gcx, &config)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

fn count_chars(s: &str) -> usize {
    s.chars().count()
}

struct TruncateResult {
    content: String,
    truncated: bool,
    char_count: usize,
    original_char_count: usize,
}

fn truncate_to_chars(s: &str, max_chars: usize) -> TruncateResult {
    let original_char_count = s.chars().count();
    if original_char_count > max_chars {
        let content: String = s.chars().take(max_chars).collect();
        let char_count = max_chars;
        TruncateResult {
            content,
            truncated: true,
            char_count,
            original_char_count,
        }
    } else {
        TruncateResult {
            content: s.to_string(),
            truncated: false,
            char_count: original_char_count,
            original_char_count,
        }
    }
}

pub async fn handle_v1_project_information_preview(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(config): axum::Json<ProjectInformationConfig>,
) -> Result<axum::Json<ProjectInformationPreviewResponse>, ScratchError> {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    if !config.enabled {
        warnings.push("Project information is disabled".into());
        return Ok(axum::Json(ProjectInformationPreviewResponse {
            blocks,
            warnings,
        }));
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    let environments = detect_environments(&project_dirs).await;

    if config.sections.system_info.enabled {
        let sys_info = SystemInfo::gather();
        let raw_content = sys_info.to_prompt_string();
        let max_chars = config.sections.system_info.max_chars.unwrap_or(4000);
        let tr = truncate_to_chars(&raw_content, max_chars);
        blocks.push(ProjectInfoBlock {
            id: "system_info".into(),
            section: "system_info".into(),
            title: "System Information".into(),
            path: None,
            char_count: tr.char_count,
            original_char_count: if tr.truncated {
                Some(tr.original_char_count)
            } else {
                None
            },
            content: tr.content,
            truncated: tr.truncated,
            enabled: true,
        });
    }

    if config.sections.environment_instructions.enabled {
        let raw_content = generate_environment_instructions(&environments);
        let max_chars = config
            .sections
            .environment_instructions
            .max_chars
            .unwrap_or(6000);
        let tr = truncate_to_chars(&raw_content, max_chars);
        blocks.push(ProjectInfoBlock {
            id: "environment_instructions".into(),
            section: "environment_instructions".into(),
            title: "Environment Instructions".into(),
            path: None,
            char_count: tr.char_count,
            original_char_count: if tr.truncated {
                Some(tr.original_char_count)
            } else {
                None
            },
            content: tr.content,
            truncated: tr.truncated,
            enabled: true,
        });
    }

    if config.sections.detected_environments.enabled {
        let max_items = config
            .sections
            .detected_environments
            .max_items
            .unwrap_or(50);
        let truncated = environments.len() > max_items;
        let envs_to_show: Vec<_> = environments.iter().take(max_items).collect();
        let content = if envs_to_show.is_empty() {
            "No environments detected".to_string()
        } else {
            envs_to_show
                .iter()
                .map(|e| format!("- {} ({}): {}", e.env_type, e.path, e.description))
                .collect::<Vec<_>>()
                .join("\n")
        };
        blocks.push(ProjectInfoBlock {
            id: "detected_environments".into(),
            section: "detected_environments".into(),
            title: format!("Detected Environments ({} items)", envs_to_show.len()),
            path: None,
            char_count: count_chars(&content),
            original_char_count: None,
            content,
            truncated,
            enabled: true,
        });
    }

    if config.sections.git_info.enabled {
        let git_infos = gather_git_info(&project_dirs).await;
        let raw_content = generate_git_info_prompt(&git_infos);
        let raw_content = if raw_content.is_empty() {
            "No git repositories found".to_string()
        } else {
            raw_content
        };
        let max_chars = config.sections.git_info.max_chars.unwrap_or(6000);
        let tr = truncate_to_chars(&raw_content, max_chars);
        blocks.push(ProjectInfoBlock {
            id: "git_info".into(),
            section: "git_info".into(),
            title: "Git Information".into(),
            path: project_dirs.first().map(|p| p.display().to_string()),
            char_count: tr.char_count,
            original_char_count: if tr.truncated {
                Some(tr.original_char_count)
            } else {
                None
            },
            content: tr.content,
            truncated: tr.truncated,
            enabled: true,
        });
    }

    if config.sections.project_tree.enabled {
        let max_depth = config.sections.project_tree.max_depth.unwrap_or(4);
        let max_chars = config.sections.project_tree.max_chars.unwrap_or(16000);
        let tr = match generate_compact_project_tree(gcx.clone(), max_depth).await {
            Ok(tree) => truncate_to_chars(&tree, max_chars),
            Err(e) => TruncateResult {
                content: format!("Failed to generate project tree: {}", e),
                truncated: false,
                char_count: 0,
                original_char_count: 0,
            },
        };
        blocks.push(ProjectInfoBlock {
            id: "project_tree".into(),
            section: "project_tree".into(),
            title: "Project Tree".into(),
            path: project_dirs.first().map(|p| p.display().to_string()),
            char_count: tr.char_count,
            original_char_count: if tr.truncated {
                Some(tr.original_char_count)
            } else {
                None
            },
            content: tr.content,
            truncated: tr.truncated,
            enabled: true,
        });
    }

    if config.sections.instruction_files.enabled {
        let instruction_files = find_instruction_files(&project_dirs).await;
        let max_items = config.sections.instruction_files.max_items.unwrap_or(20);
        let default_max_chars = config
            .sections
            .instruction_files
            .max_chars_per_item
            .unwrap_or(8000);
        let overrides = &config.sections.instruction_files.overrides;
        let list_truncated = instruction_files.len() > max_items;
        let files_to_show: Vec<_> = instruction_files.into_iter().take(max_items).collect();

        for (idx, file) in files_to_show.iter().enumerate() {
            // Use relative path as the override key (consistent with UI and sanitization)
            let override_key = to_relative_path(&file.file_path, &project_dirs);
            let file_override = override_key.as_ref().and_then(|k| overrides.get(k));
            let file_enabled = file_override.and_then(|o| o.enabled).unwrap_or(true);
            let max_chars_per_item = file_override
                .and_then(|o| o.max_chars)
                .unwrap_or(default_max_chars);

            let raw_content = if let Some(ref processed) = file.processed_content {
                processed.clone()
            } else {
                match tokio::fs::read_to_string(&file.file_path).await {
                    Ok(c) => c,
                    Err(_) => "[Could not read file]".to_string(),
                }
            };
            let tr = truncate_to_chars(&raw_content, max_chars_per_item);
            blocks.push(ProjectInfoBlock {
                id: format!("instruction_file_{}", idx),
                section: "instruction_files".into(),
                title: file.file_name.clone(),
                // Return relative path as the key for UI to use when saving overrides
                path: override_key.or_else(|| Some(file.file_path.clone())),
                char_count: if file_enabled { tr.char_count } else { 0 },
                original_char_count: if tr.truncated {
                    Some(tr.original_char_count)
                } else {
                    None
                },
                content: tr.content,
                truncated: tr.truncated,
                enabled: file_enabled,
            });
        }

        if files_to_show.is_empty() {
            let content = "No instruction files found (AGENTS.md, .cursorrules, etc.)".to_string();
            blocks.push(ProjectInfoBlock {
                id: "instruction_files_empty".into(),
                section: "instruction_files".into(),
                title: "Instruction Files".into(),
                path: None,
                char_count: count_chars(&content),
                original_char_count: None,
                content,
                truncated: false,
                enabled: true,
            });
        } else if list_truncated {
            warnings.push(format!(
                "Instruction files truncated to {} items",
                max_items
            ));
        }
    }

    if config.sections.project_configs.enabled {
        let project_configs = find_project_configs(&project_dirs).await;
        let max_items = config.sections.project_configs.max_items.unwrap_or(30);
        let truncated = project_configs.len() > max_items;
        let configs_to_show: Vec<_> = project_configs.into_iter().take(max_items).collect();

        let content = if configs_to_show.is_empty() {
            "No project config files found".to_string()
        } else {
            configs_to_show
                .iter()
                .map(|c| format!("- {} [{}]", c.file_name, c.category))
                .collect::<Vec<_>>()
                .join("\n")
        };
        blocks.push(ProjectInfoBlock {
            id: "project_configs".into(),
            section: "project_configs".into(),
            title: format!("Project Configs ({} files)", configs_to_show.len()),
            path: None,
            char_count: count_chars(&content),
            original_char_count: None,
            content,
            truncated,
            enabled: true,
        });
    }

    if config.sections.memories.enabled {
        let memory_tags = &["preference", "lesson", "insight", "pattern"];
        let max_items = config.sections.memories.max_items.unwrap_or(30);
        let memories = load_memories_by_tags(gcx.clone(), memory_tags, max_items)
            .await
            .unwrap_or_default();
        let default_max_chars = config.sections.memories.max_chars_per_item.unwrap_or(2000);
        let overrides = &config.sections.memories.overrides;

        for (idx, memo) in memories.iter().enumerate() {
            let abs_path_str = memo.file_path.as_ref().map(|p| p.display().to_string());
            // Use relative path as the override key (consistent with UI and sanitization)
            let override_key = abs_path_str
                .as_ref()
                .and_then(|p| to_relative_path(p, &project_dirs));
            let file_override = override_key.as_ref().and_then(|k| overrides.get(k));
            let file_enabled = file_override.and_then(|o| o.enabled).unwrap_or(true);
            let max_chars_per_item = file_override
                .and_then(|o| o.max_chars)
                .unwrap_or(default_max_chars);

            let tr = truncate_to_chars(&memo.content, max_chars_per_item);
            let title = memo
                .file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("Memory {}", idx + 1));
            blocks.push(ProjectInfoBlock {
                id: format!("memory_{}", idx),
                section: "memories".into(),
                title,
                // Return relative path as the key for UI to use when saving overrides
                path: override_key.or(abs_path_str),
                char_count: if file_enabled { tr.char_count } else { 0 },
                original_char_count: if tr.truncated {
                    Some(tr.original_char_count)
                } else {
                    None
                },
                content: tr.content,
                truncated: tr.truncated,
                enabled: file_enabled,
            });
        }

        if memories.is_empty() {
            let content = "No memories found".to_string();
            blocks.push(ProjectInfoBlock {
                id: "memories_empty".into(),
                section: "memories".into(),
                title: "Memories".into(),
                path: None,
                char_count: count_chars(&content),
                original_char_count: None,
                content,
                truncated: false,
                enabled: true,
            });
        }
    }

    if blocks.is_empty() {
        warnings.push("No sections enabled".into());
    }

    Ok(axum::Json(ProjectInformationPreviewResponse {
        blocks,
        warnings,
    }))
}
