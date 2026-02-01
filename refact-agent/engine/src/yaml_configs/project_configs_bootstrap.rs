use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::{info, warn};

const CHECKSUM_FILE: &str = "default-checksums.yaml";

pub async fn global_configs_try_create_all(config_dir: &Path) -> Result<(), String> {
    if !config_dir.exists() {
        return Err("Config dir does not exist".to_string());
    }

    let dirs = ["modes", "subagents", "toolbox_commands", "code_lens", "knowledge", "trajectories", "tasks"];
    for dir in &dirs {
        let dir_path = config_dir.join(dir);
        if let Err(e) = fs::create_dir_all(&dir_path).await {
            warn!("Failed to create directory {:?}: {}", dir_path, e);
        }
    }

    let checksums_path = config_dir.join(CHECKSUM_FILE);
    let existing_checksums = load_checksums(&checksums_path).await;
    let mut new_checksums: HashMap<String, String> = HashMap::new();

    for (filename, content) in get_default_modes() {
        write_default_if_unchanged(&config_dir.join("modes").join(filename), content, &existing_checksums, &mut new_checksums).await;
    }

    for (filename, content) in get_default_subagents() {
        write_default_if_unchanged(&config_dir.join("subagents").join(filename), content, &existing_checksums, &mut new_checksums).await;
    }

    for (filename, content) in get_default_toolbox_commands() {
        write_default_if_unchanged(&config_dir.join("toolbox_commands").join(filename), content, &existing_checksums, &mut new_checksums).await;
    }

    for (filename, content) in get_default_code_lens() {
        write_default_if_unchanged(&config_dir.join("code_lens").join(filename), content, &existing_checksums, &mut new_checksums).await;
    }

    save_checksums(&checksums_path, &new_checksums).await;

    info!("Global configs created/updated in {:?}", config_dir);
    Ok(())
}

pub async fn project_configs_ensure_dirs(project_root: &Path) -> Result<(), String> {
    let refact_dir = project_root.join(".refact");

    if !project_root.exists() {
        return Err("Project root does not exist".to_string());
    }

    let dirs = ["modes", "subagents", "toolbox_commands", "code_lens", "knowledge", "trajectories", "tasks"];
    for dir in &dirs {
        let dir_path = refact_dir.join(dir);
        if let Err(e) = fs::create_dir_all(&dir_path).await {
            warn!("Failed to create directory {:?}: {}", dir_path, e);
        }
    }

    Ok(())
}

async fn load_checksums(path: &Path) -> HashMap<String, String> {
    if !path.exists() {
        return HashMap::new();
    }
    match fs::read_to_string(path).await {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

async fn save_checksums(path: &Path, checksums: &HashMap<String, String>) {
    if let Ok(content) = serde_yaml::to_string(checksums) {
        let _ = fs::write(path, content).await;
    }
}

pub fn compute_checksum(content: &str) -> String {
    format!("{:x}", md5::compute(content.as_bytes()))
}

async fn write_default_if_unchanged(
    path: &Path,
    content: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    let path_str = path.to_string_lossy().to_string();
    let new_checksum = compute_checksum(content);

    new_checksums.insert(path_str.clone(), new_checksum.clone());

    if path.exists() {
        match fs::read_to_string(path).await {
            Ok(existing_content) => {
                let existing_file_checksum = compute_checksum(&existing_content);
                if let Some(old_default_checksum) = existing_checksums.get(&path_str) {
                    if &existing_file_checksum != old_default_checksum {
                        return;
                    }
                } else {
                    return;
                }
            }
            Err(_) => return,
        }
    }

    if let Err(e) = fs::write(path, content).await {
        warn!("Failed to write {:?}: {}", path, e);
    }
}

pub fn get_default_checksum(kind: &str, filename: &str) -> Option<String> {
    let defaults = match kind {
        "modes" => get_default_modes(),
        "subagents" => get_default_subagents(),
        "toolbox_commands" => get_default_toolbox_commands(),
        "code_lens" => get_default_code_lens(),
        _ => return None,
    };
    defaults.iter()
        .find(|(name, _)| *name == filename)
        .map(|(_, content)| compute_checksum(content))
}

fn get_default_modes() -> Vec<(&'static str, &'static str)> {
    vec![
        ("agent.yaml", include_str!("defaults/modes/agent.yaml")),
        ("explore.yaml", include_str!("defaults/modes/explore.yaml")),
        ("task_planner.yaml", include_str!("defaults/modes/task_planner.yaml")),
        ("task_agent.yaml", include_str!("defaults/modes/task_agent.yaml")),
        ("configurator.yaml", include_str!("defaults/modes/configurator.yaml")),
        ("project_summary.yaml", include_str!("defaults/modes/project_summary.yaml")),
    ]
}

fn get_default_subagents() -> Vec<(&'static str, &'static str)> {
    vec![
        ("subagent.yaml", include_str!("defaults/subagents/subagent.yaml")),
        ("subagent_with_editing.yaml", include_str!("defaults/subagents/subagent_with_editing.yaml")),
        ("strategic_planning.yaml", include_str!("defaults/subagents/strategic_planning.yaml")),
        ("strategic_planning_gather_files.yaml", include_str!("defaults/subagents/strategic_planning_gather_files.yaml")),
        ("code_review.yaml", include_str!("defaults/subagents/code_review.yaml")),
        ("code_review_gather_files.yaml", include_str!("defaults/subagents/code_review_gather_files.yaml")),
        ("deep_research.yaml", include_str!("defaults/subagents/deep_research.yaml")),
        ("title_generation.yaml", include_str!("defaults/subagents/title_generation.yaml")),
        ("commit_message.yaml", include_str!("defaults/subagents/commit_message.yaml")),
        ("kg_enrich.yaml", include_str!("defaults/subagents/kg_enrich.yaml")),
        ("kg_deprecate.yaml", include_str!("defaults/subagents/kg_deprecate.yaml")),
        ("code_edit.yaml", include_str!("defaults/subagents/code_edit.yaml")),
        ("compress_trajectory.yaml", include_str!("defaults/subagents/compress_trajectory.yaml")),
        ("follow_up.yaml", include_str!("defaults/subagents/follow_up.yaml")),
        ("http_subchat.yaml", include_str!("defaults/subagents/http_subchat.yaml")),
        ("http_subchat_single.yaml", include_str!("defaults/subagents/http_subchat_single.yaml")),
        ("memo_extraction.yaml", include_str!("defaults/subagents/memo_extraction.yaml")),
    ]
}

fn get_default_toolbox_commands() -> Vec<(&'static str, &'static str)> {
    vec![
        ("shorter.yaml", include_str!("defaults/toolbox_commands/shorter.yaml")),
        ("bugs.yaml", include_str!("defaults/toolbox_commands/bugs.yaml")),
        ("comment.yaml", include_str!("defaults/toolbox_commands/comment.yaml")),
        ("typehints.yaml", include_str!("defaults/toolbox_commands/typehints.yaml")),
        ("explain.yaml", include_str!("defaults/toolbox_commands/explain.yaml")),
        ("summarize.yaml", include_str!("defaults/toolbox_commands/summarize.yaml")),
        ("typos.yaml", include_str!("defaults/toolbox_commands/typos.yaml")),
        ("help.yaml", include_str!("defaults/toolbox_commands/help.yaml")),
    ]
}

fn get_default_code_lens() -> Vec<(&'static str, &'static str)> {
    vec![
        ("open_chat.yaml", include_str!("defaults/code_lens/open_chat.yaml")),
        ("problems.yaml", include_str!("defaults/code_lens/problems.yaml")),
        ("explain.yaml", include_str!("defaults/code_lens/explain.yaml")),
    ]
}
