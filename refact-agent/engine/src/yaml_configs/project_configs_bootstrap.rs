use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

const CHECKSUM_FILE: &str = "default-checksums.yaml";

pub async fn project_configs_try_create_all(project_root: &Path) -> Result<(), String> {
    let refact_dir = project_root.join(".refact");

    if !project_root.exists() {
        return Err("Project root does not exist".to_string());
    }

    let dirs = ["modes", "subagents", "toolbox_commands", "code_lens"];
    for dir in &dirs {
        let dir_path = refact_dir.join(dir);
        if let Err(e) = fs::create_dir_all(&dir_path) {
            warn!("Failed to create directory {:?}: {}", dir_path, e);
        }
    }

    let checksums_path = refact_dir.join(CHECKSUM_FILE);
    let existing_checksums = load_checksums(&checksums_path);
    let mut new_checksums: HashMap<String, String> = HashMap::new();

    for (filename, content) in get_default_modes() {
        write_default_if_unchanged(&refact_dir.join("modes").join(filename), content, &existing_checksums, &mut new_checksums);
    }

    for (filename, content) in get_default_subagents() {
        write_default_if_unchanged(&refact_dir.join("subagents").join(filename), content, &existing_checksums, &mut new_checksums);
    }

    for (filename, content) in get_default_toolbox_commands() {
        write_default_if_unchanged(&refact_dir.join("toolbox_commands").join(filename), content, &existing_checksums, &mut new_checksums);
    }

    for (filename, content) in get_default_code_lens() {
        write_default_if_unchanged(&refact_dir.join("code_lens").join(filename), content, &existing_checksums, &mut new_checksums);
    }

    save_checksums(&checksums_path, &new_checksums);

    info!("Project configs created/updated in {:?}", refact_dir);
    Ok(())
}

fn load_checksums(path: &Path) -> HashMap<String, String> {
    if !path.exists() {
        return HashMap::new();
    }
    match fs::read_to_string(path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_checksums(path: &Path, checksums: &HashMap<String, String>) {
    if let Ok(content) = serde_yaml::to_string(checksums) {
        let _ = fs::write(path, content);
    }
}

fn compute_checksum(content: &str) -> String {
    format!("{:x}", md5::compute(content.as_bytes()))
}

fn write_default_if_unchanged(
    path: &Path,
    content: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    let path_str = path.to_string_lossy().to_string();
    let new_checksum = compute_checksum(content);

    new_checksums.insert(path_str.clone(), new_checksum.clone());

    if path.exists() {
        if let Ok(existing_content) = fs::read_to_string(path) {
            let existing_file_checksum = compute_checksum(&existing_content);
            if let Some(old_default_checksum) = existing_checksums.get(&path_str) {
                if &existing_file_checksum != old_default_checksum {
                    return;
                }
            } else {
                return;
            }
        }
    }

    if let Err(e) = fs::write(path, content) {
        warn!("Failed to write {:?}: {}", path, e);
    }
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
    ]
}

fn get_default_code_lens() -> Vec<(&'static str, &'static str)> {
    vec![
        ("open_chat.yaml", include_str!("defaults/code_lens/open_chat.yaml")),
        ("problems.yaml", include_str!("defaults/code_lens/problems.yaml")),
        ("explain.yaml", include_str!("defaults/code_lens/explain.yaml")),
    ]
}
