use std::collections::HashSet;
use std::path::{Path, PathBuf};

use refact_core::chat_types::{ChatContent, ChatMessage, ContextEnum};
use refact_worktrees::scope::{ExecutionScope, ScopedPath};

#[derive(Debug, Clone)]
pub struct ScopedFiles {
    pub files: Vec<String>,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScopedScopeFilter {
    pub filter: Option<String>,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScopedResolvedPath {
    pub path: PathBuf,
    pub notices: Vec<String>,
    pub outside_absolute_path: bool,
}

pub fn path_with_sep(path: &Path) -> String {
    let path = path.to_string_lossy().to_string();
    if path.ends_with(std::path::MAIN_SEPARATOR) {
        path
    } else {
        format!("{}{}", path, std::path::MAIN_SEPARATOR)
    }
}

pub fn dedup_notices(notices: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    notices
        .into_iter()
        .filter(|notice| seen.insert(notice.clone()))
        .collect()
}

pub fn format_scope_notices(notices: &[String]) -> String {
    let notices = dedup_notices(notices.to_vec());
    if notices.is_empty() {
        String::new()
    } else {
        format!("Worktree scope notices:\n{}\n\n", notices.join("\n"))
    }
}

pub fn scoped_path_notices(scoped: &ScopedPath) -> Vec<String> {
    if let Some(source) = &scoped.remapped_from {
        vec![format!(
            "⚠️ Absolute source path was mapped to active worktree: {} -> {}",
            source.display(),
            scoped.path.display()
        )]
    } else if scoped.outside_absolute_path {
        vec![format!(
            "⚠️ STRONG NOTICE: absolute path is outside active worktree; content comes from outside active worktree: {}",
            scoped.path.display()
        )]
    } else {
        vec![]
    }
}

pub fn scoped_path_warnings(scoped: &ScopedPath, scope: &ExecutionScope) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(source_path) = &scoped.remapped_from {
        warnings.push(format!(
            "⚠️ Worktree scope: absolute source path '{}' was mapped to active worktree: '{}' -> '{}'",
            source_path.display(),
            source_path.display(),
            scoped.path.display()
        ));
    } else if scoped.outside_absolute_path {
        warnings.push(format!(
            "⚠️ Worktree scope: strong warning: operation targeted privacy-permitted absolute path outside active worktree '{}': '{}'",
            scope.effective_root().display(),
            scoped.path.display()
        ));
    }
    warnings
}

pub fn append_scope_warnings(summary: String, warnings: &[String]) -> String {
    if warnings.is_empty() {
        summary
    } else {
        format!("{}\n{}", warnings.join("\n"), summary)
    }
}

pub fn scope_warnings_to_tool_message(summary: &str, tool_call_id: &str) -> Option<ContextEnum> {
    let warnings = summary
        .lines()
        .filter(|line| line.contains("Worktree scope:"))
        .collect::<Vec<_>>();
    if warnings.is_empty() {
        None
    } else {
        Some(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(warnings.join("\n")),
            tool_calls: None,
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_worktrees::types::WorktreeMeta;
    use std::fs;

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        source: PathBuf,
        outside: PathBuf,
        scope: ExecutionScope,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = fs::canonicalize(source).unwrap();
        let outside = fs::canonicalize(outside).unwrap();
        let scope = ExecutionScope::from_worktree(&WorktreeMeta {
            id: "wt".to_string(),
            kind: "task_agent".to_string(),
            root: root.clone(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        });
        Fixture {
            _temp: temp,
            root,
            source,
            outside,
            scope,
        }
    }

    fn scoped_path(
        path: PathBuf,
        used_absolute_path: bool,
        remapped_from: Option<PathBuf>,
        outside_absolute_path: bool,
    ) -> ScopedPath {
        ScopedPath {
            raw: path.clone(),
            path,
            used_absolute_path,
            remapped_from,
            outside_absolute_path,
            privacy_check_required: outside_absolute_path,
        }
    }

    #[test]
    fn dedup_notices_preserves_first_occurrence_order() {
        let notices = vec![
            "one".to_string(),
            "two".to_string(),
            "one".to_string(),
            "three".to_string(),
            "two".to_string(),
        ];

        assert_eq!(
            dedup_notices(notices),
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
    }

    #[test]
    fn scoped_path_notices_preserve_exact_messages() {
        let f = fixture();
        let source = f.source.join("src/lib.rs");
        let worktree = f.root.join("src/lib.rs");
        let outside = f.outside.join("outside.txt");

        assert_eq!(
            scoped_path_notices(&scoped_path(
                worktree.clone(),
                true,
                Some(source.clone()),
                false,
            )),
            vec![format!(
                "⚠️ Absolute source path was mapped to active worktree: {} -> {}",
                source.display(),
                worktree.display()
            )]
        );
        assert_eq!(
            scoped_path_notices(&scoped_path(outside.clone(), true, None, true)),
            vec![format!(
                "⚠️ STRONG NOTICE: absolute path is outside active worktree; content comes from outside active worktree: {}",
                outside.display()
            )]
        );
        assert!(scoped_path_notices(&scoped_path(worktree.clone(), true, None, false)).is_empty());
        assert!(scoped_path_notices(&scoped_path(worktree, false, None, false)).is_empty());
    }

    #[test]
    fn scoped_path_warnings_preserve_exact_messages() {
        let f = fixture();
        let source = f.source.join("src/lib.rs");
        let worktree = f.root.join("src/lib.rs");
        let outside = f.outside.join("outside.txt");

        assert_eq!(
            scoped_path_warnings(
                &scoped_path(worktree.clone(), true, Some(source.clone()), false),
                &f.scope,
            ),
            vec![format!(
                "⚠️ Worktree scope: absolute source path '{}' was mapped to active worktree: '{}' -> '{}'",
                source.display(),
                source.display(),
                worktree.display()
            )]
        );
        assert_eq!(
            scoped_path_warnings(&scoped_path(outside.clone(), true, None, true), &f.scope),
            vec![format!(
                "⚠️ Worktree scope: strong warning: operation targeted privacy-permitted absolute path outside active worktree '{}': '{}'",
                f.scope.effective_root().display(),
                outside.display()
            )]
        );
        assert!(
            scoped_path_warnings(&scoped_path(worktree.clone(), true, None, false), &f.scope)
                .is_empty()
        );
        assert!(
            scoped_path_warnings(&scoped_path(worktree, false, None, false), &f.scope).is_empty()
        );
    }
}
