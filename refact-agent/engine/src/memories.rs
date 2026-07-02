use std::path::{PathBuf, Path};
use std::sync::Arc;
use chrono::{Local, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use uuid::Uuid;
use walkdir::WalkDir;

use std::collections::{HashMap, HashSet};
use sha2::{Digest, Sha256};

fn path_contains_component(path: &Path, component: &str) -> bool {
    path.components().any(|c| c.as_os_str() == component)
}

use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::find_trajectory_path;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use crate::knowledge_graph::kg_subchat::{enrich_knowledge_metadata, check_deprecation};
use crate::knowledge_graph::build_knowledge_graph;
use crate::http::routers::v1::knowledge_ops::auto_link_memory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StorageScope {
    Local,
    Global,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MemoRecord {
    pub memid: String,
    pub tags: Vec<String>,
    pub content: String,
    pub file_path: Option<PathBuf>,
    pub line_range: Option<(u64, u64)>,
    pub title: Option<String>,
    pub created: Option<String>,
    pub kind: Option<String>,
    pub score: Option<f32>, // VecDB similarity score (lower distance = higher relevance)
}

fn generate_slug(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("knowledge");
    first_line
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .take(50)
        .collect()
}

fn generate_filename(content: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let slug = generate_slug(content);
    let short_uuid = &Uuid::new_v4().to_string()[..8];
    if slug.is_empty() {
        format!("{}_{}_knowledge.md", timestamp, short_uuid)
    } else {
        format!("{}_{}_{}.md", timestamp, short_uuid, slug)
    }
}

async fn load_parent_id_from_trajectory(gcx: Arc<GlobalContext>, path: &PathBuf) -> Option<String> {
    let text = get_file_text_from_memory_or_disk(gcx, path).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("parent_id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

async fn load_root_chat_id_from_trajectory(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
) -> Option<String> {
    let text = get_file_text_from_memory_or_disk(gcx, path).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("root_chat_id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

async fn resolve_root_chat_id(
    gcx: Arc<GlobalContext>,
    start_id: &str,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(r) = cache.get(start_id) {
        return r.clone();
    }

    let mut seen = std::collections::HashSet::new();
    let mut current = start_id.to_string();

    for _ in 0..50 {
        if !seen.insert(current.clone()) {
            cache.insert(start_id.to_string(), start_id.to_string());
            return start_id.to_string();
        }

        let Some(path) = find_trajectory_path(gcx.clone(), &current).await else {
            cache.insert(start_id.to_string(), current.clone());
            return current;
        };

        match load_parent_id_from_trajectory(gcx.clone(), &path).await {
            None => {
                cache.insert(start_id.to_string(), current.clone());
                return current;
            }
            Some(parent) => current = parent,
        }
    }

    cache.insert(start_id.to_string(), start_id.to_string());
    start_id.to_string()
}

pub fn create_frontmatter(
    title: Option<&str>,
    tags: &[String],
    filenames: &[String],
    links: &[String],
    kind: &str,
) -> KnowledgeFrontmatter {
    let now = Local::now();
    let created = now.format("%Y-%m-%d").to_string();
    let review_days = match kind {
        "trajectory" => 90,
        "preference" => 365,
        _ => 90,
    };
    let review_after = (now + Duration::days(review_days))
        .format("%Y-%m-%d")
        .to_string();

    KnowledgeFrontmatter {
        id: Some(Uuid::new_v4().to_string()),
        title: title.map(|t| t.to_string()),
        tags: normalize_memory_tags(tags, 16),
        created: Some(created.clone()),
        updated: Some(created),
        filenames: filenames.to_vec(),
        links: links.to_vec(),
        kind: Some(kind.to_string()),
        status: Some("active".to_string()),
        superseded_by: None,
        deprecated_at: None,
        review_after: Some(review_after),
        source_chat_id: None,

        created_at: Some(Utc::now().to_rfc3339()),
        summary: None,
        description: None,
        entities: Vec::new(),
        related_files: Vec::new(),
        related_entities: Vec::new(),
        content_hash: None,
        source_tool: None,
        source_confidence: None,
        source_trajectory_id: None,
        source_message_range: None,
        source_commit: None,
        topic: None,
        last_used_at: None,
        use_count: 0,
        last_injected_at: None,
        dismissed_count: 0,
        source_id: None,
        source_content_hash: None,
        review_needed: None,
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

fn compute_content_hash_hex(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    hex::encode(h.finalize())
}

fn normalize_memory_tag(tag: &str) -> Option<String> {
    let raw = tag.trim().to_lowercase();
    if raw.is_empty() {
        return None;
    }

    let alias = match raw.as_str() {
        "rs" | "rust" => "language:rust",
        "py" | "python" => "language:python",
        "ts" | "typescript" => "language:typescript",
        "js" | "javascript" => "language:javascript",
        "tsx" => "language:typescript-react",
        "jsx" => "language:javascript-react",
        "c++" | "cpp" => "language:cpp",
        "c#" | "csharp" => "language:csharp",
        "behavior_learner" | "behavior-learner" => "workflow:behavior-learner",
        "knowledge_graph" | "knowledge-graph" => "domain:knowledge-graph",
        "playwright" | "playwright mcp" | "playwright-mcp" => "tool:playwright-mcp",
        "chrome" | "chrome tool" | "chrome-tool" => "tool:chrome",
        _ => raw.as_str(),
    };

    let preserve_underscores = alias.starts_with("entity:") || alias.starts_with("symbol:");
    let mut normalized = String::new();
    let mut last_was_dash = false;

    for ch in alias.chars() {
        match ch {
            'a'..='z' | '0'..='9' | ':' | '/' | '.' => {
                normalized.push(ch);
                last_was_dash = false;
            }
            '_' if preserve_underscores => {
                normalized.push(ch);
                last_was_dash = false;
            }
            '-' | '_' | ' ' | '\t' => {
                if !last_was_dash && !normalized.is_empty() {
                    normalized.push('-');
                    last_was_dash = true;
                }
            }
            _ => {}
        }
    }

    let normalized = normalized
        .trim_matches('-')
        .chars()
        .take(96)
        .collect::<String>();
    if normalized.len() >= 2 {
        Some(normalized)
    } else {
        None
    }
}

pub fn normalize_memory_tags(tags: &[String], max_tags: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for tag in tags {
        let Some(normalized) = normalize_memory_tag(tag) else {
            continue;
        };
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
        if out.len() >= max_tags {
            break;
        }
    }

    out
}

fn push_memory_tag(tags: &mut Vec<String>, tag: &str) {
    let Some(normalized) = normalize_memory_tag(tag) else {
        return;
    };
    if !tags.contains(&normalized) {
        tags.push(normalized);
    }
}

fn content_has_any(content_lower: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| content_lower.contains(term))
}

fn push_if_content_matches(tags: &mut Vec<String>, content_lower: &str, tag: &str, terms: &[&str]) {
    if content_has_any(content_lower, terms) {
        push_memory_tag(tags, tag);
    }
}

fn push_file_path_tags(tags: &mut Vec<String>, file: &str) {
    let lower = file.to_lowercase();

    match Path::new(file).extension().and_then(|ext| ext.to_str()) {
        Some("rs") => push_memory_tag(tags, "language:rust"),
        Some("py") => push_memory_tag(tags, "language:python"),
        Some("ts") => push_memory_tag(tags, "language:typescript"),
        Some("tsx") => push_memory_tag(tags, "language:typescript-react"),
        Some("js") => push_memory_tag(tags, "language:javascript"),
        Some("jsx") => push_memory_tag(tags, "language:javascript-react"),
        Some("kt") => push_memory_tag(tags, "language:kotlin"),
        Some("go") => push_memory_tag(tags, "language:go"),
        Some("java") => push_memory_tag(tags, "language:java"),
        Some("md") | Some("mdx") => push_memory_tag(tags, "format:markdown"),
        _ => {}
    }

    let mappings = [
        ("component:buddy", &["/buddy/", "src/buddy/"][..]),
        (
            "component:knowledge-graph",
            &["/knowledge_graph/", "src/knowledge_graph/"][..],
        ),
        ("component:knowledge-index", &["knowledge_index.rs"][..]),
        (
            "component:memory-system",
            &["memories.rs", "/knowledge/"][..],
        ),
        ("component:chat-engine", &["/chat/", "src/chat/"][..]),
        ("component:subchat", &["subchat.rs", "/subchat/"][..]),
        ("component:tools", &["/tools/", "src/tools/"][..]),
        (
            "component:gui-chat",
            &["gui/src/features/chat/", "gui/src/components/chat"][..],
        ),
        (
            "component:gui-state",
            &["gui/src/app/", "gui/src/features/"][..],
        ),
        (
            "component:integrations",
            &["/integrations/", "src/integrations/"][..],
        ),
        (
            "component:providers",
            &["/providers/", "src/providers/"][..],
        ),
        ("component:vecdb", &["/vecdb/", "src/vecdb/"][..]),
        ("component:ast", &["/ast/", "src/ast/"][..]),
        ("component:http-api", &["/http/", "src/http/"][..]),
    ];

    for (tag, terms) in mappings {
        if terms.iter().any(|term| lower.contains(term)) {
            push_memory_tag(tags, tag);
        }
    }
}

fn extract_fallback_tags(
    content: &str,
    detected_files: &[String],
    detected_entities: &[String],
) -> Vec<String> {
    let content_lower = content.to_lowercase();
    let mut tags = Vec::new();

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "domain:buddy",
        &["buddy", "memory garden", "behavior learner"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "domain:knowledge-graph",
        &[
            "knowledge graph",
            "kg builder",
            "kg linking",
            "kg mechanism",
        ],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "domain:memory-system",
        &["memory", "memories", "knowledge entry", "knowledge content"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "domain:browser-automation",
        &[
            "browser automation",
            "playwright",
            "chrome tool",
            "accessibility snapshot",
        ],
    );

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "workflow:background-chat",
        &[
            "background chat",
            "autonomous chat",
            "self-initiated",
            "proactive chat",
        ],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "workflow:behavior-learner",
        &[
            "behavior learner",
            "auto-write preferences",
            "auto write preferences",
        ],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "workflow:memory-enrichment",
        &[
            "memory enrichment",
            "kg enrich",
            "enrich knowledge",
            "tag generation",
        ],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "workflow:patch-verification",
        &[
            "patch",
            "apply_patch",
            "verify the actual file",
            "edits silently fail",
        ],
    );

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "component:buddy-scheduler",
        &["scheduler", "buddy job", "buddy jobs", "job/subchat"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "component:subchat",
        &["subchat", "run_subchat", "run_subchat_once"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "component:trajectory-persistence",
        &["trajectory", "save_trajectory_as", "source_trajectory_id"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "component:notifications",
        &["notification", "notifications", "action button", "expire"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "component:buddy-animation",
        &[
            "animation",
            "roaming",
            "waypoint",
            "motion",
            "fireflies",
            "aurora",
            "garden growth",
        ],
    );

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "tool:playwright-mcp",
        &[
            "playwright mcp",
            "playwright",
            "accessibility tree",
            "browser snapshot",
        ],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "tool:chrome",
        &["chrome tool", "chrome", "tab management"],
    );

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:cargo-check",
        &["cargo check"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:rust-tests",
        &["cargo test", "rust unit test"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:git-sanity",
        &["git status", "git diff", "git sanity"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:eslint",
        &["eslint", "max-warnings 0"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:prettier",
        &["prettier", "format:check"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "verification:vitest",
        &["vitest"],
    );

    push_if_content_matches(
        &mut tags,
        &content_lower,
        "language:rust",
        &["cargo", "rust"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "language:typescript",
        &["typescript", "tsc", "eslint", "react"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "framework:react",
        &["react", "redux"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "state:redux",
        &["redux", "rtk query"],
    );
    push_if_content_matches(
        &mut tags,
        &content_lower,
        "protocol:sse",
        &["sse", "stream_delta"],
    );

    for file in detected_files.iter().take(8) {
        push_file_path_tags(&mut tags, file);
    }

    for entity in detected_entities.iter().take(8) {
        if entity.len() >= 3 && entity.len() <= 80 {
            push_memory_tag(&mut tags, &format!("entity:{}", entity));
        }
    }

    tags.truncate(16);
    tags
}

pub async fn get_global_knowledge_dir(gcx: Arc<GlobalContext>) -> PathBuf {
    let config_dir = gcx.config_dir.clone();
    config_dir.join("knowledge")
}

async fn get_all_knowledge_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|p| p.exists())
        .collect();

    let global_dir = get_global_knowledge_dir(gcx).await;
    if global_dir.exists() {
        dirs.push(global_dir);
    }

    dirs
}

async fn get_first_knowledge_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(KNOWLEDGE_FOLDER_NAME))
}

#[allow(dead_code)]
pub async fn get_knowledge_dir_for_scope(
    gcx: Arc<GlobalContext>,
    scope: StorageScope,
) -> Result<PathBuf, String> {
    match scope {
        StorageScope::Local => get_first_knowledge_dir(gcx).await,
        StorageScope::Global => Ok(get_global_knowledge_dir(gcx).await),
    }
}

pub async fn memories_add(
    gcx: Arc<GlobalContext>,
    frontmatter: &KnowledgeFrontmatter,
    content: &str,
) -> Result<PathBuf, String> {
    let knowledge_dir = get_first_knowledge_dir(gcx.clone()).await?;
    fs::create_dir_all(&knowledge_dir)
        .await
        .map_err(|e| format!("Failed to create knowledge dir: {}", e))?;

    let filename = generate_filename(content);
    let file_path = knowledge_dir.join(&filename);

    if file_path.exists() {
        return Err(format!("File already exists: {}", file_path.display()));
    }

    let md_content = format!("{}\n\n{}", frontmatter.to_yaml(), content);
    atomic_write_text(&file_path, &md_content).await?;

    info!("Created knowledge entry: {}", file_path.display());

    // Update fast in-memory knowledge index (best-effort, new docs going forward).
    {
        let gcx_read = gcx.clone();
        let mut idx = gcx_read.knowledge_index.lock().await;
        idx.add_from_frontmatter(file_path.clone(), frontmatter, Some(content));
    }

    let vec_db = gcx.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], true)
            .await;
    }

    Ok(file_path)
}

pub async fn load_memories_by_tags(
    gcx: Arc<GlobalContext>,
    allowed_tags: &[&str],
    max_items: usize,
) -> Result<Vec<MemoRecord>, String> {
    let knowledge_dirs = get_all_knowledge_dirs(gcx.clone()).await;

    if knowledge_dirs.is_empty() {
        return Ok(vec![]);
    }

    let mut records = Vec::new();

    for knowledge_dir in &knowledge_dirs {
        if !knowledge_dir.exists() {
            continue;
        }

        for entry in WalkDir::new(knowledge_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path_contains_component(path, "archive") {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }

            let text =
                match get_file_text_from_memory_or_disk(gcx.clone(), &path.to_path_buf()).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };

            let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }

            let has_matching_tag = frontmatter.tags.iter().any(|tag| {
                let tag_lower = tag.to_lowercase();
                allowed_tags
                    .iter()
                    .any(|allowed| tag_lower.contains(&allowed.to_lowercase()))
            });

            let kind_matches = frontmatter.kind.as_ref().map_or(false, |k| {
                let kind_lower = k.to_lowercase();
                allowed_tags
                    .iter()
                    .any(|allowed| kind_lower.contains(&allowed.to_lowercase()))
            });

            if !has_matching_tag && !kind_matches {
                continue;
            }

            let content = text[content_start..].trim().to_string();
            let id = frontmatter
                .id
                .clone()
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            records.push(MemoRecord {
                memid: id,
                tags: frontmatter.tags,
                content,
                file_path: Some(path.to_path_buf()),
                line_range: None,
                title: frontmatter.title,
                created: frontmatter.created,
                kind: frontmatter.kind,
                score: None,
            });
        }
    }

    records.sort_by(|a, b| {
        let date_a = a.created.as_deref().unwrap_or("");
        let date_b = b.created.as_deref().unwrap_or("");
        date_b.cmp(date_a)
    });

    records.truncate(max_items);

    tracing::info!(
        "load_memories_by_tags: found {} memories with tags {:?}",
        records.len(),
        allowed_tags
    );

    Ok(records)
}

pub async fn load_memories_by_tags_from_index(
    gcx: Arc<GlobalContext>,
    allowed_tags: &[&str],
    max_items: usize,
) -> Vec<MemoRecord> {
    let idx_arc = gcx.knowledge_index.clone();
    let matched = {
        let idx = idx_arc.lock().await;
        idx.memos_by_tag_or_kind_substring(allowed_tags, max_items)
    };
    matched
        .into_iter()
        .map(|(card, content)| MemoRecord {
            memid: card.id,
            tags: card.tags,
            content: content.trim().to_string(),
            file_path: Some(card.file_path),
            line_range: None,
            title: Some(card.title),
            created: card.created,
            kind: card.kind,
            score: None,
        })
        .collect()
}

const PREFERENCE_MIN_CONFIDENCE: f32 = 0.85;
#[cfg(test)]
const PREFERENCE_STATEMENT_MAX_CHARS: usize = 240;
#[cfg(test)]
const PREFERENCE_EVIDENCE_MAX_CHARS: usize = 600;

pub fn normalize_preference_text_for_dedupe(text: &str) -> String {
    text.to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn preference_statement_is_safe(statement: &str, confidence: f32) -> bool {
    if confidence < PREFERENCE_MIN_CONFIDENCE {
        return false;
    }
    let trimmed = statement.trim();
    if trimmed.chars().count() < 12 {
        return false;
    }
    let redacted = crate::buddy::actor::redact_sensitive(trimmed);
    if redacted != trimmed || redacted.contains("[REDACTED") {
        return false;
    }
    let normalized = normalize_preference_text_for_dedupe(trimmed);
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() < 3 {
        return false;
    }
    let vague = [
        "remember this",
        "do this",
        "use this",
        "i like this",
        "i prefer this",
        "preference",
        "user preference",
    ];
    if vague.contains(&normalized.as_str()) {
        return false;
    }
    let sensitive_terms = [
        "password",
        "token",
        "secret",
        "credential",
        "api key",
        "apikey",
        "private key",
        "ssh key",
        "home address",
        "phone number",
        "email address",
        "ssn",
        "credit card",
    ];
    if sensitive_terms.iter().any(|term| normalized.contains(term)) {
        return false;
    }
    let lower = trimmed.to_lowercase();
    let preference_cues = [
        "prefer",
        "always",
        "never",
        "avoid",
        "don't",
        "do not",
        "i like",
        "i want",
        "please use",
        "please keep",
        "use ",
        "keep ",
        "format ",
        "respond ",
        "write ",
    ];
    preference_cues.iter().any(|cue| lower.contains(cue))
}

#[cfg(test)]
fn redact_and_cap_preference_text(text: &str, max_chars: usize) -> String {
    let redacted = crate::buddy::actor::redact_sensitive(text);
    let collapsed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::llm::safe_truncate(&collapsed, max_chars)
        .trim()
        .to_string()
}

#[cfg(test)]
fn preference_matches_existing(existing: &MemoRecord, normalized_statement: &str) -> bool {
    let mut candidates = vec![existing.content.as_str()];
    if let Some(title) = existing.title.as_deref() {
        candidates.push(title);
    }
    candidates.into_iter().any(|text| {
        let normalized = normalize_preference_text_for_dedupe(text);
        let normalized_without_heading = normalized.strip_prefix("# ").unwrap_or(&normalized);
        normalized == normalized_statement
            || normalized_without_heading == normalized_statement
            || normalized.contains(normalized_statement)
            || normalized_statement.contains(&normalized)
            || normalized_statement.contains(normalized_without_heading)
    })
}

#[cfg(test)]
fn preference_file_already_exists(knowledge_dirs: &[PathBuf], normalized_statement: &str) -> bool {
    knowledge_dirs.iter().any(|knowledge_dir| {
        if !knowledge_dir.exists() {
            return false;
        }
        WalkDir::new(knowledge_dir)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                let path = entry.path();
                if !path.is_file() || path_contains_component(path, "archive") {
                    return false;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "md" && ext != "mdx" {
                    return false;
                }
                let Ok(text) = std::fs::read_to_string(path) else {
                    return false;
                };
                let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
                if frontmatter.is_archived()
                    || frontmatter.is_deprecated()
                    || !frontmatter
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains("preference"))
                {
                    return false;
                }
                let content = text[content_start..].trim();
                let record = MemoRecord {
                    memid: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    tags: frontmatter.tags,
                    content: content.to_string(),
                    file_path: Some(path.to_path_buf()),
                    line_range: None,
                    title: frontmatter.title,
                    created: frontmatter.created,
                    kind: frontmatter.kind,
                    score: None,
                };
                preference_matches_existing(&record, normalized_statement)
            })
    })
}

#[cfg(test)]
pub async fn memories_add_preference_if_new(
    gcx: Arc<GlobalContext>,
    statement: &str,
    evidence: &str,
    confidence: f32,
) -> Result<Option<PathBuf>, String> {
    if !preference_statement_is_safe(statement, confidence) {
        return Ok(None);
    }

    let statement = redact_and_cap_preference_text(statement, PREFERENCE_STATEMENT_MAX_CHARS);
    let evidence = redact_and_cap_preference_text(evidence, PREFERENCE_EVIDENCE_MAX_CHARS);
    let normalized_statement = normalize_preference_text_for_dedupe(&statement);
    if normalized_statement.is_empty() {
        return Ok(None);
    }

    let existing = load_memories_by_tags(gcx.clone(), &["preference"], 200).await?;
    if existing
        .iter()
        .any(|memory| preference_matches_existing(memory, &normalized_statement))
    {
        return Ok(None);
    }

    let knowledge_dirs = get_all_knowledge_dirs(gcx.clone()).await;
    if preference_file_already_exists(&knowledge_dirs, &normalized_statement) {
        return Ok(None);
    }

    let tags = vec![
        "preference".to_string(),
        "buddy".to_string(),
        "behavior_learner".to_string(),
    ];
    let empty = Vec::<String>::new();
    let mut frontmatter = create_frontmatter(Some(&statement), &tags, &empty, &empty, "preference");
    frontmatter.summary = Some(statement.clone());
    if !evidence.is_empty() {
        frontmatter.description = Some(evidence.clone());
    }
    frontmatter.content_hash = Some(compute_content_hash_hex(&statement));
    frontmatter.source_tool = Some("buddy_behavior_learner".to_string());

    let content = if evidence.is_empty() {
        format!("# {}\n\nConfidence: {:.2}", statement, confidence)
    } else {
        format!(
            "# {}\n\nEvidence: {}\n\nConfidence: {:.2}",
            statement, evidence, confidence
        )
    };

    memories_add(gcx, &frontmatter, &content).await.map(Some)
}

pub async fn memories_search(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n_memories: usize,
    top_n_trajectories: usize,
    exclude_trajectory_id: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    let knowledge_dirs = get_all_knowledge_dirs(gcx.clone()).await;

    let mut root_cache: HashMap<String, String> = HashMap::new();
    let exclude_root = match exclude_trajectory_id {
        Some(id) => Some(resolve_root_chat_id(gcx.clone(), id, &mut root_cache).await),
        None => None,
    };

    // Resolve trajectory dirs before acquiring vec_db lock to avoid holding the
    // mutex across gcx.read() calls (which could deadlock with concurrent gcx.write()).
    let trajectory_dirs = crate::chat::trajectories::get_all_trajectories_dirs(gcx.clone()).await;

    let vecdb_arc = {
        let gcx_read = gcx.clone();
        gcx_read.vec_db.clone()
    };

    let vecdb_guard = vecdb_arc.lock().await;
    if vecdb_guard.is_none() {
        drop(vecdb_guard);
        return memories_search_fallback(
            gcx,
            query,
            top_n_memories,
            &knowledge_dirs,
            exclude_root.as_deref(),
        )
        .await;
    }

    let vecdb = vecdb_guard.as_ref().unwrap();

    // Improve recall by doing two scoped searches:
    // - knowledge roots
    // - trajectory roots
    // This avoids code chunks dominating a global top-K.
    let embedding = vecdb.embed_query(query).await?;

    let k_knowledge = ((top_n_memories.max(1) + top_n_trajectories.max(1)) * 50).min(400);
    let k_trajectories = ((top_n_memories.max(1) + top_n_trajectories.max(1)) * 50).min(400);

    let mut combined_results: Vec<crate::vecdb::vdb_structs::VecdbRecord> = Vec::new();

    for kd in &knowledge_dirs {
        let prefix = if kd.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
            kd.to_string_lossy().to_string()
        } else {
            format!("{}{}", kd.to_string_lossy(), std::path::MAIN_SEPARATOR)
        };
        let filter = format!("(scope LIKE '{}%')", prefix.replace('"', "\\\""));
        if let Ok(res) = vecdb
            .vecdb_search_with_embedding(&embedding, k_knowledge, Some(filter))
            .await
        {
            combined_results.extend(res);
        }
    }

    for td in &trajectory_dirs {
        let prefix = if td.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
            td.to_string_lossy().to_string()
        } else {
            format!("{}{}", td.to_string_lossy(), std::path::MAIN_SEPARATOR)
        };
        let filter = format!("(scope LIKE '{}%')", prefix.replace('"', "\\\""));
        if let Ok(res) = vecdb
            .vecdb_search_with_embedding(&embedding, k_trajectories, Some(filter))
            .await
        {
            combined_results.extend(res);
        }
    }

    // De-dup identical segments; keep best usefulness.
    combined_results.sort_by(|a, b| b.usefulness.total_cmp(&a.usefulness));
    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::<(PathBuf, u64, u64)>::new();
    for r in combined_results {
        let key = (r.file_path.clone(), r.start_line, r.end_line);
        if seen.insert(key) {
            deduped.push(r);
        }
        if deduped.len() >= (k_knowledge + k_trajectories) {
            break;
        }
    }

    let search_result = crate::vecdb::vdb_structs::SearchResult {
        query_text: query.to_string(),
        results: deduped,
    };
    drop(vecdb_guard);

    struct KnowledgeMatch {
        best_score: f32,
    }
    struct TrajectoryMatch {
        best_score: f32,
        matched_ranges: Vec<(u64, u64)>,
    }

    let mut knowledge_matches: HashMap<PathBuf, KnowledgeMatch> = HashMap::new();
    let mut trajectory_matches: HashMap<PathBuf, TrajectoryMatch> = HashMap::new();

    for rec in search_result.results.iter() {
        // Prefer VecDB usefulness normalization over raw distance.
        let score = (rec.usefulness / 100.0).clamp(0.0, 1.0);
        let is_knowledge = knowledge_dirs.iter().any(|d| rec.file_path.starts_with(d))
            && !path_contains_component(&rec.file_path, "archive");
        let is_trajectory = trajectory_dirs.iter().any(|d| rec.file_path.starts_with(d))
            && rec
                .file_path
                .extension()
                .map(|e| e == "json")
                .unwrap_or(false);

        if is_knowledge {
            knowledge_matches
                .entry(rec.file_path.clone())
                .and_modify(|m| {
                    if score > m.best_score {
                        m.best_score = score;
                    }
                })
                .or_insert(KnowledgeMatch { best_score: score });
        } else if is_trajectory {
            let traj_id = rec
                .file_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            if let Some(ref ex_root) = exclude_root {
                if !traj_id.is_empty() {
                    let candidate_root = if let Some(cached) = root_cache.get(&traj_id) {
                        cached.clone()
                    } else if let Some(stored_root) =
                        load_root_chat_id_from_trajectory(gcx.clone(), &rec.file_path).await
                    {
                        root_cache.insert(traj_id.clone(), stored_root.clone());
                        stored_root
                    } else {
                        resolve_root_chat_id(gcx.clone(), &traj_id, &mut root_cache).await
                    };
                    if candidate_root == *ex_root {
                        continue;
                    }
                }
            }

            trajectory_matches
                .entry(rec.file_path.clone())
                .and_modify(|m| {
                    if score > m.best_score {
                        m.best_score = score;
                    }
                    m.matched_ranges.push((rec.start_line, rec.end_line));
                })
                .or_insert(TrajectoryMatch {
                    best_score: score,
                    matched_ranges: vec![(rec.start_line, rec.end_line)],
                });
        }
    }

    let mut records = Vec::new();

    // Process knowledge files (whole content)
    let mut sorted_knowledge: Vec<_> = knowledge_matches.into_iter().collect();
    sorted_knowledge.sort_by(|a, b| {
        b.1.best_score
            .partial_cmp(&a.1.best_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (file_path, file_match) in sorted_knowledge.into_iter().take(top_n_memories) {
        let text = match get_file_text_from_memory_or_disk(gcx.clone(), &file_path).await {
            Ok(t) => t,
            Err(_) => continue,
        };

        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
        if frontmatter.is_archived() || frontmatter.is_deprecated() {
            continue;
        }

        if let (Some(ref ex_root), Some(ref source_id)) =
            (&exclude_root, &frontmatter.source_chat_id)
        {
            if source_id == ex_root {
                tracing::debug!(
                    "Excluding knowledge created by current chat: {:?}",
                    file_path
                );
                continue;
            }
        }

        let content = text[content_start..].trim().to_string();
        let line_count = content.lines().count();
        let id = frontmatter
            .id
            .clone()
            .unwrap_or_else(|| file_path.to_string_lossy().to_string());

        records.push(MemoRecord {
            memid: id,
            tags: frontmatter.tags,
            content,
            file_path: Some(file_path),
            line_range: Some((1, line_count as u64)),
            title: frontmatter.title,
            created: frontmatter.created,
            kind: frontmatter.kind,
            score: Some(file_match.best_score),
        });
    }

    // Process trajectories (matched parts only)
    let mut sorted_trajectories: Vec<_> = trajectory_matches.into_iter().collect();
    sorted_trajectories.sort_by(|a, b| {
        b.1.best_score
            .partial_cmp(&a.1.best_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (file_path, traj_match) in sorted_trajectories.into_iter().take(top_n_trajectories) {
        let text = match get_file_text_from_memory_or_disk(gcx.clone(), &file_path).await {
            Ok(t) => t,
            Err(_) => continue,
        };

        let traj_json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let traj_id = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let traj_title = traj_json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let messages = match traj_json.get("messages").and_then(|v| v.as_array()) {
            Some(m) => m,
            None => continue,
        };

        // Extract matched message content
        let mut matched_content = Vec::new();
        for (start, end) in &traj_match.matched_ranges {
            let start_idx = *start as usize;
            let end_idx = (*end as usize).min(messages.len().saturating_sub(1));

            for idx in start_idx..=end_idx {
                if let Some(msg) = messages.get(idx) {
                    let role = msg
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let content = msg
                        .get("content")
                        .map(|v| {
                            if let Some(s) = v.as_str() {
                                s.chars().take(500).collect::<String>()
                            } else {
                                v.to_string().chars().take(500).collect()
                            }
                        })
                        .unwrap_or_default();

                    if !content.is_empty() && role != "system" && role != "context_file" {
                        matched_content.push(format!("[msg {}] {}: {}", idx, role, content));
                    }
                }
            }
        }

        if matched_content.is_empty() {
            continue;
        }

        let content = format!(
            "Trajectory: {} ({})\n\n{}",
            traj_title,
            traj_id,
            matched_content.join("\n\n")
        );

        records.push(MemoRecord {
            memid: traj_id.clone(),
            tags: vec!["trajectory".to_string()],
            content,
            file_path: Some(file_path),
            line_range: None,
            title: Some(traj_title),
            created: None,
            kind: Some("trajectory".to_string()),
            score: Some(traj_match.best_score),
        });
    }

    tracing::info!(
        "memories_search: found {} knowledge + {} trajectories",
        records
            .iter()
            .filter(|r| r.kind.as_deref() != Some("trajectory"))
            .count(),
        records
            .iter()
            .filter(|r| r.kind.as_deref() == Some("trajectory"))
            .count()
    );

    if !records.is_empty() {
        return Ok(records);
    }

    memories_search_fallback(
        gcx,
        query,
        top_n_memories,
        &knowledge_dirs,
        exclude_root.as_deref(),
    )
    .await
}

async fn memories_search_fallback(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n: usize,
    knowledge_dirs: &[PathBuf],
    exclude_root: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    let query_lower = query.to_lowercase();
    const FALLBACK_STOP_WORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "into", "about", "what", "where",
        "when", "why", "how", "can", "could", "should", "would", "please", "find", "search",
        "look", "need", "want", "have", "does", "doesn", "work",
    ];
    let mut seen_query_words = std::collections::HashSet::new();
    let query_words: Vec<&str> = query_lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|w| w.trim_matches('-'))
        .filter(|w| w.len() >= 3)
        .filter(|w| !FALLBACK_STOP_WORDS.contains(w))
        .filter(|w| seen_query_words.insert((*w).to_string()))
        .collect();
    let mut scored_results: Vec<(usize, MemoRecord)> = Vec::new();

    if knowledge_dirs.is_empty() || query_words.is_empty() {
        return Ok(vec![]);
    }

    for knowledge_dir in knowledge_dirs {
        if !knowledge_dir.exists() {
            continue;
        }
        for entry in WalkDir::new(knowledge_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path_contains_component(path, "archive") {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }

            let text =
                match get_file_text_from_memory_or_disk(gcx.clone(), &path.to_path_buf()).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };

            let text_lower = text.to_lowercase();
            let score: usize = query_words
                .iter()
                .filter(|w| text_lower.contains(*w))
                .count();
            if score == 0 {
                continue;
            }

            let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }

            if let (Some(ex_root), Some(ref source_id)) =
                (exclude_root, &frontmatter.source_chat_id)
            {
                if source_id == ex_root {
                    tracing::debug!(
                        "Fallback: excluding knowledge created by current chat: {:?}",
                        path
                    );
                    continue;
                }
            }

            let id = frontmatter
                .id
                .clone()
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let content_preview: String = text[content_start..].chars().take(500).collect();

            let normalized_score = if score >= 2 {
                (score as f32 / 2.0).min(1.0)
            } else {
                0.70
            };

            scored_results.push((
                score,
                MemoRecord {
                    memid: id,
                    tags: frontmatter.tags,
                    content: content_preview,
                    file_path: Some(path.to_path_buf()),
                    line_range: None,
                    title: frontmatter.title,
                    created: frontmatter.created,
                    kind: frontmatter.kind,
                    score: Some(normalized_score),
                },
            ));
        }
    }

    scored_results.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(scored_results
        .into_iter()
        .take(top_n)
        .map(|(_, r)| r)
        .collect())
}

pub async fn deprecate_document(
    gcx: Arc<GlobalContext>,
    doc_path: &PathBuf,
    superseded_by: Option<&str>,
    _reason: &str,
) -> Result<(), String> {
    update_memory_document_frontmatter(gcx, doc_path, |frontmatter| {
        frontmatter.status = Some("deprecated".to_string());
        frontmatter.deprecated_at = Some(Local::now().format("%Y-%m-%d").to_string());
        frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());
        if let Some(superseded_by) = superseded_by {
            frontmatter.superseded_by = Some(superseded_by.to_string());
        }
        Ok(true)
    })
    .await
    .map(|_| ())
}

pub async fn update_memory_document_frontmatter<F>(
    gcx: Arc<GlobalContext>,
    doc_path: &Path,
    update: F,
) -> Result<bool, String>
where
    F: FnOnce(&mut KnowledgeFrontmatter) -> Result<bool, String>,
{
    let existing_text = fs::read_to_string(doc_path)
        .await
        .map_err(|e| format!("Failed to read memory file: {}", e))?;
    let (mut frontmatter, content_start) = KnowledgeFrontmatter::parse(&existing_text);
    if content_start == 0 && existing_text.starts_with("---") {
        return Err("Failed to parse memory frontmatter".to_string());
    }
    let body = existing_text.get(content_start..).unwrap_or("").to_string();
    if !update(&mut frontmatter)? {
        return Ok(false);
    }
    rewrite_memory_document(gcx, doc_path, &frontmatter, &body)
        .await
        .map(|_| true)
}

pub async fn rewrite_memory_document(
    gcx: Arc<GlobalContext>,
    doc_path: &Path,
    frontmatter: &KnowledgeFrontmatter,
    body: &str,
) -> Result<(), String> {
    let content = format!("{}\n\n{}", frontmatter.to_yaml(), body.trim());
    atomic_write_text(doc_path, &content).await?;

    let path_buf = doc_path.to_path_buf();
    {
        let gcx_read = gcx.clone();
        let mut idx = gcx_read.knowledge_index.lock().await;
        idx.remove_path(&path_buf);
        if !frontmatter.is_archived() && !frontmatter.is_deprecated() {
            idx.add_from_frontmatter(path_buf.clone(), frontmatter, Some(body));
        }
    }

    let vec_db = gcx.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        if frontmatter.is_archived() || frontmatter.is_deprecated() {
            let _ = vecdb.remove_file(&path_buf).await;
        } else {
            vecdb
                .vectorizer_enqueue_files(&vec![path_buf.to_string_lossy().to_string()], true)
                .await;
        }
    }

    gcx.documents_state
        .memory_document_map
        .lock()
        .await
        .remove(&path_buf);

    Ok(())
}

async fn atomic_write_text(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid memory path: missing parent".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Invalid memory path: missing file name".to_string())?;
    let tmp_path = parent.join(format!(".{}.tmp-{}", file_name, Uuid::new_v4()));
    let write_result = async {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .await
            .map_err(|e| format!("Failed to create temporary memory file: {}", e))?;
        file.write_all(content.as_bytes())
            .await
            .map_err(|e| format!("Failed to write temporary memory file: {}", e))?;
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush temporary memory file: {}", e))?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)
                .await
                .map_err(|e| format!("Failed to replace memory file: {}", e))?;
        }
        fs::rename(&tmp_path, path)
            .await
            .map_err(|e| format!("Failed to replace memory file: {}", e))
    }
    .await;

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path).await;
    }
    write_result
}

pub async fn delete_document_from_disk(
    gcx: Arc<GlobalContext>,
    doc_path: &PathBuf,
) -> Result<(), String> {
    match fs::remove_file(doc_path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Failed to delete document: {}", e)),
    }

    info!("Deleted document from disk: {}", doc_path.display());

    let vec_db = gcx.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        let _ = vecdb.remove_file(doc_path).await;
    }

    gcx.documents_state
        .memory_document_map
        .lock()
        .await
        .remove(doc_path);

    Ok(())
}

pub fn extract_entities(content: &str) -> Vec<String> {
    let backtick_re =
        Regex::new(r"`([a-zA-Z_][a-zA-Z0-9_:]*(?:::[a-zA-Z_][a-zA-Z0-9_]*)*)`").unwrap();
    backtick_re
        .captures_iter(content)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .filter(|e| e.len() >= 3 && e.len() <= 100)
        .collect()
}

pub fn extract_file_paths(content: &str) -> Vec<String> {
    let path_re =
        Regex::new(r"(?:^|[\s`])((?:[a-zA-Z0-9_-]+/)+[a-zA-Z0-9_-]+\.[a-zA-Z0-9]+)").unwrap();
    path_re
        .captures_iter(content)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect()
}

pub struct EnrichmentParams {
    pub base_tags: Vec<String>,
    pub base_filenames: Vec<String>,
    pub base_kind: String,
    pub base_title: Option<String>,
    pub source_chat_id: Option<String>,
}

pub async fn memories_add_enriched(
    ccx: Arc<AMutex<AtCommandsContext>>,
    content: &str,
    params: EnrichmentParams,
) -> Result<PathBuf, String> {
    let gcx = ccx.lock().await.global_context.clone();

    let entities = extract_entities(content);
    let detected_paths = extract_file_paths(content);

    let kg = build_knowledge_graph(gcx.clone()).await;

    let candidate_files: Vec<String> = {
        let mut files = params.base_filenames.clone();
        files.extend(detected_paths.clone());
        files.into_iter().take(30).collect()
    };

    let candidate_docs: Vec<(String, String)> = kg
        .active_docs()
        .take(20)
        .map(|d| {
            let id = d
                .frontmatter
                .id
                .clone()
                .unwrap_or_else(|| d.path.to_string_lossy().to_string());
            let title = d
                .frontmatter
                .title
                .clone()
                .unwrap_or_else(|| "Untitled".to_string());
            (id, title)
        })
        .collect();

    let enrichment = enrich_knowledge_metadata(
        gcx.clone(),
        content,
        &entities,
        &candidate_files,
        &candidate_docs,
    )
    .await;

    let (final_title, final_tags, final_filenames, final_kind, final_links, review_days) =
        match enrichment {
            Ok(e) => {
                let mut tags = params.base_tags.clone();
                tags.extend(e.tags);
                tags.extend(extract_fallback_tags(content, &detected_paths, &entities));
                let tags = normalize_memory_tags(&tags, 16);

                let mut files = params.base_filenames.clone();
                files.extend(e.filenames);
                files.sort();
                files.dedup();

                let kind = e.kind.unwrap_or_else(|| params.base_kind.clone());

                (
                    e.title.or(params.base_title.clone()).or_else(|| {
                        content
                            .lines()
                            .next()
                            .map(|l| l.trim_start_matches('#').trim().to_string())
                    }),
                    if tags.is_empty() {
                        let mut fallback = vec![params.base_kind.clone()];
                        fallback.extend(extract_fallback_tags(content, &detected_paths, &entities));
                        normalize_memory_tags(&fallback, 16)
                    } else {
                        tags
                    },
                    files,
                    kind,
                    e.links,
                    e.review_after_days.unwrap_or(90),
                )
            }
            Err(e) => {
                warn!(
                    "Enrichment failed, using defaults with fallback tags: {}",
                    e
                );
                let mut tags = params.base_tags.clone();

                let fallback_tags = extract_fallback_tags(content, &detected_paths, &entities);
                tags.extend(fallback_tags);

                if tags.is_empty() {
                    tags.push(params.base_kind.clone());
                }

                let tags = normalize_memory_tags(&tags, 16);

                (
                    params.base_title.or_else(|| {
                        content
                            .lines()
                            .next()
                            .map(|l| l.trim_start_matches('#').trim().to_string())
                    }),
                    tags,
                    params.base_filenames,
                    params.base_kind,
                    vec![],
                    90,
                )
            }
        };

    let now = Local::now();
    let content_hash = compute_content_hash_hex(content);
    let summary = first_nonempty_line(content);

    // Optional “short description”: prefer second non-empty line if present,
    // otherwise fall back to summary.
    let description = {
        let nonempty: Vec<&str> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        if nonempty.len() >= 2 {
            Some(nonempty[1].trim_start_matches('#').trim().to_string())
        } else {
            summary.clone()
        }
    };

    // “Related” fields are intended for fast, cheap retrieval.
    // For now we populate them from detected paths/entities as best-effort.
    let related_files: Vec<String> = {
        let mut files = detected_paths.clone();
        files.sort();
        files.dedup();
        files.into_iter().take(50).collect()
    };
    let related_entities: Vec<String> = {
        let mut ents = entities.clone();
        ents.sort();
        ents.dedup();
        ents.into_iter().take(50).collect()
    };
    let frontmatter = KnowledgeFrontmatter {
        id: Some(Uuid::new_v4().to_string()),
        title: final_title.clone(),
        tags: final_tags.clone(),
        created: Some(now.format("%Y-%m-%d").to_string()),
        updated: Some(now.format("%Y-%m-%d").to_string()),
        filenames: final_filenames.clone(),
        links: final_links,
        kind: Some(final_kind),
        status: Some("active".to_string()),
        superseded_by: None,
        deprecated_at: None,
        review_after: Some(
            (now + Duration::days(review_days))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        source_chat_id: params.source_chat_id.clone(),

        created_at: Some(Utc::now().to_rfc3339()),
        summary,
        description,
        entities: entities.clone(),
        related_files,
        related_entities,
        content_hash: Some(content_hash),
        source_tool: Some("memories_add_enriched".to_string()),
        source_confidence: None,
        source_trajectory_id: None,
        source_message_range: None,
        source_commit: None,
        topic: None,
        last_used_at: None,
        use_count: 0,
        last_injected_at: None,
        dismissed_count: 0,
        source_id: None,
        source_content_hash: None,
        review_needed: None,
    };

    let file_path = memories_add(gcx.clone(), &frontmatter, content).await?;
    let new_doc_id = frontmatter.id.clone().unwrap();

    let mut updated_frontmatter = frontmatter.clone();
    if let Err(e) =
        auto_link_memory(gcx.clone(), &mut updated_frontmatter, content, &file_path).await
    {
        warn!("Auto-linking failed for new memory: {}", e);
    } else if updated_frontmatter.links != frontmatter.links {
        let md_content = format!("{}\n\n{}", updated_frontmatter.to_yaml(), content);
        if let Err(e) = fs::write(&file_path, &md_content).await {
            warn!("Failed to update memory with auto-links: {}", e);
        } else {
            info!(
                "Auto-linked {} docs to {}",
                updated_frontmatter.links.len(),
                file_path.display()
            );
        }
    }

    let deprecation_candidates =
        kg.get_deprecation_candidates(&final_tags, &final_filenames, &entities, Some(&new_doc_id));

    if !deprecation_candidates.is_empty() {
        let snippet: String = content.chars().take(500).collect();

        match check_deprecation(
            gcx.clone(),
            final_title.as_deref().unwrap_or("Untitled"),
            &final_tags,
            &final_filenames,
            &snippet,
            &deprecation_candidates,
        )
        .await
        {
            Ok(result) => {
                for decision in result.deprecate {
                    if decision.confidence >= 0.75 {
                        if let Some(doc) = kg.get_doc_by_id(&decision.target_id) {
                            if let Err(e) = deprecate_document(
                                gcx.clone(),
                                &doc.path,
                                Some(&new_doc_id),
                                &decision.reason,
                            )
                            .await
                            {
                                warn!("Failed to deprecate {}: {}", decision.target_id, e);
                            } else {
                                info!(
                                    "Deprecated {} (confidence: {:.2}): {}",
                                    decision.target_id, decision.confidence, decision.reason
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Deprecation check failed: {}", e);
            }
        }
    }

    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_statement_validation_rejects_low_confidence_vague_and_sensitive() {
        assert!(preference_statement_is_safe(
            "I prefer concise answers with bullet lists",
            0.90
        ));
        assert!(!preference_statement_is_safe(
            "I prefer concise answers with bullet lists",
            0.80
        ));
        assert!(!preference_statement_is_safe("remember this", 0.95));
        assert!(!preference_statement_is_safe("I prefer token=secret", 0.95));
    }

    #[test]
    fn normalized_preference_dedupe_matches_case_and_punctuation() {
        let first = normalize_preference_text_for_dedupe("I prefer concise answers, with bullets!");
        let second = normalize_preference_text_for_dedupe("i prefer concise answers with bullets");

        assert_eq!(first, second);
    }

    #[test]
    fn memory_tags_are_namespaced_and_normalized() {
        let tags = normalize_memory_tags(
            &vec![
                "Rust".to_string(),
                "behavior_learner".to_string(),
                " Playwright MCP ".to_string(),
                "component:Buddy Scheduler".to_string(),
                "Rust".to_string(),
            ],
            16,
        );

        assert_eq!(
            tags,
            vec![
                "language:rust".to_string(),
                "workflow:behavior-learner".to_string(),
                "tool:playwright-mcp".to_string(),
                "component:buddy-scheduler".to_string(),
            ]
        );
    }

    #[test]
    fn fallback_tags_extract_precise_kg_linking_signals() {
        let tags = extract_fallback_tags(
            "Buddy autonomous background chat should reuse subchat and save_trajectory_as. Run cargo check and git status.",
            &vec![
                "refact-agent/engine/src/buddy/jobs.rs".to_string(),
                "refact-agent/engine/src/subchat.rs".to_string(),
            ],
            &vec!["save_trajectory_as".to_string()],
        );

        assert!(tags.contains(&"domain:buddy".to_string()));
        assert!(tags.contains(&"workflow:background-chat".to_string()));
        assert!(tags.contains(&"component:subchat".to_string()));
        assert!(tags.contains(&"component:trajectory-persistence".to_string()));
        assert!(tags.contains(&"component:buddy".to_string()));
        assert!(tags.contains(&"language:rust".to_string()));
        assert!(tags.contains(&"verification:cargo-check".to_string()));
        assert!(tags.contains(&"verification:git-sanity".to_string()));
        assert!(tags.contains(&"entity:save_trajectory_as".to_string()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_add_preference_if_new_dedupes_existing_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let first = memories_add_preference_if_new(
            gcx.clone(),
            "I prefer concise answers with bullet lists",
            "User-authored snippet",
            0.91,
        )
        .await
        .unwrap();
        let duplicate = memories_add_preference_if_new(
            gcx,
            "i prefer concise answers with bullet lists.",
            "Repeated user-authored snippet",
            0.95,
        )
        .await
        .unwrap();

        assert!(first.is_some());
        assert!(duplicate.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deprecate_document_marks_inactive_without_deleting_file() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("memory.md");
        let mut frontmatter = create_frontmatter(
            Some("Memory"),
            &vec!["old".to_string()],
            &Vec::new(),
            &Vec::new(),
            "domain",
        );
        frontmatter.id = Some("old-doc".to_string());
        let body = "Original body";
        tokio::fs::write(&path, format!("{}\n\n{}", frontmatter.to_yaml(), body))
            .await
            .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        deprecate_document(
            gcx.clone(),
            &path,
            Some("new-doc"),
            "superseded by canonical memory",
        )
        .await
        .unwrap();

        assert!(path.exists());
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let (updated, content_start) = KnowledgeFrontmatter::parse(&text);
        assert_eq!(updated.status.as_deref(), Some("deprecated"));
        assert_eq!(updated.superseded_by.as_deref(), Some("new-doc"));
        assert_eq!(text[content_start..].trim(), body);
        let found = load_memories_by_tags(gcx, &["old"], 10).await.unwrap();
        assert!(found.is_empty());
    }
}
