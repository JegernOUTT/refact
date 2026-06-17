use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::files_correction::get_project_dirs;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};

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
    pub updated: Option<String>,
    pub file_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct KnowledgeIndex {
    by_filename: HashMap<String, Vec<KnowledgeCard>>,
    by_tag: HashMap<String, Vec<KnowledgeCard>>,
    by_entity: HashMap<String, Vec<KnowledgeCard>>,
    by_related_filename: HashMap<String, Vec<KnowledgeCard>>,
    by_related_entity: HashMap<String, Vec<KnowledgeCard>>,
    by_content: HashMap<String, Vec<KnowledgeCard>>,
    by_path: HashMap<PathBuf, KnowledgeCard>,
    content_by_path: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, Default)]
pub struct KnowledgeSearchFilters {
    pub scope: Option<String>,
    pub kind: Option<String>,
    pub namespace: Option<String>,
    pub task_id: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeSearchHit {
    pub card: KnowledgeCard,
    pub snippet: String,
    pub score: f32,
}

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

fn normalize_key(s: &str) -> String {
    s.trim().to_lowercase()
}

fn normalized_contains(values: &[String], expected: &str) -> bool {
    let expected = normalize_key(expected);
    values.iter().any(|value| normalize_key(value) == expected)
}

fn push_unique(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn text_tokens(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' && ch != ':')
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 2)
        .filter(|token| seen.insert(token.clone()))
        .collect()
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

fn card_recency(card: &KnowledgeCard) -> &str {
    card.updated
        .as_deref()
        .or(card.created_at.as_deref())
        .or(card.created.as_deref())
        .unwrap_or("")
}

fn card_is_task_scoped(card: &KnowledgeCard) -> bool {
    card.tags.iter().any(|tag| {
        let key = normalize_key(tag);
        key == "scope:task" || key.starts_with("scope:task:")
    })
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

fn card_matches_filters(card: &KnowledgeCard, filters: &KnowledgeSearchFilters) -> bool {
    if let Some(scope) = &filters.scope {
        if !normalized_contains(&card.tags, &format!("scope:{}", scope)) {
            return false;
        }
    }
    if let Some(kind) = &filters.kind {
        if card.kind.as_deref().map(normalize_key) != Some(normalize_key(kind)) {
            return false;
        }
    }
    if let Some(namespace) = &filters.namespace {
        if !normalized_contains(&card.tags, &format!("namespace:{}", namespace)) {
            return false;
        }
    }
    if let Some(task_id) = &filters.task_id {
        if normalize_key(task_id) != "*"
            && !normalized_contains(&card.tags, &format!("scope:task:{}", task_id))
        {
            return false;
        }
    }
    filters
        .tags
        .iter()
        .all(|tag| normalized_contains(&card.tags, tag))
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = text_tokens(query);
    if terms.is_empty() {
        let trimmed = query.trim().to_ascii_lowercase();
        if !trimmed.is_empty() {
            terms.push(trimmed);
        }
    }
    terms
}

fn content_snippet(content: &str, terms: &[String]) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let prefix_chars = trimmed[..start].chars().count().saturating_sub(80);
    let snippet: String = trimmed.chars().skip(prefix_chars).take(240).collect();
    snippet.replace('\n', " ")
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
        retain_cards_not_at_path(&mut self.by_content, file_path);
        self.by_path.remove(file_path);
        self.content_by_path.remove(file_path);
    }

    pub fn is_empty(&self) -> bool {
        self.by_filename.is_empty()
            && self.by_tag.is_empty()
            && self.by_entity.is_empty()
            && self.by_related_filename.is_empty()
            && self.by_related_entity.is_empty()
            && self.by_content.is_empty()
            && self.by_path.is_empty()
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
            &self.by_content,
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
        self.add_card_with_content(card, None);
    }

    pub fn add_card_with_content(&mut self, card: KnowledgeCard, content: Option<&str>) {
        self.by_path.insert(card.file_path.clone(), card.clone());
        for filename in &card.filenames {
            self.by_filename
                .entry(filename.clone())
                .or_default()
                .push(card.clone());
            let normalized = normalize_key(filename);
            if normalized != *filename {
                self.by_filename
                    .entry(normalized)
                    .or_default()
                    .push(card.clone());
            }
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

        let content_text = content.unwrap_or("").to_string();
        self.content_by_path
            .insert(card.file_path.clone(), content_text.clone());
        let mut searchable = String::new();
        searchable.push_str(&card.title);
        searchable.push('\n');
        if let Some(summary) = &card.summary {
            searchable.push_str(summary);
            searchable.push('\n');
        }
        if let Some(description) = &card.description {
            searchable.push_str(description);
            searchable.push('\n');
        }
        searchable.push_str(&card.tags.join("\n"));
        searchable.push('\n');
        searchable.push_str(&card.filenames.join("\n"));
        searchable.push('\n');
        searchable.push_str(&content_text);
        for token in text_tokens(&searchable) {
            self.by_content.entry(token).or_default().push(card.clone());
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
        let mut filenames = fm.filenames.clone();
        if let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) {
            push_unique(&mut filenames, file_name.to_string());
        }
        if let Some(stem) = file_path.file_stem().and_then(|name| name.to_str()) {
            push_unique(&mut filenames, stem.to_string());
        }

        self.add_card_with_content(
            KnowledgeCard {
                id,
                title,
                summary,
                description,
                tags: fm.tags.clone(),
                filenames,
                entities: fm.entities.clone(),
                related_files: fm.related_files.clone(),
                related_entities: fm.related_entities.clone(),
                kind: fm.kind.clone(),
                created: fm.created.clone(),
                created_at: fm.created_at.clone(),
                updated: fm.updated.clone(),
                file_path,
            },
            content,
        );
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

    pub fn lessons_for_pulse(&self, max_items: usize) -> Vec<KnowledgeCard> {
        let mut seen = HashSet::<PathBuf>::new();
        let mut matched: Vec<&KnowledgeCard> = Vec::new();
        for tag in ["lesson", "convention"] {
            if let Some(cards) = self.by_tag.get(tag) {
                for card in cards {
                    if card_is_task_scoped(card) {
                        continue;
                    }
                    if seen.insert(card.file_path.clone()) {
                        matched.push(card);
                    }
                }
            }
        }
        matched.sort_by(|a, b| {
            card_recency(b)
                .cmp(card_recency(a))
                .then_with(|| a.title.cmp(&b.title))
        });
        matched.into_iter().take(max_items).cloned().collect()
    }

    pub fn memos_by_tag_or_kind_substring(
        &self,
        allowed_tags: &[&str],
        max_items: usize,
    ) -> Vec<(KnowledgeCard, String)> {
        let allowed_lower: Vec<String> =
            allowed_tags.iter().map(|tag| tag.to_lowercase()).collect();
        let mut matched: Vec<&KnowledgeCard> = Vec::new();
        for card in self.by_path.values() {
            if card_is_task_scoped(card) {
                continue;
            }
            let tag_match = card.tags.iter().any(|tag| {
                let tag_lower = tag.to_lowercase();
                allowed_lower
                    .iter()
                    .any(|allowed| tag_lower.contains(allowed.as_str()))
            });
            let kind_match = card.kind.as_deref().map_or(false, |kind| {
                let kind_lower = kind.to_lowercase();
                allowed_lower
                    .iter()
                    .any(|allowed| kind_lower.contains(allowed.as_str()))
            });
            if tag_match || kind_match {
                matched.push(card);
            }
        }
        matched.sort_by(|a, b| {
            card_recency(b)
                .cmp(card_recency(a))
                .then_with(|| a.title.cmp(&b.title))
        });
        matched
            .into_iter()
            .take(max_items)
            .map(|card| {
                let content = self
                    .content_by_path
                    .get(&card.file_path)
                    .cloned()
                    .unwrap_or_default();
                (card.clone(), content)
            })
            .collect()
    }

    pub fn search(
        &self,
        query: &str,
        filters: &KnowledgeSearchFilters,
        max_items: usize,
    ) -> Vec<KnowledgeSearchHit> {
        let terms = query_terms(query);
        let mut scores: HashMap<PathBuf, (KnowledgeCard, f32)> = HashMap::new();
        let mut add_score = |card: &KnowledgeCard, score: f32| {
            if !card_matches_filters(card, filters) {
                return;
            }
            scores
                .entry(card.file_path.clone())
                .and_modify(|(_, current)| *current += score)
                .or_insert_with(|| (card.clone(), score));
        };

        if terms.is_empty() {
            for card in self.all_cards() {
                add_score(&card, 1.0);
            }
        }

        for term in &terms {
            if let Some(cards) = self.by_tag.get(term) {
                for card in cards {
                    add_score(card, 4.0);
                }
            }
            if let Some(cards) = self.by_filename.get(term) {
                for card in cards {
                    add_score(card, 3.0);
                }
            }
            if let Some(cards) = self.by_content.get(term) {
                for card in cards {
                    add_score(card, 1.0);
                }
            }
        }

        let mut hits: Vec<_> = scores
            .into_values()
            .map(|(card, score)| {
                let snippet = self.content_by_path.get(&card.file_path).map_or_else(
                    || content_snippet(card.summary.as_deref().unwrap_or_default(), &terms),
                    |content| content_snippet(content, &terms),
                );
                KnowledgeSearchHit {
                    card,
                    snippet,
                    score,
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    kind_priority(b.card.kind.as_deref())
                        .cmp(&kind_priority(a.card.kind.as_deref()))
                })
                .then_with(|| {
                    recency_key(b.card.created_at.as_deref(), b.card.created.as_deref()).cmp(
                        &recency_key(a.card.created_at.as_deref(), a.card.created.as_deref()),
                    )
                })
                .then_with(|| a.card.title.cmp(&b.card.title))
        });
        hits.truncate(max_items);
        hits
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

        assert!(indexed_paths.contains(&active_path));
        assert!(!indexed_paths.contains(&archived_path));
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

        assert!(indexed_paths.contains(&active_path));
        assert!(!indexed_paths.contains(&old_path));
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

    fn lesson_frontmatter(
        title: &str,
        tags: &[&str],
        kind: Option<&str>,
        created: &str,
        status: Option<&str>,
    ) -> KnowledgeFrontmatter {
        KnowledgeFrontmatter {
            title: Some(title.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            kind: kind.map(|k| k.to_string()),
            created: Some(created.to_string()),
            status: status.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn memos_by_tag_or_kind_substring_matches_tags_kind_recency_and_bounds() {
        let mut index = KnowledgeIndex::empty();
        index.add_from_frontmatter(
            PathBuf::from("/k/a.md"),
            &lesson_frontmatter("Lesson A", &["lesson"], None, "2026-01-02", None),
            Some("body a"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/k/b.md"),
            &lesson_frontmatter("Convention B", &["convention"], None, "2026-01-05", None),
            Some("body b"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/k/c.md"),
            &lesson_frontmatter("Random C", &["misc"], None, "2026-01-09", None),
            Some("body c"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/k/d.md"),
            &lesson_frontmatter(
                "Kind Lessonish",
                &["misc"],
                Some("lesson-note"),
                "2026-01-06",
                None,
            ),
            Some("body d"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/k/e.md"),
            &lesson_frontmatter(
                "Archived Lesson",
                &["lesson"],
                None,
                "2026-01-12",
                Some("archived"),
            ),
            Some("body e"),
        );

        let got = index.memos_by_tag_or_kind_substring(&["lesson", "convention"], 10);
        let titles: Vec<String> = got.iter().map(|(c, _)| c.title.clone()).collect();
        assert!(titles.contains(&"Lesson A".to_string()));
        assert!(titles.contains(&"Convention B".to_string()));
        assert!(titles.contains(&"Kind Lessonish".to_string()));
        assert!(!titles.contains(&"Random C".to_string()));
        assert!(!titles.contains(&"Archived Lesson".to_string()));

        let (_, content_a) = got.iter().find(|(c, _)| c.title == "Lesson A").unwrap();
        assert_eq!(content_a, "body a");

        let top = index.memos_by_tag_or_kind_substring(&["lesson", "convention"], 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0.title, "Kind Lessonish");
    }

    #[test]
    fn memos_by_tag_or_kind_substring_excludes_task_scoped_cards() {
        let mut index = KnowledgeIndex::empty();
        index.add_from_frontmatter(
            PathBuf::from("/k/knowledge.md"),
            &lesson_frontmatter("Knowledge Lesson", &["lesson"], None, "2026-01-02", None),
            Some("k body"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/t/task.md"),
            &lesson_frontmatter(
                "Task Lesson",
                &["lesson", "scope:task", "scope:task:T-1"],
                None,
                "2026-01-09",
                None,
            ),
            Some("t body"),
        );

        let got = index.memos_by_tag_or_kind_substring(&["lesson"], 10);
        let titles: Vec<String> = got.iter().map(|(c, _)| c.title.clone()).collect();
        assert!(titles.contains(&"Knowledge Lesson".to_string()));
        assert!(!titles.contains(&"Task Lesson".to_string()));
    }

    #[test]
    fn lessons_for_pulse_recency_task_exclusion_and_bounds() {
        let mut index = KnowledgeIndex::empty();
        index.add_from_frontmatter(
            PathBuf::from("/k/older.md"),
            &lesson_frontmatter("Older Lesson", &["lesson"], None, "2026-01-02", None),
            Some("older"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/k/newer.md"),
            &lesson_frontmatter(
                "Newer Convention",
                &["convention"],
                None,
                "2026-01-08",
                None,
            ),
            Some("newer"),
        );
        index.add_from_frontmatter(
            PathBuf::from("/t/task.md"),
            &lesson_frontmatter(
                "Task Lesson",
                &["lesson", "scope:task:T-1"],
                None,
                "2026-01-12",
                None,
            ),
            Some("task"),
        );

        let lessons = index.lessons_for_pulse(5);
        let titles: Vec<String> = lessons.iter().map(|c| c.title.clone()).collect();
        assert_eq!(
            titles,
            vec!["Newer Convention".to_string(), "Older Lesson".to_string()]
        );

        let top = index.lessons_for_pulse(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].title, "Newer Convention");
    }
}
