use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

const DOCUMENTS_DIR: &str = "documents";
const HISTORY_DIR: &str = ".history";
const DELETED_DIR: &str = "_deleted";
const HISTORY_CAP: usize = 20;
const VALID_KINDS: [&str; 6] = ["plan", "design", "runbook", "brief", "postmortem", "spec"];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct DocumentFrontmatter {
    name: String,
    slug: String,
    kind: String,
    created_at: String,
    updated_at: String,
    author_role: String,
    pinned: bool,
    version: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relevant_cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskDocument {
    pub(crate) frontmatter: DocumentFrontmatter,
    pub(crate) content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryItem {
    version: u64,
    updated_at: String,
    snapshot_at: String,
    path: PathBuf,
    deleted: bool,
}

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn now_filename_stamp() -> String {
    now_rfc3339().replace(':', "-")
}

fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("slug cannot be empty".to_string());
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "slug must contain only alphanumeric characters, hyphens, or underscores".to_string(),
        );
    }
    Ok(())
}

fn validate_kind(kind: &str) -> Result<(), String> {
    if VALID_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(format!(
            "invalid kind `{}`. Must be one of: {}",
            kind,
            VALID_KINDS.join(", ")
        ))
    }
}

pub(crate) fn document_path(documents_dir: &Path, slug: &str) -> PathBuf {
    documents_dir.join(format!("{}.md", slug))
}

fn history_dir(documents_dir: &Path) -> PathBuf {
    documents_dir.join(HISTORY_DIR)
}

fn deleted_dir(documents_dir: &Path) -> PathBuf {
    history_dir(documents_dir).join(DELETED_DIR)
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn render_document(document: &TaskDocument) -> String {
    let fm = &document.frontmatter;
    let mut output = String::from("---\n");
    output.push_str(&format!("name: {}\n", yaml_string(&fm.name)));
    output.push_str(&format!("slug: {}\n", yaml_string(&fm.slug)));
    output.push_str(&format!("kind: {}\n", yaml_string(&fm.kind)));
    output.push_str(&format!("created_at: {}\n", yaml_string(&fm.created_at)));
    output.push_str(&format!("updated_at: {}\n", yaml_string(&fm.updated_at)));
    output.push_str(&format!("author_role: {}\n", yaml_string(&fm.author_role)));
    output.push_str(&format!("pinned: {}\n", fm.pinned));
    output.push_str(&format!("version: {}\n", fm.version));
    if !fm.relevant_cards.is_empty() {
        let cards = fm
            .relevant_cards
            .iter()
            .map(|card| yaml_string(card))
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("relevant_cards: [{}]\n", cards));
    }
    output.push_str("---\n\n");
    output.push_str(&document.content);
    output
}

fn split_frontmatter(raw: &str) -> Result<(&str, &str), String> {
    let rest = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
        .ok_or_else(|| "document is missing YAML frontmatter".to_string())?;
    if let Some(index) = rest.find("\n---\n") {
        return Ok((&rest[..index], &rest[index + 5..]));
    }
    if let Some(index) = rest.find("\r\n---\r\n") {
        return Ok((&rest[..index], &rest[index + 7..]));
    }
    Err("document frontmatter is not terminated".to_string())
}

fn parse_document(raw: &str) -> Result<TaskDocument, String> {
    let (frontmatter, content) = split_frontmatter(raw)?;
    let frontmatter: DocumentFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| format!("invalid frontmatter: {}", e))?;
    validate_slug(&frontmatter.slug)?;
    validate_kind(&frontmatter.kind)?;
    let content = content
        .strip_prefix("\r\n")
        .or_else(|| content.strip_prefix('\n'))
        .unwrap_or(content);
    Ok(TaskDocument {
        frontmatter,
        content: content.to_string(),
    })
}

async fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid destination path {}", path.display()))?;
    let tmp_path = path.with_file_name(format!(".{}.tmp", file_name));
    fs::write(&tmp_path, content).await.map_err(|e| {
        format!(
            "failed to write temporary file {}: {}",
            tmp_path.display(),
            e
        )
    })?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("failed to replace {}: {}", path.display(), e))?;
    }
    fs::rename(&tmp_path, path).await.map_err(|e| {
        format!(
            "failed to rename {} to {}: {}",
            tmp_path.display(),
            path.display(),
            e
        )
    })
}

async fn read_document(path: &Path) -> Result<TaskDocument, String> {
    let raw = fs::read_to_string(path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    parse_document(&raw)
}

async fn write_document(path: &Path, document: &TaskDocument) -> Result<(), String> {
    atomic_write(path, &render_document(document)).await
}

async fn list_history_files_in(
    dir: &Path,
    slug: &str,
    deleted: bool,
) -> Result<Vec<HistoryItem>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|e| format!("failed to read {}: {}", dir.display(), e))?;
    let prefix = format!("{}__", slug);
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read {}: {}", dir.display(), e))?
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with(&prefix) || !file_name.ends_with(".md") {
            continue;
        }
        let snapshot_at = file_name[prefix.len()..file_name.len() - 3].to_string();
        let document = read_document(&path).await?;
        items.push(HistoryItem {
            version: document.frontmatter.version,
            updated_at: document.frontmatter.updated_at,
            snapshot_at,
            path,
            deleted,
        });
    }
    items.sort_by(|a, b| a.snapshot_at.cmp(&b.snapshot_at));
    Ok(items)
}

async fn list_history_files(documents_dir: &Path, slug: &str) -> Result<Vec<HistoryItem>, String> {
    let mut items = list_history_files_in(&history_dir(documents_dir), slug, false).await?;
    items.extend(list_history_files_in(&deleted_dir(documents_dir), slug, true).await?);
    items.sort_by(|a, b| a.snapshot_at.cmp(&b.snapshot_at));
    Ok(items)
}

async fn cap_history(documents_dir: &Path, slug: &str) -> Result<(), String> {
    let mut items = list_history_files_in(&history_dir(documents_dir), slug, false).await?;
    if items.len() <= HISTORY_CAP {
        return Ok(());
    }
    items.sort_by(|a, b| a.snapshot_at.cmp(&b.snapshot_at));
    let remove_count = items.len() - HISTORY_CAP;
    for item in items.into_iter().take(remove_count) {
        fs::remove_file(&item.path).await.map_err(|e| {
            format!(
                "failed to remove old history {}: {}",
                item.path.display(),
                e
            )
        })?;
    }
    Ok(())
}

async fn snapshot_existing(documents_dir: &Path, slug: &str) -> Result<(), String> {
    let path = document_path(documents_dir, slug);
    if !path.exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let raw = fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    let history = history_dir(documents_dir);
    fs::create_dir_all(&history)
        .await
        .map_err(|e| format!("failed to create history directory: {}", e))?;
    let history_path = history.join(format!("{}__{}.md", slug, now_filename_stamp()));
    atomic_write(&history_path, &raw).await?;
    cap_history(documents_dir, slug).await
}

pub(crate) async fn create_document_at(
    documents_dir: &Path,
    slug: &str,
    name: &str,
    kind: &str,
    content: &str,
    pinned: bool,
    relevant_cards: Vec<String>,
    author_role: &str,
) -> Result<TaskDocument, String> {
    validate_slug(slug)?;
    validate_kind(kind)?;
    fs::create_dir_all(documents_dir)
        .await
        .map_err(|e| format!("failed to create documents directory: {}", e))?;
    let path = document_path(documents_dir, slug);
    if path.exists() {
        return Err(format!("document `{}` already exists", slug));
    }
    let now = now_rfc3339();
    let document = TaskDocument {
        frontmatter: DocumentFrontmatter {
            name: name.to_string(),
            slug: slug.to_string(),
            kind: kind.to_string(),
            created_at: now.clone(),
            updated_at: now,
            author_role: author_role.to_string(),
            pinned,
            version: 1,
            relevant_cards,
        },
        content: content.to_string(),
    };
    write_document(&path, &document).await?;
    Ok(document)
}

pub(crate) async fn next_available_slug_at(
    documents_dir: &Path,
    base_slug: &str,
) -> Result<String, String> {
    validate_slug(base_slug)?;
    if !document_path(documents_dir, base_slug).exists() {
        return Ok(base_slug.to_string());
    }
    for index in 2..=999 {
        let slug = format!("{}-{}", base_slug, index);
        if !document_path(documents_dir, &slug).exists() {
            return Ok(slug);
        }
    }
    Err(format!(
        "could not allocate an available slug for `{}`",
        base_slug
    ))
}

async fn list_documents_at(documents_dir: &Path) -> Result<Vec<TaskDocument>, String> {
    if !documents_dir.exists() {
        return Ok(Vec::new());
    }
    let mut documents = Vec::new();
    let mut entries = fs::read_dir(documents_dir)
        .await
        .map_err(|e| format!("failed to read {}: {}", documents_dir.display(), e))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read {}: {}", documents_dir.display(), e))?
    {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        documents.push(read_document(&path).await?);
    }
    documents.sort_by(|a, b| a.frontmatter.slug.cmp(&b.frontmatter.slug));
    Ok(documents)
}

async fn get_document_at(
    documents_dir: &Path,
    slug: &str,
    version: Option<u64>,
) -> Result<TaskDocument, String> {
    validate_slug(slug)?;
    let path = document_path(documents_dir, slug);
    if version.is_none() {
        return read_document(&path).await;
    }
    let version = version.unwrap();
    if path.exists() {
        let document = read_document(&path).await?;
        if document.frontmatter.version == version {
            return Ok(document);
        }
    }
    for item in list_history_files(documents_dir, slug).await? {
        let document = read_document(&item.path).await?;
        if document.frontmatter.version == version {
            return Ok(document);
        }
    }
    Err(format!("document `{}` version {} not found", slug, version))
}

async fn replace_document_content_at(
    documents_dir: &Path,
    slug: &str,
    content: String,
    pinned: Option<bool>,
) -> Result<TaskDocument, String> {
    validate_slug(slug)?;
    let path = document_path(documents_dir, slug);
    let mut document = read_document(&path).await?;
    snapshot_existing(documents_dir, slug).await?;
    document.content = content;
    if let Some(pinned) = pinned {
        document.frontmatter.pinned = pinned;
    }
    document.frontmatter.version += 1;
    document.frontmatter.updated_at = now_rfc3339();
    write_document(&path, &document).await?;
    Ok(document)
}

async fn update_document_at(
    documents_dir: &Path,
    slug: &str,
    content: &str,
) -> Result<TaskDocument, String> {
    replace_document_content_at(documents_dir, slug, content.to_string(), None).await
}

async fn append_document_at(
    documents_dir: &Path,
    slug: &str,
    section: &str,
) -> Result<TaskDocument, String> {
    let document = get_document_at(documents_dir, slug, None).await?;
    let trimmed = section.trim();
    let block = if trimmed.starts_with("## ") {
        trimmed.to_string()
    } else {
        format!("## Section\n\n{}", trimmed)
    };
    let mut content = document.content.trim_end_matches('\n').to_string();
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str(&block);
    content.push('\n');
    replace_document_content_at(documents_dir, slug, content, None).await
}

async fn pin_document_at(
    documents_dir: &Path,
    slug: &str,
    pinned: bool,
) -> Result<TaskDocument, String> {
    let document = get_document_at(documents_dir, slug, None).await?;
    replace_document_content_at(documents_dir, slug, document.content, Some(pinned)).await
}

async fn delete_document_at(documents_dir: &Path, slug: &str) -> Result<PathBuf, String> {
    validate_slug(slug)?;
    let path = document_path(documents_dir, slug);
    if !path.exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let deleted = deleted_dir(documents_dir);
    fs::create_dir_all(&deleted)
        .await
        .map_err(|e| format!("failed to create deleted history directory: {}", e))?;
    let deleted_path = deleted.join(format!("{}__{}.md", slug, now_filename_stamp()));
    fs::rename(&path, &deleted_path)
        .await
        .map_err(|e| format!("failed to move deleted document: {}", e))?;
    Ok(deleted_path)
}

async fn history_document_at(documents_dir: &Path, slug: &str) -> Result<Vec<HistoryItem>, String> {
    validate_slug(slug)?;
    list_history_files(documents_dir, slug).await
}

fn format_document(document: &TaskDocument) -> String {
    render_document(document)
}

fn format_doc_list(documents: &[TaskDocument]) -> String {
    if documents.is_empty() {
        return "No task documents found.".to_string();
    }
    let mut output = String::from(
        "| slug | name | kind | pinned | version | updated_at |\n|---|---|---|---|---:|---|\n",
    );
    for document in documents {
        let fm = &document.frontmatter;
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            fm.slug, fm.name, fm.kind, fm.pinned, fm.version, fm.updated_at
        ));
    }
    output
}

fn format_history(items: &[HistoryItem]) -> String {
    if items.is_empty() {
        return "No history found for document.".to_string();
    }
    let mut output = String::from(
        "| version | updated_at | snapshot_at | state | path |\n|---:|---|---|---|---|\n",
    );
    for item in items {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            item.version,
            item.updated_at,
            item.snapshot_at,
            if item.deleted { "deleted" } else { "history" },
            item.path.display()
        ));
    }
    output
}

async fn task_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<(Arc<GlobalContext>, String, String), String> {
    let ccx_lock = ccx.lock().await;
    let gcx = ccx_lock.app.gcx.clone();
    let task_id = ccx_lock
        .task_meta
        .as_ref()
        .map(|meta| meta.task_id.clone())
        .or_else(|| {
            args.get("task_id")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        })
        .ok_or_else(|| {
            "task document tools require task context or a task_id argument".to_string()
        })?;
    let author_role = match ccx_lock.task_meta.as_ref().map(|meta| meta.role.as_str()) {
        Some("planner") => "planner",
        Some("agents") | Some("agent") => "agents",
        _ => "user",
    }
    .to_string();
    Ok((gcx, task_id, author_role))
}

pub(crate) async fn documents_dir_for_task(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<PathBuf, String> {
    let task_dir = storage::find_task_dir(gcx, task_id).await?;
    Ok(task_dir.join(DOCUMENTS_DIR))
}

fn string_arg(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!("argument `{}` is not a string: {:?}", name, value)),
        None => Err(format!("argument `{}` is required", name)),
    }
}

fn optional_u64_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<u64>, String> {
    match args.get(name) {
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("argument `{}` must be a non-negative integer", name)),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("argument `{}` must be a non-negative integer", name)),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!(
            "argument `{}` must be a non-negative integer: {:?}",
            name, value
        )),
    }
}

fn optional_bool_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<bool>, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::String(value)) => match value.as_str() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            _ => Err(format!("argument `{}` must be true or false", name)),
        },
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{}` must be boolean: {:?}", name, value)),
    }
}

fn optional_cards_arg(args: &HashMap<String, Value>, name: &str) -> Result<Vec<String>, String> {
    match args.get(name) {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|value| value.trim().to_string())
                    .ok_or_else(|| format!("argument `{}` must contain only strings", name))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| {
                values
                    .into_iter()
                    .filter(|value| !value.is_empty())
                    .collect()
            }),
        Some(Value::String(value)) => Ok(value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(value) => Err(format!(
            "argument `{}` must be a string or string array: {:?}",
            name, value
        )),
    }
}

fn tool_message(tool_call_id: &String, content: String) -> Vec<ContextEnum> {
    vec![ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    })]
}

#[derive(Debug, Serialize)]
pub struct TaskDocumentSummary {
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub pinned: bool,
    pub version: u64,
    pub created_at: String,
    pub updated_at: String,
    pub author_role: String,
    pub relevant_cards: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskDocumentDetail {
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub pinned: bool,
    pub version: u64,
    pub created_at: String,
    pub updated_at: String,
    pub author_role: String,
    pub relevant_cards: Vec<String>,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct TaskDocumentListResponse {
    pub task_id: String,
    pub documents: Vec<TaskDocumentSummary>,
}

#[derive(Debug, Serialize)]
pub struct TaskDocumentHistoryEntry {
    pub version: u64,
    pub updated_at: String,
    pub author_role: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct TaskDocumentHistoryResponse {
    pub task_id: String,
    pub slug: String,
    pub history: Vec<TaskDocumentHistoryEntry>,
}

#[derive(Deserialize)]
pub struct CreateDocumentRequest {
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub content: String,
    pub pinned: Option<bool>,
    pub relevant_cards: Option<Vec<String>>,
}

fn document_to_summary(doc: &TaskDocument) -> TaskDocumentSummary {
    let fm = &doc.frontmatter;
    TaskDocumentSummary {
        slug: fm.slug.clone(),
        name: fm.name.clone(),
        kind: fm.kind.clone(),
        pinned: fm.pinned,
        version: fm.version,
        created_at: fm.created_at.clone(),
        updated_at: fm.updated_at.clone(),
        author_role: fm.author_role.clone(),
        relevant_cards: fm.relevant_cards.clone(),
    }
}

fn document_to_detail(doc: TaskDocument) -> TaskDocumentDetail {
    let fm = doc.frontmatter;
    TaskDocumentDetail {
        slug: fm.slug,
        name: fm.name,
        kind: fm.kind,
        pinned: fm.pinned,
        version: fm.version,
        created_at: fm.created_at,
        updated_at: fm.updated_at,
        author_role: fm.author_role,
        relevant_cards: fm.relevant_cards,
        content: doc.content,
    }
}

pub async fn list_task_documents_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<TaskDocumentListResponse, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    let mut documents = list_documents_at(&documents_dir).await?;
    documents.sort_by(|a, b| a.frontmatter.created_at.cmp(&b.frontmatter.created_at));
    Ok(TaskDocumentListResponse {
        task_id: task_id.to_string(),
        documents: documents.iter().map(document_to_summary).collect(),
    })
}

pub async fn get_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
    version: Option<u64>,
) -> Result<TaskDocumentDetail, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    if version.is_none() && !document_path(&documents_dir, slug).exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let document = get_document_at(&documents_dir, slug, version).await?;
    Ok(document_to_detail(document))
}

pub async fn create_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    request: CreateDocumentRequest,
) -> Result<TaskDocumentDetail, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    let document = create_document_at(
        &documents_dir,
        &request.slug,
        &request.name,
        &request.kind,
        &request.content,
        request.pinned.unwrap_or(false),
        request.relevant_cards.unwrap_or_default(),
        "user",
    )
    .await?;
    Ok(document_to_detail(document))
}

pub async fn update_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
    content: String,
) -> Result<TaskDocumentDetail, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    if !document_path(&documents_dir, slug).exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let document = update_document_at(&documents_dir, slug, &content).await?;
    Ok(document_to_detail(document))
}

pub async fn append_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
    section: String,
) -> Result<TaskDocumentDetail, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    if !document_path(&documents_dir, slug).exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let document = append_document_at(&documents_dir, slug, &section).await?;
    Ok(document_to_detail(document))
}

pub async fn delete_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
) -> Result<(), String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    delete_document_at(&documents_dir, slug).await?;
    Ok(())
}

pub async fn pin_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
    pinned: bool,
) -> Result<TaskDocumentDetail, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    if !document_path(&documents_dir, slug).exists() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let document = pin_document_at(&documents_dir, slug, pinned).await?;
    Ok(document_to_detail(document))
}

pub async fn history_task_document_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: &str,
) -> Result<TaskDocumentHistoryResponse, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    let items = history_document_at(&documents_dir, slug).await?;
    if !document_path(&documents_dir, slug).exists() && items.is_empty() {
        return Err(format!("document `{}` does not exist", slug));
    }
    let mut history = Vec::new();
    for item in &items {
        let document = read_document(&item.path).await?;
        let metadata = fs::metadata(&item.path)
            .await
            .map_err(|e| format!("failed to get file metadata: {}", e))?;
        history.push(TaskDocumentHistoryEntry {
            version: item.version,
            updated_at: item.updated_at.clone(),
            author_role: document.frontmatter.author_role,
            size_bytes: metadata.len(),
        });
    }
    Ok(TaskDocumentHistoryResponse {
        task_id: task_id.to_string(),
        slug: slug.to_string(),
        history,
    })
}

macro_rules! impl_new {
    ($tool:ident) => {
        impl $tool {
            pub fn new() -> Self {
                Self
            }
        }
    };
}

pub struct ToolDocList;
pub struct ToolDocGet;
pub struct ToolDocCreate;
pub struct ToolDocUpdate;
pub struct ToolDocAppend;
pub struct ToolDocDelete;
pub struct ToolDocPin;
pub struct ToolDocHistory;

impl_new!(ToolDocList);
impl_new!(ToolDocGet);
impl_new!(ToolDocCreate);
impl_new!(ToolDocUpdate);
impl_new!(ToolDocAppend);
impl_new!(ToolDocDelete);
impl_new!(ToolDocPin);
impl_new!(ToolDocHistory);

#[async_trait]
impl Tool for ToolDocList {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let documents = list_documents_at(&documents_dir).await?;
        Ok((
            false,
            tool_message(tool_call_id, format_doc_list(&documents)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_list".to_string(),
            display_name: "Task Documents List".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "List task documents as a markdown table with slug, name, kind, pinned, version, and updated_at.".to_string(),
            input_schema: json_schema_from_params(&[("task_id", "string", "Task ID (optional if in task context)")], &[]),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocGet {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let version = optional_u64_arg(args, "version")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let document = get_document_at(&documents_dir, &slug, version).await?;
        Ok((
            false,
            tool_message(tool_call_id, format_document(&document)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_get".to_string(),
            display_name: "Task Document Get".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "Get the full markdown content of a task document. Latest version is returned unless version is provided.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                    ("version", "number", "Historical version to retrieve"),
                ],
                &["slug"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocCreate {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, author_role) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let name = string_arg(args, "name")?;
        let kind = string_arg(args, "kind")?;
        let content = string_arg(args, "content")?;
        let pinned = optional_bool_arg(args, "pinned")?.unwrap_or(true);
        let relevant_cards = optional_cards_arg(args, "relevant_cards")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let document = create_document_at(
            &documents_dir,
            &slug,
            &name,
            &kind,
            &content,
            pinned,
            relevant_cards,
            &author_role,
        )
        .await?;
        Ok((
            false,
            tool_message(tool_call_id, format_document(&document)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_create".to_string(),
            display_name: "Task Document Create".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Create a new task document. Fails if the slug already exists."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    (
                        "slug",
                        "string",
                        "Document slug: alphanumeric, dash, underscore",
                    ),
                    ("name", "string", "Document display name"),
                    (
                        "kind",
                        "string",
                        "Document kind: plan, design, runbook, brief, postmortem, or spec",
                    ),
                    ("content", "string", "Markdown body content"),
                    (
                        "pinned",
                        "boolean",
                        "Whether the document should always inject; defaults to true",
                    ),
                    (
                        "relevant_cards",
                        "string",
                        "Comma-separated relevant card IDs",
                    ),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug", "name", "kind", "content"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocUpdate {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let content = string_arg(args, "content")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let document = update_document_at(&documents_dir, &slug, &content).await?;
        Ok((
            false,
            tool_message(tool_call_id, format_document(&document)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_update".to_string(),
            display_name: "Task Document Update".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Replace a task document body, snapshotting the old version first."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("content", "string", "New markdown body content"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug", "content"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocAppend {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let section = string_arg(args, "section")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let document = append_document_at(&documents_dir, &slug, &section).await?;
        Ok((
            false,
            tool_message(tool_call_id, format_document(&document)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_append".to_string(),
            display_name: "Task Document Append".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Append a markdown section block to a task document.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("section", "string", "Markdown section to append"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug", "section"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocDelete {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let deleted_path = delete_document_at(&documents_dir, &slug).await?;
        Ok((
            false,
            tool_message(
                tool_call_id,
                format!(
                    "Deleted document `{}`. Moved to {}",
                    slug,
                    deleted_path.display()
                ),
            ),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_delete".to_string(),
            display_name: "Task Document Delete".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Soft-delete a task document by moving it to document history."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocPin {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let pinned = optional_bool_arg(args, "pinned")?
            .ok_or_else(|| "argument `pinned` is required".to_string())?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let document = pin_document_at(&documents_dir, &slug, pinned).await?;
        Ok((
            false,
            tool_message(tool_call_id, format_document(&document)),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_pin".to_string(),
            display_name: "Task Document Pin".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Toggle whether a task document is pinned for automatic injection."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("pinned", "boolean", "Pinned state"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug", "pinned"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolDocHistory {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, task_id, _) = task_context(&ccx, args).await?;
        let slug = string_arg(args, "slug")?;
        let documents_dir = documents_dir_for_task(gcx, &task_id).await?;
        let items = history_document_at(&documents_dir, &slug).await?;
        Ok((false, tool_message(tool_call_id, format_history(&items))))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "doc_history".to_string(),
            display_name: "Task Document History".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "List historical versions of a task document.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("slug", "string", "Document slug"),
                    ("task_id", "string", "Task ID (optional if in task context)"),
                ],
                &["slug"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn temp_documents_dir() -> (TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".refact/tasks/task-1/documents");
        fs::create_dir_all(&dir).await.unwrap();
        (temp, dir)
    }

    #[test]
    fn slug_validation_accepts_alphanumeric_dash_underscore() {
        assert!(validate_slug("main-plan_1").is_ok());
        assert!(validate_slug("T-23").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("main plan").is_err());
        assert!(validate_slug("../plan").is_err());
        assert!(validate_slug("plan.md").is_err());
    }

    #[tokio::test]
    async fn create_get_and_list_round_trip() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "main-plan",
            "Main Plan",
            "plan",
            "Initial body",
            true,
            vec!["T-22".to_string(), "T-23".to_string()],
            "planner",
        )
        .await
        .unwrap();

        let doc = get_document_at(&dir, "main-plan", None).await.unwrap();
        assert_eq!(doc.frontmatter.slug, "main-plan");
        assert_eq!(doc.frontmatter.name, "Main Plan");
        assert_eq!(doc.frontmatter.kind, "plan");
        assert_eq!(doc.frontmatter.author_role, "planner");
        assert!(doc.frontmatter.pinned);
        assert_eq!(doc.frontmatter.version, 1);
        assert_eq!(doc.frontmatter.relevant_cards, vec!["T-22", "T-23"]);
        assert_eq!(doc.content, "Initial body");

        let docs = list_documents_at(&dir).await.unwrap();
        assert_eq!(docs.len(), 1);
        let table = format_doc_list(&docs);
        assert!(table.contains("| slug | name | kind | pinned | version | updated_at |"));
        assert!(table.contains("| main-plan | Main Plan | plan | true | 1 |"));
    }

    #[tokio::test]
    async fn next_available_slug_appends_version_for_collision() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "initial-plan",
            "Initial Plan",
            "plan",
            "v1",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap();
        create_document_at(
            &dir,
            "initial-plan-2",
            "Initial Plan",
            "plan",
            "v2",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap();

        let slug = next_available_slug_at(&dir, "initial-plan").await.unwrap();

        assert_eq!(slug, "initial-plan-3");
    }

    #[tokio::test]
    async fn update_creates_history_and_get_version_reads_historical_body() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "spec",
            "Spec",
            "spec",
            "v1",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap();
        update_document_at(&dir, "spec", "v2").await.unwrap();
        update_document_at(&dir, "spec", "v3").await.unwrap();

        let latest = get_document_at(&dir, "spec", None).await.unwrap();
        assert_eq!(latest.frontmatter.version, 3);
        assert_eq!(latest.content, "v3");

        let version_one = get_document_at(&dir, "spec", Some(1)).await.unwrap();
        assert_eq!(version_one.frontmatter.version, 1);
        assert_eq!(version_one.content, "v1");

        let history = history_document_at(&dir, "spec").await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[1].version, 2);
    }

    #[tokio::test]
    async fn history_cap_keeps_last_twenty_snapshots() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "runbook",
            "Runbook",
            "runbook",
            "v1",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap();
        for version in 2..=26 {
            update_document_at(&dir, "runbook", &format!("v{}", version))
                .await
                .unwrap();
        }

        let history = history_document_at(&dir, "runbook").await.unwrap();
        assert_eq!(history.len(), HISTORY_CAP);
        assert_eq!(history.first().unwrap().version, 6);
        assert_eq!(history.last().unwrap().version, 25);
        assert!(get_document_at(&dir, "runbook", Some(1)).await.is_err());
    }

    #[tokio::test]
    async fn append_and_pin_update_document_versions() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "brief",
            "Brief",
            "brief",
            "",
            true,
            Vec::new(),
            "agents",
        )
        .await
        .unwrap();
        let appended = append_document_at(&dir, "brief", "Notes body")
            .await
            .unwrap();
        assert_eq!(appended.frontmatter.version, 2);
        assert_eq!(appended.content, "## Section\n\nNotes body\n");

        let pinned = pin_document_at(&dir, "brief", false).await.unwrap();
        assert_eq!(pinned.frontmatter.version, 3);
        assert!(!pinned.frontmatter.pinned);

        let history = history_document_at(&dir, "brief").await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[1].version, 2);
    }

    #[tokio::test]
    async fn delete_moves_document_to_deleted_history() {
        let (_temp, dir) = temp_documents_dir().await;
        create_document_at(
            &dir,
            "postmortem",
            "Postmortem",
            "postmortem",
            "body",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap();

        let deleted_path = delete_document_at(&dir, "postmortem").await.unwrap();
        assert!(!document_path(&dir, "postmortem").exists());
        assert!(deleted_path.exists());
        assert!(deleted_path.starts_with(deleted_dir(&dir)));
        assert!(get_document_at(&dir, "postmortem", None).await.is_err());

        let history = history_document_at(&dir, "postmortem").await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].deleted);
        assert_eq!(history[0].version, 1);
    }

    #[tokio::test]
    async fn invalid_kind_is_rejected() {
        let (_temp, dir) = temp_documents_dir().await;
        let err = create_document_at(
            &dir,
            "bad-kind",
            "Bad Kind",
            "memo",
            "body",
            true,
            Vec::new(),
            "planner",
        )
        .await
        .unwrap_err();
        assert!(err.contains("invalid kind"));
    }

    use crate::tasks::{
        storage,
        types::{TaskMeta, TaskStatus},
    };

    async fn setup_task_gcx(task_id: &str) -> (TempDir, Arc<GlobalContext>) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks").join(task_id);
        fs::create_dir_all(&task_dir).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: TaskStatus::Planning,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        (temp, gcx)
    }

    #[tokio::test]
    async fn list_task_documents_for_api_returns_summaries_in_creation_order() {
        let (_temp, gcx) = setup_task_gcx("task-docs-list").await;
        let tid = "task-docs-list";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "doc-a".to_string(),
                name: "A".to_string(),
                kind: "plan".to_string(),
                content: "body-a".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "doc-b".to_string(),
                name: "B".to_string(),
                kind: "spec".to_string(),
                content: "body-b".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "doc-c".to_string(),
                name: "C".to_string(),
                kind: "brief".to_string(),
                content: "body-c".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        let result = list_task_documents_for_api(gcx, tid).await.unwrap();
        assert_eq!(result.task_id, tid);
        assert_eq!(result.documents.len(), 3);
        let slugs: Vec<&str> = result.documents.iter().map(|d| d.slug.as_str()).collect();
        assert_eq!(slugs, vec!["doc-a", "doc-b", "doc-c"]);
    }

    #[tokio::test]
    async fn get_task_document_for_api_returns_detail_with_content() {
        let (_temp, gcx) = setup_task_gcx("task-docs-get").await;
        let tid = "task-docs-get";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "my-doc".to_string(),
                name: "My Doc".to_string(),
                kind: "plan".to_string(),
                content: "hello content".to_string(),
                pinned: Some(true),
                relevant_cards: Some(vec!["T-1".to_string()]),
            },
        )
        .await
        .unwrap();
        let detail = get_task_document_for_api(gcx, tid, "my-doc", None)
            .await
            .unwrap();
        assert_eq!(detail.slug, "my-doc");
        assert_eq!(detail.name, "My Doc");
        assert_eq!(detail.content, "hello content");
        assert!(detail.pinned);
        assert_eq!(detail.relevant_cards, vec!["T-1"]);
    }

    #[tokio::test]
    async fn create_task_document_for_api_rejects_duplicate_slug() {
        let (_temp, gcx) = setup_task_gcx("task-docs-dup").await;
        let tid = "task-docs-dup";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "dup".to_string(),
                name: "Dup".to_string(),
                kind: "spec".to_string(),
                content: "first".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        let err = create_task_document_for_api(
            gcx,
            tid,
            CreateDocumentRequest {
                slug: "dup".to_string(),
                name: "Dup Again".to_string(),
                kind: "spec".to_string(),
                content: "second".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("already exists"), "error was: {}", err);
    }

    #[tokio::test]
    async fn update_task_document_for_api_increments_version_and_writes_history() {
        let (_temp, gcx) = setup_task_gcx("task-docs-upd").await;
        let tid = "task-docs-upd";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "upd".to_string(),
                name: "Upd".to_string(),
                kind: "plan".to_string(),
                content: "v1".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        let updated = update_task_document_for_api(gcx.clone(), tid, "upd", "v2".to_string())
            .await
            .unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(updated.content, "v2");
        let hist = history_task_document_for_api(gcx, tid, "upd")
            .await
            .unwrap();
        assert!(!hist.history.is_empty());
        assert_eq!(hist.history[0].version, 1);
    }

    #[tokio::test]
    async fn pin_task_document_for_api_flips_pinned_flag() {
        let (_temp, gcx) = setup_task_gcx("task-docs-pin").await;
        let tid = "task-docs-pin";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "pin-doc".to_string(),
                name: "Pin Doc".to_string(),
                kind: "brief".to_string(),
                content: "content".to_string(),
                pinned: Some(true),
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        let unpinned = pin_task_document_for_api(gcx.clone(), tid, "pin-doc", false)
            .await
            .unwrap();
        assert!(!unpinned.pinned);
        let repinned = pin_task_document_for_api(gcx, tid, "pin-doc", true)
            .await
            .unwrap();
        assert!(repinned.pinned);
    }

    #[tokio::test]
    async fn delete_task_document_for_api_removes_document() {
        let (_temp, gcx) = setup_task_gcx("task-docs-del").await;
        let tid = "task-docs-del";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "del-doc".to_string(),
                name: "Del Doc".to_string(),
                kind: "spec".to_string(),
                content: "content".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        delete_task_document_for_api(gcx.clone(), tid, "del-doc")
            .await
            .unwrap();
        let result = list_task_documents_for_api(gcx, tid).await.unwrap();
        assert!(result.documents.iter().all(|d| d.slug != "del-doc"));
    }

    #[tokio::test]
    async fn history_task_document_for_api_returns_entries_for_each_version() {
        let (_temp, gcx) = setup_task_gcx("task-docs-hist").await;
        let tid = "task-docs-hist";
        create_task_document_for_api(
            gcx.clone(),
            tid,
            CreateDocumentRequest {
                slug: "hist-doc".to_string(),
                name: "Hist".to_string(),
                kind: "runbook".to_string(),
                content: "v1".to_string(),
                pinned: None,
                relevant_cards: None,
            },
        )
        .await
        .unwrap();
        update_task_document_for_api(gcx.clone(), tid, "hist-doc", "v2".to_string())
            .await
            .unwrap();
        update_task_document_for_api(gcx.clone(), tid, "hist-doc", "v3".to_string())
            .await
            .unwrap();
        let hist = history_task_document_for_api(gcx, tid, "hist-doc")
            .await
            .unwrap();
        assert_eq!(hist.history.len(), 2);
        assert_eq!(hist.history[0].version, 1);
        assert_eq!(hist.history[1].version, 2);
    }
}
