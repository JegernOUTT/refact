use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::files_correction::get_project_dirs;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};

pub use refact_core::knowledge_index::{
    KnowledgeCard, KnowledgeIndex, KnowledgeSearchFilters, KnowledgeSearchHit,
};

fn path_has_any_relative_component(path: &Path, root: &Path, components: &[&str]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    path_components_match(relative, components)
}

fn path_components_match(path: &Path, components: &[&str]) -> bool {
    path.components().any(|c| {
        let candidate = c.as_os_str().to_string_lossy();
        components.iter().any(|component| candidate == *component)
    })
}

fn is_tmp_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name.ends_with(".tmp") || file_name.contains(".tmp-")
}

fn push_unique(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn first_nonempty_line(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return Some(trimmed.trim_start_matches('#').trim().to_string());
    }
    None
}

fn yaml_value_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn yaml_string(mapping: &YamlMapping, key: &str) -> Option<String> {
    mapping
        .get(&YamlValue::String(key.to_string()))
        .and_then(yaml_value_string)
}

fn yaml_string_list(mapping: &YamlMapping, key: &str) -> Vec<String> {
    let Some(value) = mapping.get(&YamlValue::String(key.to_string())) else {
        return Vec::new();
    };
    match value {
        YamlValue::Sequence(values) => values.iter().filter_map(yaml_value_string).collect(),
        YamlValue::String(value) => value
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_yaml_frontmatter(text: &str) -> (YamlMapping, usize) {
    if !text.starts_with("---") {
        return (YamlMapping::new(), 0);
    }
    let rest = &text[3..];
    let Some(end_idx) = rest.find("\n---") else {
        return (YamlMapping::new(), 0);
    };
    let yaml_content = &rest[..end_idx];
    let mut end_offset = 3 + end_idx + 4;
    if text.len() > end_offset && text.as_bytes().get(end_offset) == Some(&b'\n') {
        end_offset += 1;
    }
    let mapping = match serde_yaml::from_str::<YamlValue>(yaml_content) {
        Ok(YamlValue::Mapping(mapping)) => mapping,
        _ => YamlMapping::new(),
    };
    (mapping, end_offset)
}

fn mapping_is_inactive(mapping: &YamlMapping) -> bool {
    matches!(
        yaml_string(mapping, "status")
            .unwrap_or_else(|| "active".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "archived" | "deprecated" | "superseded"
    )
}

fn component_matches(component: Component<'_>, expected: &str) -> bool {
    matches!(component, Component::Normal(value) if value.to_str() == Some(expected))
}

fn extract_task_id_from_path(path: &Path) -> Option<String> {
    let components = path.components().collect::<Vec<_>>();
    for window in components.windows(3) {
        if component_matches(window[0], ".refact") && component_matches(window[1], "tasks") {
            if let Component::Normal(task_id) = window[2] {
                return Some(task_id.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn task_card_tags(mapping: &YamlMapping) -> Vec<String> {
    let mut tags = Vec::new();
    for field in ["card_id", "relevant_cards"] {
        for value in yaml_string_list(mapping, field) {
            push_unique(&mut tags, format!("scope:card:{}", value));
        }
    }
    if let Some(value) = yaml_string(mapping, "card_id") {
        push_unique(&mut tags, format!("scope:card:{}", value));
    }
    tags
}

fn task_card_from_mapping(
    mapping: &YamlMapping,
    path: &Path,
    directory_kind: &str,
    body: &str,
) -> KnowledgeCard {
    let task_id = yaml_string(mapping, "task_id").or_else(|| extract_task_id_from_path(path));
    let title = yaml_string(mapping, "title")
        .or_else(|| yaml_string(mapping, "name"))
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    let kind = yaml_string(mapping, "kind").or_else(|| {
        if directory_kind == "memories" {
            Some("freeform".to_string())
        } else {
            None
        }
    });
    let namespace = yaml_string(mapping, "namespace").unwrap_or_else(|| "task".to_string());
    let mut tags = yaml_string_list(mapping, "tags");
    push_unique(&mut tags, "scope:task");
    push_unique(&mut tags, format!("type:{}", directory_kind));
    push_unique(&mut tags, format!("namespace:{}", namespace));
    if let Some(task_id) = &task_id {
        push_unique(&mut tags, format!("scope:task:{}", task_id));
    }
    if let Some(kind) = &kind {
        push_unique(&mut tags, format!("kind:{}", kind));
    }
    for tag in task_card_tags(mapping) {
        push_unique(&mut tags, tag);
    }

    let mut filenames = Vec::new();
    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
        filenames.push(file_name.to_string());
    }
    if let Some(stem) = path.file_stem().and_then(|name| name.to_str()) {
        push_unique(&mut filenames, stem.to_string());
    }
    if let Some(slug) = yaml_string(mapping, "slug") {
        push_unique(&mut filenames, slug);
    }

    KnowledgeCard {
        id: path.to_string_lossy().to_string(),
        title,
        summary: first_nonempty_line(body),
        description: None,
        tags,
        filenames,
        entities: Vec::new(),
        related_files: Vec::new(),
        related_entities: Vec::new(),
        kind,
        created: None,
        created_at: yaml_string(mapping, "created_at"),
        updated: yaml_string(mapping, "updated_at"),
        file_path: path.to_path_buf(),
    }
}

pub fn format_related_memories_section(
    cards: &[KnowledgeCard],
    exclude_path: Option<&Path>,
) -> String {
    let mut shown = Vec::new();
    for c in cards {
        if let Some(ex) = exclude_path {
            if c.file_path == ex {
                continue;
            }
        }
        let mut line = format!("- {} ({})", c.title, c.file_path.display());
        let desc = c
            .description
            .as_deref()
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .or_else(|| {
                c.summary
                    .as_deref()
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .map(|x| x.to_string())
            });
        if let Some(d) = desc {
            line.push_str(&format!("\n  {}", d));
        }
        shown.push(line);
        if shown.len() >= 5 {
            break;
        }
    }
    if shown.is_empty() {
        return String::new();
    }
    format!(
        "\n\n## Related memories (short form)\n\n{}\n\nNote: these are heuristic matches and may be unrelated. To load full content of any memory above, call `cat(paths=\"<path>\")` using the memory file path shown above.",
        shown.join("\n")
    )
}

pub async fn build_knowledge_index(gcx: Arc<GlobalContext>) -> KnowledgeIndex {
    let mut index = KnowledgeIndex::empty();

    let project_dirs = get_project_dirs(gcx.clone()).await;

    // Local + global knowledge dirs.
    let mut knowledge_dirs: Vec<PathBuf> = project_dirs
        .iter()
        .map(|d| d.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|d| d.exists())
        .collect();

    // Global knowledge dir lives under the config dir.
    // This keeps KG/index behavior aligned with memories_search().
    let global_dir = gcx.config_dir.join("knowledge");
    if global_dir.exists() {
        knowledge_dirs.push(global_dir);
    }

    scan_knowledge_dirs(&mut index, knowledge_dirs).await;

    let task_dirs = crate::tasks::storage::get_all_tasks_dirs(gcx).await;
    scan_task_dirs(&mut index, task_dirs).await;

    index
}

async fn scan_knowledge_dirs(index: &mut KnowledgeIndex, knowledge_dirs: Vec<PathBuf>) {
    for path_buf in collect_knowledge_markdown_paths(knowledge_dirs).await {
        let text = match tokio::fs::read_to_string(&path_buf).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (fm, content_start) = KnowledgeFrontmatter::parse(&text);
        if fm.is_archived() || fm.is_deprecated() {
            continue;
        }

        let content_slice = text.get(content_start..).unwrap_or("");
        index.add_signature(
            refact_buddy_core::memory_dedup::content_signature(content_slice),
            path_buf.clone(),
        );
        index.add_from_frontmatter(path_buf, &fm, Some(content_slice));
    }
}

async fn collect_knowledge_markdown_paths(knowledge_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    tokio::task::spawn_blocking(move || collect_knowledge_markdown_paths_blocking(knowledge_dirs))
        .await
        .unwrap_or_default()
}

fn collect_knowledge_markdown_paths_blocking(knowledge_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in knowledge_dirs {
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if should_index_markdown_path(path, &dir, &["archive", "archived", ".history"]) {
                paths.push(path.to_path_buf());
            }
        }
    }
    paths
}

fn should_index_markdown_path(path: &Path, root: &Path, ignored_components: &[&str]) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "md" && ext != "mdx" {
        return false;
    }
    !path_has_any_relative_component(path, root, ignored_components) && !is_tmp_path(path)
}

#[derive(Debug, Clone)]
struct TaskMarkdownPath {
    path: PathBuf,
    directory_kind: &'static str,
}

async fn collect_task_markdown_paths(task_roots: Vec<PathBuf>) -> Vec<TaskMarkdownPath> {
    tokio::task::spawn_blocking(move || collect_task_markdown_paths_blocking(task_roots))
        .await
        .unwrap_or_default()
}

fn collect_task_markdown_paths_blocking(task_roots: Vec<PathBuf>) -> Vec<TaskMarkdownPath> {
    let mut paths = Vec::new();
    for tasks_dir in task_roots {
        let task_entries = match std::fs::read_dir(&tasks_dir) {
            Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        for task_entry in task_entries {
            let task_dir = task_entry.path();
            if !task_dir.is_dir() {
                continue;
            }
            for subdir in ["memories", "documents"] {
                let scan_dir = task_dir.join(subdir);
                if !scan_dir.exists() {
                    continue;
                }
                for entry in walkdir::WalkDir::new(&scan_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let path = entry.path();
                    if should_index_markdown_path(
                        path,
                        &scan_dir,
                        &[".history", "archived", "archive"],
                    ) {
                        paths.push(TaskMarkdownPath {
                            path: path.to_path_buf(),
                            directory_kind: subdir,
                        });
                    }
                }
            }
        }
    }
    paths
}

async fn scan_task_dirs(index: &mut KnowledgeIndex, task_roots: Vec<PathBuf>) {
    for task_path in collect_task_markdown_paths(task_roots).await {
        let text = match tokio::fs::read_to_string(&task_path.path).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (mapping, content_start) = parse_yaml_frontmatter(&text);
        if mapping_is_inactive(&mapping) {
            continue;
        }
        let content_slice = text.get(content_start..).unwrap_or("");
        let card = task_card_from_mapping(
            &mapping,
            &task_path.path,
            task_path.directory_kind,
            content_slice,
        );
        index.add_card_with_content(card, Some(content_slice));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_task_id_rejects_non_refact_tasks_path() {
        assert_eq!(
            extract_task_id_from_path(Path::new("/repo/tasks/examples/x.md")),
            None
        );
    }

    #[test]
    fn extract_task_id_accepts_refact_tasks_path() {
        assert_eq!(
            extract_task_id_from_path(Path::new("/workspace/.refact/tasks/T-1/memories/x.md")),
            Some("T-1".to_string())
        );
    }

    #[tokio::test]
    async fn build_index_skips_archived_and_deprecated_memories() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();

        let archived_path = knowledge_dir.join("archived.md");
        let deprecated_path = knowledge_dir.join("deprecated.md");
        let active_path = knowledge_dir.join("active.md");

        tokio::fs::write(
            &archived_path,
            "---\nstatus: archived\ntags: [old]\n---\n\nArchived memory",
        )
        .await
        .unwrap();
        tokio::fs::write(
            &deprecated_path,
            "---\nstatus: deprecated\ntags: [old]\n---\n\nDeprecated memory",
        )
        .await
        .unwrap();
        tokio::fs::write(
            &active_path,
            "---\nstatus: active\ntags: [new]\n---\n\nActive memory",
        )
        .await
        .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let index = build_knowledge_index(gcx).await;

        assert!(archived_path.exists());
        assert!(deprecated_path.exists());
        assert!(active_path.exists());
        assert_eq!(index.related_for_tags(&vec!["new".to_string()], 5).len(), 1);
    }

    #[tokio::test]
    async fn build_index_picks_up_task_memories_and_documents() {
        let dir = tempfile::tempdir().unwrap();
        let task_dir = dir.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join("memories");
        let documents_dir = task_dir.join("documents");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::create_dir_all(&documents_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("decision.md"),
            "---\ntitle: Routing\ntask_id: task-1\nkind: decision\nnamespace: card:T-22\ntags: [routing]\n---\n\nUse text search for task memory.",
        )
        .await
        .unwrap();
        tokio::fs::write(
            documents_dir.join("spec.md"),
            "---\nname: Main Spec\nslug: main-spec\nkind: spec\ncreated_at: now\nupdated_at: now\nauthor_role: planner\npinned: true\nversion: 1\nrelevant_cards: [T-22]\n---\n\nDocument body token.",
        )
        .await
        .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let index = build_knowledge_index(gcx).await;
        let filters = KnowledgeSearchFilters {
            scope: Some("task".to_string()),
            task_id: Some("task-1".to_string()),
            ..Default::default()
        };

        assert_eq!(index.search("routing", &filters, 10).len(), 1);
        assert_eq!(index.search("main-spec", &filters, 10).len(), 1);
        assert_eq!(index.search("body", &filters, 10).len(), 1);
    }

    #[tokio::test]
    async fn existing_knowledge_still_found_after_task_extension() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        tokio::fs::write(
            knowledge_dir.join("active.md"),
            "---\ntitle: Knowledge Card\ntags: [stable]\n---\n\nEvergreen content",
        )
        .await
        .unwrap();
        tokio::fs::create_dir_all(dir.path().join(".refact/tasks/task-1/memories"))
            .await
            .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let index = build_knowledge_index(gcx).await;
        assert_eq!(
            index.related_for_tags(&vec!["stable".to_string()], 5).len(),
            1
        );
        assert_eq!(
            index
                .search("evergreen", &KnowledgeSearchFilters::default(), 5)
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn build_knowledge_index_uses_spawn_blocking_for_walk() {
        let dir = tempfile::tempdir().unwrap();
        let memories_dir = dir.path().join(".refact/tasks/T-1/memories");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        for idx in 0..200 {
            tokio::fs::write(
                memories_dir.join(format!("memory-{idx}.md")),
                format!(
                    "---\ntitle: Memory {idx}\nkind: finding\n---\n\nlarge synthetic tree token {idx}"
                ),
            )
            .await
            .unwrap();
        }

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let index = build_knowledge_index(gcx).await;
        let hits = index.search(
            "synthetic",
            &KnowledgeSearchFilters {
                scope: Some("task".to_string()),
                task_id: Some("T-1".to_string()),
                ..Default::default()
            },
            250,
        );

        assert_eq!(hits.len(), 200);
    }

    #[tokio::test]
    async fn task_index_excludes_archived_superseded_history_and_tmp_files() {
        let dir = tempfile::tempdir().unwrap();
        let memories_dir = dir.path().join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(memories_dir.join(".history"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(memories_dir.join("archived"))
            .await
            .unwrap();
        tokio::fs::write(
            memories_dir.join("active.md"),
            "---\ntitle: Active\ntask_id: task-1\nkind: finding\n---\n\nneedle active",
        )
        .await
        .unwrap();
        tokio::fs::write(
            memories_dir.join("superseded.md"),
            "---\nstatus: superseded\ntags: [needle]\n---\n\nneedle superseded",
        )
        .await
        .unwrap();
        tokio::fs::write(memories_dir.join("archived/old.md"), "needle archived")
            .await
            .unwrap();
        tokio::fs::write(memories_dir.join(".history/old.md"), "needle history")
            .await
            .unwrap();
        tokio::fs::write(memories_dir.join("draft.md.tmp"), "needle tmp")
            .await
            .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let index = build_knowledge_index(gcx).await;
        let filters = KnowledgeSearchFilters {
            scope: Some("task".to_string()),
            task_id: Some("task-1".to_string()),
            ..Default::default()
        };

        let hits = index.search("needle", &filters, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card.title, "Active");
    }

    #[tokio::test]
    async fn knowledge_index_skips_archived_dir_memories() {
        let dir = tempfile::tempdir().unwrap();
        let memories_dir = dir.path().join(".refact/tasks/T-1/memories");
        let active_path = memories_dir.join("active.md");
        let archived_path = memories_dir.join("archived/old.md");
        tokio::fs::create_dir_all(memories_dir.join("archived"))
            .await
            .unwrap();
        tokio::fs::write(
            &active_path,
            "---\ntitle: Active\ntask_id: T-1\nkind: finding\n---\n\nactive body",
        )
        .await
        .unwrap();
        tokio::fs::write(
            &archived_path,
            "---\ntitle: Old\ntask_id: T-1\nkind: finding\n---\n\nold body",
        )
        .await
        .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let indexed_paths = build_knowledge_index(gcx)
            .await
            .all_cards()
            .into_iter()
            .map(|card| card.file_path)
            .collect::<Vec<_>>();

        let indexed_paths = indexed_paths
            .into_iter()
            .map(crate::files_correction::canonicalize_normalized_path)
            .collect::<Vec<_>>();
        assert!(
            indexed_paths.contains(&crate::files_correction::canonicalize_normalized_path(
                active_path
            ))
        );
        assert!(
            !indexed_paths.contains(&crate::files_correction::canonicalize_normalized_path(
                archived_path
            ))
        );
    }

    #[tokio::test]
    async fn knowledge_index_skips_status_archived_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let memories_dir = dir.path().join(".refact/tasks/T-1/memories");
        let active_path = memories_dir.join("active.md");
        let old_path = memories_dir.join("old.md");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            &active_path,
            "---\ntitle: Active\ntask_id: T-1\nkind: finding\n---\n\nactive body",
        )
        .await
        .unwrap();
        tokio::fs::write(
            &old_path,
            "---\ntitle: Old\ntask_id: T-1\nkind: finding\nstatus: archived\n---\n\nold body",
        )
        .await
        .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let indexed_paths = build_knowledge_index(gcx)
            .await
            .all_cards()
            .into_iter()
            .map(|card| card.file_path)
            .collect::<Vec<_>>();

        let indexed_paths = indexed_paths
            .into_iter()
            .map(crate::files_correction::canonicalize_normalized_path)
            .collect::<Vec<_>>();
        assert!(
            indexed_paths.contains(&crate::files_correction::canonicalize_normalized_path(
                active_path
            ))
        );
        assert!(
            !indexed_paths.contains(&crate::files_correction::canonicalize_normalized_path(
                old_path
            ))
        );
    }

    #[tokio::test]
    async fn indexes_workspace_under_absolute_archive_parent() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("archive/project");
        let knowledge_dir = workspace.join(KNOWLEDGE_FOLDER_NAME);
        let memories_dir = workspace.join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            knowledge_dir.join("knowledge.md"),
            "---\ntitle: Absolute Archive Knowledge\ntags: [absolute-archive]\n---\n\nknowledge needle",
        )
        .await
        .unwrap();
        tokio::fs::write(
            memories_dir.join("memory.md"),
            "---\ntitle: Absolute Archive Memory\ntask_id: task-1\nkind: finding\n---\n\nmemory needle",
        )
        .await
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace];

        let index = build_knowledge_index(gcx).await;

        assert_eq!(
            index
                .search("knowledge", &KnowledgeSearchFilters::default(), 10)
                .len(),
            1
        );
        assert_eq!(
            index
                .search(
                    "memory",
                    &KnowledgeSearchFilters {
                        scope: Some("task".to_string()),
                        task_id: Some("task-1".to_string()),
                        ..Default::default()
                    },
                    10,
                )
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn task_index_skips_relative_archive_and_history_document_roots() {
        let dir = tempfile::tempdir().unwrap();
        let task_dir = dir.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join("memories");
        let documents_dir = task_dir.join("documents");
        tokio::fs::create_dir_all(memories_dir.join("archived"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(documents_dir.join(".history"))
            .await
            .unwrap();
        tokio::fs::write(
            memories_dir.join("active.md"),
            "---\ntitle: Active\ntask_id: task-1\nkind: finding\n---\n\nneedle active",
        )
        .await
        .unwrap();
        tokio::fs::write(
            memories_dir.join("archived/old.md"),
            "---\ntitle: Archived\ntask_id: task-1\nkind: finding\n---\n\nneedle archived",
        )
        .await
        .unwrap();
        tokio::fs::write(
            documents_dir.join(".history/old.md"),
            "---\ntitle: History\nkind: spec\n---\n\nneedle history",
        )
        .await
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];

        let index = build_knowledge_index(gcx).await;
        let hits = index.search(
            "needle",
            &KnowledgeSearchFilters {
                scope: Some("task".to_string()),
                task_id: Some("task-1".to_string()),
                ..Default::default()
            },
            10,
        );

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card.title, "Active");
    }
}
