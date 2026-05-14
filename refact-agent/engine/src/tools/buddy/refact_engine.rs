use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const REFACT_ENGINE_URL: &str = "https://github.com/smallcloudai/refact.git";
const MIRROR_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CAT_BYTES: u64 = 200 * 1024;
const MAX_SEARCH_MATCHES: usize = 50;

pub struct ToolRefactEngineClone {
    pub config_path: String,
}

pub struct ToolRefactEngineSearch {
    pub config_path: String,
}

pub struct ToolRefactEngineCat {
    pub config_path: String,
}

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn desc(
    config_path: &str,
    name: &str,
    display_name: &str,
    description: &str,
    input_schema: Value,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: source(config_path),
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn result(tool_call_id: &String, text: impl Into<String>) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.into()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

fn string_arg<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("argument `{name}` is missing or not a non-empty string"))
}

fn optional_string_arg(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bool_arg(args: &HashMap<String, Value>, name: &str, default: bool) -> Result<bool, String> {
    args.get(name)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("argument `{name}` must be a boolean"))
        })
        .unwrap_or(Ok(default))
}

fn line_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<usize>, String> {
    args.get(name)
        .map(|value| {
            let line = value
                .as_u64()
                .ok_or_else(|| format!("argument `{name}` must be a positive integer"))?;
            if line == 0 {
                return Err(format!("argument `{name}` must be a positive integer"));
            }
            Ok(line as usize)
        })
        .transpose()
}

fn cache_root() -> Result<PathBuf, String> {
    dirs::cache_dir()
        .map(|dir| dir.join("refact").join("engine-mirror"))
        .ok_or_else(|| "failed to resolve cache directory".to_string())
}

fn validate_branch_path(branch: &str) -> Result<PathBuf, String> {
    let branch = branch.trim();
    if branch.is_empty() || branch.contains('\0') {
        return Err("branch is empty or invalid".to_string());
    }
    let path = PathBuf::from(branch);
    if path.is_absolute() {
        return Err("branch cannot be an absolute path".to_string());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err("branch contains invalid path components".to_string()),
        }
    }
    Ok(path)
}

fn mirror_root_for_branch(root: &Path, branch: &str) -> Result<PathBuf, String> {
    Ok(root.join(validate_branch_path(branch)?))
}

fn mirror_root(branch: &str) -> Result<PathBuf, String> {
    mirror_root_for_branch(&cache_root()?, branch)
}

fn mirror_is_fresh(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        < MIRROR_TTL
}

fn clone_refact_engine_from_url(
    root: &Path,
    branch: &str,
    force_refresh: bool,
    url: &str,
    shallow: bool,
) -> Result<String, String> {
    let target = mirror_root_for_branch(root, branch)?;
    if target.exists() && mirror_is_fresh(&target) && !force_refresh {
        return Ok(format!("Mirror up-to-date at {}", target.display()));
    }
    let parent = target
        .parent()
        .ok_or_else(|| "failed to resolve mirror parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create mirror directory: {err}"))?;
    let tmp = parent.join(format!(
        ".{}-tmp-{}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mirror"),
        Uuid::new_v4()
    ));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(|err| format!("failed to clean temp mirror: {err}"))?;
    }
    let mut builder = git2::build::RepoBuilder::new();
    builder.branch(branch);
    if shallow {
        let mut fetch = git2::FetchOptions::new();
        fetch.depth(1);
        builder.fetch_options(fetch);
    }
    if let Err(err) = builder.clone(url, &tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(format!("failed to clone refact engine mirror: {err}"));
    }
    if target.exists() {
        let backup = parent.join(format!(
            ".{}-old-{}",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("mirror"),
            Uuid::new_v4()
        ));
        fs::rename(&target, &backup)
            .map_err(|err| format!("failed to replace existing mirror: {err}"))?;
        if let Err(err) = fs::rename(&tmp, &target) {
            let _ = fs::rename(&backup, &target);
            let _ = fs::remove_dir_all(&tmp);
            return Err(format!("failed to activate cloned mirror: {err}"));
        }
        let _ = fs::remove_dir_all(&backup);
    } else {
        fs::rename(&tmp, &target)
            .map_err(|err| format!("failed to activate cloned mirror: {err}"))?;
    }
    Ok(format!("Cloned to {}", target.display()))
}

fn reject_relative_escape(path: &Path) -> Result<(), String> {
    if path.is_absolute() {
        return Err("path must be relative".to_string());
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("path cannot contain '..'".to_string());
        }
    }
    Ok(())
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|err| format!("failed to canonicalize mirror root: {err}"))
}

fn resolve_under_root(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let raw_path = PathBuf::from(raw.trim());
    reject_relative_escape(&raw_path)?;
    let root = canonical_root(root)?;
    let path = root.join(raw_path);
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize path: {err}"))?;
    if !canonical.starts_with(&root) {
        return Err("path is outside refact engine mirror".to_string());
    }
    Ok(canonical)
}

fn resolve_scope(root: &Path, scope: Option<&str>) -> Result<PathBuf, String> {
    match scope.map(str::trim).filter(|scope| !scope.is_empty()) {
        Some(scope) => resolve_under_root(root, scope),
        None => canonical_root(root),
    }
}

fn read_file_capped(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|err| format!("failed to open file: {err}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_CAT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read file: {err}"))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn apply_line_range(
    text: &str,
    line1: Option<usize>,
    line2: Option<usize>,
) -> Result<String, String> {
    let start = line1.unwrap_or(1);
    let end = line2.unwrap_or(usize::MAX);
    if end < start {
        return Err("line2 must be greater than or equal to line1".to_string());
    }
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_no = index + 1;
            (line_no >= start && line_no <= end).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn cat_refact_engine_file_at(
    root: &Path,
    raw_path: &str,
    line1: Option<usize>,
    line2: Option<usize>,
) -> Result<String, String> {
    let path = resolve_under_root(root, raw_path)?;
    if !path.is_file() {
        return Err("path is not a file".to_string());
    }
    apply_line_range(&read_file_capped(&path)?, line1, line2)
}

fn search_refact_engine_at(
    root: &Path,
    pattern: &str,
    scope: Option<&str>,
) -> Result<String, String> {
    let regex = Regex::new(pattern).map_err(|err| format!("invalid regex pattern: {err}"))?;
    let scope = resolve_scope(root, scope)?;
    let root = canonical_root(root)?;
    let mut matches = Vec::new();
    for entry in WalkDir::new(scope).into_iter().filter_map(Result::ok) {
        if matches.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        let path = entry.path();
        let Ok(metadata) = path.symlink_metadata() else {
            continue;
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_CAT_BYTES
        {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if regex.is_match(line) {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                matches.push(format!(
                    "{}:{}: {}",
                    rel.display(),
                    index + 1,
                    line.trim_end()
                ));
                if matches.len() >= MAX_SEARCH_MATCHES {
                    break;
                }
            }
        }
    }
    if matches.is_empty() {
        Ok("No matches".to_string())
    } else {
        Ok(matches.join("\n"))
    }
}

#[async_trait]
impl Tool for ToolRefactEngineClone {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "refact_engine_clone",
            "Refact Engine Clone",
            "Clone or refresh the cached Refact engine mirror.",
            json!({
                "type": "object",
                "properties": {
                    "branch": {"type": "string", "default": "main"},
                    "force_refresh": {"type": "boolean", "default": false}
                },
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let branch = optional_string_arg(args, "branch").unwrap_or_else(|| "main".to_string());
        let message = clone_refact_engine_from_url(
            &cache_root()?,
            &branch,
            bool_arg(args, "force_refresh", false)?,
            REFACT_ENGINE_URL,
            true,
        )?;
        Ok(result(tool_call_id, message))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolRefactEngineSearch {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "refact_engine_search",
            "Refact Engine Search",
            "Regex-search the cached Refact engine mirror.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "scope": {"type": "string"}
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let output = search_refact_engine_at(
            &mirror_root("main")?,
            string_arg(args, "pattern")?,
            optional_string_arg(args, "scope").as_deref(),
        )?;
        Ok(result(tool_call_id, output))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolRefactEngineCat {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "refact_engine_cat",
            "Refact Engine Cat",
            "Read a file from the cached Refact engine mirror with path validation.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "line1": {"type": "integer", "minimum": 1},
                    "line2": {"type": "integer", "minimum": 1}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let output = cat_refact_engine_file_at(
            &mirror_root("main")?,
            string_arg(args, "path")?,
            line_arg(args, "line1")?,
            line_arg(args, "line2")?,
        )?;
        Ok(result(tool_call_id, output))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{IndexAddOption, Repository, RepositoryInitOptions, Signature};

    fn create_fake_repo(path: &Path) -> Repository {
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = Repository::init_opts(path, &opts).unwrap();
        fs::write(path.join("README.md"), "hello refact\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        drop(tree);
        repo
    }

    #[test]
    fn refact_engine_clone_caches_for_24h() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let _repo = create_fake_repo(&source);
        let cache = dir.path().join("cache");
        let first =
            clone_refact_engine_from_url(&cache, "main", false, source.to_str().unwrap(), false)
                .unwrap();
        let second =
            clone_refact_engine_from_url(&cache, "main", false, "invalid://unused", false).unwrap();
        assert!(first.contains("Cloned to"));
        assert!(second.contains("Mirror up-to-date"));
        assert!(cache.join("main").join("README.md").is_file());
    }

    #[test]
    fn refact_engine_cat_rejects_path_outside_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mirror");
        fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, "secret").unwrap();
        let link = root.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, &link).unwrap();
        let err = cat_refact_engine_file_at(&root, "link.txt", None, None).unwrap_err();
        assert!(err.contains("outside"));
    }

    #[test]
    fn refact_engine_cat_rejects_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mirror");
        fs::create_dir_all(&root).unwrap();
        let err = cat_refact_engine_file_at(&root, "../outside.txt", None, None).unwrap_err();
        assert!(err.contains(".."));
    }
}
