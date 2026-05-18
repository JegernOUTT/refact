use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;


use crate::files_correction::get_project_dirs;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;

#[derive(Debug, Clone)]
pub struct KnowledgeCard {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub filenames: Vec<String>,
    pub entities: Vec<String>,
    pub related_files: Vec<String>,
    pub related_entities: Vec<String>,
    pub kind: Option<String>,
    pub created: Option<String>,
    pub created_at: Option<String>,
    pub file_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct KnowledgeIndex {
    by_filename: HashMap<String, Vec<KnowledgeCard>>,
    by_tag: HashMap<String, Vec<KnowledgeCard>>,
    by_entity: HashMap<String, Vec<KnowledgeCard>>,
    by_related_filename: HashMap<String, Vec<KnowledgeCard>>,
    by_related_entity: HashMap<String, Vec<KnowledgeCard>>,
}

fn path_has_component(path: &Path, component: &str) -> bool {
    path.components().any(|c| c.as_os_str() == component)
}

fn normalize_key(s: &str) -> String {
    s.trim().to_lowercase()
}

fn kind_priority(kind: Option<&str>) -> i32 {
    // Higher = better.
    // Keep conservative: we mainly want durable preference/lesson/memory cards to outrank raw reports.
    match kind.unwrap_or("") {
        "preference" => 120,
        "memory" => 110,
        "lesson" => 105,
        "pattern" => 100,
        "insight" => 95,
        "decision" => 90,
        "process" => 80,
        "task-report" => 70,
        "research" => 60,
        "trajectory" => 20,
        _ => 50,
    }
}

fn recency_key(created_at: Option<&str>, created: Option<&str>) -> String {
    // Lexicographic sort works for RFC3339 and YYYY-MM-DD.
    created_at.or(created).unwrap_or("").to_string()
}

fn rank_cards(mut cards: Vec<KnowledgeCard>, max_items: usize) -> Vec<KnowledgeCard> {
    cards.sort_by(|a, b| {
        let ak = kind_priority(a.kind.as_deref());
        let bk = kind_priority(b.kind.as_deref());
        bk.cmp(&ak)
            .then_with(|| {
                let ar = recency_key(a.created_at.as_deref(), a.created.as_deref());
                let br = recency_key(b.created_at.as_deref(), b.created.as_deref());
                br.cmp(&ar)
            })
            .then_with(|| a.title.cmp(&b.title))
    });
    cards.truncate(max_items);
    cards
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

fn retain_cards_not_at_path(index: &mut HashMap<String, Vec<KnowledgeCard>>, file_path: &Path) {
    index.retain(|_, cards| {
        cards.retain(|card| card.file_path != file_path);
        !cards.is_empty()
    });
}

impl KnowledgeIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn remove_path(&mut self, file_path: &Path) {
        retain_cards_not_at_path(&mut self.by_filename, file_path);
        retain_cards_not_at_path(&mut self.by_tag, file_path);
        retain_cards_not_at_path(&mut self.by_entity, file_path);
        retain_cards_not_at_path(&mut self.by_related_filename, file_path);
        retain_cards_not_at_path(&mut self.by_related_entity, file_path);
    }

    pub fn is_empty(&self) -> bool {
        self.by_filename.is_empty()
            && self.by_tag.is_empty()
            && self.by_entity.is_empty()
            && self.by_related_filename.is_empty()
            && self.by_related_entity.is_empty()
    }

    pub fn all_cards(&self) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for cards in [
            &self.by_filename,
            &self.by_tag,
            &self.by_entity,
            &self.by_related_filename,
            &self.by_related_entity,
        ] {
            for card in cards.values().flatten() {
                if seen.insert(card.file_path.clone()) {
                    out.push(card.clone());
                }
            }
        }
        out
    }

    pub fn add_card(&mut self, card: KnowledgeCard) {
        for filename in &card.filenames {
            self.by_filename
                .entry(filename.clone())
                .or_default()
                .push(card.clone());
        }
        for tag in &card.tags {
            self.by_tag
                .entry(normalize_key(tag))
                .or_default()
                .push(card.clone());
        }
        for ent in &card.entities {
            self.by_entity
                .entry(ent.clone())
                .or_default()
                .push(card.clone());
        }

        for filename in &card.related_files {
            self.by_related_filename
                .entry(filename.clone())
                .or_default()
                .push(card.clone());
        }
        for ent in &card.related_entities {
            self.by_related_entity
                .entry(ent.clone())
                .or_default()
                .push(card.clone());
        }
    }

    pub fn add_from_frontmatter(
        &mut self,
        file_path: PathBuf,
        fm: &KnowledgeFrontmatter,
        content: Option<&str>,
    ) {
        if fm.is_archived() || fm.is_deprecated() {
            return;
        }
        let id = fm
            .id
            .clone()
            .unwrap_or_else(|| file_path.to_string_lossy().to_string());
        let title = fm.title.clone().unwrap_or_else(|| {
            file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
        let summary = fm
            .summary
            .clone()
            .or_else(|| content.and_then(first_nonempty_line));

        let description = fm.description.clone();

        self.add_card(KnowledgeCard {
            id,
            title,
            summary,
            description,
            tags: fm.tags.clone(),
            filenames: fm.filenames.clone(),
            entities: fm.entities.clone(),
            related_files: fm.related_files.clone(),
            related_entities: fm.related_entities.clone(),
            kind: fm.kind.clone(),
            created: fm.created.clone(),
            created_at: fm.created_at.clone(),
            file_path,
        });
    }

    pub fn related_for_files(&self, filenames: &[String], max_items: usize) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<String>::new();
        let mut out = Vec::new();
        for f in filenames {
            if let Some(cards) = self.by_filename.get(f) {
                for c in cards {
                    if seen.insert(c.id.clone()) {
                        out.push(c.clone());
                    }
                }
            }
        }
        rank_cards(out, max_items)
    }

    pub fn related_for_related_files(
        &self,
        filenames: &[String],
        max_items: usize,
    ) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<String>::new();
        let mut out = Vec::new();
        for f in filenames {
            if let Some(cards) = self.by_related_filename.get(f) {
                for c in cards {
                    if seen.insert(c.id.clone()) {
                        out.push(c.clone());
                    }
                }
            }
        }
        rank_cards(out, max_items)
    }

    pub fn related_for_entities(
        &self,
        entities: &[String],
        max_items: usize,
    ) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<String>::new();
        let mut out = Vec::new();
        for e in entities {
            if let Some(cards) = self.by_entity.get(e) {
                for c in cards {
                    if seen.insert(c.id.clone()) {
                        out.push(c.clone());
                    }
                }
            }
        }
        rank_cards(out, max_items)
    }

    pub fn related_for_related_entities(
        &self,
        entities: &[String],
        max_items: usize,
    ) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<String>::new();
        let mut out = Vec::new();
        for e in entities {
            if let Some(cards) = self.by_related_entity.get(e) {
                for c in cards {
                    if seen.insert(c.id.clone()) {
                        out.push(c.clone());
                    }
                }
            }
        }
        rank_cards(out, max_items)
    }

    pub fn related_for_tags(&self, tags: &[String], max_items: usize) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<String>::new();
        let mut out = Vec::new();
        for t in tags {
            let key = normalize_key(t);
            if let Some(cards) = self.by_tag.get(&key) {
                for c in cards {
                    if seen.insert(c.id.clone()) {
                        out.push(c.clone());
                    }
                }
            }
        }
        rank_cards(out, max_items)
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

    for dir in knowledge_dirs {
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }
            if path_has_component(path, "archive") {
                continue;
            }

            let path_buf = path.to_path_buf();
            let text = match tokio::fs::read_to_string(&path_buf).await {
                Ok(t) => t,
                Err(_) => continue,
            };
            let (fm, content_start) = KnowledgeFrontmatter::parse(&text);
            if fm.is_archived() || fm.is_deprecated() {
                continue;
            }

            let content_slice = text.get(content_start..).unwrap_or("");
            index.add_from_frontmatter(path_buf, &fm, Some(content_slice));
        }
    }

    index
}

#[cfg(test)]
mod tests {
    use super::*;

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
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![dir.path().to_path_buf()];
        }

        let index = build_knowledge_index(gcx).await;

        assert!(archived_path.exists());
        assert!(deprecated_path.exists());
        assert!(active_path.exists());
        assert_eq!(index.related_for_tags(&vec!["new".to_string()], 5).len(), 1);
    }
}
