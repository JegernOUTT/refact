use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SHORT_DESC_MAX_LEN: usize = 80;
pub const DEFAULT_EXEC_OUTPUT_LIMIT_BYTES: usize = 512 * 1024;
pub const EXEC_ENV_DEFAULTS: &[(&str, &str)] = &[
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
    ("REFACT_EXEC", "1"),
];

pub fn normalize_workspace_path(path: &Path) -> PathBuf {
    let normalized = dunce::canonicalize(path).unwrap_or_else(|_| lexical_normalize_path(path));
    normalize_windows_drive(dunce::simplified(&normalized).to_path_buf())
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let last = normalized.components().next_back().map(|component| {
                    (
                        matches!(component, Component::Normal(_)),
                        matches!(component, Component::ParentDir),
                    )
                });
                match last {
                    Some((true, _)) => {
                        normalized.pop();
                    }
                    Some((_, true)) | None => normalized.push(".."),
                    Some((false, false)) => {}
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

#[cfg(windows)]
fn normalize_windows_drive(path: PathBuf) -> PathBuf {
    let mut value = path.to_string_lossy().into_owned();
    let bytes = value.as_bytes();
    let drive_index = if bytes.get(1) == Some(&b':') {
        Some(0)
    } else if value.starts_with(r"\\?\") && bytes.get(5) == Some(&b':') {
        Some(4)
    } else {
        None
    };
    if let Some(index) = drive_index {
        let drive = value.as_bytes()[index];
        if value.starts_with(r"\\?\") {
            value.replace_range(0..4, "");
        }
        let index = if index == 4 { 0 } else { index };
        if drive.is_ascii_uppercase() {
            value.replace_range(
                index..index + 1,
                &(drive as char).to_ascii_lowercase().to_string(),
            );
        }
    }
    PathBuf::from(value)
}

#[cfg(not(windows))]
fn normalize_windows_drive(path: PathBuf) -> PathBuf {
    path
}

fn normalize_workspace_option(workspace: &Option<PathBuf>) -> Option<PathBuf> {
    workspace.as_deref().map(normalize_workspace_path)
}

pub fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecProcessId(pub String);

impl ExecProcessId {
    pub fn new() -> Self {
        Self(format!("exec_{}", Uuid::new_v4()))
    }

    pub fn for_service(service_name: &str, owner: &ExecOwnerMeta) -> Self {
        let slug = service_slug(service_name);
        let scope_hash = service_scope_hash(owner);
        Self(format!("exec_service_{slug}_{scope_hash:016x}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExecProcessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn service_slug(service_name: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for c in service_name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            slug.push('_');
            previous_separator = true;
        }
    }
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        "service".to_string()
    } else {
        slug.to_string()
    }
}

fn service_scope_hash(owner: &ExecOwnerMeta) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    owner.chat_id.hash(&mut hasher);
    normalize_workspace_option(&owner.workspace).hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecMode {
    Foreground,
    Background,
    Service,
    Interactive,
}

impl std::fmt::Display for ExecMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecMode::Foreground => write!(f, "foreground"),
            ExecMode::Background => write!(f, "background"),
            ExecMode::Service => write!(f, "service"),
            ExecMode::Interactive => write!(f, "interactive"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecStatusKind {
    Starting,
    Running,
    Exited,
    Failed,
    Killed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecStatus {
    Starting,
    Running,
    Exited { exit_code: Option<i32> },
    Failed { message: String },
    Killed,
    TimedOut,
}

impl ExecStatus {
    pub fn kind(&self) -> ExecStatusKind {
        match self {
            ExecStatus::Starting => ExecStatusKind::Starting,
            ExecStatus::Running => ExecStatusKind::Running,
            ExecStatus::Exited { .. } => ExecStatusKind::Exited,
            ExecStatus::Failed { .. } => ExecStatusKind::Failed,
            ExecStatus::Killed => ExecStatusKind::Killed,
            ExecStatus::TimedOut => ExecStatusKind::TimedOut,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ExecStatus::Exited { .. }
                | ExecStatus::Failed { .. }
                | ExecStatus::Killed
                | ExecStatus::TimedOut
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecOutputStream {
    Stdout,
    Stderr,
    Combined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutputChunk {
    pub process_id: ExecProcessId,
    pub seq: u64,
    pub stream: ExecOutputStream,
    pub text: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutputLimits {
    pub transcript_max_bytes: usize,
}

impl Default for ExecOutputLimits {
    fn default() -> Self {
        Self {
            transcript_max_bytes: DEFAULT_EXEC_OUTPUT_LIMIT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecReadinessProbe {
    pub wait_keyword: Option<String>,
    pub wait_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct ExecSpawnRequest {
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub mode: ExecMode,
    pub tty: bool,
    pub rows: u16,
    pub cols: u16,
    pub timeout: Option<Duration>,
    pub startup_wait: Option<Duration>,
    pub readiness: Option<ExecReadinessProbe>,
    pub owner: ExecOwnerMeta,
    pub output_limits: ExecOutputLimits,
    pub short_description: Option<String>,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub output_progress_tx: Option<tokio::sync::mpsc::UnboundedSender<ExecOutputChunk>>,
}

impl ExecSpawnRequest {
    pub fn new(mode: ExecMode, command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            env: HashMap::new(),
            mode,
            tty: false,
            rows: 24,
            cols: 80,
            timeout: None,
            startup_wait: None,
            readiness: None,
            owner: ExecOwnerMeta::default(),
            output_limits: ExecOutputLimits::default(),
            short_description: None,
            abort_flag: None,
            output_progress_tx: None,
        }
    }

    pub fn foreground(command: impl Into<String>) -> Self {
        Self::new(ExecMode::Foreground, command)
    }

    pub fn background(command: impl Into<String>) -> Self {
        Self::new(ExecMode::Background, command)
    }

    pub fn service(command: impl Into<String>) -> Self {
        Self::new(ExecMode::Service, command)
    }

    pub fn interactive(command: impl Into<String>) -> Self {
        Self::new(ExecMode::Interactive, command)
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn with_env_map(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_tty(mut self, tty: bool) -> Self {
        self.tty = tty;
        self
    }

    pub fn with_pty_size(mut self, rows: u16, cols: u16) -> Self {
        self.rows = rows;
        self.cols = cols;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_startup_wait(mut self, startup_wait: Duration) -> Self {
        self.startup_wait = Some(startup_wait);
        self
    }

    pub fn with_readiness(mut self, readiness: ExecReadinessProbe) -> Self {
        self.readiness = Some(readiness);
        self
    }

    pub fn with_owner(mut self, owner: ExecOwnerMeta) -> Self {
        self.owner = owner.with_normalized_workspace();
        self
    }

    pub fn with_output_limits(mut self, output_limits: ExecOutputLimits) -> Self {
        self.output_limits = output_limits;
        self
    }

    pub fn with_transcript_limit(mut self, transcript_max_bytes: usize) -> Self {
        self.output_limits.transcript_max_bytes = transcript_max_bytes;
        self
    }

    pub fn with_short_description(mut self, short_description: impl Into<String>) -> Self {
        self.short_description = Some(short_description.into());
        self
    }

    pub fn with_abort_flag(mut self, abort_flag: Arc<AtomicBool>) -> Self {
        self.abort_flag = Some(abort_flag);
        self
    }

    pub fn with_output_progress_tx(
        mut self,
        output_progress_tx: tokio::sync::mpsc::UnboundedSender<ExecOutputChunk>,
    ) -> Self {
        self.output_progress_tx = Some(output_progress_tx);
        self
    }
}

pub fn sanitize_short_description(s: &str) -> String {
    let first_line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let cleaned: String = first_line
        .chars()
        .filter_map(|c| {
            if c == '\t' {
                Some(' ')
            } else if c.is_control() || c == '\x7f' {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= SHORT_DESC_MAX_LEN {
        chars.into_iter().collect()
    } else {
        let mut result: String = chars[..SHORT_DESC_MAX_LEN - 1].iter().collect();
        result.push('\u{2026}');
        result
    }
}

pub fn generate_short_description(command: &str, mode: &ExecMode) -> String {
    if command.trim().is_empty() {
        sanitize_short_description(&mode.to_string())
    } else {
        sanitize_short_description(command)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOwnerMeta {
    pub chat_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub service_name: Option<String>,
    pub workspace: Option<PathBuf>,
}

impl ExecOwnerMeta {
    pub(crate) fn with_normalized_workspace(mut self) -> Self {
        self.workspace = normalize_workspace_option(&self.workspace);
        self
    }

    fn normalized_workspace(&self) -> Option<PathBuf> {
        normalize_workspace_option(&self.workspace)
    }

    pub fn matches_filter(&self, filter: &ExecProcessFilter) -> bool {
        if let Some(chat_id) = filter.chat_id.as_ref() {
            if self.chat_id.as_ref() != Some(chat_id) {
                return false;
            }
        }
        if let Some(tool_call_id) = filter.tool_call_id.as_ref() {
            if self.tool_call_id.as_ref() != Some(tool_call_id) {
                return false;
            }
        }
        if let Some(service_name) = filter.service_name.as_ref() {
            if self.service_name.as_ref() != Some(service_name) {
                return false;
            }
        }
        if let Some(workspace) = filter.workspace.as_ref() {
            if self.normalized_workspace() != Some(normalize_workspace_path(workspace)) {
                return false;
            }
        }
        true
    }

    pub fn matches_service_lookup(&self, lookup: &ExecServiceLookup) -> bool {
        if self.service_name.as_ref() != Some(&lookup.service_name) {
            return false;
        }
        if let Some(chat_id) = lookup.chat_id.as_ref() {
            if self.chat_id.as_ref() != Some(chat_id) {
                return false;
            }
        }
        if let Some(workspace) = lookup.workspace.as_ref() {
            if self.normalized_workspace() != Some(normalize_workspace_path(workspace)) {
                return false;
            }
        }
        true
    }

    pub fn permits_access(&self, current_chat_id: &str, workspace: Option<&Path>) -> bool {
        if !current_chat_id.is_empty() && self.chat_id.as_deref() == Some(current_chat_id) {
            return true;
        }
        if let Some(workspace) = workspace {
            if self.normalized_workspace() == Some(normalize_workspace_path(workspace)) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecProcessMeta {
    pub process_id: ExecProcessId,
    pub owner: ExecOwnerMeta,
    pub mode: ExecMode,
    pub cwd: Option<PathBuf>,
    pub command: String,
    pub short_description: String,
    #[serde(default)]
    pub tty: bool,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
}

impl ExecProcessMeta {
    pub fn new(mode: ExecMode, command: String) -> Self {
        let short_description = generate_short_description(&command, &mode);
        Self {
            process_id: ExecProcessId::new(),
            owner: ExecOwnerMeta::default(),
            mode,
            cwd: None,
            command,
            short_description,
            tty: false,
            created_at_ms: current_timestamp_ms(),
            started_at_ms: None,
            ended_at_ms: None,
        }
    }

    pub fn with_process_id(mut self, process_id: ExecProcessId) -> Self {
        self.process_id = process_id;
        self
    }

    pub fn with_owner(mut self, owner: ExecOwnerMeta) -> Self {
        self.owner = owner.with_normalized_workspace();
        self
    }

    pub fn with_chat_id(mut self, chat_id: impl Into<String>) -> Self {
        self.owner.chat_id = Some(chat_id.into());
        self
    }

    pub fn with_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.owner.tool_call_id = Some(tool_call_id.into());
        self
    }

    pub fn with_service_name(mut self, service_name: impl Into<String>) -> Self {
        self.owner.service_name = Some(service_name.into());
        self
    }

    pub fn with_workspace(mut self, workspace: PathBuf) -> Self {
        self.owner.workspace = Some(normalize_workspace_path(&workspace));
        self
    }

    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn with_tty(mut self, tty: bool) -> Self {
        self.tty = tty;
        self
    }

    pub fn with_short_description(mut self, desc: String) -> Self {
        self.short_description = sanitize_short_description(&desc);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecProcessFilter {
    pub chat_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub service_name: Option<String>,
    pub workspace: Option<PathBuf>,
    pub mode: Option<ExecMode>,
    pub status: Option<ExecStatusKind>,
}

impl ExecProcessFilter {
    pub fn is_empty(&self) -> bool {
        self.chat_id.is_none()
            && self.tool_call_id.is_none()
            && self.service_name.is_none()
            && self.workspace.is_none()
            && self.mode.is_none()
            && self.status.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecServiceLookup {
    pub service_name: String,
    pub chat_id: Option<String>,
    pub workspace: Option<PathBuf>,
}

impl ExecServiceLookup {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            chat_id: None,
            workspace: None,
        }
    }

    pub fn with_chat_id(mut self, chat_id: impl Into<String>) -> Self {
        self.chat_id = Some(chat_id.into());
        self
    }

    pub fn with_workspace(mut self, workspace: PathBuf) -> Self {
        self.workspace = Some(normalize_workspace_path(&workspace));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecProcessSnapshot {
    pub meta: ExecProcessMeta,
    pub status: ExecStatus,
    pub disk_log_path: Option<PathBuf>,
}

impl ExecProcessSnapshot {
    pub fn new(meta: ExecProcessMeta) -> Self {
        Self {
            status: ExecStatus::Starting,
            meta,
            disk_log_path: None,
        }
    }

    pub fn with_status(mut self, status: ExecStatus) -> Self {
        self.status = status;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecReadResult {
    pub process_id: ExecProcessId,
    pub found: bool,
    pub since_seq: u64,
    pub next_seq: u64,
    pub latest_seq: u64,
    pub chunks: Vec<ExecOutputChunk>,
    pub total_bytes_appended: usize,
    pub total_lines_appended: u64,
    pub dropped_chunks: u64,
    pub dropped_bytes: usize,
    pub truncated_chunks: u64,
    pub current_bytes: usize,
    pub max_bytes: usize,
    pub chunk_count: usize,
    pub is_truncated: bool,
    pub disk_log_path: Option<PathBuf>,
}

impl ExecReadResult {
    pub fn not_found(process_id: ExecProcessId, since_seq: u64) -> Self {
        Self {
            process_id,
            found: false,
            since_seq,
            next_seq: since_seq,
            latest_seq: since_seq,
            chunks: Vec::new(),
            total_bytes_appended: 0,
            total_lines_appended: 0,
            dropped_chunks: 0,
            dropped_bytes: 0,
            truncated_chunks: 0,
            current_bytes: 0,
            max_bytes: 0,
            chunk_count: 0,
            is_truncated: false,
            disk_log_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecWriteStdinResult {
    pub process_id: ExecProcessId,
    pub bytes_written: usize,
    pub chunks_returned: usize,
    pub read: ExecReadResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_id_prefix() {
        let id = ExecProcessId::new();
        assert!(
            id.as_str().starts_with("exec_"),
            "ID should start with 'exec_': {id}"
        );
    }

    #[test]
    fn test_service_process_id_is_scope_aware() {
        let workspace_a = ExecOwnerMeta {
            chat_id: Some("chat-a".to_string()),
            workspace: Some(PathBuf::from("/workspace-a")),
            ..ExecOwnerMeta::default()
        };
        let workspace_b = ExecOwnerMeta {
            chat_id: Some("chat-a".to_string()),
            workspace: Some(PathBuf::from("/workspace-b")),
            ..ExecOwnerMeta::default()
        };
        let id_a = ExecProcessId::for_service("API server!", &workspace_a);
        let id_b = ExecProcessId::for_service("API server!", &workspace_b);

        assert!(id_a.as_str().starts_with("exec_service_api_server_"));
        assert_ne!(id_a, id_b);
        assert_eq!(
            id_a,
            ExecProcessId::for_service("API server!", &workspace_a)
        );
        assert!(ExecProcessId::for_service("  ", &workspace_a)
            .as_str()
            .starts_with("exec_service_service_"));
    }

    #[test]
    fn test_service_process_id_normalizes_dot_workspace() {
        let owner = ExecOwnerMeta {
            workspace: Some(PathBuf::from("/tmp/ws")),
            ..ExecOwnerMeta::default()
        };
        let owner_with_dot = ExecOwnerMeta {
            workspace: Some(PathBuf::from("/tmp/ws/.")),
            ..ExecOwnerMeta::default()
        };

        assert_eq!(
            ExecProcessId::for_service("api", &owner),
            ExecProcessId::for_service("api", &owner_with_dot)
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_service_process_id_resolves_symlink_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let link = temp.path().join("link");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let real_owner = ExecOwnerMeta {
            workspace: Some(real),
            ..ExecOwnerMeta::default()
        };
        let link_owner = ExecOwnerMeta {
            workspace: Some(link),
            ..ExecOwnerMeta::default()
        };

        assert_eq!(
            ExecProcessId::for_service("api", &real_owner),
            ExecProcessId::for_service("api", &link_owner)
        );
    }

    #[test]
    fn test_service_process_id_keeps_distinct_absolute_workspaces() {
        let first = ExecOwnerMeta {
            workspace: Some(PathBuf::from("/tmp/ws-a")),
            ..ExecOwnerMeta::default()
        };
        let second = ExecOwnerMeta {
            workspace: Some(PathBuf::from("/tmp/ws-b")),
            ..ExecOwnerMeta::default()
        };

        assert_ne!(
            ExecProcessId::for_service("api", &first),
            ExecProcessId::for_service("api", &second)
        );
    }

    #[test]
    fn test_process_id_display() {
        let id = ExecProcessId("exec_abc123".to_string());
        assert_eq!(id.to_string(), "exec_abc123");
    }

    #[test]
    fn test_sanitize_short_description_basic() {
        assert_eq!(sanitize_short_description("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_short_description_control_chars() {
        let input = "hello\x01\x02world\x7f";
        assert_eq!(sanitize_short_description(input), "helloworld");
    }

    #[test]
    fn test_sanitize_short_description_tabs_become_spaces() {
        let input = "hello\tworld";
        assert_eq!(sanitize_short_description(input), "hello world");
    }

    #[test]
    fn test_sanitize_short_description_capping() {
        let long = "a".repeat(100);
        let result = sanitize_short_description(&long);
        let chars: Vec<char> = result.chars().collect();
        assert_eq!(chars.len(), SHORT_DESC_MAX_LEN);
        assert_eq!(chars[SHORT_DESC_MAX_LEN - 1], '\u{2026}');
    }

    #[test]
    fn test_sanitize_short_description_exactly_at_limit() {
        let s = "a".repeat(SHORT_DESC_MAX_LEN);
        let result = sanitize_short_description(&s);
        assert_eq!(result, s);
    }

    #[test]
    fn test_sanitize_short_description_multiline_takes_first_nonempty() {
        let input = "\n  \nfirst line\nsecond line";
        assert_eq!(sanitize_short_description(input), "first line");
    }

    #[test]
    fn test_sanitize_short_description_trims_whitespace() {
        assert_eq!(sanitize_short_description("  hello  "), "hello");
    }

    #[test]
    fn test_sanitize_short_description_empty_string() {
        assert_eq!(sanitize_short_description(""), "");
    }

    #[test]
    fn test_generate_short_description_from_command() {
        let result = generate_short_description("cargo build --release", &ExecMode::Foreground);
        assert_eq!(result, "cargo build --release");
    }

    #[test]
    fn test_generate_short_description_fallback_to_mode_when_empty() {
        let result = generate_short_description("", &ExecMode::Background);
        assert_eq!(result, "background");
    }

    #[test]
    fn test_generate_short_description_fallback_whitespace_only() {
        let result = generate_short_description("   ", &ExecMode::Service);
        assert_eq!(result, "service");
    }

    #[test]
    fn test_process_meta_uses_command_for_short_desc() {
        let meta = ExecProcessMeta::new(ExecMode::Foreground, "echo hello".to_string());
        assert_eq!(meta.short_description, "echo hello");
        assert_eq!(meta.command, "echo hello");
        assert_eq!(meta.owner, ExecOwnerMeta::default());
        assert!(meta.started_at_ms.is_none());
        assert!(meta.ended_at_ms.is_none());
    }

    #[test]
    fn test_process_meta_custom_short_description() {
        let meta = ExecProcessMeta::new(ExecMode::Service, "nginx".to_string())
            .with_short_description("Web server (nginx)".to_string());
        assert_eq!(meta.short_description, "Web server (nginx)");
        assert_eq!(meta.command, "nginx");
    }

    #[test]
    fn test_process_meta_owner_builders() {
        let meta = ExecProcessMeta::new(ExecMode::Service, "server".to_string())
            .with_chat_id("chat-a")
            .with_tool_call_id("tool-a")
            .with_service_name("api")
            .with_workspace(PathBuf::from("/workspace"))
            .with_cwd(PathBuf::from("/workspace/app"));
        assert_eq!(meta.owner.chat_id.as_deref(), Some("chat-a"));
        assert_eq!(meta.owner.tool_call_id.as_deref(), Some("tool-a"));
        assert_eq!(meta.owner.service_name.as_deref(), Some("api"));
        assert_eq!(meta.owner.workspace, Some(PathBuf::from("/workspace")));
        assert_eq!(meta.cwd, Some(PathBuf::from("/workspace/app")));
    }

    #[test]
    fn test_status_kind_and_terminal() {
        assert_eq!(ExecStatus::Starting.kind(), ExecStatusKind::Starting);
        assert_eq!(ExecStatus::Running.kind(), ExecStatusKind::Running);
        assert_eq!(
            ExecStatus::Exited { exit_code: Some(0) }.kind(),
            ExecStatusKind::Exited
        );
        assert_eq!(
            ExecStatus::Failed {
                message: "nope".to_string()
            }
            .kind(),
            ExecStatusKind::Failed
        );
        assert_eq!(ExecStatus::Killed.kind(), ExecStatusKind::Killed);
        assert_eq!(ExecStatus::TimedOut.kind(), ExecStatusKind::TimedOut);
        assert!(!ExecStatus::Starting.is_terminal());
        assert!(!ExecStatus::Running.is_terminal());
        assert!(ExecStatus::Exited { exit_code: Some(0) }.is_terminal());
        assert!(ExecStatus::Failed {
            message: "nope".to_string()
        }
        .is_terminal());
        assert!(ExecStatus::Killed.is_terminal());
        assert!(ExecStatus::TimedOut.is_terminal());
    }

    #[test]
    fn test_owner_matches_filter() {
        let owner = ExecOwnerMeta {
            chat_id: Some("chat-a".to_string()),
            tool_call_id: Some("tool-a".to_string()),
            service_name: Some("svc".to_string()),
            workspace: Some(PathBuf::from("/workspace")),
        };
        let filter = ExecProcessFilter {
            chat_id: Some("chat-a".to_string()),
            tool_call_id: None,
            service_name: Some("svc".to_string()),
            workspace: Some(PathBuf::from("/workspace")),
            mode: None,
            status: None,
        };
        assert!(owner.matches_filter(&filter));

        let wrong_filter = ExecProcessFilter {
            chat_id: Some("chat-b".to_string()),
            ..ExecProcessFilter::default()
        };
        assert!(!owner.matches_filter(&wrong_filter));
    }

    #[test]
    fn test_service_lookup_is_scoped() {
        let owner = ExecOwnerMeta {
            chat_id: Some("chat-a".to_string()),
            tool_call_id: None,
            service_name: Some("svc".to_string()),
            workspace: Some(PathBuf::from("/workspace-a")),
        };
        assert!(owner.matches_service_lookup(
            &ExecServiceLookup::new("svc")
                .with_chat_id("chat-a")
                .with_workspace(PathBuf::from("/workspace-a"))
        ));
        assert!(owner.matches_service_lookup(
            &ExecServiceLookup::new("svc")
                .with_chat_id("chat-a")
                .with_workspace(PathBuf::from("/workspace-a/."))
        ));
        assert!(!owner.matches_service_lookup(
            &ExecServiceLookup::new("svc")
                .with_chat_id("chat-b")
                .with_workspace(PathBuf::from("/workspace-a"))
        ));
    }

    #[test]
    fn test_snapshot_starts_in_starting_status() {
        let meta = ExecProcessMeta::new(ExecMode::Foreground, "ls".to_string());
        let snap = ExecProcessSnapshot::new(meta);
        assert_eq!(snap.status, ExecStatus::Starting);
    }

    #[test]
    fn test_snapshot_with_status() {
        let meta = ExecProcessMeta::new(ExecMode::Foreground, "ls".to_string());
        let snap =
            ExecProcessSnapshot::new(meta).with_status(ExecStatus::Exited { exit_code: Some(0) });
        assert_eq!(snap.status, ExecStatus::Exited { exit_code: Some(0) });
    }

    #[test]
    fn test_snapshot_serialization_round_trip() {
        let meta = ExecProcessMeta {
            process_id: ExecProcessId("exec_test123".to_string()),
            owner: ExecOwnerMeta {
                chat_id: Some("chat-a".to_string()),
                tool_call_id: Some("tool-a".to_string()),
                service_name: Some("worker".to_string()),
                workspace: Some(PathBuf::from("/workspace")),
            },
            mode: ExecMode::Background,
            cwd: Some(PathBuf::from("/workspace/app")),
            command: "sleep 10".to_string(),
            short_description: "sleep 10".to_string(),
            tty: false,
            created_at_ms: 1_000_000,
            started_at_ms: Some(1_000_010),
            ended_at_ms: None,
        };
        let snap = ExecProcessSnapshot::new(meta).with_status(ExecStatus::Running);

        let json = serde_json::to_string(&snap).expect("serialization failed");
        let deserialized: ExecProcessSnapshot =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(snap, deserialized);
    }

    #[test]
    fn test_exec_status_failed_serialization() {
        let status = ExecStatus::Failed {
            message: "oom killed".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: ExecStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, back);
    }

    #[test]
    fn test_not_found_read_result() {
        let result = ExecReadResult::not_found(ExecProcessId("exec_missing".to_string()), 42);
        assert_eq!(result.process_id, ExecProcessId("exec_missing".to_string()));
        assert!(!result.found);
        assert_eq!(result.since_seq, 42);
        assert_eq!(result.next_seq, 42);
        assert_eq!(result.latest_seq, 42);
        assert!(result.chunks.is_empty());
    }
}
