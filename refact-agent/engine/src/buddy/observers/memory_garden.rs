use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;

pub struct MemoryGardenObserver;

const MAX_ORPHAN_IDS: usize = 50;
const MAX_MEMORY_FILES: usize = 500;
const MAX_KNOWLEDGE_SCAN_ENTRIES: usize = 5_000;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FRONTMATTER_BYTES: usize = 32 * 1024;

struct KnowledgeEntry {
    id: String,
    title: String,
    tags: Vec<String>,
    related_files: Vec<String>,
    links: Vec<String>,
    file_path: PathBuf,
    created_at: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct KnowledgeCandidate {
    modified_key: u64,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KnowledgeScanStats {
    visited_entries: usize,
    matching_files_considered: usize,
}

#[derive(Debug, Clone, Default)]
struct KnowledgeReferenceScan {
    referenced: HashSet<String>,
    stats: KnowledgeScanStats,
}

async fn knowledge_dirs(gcx: Arc<RwLock<GlobalContext>>) -> Vec<PathBuf> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let mut dirs: Vec<PathBuf> = project_dirs
        .iter()
        .map(|d| d.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|d| d.exists())
        .collect();
    let global_dir = gcx.read().await.config_dir.join("knowledge");
    if global_dir.exists() {
        dirs.push(global_dir);
    }
    dirs
}

async fn scan_knowledge_dirs_from_paths(dirs: Vec<PathBuf>) -> Vec<KnowledgeEntry> {
    let candidates = collect_knowledge_candidates_from_dirs(&dirs, MAX_MEMORY_FILES);
    let mut entries = Vec::new();
    for candidate in candidates {
        let text = match tokio::fs::read_to_string(&candidate.path).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (fm, _) = KnowledgeFrontmatter::parse(&text);
        if fm.is_archived() || fm.is_deprecated() {
            continue;
        }
        let id = fm
            .id
            .clone()
            .unwrap_or_else(|| candidate.path.to_string_lossy().to_string());
        let title = fm.title.clone().unwrap_or_default();
        entries.push(KnowledgeEntry {
            id,
            title,
            tags: fm.tags.clone(),
            related_files: fm.related_files.clone(),
            links: fm.links.clone(),
            file_path: candidate.path,
            created_at: fm.created_at.clone().or_else(|| fm.created.clone()),
            status: fm.status.clone(),
        });
    }
    entries
}

fn system_time_key(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn push_knowledge_candidate(
    heap: &mut BinaryHeap<Reverse<KnowledgeCandidate>>,
    candidate: KnowledgeCandidate,
    max_candidates: usize,
) {
    if max_candidates == 0 {
        return;
    }
    if heap.len() < max_candidates {
        heap.push(Reverse(candidate));
        return;
    }
    let should_replace = heap
        .peek()
        .map(|oldest| candidate > oldest.0)
        .unwrap_or(false);
    if should_replace {
        heap.pop();
        heap.push(Reverse(candidate));
    }
}

fn collect_knowledge_candidates_from_dir(
    dir: &Path,
    heap: &mut BinaryHeap<Reverse<KnowledgeCandidate>>,
    max_candidates: usize,
    max_visited_entries: usize,
    stats: &mut KnowledgeScanStats,
) {
    if max_visited_entries == 0 || stats.visited_entries >= max_visited_entries {
        return;
    }
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if stats.visited_entries >= max_visited_entries {
                return;
            }
            stats.visited_entries += 1;
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                dirs.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            stats.matching_files_considered += 1;
            let modified_key = metadata.modified().map(system_time_key).unwrap_or_default();
            push_knowledge_candidate(
                heap,
                KnowledgeCandidate { modified_key, path },
                max_candidates,
            );
        }
    }
}

fn collect_knowledge_candidates_from_dirs(
    dirs: &[PathBuf],
    max_candidates: usize,
) -> Vec<KnowledgeCandidate> {
    collect_knowledge_candidates_from_dirs_with_stats(
        dirs,
        max_candidates,
        MAX_KNOWLEDGE_SCAN_ENTRIES,
    )
    .0
}

fn collect_knowledge_candidates_from_dirs_with_stats(
    dirs: &[PathBuf],
    max_candidates: usize,
    max_visited_entries: usize,
) -> (Vec<KnowledgeCandidate>, KnowledgeScanStats) {
    let mut heap = BinaryHeap::new();
    let mut stats = KnowledgeScanStats::default();
    for dir in dirs {
        if stats.visited_entries >= max_visited_entries {
            break;
        }
        collect_knowledge_candidates_from_dir(
            dir,
            &mut heap,
            max_candidates,
            max_visited_entries,
            &mut stats,
        );
    }
    let mut candidates = heap
        .into_iter()
        .map(|Reverse(candidate)| candidate)
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.modified_key
            .cmp(&a.modified_key)
            .then_with(|| a.path.cmp(&b.path))
    });
    (candidates, stats)
}

fn read_frontmatter_only(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut frontmatter = String::new();
    let mut line = String::new();
    let mut lines_read = 0usize;
    loop {
        line.clear();
        let read = reader.read_line(&mut line).ok()?;
        if read == 0 {
            return None;
        }
        if frontmatter.len().saturating_add(line.len()) > MAX_FRONTMATTER_BYTES {
            return None;
        }
        lines_read += 1;
        let trimmed = line.trim_end_matches(|ch| ch == '\r' || ch == '\n');
        if lines_read == 1 && trimmed != "---" {
            return None;
        }
        frontmatter.push_str(&line);
        if lines_read > 1 && trimmed == "---" {
            return Some(frontmatter);
        }
    }
}

fn collect_knowledge_references_from_dir(
    dir: &Path,
    scan: &mut KnowledgeReferenceScan,
    max_visited_entries: usize,
) {
    if max_visited_entries == 0 || scan.stats.visited_entries >= max_visited_entries {
        return;
    }
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scan.stats.visited_entries >= max_visited_entries {
                return;
            }
            scan.stats.visited_entries += 1;
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                dirs.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            scan.stats.matching_files_considered += 1;
            let Some(frontmatter) = read_frontmatter_only(&path) else {
                continue;
            };
            let (fm, _) = KnowledgeFrontmatter::parse(&frontmatter);
            if fm.is_archived() || fm.is_deprecated() {
                continue;
            }
            scan.referenced.extend(fm.related_files);
            scan.referenced.extend(fm.links);
            if let Some(superseded_by) = fm.superseded_by {
                scan.referenced.insert(superseded_by);
            }
        }
    }
}

fn scan_knowledge_references_from_paths(dirs: &[PathBuf]) -> KnowledgeReferenceScan {
    let mut scan = KnowledgeReferenceScan::default();
    for dir in dirs {
        if scan.stats.visited_entries >= MAX_KNOWLEDGE_SCAN_ENTRIES {
            break;
        }
        collect_knowledge_references_from_dir(dir, &mut scan, MAX_KNOWLEDGE_SCAN_ENTRIES);
    }
    scan
}

#[cfg(test)]
pub(crate) async fn scan_knowledge_dir_count_for_test(dir: PathBuf) -> usize {
    scan_knowledge_dirs_from_paths(vec![dir]).await.len()
}

fn age_days(created_at: Option<&str>, now: DateTime<Utc>) -> u32 {
    created_at
        .and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s).ok().or_else(|| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .map(|d| {
                        d.and_hms_opt(0, 0, 0)
                            .unwrap()
                            .and_local_timezone(chrono::Utc)
                            .earliest()
                            .unwrap()
                            .into()
                    })
            })
        })
        .map(|dt: chrono::DateTime<chrono::FixedOffset>| {
            now.signed_duration_since(dt.with_timezone(&Utc))
                .num_days()
                .max(0) as u32
        })
        .unwrap_or(0)
}

fn tags_hash(tags: &[String]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut sorted = tags.to_vec();
    sorted.sort();
    let mut h = DefaultHasher::new();
    sorted.hash(&mut h);
    format!("{:x}", h.finish())
}

fn normalized_negation_subject(title: &str) -> Option<(bool, String)> {
    let normalized = title
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let pairs = [
        (true, "do not use "),
        (true, "don't use "),
        (true, "do not "),
        (true, "don't "),
        (true, "avoid "),
        (true, "disable "),
        (false, "use "),
        (false, "enable "),
        (false, "prefer "),
        (false, "do "),
    ];
    for (negated, prefix) in pairs {
        let Some(subject) = normalized.strip_prefix(prefix) else {
            continue;
        };
        let subject = subject
            .trim_matches(|ch: char| ch.is_ascii_punctuation() || ch.is_whitespace())
            .to_string();
        if !subject.is_empty() {
            return Some((negated, subject));
        }
    }
    None
}

fn has_negation_conflict(a_title: &str, b_title: &str) -> Option<String> {
    let (a_negated, a_subject) = normalized_negation_subject(a_title)?;
    let (b_negated, b_subject) = normalized_negation_subject(b_title)?;
    if a_subject == b_subject && a_negated != b_negated {
        return Some(format!("negation subject: {}", a_subject));
    }
    None
}

#[cfg(test)]
fn memory_garden_facts_from_entries(
    entries: &[KnowledgeEntry],
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    memory_garden_facts_from_entries_with_references(entries, &HashSet::new(), now)
}

fn memory_garden_facts_from_entries_with_references(
    entries: &[KnowledgeEntry],
    broader_references: &HashSet<String>,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];

    let mut all_referenced: HashSet<String> = entries
        .iter()
        .flat_map(|e| e.related_files.iter().chain(e.links.iter()).cloned())
        .collect();
    all_referenced.extend(broader_references.iter().cloned());

    let mut orphan_ids: Vec<String> = Vec::new();
    for entry in entries {
        let file_name = entry
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let path_str = entry.file_path.to_string_lossy().to_string();
        let is_referenced = all_referenced.contains(&file_name)
            || all_referenced.contains(&path_str)
            || all_referenced.contains(&entry.id);
        let days = age_days(entry.created_at.as_deref(), now);
        let is_pinned = entry.status.as_deref() == Some("pinned");
        if !is_referenced && days > 7 && !is_pinned {
            orphan_ids.push(entry.id.clone());
            if orphan_ids.len() >= MAX_ORPHAN_IDS {
                break;
            }
        }
    }

    if !orphan_ids.is_empty() {
        tracing::debug!("memory_garden: {} orphan(s)", orphan_ids.len());
        let project_hash = entries
            .first()
            .map(|e| {
                tags_hash(&[e
                    .file_path
                    .parent()
                    .and_then(|p| p.to_str())
                    .unwrap_or("")
                    .to_string()])
            })
            .unwrap_or_default();
        facts.push(BuddyFact {
            kind: BuddyFactKind::MemoryOrphan,
            key: format!("memory:orphan:batch:{}", project_hash),
            source: "memory_garden",
            payload: serde_json::json!({
                "memory_ids": orphan_ids,
                "count": orphan_ids.len(),
                "scope": "scanned_subset",
                "partial": true,
            }),
            seen_at: now,
            confidence: 0.55,
        });
    }

    let mut conflict_groups: HashMap<String, Vec<&KnowledgeEntry>> = HashMap::new();
    for entry in entries {
        let normalized_title = entry.title.trim().to_lowercase();
        if !normalized_title.is_empty() {
            conflict_groups
                .entry(format!("title:{}", normalized_title))
                .or_default()
                .push(entry);
        }
        if let Some((_, subject)) = normalized_negation_subject(&entry.title) {
            conflict_groups
                .entry(format!("negation_subject:{}", subject))
                .or_default()
                .push(entry);
        }
        if !entry.tags.is_empty() {
            conflict_groups
                .entry(format!("tags:{}", tags_hash(&entry.tags)))
                .or_default()
                .push(entry);
        }
    }
    let mut seen_conflicts = HashSet::new();
    for group in conflict_groups.values() {
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let a = group[i];
                let b = group[j];
                let (id_a, id_b) = if a.id <= b.id {
                    (&a.id, &b.id)
                } else {
                    (&b.id, &a.id)
                };
                let key = format!("memory:conflict:{}:{}", id_a, id_b);
                if !seen_conflicts.insert(key.clone()) {
                    continue;
                }
                if let Some(summary) = has_negation_conflict(&a.title, &b.title) {
                    tracing::debug!("memory_garden: conflict {}~{}", id_a, id_b);
                    facts.push(BuddyFact {
                        kind: BuddyFactKind::MemoryStaleConflict,
                        key,
                        source: "memory_garden",
                        payload: serde_json::json!({
                            "doc_ids": [id_a, id_b],
                            "conflict_summary": summary,
                        }),
                        seen_at: now,
                        confidence: 0.65,
                    });
                }
            }
        }
    }

    let mut by_tag_hash: HashMap<String, Vec<&KnowledgeEntry>> = HashMap::new();
    let cutoff = now - chrono::Duration::days(14);
    for entry in entries {
        let days = age_days(entry.created_at.as_deref(), now);
        if days > 14 {
            continue;
        }
        let ts = entry
            .created_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        if let Some(t) = ts {
            if t < cutoff {
                continue;
            }
        }
        if entry.tags.is_empty() {
            continue;
        }
        let hash = tags_hash(&entry.tags);
        by_tag_hash.entry(hash).or_default().push(entry);
    }

    for (hash, group) in &by_tag_hash {
        if group.len() >= 3 {
            tracing::debug!("memory_garden: recurring lesson tag_hash={}", hash);
            facts.push(BuddyFact {
                kind: BuddyFactKind::MemoryRecurringLesson,
                key: format!("memory:recurring:{}", hash),
                source: "memory_garden",
                payload: serde_json::json!({
                    "memory_ids": group.iter().map(|e| &e.id).collect::<Vec<_>>(),
                    "count": group.len(),
                    "tag_hash": hash,
                }),
                seen_at: now,
                confidence: 0.75,
            });
        }
    }

    facts
}

async fn detect_memory_garden(
    gcx: Arc<RwLock<GlobalContext>>,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let dirs = knowledge_dirs(gcx).await;
    let entries = scan_knowledge_dirs_from_paths(dirs.clone()).await;
    let references = scan_knowledge_references_from_paths(&dirs);
    memory_garden_facts_from_entries_with_references(&entries, &references.referenced, now)
}

#[async_trait::async_trait]
impl BuddyObserver for MemoryGardenObserver {
    fn id(&self) -> &'static str {
        "memory_garden"
    }

    fn cadence_seconds(&self) -> u64 {
        600
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.memory_garden
            && settings.housekeeping_enabled
            && settings.proactive_enabled
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        detect_memory_garden(gcx, ctx.now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(id: &str, title: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            title: title.to_string(),
            tags: vec![],
            related_files: vec![id.to_string()],
            links: vec![],
            file_path: PathBuf::from(format!("{id}.md")),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            status: None,
        }
    }

    fn orphan_candidate(id: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            title: id.to_string(),
            tags: vec![],
            related_files: vec![],
            links: vec![],
            file_path: PathBuf::from(format!("{id}.md")),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            status: None,
        }
    }

    fn write_memory(path: &Path, frontmatter: &str) {
        std::fs::write(
            path,
            format!("---\n{frontmatter}\n---\nBody must not be needed\n"),
        )
        .unwrap();
    }

    #[test]
    fn detects_untagged_use_avoid_title_conflict() {
        let entries = vec![
            test_entry("use-x", "Use X"),
            test_entry("avoid-x", "Avoid X"),
        ];
        let facts = memory_garden_facts_from_entries(&entries, Utc::now());

        let conflict = facts
            .iter()
            .find(|fact| fact.kind == BuddyFactKind::MemoryStaleConflict)
            .expect("expected title conflict");
        assert_eq!(
            conflict.payload["doc_ids"],
            serde_json::json!(["avoid-x", "use-x"])
        );
        assert!(conflict.payload["conflict_summary"]
            .as_str()
            .unwrap()
            .contains("negation subject: x"));
    }

    #[test]
    fn detects_do_not_use_before_positive_do_prefix() {
        let entries = vec![
            test_entry("use-pnpm", "Use pnpm"),
            test_entry("do-not-use-pnpm", "Do not use pnpm"),
        ];
        let facts = memory_garden_facts_from_entries(&entries, Utc::now());

        let conflict = facts
            .iter()
            .find(|fact| fact.kind == BuddyFactKind::MemoryStaleConflict)
            .expect("expected do-not-use conflict");
        assert_eq!(
            conflict.payload["doc_ids"],
            serde_json::json!(["do-not-use-pnpm", "use-pnpm"])
        );
        assert!(conflict.payload["conflict_summary"]
            .as_str()
            .unwrap()
            .contains("negation subject: pnpm"));
    }

    #[test]
    fn detects_do_not_before_positive_do_prefix() {
        let positive = normalized_negation_subject("Do deploy previews").unwrap();
        let negative = normalized_negation_subject("Do not deploy previews").unwrap();

        assert_eq!(positive, (false, "deploy previews".to_string()));
        assert_eq!(negative, (true, "deploy previews".to_string()));
        assert_eq!(
            has_negation_conflict("Do deploy previews", "Do not deploy previews").as_deref(),
            Some("negation subject: deploy previews")
        );
    }

    #[test]
    fn knowledge_candidate_collection_is_bounded_and_recent_biased() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..5 {
            let path = dir.path().join(format!("memory_{idx}.md"));
            std::fs::write(&path, format!("# Memory {idx}\n")).unwrap();
            let modified = filetime::FileTime::from_unix_time(100 + idx as i64, 0);
            filetime::set_file_mtime(&path, modified).unwrap();
        }

        let candidates = collect_knowledge_candidates_from_dirs(&[dir.path().to_path_buf()], 3);
        let names = candidates
            .iter()
            .map(|candidate| {
                candidate
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(candidates.len(), 3);
        assert_eq!(names, vec!["memory_4.md", "memory_3.md", "memory_2.md"]);
    }

    #[test]
    fn knowledge_candidate_scan_stops_at_visit_budget() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..8 {
            let path = dir.path().join(format!("memory_{idx}.md"));
            std::fs::write(&path, format!("# Memory {idx}\n")).unwrap();
        }

        let (candidates, stats) =
            collect_knowledge_candidates_from_dirs_with_stats(&[dir.path().to_path_buf()], 8, 4);

        assert_eq!(stats.visited_entries, 4);
        assert_eq!(stats.matching_files_considered, candidates.len());
        assert!(stats.matching_files_considered < 8);
    }

    #[test]
    fn orphan_fact_is_partial_when_based_on_bounded_subset() {
        let entries = vec![orphan_candidate("old-memory")];
        let facts = memory_garden_facts_from_entries(&entries, Utc::now());

        let orphan = facts
            .iter()
            .find(|fact| fact.kind == BuddyFactKind::MemoryOrphan)
            .expect("expected partial orphan fact");
        assert_eq!(orphan.payload["scope"], serde_json::json!("scanned_subset"));
        assert_eq!(orphan.payload["partial"], serde_json::json!(true));
        assert!(orphan.confidence < 0.7);
    }

    #[test]
    fn older_frontmatter_reference_prevents_recent_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let recent = dir.path().join("recent.md");
        let older = dir.path().join("older.md");
        write_memory(
            &recent,
            "id: recent\ntitle: Recent\ncreated_at: \"2026-01-01T00:00:00Z\"",
        );
        write_memory(
            &older,
            "id: older\ntitle: Older\nrelated_files: [\"recent\"]\ncreated_at: \"2025-01-01T00:00:00Z\"",
        );
        filetime::set_file_mtime(&recent, filetime::FileTime::from_unix_time(200, 0)).unwrap();
        filetime::set_file_mtime(&older, filetime::FileTime::from_unix_time(100, 0)).unwrap();

        let entries = collect_knowledge_candidates_from_dirs(&[dir.path().to_path_buf()], 1)
            .into_iter()
            .map(|candidate| KnowledgeEntry {
                id: "recent".to_string(),
                title: "Recent".to_string(),
                tags: vec![],
                related_files: vec![],
                links: vec![],
                file_path: candidate.path,
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                status: None,
            })
            .collect::<Vec<_>>();
        let references = scan_knowledge_references_from_paths(&[dir.path().to_path_buf()]);
        let facts = memory_garden_facts_from_entries_with_references(
            &entries,
            &references.referenced,
            Utc::now(),
        );

        assert!(references.referenced.contains("recent"));
        assert!(!facts
            .iter()
            .any(|fact| fact.kind == BuddyFactKind::MemoryOrphan));
    }

    #[test]
    fn malformed_older_frontmatter_reference_pass_is_skipped_safely() {
        let dir = tempfile::tempdir().unwrap();
        let malformed = dir.path().join("malformed.md");
        std::fs::write(
            &malformed,
            "---\nid: malformed\nrelated_files: [\"target\"\n---\nBody\n",
        )
        .unwrap();

        let references = scan_knowledge_references_from_paths(&[dir.path().to_path_buf()]);

        assert_eq!(references.stats.matching_files_considered, 1);
        assert!(!references.referenced.contains("target"));
    }
}
