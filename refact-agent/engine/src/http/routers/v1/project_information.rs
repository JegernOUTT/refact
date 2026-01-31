use axum::Extension;
use axum::response::Result;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

async fn get_project_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    crate::files_correction::get_project_dirs(gcx).await
}

fn is_safe_path(path: &str, project_roots: &[PathBuf]) -> bool {
    if path.is_empty() {
        return false;
    }
    if Path::new(path).is_absolute() {
        return false;
    }
    if path.contains("..") {
        return false;
    }
    for root in project_roots {
        let full_path = root.join(path);
        if let Ok(canonical) = full_path.canonicalize() {
            if let Ok(root_canonical) = root.canonicalize() {
                if canonical.starts_with(&root_canonical) {
                    return true;
                }
            }
        }
        if full_path.starts_with(root) {
            return true;
        }
    }
    !project_roots.is_empty()
}

fn sanitize_overrides(overrides: &HashMap<String, FileOverride>, project_roots: &[PathBuf]) -> HashMap<String, FileOverride> {
    overrides
        .iter()
        .filter(|(path, _)| is_safe_path(path, project_roots))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars_per_item: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub overrides: HashMap<String, FileOverride>,
}

impl Default for SectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: None,
            max_items: None,
            max_chars_per_item: None,
            max_depth: None,
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationDefaults {
    pub max_chars_per_item: usize,
    pub max_items_per_section: usize,
}

impl Default for ProjectInformationDefaults {
    fn default() -> Self {
        Self {
            max_chars_per_item: 8000,
            max_items_per_section: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationSections {
    #[serde(default)]
    pub system_info: SectionConfig,
    #[serde(default)]
    pub environment_instructions: SectionConfig,
    #[serde(default)]
    pub detected_environments: SectionConfig,
    #[serde(default)]
    pub git_info: SectionConfig,
    #[serde(default)]
    pub project_tree: SectionConfig,
    #[serde(default)]
    pub instruction_files: SectionConfig,
    #[serde(default)]
    pub project_configs: SectionConfig,
    #[serde(default)]
    pub memories: SectionConfig,
}

impl Default for ProjectInformationSections {
    fn default() -> Self {
        Self {
            system_info: SectionConfig { enabled: true, ..Default::default() },
            environment_instructions: SectionConfig { enabled: true, max_chars: Some(6000), ..Default::default() },
            detected_environments: SectionConfig { enabled: true, max_items: Some(50), ..Default::default() },
            git_info: SectionConfig { enabled: true, max_chars: Some(6000), ..Default::default() },
            project_tree: SectionConfig { enabled: true, max_depth: Some(4), max_chars: Some(16000), ..Default::default() },
            instruction_files: SectionConfig { enabled: true, max_items: Some(20), max_chars_per_item: Some(8000), ..Default::default() },
            project_configs: SectionConfig { enabled: true, max_items: Some(30), max_chars_per_item: Some(4000), ..Default::default() },
            memories: SectionConfig { enabled: true, max_items: Some(30), max_chars_per_item: Some(2000), ..Default::default() },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub defaults: ProjectInformationDefaults,
    #[serde(default)]
    pub sections: ProjectInformationSections,
}

fn default_schema_version() -> u32 { 1 }
fn default_enabled() -> bool { true }

impl Default for ProjectInformationConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            defaults: ProjectInformationDefaults::default(),
            sections: ProjectInformationSections::default(),
        }
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationPreviewResponse {
    pub blocks: Vec<ProjectInfoBlock>,
    pub warnings: Vec<String>,
}

async fn get_config_path(gcx: Arc<ARwLock<GlobalContext>>) -> Option<PathBuf> {
    let dirs = get_project_dirs(gcx).await;
    dirs.first().map(|d| d.join(".refact").join("project_information.yaml"))
}

async fn load_config(gcx: Arc<ARwLock<GlobalContext>>) -> ProjectInformationConfig {
    let Some(path) = get_config_path(gcx).await else {
        return ProjectInformationConfig::default();
    };
    if !path.exists() {
        return ProjectInformationConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => ProjectInformationConfig::default(),
    }
}

async fn save_config(gcx: Arc<ARwLock<GlobalContext>>, config: &ProjectInformationConfig) -> std::io::Result<()> {
    let Some(path) = get_config_path(gcx).await else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No project directory"));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("yaml.tmp");
    let yaml = serde_yaml::to_string(config).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp_path, yaml)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

pub async fn handle_v1_project_information_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<ProjectInformationConfig>, ScratchError> {
    let config = load_config(gcx).await;
    Ok(axum::Json(config))
}

pub async fn handle_v1_project_information_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(mut config): axum::Json<ProjectInformationConfig>,
) -> Result<StatusCode, ScratchError> {
    let project_roots = get_project_dirs(gcx.clone()).await;
    config.sections.instruction_files.overrides = sanitize_overrides(&config.sections.instruction_files.overrides, &project_roots);
    config.sections.project_configs.overrides = sanitize_overrides(&config.sections.project_configs.overrides, &project_roots);
    config.sections.memories.overrides = sanitize_overrides(&config.sections.memories.overrides, &project_roots);
    save_config(gcx, &config).await.map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

pub async fn handle_v1_project_information_preview(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(config): axum::Json<ProjectInformationConfig>,
) -> Result<axum::Json<ProjectInformationPreviewResponse>, ScratchError> {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    if config.sections.system_info.enabled {
        let content = format!(
            "OS: {}\nDateTime: {}",
            std::env::consts::OS,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        blocks.push(ProjectInfoBlock {
            id: "system_info".into(),
            section: "system_info".into(),
            title: "System Information".into(),
            path: None,
            char_count: content.len(),
            content,
            truncated: false,
            enabled: true,
        });
    }

    if config.sections.git_info.enabled {
        let dirs = get_project_dirs(gcx.clone()).await;
        if let Some(project_dir) = dirs.first() {
            let commits = crate::git::commit_info::get_commit_information_from_current_changes(gcx.clone()).await;
            let content = if commits.is_empty() {
                "No git info available".to_string()
            } else {
                commits.iter().map(|c| c.commit_message.clone()).collect::<Vec<_>>().join("\n---\n")
            };
            let max_chars = config.sections.git_info.max_chars.unwrap_or(6000);
            let truncated = content.len() > max_chars;
            let content = if truncated { content.chars().take(max_chars).collect() } else { content };
            blocks.push(ProjectInfoBlock {
                id: "git_info".into(),
                section: "git_info".into(),
                title: "Git Information".into(),
                path: Some(project_dir.display().to_string()),
                char_count: content.len(),
                content,
                truncated,
                enabled: true,
            });
        }
    }

    if config.sections.project_tree.enabled {
        let dirs = get_project_dirs(gcx.clone()).await;
        if let Some(project_dir) = dirs.first() {
            let max_chars = config.sections.project_tree.max_chars.unwrap_or(16000);
            let content = format!("[Project tree for: {}]", project_dir.display());
            let truncated = content.len() > max_chars;
            blocks.push(ProjectInfoBlock {
                id: "project_tree".into(),
                section: "project_tree".into(),
                title: "Project Tree".into(),
                path: Some(project_dir.display().to_string()),
                char_count: content.len(),
                content,
                truncated,
                enabled: true,
            });
        }
    }

    for section in &["environment_instructions", "detected_environments", "instruction_files", "project_configs", "memories"] {
        let section_config = match *section {
            "environment_instructions" => &config.sections.environment_instructions,
            "detected_environments" => &config.sections.detected_environments,
            "instruction_files" => &config.sections.instruction_files,
            "project_configs" => &config.sections.project_configs,
            "memories" => &config.sections.memories,
            _ => continue,
        };
        if section_config.enabled {
            blocks.push(ProjectInfoBlock {
                id: format!("{}_placeholder", section),
                section: section.to_string(),
                title: format!("{} (preview)", section.replace('_', " ")),
                path: None,
                content: format!("[{} content will be loaded at runtime]", section.replace('_', " ")),
                truncated: false,
                enabled: true,
                char_count: 50,
            });
        }
    }

    if blocks.is_empty() {
        warnings.push("No sections enabled".into());
    }

    Ok(axum::Json(ProjectInformationPreviewResponse { blocks, warnings }))
}
