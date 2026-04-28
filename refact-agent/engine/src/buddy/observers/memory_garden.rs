use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
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
const MAX_FILE_BYTES: u64 = 256 * 1024;

struct KnowledgeEntry {
    id: String,
    title: String,
    tags: Vec<String>,
    related_files: Vec<String>,
    file_path: PathBuf,
    created_at: Option<String>,
    status: Option<String>,
}

async fn scan_knowledge_dirs(gcx: Arc<RwLock<GlobalContext>>) -> Vec<KnowledgeEntry> {
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
    scan_knowledge_dirs_from_paths(dirs).await
}

async fn scan_knowledge_dirs_from_paths(dirs: Vec<PathBuf>) -> Vec<KnowledgeEntry> {
    let mut candidates = Vec::new();
    for dir in dirs {
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
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            candidates.push((modified, path.to_path_buf()));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut entries = Vec::new();
    for (_, path) in candidates.into_iter().take(MAX_MEMORY_FILES) {
        let text = match tokio::fs::read_to_string(&path).await {
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
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let title = fm.title.clone().unwrap_or_default();
        entries.push(KnowledgeEntry {
            id,
            title,
            tags: fm.tags.clone(),
            related_files: fm.related_files.clone(),
            file_path: path,
            created_at: fm.created_at.clone().or_else(|| fm.created.clone()),
            status: fm.status.clone(),
        });
    }
    entries
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

fn has_negation_conflict(a_title: &str, b_title: &str) -> Option<String> {
    let neg_pairs = [
        ("use ", "avoid "),
        ("enable ", "disable "),
        ("prefer ", "avoid "),
        ("use ", "don't use "),
        ("do ", "don't "),
    ];
    let al = a_title.to_lowercase();
    let bl = b_title.to_lowercase();
    for (pos, neg) in &neg_pairs {
        if al.starts_with(pos) && bl.starts_with(neg) {
            return Some(format!("negation: '{}' vs '{}'", pos.trim(), neg.trim()));
        }
        if bl.starts_with(pos) && al.starts_with(neg) {
            return Some(format!("negation: '{}' vs '{}'", pos.trim(), neg.trim()));
        }
    }
    None
}

async fn detect_memory_garden(
    gcx: Arc<RwLock<GlobalContext>>,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let entries = scan_knowledge_dirs(gcx).await;
    let mut facts = vec![];

    let all_referenced: HashSet<String> = entries
        .iter()
        .flat_map(|e| e.related_files.iter().cloned())
        .collect();

    let mut orphan_ids: Vec<String> = Vec::new();
    for entry in &entries {
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
            }),
            seen_at: now,
            confidence: 0.7,
        });
    }

    let mut conflict_groups: HashMap<String, Vec<&KnowledgeEntry>> = HashMap::new();
    for entry in &entries {
        let normalized_title = entry.title.trim().to_lowercase();
        if !normalized_title.is_empty() {
            conflict_groups
                .entry(format!("title:{}", normalized_title))
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
    for entry in &entries {
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
