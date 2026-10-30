use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Query, State};
use axum::Json;
use git2::Repository;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_blocklist::{is_blocklisted, IndexingEverywhere};
use crate::files_correction::{
    check_if_its_inside_a_workspace_worktree_or_config, get_unscoped_project_dirs,
    registered_worktree_path_mappings, WorktreePathMappings,
};
use crate::files_in_workspace::{check_file_privacy_for_send, strictest_zone_for_path};
use crate::global_context::GlobalContext;

pub const PRIVACY_BLOCKED_PREFIX: &str = "Blocked by privacy rules:";

const DEFAULT_MAX_ENTRIES: usize = 2_000;
const MAX_ENTRIES: usize = 5_000;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const BINARY_PROBE_BYTES: usize = 8 * 1024;

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_entries: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TreeEntry {
    name: String,
    path: String,
    kind: &'static str,
    size: Option<u64>,
    #[serde(default)]
    ignored: bool,
    privacy_zone: ResolvedPrivacyZone,
}

#[derive(Debug, Serialize)]
pub struct ResolvedPrivacyZone {
    name: String,
    send_to: Vec<String>,
    on_shell_read: refact_privacy::ShellBehavior,
}

impl From<refact_privacy::Zone> for ResolvedPrivacyZone {
    fn from(zone: refact_privacy::Zone) -> Self {
        Self {
            name: zone.name,
            send_to: zone.send_to,
            on_shell_read: zone.on_shell_read,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TreeResponse {
    path: String,
    entries: Vec<TreeEntry>,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    path: String,
    #[serde(default)]
    line_start: Option<usize>,
    #[serde(default)]
    line_end: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct OpenRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    path: String,
    content: String,
    #[serde(default)]
    expected_mtime_ms: Option<u64>,
    #[serde(default)]
    expected_digest: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteResponse {
    path: String,
    size: u64,
    mtime_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct ReadResponse {
    path: String,
    content: String,
    language: Option<String>,
    size: u64,
    truncated: bool,
    line_start: Option<usize>,
    line_end: Option<usize>,
    mtime_ms: u64,
    digest: String,
    #[serde(skip_serializing_if = "is_false")]
    binary: bool,
    can_edit: bool,
    image_mime: Option<String>,
    image_data: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn io_error(status: StatusCode, action: &str, path: &Path, error: std::io::Error) -> ScratchError {
    ScratchError::new(
        status,
        format!("Failed to {action} '{}': {error}", path.display()),
    )
}

async fn validated_existing_path(
    gcx: Arc<GlobalContext>,
    requested: &Path,
) -> Result<PathBuf, ScratchError> {
    if !requested.is_absolute() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Path must be absolute".to_string(),
        ));
    }
    check_if_its_inside_a_workspace_worktree_or_config(gcx.clone(), requested)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    let canonical = tokio::fs::canonicalize(requested)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|error| io_error(StatusCode::NOT_FOUND, "resolve path", requested, error))?;
    check_if_its_inside_a_workspace_worktree_or_config(gcx, &canonical)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    Ok(canonical)
}

async fn privacy_checked_path(gcx: Arc<GlobalContext>, path: &PathBuf) -> Result<(), ScratchError> {
    check_file_privacy_for_send(gcx, path)
        .await
        .map_err(|error| {
            ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("{PRIVACY_BLOCKED_PREFIX} {error}"),
            )
        })
}

fn clamped_tree_limit(max_entries: Option<usize>) -> usize {
    max_entries.unwrap_or(DEFAULT_MAX_ENTRIES).min(MAX_ENTRIES)
}

fn hidden_or_heavy_directory(name: &str, is_dir: bool) -> bool {
    is_dir && (name.starts_with('.') || matches!(name, "node_modules" | "target"))
}

fn blocklisted_entry(
    indexing: &IndexingEverywhere,
    directory: &Path,
    path: &Path,
    is_dir: bool,
) -> bool {
    let settings = indexing.indexing_for_path(path);
    let relative = path.strip_prefix(directory).unwrap_or(path);
    if is_blocklisted(&settings, relative) {
        return true;
    }
    if is_dir {
        let relative_child = relative.join("__refact_files_entry__");
        return is_blocklisted(&settings, &relative_child);
    }
    false
}

fn sort_entries(entries: &mut [TreeEntry]) {
    entries.sort_by(|left, right| {
        let left_dir = left.kind == "dir";
        let right_dir = right.kind == "dir";
        right_dir.cmp(&left_dir).then_with(|| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.name.cmp(&right.name))
        })
    });
}

struct TreePrivacyResolver {
    policy: refact_privacy::CompiledPolicy,
    workspace_roots: Vec<PathBuf>,
    worktree_mappings: WorktreePathMappings,
}

impl TreePrivacyResolver {
    async fn new(gcx: Arc<GlobalContext>) -> Result<Self, ScratchError> {
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
        let compiled = policy.compile().map_err(|error| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to compile privacy policy: {error}"),
            )
        })?;
        Ok(Self {
            policy: compiled,
            workspace_roots: get_unscoped_project_dirs(gcx.clone()).await,
            worktree_mappings: registered_worktree_path_mappings(gcx.cache_dir.as_path()),
        })
    }

    fn zone_for_path(&self, path: &Path) -> ResolvedPrivacyZone {
        strictest_zone_for_path(
            &self.policy,
            path,
            &self.workspace_roots,
            &self.worktree_mappings,
        )
        .into()
    }
}

async fn workspace_root_entries(
    roots: &[PathBuf],
    privacy: &TreePrivacyResolver,
) -> Result<Vec<TreeEntry>, ScratchError> {
    let mut entries = Vec::new();
    for root in roots {
        let canonical = tokio::fs::canonicalize(root)
            .await
            .map(|path| dunce::simplified(&path).to_path_buf())
            .map_err(|error| {
                io_error(
                    StatusCode::NOT_FOUND,
                    "resolve workspace root",
                    &root,
                    error,
                )
            })?;
        let name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| canonical.to_string_lossy().to_string());
        entries.push(TreeEntry {
            name,
            path: canonical.to_string_lossy().to_string(),
            kind: "dir",
            size: None,
            ignored: false,
            privacy_zone: privacy.zone_for_path(&canonical),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

async fn list_dir_core(
    directory: &Path,
    indexing: &IndexingEverywhere,
    privacy: &TreePrivacyResolver,
) -> Result<Vec<TreeEntry>, ScratchError> {
    let repository = Repository::discover(directory).ok();
    let mut read_dir = tokio::fs::read_dir(directory).await.map_err(|error| {
        io_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "list directory",
            directory,
            error,
        )
    })?;
    let mut entries = Vec::new();
    while let Some(entry) = read_dir.next_entry().await.map_err(|error| {
        io_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "list directory",
            directory,
            error,
        )
    })? {
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().await.map_err(|error| {
            io_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "inspect entry",
                &entry.path(),
                error,
            )
        })?;
        if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
            continue;
        }
        let path = entry.path();
        let is_dir = file_type.is_dir();
        if hidden_or_heavy_directory(&name, is_dir) {
            continue;
        }
        if blocklisted_entry(indexing, directory, &path, is_dir) {
            continue;
        }
        let ignored = repository
            .as_ref()
            .and_then(|repository| repository.workdir().map(|workdir| (repository, workdir)))
            .and_then(|(repository, workdir)| {
                path.strip_prefix(workdir)
                    .ok()
                    .map(|path| (repository, path))
            })
            .and_then(|(repository, path)| repository.status_should_ignore(path).ok())
            .unwrap_or(false);
        let size = if is_dir {
            None
        } else {
            Some(
                entry
                    .metadata()
                    .await
                    .map_err(|error| {
                        io_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "inspect entry",
                            &path,
                            error,
                        )
                    })?
                    .len(),
            )
        };
        entries.push(TreeEntry {
            name,
            path: path.to_string_lossy().to_string(),
            kind: if is_dir { "dir" } else { "file" },
            size,
            ignored,
            privacy_zone: privacy.zone_for_path(&path),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn truncate_entries(mut entries: Vec<TreeEntry>, limit: usize) -> (Vec<TreeEntry>, bool) {
    let truncated = entries.len() > limit;
    entries.truncate(limit);
    (entries, truncated)
}

fn validate_line_range(
    line_start: Option<usize>,
    line_end: Option<usize>,
) -> Result<(), ScratchError> {
    if line_start == Some(0) || line_end == Some(0) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Line numbers are 1-based".to_string(),
        ));
    }
    if matches!((line_start, line_end), (Some(start), Some(end)) if start > end) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "line_start must not exceed line_end".to_string(),
        ));
    }
    Ok(())
}

fn truncate_content(mut content: String) -> (String, bool) {
    if content.len() <= MAX_CONTENT_BYTES {
        return (content, false);
    }
    let mut end = MAX_CONTENT_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    (content, true)
}

fn language_for_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();
    let language = match file_name.as_str() {
        "dockerfile" => "dockerfile",
        "makefile" => "makefile",
        _ => match path.extension()?.to_string_lossy().to_lowercase().as_str() {
            "c" | "h" => "c",
            "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
            "cs" => "csharp",
            "css" => "css",
            "go" => "go",
            "html" | "htm" => "html",
            "java" => "java",
            "js" | "cjs" | "mjs" => "javascript",
            "jsx" => "javascriptreact",
            "json" => "json",
            "kt" | "kts" => "kotlin",
            "lua" => "lua",
            "md" | "mdx" => "markdown",
            "php" => "php",
            "py" => "python",
            "r" => "r",
            "rb" => "ruby",
            "rs" => "rust",
            "scss" => "scss",
            "sh" | "bash" | "zsh" => "shellscript",
            "sql" => "sql",
            "swift" => "swift",
            "toml" => "toml",
            "txt" => "plaintext",
            "ts" => "typescript",
            "tsx" => "typescriptreact",
            "xml" => "xml",
            "yaml" | "yml" => "yaml",
            _ => return None,
        },
    };
    Some(language.to_string())
}

fn mtime_ms(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn browser_safe_image_mime(bytes: &[u8]) -> Option<&'static str> {
    let (format, mime) = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => (image::ImageFormat::Png, "image/png"),
        image::ImageFormat::Jpeg => (image::ImageFormat::Jpeg, "image/jpeg"),
        image::ImageFormat::Gif => (image::ImageFormat::Gif, "image/gif"),
        image::ImageFormat::WebP => (image::ImageFormat::WebP, "image/webp"),
        _ => return None,
    };
    image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .ok()
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|_| mime)
}

fn may_be_preview_image(prefix: &[u8]) -> bool {
    matches!(
        image::guess_format(prefix),
        Ok(image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP)
    )
}

struct LineSelector {
    start: usize,
    end: usize,
    line: usize,
    selected: Vec<u8>,
    overflow: bool,
}

impl LineSelector {
    fn new(line_start: Option<usize>, line_end: Option<usize>) -> Self {
        Self {
            start: line_start.unwrap_or(1),
            end: line_end.unwrap_or(usize::MAX),
            line: 1,
            selected: Vec::new(),
            overflow: false,
        }
    }

    fn push(&mut self, mut data: &[u8]) {
        let cap = MAX_CONTENT_BYTES + 4;
        while !data.is_empty() && self.line <= self.end {
            let (segment, rest, newline) = match data.iter().position(|byte| *byte == b'\n') {
                Some(index) => (&data[..=index], &data[index + 1..], true),
                None => (data, &data[data.len()..], false),
            };
            if self.line >= self.start {
                let keep = segment.len().min(cap - self.selected.len());
                self.selected.extend_from_slice(&segment[..keep]);
                self.overflow |= keep < segment.len();
            }
            if newline {
                self.line += 1;
            }
            data = rest;
        }
    }
}

struct Utf8Stream {
    carry: Vec<u8>,
    valid: bool,
}

impl Utf8Stream {
    fn push(&mut self, data: &[u8]) {
        if !self.valid {
            return;
        }
        self.carry.extend_from_slice(data);
        match std::str::from_utf8(&self.carry) {
            Ok(_) => self.carry.clear(),
            Err(error) if error.error_len().is_none() => {
                self.carry.drain(..error.valid_up_to());
            }
            Err(_) => self.valid = false,
        }
    }

    fn finish(&self) -> bool {
        self.valid && self.carry.is_empty()
    }
}

struct StreamedFile {
    metadata: std::fs::Metadata,
    head: Vec<u8>,
    complete_head: bool,
    selected: Vec<u8>,
    selected_overflow: bool,
    utf8: bool,
    digest: String,
}

fn not_regular_file() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file")
}

fn stream_regular_file(
    path: &Path,
    line_start: Option<usize>,
    line_end: Option<usize>,
) -> std::io::Result<StreamedFile> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(not_regular_file());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(not_regular_file());
    }
    let mut file = file.take(metadata.len());
    let mut hasher = Sha256::new();
    let mut head_cap = BINARY_PROBE_BYTES;
    let mut head = Vec::with_capacity(head_cap);
    let mut total = 0usize;
    let mut lines = LineSelector::new(line_start, line_end);
    let mut utf8 = Utf8Stream {
        carry: Vec::new(),
        valid: true,
    };
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let data = &chunk[..read];
        if total == 0 && may_be_preview_image(data) {
            head_cap = MAX_IMAGE_BYTES;
        }
        hasher.update(data);
        let keep = read.min(head_cap.saturating_sub(head.len()));
        head.extend_from_slice(&data[..keep]);
        utf8.push(data);
        lines.push(data);
        total = total.saturating_add(read);
    }
    let out = hasher.finalize();
    Ok(StreamedFile {
        metadata,
        complete_head: total == head.len(),
        head,
        selected: lines.selected,
        selected_overflow: lines.overflow,
        utf8: utf8.finish(),
        digest: hex::encode(&out[..16]),
    })
}

async fn read_file_core(
    path: &Path,
    line_start: Option<usize>,
    line_end: Option<usize>,
    can_edit: bool,
) -> Result<ReadResponse, ScratchError> {
    use base64::Engine;

    validate_line_range(line_start, line_end)?;
    let owned = path.to_path_buf();
    let streamed =
        tokio::task::spawn_blocking(move || stream_regular_file(&owned, line_start, line_end))
            .await
            .map_err(|error| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read '{}': {error}", path.display()),
                )
            })?
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::InvalidInput => ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Path '{}' is not a file", path.display()),
                ),
                std::io::ErrorKind::NotFound => {
                    io_error(StatusCode::NOT_FOUND, "read file", path, error)
                }
                _ => io_error(StatusCode::INTERNAL_SERVER_ERROR, "read file", path, error),
            })?;
    let StreamedFile {
        metadata,
        head,
        complete_head,
        selected,
        selected_overflow,
        utf8,
        digest,
    } = streamed;
    let language = language_for_path(path);
    let path = path.to_string_lossy().to_string();
    let image_mime = complete_head
        .then(|| browser_safe_image_mime(&head))
        .flatten();
    let binary = image_mime.is_some() || head[..head.len().min(BINARY_PROBE_BYTES)].contains(&0);
    if !binary && utf8 {
        let text = match std::str::from_utf8(&selected) {
            Ok(text) => text,
            Err(error) => std::str::from_utf8(&selected[..error.valid_up_to()]).unwrap_or_default(),
        };
        let (content, truncated) = truncate_content(text.to_string());
        return Ok(ReadResponse {
            path,
            content,
            language,
            size: metadata.len(),
            truncated: truncated || selected_overflow,
            line_start,
            line_end,
            mtime_ms: mtime_ms(&metadata),
            digest,
            binary: false,
            can_edit,
            image_mime: None,
            image_data: None,
        });
    }
    let image_data = image_mime.map(|_| base64::engine::general_purpose::STANDARD.encode(&head));
    Ok(ReadResponse {
        path,
        content: String::new(),
        language,
        size: metadata.len(),
        truncated: false,
        line_start,
        line_end,
        mtime_ms: mtime_ms(&metadata),
        digest,
        binary: true,
        can_edit,
        image_mime: image_mime.map(str::to_string),
        image_data,
    })
}

pub async fn handle_v1_files_tree(
    State(app): State<AppState>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, ScratchError> {
    let gcx = app.gcx.clone();
    let limit = clamped_tree_limit(query.max_entries);
    let raw_path = query.path.unwrap_or_default();
    let privacy = TreePrivacyResolver::new(gcx.clone()).await?;
    if raw_path.is_empty() {
        let entries = workspace_root_entries(&privacy.workspace_roots, &privacy).await?;
        let (entries, truncated) = truncate_entries(entries, limit);
        return Ok(Json(TreeResponse {
            path: String::new(),
            entries,
            truncated,
        }));
    }
    let path = validated_existing_path(gcx.clone(), Path::new(&raw_path)).await?;
    if !path.is_dir() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Path '{}' is not a directory", path.display()),
        ));
    }
    let indexing = crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx).await;
    let entries = list_dir_core(&path, &indexing, &privacy).await?;
    let (entries, truncated) = truncate_entries(entries, limit);
    Ok(Json(TreeResponse {
        path: path.to_string_lossy().to_string(),
        entries,
        truncated,
    }))
}

pub async fn handle_v1_files_read(
    State(app): State<AppState>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<ReadResponse>, ScratchError> {
    validate_line_range(query.line_start, query.line_end)?;
    let requested = PathBuf::from(&query.path);
    let path = validated_existing_path(app.gcx.clone(), &requested).await?;
    privacy_checked_path(app.gcx.clone(), &requested).await?;
    privacy_checked_path(app.gcx, &path).await?;
    Ok(Json(
        read_file_core(&path, query.line_start, query.line_end, true).await?,
    ))
}

fn trusted_open_origin(headers: &axum::http::HeaderMap) -> bool {
    use std::net::IpAddr;
    use url::Host;

    let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
        return true;
    };
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|origin| url::Url::parse(origin).ok())
    else {
        return false;
    };
    if origin.scheme() == "vscode-webview" {
        return origin.host().is_some() && matches!(origin.path(), "" | "/");
    }
    if origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return false;
    }
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    let local_host = match origin.host() {
        Some(Host::Domain(domain)) if domain == "localhost" => return true,
        Some(Host::Domain(domain)) if domain == "refactai" => {
            return origin.scheme() == "http" && origin.port().is_none();
        }
        Some(Host::Ipv4(ip)) if ip.is_loopback() => return true,
        Some(Host::Ipv6(ip)) if ip.is_loopback() => return true,
        Some(Host::Domain(domain)) => domain
            .strip_suffix(".local")
            .is_some_and(|label| !label.is_empty() && !label.ends_with('.')),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => true,
        None => false,
    };
    let Some(port) = origin.port_or_known_default() else {
        return false;
    };
    let Some(request_host) = headers
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
        .and_then(|host| host.parse::<axum::http::uri::Authority>().ok())
    else {
        return false;
    };
    let request_port = request_host
        .port_u16()
        .unwrap_or(port_default(origin.scheme()));
    let request_name = request_host
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let same_host = match origin.host() {
        Some(Host::Domain(domain)) => request_name == domain,
        Some(Host::Ipv4(ip)) => request_name.parse::<IpAddr>().ok() == Some(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => request_name.parse::<IpAddr>().ok() == Some(IpAddr::V6(ip)),
        None => false,
    };
    local_host && same_host && request_port == port
}

fn port_default(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

pub async fn handle_v1_files_open(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<OpenRequest>,
) -> Result<Json<ReadResponse>, ScratchError> {
    if !trusted_open_origin(&headers) {
        return Err(ScratchError::new(
            StatusCode::FORBIDDEN,
            "Opening files is not allowed from this origin".to_string(),
        ));
    }
    let requested = PathBuf::from(&request.path);
    if !requested.is_absolute() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Path must be absolute".to_string(),
        ));
    }
    let canonical = tokio::fs::canonicalize(&requested)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|error| io_error(StatusCode::NOT_FOUND, "resolve path", &requested, error))?;
    let privacy = crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    crate::privacy::check_file_privacy(
        privacy,
        &requested,
        &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
    )
    .map_err(|error| {
        ScratchError::new(
            StatusCode::FORBIDDEN,
            format!("{PRIVACY_BLOCKED_PREFIX} {error}"),
        )
    })?;
    privacy_checked_path(app.gcx.clone(), &requested).await?;
    privacy_checked_path(app.gcx.clone(), &canonical).await?;
    let can_edit = validated_existing_path(app.gcx.clone(), &requested)
        .await
        .is_ok_and(|validated| validated == canonical);
    Ok(Json(
        read_file_core(&canonical, None, None, can_edit).await?,
    ))
}

pub async fn handle_v1_files_write(
    State(app): State<AppState>,
    Json(request): Json<WriteRequest>,
) -> Result<Json<WriteResponse>, ScratchError> {
    if request.content.len() > MAX_CONTENT_BYTES {
        return Err(ScratchError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("File content exceeds the {MAX_CONTENT_BYTES} byte editing limit"),
        ));
    }
    let requested = PathBuf::from(&request.path);
    let path = validated_existing_path(app.gcx.clone(), &requested).await?;
    privacy_checked_path(app.gcx.clone(), &requested).await?;
    privacy_checked_path(app.gcx.clone(), &path).await?;

    let guard = crate::file_activity::begin_write(
        None,
        &path,
        crate::file_activity::WriteIntent::Contextual,
        "files.write",
    )
    .await
    .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| io_error(StatusCode::NOT_FOUND, "inspect file", &path, error))?;
    if !metadata.is_file() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Path '{}' is not a file", path.display()),
        ));
    }
    if let Some(expected_mtime_ms) = request.expected_mtime_ms {
        let current_mtime_ms = mtime_ms(&metadata);
        if current_mtime_ms != expected_mtime_ms {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!(
                    "File '{}' changed on disk since it was loaded",
                    path.display()
                ),
            ));
        }
    }

    let before = tokio::fs::read_to_string(&path)
        .await
        .map_err(|error| io_error(StatusCode::INTERNAL_SERVER_ERROR, "read file", &path, error))?;
    if request
        .expected_digest
        .as_ref()
        .is_some_and(|digest| *digest != crate::file_activity::digest_of(&before))
    {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "File content changed since it was loaded".into(),
        ));
    }
    let buffer =
        crate::files_in_workspace::get_file_text_from_memory_or_disk(app.gcx.clone(), &path)
            .await
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
    if buffer != before {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "Editor buffer differs from disk".into(),
        ));
    }
    tokio::fs::write(&path, &request.content)
        .await
        .map_err(|error| {
            io_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "write file",
                &path,
                error,
            )
        })?;
    crate::tools::file_edit::undo_history::record_edit(
        &path,
        Some(&before),
        Some(&request.content),
    );
    crate::files_in_workspace::remove_memory_document_for_path(app.gcx.clone(), &path).await;
    guard.commit(
        crate::file_activity::WriteKind::Modified,
        Some(&request.content),
    );

    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        io_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "inspect file",
            &path,
            error,
        )
    })?;
    Ok(Json(WriteResponse {
        path: path.to_string_lossy().to_string(),
        size: metadata.len(),
        mtime_ms: mtime_ms(&metadata),
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use hyper::StatusCode;
    use serde_json::Value;
    use tower::ServiceExt;
    use refact_privacy::{PrivacyPolicy, ShellBehavior, SubagentPolicy, Zone};

    use super::{clamped_tree_limit, MAX_CONTENT_BYTES, MAX_ENTRIES};
    use crate::app_state::AppState;
    use crate::global_context::{GlobalContext, SharedGlobalContext};
    use crate::privacy::{FilePrivacySettings, PrivacySettings};

    async fn test_router(workspace_roots: &[&Path]) -> (SharedGlobalContext, Router) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = workspace_roots
            .iter()
            .map(|path| path.to_path_buf())
            .collect();
        set_privacy(gcx.clone(), Vec::new());
        let app = AppState::from_gcx(gcx.clone()).await;
        (gcx, crate::http::routers::make_refact_http_server(app))
    }

    fn set_privacy(gcx: Arc<GlobalContext>, blocked: Vec<String>) {
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked,
            },
            loaded_ts: u64::MAX / 2,
        });
    }

    fn set_privacy_policy(gcx: &Arc<GlobalContext>, policy: PrivacyPolicy) {
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad {
            policy: Arc::new(policy),
            error: None,
            source_paths: Vec::new(),
        };
    }

    fn query_uri(route: &str, pairs: &[(&str, String)]) -> String {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in pairs {
            serializer.append_pair(key, value);
        }
        format!("{route}?{}", serializer.finish())
    }

    async fn get_json(router: Router, uri: String) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn tree_resolves_dotfile_privacy_zones_with_engine_matching() {
        let workspace = tempfile::tempdir().unwrap();
        let secret = workspace.path().join(".env");
        tokio::fs::write(&secret, "TOKEN=secret").await.unwrap();
        let (gcx, router) = test_router(&[workspace.path()]).await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        });
        set_privacy_policy(
            &gcx,
            PrivacyPolicy {
                blocked: Vec::new(),
                zones: vec![
                    Zone {
                        name: "secrets".to_string(),
                        patterns: vec![".env*".to_string()],
                        send_to: Vec::new(),
                        on_shell_read: ShellBehavior::Withhold,
                    },
                    Zone {
                        name: "normal".to_string(),
                        patterns: vec!["**".to_string()],
                        send_to: vec!["*".to_string()],
                        on_shell_read: ShellBehavior::Withhold,
                    },
                ],
                subagents: SubagentPolicy::default(),
                ..Default::default()
            },
        );

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/tree",
                &[("path", workspace.path().to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let secret = response["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == ".env")
            .unwrap();
        assert_eq!(secret["privacy_zone"]["name"], "secrets");
        assert_eq!(secret["privacy_zone"]["send_to"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn tree_rejects_path_outside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, _) = get_json(
            router,
            query_uri(
                "/v1/files/tree",
                &[("path", outside.path().to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    async fn worktree_router(
        source: &Path,
        worktree_id: &str,
    ) -> (PathBuf, SharedGlobalContext, Router) {
        let cache_dir =
            std::env::temp_dir().join(format!("refact-files-wt-{}", uuid::Uuid::new_v4()));
        let config_dir =
            std::env::temp_dir().join(format!("refact-files-cfg-{}", uuid::Uuid::new_v4()));
        let hash = refact_worktrees::service::project_hash_for_path(source);
        let registry_dir = cache_dir.join("worktrees").join(&hash);
        let worktree_root = registry_dir.join(worktree_id);
        std::fs::create_dir_all(&worktree_root).unwrap();
        let registry = refact_worktrees::types::WorktreeRegistry {
            schema_version: 1,
            source_workspace_root: source.to_path_buf(),
            project_hash: hash,
            records: vec![refact_worktrees::types::WorktreeRegistryRecord {
                creator_chat_id: None,
                meta: refact_worktrees::types::WorktreeMeta {
                    id: worktree_id.to_string(),
                    kind: "chat".to_string(),
                    root: worktree_root.clone(),
                    source_workspace_root: source.to_path_buf(),
                    repo_root: source.to_path_buf(),
                    branch: Some("refact/chat/test".to_string()),
                    base_branch: Some("main".to_string()),
                    base_commit: None,
                    task_id: None,
                    card_id: None,
                    agent_id: None,
                    enforce: true,
                },
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                last_seen_at: None,
                references: Vec::new(),
                last_known_status: None,
            }],
        };
        std::fs::write(
            registry_dir.join("index.json"),
            serde_json::to_string_pretty(&registry).unwrap(),
        )
        .unwrap();

        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.to_path_buf()];
        set_privacy(gcx.clone(), Vec::new());
        let app = AppState::from_gcx(gcx.clone()).await;
        (
            worktree_root,
            gcx,
            crate::http::routers::make_refact_http_server(app),
        )
    }

    async fn post_json(router: Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn write_updates_file_and_reports_new_mtime() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("main.rs");
        tokio::fs::write(&path, "fn main() {}\n").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "fn main() { println!(\"hi\"); }\n",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(response["mtime_ms"].as_u64().is_some());
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "fn main() { println!(\"hi\"); }\n"
        );
    }

    #[tokio::test]
    async fn write_rejects_stale_expected_mtime() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("main.rs");
        tokio::fs::write(&path, "original\n").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, _) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "clobbered\n",
                "expected_mtime_ms": 1,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "original\n"
        );
    }

    #[tokio::test]
    async fn write_rechecks_preconditions_after_waiting_for_cooperative_writer() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("main.rs");
        tokio::fs::write(&path, "original\n").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;
        let guard = crate::file_activity::begin_write(
            None,
            &path,
            crate::file_activity::WriteIntent::Contextual,
            "other",
        )
        .await
        .unwrap();
        let mut pending = Box::pin(post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path":path,"content":"mine\n","expected_digest":crate::file_activity::digest_of("original\n")
            }),
        ));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        tokio::fs::write(&path, "other\n").await.unwrap();
        guard.commit(crate::file_activity::WriteKind::Modified, Some("other\n"));
        assert_eq!(pending.await.0, StatusCode::CONFLICT);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "other\n");
    }

    #[tokio::test]
    async fn write_rejects_privacy_blocked_path() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("blocked.secret");
        tokio::fs::write(&path, "nope\n").await.unwrap();
        let (gcx, router) = test_router(&[workspace.path()]).await;
        set_privacy(gcx, vec!["*.secret".to_string()]);

        let (status, _) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "leaked\n",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "nope\n");
    }

    #[tokio::test]
    async fn write_rejects_path_outside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("main.rs");
        tokio::fs::write(&path, "original\n").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, _) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "clobbered\n",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "original\n"
        );
    }

    #[tokio::test]
    async fn write_allows_registered_worktree_path() {
        let source = tempfile::tempdir().unwrap();
        let (worktree_root, _gcx, router) = worktree_router(source.path(), "wt-write").await;
        let path = worktree_root.join("main.rs");
        tokio::fs::write(&path, "old\n").await.unwrap();

        let (status, response) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "new\n",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{response:?}");
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "new\n");
    }

    #[tokio::test]
    async fn read_allows_registered_worktree_path() {
        let source = tempfile::tempdir().unwrap();
        let (worktree_root, _gcx, router) = worktree_router(source.path(), "wt-read").await;
        let path = worktree_root.join("main.rs");
        tokio::fs::write(&path, "fn main() {}\n").await.unwrap();

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{response:?}");
        assert_eq!(response["content"], "fn main() {}\n");
    }

    #[tokio::test]
    async fn read_rejects_unregistered_cache_worktree_path() {
        let source = tempfile::tempdir().unwrap();
        let (worktree_root, gcx, router) = worktree_router(source.path(), "wt-guard").await;
        let sibling = worktree_root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("other-project")
            .join("wt-unregistered");
        std::fs::create_dir_all(&sibling).unwrap();
        let path = sibling.join("secret.rs");
        tokio::fs::write(&path, "nope\n").await.unwrap();
        assert!(path.starts_with(gcx.cache_dir.join("worktrees")));

        let (status, _) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn read_rejects_worktree_root_escaping_the_cache_dir() {
        let source = tempfile::tempdir().unwrap();
        let escape_target = tempfile::tempdir().unwrap();
        let path = escape_target.path().join("secret.rs");
        tokio::fs::write(&path, "secret\n").await.unwrap();

        let cache_dir =
            std::env::temp_dir().join(format!("refact-files-esc-{}", uuid::Uuid::new_v4()));
        let config_dir =
            std::env::temp_dir().join(format!("refact-files-esc-cfg-{}", uuid::Uuid::new_v4()));
        let hash = refact_worktrees::service::project_hash_for_path(source.path());
        let registry_dir = cache_dir.join("worktrees").join(&hash);
        std::fs::create_dir_all(&registry_dir).unwrap();
        let escaping_root = escape_target.path().to_path_buf();
        let ascent = "../".repeat(registry_dir.components().count());
        let relative_id = format!(
            "{ascent}{}",
            escaping_root
                .to_string_lossy()
                .trim_start_matches(std::path::MAIN_SEPARATOR)
        );
        let registry = refact_worktrees::types::WorktreeRegistry {
            schema_version: 1,
            source_workspace_root: source.path().to_path_buf(),
            project_hash: hash,
            records: vec![refact_worktrees::types::WorktreeRegistryRecord {
                creator_chat_id: None,
                meta: refact_worktrees::types::WorktreeMeta {
                    id: relative_id,
                    kind: "chat".to_string(),
                    root: escaping_root,
                    source_workspace_root: source.path().to_path_buf(),
                    repo_root: source.path().to_path_buf(),
                    branch: None,
                    base_branch: None,
                    base_commit: None,
                    task_id: None,
                    card_id: None,
                    agent_id: None,
                    enforce: true,
                },
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                last_seen_at: None,
                references: Vec::new(),
                last_known_status: None,
            }],
        };
        std::fs::write(
            registry_dir.join("index.json"),
            serde_json::to_string_pretty(&registry).unwrap(),
        )
        .unwrap();

        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.path().to_path_buf()];
        set_privacy(gcx.clone(), Vec::new());
        let app = AppState::from_gcx(gcx.clone()).await;
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, _) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn read_rejects_privacy_blocked_path() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("blocked.secret");
        tokio::fs::write(&path, "nope").await.unwrap();
        let (gcx, router) = test_router(&[workspace.path()]).await;
        set_privacy(gcx, vec!["*.secret".to_string()]);

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(response["detail"].as_str().unwrap().contains("privacy"));
    }

    #[tokio::test]
    async fn traversal_outside_workspace_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside.txt");
        tokio::fs::create_dir_all(workspace.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(&outside, "outside").await.unwrap();
        let (_gcx, router) = test_router(&[&workspace]).await;
        let traversal = workspace
            .join("nested")
            .join("..")
            .join("..")
            .join("outside.txt");

        let (status, _) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", traversal.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let link = workspace.path().join("escape.txt");
        symlink(outside.path(), &link).unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, _) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", link.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn big_file_is_truncated_on_utf8_boundary() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("big.txt");
        let content = format!("{}é-tail\nlast\n", "a".repeat(MAX_CONTENT_BYTES - 1));
        tokio::fs::write(&path, content.as_bytes()).await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router.clone(),
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["truncated"], true);
        assert_eq!(
            response["content"].as_str().unwrap().len(),
            MAX_CONTENT_BYTES - 1
        );
        assert_eq!(response["language"], "plaintext");
        assert_eq!(
            response["digest"],
            crate::file_activity::digest_bytes(content.as_bytes())
        );

        let (status, late) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[
                    ("path", path.to_string_lossy().to_string()),
                    ("line_start", "2".to_string()),
                ],
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(late["content"], "last\n");
        assert_eq!(late["truncated"], false);
    }

    #[tokio::test]
    async fn open_reads_external_text_read_only_without_widening_scope() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside = workspace.path().join("main.rs");
        let external = outside.path().join("notes.txt");
        tokio::fs::write(&inside, "fn main() {}\n").await.unwrap();
        tokio::fs::write(&external, "external\n").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, opened) = post_json(
            router.clone(),
            "/v1/files/open",
            serde_json::json!({ "path": external }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{opened:?}");
        assert_eq!(opened["content"], "external\n");
        assert_eq!(opened["can_edit"], false);
        assert_eq!(opened["image_mime"], Value::Null);
        assert_eq!(
            opened["digest"],
            crate::file_activity::digest_of("external\n")
        );

        let (status, inside_opened) = post_json(
            router.clone(),
            "/v1/files/open",
            serde_json::json!({ "path": inside }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(inside_opened["can_edit"], true);

        for (origin, expected) in [
            ("https://evil.example", StatusCode::FORBIDDEN),
            ("null", StatusCode::FORBIDDEN),
            ("http://localhost.evil.example:5173", StatusCode::FORBIDDEN),
            ("http://localhost:5173", StatusCode::OK),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/files/open")
                        .header("content-type", "application/json")
                        .header("origin", origin)
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({ "path": external })).unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{origin}");
            let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
            assert_eq!(
                String::from_utf8_lossy(&body).contains("external"),
                expected == StatusCode::OK,
                "{origin}"
            );
        }

        let (status, _) = get_json(
            router.clone(),
            query_uri(
                "/v1/files/read",
                &[("path", external.to_string_lossy().to_string())],
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = post_json(
            router,
            "/v1/files/write",
            serde_json::json!({ "path": external, "content": "clobbered\n" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            tokio::fs::read_to_string(&external).await.unwrap(),
            "external\n"
        );
    }

    #[tokio::test]
    async fn open_previews_only_bounded_recognized_raster_regular_files() {
        use base64::Engine;

        let outside = tempfile::tempdir().unwrap();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let png_path = outside.path().join("pixel.dat");
        tokio::fs::write(&png_path, &png).await.unwrap();
        let mut oversized = png.clone();
        oversized.resize(super::MAX_IMAGE_BYTES + 1, 0);
        let oversized_path = outside.path().join("huge.png");
        tokio::fs::write(&oversized_path, &oversized).await.unwrap();
        let svg_path = outside.path().join("icon.svg");
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>x</script></svg>";
        tokio::fs::write(&svg_path, svg).await.unwrap();
        let (_gcx, router) = test_router(&[]).await;
        let open = |path: PathBuf| {
            post_json(
                router.clone(),
                "/v1/files/open",
                serde_json::json!({ "path": path }),
            )
        };

        let (status, image) = open(png_path).await;
        assert_eq!(status, StatusCode::OK, "{image:?}");
        assert_eq!(image["image_mime"], "image/png");
        assert_eq!(image["size"], png.len() as u64);
        assert_eq!(image["binary"], true);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(image["image_data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, png);

        let (status, huge) = open(oversized_path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(huge["image_mime"], Value::Null);
        assert_eq!(huge["image_data"], Value::Null);
        assert_eq!(huge["size"], oversized.len() as u64);

        let (status, svg_response) = open(svg_path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(svg_response["image_mime"], Value::Null);
        assert_eq!(svg_response["content"], svg);

        #[cfg(unix)]
        {
            let fifo = outside.path().join("pipe");
            let c_path = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
            let (status, _) = tokio::time::timeout(std::time::Duration::from_secs(10), open(fifo))
                .await
                .expect("opening a FIFO must not block");
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn open_checks_privacy_and_edit_scope_on_canonical_alias() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("token.secret");
        let plain = outside.path().join("plain.txt");
        tokio::fs::write(&secret, "TOKEN\n").await.unwrap();
        tokio::fs::write(&plain, "plain\n").await.unwrap();
        let secret_alias = outside.path().join("alias.txt");
        symlink(&secret, &secret_alias).unwrap();
        let requested_secret_alias = outside.path().join("plain.secret");
        symlink(&plain, &requested_secret_alias).unwrap();
        let workspace_alias = workspace.path().join("linked.txt");
        symlink(&plain, &workspace_alias).unwrap();
        let (gcx, router) = test_router(&[workspace.path()]).await;
        set_privacy(gcx, vec!["*.secret".to_string()]);

        let (status, blocked) = post_json(
            router.clone(),
            "/v1/files/open",
            serde_json::json!({ "path": secret_alias }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(blocked["detail"].as_str().unwrap().contains("privacy"));
        assert!(!blocked.to_string().contains("TOKEN"));

        let (status, blocked) = post_json(
            router.clone(),
            "/v1/files/open",
            serde_json::json!({ "path": requested_secret_alias }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(blocked["detail"].as_str().unwrap().contains("privacy"));

        let (status, aliased) = post_json(
            router,
            "/v1/files/open",
            serde_json::json!({ "path": workspace_alias }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{aliased:?}");
        assert_eq!(aliased["content"], "plain\n");
        assert_eq!(aliased["can_edit"], false);
    }
}
