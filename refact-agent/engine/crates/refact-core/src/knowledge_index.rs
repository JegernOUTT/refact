use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::knowledge_frontmatter::KnowledgeFrontmatter;

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
    by_signature: HashMap<String, Vec<PathBuf>>,
    by_signal_key: HashMap<String, Vec<PathBuf>>,
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
        self.by_signature.retain(|_, paths| {
            paths.retain(|path| path != file_path);
            !paths.is_empty()
        });
        self.by_signal_key.retain(|_, paths| {
            paths.retain(|path| path != file_path);
            !paths.is_empty()
        });
    }

    pub fn add_signature(&mut self, signature: impl Into<String>, file_path: PathBuf) {
        let signature = signature.into();
        if signature.trim().is_empty() {
            return;
        }
        let entry = self.by_signature.entry(signature).or_default();
        if !entry.contains(&file_path) {
            entry.push(file_path);
        }
    }

    pub fn first_path_for_signature(&self, signature: &str) -> Option<&PathBuf> {
        self.by_signature
            .get(signature)
            .and_then(|paths| paths.first())
    }

    pub fn add_signal_key(&mut self, signal_key: impl Into<String>, file_path: PathBuf) {
        let signal_key = normalize_key(&signal_key.into());
        if signal_key.is_empty() {
            return;
        }
        let entry = self.by_signal_key.entry(signal_key).or_default();
        if !entry.contains(&file_path) {
            entry.push(file_path);
        }
    }

    pub fn first_path_for_signal_key(&self, signal_key: &str) -> Option<&PathBuf> {
        self.by_signal_key
            .get(&normalize_key(signal_key))
            .and_then(|paths| paths.first())
    }

    pub fn content_for_path(&self, file_path: &Path) -> Option<&str> {
        self.content_by_path
            .get(file_path)
            .map(|content| content.as_str())
    }

    pub fn card_for_path(&self, file_path: &Path) -> Option<&KnowledgeCard> {
        self.by_path.get(file_path)
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

        if let Some(signal_key) = fm.signal_key.as_deref() {
            self.add_signal_key(signal_key, file_path.clone());
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
        let mut handbook: Vec<&KnowledgeCard> = Vec::new();
        for (path, card) in &self.by_path {
            if path
                .components()
                .any(|component| component.as_os_str() == "handbook")
                && seen.insert(path.clone())
            {
                handbook.push(card);
            }
        }
        handbook.sort_by(|a, b| {
            card_recency(b)
                .cmp(card_recency(a))
                .then_with(|| a.title.cmp(&b.title))
        });
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
        handbook
            .into_iter()
            .chain(matched)
            .take(max_items)
            .cloned()
            .collect()
    }

    pub fn tag_clusters(&self, min_docs: usize) -> Vec<(String, Vec<KnowledgeCard>)> {
        let mut clusters: Vec<(String, Vec<KnowledgeCard>)> = self
            .by_tag
            .iter()
            .filter(|(tag, _)| {
                tag.len() >= 4
                    && !tag.starts_with("scope:")
                    && tag.chars().any(|ch| ch.is_ascii_alphabetic())
            })
            .filter_map(|(tag, cards)| {
                let mut seen = HashSet::<PathBuf>::new();
                let unique: Vec<KnowledgeCard> = cards
                    .iter()
                    .filter(|card| !card_is_task_scoped(card))
                    .filter(|card| {
                        !card
                            .file_path
                            .components()
                            .any(|component| component.as_os_str() == "handbook")
                    })
                    .filter(|card| seen.insert(card.file_path.clone()))
                    .cloned()
                    .collect();
                (unique.len() >= min_docs).then(|| (tag.clone(), unique))
            })
            .collect();
        clusters.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        clusters
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn add_card_indexes_tags_filenames_and_content() {
        let mut index = KnowledgeIndex::empty();
        index.add_card_with_content(
            KnowledgeCard {
                id: "card-1".to_string(),
                title: "Routing Memo".to_string(),
                summary: Some("summary needle".to_string()),
                description: None,
                tags: vec!["Decision".to_string()],
                filenames: vec!["router.rs".to_string()],
                entities: Vec::new(),
                related_files: Vec::new(),
                related_entities: Vec::new(),
                kind: Some("decision".to_string()),
                created: Some("2026-01-01".to_string()),
                created_at: None,
                updated: None,
                file_path: PathBuf::from("/k/router.md"),
            },
            Some("content token"),
        );

        assert_eq!(
            index
                .related_for_tags(&vec!["decision".to_string()], 5)
                .len(),
            1
        );
        assert_eq!(
            index
                .related_for_files(&vec!["router.rs".to_string()], 5)
                .len(),
            1
        );

        let hits = index.search("token", &KnowledgeSearchFilters::default(), 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card.id, "card-1");
        assert!(hits[0].snippet.contains("content token"));
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
