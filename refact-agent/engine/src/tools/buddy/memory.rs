use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Local, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;
use walkdir::WalkDir;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::memory_lifecycle::{
    archive_memory_file_checked, compute_content_hash, find_memory_by_source, normalize_kind,
    normalize_tags, MemoryCreatePayload, MemoryLifecycleOp, MemoryLifecyclePayload, MemoryOpStatus,
    MemoryOpType, MemorySource,
};
use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::buddy::storage::enqueue_memory_op;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use crate::knowledge_index::KnowledgeCard;
use crate::memories::{
    get_global_knowledge_dir, memories_add, normalize_memory_tags,
    update_memory_document_frontmatter,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const MEMORY_KINDS: &[&str] = &[
    "domain",
    "lesson",
    "convention",
    "insight",
    "humor",
    "artifact",
];
const WRITABLE_KINDS: &[&str] = &["domain", "lesson", "convention", "insight", "humor"];
const MAX_KNOWLEDGE_SCAN_FILES: usize = 1000;
const MAX_KNOWLEDGE_SCAN_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILE_SIZE_TO_SCAN: u64 = 512 * 1024;

pub struct ToolBuddyMemorySearch {
    pub config_path: String,
}

pub struct ToolBuddyMemoryCreate {
    pub config_path: String,
}

pub struct ToolBuddyMemoryArchive {
    pub config_path: String,
}

pub struct ToolBuddyMemoryRetag {
    pub config_path: String,
}

pub struct ToolBuddyMemoryMerge {
    pub config_path: String,
}

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn result(tool_call_id: &String, text: impl Into<String>) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.into()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

fn string_arg<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("argument `{name}` is missing or not a non-empty string"))
}

fn optional_string_arg(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array_arg(args: &HashMap<String, Value>, name: &str) -> Result<Vec<String>, String> {
    let value = args
        .get(name)
        .ok_or_else(|| format!("argument `{name}` is missing"))?;
    let array = value
        .as_array()
        .ok_or_else(|| format!("argument `{name}` must be an array of strings"))?;
    let mut out = Vec::new();
    for item in array {
        let item = item
            .as_str()
            .ok_or_else(|| format!("argument `{name}` must be an array of strings"))?
            .trim();
        if !item.is_empty() {
            out.push(item.to_string());
        }
    }
    Ok(out)
}

fn optional_string_array_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Vec<String>, String> {
    if !args.contains_key(name) {
        return Ok(Vec::new());
    }
    string_array_arg(args, name)
}

fn limit_arg(args: &HashMap<String, Value>) -> Result<usize, String> {
    let raw = args.get("limit").and_then(Value::as_u64).unwrap_or(10);
    Ok(raw.min(25).max(1) as usize)
}

fn confidence_arg(args: &HashMap<String, Value>) -> Result<f32, String> {
    let confidence = args
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.8);
    if !(0.0..=1.0).contains(&confidence) {
        return Err("argument `confidence` must be between 0.0 and 1.0".to_string());
    }
    Ok(confidence as f32)
}

fn validate_kind(kind: &str, allowed: &[&str]) -> Result<String, String> {
    let kind = normalize_kind(kind);
    if !allowed.contains(&kind.as_str()) {
        return Err(format!("unsupported memory kind: {kind}"));
    }
    Ok(kind)
}

fn tags_with_buddy(tags: Vec<String>) -> Vec<String> {
    let mut tags = tags;
    tags.push("buddy".to_string());
    normalize_memory_tags(&tags, 32)
}

fn hash_parts(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn op_id(tool_name: &str, parts: &[&str]) -> String {
    format!("memop_buddy_{}_{}", tool_name, &hash_parts(parts)[..16])
}

async fn knowledge_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|dir| dir.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|dir| dir.exists())
        .collect();
    let global = get_global_knowledge_dir(gcx).await;
    if global.exists() {
        dirs.push(global);
    }
    dirs
}

fn reject_dotdot(path: &Path) -> Result<(), String> {
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("memory path cannot contain '..'".to_string());
        }
    }
    Ok(())
}

async fn project_root(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
}

async fn resolve_memory_path(gcx: Arc<GlobalContext>, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err("memory path is empty or invalid".to_string());
    }
    let raw_path = PathBuf::from(raw);
    reject_dotdot(&raw_path)?;
    if raw_path.is_absolute() {
        return Ok(raw_path);
    }
    let root = project_root(gcx).await?;
    Ok(root.join(raw_path))
}

async fn checked_existing_memory_path(
    gcx: Arc<GlobalContext>,
    path: &Path,
) -> Result<PathBuf, String> {
    reject_dotdot(path)?;
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| format!("memory path not accessible: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err("memory path cannot be a symlink".to_string());
    }
    if !metadata.is_file() {
        return Err("memory path must be a file".to_string());
    }
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|e| format!("failed to canonicalize memory path: {e}"))?;
    let mut roots = Vec::new();
    for root in knowledge_dirs(gcx).await {
        if root.exists() {
            roots.push(
                tokio::fs::canonicalize(root)
                    .await
                    .map(|path| dunce::simplified(&path).to_path_buf())
                    .map_err(|e| format!("failed to canonicalize knowledge root: {e}"))?,
            );
        }
    }
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("memory path is outside knowledge directories".to_string());
    }
    Ok(canonical)
}

fn parse_memory_text(text: &str) -> (KnowledgeFrontmatter, String) {
    let (frontmatter, body_start) = KnowledgeFrontmatter::parse(text);
    let body = text.get(body_start..).unwrap_or("").trim().to_string();
    (frontmatter, body)
}

async fn scan_cards(gcx: Arc<GlobalContext>) -> Vec<KnowledgeCard> {
    scan_cards_in_dirs(knowledge_dirs(gcx).await).await
}

async fn scan_cards_in_dirs(dirs: Vec<PathBuf>) -> Vec<KnowledgeCard> {
    let mut cards = Vec::new();
    let mut scanned_files = 0usize;
    let mut scanned_bytes = 0u64;
    for dir in dirs {
        for entry in WalkDir::new(&dir).into_iter().filter_map(Result::ok) {
            if scanned_files >= MAX_KNOWLEDGE_SCAN_FILES
                || scanned_bytes >= MAX_KNOWLEDGE_SCAN_BYTES
            {
                return dedup_cards(cards);
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }
            let Ok(metadata) = path.symlink_metadata() else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.len() > MAX_FILE_SIZE_TO_SCAN {
                continue;
            }
            if scanned_bytes.saturating_add(metadata.len()) > MAX_KNOWLEDGE_SCAN_BYTES {
                return dedup_cards(cards);
            }
            let Ok(text) = tokio::fs::read_to_string(path).await else {
                continue;
            };
            scanned_files += 1;
            scanned_bytes = scanned_bytes.saturating_add(metadata.len());
            let (frontmatter, body) = parse_memory_text(&text);
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }
            let id = frontmatter
                .id
                .clone()
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let title = frontmatter.title.clone().unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let preview = body
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_string());
            cards.push(KnowledgeCard {
                id,
                title,
                summary: frontmatter.summary.clone().or(preview),
                description: frontmatter.description.clone(),
                tags: frontmatter.tags.clone(),
                filenames: frontmatter.filenames.clone(),
                entities: frontmatter.entities.clone(),
                related_files: frontmatter.related_files.clone(),
                related_entities: frontmatter.related_entities.clone(),
                kind: frontmatter.kind.clone(),
                created: frontmatter.created.clone(),
                created_at: frontmatter.created_at.clone(),
                updated: frontmatter.updated.clone(),
                file_path: path.to_path_buf(),
            });
        }
    }
    dedup_cards(cards)
}

fn dedup_cards(cards: Vec<KnowledgeCard>) -> Vec<KnowledgeCard> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for card in cards {
        if seen.insert(card.file_path.clone()) {
            out.push(card);
        }
    }
    out
}

async fn all_index_cards(gcx: Arc<GlobalContext>) -> Vec<KnowledgeCard> {
    let gcx_read = gcx.clone();
    let index = gcx_read.knowledge_index.lock().await;
    if !index.is_empty() {
        return dedup_cards(index.all_cards());
    }
    drop(index);
    drop(gcx_read);
    scan_cards(gcx).await
}

fn score_card(card: &KnowledgeCard, query: &str, kind: Option<&str>, tags: &[String]) -> i32 {
    let q = query.to_lowercase();
    let mut score = 0;
    let text = format!(
        "{} {} {} {} {}",
        card.title,
        card.summary.clone().unwrap_or_default(),
        card.description.clone().unwrap_or_default(),
        card.kind.clone().unwrap_or_default(),
        card.tags.join(" ")
    )
    .to_lowercase();
    if text.contains(&q) {
        score += 50;
    }
    for token in q.split_whitespace() {
        if text.contains(token) {
            score += 10;
        }
    }
    if let Some(kind) = kind {
        if card.kind.as_deref() == Some(kind) {
            score += 25;
        }
    }
    for tag in tags {
        if card.tags.iter().any(|existing| existing == tag) {
            score += 15;
        }
    }
    score
}

fn card_matches(card: &KnowledgeCard, kind: Option<&str>, tags: &[String]) -> bool {
    if let Some(kind) = kind {
        if card.kind.as_deref() != Some(kind) {
            return false;
        }
    }
    tags.iter()
        .all(|tag| card.tags.iter().any(|existing| existing == tag))
}

fn markdown_table(cards: &[KnowledgeCard]) -> String {
    let mut lines = vec![
        "| path | title | kind | tags | preview |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    for card in cards {
        let preview = card
            .summary
            .as_deref()
            .or(card.description.as_deref())
            .unwrap_or("")
            .replace('|', "\\|")
            .replace('\n', " ");
        lines.push(format!(
            "| {} | {} | {} | {} | {} |",
            card.file_path.display(),
            card.title.replace('|', "\\|"),
            card.kind.clone().unwrap_or_default(),
            card.tags.join(", "),
            preview.chars().take(120).collect::<String>()
        ));
    }
    lines.join("\n")
}

async fn append_audit_op(gcx: Arc<GlobalContext>, mut op: MemoryLifecycleOp) -> Result<(), String> {
    let root = project_root(gcx).await?;
    let now = Utc::now().to_rfc3339();
    op.status = MemoryOpStatus::Applied;
    op.requires_approval = false;
    op.applied_at = Some(now);
    enqueue_memory_op(&root, op).await?;
    Ok(())
}

async fn find_duplicate_content(gcx: Arc<GlobalContext>, hash: &str) -> Option<PathBuf> {
    for dir in knowledge_dirs(gcx).await {
        for entry in WalkDir::new(&dir).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            let Ok(metadata) = path.symlink_metadata() else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }
            let Ok(text) = tokio::fs::read_to_string(path).await else {
                continue;
            };
            let (frontmatter, body) = parse_memory_text(&text);
            // Status is authoritative: only skip docs whose status marks them inactive. Do not
            // skip an explicitly-active/pinned doc just because it carries a stale
            // deprecated_at/superseded_by field, or dedup would miss live canonicals.
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }
            if frontmatter.content_hash.as_deref() == Some(hash)
                || compute_content_hash(&body) == hash
            {
                return Some(path.to_path_buf());
            }
        }
    }
    None
}

async fn create_memory(
    gcx: Arc<GlobalContext>,
    title: &str,
    content: &str,
    tags: Vec<String>,
    kind: &str,
    source_id: &str,
    confidence: f32,
) -> Result<CreateOutcome, String> {
    if title.chars().count() > 160 {
        return Err("argument `title` must be at most 160 chars".to_string());
    }
    if content.chars().count() > 4000 {
        return Err("argument `content` must be at most 4000 chars".to_string());
    }
    let kind = validate_kind(kind, WRITABLE_KINDS)?;
    let tags = tags_with_buddy(tags);
    let title = redact_and_cap_text(title, 160);
    let content = redact_and_cap_text(content, 4000);
    let content_hash = compute_content_hash(&content);
    if let Some(path) = find_memory_by_source(
        gcx.clone(),
        MemorySource::Buddy,
        Some(source_id),
        Some(&content_hash),
    )
    .await
    {
        return Ok(CreateOutcome::Skipped(path));
    }
    if let Some(path) = find_duplicate_content(gcx.clone(), &content_hash).await {
        return Ok(CreateOutcome::Skipped(path));
    }
    let now = Local::now().format("%Y-%m-%d").to_string();
    let frontmatter = KnowledgeFrontmatter {
        id: Some(uuid::Uuid::new_v4().to_string()),
        title: Some(title.clone()),
        tags: tags.clone(),
        created: Some(now.clone()),
        updated: Some(now),
        kind: Some(kind.clone()),
        status: Some("active".to_string()),
        review_after: Some(
            (Utc::now() + chrono::Duration::days(90))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        created_at: Some(Utc::now().to_rfc3339()),
        content_hash: Some(content_hash.clone()),
        source_id: Some(source_id.to_string()),
        source_content_hash: Some(content_hash.clone()),
        source_tool: Some("buddy_memory_create".to_string()),
        source_confidence: Some(confidence),
        ..Default::default()
    };
    let path = memories_add(gcx.clone(), &frontmatter, &content).await?;
    let mut op = MemoryLifecycleOp::pending(
        op_id("create", &[source_id, &content_hash]),
        MemorySource::Buddy,
        MemoryOpType::CreateMemory,
        vec![path.to_string_lossy().to_string()],
        format!("buddy_memory_create source_id={source_id}"),
        confidence,
        Utc::now().to_rfc3339(),
    );
    op.payload = MemoryLifecyclePayload {
        title: Some(title),
        content: Some(content.clone()),
        tags: Some(tags),
        kind: Some(kind),
        source_id: Some(source_id.to_string()),
        source_content_hash: Some(content_hash),
        ..Default::default()
    };
    if let Err(err) = append_audit_op(gcx, op).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(err);
    }
    Ok(CreateOutcome::Created(path))
}

enum CreateOutcome {
    Created(PathBuf),
    Skipped(PathBuf),
}

#[async_trait]
impl Tool for ToolBuddyMemorySearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_memory_search".to_string(),
            display_name: "Buddy Memory Search".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Search Buddy-managed memory files by query, kind, and tags.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "kind": {"type": "string", "enum": MEMORY_KINDS},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "limit": {"type": "integer", "default": 10, "maximum": 25}
                },
                "required": ["query"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let query = string_arg(args, "query")?;
        let kind = optional_string_arg(args, "kind")
            .map(|kind| validate_kind(&kind, MEMORY_KINDS))
            .transpose()?;
        let tags = normalize_tags(&optional_string_array_arg(args, "tags")?);
        let limit = limit_arg(args)?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let mut cards = all_index_cards(gcx).await;
        cards.retain(|card| card_matches(card, kind.as_deref(), &tags));
        cards.sort_by(|a, b| {
            score_card(b, query, kind.as_deref(), &tags)
                .cmp(&score_card(a, query, kind.as_deref(), &tags))
                .then_with(|| a.title.cmp(&b.title))
        });
        cards.truncate(limit);
        Ok(result(tool_call_id, markdown_table(&cards)))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyMemoryCreate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_memory_create".to_string(),
            display_name: "Buddy Memory Create".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Create a Buddy memory immediately and record an applied audit op."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "maxLength": 160},
                    "content": {"type": "string", "maxLength": 4000},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "kind": {"type": "string", "enum": WRITABLE_KINDS},
                    "source_id": {"type": "string"},
                    "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0, "default": 0.8}
                },
                "required": ["title", "content", "tags", "kind", "source_id"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();
        let outcome = create_memory(
            gcx,
            string_arg(args, "title")?,
            string_arg(args, "content")?,
            string_array_arg(args, "tags")?,
            string_arg(args, "kind")?,
            string_arg(args, "source_id")?,
            confidence_arg(args)?,
        )
        .await?;
        Ok(match outcome {
            CreateOutcome::Created(path) => {
                result(tool_call_id, format!("Created memory: {}", path.display()))
            }
            CreateOutcome::Skipped(path) => result(
                tool_call_id,
                format!("Skipped: identical memory exists at {}", path.display()),
            ),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyMemoryArchive {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_memory_archive".to_string(),
            display_name: "Buddy Memory Archive".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Archive a memory file immediately and record an applied audit op."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "reason": {"type": "string", "maxLength": 240},
                    "superseded_by": {"type": "string"}
                },
                "required": ["path", "reason"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let reason = string_arg(args, "reason")?;
        if reason.chars().count() > 240 {
            return Err("argument `reason` must be at most 240 chars".to_string());
        }
        let gcx = ccx.lock().await.app.gcx.clone();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let path = resolve_memory_path(gcx.clone(), string_arg(args, "path")?).await?;
        let superseded_by = optional_string_arg(args, "superseded_by");
        let changed = archive_memory_file_checked(app, &path, superseded_by.as_deref()).await?;
        let mut op = MemoryLifecycleOp::pending(
            op_id("archive", &[&path.to_string_lossy(), reason]),
            MemorySource::Buddy,
            MemoryOpType::ArchiveCandidate,
            vec![path.to_string_lossy().to_string()],
            reason.to_string(),
            1.0,
            Utc::now().to_rfc3339(),
        );
        op.payload.superseded_by = superseded_by;
        append_audit_op(gcx, op).await?;
        Ok(if changed {
            result(tool_call_id, format!("Archived: {}", path.display()))
        } else {
            result(
                tool_call_id,
                format!("Already archived: {}", path.display()),
            )
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyMemoryRetag {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_memory_retag".to_string(),
            display_name: "Buddy Memory Retag".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Replace a memory file's tags immediately and record an applied audit op."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "new_tags": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["path", "new_tags"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();
        let path = resolve_memory_path(gcx.clone(), string_arg(args, "path")?).await?;
        let path = checked_existing_memory_path(gcx.clone(), &path).await?;
        let tags = tags_with_buddy(string_array_arg(args, "new_tags")?);
        update_memory_document_frontmatter(gcx.clone(), &path, |frontmatter| {
            frontmatter.tags = tags.clone();
            frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());
            Ok(true)
        })
        .await?;
        let mut op = MemoryLifecycleOp::pending(
            op_id("retag", &[&path.to_string_lossy(), &tags.join(",")]),
            MemorySource::Buddy,
            MemoryOpType::Retag,
            vec![path.to_string_lossy().to_string()],
            "buddy_memory_retag".to_string(),
            1.0,
            Utc::now().to_rfc3339(),
        );
        op.payload.tags = Some(tags);
        append_audit_op(gcx, op).await?;
        Ok(result(
            tool_call_id,
            format!("Retagged: {}", path.display()),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyMemoryMerge {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_memory_merge".to_string(),
            display_name: "Buddy Memory Merge".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Create a canonical memory and archive superseded memories.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "canonical_title": {"type": "string"},
                    "canonical_content": {"type": "string"},
                    "superseded_paths": {"type": "array", "items": {"type": "string"}},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "kind": {"type": "string", "enum": WRITABLE_KINDS}
                },
                "required": ["canonical_title", "canonical_content", "superseded_paths", "tags", "kind"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let superseded = string_array_arg(args, "superseded_paths")?;
        if superseded.is_empty() {
            return Err("argument `superseded_paths` must be non-empty".to_string());
        }
        let gcx = ccx.lock().await.app.gcx.clone();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let source_id = hash_parts(&[
            string_arg(args, "canonical_title")?,
            string_arg(args, "canonical_content")?,
            &superseded.join(","),
        ]);
        let canonical_path = match create_memory(
            gcx.clone(),
            string_arg(args, "canonical_title")?,
            string_arg(args, "canonical_content")?,
            string_array_arg(args, "tags")?,
            string_arg(args, "kind")?,
            &source_id,
            0.9,
        )
        .await?
        {
            CreateOutcome::Created(path) | CreateOutcome::Skipped(path) => path,
        };
        // D3: never archive the canonical itself. With source/content dedup, create_memory may
        // return an existing (possibly superseded) path as the canonical; archiving it would
        // leave the merged cluster with no active canonical.
        let canonical_canon = tokio::fs::canonicalize(&canonical_path).await.ok();
        let mut archived = 0usize;
        for raw in &superseded {
            let path = resolve_memory_path(gcx.clone(), raw).await?;
            let path_canon = tokio::fs::canonicalize(&path).await.ok();
            let is_canonical = path == canonical_path
                || (canonical_canon.is_some() && path_canon == canonical_canon);
            if is_canonical {
                continue;
            }
            if archive_memory_file_checked(
                app.clone(),
                &path,
                Some(&canonical_path.to_string_lossy()),
            )
            .await?
            {
                archived += 1;
            }
        }
        let mut op = MemoryLifecycleOp::pending(
            op_id(
                "merge",
                &[&canonical_path.to_string_lossy(), &superseded.join(",")],
            ),
            MemorySource::Buddy,
            MemoryOpType::MergeArchive,
            vec![canonical_path.to_string_lossy().to_string()],
            "buddy_memory_merge".to_string(),
            1.0,
            Utc::now().to_rfc3339(),
        );
        op.payload = MemoryLifecyclePayload {
            superseded_paths: superseded,
            canonical: Some(MemoryCreatePayload {
                title: Some(string_arg(args, "canonical_title")?.to_string()),
                content: string_arg(args, "canonical_content")?.to_string(),
                tags: tags_with_buddy(string_array_arg(args, "tags")?),
                kind: normalize_kind(string_arg(args, "kind")?),
                ..Default::default()
            }),
            ..Default::default()
        };
        append_audit_op(gcx, op).await?;
        Ok(result(
            tool_call_id,
            format!(
                "Merged {} memories into {}",
                archived,
                canonical_path.display()
            ),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::buddy::storage::load_memory_ops;
    use crate::tools::tools_description::Tool;
    use crate::worktrees::types::WorktreeMeta;
    use std::collections::HashMap;

    async fn test_gcx(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_read = gcx.clone();
            *gcx_read.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        }
        gcx
    }

    async fn ccx(root: &Path) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = test_gcx(root).await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                crate::app_state::AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "chat".to_string(),
                None,
                "model".to_string(),
                None,
                Some(WorktreeMeta {
                    id: "wt".to_string(),
                    kind: "chat".to_string(),
                    root: root.to_path_buf(),
                    source_workspace_root: root.to_path_buf(),
                    repo_root: root.to_path_buf(),
                    branch: None,
                    base_branch: None,
                    base_commit: None,
                    task_id: None,
                    card_id: None,
                    agent_id: None,
                    enforce: true,
                }),
            )
            .await,
        ))
    }

    fn text(result: &(bool, Vec<ContextEnum>)) -> String {
        result
            .1
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ChatMessage(message) => match &message.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn args(pairs: Vec<(&str, Value)>) -> HashMap<String, Value> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn merge_schema_kind_enum() -> Vec<String> {
        ToolBuddyMemoryMerge {
            config_path: String::new(),
        }
        .tool_description()
        .input_schema
        .get("properties")
        .and_then(|properties| properties.get("kind"))
        .and_then(|kind| kind.get("enum"))
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
    }

    #[test]
    fn merge_schema_kind_enum_matches_writable_kinds() {
        let writable_kinds = WRITABLE_KINDS
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(merge_schema_kind_enum(), writable_kinds);
    }

    async fn write_memory(
        root: &Path,
        name: &str,
        title: &str,
        kind: &str,
        tags: &[&str],
        body: &str,
    ) -> PathBuf {
        let dir = root.join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(name);
        let content = format!(
            "---\ntitle: {title}\nkind: {kind}\ntags: [{}]\nstatus: active\ncontent_hash: {}\n---\n\n{body}\n",
            tags.join(", "),
            compute_content_hash(body)
        );
        tokio::fs::write(&path, content).await.unwrap();
        path
    }

    #[tokio::test]
    async fn buddy_memory_create_writes_real_file_and_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryCreate {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(vec![
                    ("title", serde_json::json!("Create Test")),
                    ("content", serde_json::json!("remember the frog")),
                    ("tags", serde_json::json!(["test"])),
                    ("kind", serde_json::json!("lesson")),
                    ("source_id", serde_json::json!("src-1")),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Created memory:"));
        let files = std::fs::read_dir(dir.path().join(KNOWLEDGE_FOLDER_NAME))
            .unwrap()
            .count();
        assert_eq!(files, 1);
        let gcx = ccx.lock().await.app.gcx.clone();
        let cards = gcx.knowledge_index.lock().await.all_cards();
        assert!(cards.iter().any(|card| card.title == "Create Test"));
        let state = load_memory_ops(dir.path()).await;
        assert_eq!(state.applied_count, 1);
        let memory_path = std::fs::read_dir(dir.path().join(KNOWLEDGE_FOLDER_NAME))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = tokio::fs::read_to_string(memory_path).await.unwrap();
        let (frontmatter, _) = KnowledgeFrontmatter::parse(&text);
        assert_eq!(frontmatter.source_id.as_deref(), Some("src-1"));
        assert_eq!(
            frontmatter.source_content_hash.as_deref(),
            Some(compute_content_hash("remember the frog").as_str())
        );
    }

    #[tokio::test]
    async fn buddy_memory_create_skips_same_source_id_reworded_content() {
        let dir = tempfile::tempdir().unwrap();
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryCreate {
            config_path: String::new(),
        };
        let base_args = |content: &str| {
            args(vec![
                ("title", serde_json::json!("Source Test")),
                ("content", serde_json::json!(content)),
                ("tags", serde_json::json!(["test"])),
                ("kind", serde_json::json!("lesson")),
                ("source_id", serde_json::json!("same-source")),
            ])
        };
        let first = tool
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &base_args("first wording"),
            )
            .await
            .unwrap();
        let second = tool
            .tool_execute(ccx, &"call".to_string(), &base_args("second wording"))
            .await
            .unwrap();
        assert!(text(&first).contains("Created memory:"));
        assert!(text(&second).contains("Skipped: identical memory exists"));
        assert_eq!(
            std::fs::read_dir(dir.path().join(KNOWLEDGE_FOLDER_NAME))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn buddy_memory_create_skips_when_content_hash_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        write_memory(dir.path(), "old.md", "Old", "lesson", &["dup"], "same body").await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryCreate {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("title", serde_json::json!("New")),
                    ("content", serde_json::json!("same body")),
                    ("tags", serde_json::json!([])),
                    ("kind", serde_json::json!("lesson")),
                    ("source_id", serde_json::json!("src-dup")),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Skipped: identical memory exists"));
        assert_eq!(
            std::fs::read_dir(dir.path().join(KNOWLEDGE_FOLDER_NAME))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn buddy_memory_create_ignores_archived_exact_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let body = "same archived body";
        tokio::fs::write(
            knowledge_dir.join("archived.md"),
            format!(
                "---\ntitle: Archived\nkind: lesson\nstatus: archived\ncontent_hash: {}\ndeprecated_at: 2026-01-01\n---\n\n{body}\n",
                compute_content_hash(body)
            ),
        )
        .await
        .unwrap();
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryCreate {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("title", serde_json::json!("Fresh")),
                    ("content", serde_json::json!(body)),
                    ("tags", serde_json::json!([])),
                    ("kind", serde_json::json!("lesson")),
                    ("source_id", serde_json::json!("fresh-source")),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Created memory:"));
        assert_eq!(std::fs::read_dir(knowledge_dir).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn find_duplicate_content_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        tokio::fs::write(
            knowledge_dir.join("real.md"),
            "---\ntitle: Real\n---\n\nreal body\n",
        )
        .await
        .unwrap();

        let outside_dir = tempfile::tempdir().unwrap();
        let outside_body = format!("outside symlink body {}", uuid::Uuid::new_v4());
        let outside_file = outside_dir.path().join("outside.md");
        tokio::fs::write(
            &outside_file,
            format!("---\ntitle: Outside\n---\n\n{outside_body}\n"),
        )
        .await
        .unwrap();
        std::os::unix::fs::symlink(&outside_file, knowledge_dir.join("link.md")).unwrap();

        let gcx = test_gcx(dir.path()).await;
        let found = find_duplicate_content(gcx, &compute_content_hash(&outside_body)).await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn buddy_memory_search_finds_existing_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        write_memory(
            dir.path(),
            "lesson.md",
            "Lesson Hit",
            "lesson",
            &["alpha"],
            "frogs jump",
        )
        .await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemorySearch {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("query", serde_json::json!("frogs")),
                    ("kind", serde_json::json!("lesson")),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Lesson Hit"));
    }

    #[tokio::test]
    async fn scan_cards_caps_at_max_files() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..1500 {
            let path = dir.path().join(format!("memory-{idx:04}.md"));
            tokio::fs::write(
                path,
                format!("---\ntitle: Memory {idx}\n---\n\nbody {idx}\n"),
            )
            .await
            .unwrap();
        }

        let cards = scan_cards_in_dirs(vec![dir.path().to_path_buf()]).await;

        assert_eq!(cards.len(), MAX_KNOWLEDGE_SCAN_FILES);
    }

    #[tokio::test]
    async fn buddy_memory_search_finds_existing_by_tag() {
        let dir = tempfile::tempdir().unwrap();
        write_memory(
            dir.path(),
            "tag.md",
            "Tag Hit",
            "insight",
            &["rare"],
            "hidden gem",
        )
        .await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemorySearch {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("query", serde_json::json!("hidden")),
                    ("tags", serde_json::json!(["rare"])),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Tag Hit"));
    }

    #[tokio::test]
    async fn buddy_memory_archive_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_memory(
            dir.path(),
            "archive.md",
            "Archive",
            "lesson",
            &["old"],
            "old body",
        )
        .await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryArchive {
            config_path: String::new(),
        };
        let args = args(vec![
            ("path", serde_json::json!(path.to_string_lossy())),
            ("reason", serde_json::json!("obsolete")),
        ]);
        let first = tool
            .tool_execute(ccx.clone(), &"call".to_string(), &args)
            .await
            .unwrap();
        let second = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        assert!(text(&first).contains("Archived:"));
        assert!(text(&second).contains("Already archived:"));
    }

    #[tokio::test]
    async fn buddy_memory_archive_rejects_path_outside_knowledge_dir() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(KNOWLEDGE_FOLDER_NAME))
            .await
            .unwrap();
        let outside = dir.path().join("outside.md");
        tokio::fs::write(&outside, "---\n---\n\nbody")
            .await
            .unwrap();
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryArchive {
            config_path: String::new(),
        };
        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("path", serde_json::json!(outside.to_string_lossy())),
                    ("reason", serde_json::json!("bad")),
                ]),
            )
            .await
            .unwrap_err();
        assert!(err.contains("outside knowledge") || err.contains("outside knowledge directories"));
    }

    #[tokio::test]
    async fn buddy_memory_archive_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = write_memory(
            dir.path(),
            "target.md",
            "Target",
            "lesson",
            &["old"],
            "body",
        )
        .await;
        let link = dir.path().join(KNOWLEDGE_FOLDER_NAME).join("link.md");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryArchive {
            config_path: String::new(),
        };
        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("path", serde_json::json!(link.to_string_lossy())),
                    ("reason", serde_json::json!("bad")),
                ]),
            )
            .await
            .unwrap_err();
        assert!(err.contains("symlink"));
    }

    #[tokio::test]
    async fn buddy_memory_retag_preserves_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_memory(
            dir.path(),
            "retag.md",
            "Retag",
            "lesson",
            &["old"],
            "keep this body",
        )
        .await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryRetag {
            config_path: String::new(),
        };
        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(vec![
                ("path", serde_json::json!(path.to_string_lossy())),
                ("new_tags", serde_json::json!(["new"])),
            ]),
        )
        .await
        .unwrap();
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(text.contains("keep this body"));
        assert!(text.contains("new"));
        assert!(text.contains("buddy"));
    }

    #[tokio::test]
    async fn buddy_memory_merge_creates_canonical_and_archives_superseded() {
        let dir = tempfile::tempdir().unwrap();
        let one = write_memory(dir.path(), "one.md", "One", "lesson", &["old"], "one body").await;
        let two = write_memory(dir.path(), "two.md", "Two", "lesson", &["old"], "two body").await;
        let ccx = ccx(dir.path()).await;
        let mut tool = ToolBuddyMemoryMerge {
            config_path: String::new(),
        };
        let result = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(vec![
                    ("canonical_title", serde_json::json!("Canonical")),
                    ("canonical_content", serde_json::json!("canonical body")),
                    (
                        "superseded_paths",
                        serde_json::json!([one.to_string_lossy(), two.to_string_lossy()]),
                    ),
                    ("tags", serde_json::json!(["merged"])),
                    ("kind", serde_json::json!("lesson")),
                ]),
            )
            .await
            .unwrap();
        assert!(text(&result).contains("Merged 2 memories into"));
        assert!(tokio::fs::read_to_string(&one)
            .await
            .unwrap()
            .contains("status: archived"));
        assert!(tokio::fs::read_to_string(&two)
            .await
            .unwrap()
            .contains("status: archived"));
        assert!(
            std::fs::read_dir(dir.path().join(KNOWLEDGE_FOLDER_NAME))
                .unwrap()
                .count()
                >= 3
        );
    }
}
