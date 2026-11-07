use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind, BuddyJobState};
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::app_state::AppState;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;

pub struct MemoryGardenObserver;

pub(crate) const MAX_PROPOSALS_PER_TICK: usize = 5;
pub(crate) const MAX_PROPOSALS_PER_DAY: usize = 50;
const MAX_ORPHAN_IDS: usize = MAX_PROPOSALS_PER_TICK;
const MAX_MEMORY_FILES: usize = 500;
const MAX_KNOWLEDGE_SCAN_ENTRIES: usize = 5_000;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FRONTMATTER_BYTES: usize = 32 * 1024;
const MEMORY_GARDEN_DAILY_COUNTER_JOB_ID: &str = "memory_garden";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct MemoryGardenDailyCounter {
    day: String,
    count: usize,
    #[serde(default)]
    last_fact_set_fingerprint: Option<String>,
}

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

async fn memory_garden_job_state(gcx: AppState) -> BuddyJobState {
    let buddy_arc = gcx.buddy.buddy.clone();
    let buddy = buddy_arc.lock().await;
    buddy
        .as_ref()
        .and_then(|svc| {
            svc.state
                .job_cooldowns
                .get(MEMORY_GARDEN_DAILY_COUNTER_JOB_ID)
                .cloned()
        })
        .unwrap_or_default()
}

async fn persist_memory_garden_daily_counter(gcx: AppState, counter: &MemoryGardenDailyCounter) {
    let Ok(last_result) = serde_json::to_string(counter) else {
        return;
    };
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut buddy = buddy_arc.lock().await;
    if let Some(svc) = buddy.as_mut() {
        let state = svc
            .state
            .job_cooldowns
            .entry(MEMORY_GARDEN_DAILY_COUNTER_JOB_ID.to_string())
            .or_default();
        state.last_result = Some(last_result);
        svc.dirty = true;
    }
}

async fn knowledge_dirs(gcx: AppState) -> Vec<PathBuf> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.gcx.clone()).await;
    let mut dirs: Vec<PathBuf> = project_dirs
        .iter()
        .map(|d| d.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|d| d.exists())
        .collect();
    let global_dir = gcx.paths.config_dir.join("knowledge");
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

const NEWEST_ALWAYS_INCLUDED: usize = 50;

fn sort_newest_first(candidates: &mut [KnowledgeCandidate]) {
    candidates.sort_by(|a, b| {
        b.modified_key
            .cmp(&a.modified_key)
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn select_rotating_chunk(
    mut all: Vec<KnowledgeCandidate>,
    max_candidates: usize,
    rotation: u64,
) -> Vec<KnowledgeCandidate> {
    if max_candidates == 0 {
        return Vec::new();
    }
    sort_newest_first(&mut all);
    if all.len() <= max_candidates {
        return all;
    }
    let newest_take = (max_candidates / 10).clamp(1, NEWEST_ALWAYS_INCLUDED);
    let mut out: Vec<KnowledgeCandidate> = all.iter().take(newest_take).cloned().collect();
    let chunk_size = max_candidates.saturating_sub(out.len()).max(1);
    all.sort_by(|a, b| a.path.cmp(&b.path));
    let chunk_count = all.len().div_ceil(chunk_size);
    let chunk_idx = (rotation % chunk_count as u64) as usize;
    let start = chunk_idx * chunk_size;
    let end = (start + chunk_size).min(all.len());
    for candidate in &all[start..end] {
        if out.len() >= max_candidates {
            break;
        }
        if !out.iter().any(|existing| existing.path == candidate.path) {
            out.push(candidate.clone());
        }
    }
    if out.len() < max_candidates {
        sort_newest_first(&mut all);
        for candidate in &all {
            if out.len() >= max_candidates {
                break;
            }
            if !out.iter().any(|existing| existing.path == candidate.path) {
                out.push(candidate.clone());
            }
        }
    }
    sort_newest_first(&mut out);
    out
}

fn hourly_rotation() -> u64 {
    (Utc::now().timestamp().max(0) as u64) / 3600
}

fn collect_knowledge_candidates_from_dir(
    dir: &Path,
    all: &mut Vec<KnowledgeCandidate>,
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
            all.push(KnowledgeCandidate {
                modified_key,
                path,
            });
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
    collect_knowledge_candidates_from_dirs_with_rotation(
        dirs,
        max_candidates,
        max_visited_entries,
        hourly_rotation(),
    )
}

fn collect_knowledge_candidates_from_dirs_with_rotation(
    dirs: &[PathBuf],
    max_candidates: usize,
    max_visited_entries: usize,
    rotation: u64,
) -> (Vec<KnowledgeCandidate>, KnowledgeScanStats) {
    let mut all = Vec::new();
    let mut stats = KnowledgeScanStats::default();
    for dir in dirs {
        if stats.visited_entries >= max_visited_entries {
            break;
        }
        collect_knowledge_candidates_from_dir(dir, &mut all, max_visited_entries, &mut stats);
    }
    let candidates = select_rotating_chunk(all, max_candidates, rotation);
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

fn memory_garden_fact_priority(kind: BuddyFactKind) -> u8 {
    if kind == BuddyFactKind::MemoryStaleConflict {
        0
    } else if kind == BuddyFactKind::MemoryRecurringLesson {
        1
    } else {
        2
    }
}

fn sort_and_truncate_memory_garden_facts(facts: &mut Vec<BuddyFact>, limit: usize) {
    facts.sort_by(|a, b| {
        memory_garden_fact_priority(a.kind)
            .cmp(&memory_garden_fact_priority(b.kind))
            .then_with(|| b.seen_at.cmp(&a.seen_at))
            .then_with(|| a.key.cmp(&b.key))
    });
    facts.truncate(limit);
}

fn daily_counter_from_job_state(
    job_state: &BuddyJobState,
    now: DateTime<Utc>,
) -> MemoryGardenDailyCounter {
    let day = now.date_naive().to_string();
    let mut counter = job_state
        .last_result
        .as_deref()
        .and_then(|value| serde_json::from_str::<MemoryGardenDailyCounter>(value).ok())
        .unwrap_or_default();
    if counter.day != day {
        counter.day = day;
        counter.count = 0;
        counter.last_fact_set_fingerprint = None;
    }
    counter
}

fn apply_daily_cap_to_memory_garden_facts(
    mut facts: Vec<BuddyFact>,
    job_state: &BuddyJobState,
    now: DateTime<Utc>,
) -> (Vec<BuddyFact>, MemoryGardenDailyCounter) {
    sort_and_truncate_memory_garden_facts(&mut facts, MAX_PROPOSALS_PER_TICK);
    let mut counter = daily_counter_from_job_state(job_state, now);
    let remaining = MAX_PROPOSALS_PER_DAY.saturating_sub(counter.count);
    facts.truncate(remaining);
    counter.count = counter.count.saturating_add(facts.len());
    (facts, counter)
}

fn memory_garden_fact_set_fingerprint(facts: &[BuddyFact]) -> String {
    let mut parts = facts
        .iter()
        .map(|fact| {
            let mut ids = fact
                .payload
                .get("memory_ids")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .chain(
                    fact.payload
                        .get("doc_ids")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten(),
                )
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect::<Vec<_>>();
            ids.sort();
            format!("{}\u{0}{}", fact.key, ids.join("\u{0}"))
        })
        .collect::<Vec<_>>();
    parts.sort();

    let mut hasher = Sha256::new();
    hasher.update(parts.join("\u{1}").as_bytes());
    hex::encode(hasher.finalize())
}

fn memory_garden_fact_set_changed(facts: &[BuddyFact], counter: &MemoryGardenDailyCounter) -> bool {
    let fingerprint = memory_garden_fact_set_fingerprint(facts);
    counter.last_fact_set_fingerprint.as_deref() != Some(fingerprint.as_str())
}

fn transition_memory_garden_facts(
    facts: Vec<BuddyFact>,
    job_state: &BuddyJobState,
    now: DateTime<Utc>,
) -> (Vec<BuddyFact>, Option<MemoryGardenDailyCounter>) {
    let counter = daily_counter_from_job_state(job_state, now);
    let fingerprint = memory_garden_fact_set_fingerprint(&facts);
    if !memory_garden_fact_set_changed(&facts, &counter) {
        return (vec![], None);
    }
    if facts.is_empty() {
        let mut counter = counter;
        counter.last_fact_set_fingerprint = Some(fingerprint);
        return (vec![], Some(counter));
    }

    let (facts, mut counter) = apply_daily_cap_to_memory_garden_facts(facts, job_state, now);
    counter.last_fact_set_fingerprint = Some(fingerprint);
    (facts, Some(counter))
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

async fn detect_memory_garden(gcx: AppState, now: DateTime<Utc>) -> Vec<BuddyFact> {
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

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let facts = detect_memory_garden(gcx.clone(), ctx.now).await;
        let job_state = memory_garden_job_state(gcx.clone()).await;
        let (facts, counter) = transition_memory_garden_facts(facts, &job_state, ctx.now);
        if let Some(counter) = counter {
            persist_memory_garden_daily_counter(gcx, &counter).await;
        }
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(idx: usize, kind: BuddyFactKind) -> BuddyFact {
        BuddyFact {
            kind,
            key: format!("fact-{idx}"),
            source: "memory_garden",
            payload: serde_json::json!({ "idx": idx }),
            seen_at: DateTime::parse_from_rfc3339(&format!("2026-05-02T00:{:02}:00Z", idx % 60))
                .unwrap()
                .with_timezone(&Utc),
            confidence: 0.9,
        }
    }
}
