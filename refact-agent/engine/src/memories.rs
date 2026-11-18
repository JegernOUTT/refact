use std::path::{PathBuf, Path};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use chrono::{Local, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tokio::sync::Semaphore;
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
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use crate::knowledge_index::{KnowledgeIndex, KnowledgeSearchFilters};
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use crate::knowledge_graph::kg_subchat::{enrich_knowledge_metadata, check_deprecation};
use crate::knowledge_graph::build_knowledge_graph;
use crate::http::routers::v1::knowledge_ops::auto_link_memory;

pub(crate) const MAX_CONCURRENT_ENRICHMENT_SEARCHES: usize = 8;
static ENRICHMENT_SEARCH_PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
const ROOT_CHAT_MEMO_CAPACITY: usize = 8_192;
static ROOT_CHAT_MEMO: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

static MEMORY_MUTATION_LOCK: AMutex<()> = AMutex::const_new(());

tokio::task_local! {
    static MEMORY_MUTATION_HELD: ();
}

pub async fn with_memory_mutation_lock<F: std::future::Future>(future: F) -> F::Output {
    if MEMORY_MUTATION_HELD.try_with(|_| ()).is_ok() {
        return future.await;
    }
    let _guard = MEMORY_MUTATION_LOCK.lock().await;
    MEMORY_MUTATION_HELD.scope((), future).await
}

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
    pub score: Option<f32>,
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

fn root_chat_memo() -> &'static Mutex<HashMap<String, String>> {
    ROOT_CHAT_MEMO.get_or_init(|| Mutex::new(HashMap::new()))
}

fn root_chat_memo_get(chat_id: &str) -> Option<String> {
    let memo = match root_chat_memo().lock() {
        Ok(memo) => memo,
        Err(poisoned) => poisoned.into_inner(),
    };
    memo.get(chat_id).cloned()
}

fn insert_bounded_root_chat_memo(
    memo: &mut HashMap<String, String>,
    chat_id: String,
    root_chat_id: String,
) {
    if memo.len() >= ROOT_CHAT_MEMO_CAPACITY && !memo.contains_key(&chat_id) {
        memo.clear();
    }
    memo.insert(chat_id, root_chat_id);
}

fn root_chat_memo_insert_many(chat_ids: impl IntoIterator<Item = String>, root_chat_id: &str) {
    let mut memo = match root_chat_memo().lock() {
        Ok(memo) => memo,
        Err(poisoned) => poisoned.into_inner(),
    };
    for chat_id in chat_ids {
        insert_bounded_root_chat_memo(&mut memo, chat_id, root_chat_id.to_string());
    }
}

async fn resolve_root_chat_id(
    gcx: Arc<GlobalContext>,
    start_id: &str,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(root) = root_chat_memo_get(start_id) {
        cache.insert(start_id.to_string(), root.clone());
        return root;
    }
    if let Some(r) = cache.get(start_id) {
        return r.clone();
    }

    let mut seen = std::collections::HashSet::new();
    let mut traversed = Vec::new();
    let mut current = start_id.to_string();

    for _ in 0..50 {
        if !seen.insert(current.clone()) {
            let root = start_id.to_string();
            cache.insert(start_id.to_string(), root.clone());
            root_chat_memo_insert_many(std::iter::once(start_id.to_string()), &root);
            return root;
        }
        traversed.push(current.clone());

        if let Some(root) = root_chat_memo_get(&current).or_else(|| cache.get(&current).cloned()) {
            for chat_id in &traversed {
                cache.insert(chat_id.clone(), root.clone());
            }
            root_chat_memo_insert_many(traversed, &root);
            return root;
        }

        match crate::chat::trajectories::find_trajectory_parent_id(gcx.clone(), &current).await {
            Some(Some(parent)) => current = parent,
            Some(None) => {
                let root = current;
                for chat_id in &traversed {
                    cache.insert(chat_id.clone(), root.clone());
                }
                root_chat_memo_insert_many(traversed, &root);
                return root;
            }
            None => {
                for chat_id in &traversed {
                    cache.insert(chat_id.clone(), current.clone());
                }
                traversed.pop();
                root_chat_memo_insert_many(traversed, &current);
                return current;
            }
        }
    }

    let root = start_id.to_string();
    cache.insert(start_id.to_string(), root.clone());
    root_chat_memo_insert_many(std::iter::once(start_id.to_string()), &root);
    root
}

pub async fn enrichment_root_id(gcx: Arc<GlobalContext>, chat_id: &str) -> String {
    let mut cache = HashMap::new();
    resolve_root_chat_id(gcx, chat_id, &mut cache).await
}

pub async fn enrichment_current_root_id(
    gcx: Arc<GlobalContext>,
    current_chat_id: Option<&str>,
) -> Option<String> {
    let current_chat_id = current_chat_id?;
    Some(enrichment_root_id(gcx, current_chat_id).await)
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
        occurrences: 0,
        signal_key: None,
        last_observed: None,
        extra: Default::default(),
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
    with_memory_mutation_lock(async {
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
        gcx.enrichment_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        gcx.knowledge_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        info!("Created knowledge entry: {}", file_path.display());

        // Update fast in-memory knowledge index (best-effort, new docs going forward).
        {
            let gcx_read = gcx.clone();
            let mut idx = gcx_read.knowledge_index.lock().await;
            idx.add_signature(
                refact_buddy_core::memory_dedup::content_signature(content),
                file_path.clone(),
            );
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut idx,
                file_path.clone(),
                frontmatter,
                Some(content),
            );
        }

        Ok(file_path)
    })
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryWriteOutcome {
    Created(PathBuf),
    DuplicateTouched(PathBuf),
}

impl MemoryWriteOutcome {
    pub fn path(&self) -> &PathBuf {
        match self {
            MemoryWriteOutcome::Created(path) => path,
            MemoryWriteOutcome::DuplicateTouched(path) => path,
        }
    }

    pub fn created(&self) -> bool {
        matches!(self, MemoryWriteOutcome::Created(_))
    }
}

pub async fn touch_memory_observation(
    gcx: Arc<GlobalContext>,
    doc_path: &Path,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    update_memory_document_frontmatter(gcx, doc_path, |frontmatter| {
        frontmatter.occurrences = frontmatter.occurrences.saturating_add(1).max(2);
        frontmatter.last_observed = Some(now);
        Ok(true)
    })
    .await
    .map(|_| ())
}

const NEAR_DUPLICATE_CANDIDATE_CAP: usize = 64;

async fn evict_stale_index_path(gcx: Arc<GlobalContext>, path: &Path) {
    warn!(
        "memories: knowledge index pointed at missing file {}, evicting stale entry",
        path.display()
    );
    let mut idx = gcx.knowledge_index.lock().await;
    idx.remove_path(path);
}

async fn find_duplicate_memory(
    gcx: Arc<GlobalContext>,
    frontmatter: &KnowledgeFrontmatter,
    content: &str,
) -> Option<PathBuf> {
    loop {
        let candidate = {
            let idx = gcx.knowledge_index.lock().await;
            find_duplicate_in_index(&idx, frontmatter, content)
        };
        let path = candidate?;
        // A local create must never touch a matching global/other-project memory.
        let root = get_first_knowledge_dir(gcx.clone()).await.ok()?;
        let canonical_root = tokio::fs::canonicalize(&root).await.ok()?;
        let canonical_root = dunce::simplified(&canonical_root);
        if !path.starts_with(&root) {
            return None;
        }
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            let canonical = tokio::fs::canonicalize(&path).await.ok()?;
            let canonical = dunce::simplified(&canonical);
            if canonical != dunce::simplified(&path) || !canonical.starts_with(canonical_root) {
                return None;
            }
            return Some(path);
        }
        evict_stale_index_path(gcx.clone(), &path).await;
    }
}

fn find_duplicate_in_index(
    idx: &KnowledgeIndex,
    frontmatter: &KnowledgeFrontmatter,
    content: &str,
) -> Option<PathBuf> {
    let signature = refact_buddy_core::memory_dedup::content_signature(content);
    if let Some(path) = idx.first_path_for_signature(&signature) {
        return Some(path.clone());
    }

    let mut seen = HashSet::new();
    let mut checked = 0usize;
    for tag in &frontmatter.tags {
        for card in idx.related_for_tags(std::slice::from_ref(tag), NEAR_DUPLICATE_CANDIDATE_CAP) {
            if checked >= NEAR_DUPLICATE_CANDIDATE_CAP {
                return None;
            }
            if !seen.insert(card.file_path.clone()) {
                continue;
            }
            let Some(existing_content) = idx.content_for_path(&card.file_path) else {
                continue;
            };
            checked += 1;
            if refact_buddy_core::memory_dedup::near_duplicate(content, existing_content) {
                return Some(card.file_path.clone());
            }
        }
    }
    None
}

pub async fn memories_add_dedup(
    gcx: Arc<GlobalContext>,
    frontmatter: &KnowledgeFrontmatter,
    content: &str,
) -> Result<MemoryWriteOutcome, String> {
    with_memory_mutation_lock(async {
        if let Some(existing) = find_duplicate_memory(gcx.clone(), frontmatter, content).await {
            info!(
                "memories: duplicate content detected, touching {} instead of creating a new file",
                existing.display()
            );
            match touch_memory_observation(gcx.clone(), &existing).await {
                Ok(()) => return Ok(MemoryWriteOutcome::DuplicateTouched(existing)),
                Err(err) => {
                    warn!(
                        "memories: failed to touch duplicate {}: {}, creating instead",
                        existing.display(),
                        err
                    );
                    evict_stale_index_path(gcx.clone(), &existing).await;
                }
            }
        }
        memories_add(gcx, frontmatter, content)
            .await
            .map(MemoryWriteOutcome::Created)
    })
    .await
}

pub async fn memories_upsert_by_signal_key(
    gcx: Arc<GlobalContext>,
    signal_key: &str,
    frontmatter: &KnowledgeFrontmatter,
    content: &str,
) -> Result<MemoryWriteOutcome, String> {
    with_memory_mutation_lock(async {
        let signal_key = signal_key.trim();
        if signal_key.is_empty() {
            return memories_add_dedup(gcx, frontmatter, content).await;
        }
        loop {
            let existing = {
                let idx = gcx.knowledge_index.lock().await;
                idx.first_path_for_signal_key(signal_key).cloned()
            };
            let Some(doc_path) = existing else {
                break;
            };
            let existing_text = match fs::read_to_string(&doc_path).await {
                Ok(text) => text,
                Err(err) => {
                    warn!(
                        "memories: failed to read signal doc {}: {}",
                        doc_path.display(),
                        err
                    );
                    evict_stale_index_path(gcx.clone(), &doc_path).await;
                    continue;
                }
            };
            let (mut existing_frontmatter, _) = KnowledgeFrontmatter::parse(&existing_text);
            existing_frontmatter.occurrences =
                existing_frontmatter.occurrences.saturating_add(1).max(2);
            existing_frontmatter.last_observed = Some(Utc::now().to_rfc3339());
            existing_frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());
            existing_frontmatter.signal_key = Some(signal_key.to_string());
            if let Some(title) = &frontmatter.title {
                existing_frontmatter.title = Some(title.clone());
            }
            if let Some(kind) = &frontmatter.kind {
                existing_frontmatter.kind = Some(kind.clone());
            }
            for tag in &frontmatter.tags {
                if !existing_frontmatter.tags.contains(tag) {
                    existing_frontmatter.tags.push(tag.clone());
                }
            }
            for filename in &frontmatter.filenames {
                if !existing_frontmatter.filenames.contains(filename) {
                    existing_frontmatter.filenames.push(filename.clone());
                }
            }
            existing_frontmatter.content_hash = Some(compute_content_hash_hex(content));
            rewrite_memory_document(gcx, &doc_path, &existing_frontmatter, content).await?;
            info!(
                "memories: upserted signal '{}' in place at {}",
                signal_key,
                doc_path.display()
            );
            return Ok(MemoryWriteOutcome::DuplicateTouched(doc_path));
        }
        let mut stamped = frontmatter.clone();
        stamped.signal_key = Some(signal_key.to_string());
        stamped.occurrences = stamped.occurrences.max(1);
        stamped.last_observed = Some(Utc::now().to_rfc3339());
        let outcome = memories_add_dedup(gcx.clone(), &stamped, content).await?;
        if let MemoryWriteOutcome::DuplicateTouched(path) = &outcome {
            let key = signal_key.to_string();
            let stamp = update_memory_document_frontmatter(gcx.clone(), path, move |fm| {
                if fm.signal_key.as_deref() == Some(key.as_str()) {
                    return Ok(false);
                }
                fm.signal_key = Some(key);
                Ok(true)
            })
            .await;
            if let Err(err) = stamp {
                warn!(
                    "memories: failed to stamp signal key on deduped doc {}: {}",
                    path.display(),
                    err
                );
            }
        }
        Ok(outcome)
    })
    .await
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
    with_memory_mutation_lock(async {
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
        let mut frontmatter =
            create_frontmatter(Some(&statement), &tags, &empty, &empty, "preference");
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
    })
    .await
}

pub async fn memories_search(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n_memories: usize,
    exclude_trajectory_id: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    memories_search_inner(gcx, query, top_n_memories, exclude_trajectory_id, None).await
}

pub async fn memo_record_visible_to_memory_reader(
    memory: &MemoRecord,
    reader: &crate::tools::tool_task_memory::MemoryAccessContext,
) -> bool {
    if !crate::tools::tool_task_memory::memory_tags_visible_to_reader(&memory.tags, reader) {
        return false;
    }
    let Some(path) = memory.file_path.as_ref() else {
        return true;
    };
    let Ok(content) = fs::read_to_string(path).await else {
        return true;
    };
    let (frontmatter, _) = KnowledgeFrontmatter::parse(&content);
    let namespace = frontmatter
        .extra
        .get("namespace")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("task");
    crate::tools::tool_task_memory::memory_namespace_visible_to_reader(namespace, reader)
}

#[cfg(test)]
mod related_memory_visibility_tests {
    use super::*;
    use crate::tools::tool_task_memory::{related_memory_access_context, MemoryAccessContext};

    #[tokio::test]
    async fn memo_records_hide_agent_namespaces_from_foreign_or_missing_task_meta() {
        let memory = MemoRecord {
            tags: vec!["namespace:agent:A".to_string()],
            ..Default::default()
        };
        let owner = related_memory_access_context(Some(&crate::chat::types::TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("A".to_string()),
            ..Default::default()
        }));
        let foreign = related_memory_access_context(Some(&crate::chat::types::TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("B".to_string()),
            ..Default::default()
        }));

        assert!(memo_record_visible_to_memory_reader(&memory, &owner).await);
        assert!(!memo_record_visible_to_memory_reader(&memory, &foreign).await);
        assert!(
            !memo_record_visible_to_memory_reader(&memory, &related_memory_access_context(None))
                .await
        );
        assert!(
            !memo_record_visible_to_memory_reader(
                &MemoRecord {
                    tags: vec!["namespace:agent:".to_string()],
                    ..Default::default()
                },
                &MemoryAccessContext::default()
            )
            .await
        );
        for tag in ["namespace:task", "namespace:card:T-1"] {
            assert!(
                memo_record_visible_to_memory_reader(
                    &MemoRecord {
                        tags: vec![tag.to_string()],
                        ..Default::default()
                    },
                    &MemoryAccessContext::default()
                )
                .await
            );
        }
        assert!(
            memo_record_visible_to_memory_reader(
                &MemoRecord::default(),
                &MemoryAccessContext::default()
            )
            .await
        );
    }
}

pub async fn memories_search_for_enrichment(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n_memories: usize,
    exclude_trajectory_id: Option<&str>,
    diagnostic_chat_id: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    memories_search_inner(
        gcx,
        query,
        top_n_memories,
        exclude_trajectory_id,
        diagnostic_chat_id,
    )
    .await
}

async fn memories_search_inner(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n_memories: usize,
    exclude_trajectory_id: Option<&str>,
    diagnostic_chat_id: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    let root_discovery_started = perf_diagnostics::is_enabled().then(Instant::now);
    let knowledge_dirs = get_all_knowledge_dirs(gcx.clone()).await;
    record_enrichment_metric(
        diagnostic_chat_id,
        PerfComponent::EnrichmentRootDiscovery,
        PerfOutcome::Success,
        elapsed_us(root_discovery_started),
        None,
        Some(knowledge_dirs.len() as u64),
    );

    let mut root_cache: HashMap<String, String> = HashMap::new();
    let root_resolve_started = perf_diagnostics::is_enabled().then(Instant::now);
    let exclude_root = match exclude_trajectory_id {
        Some(id) => Some(resolve_root_chat_id(gcx.clone(), id, &mut root_cache).await),
        None => None,
    };
    record_enrichment_metric(
        diagnostic_chat_id,
        PerfComponent::EnrichmentCurrentRootResolve,
        PerfOutcome::Success,
        elapsed_us(root_resolve_started),
        None,
        Some(usize::from(exclude_root.is_some()) as u64),
    );

    let _enrichment_permit = enrichment_search_semaphore()
        .acquire_owned()
        .await
        .map_err(|_| "knowledge enrichment search is shutting down".to_string())?;
    memories_search_lexical(
        gcx,
        query,
        top_n_memories,
        &knowledge_dirs,
        exclude_root.as_deref(),
        diagnostic_chat_id,
    )
    .await
}

async fn memories_search_lexical(
    gcx: Arc<GlobalContext>,
    query: &str,
    top_n: usize,
    knowledge_dirs: &[PathBuf],
    exclude_root: Option<&str>,
    diagnostic_chat_id: Option<&str>,
) -> Result<Vec<MemoRecord>, String> {
    let lexical_started = perf_diagnostics::is_enabled().then(Instant::now);
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
    if top_n == 0 || knowledge_dirs.is_empty() || query_words.is_empty() {
        record_enrichment_metric(
            diagnostic_chat_id,
            PerfComponent::EnrichmentLexical,
            PerfOutcome::Skipped,
            elapsed_us(lexical_started),
            Some(0),
            Some(0),
        );
        return Ok(Vec::new());
    }

    let candidate_cap = top_n.saturating_mul(4).clamp(1, 32);
    let (records, index_available) = {
        let index = gcx.knowledge_index.lock().await;
        let index_available = index.is_ready();
        let records = if index_available {
            index
                .search_bounded(
                    query,
                    &KnowledgeSearchFilters::default(),
                    candidate_cap,
                    candidate_cap,
                )
                .into_iter()
                .filter(|hit| {
                    knowledge_dirs
                        .iter()
                        .any(|dir| hit.card.file_path.starts_with(dir))
                })
                .filter(|hit| {
                    exclude_root.is_none_or(|root| {
                        index.source_chat_id_for_path(&hit.card.file_path) != Some(root)
                    })
                })
                .take(top_n)
                .map(|hit| {
                    let content = index
                        .content_for_path(&hit.card.file_path)
                        .unwrap_or_default()
                        .chars()
                        .take(500)
                        .collect();
                    MemoRecord {
                        memid: hit.card.id,
                        tags: hit.card.tags,
                        content,
                        file_path: Some(hit.card.file_path),
                        line_range: None,
                        title: Some(hit.card.title),
                        created: hit.card.created,
                        kind: hit.card.kind,
                        score: Some((hit.score / 2.0).clamp(0.0, 1.0)),
                    }
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        (records, index_available)
    };
    if !index_available {
        warn!("memories: knowledge index unavailable; returning no knowledge candidates");
    }
    record_enrichment_metric(
        diagnostic_chat_id,
        PerfComponent::EnrichmentLexical,
        if index_available {
            PerfOutcome::Success
        } else {
            PerfOutcome::Skipped
        },
        elapsed_us(lexical_started),
        Some(0),
        Some(0),
    );
    Ok(records)
}

fn record_enrichment_metric(
    diagnostic_chat_id: Option<&str>,
    component: PerfComponent,
    outcome: PerfOutcome,
    elapsed_us: u64,
    size_bytes: Option<u64>,
    item_count: Option<u64>,
) {
    if let Some(chat_id) = diagnostic_chat_id {
        perf_diagnostics::record_enrichment(
            component, chat_id, outcome, elapsed_us, size_bytes, item_count, None,
        );
    }
}

fn elapsed_us(started: Option<Instant>) -> u64 {
    started
        .map(|started| started.elapsed().as_micros().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn enrichment_search_semaphore() -> Arc<Semaphore> {
    ENRICHMENT_SEARCH_PERMITS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_ENRICHMENT_SEARCHES)))
        .clone()
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
    with_memory_mutation_lock(async {
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
    })
    .await
}

pub async fn rewrite_memory_document(
    gcx: Arc<GlobalContext>,
    doc_path: &Path,
    frontmatter: &KnowledgeFrontmatter,
    body: &str,
) -> Result<(), String> {
    with_memory_mutation_lock(async {
        let content = format!("{}\n\n{}", frontmatter.to_yaml(), body.trim());
        atomic_write_text(doc_path, &content).await?;
        gcx.enrichment_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        gcx.knowledge_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let path_buf = doc_path.to_path_buf();
        {
            let gcx_read = gcx.clone();
            let mut idx = gcx_read.knowledge_index.lock().await;
            idx.remove_path(&path_buf);
            if !frontmatter.is_archived() && !frontmatter.is_deprecated() {
                idx.add_signature(
                    refact_buddy_core::memory_dedup::content_signature(body),
                    path_buf.clone(),
                );
                crate::knowledge_index::add_knowledge_frontmatter_to_index(
                    &mut idx,
                    path_buf.clone(),
                    frontmatter,
                    Some(body),
                );
            }
        }

        gcx.documents_state
            .memory_document_map
            .lock()
            .await
            .remove(&path_buf);

        Ok(())
    })
    .await
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
    with_memory_mutation_lock(async {
        match fs::remove_file(doc_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Failed to delete document: {}", e)),
        }
        gcx.enrichment_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        gcx.knowledge_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        info!("Deleted document from disk: {}", doc_path.display());

        gcx.knowledge_index.lock().await.remove_path(doc_path);

        gcx.documents_state
            .memory_document_map
            .lock()
            .await
            .remove(doc_path);

        Ok(())
    })
    .await
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

    let exact_duplicate = {
        let signature = refact_buddy_core::memory_dedup::content_signature(content);
        let idx = gcx.knowledge_index.lock().await;
        idx.first_path_for_signature(&signature).cloned()
    };
    if let Some(existing) = exact_duplicate {
        info!(
            "memories: enriched save is an exact duplicate of {}, touching instead",
            existing.display()
        );
        match touch_memory_observation(gcx.clone(), &existing).await {
            Ok(()) => return Ok(existing),
            Err(err) => {
                warn!(
                    "memories: failed to touch enriched duplicate {}: {}, saving normally",
                    existing.display(),
                    err
                );
                evict_stale_index_path(gcx.clone(), &existing).await;
            }
        }
    }

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
        params.source_chat_id.as_deref(),
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
        occurrences: 0,
        signal_key: None,
        last_observed: None,
        extra: Default::default(),
    };

    let outcome = memories_add_dedup(gcx.clone(), &frontmatter, content).await?;
    if !outcome.created() {
        return Ok(outcome.path().clone());
    }
    let file_path = outcome.path().clone();
    let new_doc_id = frontmatter.id.clone().unwrap();

    let mut updated_frontmatter = frontmatter.clone();
    if let Err(e) =
        auto_link_memory(gcx.clone(), &mut updated_frontmatter, content, &file_path).await
    {
        warn!("Auto-linking failed for new memory: {}", e);
    } else if updated_frontmatter.links != frontmatter.links {
        if let Err(e) = update_memory_document_frontmatter(gcx.clone(), &file_path, |current| {
            let mut changed = false;
            for link in &updated_frontmatter.links {
                if !current.links.contains(link) {
                    current.links.push(link.clone());
                    changed = true;
                }
            }
            Ok(changed)
        })
        .await
        {
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
            params.source_chat_id.as_deref(),
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

    #[tokio::test]
    async fn local_dedup_never_touches_other_project_memory() {
        let local = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let local_root = crate::files_correction::canonicalize_normalized_path(local.path().into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![local_root.clone()];
        let foreign_path = foreign.path().join("foreign.md");
        let fm = create_frontmatter(Some("Foreign"), &[], &[], &[], "domain");
        let body = "Same exact body but another project's evidence";
        let original = format!("{}\n\n{}", fm.to_yaml(), body);
        fs::write(&foreign_path, &original).await.unwrap();
        {
            let mut index = gcx.knowledge_index.lock().await;
            index.add_signature(
                refact_buddy_core::memory_dedup::content_signature(body),
                foreign_path.clone(),
            );
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                foreign_path.clone(),
                &fm,
                Some(body),
            );
        }
        let outcome = memories_add_dedup(gcx, &fm, body).await.unwrap();
        assert!(outcome.created());
        assert!(outcome.path().starts_with(local_root));
        assert_eq!(fs::read_to_string(foreign_path).await.unwrap(), original);
    }

    #[tokio::test]
    async fn memory_mutation_lock_nests_and_blocks_native_writer_until_cas_finishes() {
        use std::future::Future;
        use std::task::Poll;

        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let path = memories_add(gcx.clone(), &dedup_test_frontmatter("CAS"), "Original body")
            .await
            .unwrap();
        let contender = with_memory_mutation_lock(async {
            let before = fs::read_to_string(&path).await.unwrap();
            let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
            let writer_gcx = gcx.clone();
            let writer_path = path.clone();
            let contender = tokio::spawn(async move {
                let mut write = Box::pin(touch_memory_observation(writer_gcx, &writer_path));
                std::future::poll_fn(|cx| {
                    assert!(write.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
                polled_tx.send(()).unwrap();
                write.await.unwrap();
            });
            polled_rx.await.unwrap();
            assert_eq!(fs::read_to_string(&path).await.unwrap(), before);
            with_memory_mutation_lock(update_memory_document_frontmatter(
                gcx.clone(),
                &path,
                |fm| {
                    fm.title = Some("CAS committed".to_string());
                    Ok(true)
                },
            ))
            .await
            .unwrap();
            contender
        })
        .await;
        contender.await.unwrap();
        let text = fs::read_to_string(&path).await.unwrap();
        let (fm, start) = KnowledgeFrontmatter::parse(&text);
        assert_eq!(fm.title.as_deref(), Some("CAS committed"));
        assert_eq!(fm.occurrences, 2);
        assert_eq!(text[start..].trim(), "Original body");
    }

    #[tokio::test]
    async fn concurrent_native_dedup_creates_one_document_and_counts_every_observation() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let barrier = Arc::new(tokio::sync::Barrier::new(8));
        let mut writers = Vec::new();
        for _ in 0..8 {
            let gcx = gcx.clone();
            let barrier = barrier.clone();
            writers.push(tokio::spawn(async move {
                barrier.wait().await;
                memories_add_dedup(
                    gcx,
                    &dedup_test_frontmatter("Concurrent"),
                    "A distinctive canonical observation shared by all concurrent writers.",
                )
                .await
                .unwrap()
            }));
        }
        let mut outcomes = Vec::new();
        for writer in writers {
            outcomes.push(writer.await.unwrap());
        }
        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.created()).count(),
            1
        );
        let path = outcomes[0].path();
        assert!(outcomes.iter().all(|outcome| outcome.path() == path));
        let text = fs::read_to_string(path).await.unwrap();
        assert_eq!(KnowledgeFrontmatter::parse(&text).0.occurrences, 8);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[tokio::test]
    async fn native_frontmatter_updates_preserve_unknown_fields_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let path = dir.path().join("unknown.md");
        let original = "---\nid: unknown\ntitle: Original\ncustom_scalar: keep\ncustom_list: [alpha, beta]\ncustom_object:\n  nested: true\n---\nOriginal body";
        fs::write(&path, original).await.unwrap();
        let expected = KnowledgeFrontmatter::parse(original).0.extra;
        with_memory_mutation_lock(async {
            touch_memory_observation(gcx.clone(), &path).await.unwrap();
            deprecate_document(gcx.clone(), &path, Some("replacement"), "superseded")
                .await
                .unwrap();
        })
        .await;
        let text = fs::read_to_string(&path).await.unwrap();
        let (fm, start) = KnowledgeFrontmatter::parse(&text);
        assert_eq!(fm.extra, expected);
        assert_eq!(fm.extra.len(), 3);
        assert_eq!(fm.occurrences, 2);
        assert_eq!(fm.status.as_deref(), Some("deprecated"));
        assert_eq!(text[start..].trim(), "Original body");
        with_memory_mutation_lock(delete_document_from_disk(gcx, &path))
            .await
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn root_chat_memo_insert_is_bounded() {
        let mut memo = HashMap::new();
        for index in 0..=ROOT_CHAT_MEMO_CAPACITY {
            insert_bounded_root_chat_memo(
                &mut memo,
                format!("chat-{index}"),
                format!("root-{index}"),
            );
            assert!(memo.len() <= ROOT_CHAT_MEMO_CAPACITY);
        }
        assert_eq!(memo.len(), 1);
        assert_eq!(
            memo.get(&format!("chat-{ROOT_CHAT_MEMO_CAPACITY}")),
            Some(&format!("root-{ROOT_CHAT_MEMO_CAPACITY}"))
        );
    }

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

    fn dedup_test_frontmatter(title: &str) -> KnowledgeFrontmatter {
        let tags = vec!["buddy".to_string(), "diagnostics".to_string()];
        let empty = Vec::<String>::new();
        create_frontmatter(Some(title), &tags, &empty, &empty, "insight")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_add_dedup_touches_exact_normalized_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let first = memories_add_dedup(
            gcx.clone(),
            &dedup_test_frontmatter("Signal insight"),
            "Signal hash fd0347ef seen 43 times at 2026-07-01T10:00:00Z in chat generation",
        )
        .await
        .unwrap();
        assert!(first.created());

        let second = memories_add_dedup(
            gcx.clone(),
            &dedup_test_frontmatter("Signal insight repeat"),
            "Signal hash ab99cd11 seen 57 times at 2026-07-02T08:30:00Z in chat generation",
        )
        .await
        .unwrap();

        assert!(!second.created());
        assert_eq!(second.path(), first.path());
        let text = tokio::fs::read_to_string(second.path()).await.unwrap();
        let (frontmatter, _) = KnowledgeFrontmatter::parse(&text);
        assert!(frontmatter.occurrences >= 2);
        assert!(frontmatter.last_observed.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_add_dedup_touches_near_duplicate_with_shared_tag() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let first = memories_add_dedup(
            gcx.clone(),
            &dedup_test_frontmatter("Caps drift"),
            "Pattern observed: repeated llm_error diagnostics clustered at chat generation, \
             mostly caps-registry drift where the default model id is no longer served by \
             the server. Fix is in user config, update default models.",
        )
        .await
        .unwrap();
        assert!(first.created());

        let second = memories_add_dedup(
            gcx.clone(),
            &dedup_test_frontmatter("Caps drift again"),
            "Pattern observed: repeated llm_error diagnostics clustered at chat generation, \
             mostly caps-registry drift where the default model id is no longer served by \
             the server. Fix is in user config, update the default models slot.",
        )
        .await
        .unwrap();

        assert!(!second.created());
        assert_eq!(second.path(), first.path());

        let distinct = memories_add_dedup(
            gcx,
            &dedup_test_frontmatter("Voice cache"),
            "Voice rendering caches the pulse one-liner for five minutes which replays \
             identical lines when the pulse is quiet, needs a said-recently ring.",
        )
        .await
        .unwrap();
        assert!(distinct.created());
        assert_ne!(distinct.path(), first.path());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_add_dedup_recovers_from_stale_index_hint() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let content =
            "Stale hint recovery memo body with enough distinctive tokens to index cleanly.";

        let first = memories_add_dedup(gcx.clone(), &dedup_test_frontmatter("Stale"), content)
            .await
            .unwrap();
        assert!(first.created());
        tokio::fs::remove_file(first.path()).await.unwrap();

        let second = memories_add_dedup(gcx.clone(), &dedup_test_frontmatter("Stale"), content)
            .await
            .unwrap();
        assert!(second.created());
        assert!(tokio::fs::try_exists(second.path()).await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_upsert_signal_key_survives_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let first = memories_upsert_by_signal_key(
            gcx.clone(),
            "diag:signal:stale00",
            &dedup_test_frontmatter("Stale signal"),
            "Original signal body that will disappear from disk before the next upsert.",
        )
        .await
        .unwrap();
        assert!(first.created());
        tokio::fs::remove_file(first.path()).await.unwrap();

        let second = memories_upsert_by_signal_key(
            gcx.clone(),
            "diag:signal:stale00",
            &dedup_test_frontmatter("Stale signal refreshed"),
            "Replacement body recreated after the original file vanished from disk.",
        )
        .await
        .unwrap();
        assert!(second.created());
        let text = tokio::fs::read_to_string(second.path()).await.unwrap();
        assert!(text.contains("signal_key: \"diag:signal:stale00\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memories_upsert_by_signal_key_rewrites_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let first = memories_upsert_by_signal_key(
            gcx.clone(),
            "diag:signal:fd0347ef",
            &dedup_test_frontmatter("Detective finding"),
            "First investigation body with initial conclusions about the failure cluster.",
        )
        .await
        .unwrap();
        assert!(first.created());

        let second = memories_upsert_by_signal_key(
            gcx.clone(),
            "diag:signal:fd0347ef",
            &dedup_test_frontmatter("Detective finding refreshed"),
            "Updated investigation body superseding stale conclusions with fresh evidence.",
        )
        .await
        .unwrap();

        assert!(!second.created());
        assert_eq!(second.path(), first.path());
        let text = tokio::fs::read_to_string(second.path()).await.unwrap();
        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
        assert_eq!(frontmatter.occurrences, 2);
        assert_eq!(
            frontmatter.signal_key.as_deref(),
            Some("diag:signal:fd0347ef")
        );
        assert!(text[content_start..].contains("Updated investigation body"));
        assert!(!text[content_start..].contains("First investigation body"));

        let files = std::fs::read_dir(dir.path().join(crate::file_filter::KNOWLEDGE_FOLDER_NAME))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
            .count();
        assert_eq!(files, 1);
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

#[cfg(test)]
mod firewall_tests {
    use super::*;
    async fn write_knowledge_note(knowledge_dir: &Path, name: &str, body: &str) -> PathBuf {
        tokio::fs::create_dir_all(knowledge_dir).await.unwrap();
        let path = knowledge_dir.join(name);
        let frontmatter = create_frontmatter(
            Some("Firewall note"),
            &vec!["preference".to_string()],
            &Vec::new(),
            &Vec::new(),
            "domain",
        );
        let md = format!("{}\n\n{}", frontmatter.to_yaml(), body);
        tokio::fs::write(&path, md).await.unwrap();
        path
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lexical_search_uses_ready_index_without_reading_the_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let project_root =
            dunce::simplified(&std::fs::canonicalize(dir.path()).unwrap()).to_path_buf();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project_root.clone()];
        let knowledge_dir = project_root.join(KNOWLEDGE_FOLDER_NAME);
        let indexed = write_knowledge_note(&knowledge_dir, "indexed.md", "codegraph needle").await;
        for index in 0..1_000 {
            tokio::fs::write(
                knowledge_dir.join(format!("unindexed-{index}.md")),
                "codegraph corpus file that fallback must not read",
            )
            .await
            .unwrap();
        }
        {
            let mut index = gcx.knowledge_index.lock().await;
            let text = tokio::fs::read_to_string(&indexed).await.unwrap();
            let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                indexed.clone(),
                &frontmatter,
                Some(&text[content_start..]),
            );
            index.mark_ready();
        }

        let records = memories_search(gcx, "codegraph", 5, None).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].file_path.as_deref(), Some(indexed.as_path()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lexical_search_discloses_unavailable_index_without_scanning() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        write_knowledge_note(&knowledge_dir, "unavailable.md", "codegraph needle").await;

        let records = memories_search(gcx, "codegraph", 5, None).await.unwrap();
        assert!(records.is_empty());
    }
    #[tokio::test]
    async fn lexical_search_tracks_rewrite_archive_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![dunce::simplified(&std::fs::canonicalize(dir.path()).unwrap()).to_path_buf()];
        gcx.knowledge_index.lock().await.mark_ready();
        let frontmatter = create_frontmatter(
            Some("Routing decision"),
            &vec!["routing".to_string()],
            &Vec::new(),
            &Vec::new(),
            "domain",
        );
        let path = memories_add(gcx.clone(), &frontmatter, "originalneedle")
            .await
            .unwrap();
        let hits = memories_search(gcx.clone(), "originalneedle", 1, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_path.as_deref(), Some(path.as_path()));
        assert_ne!(hits[0].kind.as_deref(), Some("trajectory"));
        assert!(memories_search(gcx.clone(), "originalneedle", 0, None)
            .await
            .unwrap()
            .is_empty());

        rewrite_memory_document(gcx.clone(), &path, &frontmatter, "replacementneedle")
            .await
            .unwrap();
        assert!(memories_search(gcx.clone(), "originalneedle", 1, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            memories_search(gcx.clone(), "replacementneedle", 1, None)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut archived = frontmatter.clone();
        archived.status = Some("archived".to_string());
        rewrite_memory_document(gcx.clone(), &path, &archived, "replacementneedle")
            .await
            .unwrap();
        assert!(memories_search(gcx.clone(), "replacementneedle", 1, None)
            .await
            .unwrap()
            .is_empty());
        rewrite_memory_document(gcx.clone(), &path, &frontmatter, "replacementneedle")
            .await
            .unwrap();
        assert_eq!(
            memories_search(gcx.clone(), "replacementneedle", 1, None)
                .await
                .unwrap()
                .len(),
            1
        );
        delete_document_from_disk(gcx.clone(), &path).await.unwrap();
        assert!(memories_search(gcx, "replacementneedle", 1, None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn lexical_search_skips_stop_words_and_short_tokens() {
        struct Clock;
        impl perf_diagnostics::PerfClock for Clock {
            fn now_us(&self) -> u64 {
                0
            }
        }
        let sink = Arc::new(perf_diagnostics::MemoryPerfSink::new());
        let _recorder = perf_diagnostics::install_test_recorder(Arc::new(
            perf_diagnostics::PerfRecorder::new(Arc::new(Clock), sink.clone()),
        ));
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let knowledge = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let fm = create_frontmatter(
            Some("how does this work"),
            &vec![],
            &vec![],
            &vec![],
            "domain",
        );
        {
            let mut index = gcx.knowledge_index.lock().await;
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                knowledge.join("note.md"),
                &fm,
                Some("how does this work? an ox is up"),
            );
            index.mark_ready();
            assert!(!index
                .search_bounded(
                    "how does this work?",
                    &KnowledgeSearchFilters::default(),
                    4,
                    4
                )
                .is_empty());
        }
        for query in [
            "how does this work?",
            "an ox is up",
            "--how-- does this work?",
        ] {
            assert!(
                memories_search_lexical(
                    gcx.clone(),
                    query,
                    3,
                    &[knowledge.clone()],
                    None,
                    Some("stop-word-test")
                )
                .await
                .unwrap()
                .is_empty(),
                "{query}"
            );
        }
        let events = sink.events();
        assert_eq!(events.len(), 3);
        for event in events {
            assert_eq!(event.component, "enrichment.lexical");
            assert_eq!(event.outcome, "skipped");
            assert_eq!(event.size_bytes, Some(0));
            assert_eq!(event.item_count, Some(0));
        }
    }

    #[tokio::test]
    async fn lexical_search_excludes_foreign_roots_and_current_chat_sources() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = dunce::simplified(&std::fs::canonicalize(dir.path()).unwrap()).to_path_buf();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let local = root.join(KNOWLEDGE_FOLDER_NAME).join("local.md");
        let foreign = foreign.path().join("foreign.md");
        let mut fm = create_frontmatter(Some("needle"), &vec![], &vec![], &vec![], "domain");
        fm.source_chat_id = Some("current-root".to_string());
        {
            let mut index = gcx.knowledge_index.lock().await;
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                local.clone(),
                &fm,
                Some("needle"),
            );
            fm.source_chat_id = None;
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                foreign,
                &fm,
                Some("needle"),
            );
            index.mark_ready();
        }
        let dirs = vec![root.join(KNOWLEDGE_FOLDER_NAME)];
        let hits = memories_search_lexical(gcx.clone(), "needle", 8, &dirs, None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_path.as_ref(), Some(&local));
        assert!(
            memories_search_lexical(gcx, "needle", 8, &dirs, Some("current-root"), None)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
