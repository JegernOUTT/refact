use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AMutex;
use tracing::info;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::knowledge_index::{build_knowledge_index, KnowledgeSearchFilters, KnowledgeSearchHit};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage::find_task_dir;
use crate::tools::task_tool_helpers::resolve_readonly_task_id;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};

const MEMORIES_DIR: &str = "memories";
const ARCHIVED_MEMORIES_DIR: &str = "archived";
const MEMORY_INBOX_CURSOR_FILE: &str = ".mem_inbox_cursor";
const DEFAULT_INBOX_LIMIT: usize = 20;
const STALE_PROGRESS_DAYS: i64 = 7;
const MAX_MEMORIES_CHARS: usize = 120_000;
const MAX_DUPLICATE_MEMORIES: usize = 500;
const MAX_DUPLICATE_COMPARISONS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Decision,
    Spec,
    Finding,
    Gotcha,
    Risk,
    Handoff,
    Progress,
    Postmortem,
    Brief,
    Freeform,
}

impl MemoryKind {
    fn values() -> &'static [&'static str] {
        &[
            "decision",
            "spec",
            "finding",
            "gotcha",
            "risk",
            "handoff",
            "progress",
            "postmortem",
            "brief",
            "freeform",
        ]
    }
}

impl Default for MemoryKind {
    fn default() -> Self {
        Self::Freeform
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Decision => "decision",
            Self::Spec => "spec",
            Self::Finding => "finding",
            Self::Gotcha => "gotcha",
            Self::Risk => "risk",
            Self::Handoff => "handoff",
            Self::Progress => "progress",
            Self::Postmortem => "postmortem",
            Self::Brief => "brief",
            Self::Freeform => "freeform",
        };
        f.write_str(value)
    }
}

impl FromStr for MemoryKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "decision" => Ok(Self::Decision),
            "spec" => Ok(Self::Spec),
            "finding" => Ok(Self::Finding),
            "gotcha" => Ok(Self::Gotcha),
            "risk" => Ok(Self::Risk),
            "handoff" => Ok(Self::Handoff),
            "progress" => Ok(Self::Progress),
            "postmortem" => Ok(Self::Postmortem),
            "brief" => Ok(Self::Brief),
            "freeform" => Ok(Self::Freeform),
            other => Err(format!(
                "Invalid memory kind `{}`. Expected one of: {}",
                other,
                Self::values().join(", ")
            )),
        }
    }
}

impl Serialize for MemoryKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MemoryKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryNamespace {
    Global,
    Task,
    Card(String),
    Agent(String),
}

impl MemoryNamespace {
    fn values() -> &'static [&'static str] {
        &["global", "task", "card:<card-id>", "agent:<agent-id>"]
    }
}

impl Default for MemoryNamespace {
    fn default() -> Self {
        Self::Task
    }
}

impl fmt::Display for MemoryNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => f.write_str("global"),
            Self::Task => f.write_str("task"),
            Self::Card(card_id) => write!(f, "card:{}", card_id),
            Self::Agent(agent_id) => write!(f, "agent:{}", agent_id),
        }
    }
}

impl FromStr for MemoryNamespace {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered == "global" {
            return Ok(Self::Global);
        }
        if lowered == "task" {
            return Ok(Self::Task);
        }
        if lowered.starts_with("card:") {
            let id = trimmed[5..].trim();
            if id.is_empty() {
                return Err("Invalid memory namespace `card:`. Card id cannot be empty".to_string());
            }
            return Ok(Self::Card(id.to_string()));
        }
        if lowered.starts_with("agent:") {
            let id = trimmed[6..].trim();
            if id.is_empty() {
                return Err(
                    "Invalid memory namespace `agent:`. Agent id cannot be empty".to_string(),
                );
            }
            return Ok(Self::Agent(id.to_string()));
        }
        Err(format!(
            "Invalid memory namespace `{}`. Expected one of: {}",
            trimmed,
            Self::values().join(", ")
        ))
    }
}

impl Serialize for MemoryNamespace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MemoryNamespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatus {
    Active,
    Archived,
    Superseded,
}

impl MemoryStatus {
    fn values() -> &'static [&'static str] {
        &["active", "archived", "superseded"]
    }
}

impl Default for MemoryStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl fmt::Display for MemoryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Superseded => "superseded",
        };
        f.write_str(value)
    }
}

impl FromStr for MemoryStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "superseded" => Ok(Self::Superseded),
            other => Err(format!(
                "Invalid memory status `{}`. Expected one of: {}",
                other,
                Self::values().join(", ")
            )),
        }
    }
}

impl Serialize for MemoryStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MemoryStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskMemoryFrontmatter {
    pub created_at: Option<String>,
    pub task_id: Option<String>,
    pub role: Option<String>,
    pub agent_id: Option<String>,
    pub card_id: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub kind: MemoryKind,
    pub namespace: MemoryNamespace,
    pub pinned: bool,
    pub supersedes: Option<String>,
    pub status: MemoryStatus,
}

impl Default for TaskMemoryFrontmatter {
    fn default() -> Self {
        Self {
            created_at: None,
            task_id: None,
            role: None,
            agent_id: None,
            card_id: None,
            title: None,
            tags: Vec::new(),
            kind: MemoryKind::default(),
            namespace: MemoryNamespace::default(),
            pinned: false,
            supersedes: None,
            status: MemoryStatus::default(),
        }
    }
}

impl TaskMemoryFrontmatter {
    pub fn from_yaml(frontmatter: &str) -> Result<Self, String> {
        let mapping = if frontmatter.trim().is_empty() {
            YamlMapping::new()
        } else {
            match serde_yaml::from_str::<YamlValue>(frontmatter)
                .map_err(|e| format!("Failed to parse memory frontmatter: {}", e))?
            {
                YamlValue::Mapping(mapping) => mapping,
                YamlValue::Null => YamlMapping::new(),
                _ => return Err("Memory frontmatter must be a YAML mapping".to_string()),
            }
        };

        let kind = yaml_string(&mapping, "kind")
            .map(|value| value.parse::<MemoryKind>())
            .transpose()?
            .unwrap_or_default();
        let namespace = yaml_string(&mapping, "namespace")
            .map(|value| value.parse::<MemoryNamespace>())
            .transpose()?
            .unwrap_or_default();
        let status = yaml_string(&mapping, "status")
            .map(|value| value.parse::<MemoryStatus>())
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            created_at: yaml_string(&mapping, "created_at"),
            task_id: yaml_string(&mapping, "task_id"),
            role: yaml_string(&mapping, "role"),
            agent_id: yaml_string(&mapping, "agent_id"),
            card_id: yaml_string(&mapping, "card_id"),
            title: yaml_string(&mapping, "title"),
            tags: yaml_string_list(&mapping, "tags")?,
            kind,
            namespace,
            pinned: yaml_bool(&mapping, "pinned")?.unwrap_or(false),
            supersedes: yaml_string(&mapping, "supersedes"),
            status,
        })
    }

    pub fn to_yaml_block(&self) -> String {
        let mut frontmatter = String::from("---\n");
        push_yaml_string(&mut frontmatter, "created_at", self.created_at.as_deref());
        push_yaml_string(&mut frontmatter, "task_id", self.task_id.as_deref());
        push_yaml_string(&mut frontmatter, "role", self.role.as_deref());
        push_yaml_string(&mut frontmatter, "agent_id", self.agent_id.as_deref());
        push_yaml_string(&mut frontmatter, "card_id", self.card_id.as_deref());
        push_yaml_string(&mut frontmatter, "title", self.title.as_deref());
        if !self.tags.is_empty() {
            let tags = self
                .tags
                .iter()
                .map(|tag| yaml_scalar(tag))
                .collect::<Vec<_>>()
                .join(", ");
            frontmatter.push_str(&format!("tags: [{}]\n", tags));
        }
        if self.kind != MemoryKind::default() {
            frontmatter.push_str(&format!("kind: {}\n", self.kind));
        }
        if self.namespace != MemoryNamespace::default() {
            frontmatter.push_str(&format!(
                "namespace: {}\n",
                yaml_scalar(&self.namespace.to_string())
            ));
        }
        if self.pinned {
            frontmatter.push_str("pinned: true\n");
        }
        push_yaml_string(&mut frontmatter, "supersedes", self.supersedes.as_deref());
        if self.status != MemoryStatus::default() {
            frontmatter.push_str(&format!("status: {}\n", self.status));
        }
        frontmatter.push_str("---");
        frontmatter
    }
}

fn mapping_value<'a>(mapping: &'a YamlMapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(&YamlValue::String(key.to_string()))
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
    mapping_value(mapping, key).and_then(yaml_value_string)
}

fn yaml_string_list(mapping: &YamlMapping, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = mapping_value(mapping, key) else {
        return Ok(Vec::new());
    };
    match value {
        YamlValue::Sequence(values) => values
            .iter()
            .map(|value| {
                yaml_value_string(value)
                    .ok_or_else(|| format!("Memory frontmatter `{}` entries must be strings", key))
            })
            .collect(),
        YamlValue::String(value) => Ok(value
            .split(',')
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect()),
        _ => Err(format!(
            "Memory frontmatter `{}` must be a string or string list",
            key
        )),
    }
}

fn yaml_bool(mapping: &YamlMapping, key: &str) -> Result<Option<bool>, String> {
    let Some(value) = mapping_value(mapping, key) else {
        return Ok(None);
    };
    match value {
        YamlValue::Bool(value) => Ok(Some(*value)),
        YamlValue::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            _ => Err(format!("Memory frontmatter `{}` must be a boolean", key)),
        },
        _ => Err(format!("Memory frontmatter `{}` must be a boolean", key)),
    }
}

fn push_yaml_string(frontmatter: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        frontmatter.push_str(&format!("{}: {}\n", key, yaml_scalar(value)));
    }
}

fn yaml_scalar(value: &str) -> String {
    let safe = !value.is_empty()
        && value.trim() == value
        && !matches!(value, "true" | "false" | "null" | "~")
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '/' | '.' | '@'));
    if safe {
        value.to_string()
    } else {
        yaml_quote(value)
    }
}

fn yaml_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn split_memory_frontmatter(content: &str) -> Result<(Option<&str>, &str), String> {
    let delimiter_len = if content.starts_with("---\r\n") {
        5
    } else if content.starts_with("---\n") {
        4
    } else {
        return Ok((None, content));
    };

    let mut position = delimiter_len;
    while position < content.len() {
        let line_end = content[position..]
            .find('\n')
            .map(|offset| position + offset + 1)
            .unwrap_or(content.len());
        let line = &content[position..line_end];
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]).trim();
        if trimmed == "---" {
            return Ok((
                Some(&content[delimiter_len..position]),
                &content[line_end..],
            ));
        }
        position = line_end;
    }

    Err("Invalid memory file: missing closing frontmatter delimiter".to_string())
}

fn parse_memory_file(content: &str) -> Result<(TaskMemoryFrontmatter, String), String> {
    let (frontmatter_text, body) = split_memory_frontmatter(content)?;
    let frontmatter = TaskMemoryFrontmatter::from_yaml(frontmatter_text.unwrap_or(""))?;
    Ok((frontmatter, body.trim_start_matches('\n').to_string()))
}

fn render_memory_file(frontmatter: &TaskMemoryFrontmatter, body: &str) -> String {
    format!(
        "{}\n\n{}",
        frontmatter.to_yaml_block(),
        body.trim_start_matches('\n')
    )
}

fn resolve_memory_namespace(
    namespace_arg: Option<&str>,
    card_id: Option<&str>,
) -> Result<MemoryNamespace, String> {
    if let Some(namespace) = namespace_arg {
        if namespace.trim().is_empty() {
            return Ok(MemoryNamespace::default());
        }
        return namespace.parse();
    }
    if let Some(card_id) = card_id {
        return Ok(MemoryNamespace::Card(card_id.to_string()));
    }
    Ok(MemoryNamespace::default())
}

fn optional_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{}` is not a string: {:?}", name, value)),
    }
}

fn optional_bool_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<bool>, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{}` is not a boolean: {:?}", name, value)),
    }
}

fn required_string_arg(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    optional_string_arg(args, name)?.ok_or_else(|| format!("argument `{}` is required", name))
}

fn required_bool_arg(args: &HashMap<String, Value>, name: &str) -> Result<bool, String> {
    optional_bool_arg(args, name)?.ok_or_else(|| format!("argument `{}` is required", name))
}

fn optional_usize_arg(
    args: &HashMap<String, Value>,
    name: &str,
    default: usize,
) -> Result<usize, String> {
    match args.get(name) {
        Some(Value::Number(value)) => value
            .as_u64()
            .map(|value| value as usize)
            .ok_or_else(|| format!("argument `{}` must be a non-negative integer", name)),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(default),
        Some(Value::String(value)) => value
            .parse::<usize>()
            .map_err(|_| format!("argument `{}` must be a non-negative integer", name)),
        Some(Value::Null) | None => Ok(default),
        Some(value) => Err(format!(
            "argument `{}` must be a non-negative integer: {:?}",
            name, value
        )),
    }
}

fn optional_string_list_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Vec<String>, String> {
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

fn safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn planner_task_id_from_meta(
    task_meta: Option<&crate::chat::types::TaskMeta>,
    tool_name: &str,
) -> Result<String, String> {
    let meta = task_meta.ok_or_else(|| {
        format!(
            "{} requires task planner context (task_id missing).",
            tool_name
        )
    })?;
    if meta.role != "planner" {
        return Err(format!(
            "{} can only be called by the task planner.",
            tool_name
        ));
    }
    Ok(meta.task_id.clone())
}

async fn memory_read_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<String, String> {
    if crate::tools::task_tool_helpers::optional_id_string(args, "task_id")?.is_some() {
        return resolve_readonly_task_id(ccx, args, tool_name).await;
    }
    let cgcx = ccx.lock().await;
    cgcx.task_meta
        .as_ref()
        .map(|meta| meta.task_id.clone())
        .ok_or_else(|| {
            format!(
                "{} requires task context (task_id missing). This tool only works within task chats unless task_id is provided.",
                tool_name
            )
        })
}

fn validate_memory_reference(reference: &str) -> Result<String, String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err("memory_id cannot be empty".to_string());
    }
    if reference.contains('/') || reference.contains('\\') {
        return Err("memory_id must be a filename or slug without path separators".to_string());
    }
    let mut components = Path::new(reference).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(reference.to_string()),
        _ => Err("memory_id must be a filename or slug without path separators".to_string()),
    }
}

fn is_memory_markdown_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md" | "mdx")
    )
}

fn memory_slug_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.splitn(4, '_').collect();
    if parts.len() == 4 && !parts[3].is_empty() {
        Some(parts[3].to_string())
    } else {
        Some(stem.to_string())
    }
}

fn memory_short_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.splitn(4, '_').collect();
    if parts.len() == 4 && !parts[2].is_empty() {
        Some(parts[2].to_string())
    } else {
        None
    }
}

fn memory_reference_stem(reference: &str) -> String {
    Path::new(reference)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(reference)
        .to_string()
}

fn task_memory_cursor_path(task_dir: &Path) -> PathBuf {
    task_dir.join(MEMORY_INBOX_CURSOR_FILE)
}

async fn read_memory_inbox_cursor(task_dir: &Path) -> Result<Option<DateTime<Utc>>, String> {
    let path = task_memory_cursor_path(task_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read memory inbox cursor: {}", e))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_rfc3339_utc(trimmed).map(Some)
}

async fn write_memory_inbox_cursor(task_dir: &Path, cursor: DateTime<Utc>) -> Result<(), String> {
    fs::create_dir_all(task_dir)
        .await
        .map_err(|e| format!("Failed to create task directory: {}", e))?;
    atomic_write_text(&task_memory_cursor_path(task_dir), &cursor.to_rfc3339()).await
}

pub fn parse_rfc3339_utc(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|e| format!("Invalid rfc3339 timestamp `{}`: {}", value, e))
}

fn memory_created_at(frontmatter: &TaskMemoryFrontmatter) -> Option<DateTime<Utc>> {
    frontmatter
        .created_at
        .as_deref()
        .and_then(|value| parse_rfc3339_utc(value).ok())
}

#[derive(Debug, Clone)]
struct TaskMemoryInboxEntry {
    path: PathBuf,
    frontmatter: TaskMemoryFrontmatter,
    body: String,
    created_at: DateTime<Utc>,
    created_at_known: bool,
}

impl TaskMemoryInboxEntry {
    fn memory_id(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string()
    }

    fn title(&self) -> String {
        self.frontmatter
            .title
            .clone()
            .or_else(|| {
                self.body.lines().find_map(|line| {
                    line.trim()
                        .strip_prefix("# ")
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                })
            })
            .or_else(|| memory_slug_from_path(&self.path))
            .unwrap_or_else(|| "memory".to_string())
    }
}

#[derive(Debug, Clone)]
struct DuplicateMemoryPair {
    left_path: PathBuf,
    right_path: PathBuf,
    overlap_percent: usize,
}

#[derive(Debug, Clone)]
struct SkippedMemoryWarning {
    path: PathBuf,
    error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryApiEntry {
    pub filename: String,
    pub created_at: String,
    pub created_at_known: bool,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub kind: MemoryKind,
    pub namespace: MemoryNamespace,
    pub pinned: bool,
    pub status: MemoryStatus,
    pub role: Option<String>,
    pub agent_id: Option<String>,
    pub card_id: Option<String>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryApiWarning {
    pub filename: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoriesApiResponse {
    pub task_id: String,
    pub since: String,
    pub new_count: usize,
    pub memories: Vec<TaskMemoryApiEntry>,
    pub warnings: Vec<TaskMemoryApiWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryFacetsResponse {
    pub task_id: String,
    pub namespaces: Vec<String>,
    pub tags: Vec<String>,
    pub kinds: Vec<String>,
    pub total_count: usize,
    pub pinned_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskMemoryListFilters {
    pub since: Option<DateTime<Utc>>,
    pub kind: Option<MemoryKind>,
    pub namespace: Option<MemoryNamespace>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryPinApiResponse {
    pub ok: bool,
    pub filename: String,
    pub pinned: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryArchiveApiResponse {
    pub ok: bool,
    pub filename: String,
    pub archived_filename: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMemoryTriageApiResponse {
    pub ok: bool,
    pub cursor: String,
}

async fn memory_file_mtime(path: &Path) -> Option<DateTime<Utc>> {
    fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
}

async fn memory_entry_timestamp(
    path: &Path,
    frontmatter: &TaskMemoryFrontmatter,
) -> (DateTime<Utc>, bool) {
    if let Some(created_at) = memory_created_at(frontmatter) {
        return (created_at, true);
    }
    if let Some(modified_at) = memory_file_mtime(path).await {
        return (modified_at, true);
    }
    (DateTime::<Utc>::from(std::time::UNIX_EPOCH), false)
}

async fn load_task_memory_inbox_entries(
    memories_dir: &Path,
) -> Result<(Vec<TaskMemoryInboxEntry>, Vec<SkippedMemoryWarning>), String> {
    if !memories_dir.exists() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut memories = Vec::new();
    let mut warnings = Vec::new();
    for entry in WalkDir::new(memories_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path().to_path_buf();
        if !path.is_file() || !is_memory_markdown_file(&path) {
            continue;
        }
        let content = match fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(error) => {
                warnings.push(SkippedMemoryWarning {
                    path: path.clone(),
                    error: format!("Failed to read memory: {}", error),
                });
                continue;
            }
        };
        let (frontmatter, body) = match parse_memory_file(&content) {
            Ok(memory) => memory,
            Err(error) => {
                warnings.push(SkippedMemoryWarning {
                    path: path.clone(),
                    error,
                });
                continue;
            }
        };
        if matches!(
            frontmatter.status,
            MemoryStatus::Archived | MemoryStatus::Superseded
        ) {
            continue;
        }
        let (created_at, created_at_known) = memory_entry_timestamp(&path, &frontmatter).await;
        memories.push(TaskMemoryInboxEntry {
            path,
            frontmatter,
            body,
            created_at,
            created_at_known,
        });
    }
    memories.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.created_at_known.cmp(&a.created_at_known))
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok((memories, warnings))
}

fn new_memories_since(
    memories: &[TaskMemoryInboxEntry],
    cursor: DateTime<Utc>,
    limit: usize,
) -> Vec<TaskMemoryInboxEntry> {
    memories
        .iter()
        .filter(|memory| memory.created_at_known && memory.created_at > cursor)
        .take(limit)
        .cloned()
        .collect()
}

fn stale_memory_candidates(
    memories: &[TaskMemoryInboxEntry],
    now: DateTime<Utc>,
) -> Vec<TaskMemoryInboxEntry> {
    memories
        .iter()
        .filter(|memory| {
            memory.created_at_known
                && memory.frontmatter.kind == MemoryKind::Progress
                && now.signed_duration_since(memory.created_at)
                    > Duration::days(STALE_PROGRESS_DAYS)
                && memory.frontmatter.namespace != MemoryNamespace::Global
        })
        .cloned()
        .collect()
}

fn memory_body_without_frontmatter(content: &str) -> String {
    split_memory_frontmatter(content)
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|_| content.to_string())
}

fn content_tokens(content: &str) -> HashSet<String> {
    memory_body_without_frontmatter(content)
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 3)
        .collect()
}

fn token_overlap_percent_from_sets(left: &HashSet<String>, right: &HashSet<String>) -> usize {
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    let overlap = left.intersection(&right).count();
    let denominator = left.union(&right).count();
    overlap * 100 / denominator
}

fn duplicate_memory_pairs(memories: &[TaskMemoryInboxEntry]) -> Vec<DuplicateMemoryPair> {
    let token_sets = memories
        .iter()
        .take(MAX_DUPLICATE_MEMORIES)
        .map(|memory| (memory.path.clone(), content_tokens(&memory.body)))
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    let mut comparisons = 0usize;
    'outer: for left_idx in 0..token_sets.len() {
        for right_idx in (left_idx + 1)..token_sets.len() {
            if comparisons >= MAX_DUPLICATE_COMPARISONS {
                break 'outer;
            }
            comparisons += 1;
            let overlap_percent =
                token_overlap_percent_from_sets(&token_sets[left_idx].1, &token_sets[right_idx].1);
            if overlap_percent > 70 {
                pairs.push(DuplicateMemoryPair {
                    left_path: token_sets[left_idx].0.clone(),
                    right_path: token_sets[right_idx].0.clone(),
                    overlap_percent,
                });
            }
        }
    }
    pairs.sort_by(|a, b| b.overlap_percent.cmp(&a.overlap_percent));
    pairs
}

fn format_memory_age(now: DateTime<Utc>, timestamp: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(timestamp);
    if duration.num_days() >= 1 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() >= 1 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() >= 1 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

fn format_cursor_age(now: DateTime<Utc>, cursor: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(cursor);
    if duration.num_days() >= 1 {
        format!("{} days ago", duration.num_days())
    } else if duration.num_hours() >= 1 {
        format!("{} hours ago", duration.num_hours())
    } else if duration.num_minutes() >= 1 {
        format!("{} minutes ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

fn render_memory_entry_line(memory: &TaskMemoryInboxEntry, now: DateTime<Utc>) -> String {
    let card = memory
        .frontmatter
        .card_id
        .clone()
        .or_else(|| match &memory.frontmatter.namespace {
            MemoryNamespace::Card(card_id) => Some(card_id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "task".to_string());
    let short_id = memory_short_id_from_path(&memory.path)
        .or_else(|| memory_slug_from_path(&memory.path))
        .unwrap_or_else(|| memory.memory_id());
    let age = if memory.created_at_known {
        format_memory_age(now, memory.created_at)
    } else {
        "unknown age".to_string()
    };
    format!(
        "- {} | {}-{} | {} | \"{}\" [{}]",
        memory.frontmatter.kind,
        card,
        short_id,
        age,
        memory.title().replace('"', "'"),
        memory.path.display()
    )
}

fn render_memory_inbox(
    cursor: DateTime<Utc>,
    now: DateTime<Utc>,
    new_memories: &[TaskMemoryInboxEntry],
    stale_candidates: &[TaskMemoryInboxEntry],
    duplicate_pairs: &[DuplicateMemoryPair],
    warnings: &[SkippedMemoryWarning],
) -> String {
    let mut output = String::from("# Memory Inbox\n\n");
    output.push_str(&format!(
        "## New since {} ({})\n",
        format_cursor_age(now, cursor),
        new_memories.len()
    ));
    if new_memories.is_empty() {
        output.push_str("- No new memories.\n");
    } else {
        for memory in new_memories {
            output.push_str(&render_memory_entry_line(memory, now));
            output.push('\n');
        }
    }

    output.push_str(&format!(
        "\n## Stale candidates ({})\n",
        stale_candidates.len()
    ));
    if stale_candidates.is_empty() {
        output.push_str("- No stale candidates.\n");
    } else {
        for memory in stale_candidates {
            let age = if memory.created_at_known {
                format_memory_age(now, memory.created_at)
            } else {
                "unknown age".to_string()
            };
            output.push_str(&format!(
                "- {} | {} | {} — consider archive [{}]\n",
                memory.frontmatter.kind,
                memory.memory_id(),
                age,
                memory.path.display()
            ));
        }
    }

    output.push_str(&format!(
        "\n## Possible duplicates ({} pairs)\n",
        duplicate_pairs.len()
    ));
    if duplicate_pairs.is_empty() {
        output.push_str("- No likely duplicates.\n");
    } else {
        for pair in duplicate_pairs {
            output.push_str(&format!(
                "- [{}] vs [{}] — {}% token overlap\n",
                pair.left_path.display(),
                pair.right_path.display(),
                pair.overlap_percent
            ));
        }
    }

    output.push_str(&format!("\n## Warnings ({} skipped)\n", warnings.len()));
    if warnings.is_empty() {
        output.push_str("- No inbox warnings.\n");
    } else {
        for warning in warnings {
            output.push_str(&format!(
                "- [{}] — {}\n",
                warning.path.display(),
                warning.error
            ));
        }
    }

    output.push_str(
        "\n## Actions\n- task_mem_pin(memory_id) to keep one forever\n- task_mem_archive(memory_id) to hide from auto-inject\n- task_mem_save(content=\"...\", supersedes=\"<old>\") to replace\n- task_mem_triage_done() when finished\n",
    );
    output
}

fn memory_matches_search(memory: &TaskMemoryInboxEntry, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    let filename = memory.memory_id().to_ascii_lowercase();
    let title = memory.title().to_ascii_lowercase();
    let body = memory.body.to_ascii_lowercase();
    let tags = memory
        .frontmatter
        .tags
        .iter()
        .any(|tag| tag.to_ascii_lowercase().contains(&query));
    filename.contains(&query) || title.contains(&query) || body.contains(&query) || tags
}

fn memory_to_api_entry(memory: TaskMemoryInboxEntry) -> TaskMemoryApiEntry {
    TaskMemoryApiEntry {
        filename: memory.memory_id(),
        created_at: memory.created_at.to_rfc3339(),
        created_at_known: memory.created_at_known,
        title: memory.title(),
        content: memory.body,
        tags: memory.frontmatter.tags,
        kind: memory.frontmatter.kind,
        namespace: memory.frontmatter.namespace,
        pinned: memory.frontmatter.pinned,
        status: memory.frontmatter.status,
        role: memory.frontmatter.role,
        agent_id: memory.frontmatter.agent_id,
        card_id: memory.frontmatter.card_id,
        supersedes: memory.frontmatter.supersedes,
    }
}

pub async fn list_task_memories_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    filters: TaskMemoryListFilters,
) -> Result<TaskMemoriesApiResponse, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let now = Utc::now();
    let since = filters
        .since
        .or(read_memory_inbox_cursor(&task_dir).await?)
        .unwrap_or_else(|| now - Duration::hours(24));
    let (mut memories, warnings) =
        load_task_memory_inbox_entries(&task_dir.join(MEMORIES_DIR)).await?;
    let new_count = memories
        .iter()
        .filter(|memory| memory.created_at_known && memory.created_at > since)
        .count();

    memories.retain(|memory| {
        if let Some(kind) = filters.kind {
            if memory.frontmatter.kind != kind {
                return false;
            }
        }
        if let Some(namespace) = &filters.namespace {
            if &memory.frontmatter.namespace != namespace {
                return false;
            }
        }
        if let Some(search) = &filters.search {
            if !memory_matches_search(memory, search) {
                return false;
            }
        }
        true
    });

    Ok(TaskMemoriesApiResponse {
        task_id: task_id.to_string(),
        since: since.to_rfc3339(),
        new_count,
        memories: memories.into_iter().map(memory_to_api_entry).collect(),
        warnings: warnings
            .into_iter()
            .map(|warning| TaskMemoryApiWarning {
                filename: warning
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                error: warning.error,
            })
            .collect(),
    })
}

pub async fn task_memory_facets_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<TaskMemoryFacetsResponse, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let (memories, _warnings) =
        load_task_memory_inbox_entries(&task_dir.join(MEMORIES_DIR)).await?;
    let total_count = memories.len();
    let pinned_count = memories.iter().filter(|m| m.frontmatter.pinned).count();
    let mut namespaces: Vec<String> = memories
        .iter()
        .map(|m| m.frontmatter.namespace.to_string())
        .collect();
    namespaces.sort();
    namespaces.dedup();
    let mut tags: Vec<String> = memories
        .iter()
        .flat_map(|m| m.frontmatter.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    let mut kinds: Vec<String> = memories
        .iter()
        .map(|m| m.frontmatter.kind.to_string())
        .collect();
    kinds.sort();
    kinds.dedup();
    Ok(TaskMemoryFacetsResponse {
        task_id: task_id.to_string(),
        namespaces,
        tags,
        kinds,
        total_count,
        pinned_count,
    })
}

pub async fn set_task_memory_pinned_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    filename: &str,
    pinned: bool,
) -> Result<TaskMemoryPinApiResponse, String> {
    let memories_dir = get_task_memories_dir(gcx, task_id).await?;
    let (path, changed) = set_task_memory_pinned(&memories_dir, filename, pinned).await?;
    Ok(TaskMemoryPinApiResponse {
        ok: true,
        filename: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(filename)
            .to_string(),
        pinned,
        changed,
    })
}

pub async fn archive_task_memory_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    filename: &str,
) -> Result<TaskMemoryArchiveApiResponse, String> {
    let memories_dir = get_task_memories_dir(gcx, task_id).await?;
    let (source_path, dest_path) = archive_task_memory(&memories_dir, filename).await?;
    Ok(TaskMemoryArchiveApiResponse {
        ok: true,
        filename: source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(filename)
            .to_string(),
        archived_filename: dest_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(filename)
            .to_string(),
    })
}

pub async fn mark_task_memories_triaged_for_api(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    cursor: Option<DateTime<Utc>>,
) -> Result<TaskMemoryTriageApiResponse, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let cursor = cursor.unwrap_or_else(Utc::now);
    write_memory_inbox_cursor(&task_dir, cursor).await?;
    Ok(TaskMemoryTriageApiResponse {
        ok: true,
        cursor: cursor.to_rfc3339(),
    })
}

async fn find_task_memory_path(
    search_dir: &Path,
    reference: &str,
    scope_label: &str,
) -> Result<PathBuf, String> {
    let reference = validate_memory_reference(reference)?;
    if !search_dir.exists() {
        return Err(format!(
            "No {} directory found: {}",
            scope_label,
            search_dir.display()
        ));
    }

    let reference_stem = memory_reference_stem(&reference);
    let mut exact_matches = Vec::new();
    let mut slug_matches = Vec::new();
    let mut available = Vec::new();

    for entry in WalkDir::new(search_dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if !path.is_file() || !is_memory_markdown_file(path) {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        available.push(file_name.clone());
        if file_name == reference {
            exact_matches.push(path.to_path_buf());
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let slug = memory_slug_from_path(path).unwrap_or_default();
        let short_id = memory_short_id_from_path(path).unwrap_or_default();
        if stem == reference
            || stem == reference_stem
            || slug == reference
            || slug == reference_stem
            || short_id == reference
            || short_id == reference_stem
        {
            slug_matches.push(path.to_path_buf());
        }
    }

    if let Some(path) = exact_matches.into_iter().next() {
        return Ok(path);
    }

    if slug_matches.len() == 1 {
        return Ok(slug_matches.remove(0));
    }
    if slug_matches.len() > 1 {
        let matches = slug_matches
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "memory_id `{}` matches multiple {} files: {}",
            reference, scope_label, matches
        ));
    }

    available.sort();
    let available = if available.is_empty() {
        "none".to_string()
    } else {
        available
            .into_iter()
            .take(10)
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(format!(
        "Memory not found: `{}` in {} at {}. Available files: {}",
        reference,
        scope_label,
        search_dir.display(),
        available
    ))
}

async fn rewrite_memory_frontmatter_path<F>(
    path: &Path,
    update: F,
) -> Result<(TaskMemoryFrontmatter, bool), String>
where
    F: FnOnce(&mut TaskMemoryFrontmatter) -> bool,
{
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read memory {}: {}", path.display(), e))?;
    let (mut frontmatter, body) = parse_memory_file(&content)?;
    let changed = update(&mut frontmatter);
    if changed {
        let updated = render_memory_file(&frontmatter, &body);
        atomic_write_text(path, &updated).await?;
    }
    Ok((frontmatter, changed))
}

async fn set_task_memory_pinned(
    memories_dir: &Path,
    memory_id: &str,
    pinned: bool,
) -> Result<(PathBuf, bool), String> {
    let path = find_task_memory_path(memories_dir, memory_id, "active task memories").await?;
    let (_, changed) = rewrite_memory_frontmatter_path(&path, |frontmatter| {
        if frontmatter.pinned == pinned {
            false
        } else {
            frontmatter.pinned = pinned;
            true
        }
    })
    .await?;
    Ok((path, changed))
}

async fn move_task_memory_with_status(
    source_dir: &Path,
    dest_dir: &Path,
    memory_id: &str,
    status: MemoryStatus,
    source_label: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let source_path = find_task_memory_path(source_dir, memory_id, source_label).await?;
    fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("Failed to create memory destination directory: {}", e))?;
    let file_name = source_path
        .file_name()
        .ok_or_else(|| "Invalid memory path: missing file name".to_string())?;
    let dest_path = dest_dir.join(file_name);
    if dest_path.exists() {
        return Err(format!(
            "Cannot move memory because destination already exists: {}",
            dest_path.display()
        ));
    }
    rewrite_memory_frontmatter_path(&source_path, |frontmatter| {
        frontmatter.status = status;
        true
    })
    .await?;
    fs::rename(&source_path, &dest_path)
        .await
        .map_err(|e| format!("Failed to move memory with atomic rename: {}", e))?;
    Ok((source_path, dest_path))
}

async fn archive_task_memory(
    memories_dir: &Path,
    memory_id: &str,
) -> Result<(PathBuf, PathBuf), String> {
    move_task_memory_with_status(
        memories_dir,
        &memories_dir.join(ARCHIVED_MEMORIES_DIR),
        memory_id,
        MemoryStatus::Archived,
        "active task memories",
    )
    .await
}

async fn unarchive_task_memory(
    memories_dir: &Path,
    memory_id: &str,
) -> Result<(PathBuf, PathBuf), String> {
    move_task_memory_with_status(
        &memories_dir.join(ARCHIVED_MEMORIES_DIR),
        memories_dir,
        memory_id,
        MemoryStatus::Active,
        "archived task memories",
    )
    .await
}

fn task_memory_tool_output(tool_call_id: &String, output: String) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(output),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

async fn find_superseded_memory_path(
    memories_dir: &Path,
    reference: &str,
) -> Result<PathBuf, String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err("supersedes cannot be empty".to_string());
    }

    let reference_path = Path::new(reference);
    if !safe_relative_path(reference_path) {
        return Err(
            "supersedes must be a filename or relative path inside the task memories directory"
                .to_string(),
        );
    }

    let direct_path = memories_dir.join(reference_path);
    if direct_path.is_file() {
        return Ok(direct_path);
    }

    if reference_path.components().count() == 1 {
        for entry in WalkDir::new(memories_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(reference) {
                return Ok(path.to_path_buf());
            }
        }
    }

    Err(format!(
        "Memory to supersede not found: {} in {}",
        reference,
        memories_dir.display()
    ))
}

async fn mark_memory_superseded_path(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).await.map_err(|e| {
        format!(
            "Failed to read memory to supersede {}: {}",
            path.display(),
            e
        )
    })?;
    let (mut frontmatter, body) = parse_memory_file(&content)?;
    frontmatter.status = MemoryStatus::Superseded;
    let updated = render_memory_file(&frontmatter, &body);
    atomic_write_text(path, &updated).await
}

async fn mark_memory_superseded(memories_dir: &Path, reference: &str) -> Result<PathBuf, String> {
    let path = find_superseded_memory_path(memories_dir, reference).await?;
    mark_memory_superseded_path(&path).await?;
    Ok(path)
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

pub async fn get_task_memories_dir(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<PathBuf, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    Ok(task_dir.join(MEMORIES_DIR))
}

fn generate_memory_filename(title: Option<&str>, content: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let short_uuid = &Uuid::new_v4().to_string()[..8];

    let slug = title
        .or_else(|| content.lines().next())
        .unwrap_or("memory")
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .take(40)
        .collect::<String>();

    if slug.is_empty() {
        format!("{}_{}_{}.md", timestamp, short_uuid, "memory")
    } else {
        format!("{}_{}_{}.md", timestamp, short_uuid, slug)
    }
}

pub struct ToolTaskMemorySave;

impl ToolTaskMemorySave {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMemorySave {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_save".to_string(),
            display_name: "Save Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Saves a typed memory/note for the current task. Use this to record decisions, specs, findings, risks, handoffs, progress, gotchas, or any useful information that should be shared with other agents and future planner iterations. Memories are automatically injected into all task chats.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("content", "string", "The content to save. Can be markdown formatted."),
                    ("title", "string", "Optional title for the memory (used in filename)."),
                    ("tags", "string", "Optional comma-separated tags for categorization."),
                    ("kind", "string", "Optional memory kind: decision, spec, finding, gotcha, risk, handoff, progress, postmortem, brief, or freeform. Defaults to freeform."),
                    ("namespace", "string", "Optional namespace: global, task, card:T-N, or agent:A-id. Defaults to task, or card:{card_id} inside task-agent card context."),
                    ("pinned", "boolean", "If true, mark this memory as pinned. Defaults to false."),
                    ("supersedes", "string", "Optional filename or relative path of an existing memory to mark as superseded."),
                ],
                &["content"],
            ),
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
        let (gcx, task_meta) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.task_meta.clone())
        };

        let task_id = task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .ok_or("task_mem_save requires task context (task_id missing). This tool only works within task planner/agent chats.")?;

        let content = match args.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `content` is not a string: {:?}", v)),
            None => return Err("argument `content` is required".to_string()),
        };

        if content.trim().is_empty() {
            return Err("content cannot be empty".to_string());
        }

        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let role = task_meta
            .as_ref()
            .map(|m| m.role.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let agent_id = task_meta.as_ref().and_then(|m| m.agent_id.clone());
        let card_id = task_meta.as_ref().and_then(|m| m.card_id.clone());
        let kind = optional_string_arg(args, "kind")?
            .map(|value| value.parse::<MemoryKind>())
            .transpose()?
            .unwrap_or_default();
        let namespace = resolve_memory_namespace(
            optional_string_arg(args, "namespace")?.as_deref(),
            card_id.as_deref(),
        )?;
        let pinned = optional_bool_arg(args, "pinned")?.unwrap_or(false);
        let supersedes = optional_string_arg(args, "supersedes")?;

        let memories_dir = get_task_memories_dir(gcx.clone(), &task_id).await?;
        fs::create_dir_all(&memories_dir)
            .await
            .map_err(|e| format!("Failed to create memories directory: {}", e))?;

        if let Some(supersedes) = &supersedes {
            mark_memory_superseded(&memories_dir, supersedes).await?;
        }

        let filename = generate_memory_filename(title.as_deref(), &content);
        let file_path = memories_dir.join(&filename);

        let frontmatter = TaskMemoryFrontmatter {
            created_at: Some(Utc::now().to_rfc3339()),
            task_id: Some(task_id.clone()),
            role: Some(role.clone()),
            agent_id: agent_id.clone(),
            card_id: card_id.clone(),
            title: title.clone(),
            tags,
            kind,
            namespace: namespace.clone(),
            pinned,
            supersedes: supersedes.clone(),
            status: MemoryStatus::Active,
        };

        let body = if let Some(t) = &title {
            format!("# {}\n\n{}", t, content)
        } else {
            content
        };
        let full_content = render_memory_file(&frontmatter, &body);

        atomic_write_text(&file_path, &full_content)
            .await
            .map_err(|e| format!("Failed to write memory file: {}", e))?;

        info!("Task memory saved: {}", file_path.display());

        let mut result = format!(
            "Memory saved successfully.\nFile: {}\nTask: {}\nRole: {}\nKind: {}\nNamespace: {}",
            file_path.display(),
            task_id,
            role,
            kind,
            namespace
        );
        if let Some(supersedes) = &supersedes {
            result.push_str(&format!("\nSupersedes: {}", supersedes));
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub struct ToolTaskMemoriesGet;

impl ToolTaskMemoriesGet {
    pub fn new() -> Self {
        Self
    }
}

pub struct ToolTaskMemorySearch;

impl ToolTaskMemorySearch {
    pub fn new() -> Self {
        Self
    }
}

pub struct ToolTaskMemoryPin;

impl ToolTaskMemoryPin {
    pub fn new() -> Self {
        Self
    }
}

/// `task_mem_archive` marks frontmatter as `status: archived` and moves the file into
/// the task memory `archived/` directory.
pub struct ToolTaskMemoryArchive;

impl ToolTaskMemoryArchive {
    pub fn new() -> Self {
        Self
    }
}

pub struct ToolTaskMemoryUnarchive;

impl ToolTaskMemoryUnarchive {
    pub fn new() -> Self {
        Self
    }
}

pub struct ToolTaskMemoryInbox;

impl ToolTaskMemoryInbox {
    pub fn new() -> Self {
        Self
    }
}

pub struct ToolTaskMemoryTriageDone;

impl ToolTaskMemoryTriageDone {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMemoryInbox {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_inbox".to_string(),
            display_name: "Task Memory Inbox".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that shows new task memories since the last triage cursor, stale progress candidates, and likely duplicate memories.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("since", "string", "Optional rfc3339 timestamp overriding the saved triage cursor."),
                    ("limit", "number", "Maximum number of new memories to render. Defaults to 20."),
                ],
                &[],
            ),
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
        let (gcx, task_id) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                planner_task_id_from_meta(cgcx.task_meta.as_ref(), "task_mem_inbox")?,
            )
        };
        let task_dir = find_task_dir(gcx, &task_id).await?;
        let now = Utc::now();
        let cursor = if let Some(since) = optional_string_arg(args, "since")? {
            parse_rfc3339_utc(&since)?
        } else {
            read_memory_inbox_cursor(&task_dir)
                .await?
                .unwrap_or_else(|| now - Duration::hours(24))
        };
        let limit = optional_usize_arg(args, "limit", DEFAULT_INBOX_LIMIT)?.min(100);
        let (memories, warnings) =
            load_task_memory_inbox_entries(&task_dir.join(MEMORIES_DIR)).await?;
        let new_memories = new_memories_since(&memories, cursor, limit);
        let stale_candidates = stale_memory_candidates(&memories, now);
        let duplicate_pairs = duplicate_memory_pairs(&memories);
        let output = render_memory_inbox(
            cursor,
            now,
            &new_memories,
            &stale_candidates,
            &duplicate_pairs,
            &warnings,
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMemoryTriageDone {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_triage_done".to_string(),
            display_name: "Task Memory Triage Done".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that updates the memory inbox triage cursor."
                .to_string(),
            input_schema: json_schema_from_params(
                &[(
                    "cursor",
                    "string",
                    "Optional rfc3339 timestamp. Defaults to now.",
                )],
                &[],
            ),
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
        let (gcx, task_id) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                planner_task_id_from_meta(cgcx.task_meta.as_ref(), "task_mem_triage_done")?,
            )
        };
        let task_dir = find_task_dir(gcx, &task_id).await?;
        let cursor = optional_string_arg(args, "cursor")?
            .map(|value| parse_rfc3339_utc(&value))
            .transpose()?
            .unwrap_or_else(Utc::now);
        write_memory_inbox_cursor(&task_dir, cursor).await?;
        Ok(task_memory_tool_output(
            tool_call_id,
            format!(
                "Memory inbox triage cursor updated.\nTask: {}\nCursor: {}",
                task_id,
                cursor.to_rfc3339()
            ),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMemoryPin {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_pin".to_string(),
            display_name: "Pin Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that pins or unpins a task memory by filename or slug."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("memory_id", "string", "Memory filename or short slug."),
                    ("pinned", "boolean", "Whether the memory should be pinned."),
                ],
                &["memory_id", "pinned"],
            ),
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
        let (gcx, task_id) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                planner_task_id_from_meta(cgcx.task_meta.as_ref(), "task_mem_pin")?,
            )
        };
        let memory_id = required_string_arg(args, "memory_id")?;
        let pinned = required_bool_arg(args, "pinned")?;
        let memories_dir = get_task_memories_dir(gcx, &task_id).await?;
        let (path, changed) = set_task_memory_pinned(&memories_dir, &memory_id, pinned).await?;
        info!(
            "Task memory pin updated: {} pinned={} changed={}",
            path.display(),
            pinned,
            changed
        );
        let output = if changed {
            format!(
                "Memory pin updated.\nFile: {}\nPinned: {}",
                path.display(),
                pinned
            )
        } else {
            format!(
                "Memory pin unchanged.\nFile: {}\nPinned: {}",
                path.display(),
                pinned
            )
        };
        Ok(task_memory_tool_output(tool_call_id, output))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMemoryArchive {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_archive".to_string(),
            display_name: "Archive Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that archives a task memory by filename or slug."
                .to_string(),
            input_schema: json_schema_from_params(
                &[("memory_id", "string", "Memory filename or short slug.")],
                &["memory_id"],
            ),
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
        let (gcx, task_id) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                planner_task_id_from_meta(cgcx.task_meta.as_ref(), "task_mem_archive")?,
            )
        };
        let memory_id = required_string_arg(args, "memory_id")?;
        let memories_dir = get_task_memories_dir(gcx, &task_id).await?;
        let (source_path, dest_path) = archive_task_memory(&memories_dir, &memory_id).await?;
        info!(
            "Task memory archived: {} -> {}",
            source_path.display(),
            dest_path.display()
        );
        Ok(task_memory_tool_output(
            tool_call_id,
            format!(
                "Memory archived.\nFrom: {}\nTo: {}",
                source_path.display(),
                dest_path.display()
            ),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMemoryUnarchive {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_unarchive".to_string(),
            display_name: "Unarchive Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description:
                "Planner-only tool that restores an archived task memory by filename or slug."
                    .to_string(),
            input_schema: json_schema_from_params(
                &[(
                    "memory_id",
                    "string",
                    "Archived memory filename or short slug.",
                )],
                &["memory_id"],
            ),
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
        let (gcx, task_id) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                planner_task_id_from_meta(cgcx.task_meta.as_ref(), "task_mem_unarchive")?,
            )
        };
        let memory_id = required_string_arg(args, "memory_id")?;
        let memories_dir = get_task_memories_dir(gcx, &task_id).await?;
        let (source_path, dest_path) = unarchive_task_memory(&memories_dir, &memory_id).await?;
        info!(
            "Task memory unarchived: {} -> {}",
            source_path.display(),
            dest_path.display()
        );
        Ok(task_memory_tool_output(
            tool_call_id,
            format!(
                "Memory unarchived.\nFrom: {}\nTo: {}",
                source_path.display(),
                dest_path.display()
            ),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMemoriesGet {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_get".to_string(),
            display_name: "Get Task Memories".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Retrieves all saved memories for the current task. Returns the content of all memory files from the task's memories folder.".to_string(),
            input_schema: json_schema_from_params(&[("format", "string", "Output format: 'full' (default) returns all content, 'titles' returns only titles/filenames, 'paths' returns only file paths."), ("task_id", "string", "Optional task id. Defaults to current task context when available.")], &[]),
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
        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };
        let task_id = memory_read_task_id(&ccx, args, "task_mem_get").await?;

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("full");

        let memories_dir = get_task_memories_dir(gcx.clone(), &task_id).await?;

        if !memories_dir.exists() {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("No task memories found.".to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let mut memories: Vec<(PathBuf, String)> = Vec::new();

        for entry in WalkDir::new(&memories_dir)
            .max_depth(1)
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

            match fs::read_to_string(path).await {
                Ok(content) => memories.push((path.to_path_buf(), content)),
                Err(e) => {
                    tracing::warn!("Failed to read memory file {:?}: {}", path, e);
                }
            }
        }

        memories.sort_by(|a, b| b.0.cmp(&a.0));

        if memories.is_empty() {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("No task memories found.".to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let result = match format {
            "paths" => {
                let paths: Vec<String> = memories
                    .iter()
                    .map(|(p, _)| p.display().to_string())
                    .collect();
                format!("## Task Memories ({})\n\n{}", paths.len(), paths.join("\n"))
            }
            "titles" => {
                let titles: Vec<String> = memories
                    .iter()
                    .map(|(p, content)| {
                        let title = content
                            .lines()
                            .find(|l| l.starts_with("# ") || l.starts_with("title:"))
                            .map(|l| {
                                l.trim_start_matches("# ")
                                    .trim_start_matches("title:")
                                    .trim()
                            })
                            .unwrap_or_else(|| {
                                p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown")
                            });
                        format!(
                            "- {} ({})",
                            title,
                            p.file_name().unwrap_or_default().to_string_lossy()
                        )
                    })
                    .collect();
                format!(
                    "## Task Memories ({})\n\n{}",
                    titles.len(),
                    titles.join("\n")
                )
            }
            _ => {
                let mut output = format!("## Task Memories ({})\n\n", memories.len());
                let mut total_chars = output.len();

                for (path, content) in &memories {
                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                    let entry = format!("--- file: {} ---\n{}\n\n", filename, content);

                    if total_chars + entry.len() > MAX_MEMORIES_CHARS {
                        output.push_str(&format!(
                            "\n[TRUNCATED: {} more memories not shown. Use format='paths' to see all.]\n",
                            memories.len() - memories.iter().position(|(p, _)| p == path).unwrap_or(0)
                        ));
                        break;
                    }

                    output.push_str(&entry);
                    total_chars += entry.len();
                }

                output
            }
        };

        info!(
            "Task memories retrieved: {} files for task {}",
            memories.len(),
            task_id
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn task_search_filters_from_args(
    args: &HashMap<String, Value>,
    task_id: Option<String>,
) -> Result<KnowledgeSearchFilters, String> {
    let kind = optional_string_arg(args, "kind")?;
    let namespace = optional_string_arg(args, "namespace")?;
    Ok(KnowledgeSearchFilters {
        scope: Some("task".to_string()),
        kind,
        namespace,
        task_id,
        tags: optional_string_list_arg(args, "tags")?,
    })
}

fn format_task_memory_hits(hits: &[KnowledgeSearchHit]) -> String {
    let items: Vec<Value> = hits
        .iter()
        .map(|hit| {
            let namespace = hit
                .card
                .tags
                .iter()
                .find_map(|tag| tag.strip_prefix("namespace:"))
                .map(str::to_string);
            serde_json::json!({
                "path": hit.card.file_path.display().to_string(),
                "title": hit.card.title.clone(),
                "kind": hit.card.kind.clone(),
                "namespace": namespace,
                "tags": hit.card.tags.clone(),
                "snippet": hit.snippet,
                "score": hit.score,
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
}

pub async fn search_task_memories_and_documents(
    gcx: Arc<GlobalContext>,
    query: &str,
    filters: KnowledgeSearchFilters,
    top_k: usize,
) -> Vec<KnowledgeSearchHit> {
    let index = build_knowledge_index(gcx.clone()).await;
    let hits = index.search(query, &filters, top_k);
    *gcx.knowledge_index.lock().await = index;
    hits
}

#[async_trait]
impl Tool for ToolTaskMemorySearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mem_search".to_string(),
            display_name: "Search Task Memory".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Search task memories and task documents by tag, filename, and content text. Does not use VecDB.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("query", "string", "Text, tag, or filename query."),
                    ("kind", "string", "Optional memory/document kind filter."),
                    ("namespace", "string", "Optional namespace filter, such as task or card:T-22."),
                    ("tags", "string", "Optional comma-separated tags that every result must have."),
                    ("top_k", "number", "Maximum results to return. Defaults to 10."),
                    ("task_id", "string", "Optional task id. Defaults to current task context when available."),
                ],
                &["query"],
            ),
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
        let (gcx, task_meta) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.task_meta.clone())
        };
        let query = match args.get("query") {
            Some(Value::String(value)) => value.trim().to_string(),
            Some(value) => return Err(format!("argument `query` is not a string: {:?}", value)),
            None => return Err("argument `query` is required".to_string()),
        };
        if query.is_empty() {
            return Err("query cannot be empty".to_string());
        }
        let task_id =
            if crate::tools::task_tool_helpers::optional_id_string(args, "task_id")?.is_some() {
                Some(resolve_readonly_task_id(&ccx, args, "task_mem_search").await?)
            } else {
                task_meta
                    .as_ref()
                    .map(|meta| meta.task_id.clone())
                    .or_else(|| Some("*".to_string()))
            };
        let filters = task_search_filters_from_args(args, task_id)?;
        let top_k = optional_usize_arg(args, "top_k", 10)?.clamp(1, 50);
        let hits = search_task_memories_and_documents(gcx, &query, filters, top_k).await;
        let output = if hits.is_empty() {
            "[]".to_string()
        } else {
            format_task_memory_hits(&hits)
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub async fn load_task_memories(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<Vec<(PathBuf, String)>, String> {
    let memories_dir = get_task_memories_dir(gcx, task_id).await?;

    if !memories_dir.exists() {
        return Ok(vec![]);
    }

    let mut memories: Vec<(PathBuf, String)> = Vec::new();

    for entry in WalkDir::new(&memories_dir)
        .max_depth(1)
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

        match fs::read_to_string(path).await {
            Ok(content) => memories.push((path.to_path_buf(), content)),
            Err(e) => {
                tracing::warn!("Failed to read task memory file {:?}: {}", path, e);
            }
        }
    }

    memories.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(memories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::chat::types::TaskMeta;
    use crate::tools::tools_description::Tool;
    use chrono::TimeZone;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex as AMutex;

    const MEMORY_FILE: &str = "2026-05-22_023548_edf49905_master-plan.md";

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    async fn write_memory(dir: &Path, file_name: &str, frontmatter: TaskMemoryFrontmatter) {
        write_memory_with_body(dir, file_name, frontmatter, "Body").await;
    }

    async fn write_memory_with_body(
        dir: &Path,
        file_name: &str,
        frontmatter: TaskMemoryFrontmatter,
        body: &str,
    ) {
        tokio::fs::create_dir_all(dir).await.unwrap();
        tokio::fs::write(dir.join(file_name), render_memory_file(&frontmatter, body))
            .await
            .unwrap();
    }

    async fn read_frontmatter(path: &Path) -> TaskMemoryFrontmatter {
        let text = tokio::fs::read_to_string(path).await.unwrap();
        parse_memory_file(&text).unwrap().0
    }

    async fn make_ccx(gcx: Arc<GlobalContext>, role: &str) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "planner-chat".to_string(),
                None,
                "model".to_string(),
                Some(TaskMeta {
                    task_id: "task-1".to_string(),
                    role: role.to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: None,
                }),
                None,
            )
            .await,
        ))
    }

    async fn make_unbound_ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "unbound-chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    async fn make_task_with_memory(
        role: &str,
    ) -> (
        tempfile::TempDir,
        Arc<GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
        PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        write_memory(
            &memories_dir,
            MEMORY_FILE,
            TaskMemoryFrontmatter {
                task_id: Some("task-1".to_string()),
                title: Some("Master Plan".to_string()),
                ..Default::default()
            },
        )
        .await;
        let ccx = make_ccx(gcx.clone(), role).await;
        (temp, gcx, ccx, memories_dir.join(MEMORY_FILE))
    }

    fn inbox_memory(
        path: &str,
        created_at: DateTime<Utc>,
        kind: MemoryKind,
        namespace: MemoryNamespace,
        body: &str,
    ) -> TaskMemoryInboxEntry {
        TaskMemoryInboxEntry {
            path: PathBuf::from(path),
            frontmatter: TaskMemoryFrontmatter {
                created_at: Some(created_at.to_rfc3339()),
                kind,
                namespace,
                ..Default::default()
            },
            body: body.to_string(),
            created_at,
            created_at_known: true,
        }
    }

    #[test]
    fn memory_enums_parse_display_round_trip() {
        for kind in [
            MemoryKind::Decision,
            MemoryKind::Spec,
            MemoryKind::Finding,
            MemoryKind::Gotcha,
            MemoryKind::Risk,
            MemoryKind::Handoff,
            MemoryKind::Progress,
            MemoryKind::Postmortem,
            MemoryKind::Brief,
            MemoryKind::Freeform,
        ] {
            assert_eq!(kind.to_string().parse::<MemoryKind>().unwrap(), kind);
        }

        for namespace in [
            MemoryNamespace::Global,
            MemoryNamespace::Task,
            MemoryNamespace::Card("T-1".to_string()),
            MemoryNamespace::Agent("A-1".to_string()),
        ] {
            assert_eq!(
                namespace.to_string().parse::<MemoryNamespace>().unwrap(),
                namespace
            );
        }

        for status in [
            MemoryStatus::Active,
            MemoryStatus::Archived,
            MemoryStatus::Superseded,
        ] {
            assert_eq!(status.to_string().parse::<MemoryStatus>().unwrap(), status);
        }
    }

    #[test]
    fn memory_namespace_card_serializes_with_prefix() {
        let namespace = MemoryNamespace::Card("T-N".to_string());
        let value = serde_json::to_value(&namespace).unwrap();
        assert_eq!(value, json!("card:T-N"));
        let parsed: MemoryNamespace = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, namespace);
    }

    #[test]
    fn legacy_memory_file_loads_new_fields_with_defaults() {
        let content = "---\ncreated_at: 2026-05-22T00:00:00Z\ntask_id: task-1\nrole: agents\ntags: [old, memory]\n---\n\nLegacy body";
        let (frontmatter, body) = parse_memory_file(content).unwrap();

        assert_eq!(frontmatter.kind, MemoryKind::Freeform);
        assert_eq!(frontmatter.namespace, MemoryNamespace::Task);
        assert!(!frontmatter.pinned);
        assert_eq!(frontmatter.status, MemoryStatus::Active);
        assert_eq!(frontmatter.supersedes, None);
        assert_eq!(
            frontmatter.tags,
            vec!["old".to_string(), "memory".to_string()]
        );
        assert_eq!(body, "Legacy body");
    }

    #[test]
    fn frontmatter_writer_omits_default_new_fields() {
        let frontmatter = TaskMemoryFrontmatter {
            created_at: Some("2026-05-22T00:00:00Z".to_string()),
            task_id: Some("task-1".to_string()),
            role: Some("planner".to_string()),
            ..Default::default()
        };
        let yaml = frontmatter.to_yaml_block();

        assert!(!yaml.contains("kind:"));
        assert!(!yaml.contains("namespace:"));
        assert!(!yaml.contains("pinned:"));
        assert!(!yaml.contains("supersedes:"));
        assert!(!yaml.contains("status:"));
    }

    #[tokio::test]
    async fn supersedes_updates_referenced_memory_status() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("old.md");
        let frontmatter = TaskMemoryFrontmatter {
            title: Some("Old".to_string()),
            kind: MemoryKind::Finding,
            ..Default::default()
        };
        tokio::fs::write(&path, render_memory_file(&frontmatter, "Old body"))
            .await
            .unwrap();

        let updated_path = mark_memory_superseded(temp.path(), "old.md").await.unwrap();

        assert_eq!(updated_path, path);
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let (updated_frontmatter, body) = parse_memory_file(&text).unwrap();
        assert_eq!(updated_frontmatter.kind, MemoryKind::Finding);
        assert_eq!(updated_frontmatter.status, MemoryStatus::Superseded);
        assert_eq!(body, "Old body");
    }

    #[tokio::test]
    async fn task_mem_pin_unpin_round_trip() {
        let (_temp, _gcx, ccx, path) = make_task_with_memory("planner").await;
        let mut tool = ToolTaskMemoryPin::new();

        tool.tool_execute(
            ccx.clone(),
            &"call".to_string(),
            &args(&[("memory_id", json!("master-plan")), ("pinned", json!(true))]),
        )
        .await
        .unwrap();
        assert!(read_frontmatter(&path).await.pinned);

        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[("memory_id", json!(MEMORY_FILE)), ("pinned", json!(false))]),
        )
        .await
        .unwrap();
        assert!(!read_frontmatter(&path).await.pinned);
    }

    #[tokio::test]
    async fn task_mem_archive_moves_file() {
        let (_temp, _gcx, ccx, path) = make_task_with_memory("planner").await;
        let mut tool = ToolTaskMemoryArchive::new();

        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[("memory_id", json!("master-plan"))]),
        )
        .await
        .unwrap();

        let archived_path = path
            .parent()
            .unwrap()
            .join(ARCHIVED_MEMORIES_DIR)
            .join(MEMORY_FILE);
        assert!(!path.exists());
        assert!(archived_path.exists());
        assert_eq!(
            read_frontmatter(&archived_path).await.status,
            MemoryStatus::Archived
        );
    }

    #[tokio::test]
    async fn task_mem_unarchive_moves_file_back() {
        let (_temp, _gcx, ccx, path) = make_task_with_memory("planner").await;
        let mut archive = ToolTaskMemoryArchive::new();
        let mut unarchive = ToolTaskMemoryUnarchive::new();

        archive
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[("memory_id", json!("master-plan"))]),
            )
            .await
            .unwrap();
        unarchive
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("memory_id", json!("master-plan"))]),
            )
            .await
            .unwrap();

        let archived_path = path
            .parent()
            .unwrap()
            .join(ARCHIVED_MEMORIES_DIR)
            .join(MEMORY_FILE);
        assert!(path.exists());
        assert!(!archived_path.exists());
        assert_eq!(read_frontmatter(&path).await.status, MemoryStatus::Active);
    }

    #[tokio::test]
    async fn task_mem_pin_rejects_non_planner_role() {
        let (_temp, _gcx, ccx, _path) = make_task_with_memory("agents").await;
        let mut tool = ToolTaskMemoryPin::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("memory_id", json!("master-plan")), ("pinned", json!(true))]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
    }

    #[tokio::test]
    async fn memory_inbox_cursor_read_write_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let cursor = parse_rfc3339_utc("2026-05-22T00:00:00Z").unwrap();

        assert_eq!(read_memory_inbox_cursor(temp.path()).await.unwrap(), None);
        write_memory_inbox_cursor(temp.path(), cursor)
            .await
            .unwrap();

        assert_eq!(
            read_memory_inbox_cursor(temp.path()).await.unwrap(),
            Some(cursor)
        );
        assert_eq!(
            tokio::fs::read_to_string(temp.path().join(MEMORY_INBOX_CURSOR_FILE))
                .await
                .unwrap(),
            cursor.to_rfc3339()
        );
    }

    #[test]
    fn new_memories_filter_sorts_and_limits() {
        let cursor = parse_rfc3339_utc("2026-05-22T00:00:00Z").unwrap();
        let newer = parse_rfc3339_utc("2026-05-22T02:00:00Z").unwrap();
        let newest = parse_rfc3339_utc("2026-05-22T03:00:00Z").unwrap();
        let older = parse_rfc3339_utc("2026-05-21T23:00:00Z").unwrap();
        let memories = vec![
            inbox_memory(
                "newest.md",
                newest,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                "newest",
            ),
            inbox_memory(
                "newer.md",
                newer,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                "newer",
            ),
            inbox_memory(
                "older.md",
                older,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                "older",
            ),
        ];

        let filtered = new_memories_since(&memories, cursor, 1);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].memory_id(), "newest.md");
    }

    #[test]
    fn stale_candidate_detection_uses_progress_age_and_namespace() {
        let now = parse_rfc3339_utc("2026-05-22T00:00:00Z").unwrap();
        let old = now - Duration::days(8);
        let fresh = now - Duration::days(3);
        let memories = vec![
            inbox_memory(
                "old-progress.md",
                old,
                MemoryKind::Progress,
                MemoryNamespace::Task,
                "old",
            ),
            inbox_memory(
                "global-progress.md",
                old,
                MemoryKind::Progress,
                MemoryNamespace::Global,
                "global",
            ),
            inbox_memory(
                "old-decision.md",
                old,
                MemoryKind::Decision,
                MemoryNamespace::Task,
                "decision",
            ),
            inbox_memory(
                "fresh-progress.md",
                fresh,
                MemoryKind::Progress,
                MemoryNamespace::Task,
                "fresh",
            ),
        ];

        let stale = stale_memory_candidates(&memories, now);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].memory_id(), "old-progress.md");
    }

    #[test]
    fn duplicate_pair_detection_uses_body_token_overlap() {
        let now = parse_rfc3339_utc("2026-05-22T00:00:00Z").unwrap();
        let left_body = "---\ntitle: Left\n---\n\nalpha beta gamma delta epsilon zeta";
        let right_body = "---\ntitle: Different\n---\n\nalpha beta gamma delta epsilon extra";
        let other_body = "kappa lambda mu nu xi omicron";
        let memories = vec![
            inbox_memory(
                "left.md",
                now,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                left_body,
            ),
            inbox_memory(
                "right.md",
                now,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                right_body,
            ),
            inbox_memory(
                "other.md",
                now,
                MemoryKind::Finding,
                MemoryNamespace::Task,
                other_body,
            ),
        ];

        let pairs = duplicate_memory_pairs(&memories);

        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].left_path, PathBuf::from("left.md"));
        assert_eq!(pairs[0].right_path, PathBuf::from("right.md"));
        assert!(pairs[0].overlap_percent > 70);
    }

    #[tokio::test]
    async fn task_mem_inbox_renders_all_sections() {
        let (_temp, _gcx, ccx, path) = make_task_with_memory("planner").await;
        let memories_dir = path.parent().unwrap().to_path_buf();
        let now = Utc::now();
        write_memory_with_body(
            &memories_dir,
            "new.md",
            TaskMemoryFrontmatter {
                created_at: Some((now - Duration::hours(2)).to_rfc3339()),
                title: Some("New Decision".to_string()),
                kind: MemoryKind::Decision,
                namespace: MemoryNamespace::Card("T-1".to_string()),
                card_id: Some("T-1".to_string()),
                ..Default::default()
            },
            "shared alpha beta gamma delta epsilon zeta",
        )
        .await;
        write_memory_with_body(
            &memories_dir,
            "duplicate.md",
            TaskMemoryFrontmatter {
                created_at: Some((now - Duration::hours(3)).to_rfc3339()),
                title: Some("Duplicate Decision".to_string()),
                kind: MemoryKind::Decision,
                namespace: MemoryNamespace::Card("T-1".to_string()),
                card_id: Some("T-1".to_string()),
                ..Default::default()
            },
            "shared alpha beta gamma delta epsilon extra",
        )
        .await;
        write_memory_with_body(
            &memories_dir,
            "stale.md",
            TaskMemoryFrontmatter {
                created_at: Some((now - Duration::days(8)).to_rfc3339()),
                title: Some("Old Progress".to_string()),
                kind: MemoryKind::Progress,
                namespace: MemoryNamespace::Task,
                ..Default::default()
            },
            "old progress",
        )
        .await;
        let mut tool = ToolTaskMemoryInbox::new();

        let (_, contexts) = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("since", json!((now - Duration::days(2)).to_rfc3339()))]),
            )
            .await
            .unwrap();
        let output = match &contexts[0] {
            ContextEnum::ChatMessage(message) => match &message.content {
                ChatContent::SimpleText(text) => text.clone(),
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        };

        assert!(output.contains("# Memory Inbox"));
        assert!(output.contains("## New since"));
        assert!(output.contains("New Decision"));
        assert!(output.contains("## Stale candidates (1)"));
        assert!(output.contains("stale.md"));
        assert!(output.contains("## Possible duplicates (1 pairs)"));
        assert!(output.contains("token overlap"));
        assert!(output.contains("## Actions"));
    }

    #[tokio::test]
    async fn task_mem_triage_done_persists_cursor() {
        let (_temp, _gcx, ccx, path) = make_task_with_memory("planner").await;
        let task_dir = path.parent().unwrap().parent().unwrap().to_path_buf();
        let cursor = parse_rfc3339_utc("2026-05-22T04:05:06Z").unwrap();
        let mut tool = ToolTaskMemoryTriageDone::new();

        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[("cursor", json!(cursor.to_rfc3339()))]),
        )
        .await
        .unwrap();

        assert_eq!(
            read_memory_inbox_cursor(&task_dir).await.unwrap(),
            Some(cursor)
        );
    }

    #[tokio::test]
    async fn task_mem_inbox_rejects_non_planner_role() {
        let (_temp, _gcx, ccx, _path) = make_task_with_memory("agents").await;
        let mut tool = ToolTaskMemoryInbox::new();

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args(&[]))
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
    }

    #[tokio::test]
    async fn memory_without_created_at_uses_mtime_and_respects_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        let path = memories_dir.join("missing-created-at.md");
        tokio::fs::write(
            &path,
            render_memory_file(
                &TaskMemoryFrontmatter {
                    title: Some("Missing Created At".to_string()),
                    ..Default::default()
                },
                "Body",
            ),
        )
        .await
        .unwrap();
        let mtime = Utc.with_ymd_and_hms(2026, 5, 22, 1, 0, 0).single().unwrap();
        filetime::set_file_mtime(
            &path,
            filetime::FileTime::from_unix_time(mtime.timestamp(), 0),
        )
        .unwrap();

        let (memories, warnings) = load_task_memory_inbox_entries(&memories_dir).await.unwrap();

        assert!(warnings.is_empty());
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].created_at, mtime);
        assert!(memories[0].created_at_known);
        assert_eq!(
            new_memories_since(&memories, mtime - Duration::seconds(1), 10).len(),
            1
        );
        assert!(new_memories_since(&memories, mtime, 10).is_empty());
    }

    #[tokio::test]
    async fn malformed_memory_warns_and_inbox_continues() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("valid.md"),
            render_memory_file(
                &TaskMemoryFrontmatter {
                    created_at: Some("2026-05-22T00:00:00Z".to_string()),
                    title: Some("Valid".to_string()),
                    ..Default::default()
                },
                "valid body",
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(
            memories_dir.join("malformed.md"),
            "---\ntags: [unterminated\n---\n\nbad body",
        )
        .await
        .unwrap();

        let (memories, warnings) = load_task_memory_inbox_entries(&memories_dir).await.unwrap();
        let output = render_memory_inbox(
            parse_rfc3339_utc("2026-05-21T00:00:00Z").unwrap(),
            parse_rfc3339_utc("2026-05-22T01:00:00Z").unwrap(),
            &memories,
            &[],
            &[],
            &warnings,
        );

        assert_eq!(memories.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(output.contains("Valid"));
        assert!(output.contains("## Warnings (1 skipped)"));
        assert!(output.contains("malformed.md"));
        assert!(output.contains("Failed to parse memory frontmatter"));
    }

    #[test]
    fn duplicate_detection_caps_large_memory_sets() {
        let now = parse_rfc3339_utc("2026-05-22T00:00:00Z").unwrap();
        let memories = (0..600)
            .map(|idx| {
                inbox_memory(
                    &format!("memory-{idx}.md"),
                    now,
                    MemoryKind::Finding,
                    MemoryNamespace::Task,
                    "shared alpha beta gamma delta epsilon zeta",
                )
            })
            .collect::<Vec<_>>();

        let pairs = duplicate_memory_pairs(&memories);

        assert_eq!(pairs.len(), MAX_DUPLICATE_COMPARISONS);
        assert!(pairs.iter().all(|pair| pair.overlap_percent == 100));
    }

    #[test]
    fn card_id_auto_sets_memory_namespace() {
        assert_eq!(
            resolve_memory_namespace(None, Some("T-9")).unwrap(),
            MemoryNamespace::Card("T-9".to_string())
        );
        assert_eq!(
            resolve_memory_namespace(Some("task"), Some("T-9")).unwrap(),
            MemoryNamespace::Task
        );
        assert_eq!(
            resolve_memory_namespace(None, None).unwrap(),
            MemoryNamespace::Task
        );
    }

    #[tokio::test]
    async fn task_mem_search_filters_by_kind() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("decision.md"),
            "---\ntitle: Decision\ntask_id: task-1\nkind: decision\n---\n\nshared needle",
        )
        .await
        .unwrap();
        tokio::fs::write(
            memories_dir.join("risk.md"),
            "---\ntitle: Risk\ntask_id: task-1\nkind: risk\n---\n\nshared needle",
        )
        .await
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];

        let hits = search_task_memories_and_documents(
            gcx,
            "needle",
            KnowledgeSearchFilters {
                scope: Some("task".to_string()),
                kind: Some("decision".to_string()),
                task_id: Some("task-1".to_string()),
                ..Default::default()
            },
            10,
        )
        .await;

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card.title, "Decision");
    }

    #[tokio::test]
    async fn task_mem_search_finds_tag_filename_and_content() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("filename-match.md"),
            "---\ntitle: Filename\ntask_id: task-1\nkind: finding\ntags: [tag-match]\n---\n\ncontent-match",
        )
        .await
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];

        for query in ["tag-match", "filename-match", "content-match"] {
            let hits = search_task_memories_and_documents(
                gcx.clone(),
                query,
                KnowledgeSearchFilters {
                    scope: Some("task".to_string()),
                    task_id: Some("task-1".to_string()),
                    ..Default::default()
                },
                10,
            )
            .await;
            assert_eq!(hits.len(), 1, "query {query}");
            assert_eq!(hits[0].card.title, "Filename");
        }
    }

    #[tokio::test]
    async fn task_introspection_explicit_task_id_memory_search_reads_unbound_task() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("explicit-memory.md"),
            "---\ntitle: Explicit Memory\ntask_id: task-1\nkind: finding\n---\n\nexplicit needle",
        )
        .await
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let ccx = make_unbound_ccx(gcx).await;

        let output = output_text(
            ToolTaskMemorySearch::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        ("task_id", json!("task-1")),
                        ("query", json!("explicit needle")),
                    ]),
                )
                .await
                .unwrap(),
        );

        assert!(output.contains("Explicit Memory"));
        assert!(output.contains("explicit-memory.md"));
    }

    #[tokio::test]
    async fn handle_task_memory_facets_returns_sorted_deduplicated_namespaces() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();

        for (file, namespace, kind, tags) in &[
            (
                "mem-a.md",
                MemoryNamespace::Task,
                MemoryKind::Decision,
                vec!["planner".to_string(), "search".to_string()],
            ),
            (
                "mem-b.md",
                MemoryNamespace::Global,
                MemoryKind::Spec,
                vec!["search".to_string()],
            ),
            (
                "mem-c.md",
                MemoryNamespace::Task,
                MemoryKind::Decision,
                vec!["planner".to_string()],
            ),
        ] {
            tokio::fs::write(
                memories_dir.join(file),
                render_memory_file(
                    &TaskMemoryFrontmatter {
                        namespace: namespace.clone(),
                        kind: *kind,
                        tags: tags.clone(),
                        ..Default::default()
                    },
                    "body",
                ),
            )
            .await
            .unwrap();
        }

        let result = task_memory_facets_for_api(gcx, "task-1").await.unwrap();
        assert_eq!(result.task_id, "task-1");
        assert_eq!(result.namespaces, vec!["global", "task"]);
        assert_eq!(result.kinds, vec!["decision", "spec"]);
        assert_eq!(result.tags, vec!["planner", "search"]);
        assert_eq!(result.total_count, 3);
    }

    #[tokio::test]
    async fn handle_task_memory_facets_counts_pinned_correctly() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();

        for (file, pinned) in &[
            ("pinned.md", true),
            ("active.md", false),
            ("also-pinned.md", true),
        ] {
            tokio::fs::write(
                memories_dir.join(file),
                render_memory_file(
                    &TaskMemoryFrontmatter {
                        pinned: *pinned,
                        ..Default::default()
                    },
                    "body",
                ),
            )
            .await
            .unwrap();
        }

        let result = task_memory_facets_for_api(gcx, "task-1").await.unwrap();
        assert_eq!(result.total_count, 3);
        assert_eq!(result.pinned_count, 2);
    }

    #[tokio::test]
    async fn handle_task_memory_facets_does_not_load_memory_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks/task-1");
        let memories_dir = task_dir.join(MEMORIES_DIR);
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();

        tokio::fs::write(
            memories_dir.join("mem.md"),
            render_memory_file(
                &TaskMemoryFrontmatter {
                    kind: MemoryKind::Risk,
                    namespace: MemoryNamespace::Task,
                    tags: vec!["critical".to_string()],
                    ..Default::default()
                },
                "very large body content that should not appear in facets response",
            ),
        )
        .await
        .unwrap();

        let result = task_memory_facets_for_api(gcx, "task-1").await.unwrap();
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(
            !serialized.contains("very large body content"),
            "facets response must not include memory body content"
        );
        assert_eq!(result.kinds, vec!["risk"]);
        assert_eq!(result.namespaces, vec!["task"]);
        assert_eq!(result.tags, vec!["critical"]);
    }
}
