use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandSource {
    GlobalClaude,
    GlobalRefact,
    ProjectClaude(PathBuf),
    ProjectRefact(PathBuf),
    InstalledPlugin(String),
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ExtDirs {
    pub global_dirs: Vec<PathBuf>,
    pub installed_dirs: Vec<PathBuf>,
    pub project_dirs: Vec<PathBuf>,
}

impl ExtDirs {
    pub fn all_dirs_in_order(&self) -> Vec<&PathBuf> {
        self.installed_dirs
            .iter()
            .chain(self.global_dirs.iter())
            .chain(self.project_dirs.iter())
            .collect()
    }
}

pub fn is_claude_dir(dir: &Path) -> bool {
    dir.file_name().map(|n| n == ".claude").unwrap_or(false)
}

pub fn source_for_dir(
    dir: &Path,
    global_dirs: &[PathBuf],
    installed_dirs: &[PathBuf],
) -> CommandSource {
    let in_global = global_dirs.iter().any(|d| d == dir);
    let in_installed = installed_dirs.iter().any(|d| d == dir);
    let claude = is_claude_dir(dir);
    if in_installed {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        return CommandSource::InstalledPlugin(name);
    }
    match (in_global, claude) {
        (true, true) => CommandSource::GlobalClaude,
        (true, false) => CommandSource::GlobalRefact,
        (false, true) => {
            let parent = dir.parent().unwrap_or(dir).to_path_buf();
            CommandSource::ProjectClaude(parent)
        }
        (false, false) => {
            let parent = dir.parent().unwrap_or(dir).to_path_buf();
            CommandSource::ProjectRefact(parent)
        }
    }
}

pub async fn get_ext_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> ExtDirs {
    let config_dir = gcx.read().await.config_dir.clone();
    let workspace_dirs = get_project_dirs(gcx.clone()).await;

    let mut global_dirs = Vec::new();
    if let Some(home) = home::home_dir() {
        global_dirs.push(home.join(".claude"));
    }
    global_dirs.push(config_dir.clone());

    let mut installed_dirs = Vec::new();
    let installed_root = config_dir.join("plugins").join("installed");
    if let Ok(mut entries) = tokio::fs::read_dir(&installed_root).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                installed_dirs.push(path);
            }
        }
    }
    installed_dirs.sort();

    let mut project_dirs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in &workspace_dirs {
        if seen.insert(dir.clone()) {
            project_dirs.push(dir.join(".claude"));
            project_dirs.push(dir.join(".refact"));
        }
    }

    ExtDirs {
        global_dirs,
        installed_dirs,
        project_dirs,
    }
}

pub async fn collect_md_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_md_non_recursive(dir, &mut result).await;
    result.sort();
    result
}

async fn collect_md_non_recursive(dir: &Path, result: &mut Vec<PathBuf>) {
    let mut dirs_to_visit = vec![dir.to_path_buf()];
    while let Some(current) = dirs_to_visit.pop() {
        let mut entries = match tokio::fs::read_dir(&current).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                dirs_to_visit.push(path);
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                result.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_claude_dir() {
        assert!(is_claude_dir(Path::new("/home/user/.claude")));
        assert!(is_claude_dir(Path::new("/project/.claude")));
        assert!(!is_claude_dir(Path::new("/home/user/.config/refact")));
        assert!(!is_claude_dir(Path::new("/project/.refact")));
        assert!(!is_claude_dir(Path::new("/some/dir")));
    }

    #[test]
    fn test_source_for_dir_global_claude() {
        let global_claude = PathBuf::from("/home/user/.claude");
        let global_refact = PathBuf::from("/home/user/.config/refact");
        let global_dirs = vec![global_claude.clone(), global_refact.clone()];

        let src = source_for_dir(&global_claude, &global_dirs, &[]);
        assert!(matches!(src, CommandSource::GlobalClaude));
    }

    #[test]
    fn test_source_for_dir_global_refact() {
        let global_claude = PathBuf::from("/home/user/.claude");
        let global_refact = PathBuf::from("/home/user/.config/refact");
        let global_dirs = vec![global_claude.clone(), global_refact.clone()];

        let src = source_for_dir(&global_refact, &global_dirs, &[]);
        assert!(matches!(src, CommandSource::GlobalRefact));
    }

    #[test]
    fn test_source_for_dir_project_claude() {
        let global_dirs = vec![PathBuf::from("/home/user/.config/refact")];
        let project_claude = PathBuf::from("/myproject/.claude");

        let src = source_for_dir(&project_claude, &global_dirs, &[]);
        assert!(matches!(src, CommandSource::ProjectClaude(_)));
        if let CommandSource::ProjectClaude(parent) = src {
            assert_eq!(parent, PathBuf::from("/myproject"));
        }
    }

    #[test]
    fn test_source_for_dir_project_refact() {
        let global_dirs = vec![PathBuf::from("/home/user/.config/refact")];
        let project_refact = PathBuf::from("/myproject/.refact");

        let src = source_for_dir(&project_refact, &global_dirs, &[]);
        assert!(matches!(src, CommandSource::ProjectRefact(_)));
        if let CommandSource::ProjectRefact(parent) = src {
            assert_eq!(parent, PathBuf::from("/myproject"));
        }
    }

    #[test]
    fn test_source_for_dir_installed_plugin() {
        let installed_dir = PathBuf::from("/home/user/.config/refact/plugins/installed/my-plugin");
        let global_dirs = vec![PathBuf::from("/home/user/.config/refact")];
        let installed_dirs = vec![installed_dir.clone()];

        let src = source_for_dir(&installed_dir, &global_dirs, &installed_dirs);
        assert!(matches!(src, CommandSource::InstalledPlugin(_)));
        if let CommandSource::InstalledPlugin(name) = src {
            assert_eq!(name, "my-plugin");
        }
    }

    #[test]
    fn test_all_dirs_in_order() {
        let ext_dirs = ExtDirs {
            global_dirs: vec![
                PathBuf::from("/home/.claude"),
                PathBuf::from("/home/.config/refact"),
            ],
            installed_dirs: vec![PathBuf::from(
                "/home/.config/refact/plugins/installed/plugin-a",
            )],
            project_dirs: vec![
                PathBuf::from("/proj/.claude"),
                PathBuf::from("/proj/.refact"),
            ],
        };
        let all = ext_dirs.all_dirs_in_order();
        assert_eq!(all.len(), 5);
        assert_eq!(
            all[0],
            &PathBuf::from("/home/.config/refact/plugins/installed/plugin-a")
        );
        assert_eq!(all[1], &PathBuf::from("/home/.claude"));
        assert_eq!(all[2], &PathBuf::from("/home/.config/refact"));
        assert_eq!(all[3], &PathBuf::from("/proj/.claude"));
        assert_eq!(all[4], &PathBuf::from("/proj/.refact"));
    }
}
