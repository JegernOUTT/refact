use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::ext::config_dirs::{source_for_dir, CommandSource, ExtDirs};
use crate::ext::slash_commands::{parse_frontmatter_and_body};

const MAX_FILE_SIZE: u64 = 100 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillIndex {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFull {
    pub index: SkillIndex,
    pub argument_hint: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub context: Option<String>,
    pub agent: Option<String>,
    pub body: String,
    pub skill_dir: PathBuf,
}

fn yaml_str(v: &serde_yaml::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn yaml_str_list(v: &serde_yaml::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn yaml_bool(v: &serde_yaml::Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

async fn load_skill_from_dir(
    skill_dir: &Path,
    source: CommandSource,
) -> Option<SkillFull> {
    let skill_md = skill_dir.join("SKILL.md");
    if !skill_md.exists() {
        return None;
    }
    let metadata = tokio::fs::metadata(&skill_md).await.ok()?;
    if metadata.len() > MAX_FILE_SIZE {
        tracing::warn!("Skipping SKILL.md > 100KB: {:?}", skill_md);
        return None;
    }
    let content = match tokio::fs::read_to_string(&skill_md).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read SKILL.md {:?}: {}", skill_md, e);
            return None;
        }
    };
    let (fm, body) = parse_frontmatter_and_body(&content);
    let name = yaml_str(&fm, "name");
    if name.is_empty() {
        tracing::warn!("SKILL.md missing required 'name' field: {:?}", skill_md);
        return None;
    }
    let description = yaml_str(&fm, "description");
    if description.is_empty() {
        tracing::warn!("SKILL.md missing required 'description' field: {:?}", skill_md);
        return None;
    }
    let user_invocable = yaml_bool(&fm, "user-invocable", true);
    let disable_model_invocation = yaml_bool(&fm, "disable-model-invocation", false);
    let argument_hint = yaml_str(&fm, "argument-hint");
    let allowed_tools = yaml_str_list(&fm, "allowed-tools");
    let model = fm.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
    let context = fm.get("context").and_then(|v| v.as_str()).map(|s| s.to_string());
    let agent = fm.get("agent").and_then(|v| v.as_str()).map(|s| s.to_string());
    let index = SkillIndex {
        name,
        description,
        user_invocable,
        disable_model_invocation,
        source,
    };
    Some(SkillFull {
        index,
        argument_hint,
        allowed_tools,
        model,
        context,
        agent,
        body,
        skill_dir: skill_dir.to_path_buf(),
    })
}

async fn scan_skills_dir(skills_dir: &Path) -> Vec<PathBuf> {
    let mut skill_dirs = Vec::new();
    let mut entries = match tokio::fs::read_dir(skills_dir).await {
        Ok(e) => e,
        Err(_) => return skill_dirs,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            skill_dirs.push(path);
        }
    }
    skill_dirs.sort();
    skill_dirs
}

pub async fn load_skill_indices(ext_dirs: &ExtDirs) -> Vec<SkillIndex> {
    let mut seen: std::collections::HashMap<String, SkillIndex> = std::collections::HashMap::new();
    for dir in ext_dirs.all_dirs_in_order() {
        let skills_dir = dir.join("skills");
        let source = source_for_dir(dir, &ext_dirs.global_dirs);
        let skill_dirs = scan_skills_dir(&skills_dir).await;
        for skill_dir in skill_dirs {
            if let Some(full) = load_skill_from_dir(&skill_dir, source.clone()).await {
                seen.insert(full.index.name.clone(), full.index);
            }
        }
    }
    let mut result: Vec<SkillIndex> = seen.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

pub async fn load_skill_full(ext_dirs: &ExtDirs, name: &str) -> Option<SkillFull> {
    let mut found: Option<SkillFull> = None;
    for dir in ext_dirs.all_dirs_in_order() {
        let skills_dir = dir.join("skills");
        let source = source_for_dir(dir, &ext_dirs.global_dirs);
        let candidate = skills_dir.join(name);
        if let Some(full) = load_skill_from_dir(&candidate, source).await {
            if full.index.name == name {
                found = Some(full);
            }
        }
    }
    found
}

pub async fn load_skill_linked_file(skill_dir: &Path, relative_path: &str) -> Option<String> {
    let relative_path = relative_path.trim();
    if Path::new(relative_path).is_absolute() {
        tracing::warn!("Rejecting absolute @include path: {}", relative_path);
        return None;
    }
    if relative_path.contains("..") {
        tracing::warn!("Rejecting @include with path traversal: {}", relative_path);
        return None;
    }
    let full_path = skill_dir.join(relative_path);
    let canonical = match tokio::fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to resolve @include path {:?}: {}", full_path, e);
            return None;
        }
    };
    let canonical_dir = match tokio::fs::canonicalize(skill_dir).await {
        Ok(p) => p,
        Err(_) => return None,
    };
    if !canonical.starts_with(&canonical_dir) {
        tracing::warn!("@include path escapes skill directory: {:?}", canonical);
        return None;
    }
    match tokio::fs::metadata(&canonical).await {
        Ok(meta) if meta.len() > 50 * 1024 => {
            tracing::warn!("@include file too large (>50KB): {:?}", canonical);
            return None;
        }
        Err(e) => {
            tracing::warn!("Failed to stat @include file {:?}: {}", canonical, e);
            return None;
        }
        _ => {}
    }
    match tokio::fs::read_to_string(&canonical).await {
        Ok(content) => Some(content),
        Err(e) => {
            tracing::warn!("Failed to read @include file {:?}: {}", canonical, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_load_skill_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("my_skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my_skill\ndescription: A useful skill\nargument-hint: \"<arg>\"\nallowed-tools:\n  - search\nmodel: gpt-4o\ncontext: fork\nuser-invocable: true\ndisable-model-invocation: false\n---\nDo something useful with $ARGUMENTS",
        )
        .await
        .unwrap();

        let source = CommandSource::GlobalRefact;
        let result = load_skill_from_dir(&skill_dir, source).await;
        assert!(result.is_some());
        let full = result.unwrap();
        assert_eq!(full.index.name, "my_skill");
        assert_eq!(full.index.description, "A useful skill");
        assert!(full.index.user_invocable);
        assert!(!full.index.disable_model_invocation);
        assert_eq!(full.argument_hint, "<arg>");
        assert_eq!(full.allowed_tools, vec!["search"]);
        assert_eq!(full.model, Some("gpt-4o".to_string()));
        assert_eq!(full.context, Some("fork".to_string()));
        assert_eq!(full.body, "Do something useful with $ARGUMENTS");
    }

    #[tokio::test]
    async fn test_load_skill_missing_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("not_a_skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        let result = load_skill_from_dir(&skill_dir, CommandSource::GlobalRefact).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_load_skill_missing_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("bad_skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: No name here\n---\nBody",
        )
        .await
        .unwrap();

        let result = load_skill_from_dir(&skill_dir, CommandSource::GlobalRefact).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_load_skill_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("no_desc");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: no_desc\n---\nBody",
        )
        .await
        .unwrap();

        let result = load_skill_from_dir(&skill_dir, CommandSource::GlobalRefact).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_load_skill_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("minimal");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: minimal\ndescription: Minimal skill\n---\nBody",
        )
        .await
        .unwrap();

        let full = load_skill_from_dir(&skill_dir, CommandSource::GlobalRefact).await.unwrap();
        assert!(full.index.user_invocable);
        assert!(!full.index.disable_model_invocation);
        assert!(full.model.is_none());
        assert!(full.context.is_none());
        assert!(full.agent.is_none());
        assert!(full.allowed_tools.is_empty());
    }

    #[tokio::test]
    async fn test_load_skill_indices_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        for skill_name in &["alpha", "beta", "gamma"] {
            let dir = skills_dir.join(skill_name);
            tokio::fs::create_dir_all(&dir).await.unwrap();
            tokio::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {}\ndescription: Skill {}\n---\nBody", skill_name, skill_name),
            )
            .await
            .unwrap();
        }

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            project_dirs: vec![],
        };
        let indices = load_skill_indices(&ext_dirs).await;
        assert_eq!(indices.len(), 3);
        assert_eq!(indices[0].name, "alpha");
        assert_eq!(indices[1].name, "beta");
        assert_eq!(indices[2].name, "gamma");
    }

    #[tokio::test]
    async fn test_load_skill_indices_precedence() {
        let global_tmp = tempfile::tempdir().unwrap();
        let project_tmp = tempfile::tempdir().unwrap();

        for (root, desc) in &[
            (global_tmp.path(), "Global version"),
            (project_tmp.path(), "Project version"),
        ] {
            let dir = root.join("skills").join("shared_skill");
            tokio::fs::create_dir_all(&dir).await.unwrap();
            tokio::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: shared_skill\ndescription: {}\n---\nBody", desc),
            )
            .await
            .unwrap();
        }

        let ext_dirs = ExtDirs {
            global_dirs: vec![global_tmp.path().to_path_buf()],
            project_dirs: vec![project_tmp.path().to_path_buf()],
        };
        let indices = load_skill_indices(&ext_dirs).await;
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0].description, "Project version");
    }

    #[tokio::test]
    async fn test_load_skill_full_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("finder");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: finder\ndescription: Find stuff\n---\nSearch for $ARGUMENTS",
        )
        .await
        .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            project_dirs: vec![],
        };
        let full = load_skill_full(&ext_dirs, "finder").await;
        assert!(full.is_some());
        assert_eq!(full.unwrap().index.name, "finder");
    }

    #[tokio::test]
    async fn test_load_skill_full_not_found() {
        let ext_dirs = ExtDirs {
            global_dirs: vec![PathBuf::from("/nonexistent")],
            project_dirs: vec![],
        };
        let full = load_skill_full(&ext_dirs, "nonexistent").await;
        assert!(full.is_none());
    }

    #[tokio::test]
    async fn test_load_skill_linked_file() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("context.md"), "Some linked content").await.unwrap();

        let result = load_skill_linked_file(tmp.path(), "context.md").await;
        assert_eq!(result, Some("Some linked content".to_string()));
    }

    #[tokio::test]
    async fn test_load_skill_linked_file_missing() {
        let result = load_skill_linked_file(Path::new("/nonexistent"), "missing.md").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_include_path_traversal_rejected() {
        let result = load_skill_linked_file(Path::new("/some/dir"), "../../secret.txt").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_include_absolute_path_rejected() {
        let result = load_skill_linked_file(Path::new("/some/dir"), "/etc/passwd").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_include_valid_path_works() {
        let tmp = tempfile::tempdir().unwrap();
        let templates_dir = tmp.path().join("templates");
        tokio::fs::create_dir_all(&templates_dir).await.unwrap();
        tokio::fs::write(templates_dir.join("foo.md"), "template content").await.unwrap();

        let result = load_skill_linked_file(tmp.path(), "templates/foo.md").await;
        assert_eq!(result, Some("template content".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_include_symlink_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skill_dir");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        let secret_file = tmp.path().join("secret.txt");
        tokio::fs::write(&secret_file, "secret content").await.unwrap();

        let symlink_path = skill_dir.join("link.md");
        std::os::unix::fs::symlink(&secret_file, &symlink_path).unwrap();

        let result = load_skill_linked_file(&skill_dir, "link.md").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_load_skill_no_skills_dir() {
        let ext_dirs = ExtDirs {
            global_dirs: vec![PathBuf::from("/nonexistent/path")],
            project_dirs: vec![],
        };
        let indices = load_skill_indices(&ext_dirs).await;
        assert!(indices.is_empty());
    }

    #[tokio::test]
    async fn test_load_skill_case_sensitive_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("case_test");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(skill_dir.join("skill.md"), "---\nname: case_test\ndescription: desc\n---\nBody").await.unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            project_dirs: vec![],
        };
        let indices = load_skill_indices(&ext_dirs).await;
        assert!(
            indices.is_empty(),
            "SKILL.md must be uppercase, skill.md should not be recognized"
        );
    }
}
