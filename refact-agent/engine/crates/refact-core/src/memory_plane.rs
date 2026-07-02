use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPlaneRootKind {
    Knowledge,
    Trajectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPlaneFileKind {
    KnowledgeMarkdown,
    TrajectoryJson,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPlaneRoots {
    pub project_roots: Vec<PathBuf>,
    pub global_knowledge_root: Option<PathBuf>,
    pub global_trajectories_root: Option<PathBuf>,
}

impl MemoryPlaneRoots {
    pub fn new(
        project_roots: Vec<PathBuf>,
        global_knowledge_root: Option<PathBuf>,
        global_trajectories_root: Option<PathBuf>,
    ) -> Self {
        Self {
            project_roots: dedupe_paths(project_roots),
            global_knowledge_root,
            global_trajectories_root,
        }
    }

    pub fn classify_root(&self, path: &Path) -> Option<MemoryPlaneRootKind> {
        for project_root in &self.project_roots {
            let Some(rel) = relative_components(path, project_root) else {
                continue;
            };
            if is_project_knowledge_path(&rel) {
                return Some(MemoryPlaneRootKind::Knowledge);
            }
            if is_project_trajectory_path(&rel) || is_project_task_trajectory_path(&rel) {
                return Some(MemoryPlaneRootKind::Trajectory);
            }
        }

        if self
            .global_knowledge_root
            .as_ref()
            .and_then(|root| relative_components(path, root))
            .is_some_and(|rel| !rel.is_empty())
        {
            return Some(MemoryPlaneRootKind::Knowledge);
        }

        if self
            .global_trajectories_root
            .as_ref()
            .and_then(|root| relative_components(path, root))
            .is_some_and(|rel| !rel.is_empty())
        {
            return Some(MemoryPlaneRootKind::Trajectory);
        }

        None
    }

    pub fn classify_file(&self, path: &Path) -> Option<MemoryPlaneFileKind> {
        match self.classify_root(path)? {
            MemoryPlaneRootKind::Knowledge if is_markdown_path(path) => {
                Some(MemoryPlaneFileKind::KnowledgeMarkdown)
            }
            MemoryPlaneRootKind::Trajectory if is_recognized_trajectory_json(path) => {
                Some(MemoryPlaneFileKind::TrajectoryJson)
            }
            _ => None,
        }
    }

    pub fn is_trajectory_file(&self, path: &Path) -> bool {
        self.classify_file(path) == Some(MemoryPlaneFileKind::TrajectoryJson)
    }
}

fn is_project_knowledge_path(rel: &[String]) -> bool {
    rel.len() > 2 && rel[0] == ".refact" && rel[1] == "knowledge"
}

fn is_project_trajectory_path(rel: &[String]) -> bool {
    rel.len() > 2 && rel[0] == ".refact" && rel[1] == "trajectories"
}

fn is_project_task_trajectory_path(rel: &[String]) -> bool {
    rel.len() > 4 && rel[0] == ".refact" && rel[1] == "tasks" && rel[3] == "trajectories"
}

fn is_markdown_path(path: &Path) -> bool {
    matches!(extension_lower(path).as_deref(), Some("md" | "mdx"))
}

fn is_recognized_trajectory_json(path: &Path) -> bool {
    if extension_lower(path).as_deref() != Some("json") {
        return false;
    }
    let Some(name) = path_components(path).last().cloned() else {
        return false;
    };
    !name.starts_with('.') && name != "index.json"
}

fn extension_lower(path: &Path) -> Option<String> {
    let name = path_components(path).last().cloned()?;
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn relative_components(path: &Path, root: &Path) -> Option<Vec<String>> {
    let path_parts = path_components(path);
    let root_parts = path_components(root);
    if path_parts.iter().any(|part| part == "..")
        || root_parts.iter().any(|part| part == "..")
        || root_parts.is_empty()
        || path_parts.len() <= root_parts.len()
        || !path_parts
            .iter()
            .zip(root_parts.iter())
            .all(|(path, root)| path == root)
    {
        return None;
    }
    Some(path_parts[root_parts.len()..].to_vec())
}

fn path_components(path: &Path) -> Vec<String> {
    path.to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| part.to_string())
        .collect()
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        if seen.insert(path_components(&path).join("/")) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> MemoryPlaneRoots {
        MemoryPlaneRoots::new(
            vec![PathBuf::from("/workspace/project")],
            Some(PathBuf::from("/home/user/.config/refact/knowledge")),
            Some(PathBuf::from("/home/user/.config/refact/trajectories")),
        )
    }

    #[test]
    fn classifies_allowed_memory_plane_roots() {
        let roots = roots();

        assert_eq!(
            roots.classify_root(Path::new("/workspace/project/.refact/knowledge/note.md")),
            Some(MemoryPlaneRootKind::Knowledge)
        );
        assert_eq!(
            roots.classify_root(Path::new(
                "/workspace/project/.refact/trajectories/chat.json"
            )),
            Some(MemoryPlaneRootKind::Trajectory)
        );
        assert_eq!(
            roots.classify_root(Path::new(
                "/workspace/project/.refact/tasks/task-1/trajectories/agents/chat.json"
            )),
            Some(MemoryPlaneRootKind::Trajectory)
        );
        assert_eq!(
            roots.classify_root(Path::new("/home/user/.config/refact/knowledge/global.md")),
            Some(MemoryPlaneRootKind::Knowledge)
        );
        assert_eq!(
            roots.classify_root(Path::new(
                "/home/user/.config/refact/trajectories/global.json"
            )),
            Some(MemoryPlaneRootKind::Trajectory)
        );
    }

    #[test]
    fn rejects_task_memories_and_non_refact_task_paths() {
        let roots = roots();

        assert_eq!(
            roots.classify_root(Path::new(
                "/workspace/project/.refact/tasks/task-1/memories/note.md"
            )),
            None
        );
        assert_eq!(
            roots.classify_root(Path::new(
                "/workspace/project/src/tasks/task-1/trajectories/chat.json"
            )),
            None
        );
        assert_eq!(
            roots.classify_root(Path::new(
                "/workspace/project/.refact/tasks/task-1/meta.yaml"
            )),
            None
        );
    }

    #[test]
    fn classifies_only_embeddable_memory_plane_files() {
        let roots = roots();

        assert_eq!(
            roots.classify_file(Path::new("/workspace/project/.refact/knowledge/note.md")),
            Some(MemoryPlaneFileKind::KnowledgeMarkdown)
        );
        assert_eq!(
            roots.classify_file(Path::new("/workspace/project/.refact/knowledge/source.rs")),
            None
        );
        assert_eq!(
            roots.classify_file(Path::new(
                "/workspace/project/.refact/trajectories/chat.json"
            )),
            Some(MemoryPlaneFileKind::TrajectoryJson)
        );
        assert_eq!(
            roots.classify_file(Path::new(
                "/workspace/project/.refact/trajectories/index.json"
            )),
            None
        );
    }
}
