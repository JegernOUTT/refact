use std::fs;
#[cfg(not(windows))]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use crate::path_utils::canonicalize_normalized_path;

const LARGE_FILE_SIZE_THRESHOLD: u64 = 4096 * 1024; // 4Mb files
const SMALL_FILE_SIZE_THRESHOLD: u64 = 5; // 5 Bytes

pub const KNOWLEDGE_FOLDER_NAME: &str = ".refact/knowledge";

const ALLOWED_HIDDEN_FOLDERS: &[&str] = &[".refact"];

pub const SOURCE_FILE_EXTENSIONS: &[&str] = &[
    "c",
    "cpp",
    "cc",
    "h",
    "hpp",
    "cs",
    "java",
    "py",
    "rb",
    "go",
    "rs",
    "swift",
    "php",
    "js",
    "jsx",
    "ts",
    "tsx",
    "lua",
    "pl",
    "r",
    "sh",
    "bat",
    "cmd",
    "ps1",
    "m",
    "kt",
    "kts",
    "groovy",
    "dart",
    "fs",
    "fsx",
    "fsi",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "json",
    "xml",
    "yml",
    "yaml",
    "md",
    "sql",
    "cfg",
    "conf",
    "ini",
    "toml",
    "dockerfile",
    "ipynb",
    "rmd",
    "xml",
    "kt",
    "xaml",
    "unity",
    "gd",
    "uproject",
    "asm",
    "s",
    "tex",
    "makefile",
    "mk",
    "cmake",
    "gradle",
    "liquid",
];

pub fn is_generated_index_path(path: &Path) -> bool {
    if !path.file_name().is_some_and(|name| name == "index.json") {
        return false;
    }
    let parts = normal_components(path);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    for idx in 0..parts.len() {
        if parts[idx] == ".refact" && is_generated_index_suffix(&parts[idx + 1..]) {
            return true;
        }
    }
    false
}

pub fn is_generated_index_path_with_global_config_roots(
    path: &Path,
    global_config_roots: &[PathBuf],
) -> bool {
    if is_generated_index_path(path) {
        return true;
    }
    if !path.file_name().is_some_and(|name| name == "index.json") {
        return false;
    }
    let path = canonicalize_normalized_path(path.to_path_buf());
    global_config_roots.iter().any(|root| {
        let root = canonicalize_normalized_path(root.clone());
        path.strip_prefix(root)
            .ok()
            .is_some_and(is_generated_index_relative_path)
    })
}

fn normal_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect()
}

fn is_generated_index_relative_path(path: &Path) -> bool {
    let parts = normal_components(path);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    is_generated_index_suffix(&parts)
}

fn is_generated_index_suffix(parts: &[&str]) -> bool {
    matches!(parts, ["trajectories", "index.json"])
        || matches!(parts, ["tasks", "index.json"])
        || matches!(parts, ["tasks", _, "trajectories", "planner", "index.json"])
        || matches!(parts, ["tasks", _, "trajectories", "agents", "index.json"])
        || matches!(
            parts,
            ["tasks", _, "trajectories", "agents", _, "index.json"]
        )
}

pub fn is_refact_codegraph_path(path: &Path) -> bool {
    let mut last_was_refact = false;
    for component in path.components() {
        if last_was_refact && component == Component::Normal("codegraph".as_ref()) {
            return true;
        }
        last_was_refact = component == Component::Normal(".refact".as_ref());
    }
    false
}

pub fn is_transient_tmp_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with(".tmp") || name.contains(".tmp-") || name.contains(".tmp.")
        })
}

fn is_in_allowed_hidden_folder(path: &PathBuf) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .map(|name| ALLOWED_HIDDEN_FOLDERS.contains(&name.to_string_lossy().as_ref()))
            .unwrap_or(false)
    })
}

pub fn is_valid_file(
    path: &PathBuf,
    allow_hidden_folders: bool,
    ignore_size_thresholds: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Err("Path is not a file".into());
    }

    if is_refact_codegraph_path(path) {
        return Err(".refact/codegraph is internal".into());
    }

    if is_transient_tmp_path(path) {
        return Err("Transient tmp file".into());
    }

    let in_allowed_hidden = is_in_allowed_hidden_folder(path);

    if !allow_hidden_folders
        && !in_allowed_hidden
        && path.ancestors().any(|ancestor| {
            ancestor
                .file_name()
                .map(|name| name.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
        })
    {
        return Err("Parent dir starts with a dot".into());
    }

    if let Ok(metadata) = fs::metadata(path) {
        let file_size = metadata.len();
        if !ignore_size_thresholds && file_size < SMALL_FILE_SIZE_THRESHOLD {
            return Err("File size is too small".into());
        }
        if !ignore_size_thresholds && file_size > LARGE_FILE_SIZE_THRESHOLD {
            return Err("File size is too large".into());
        }
        #[cfg(not(windows))]
        {
            let permissions = metadata.permissions();
            if permissions.mode() & 0o400 == 0 {
                return Err("File has no read permissions".into());
            }
        }
    } else {
        return Err("Unable to access file metadata".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        is_generated_index_path, is_generated_index_path_with_global_config_roots,
        is_transient_tmp_path,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn transient_tmp_paths_are_detected() {
        for path in [
            "/repo/.refact/buddy/state.json.tmp",
            "/repo/.refact/buddy/runtime_queue.jsonl.tmp",
            "/repo/.refact/buddy/chats/workflows/memo_extraction.json.tmp",
            "/repo/.refact/trajectories/.index.json.tmp-0a1b2c3d",
            "/repo/.refact/trajectories/chat-1.json.tmp.0a1b2c3d",
            "/repo/anything.tmp",
        ] {
            assert!(is_transient_tmp_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn regular_paths_are_not_transient_tmp() {
        for path in [
            "/repo/src/main.rs",
            "/repo/tmp.rs",
            "/repo/templates/foo.tmpl",
            "/repo/.refact/buddy/state.json",
            "/repo/docs/tmp/readme.md",
        ] {
            assert!(!is_transient_tmp_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn generated_refact_index_paths_match_exact_generated_shapes() {
        for path in [
            "/repo/.refact/trajectories/index.json",
            "/repo/.refact/tasks/index.json",
            "/repo/.refact/tasks/task-1/trajectories/planner/index.json",
            "/repo/.refact/tasks/task-1/trajectories/agents/index.json",
            "/repo/.refact/tasks/task-1/trajectories/agents/agent-1/index.json",
        ] {
            assert!(is_generated_index_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn generated_refact_index_paths_do_not_match_near_misses() {
        for path in [
            "/repo/trajectories/planner/index.json",
            "/repo/.refact/tasks/task-1/trajectories/docs/index.json",
            "/repo/.refact/tasks/task-1/trajectories/planner/archive/index.json",
            "/repo/.refact/tasks/task-1/notes/index.json",
            "/repo/.refact/knowledge/index.json",
            "/work/refact/docs/trajectories/index.json",
            "/home/user/refact/trajectories/index.json",
            "/repo/.config/refact/trajectories/index.json",
            "/repo/.config/not-refact/trajectories/index.json",
        ] {
            assert!(!is_generated_index_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn generated_global_config_index_paths_match_only_configured_roots() {
        let roots = vec![PathBuf::from("/home/user/.config/refact")];
        for path in [
            "/home/user/.config/refact/trajectories/index.json",
            "/home/user/.config/refact/tasks/index.json",
            "/home/user/.config/refact/tasks/task-1/trajectories/planner/index.json",
            "/home/user/.config/refact/tasks/task-1/trajectories/agents/index.json",
            "/home/user/.config/refact/tasks/task-1/trajectories/agents/agent-1/index.json",
        ] {
            assert!(
                is_generated_index_path_with_global_config_roots(Path::new(path), &roots),
                "{path}"
            );
            assert!(!is_generated_index_path(Path::new(path)), "{path}");
        }

        for path in [
            "/repo/.config/refact/trajectories/index.json",
            "/repo/.config/refact/tasks/index.json",
            "/home/user/.config/not-refact/trajectories/index.json",
            "/home/user/.config/refact/knowledge/index.json",
        ] {
            assert!(
                !is_generated_index_path_with_global_config_roots(Path::new(path), &roots),
                "{path}"
            );
        }
    }
}
