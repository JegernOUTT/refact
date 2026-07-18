use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Query, State};
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_blocklist::{is_blocklisted, IndexingEverywhere};
use crate::files_correction::{check_if_its_inside_a_workspace_or_config, get_unscoped_project_dirs};
use crate::files_in_workspace::check_file_privacy_for_send;
use crate::global_context::GlobalContext;

const DEFAULT_MAX_ENTRIES: usize = 2_000;
const MAX_ENTRIES: usize = 5_000;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
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
    #[serde(skip_serializing_if = "is_false")]
    binary: bool,
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
    check_if_its_inside_a_workspace_or_config(gcx.clone(), requested)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    let canonical = tokio::fs::canonicalize(requested)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|error| io_error(StatusCode::NOT_FOUND, "resolve path", requested, error))?;
    check_if_its_inside_a_workspace_or_config(gcx, &canonical)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    Ok(canonical)
}

fn clamped_tree_limit(max_entries: Option<usize>) -> usize {
    max_entries.unwrap_or(DEFAULT_MAX_ENTRIES).min(MAX_ENTRIES)
}

fn hidden_or_heavy_name(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target")
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

async fn workspace_root_entries(gcx: Arc<GlobalContext>) -> Result<Vec<TreeEntry>, ScratchError> {
    let mut entries = Vec::new();
    for root in get_unscoped_project_dirs(gcx).await {
        let canonical = tokio::fs::canonicalize(&root)
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
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

async fn list_dir_core(
    directory: &Path,
    indexing: &IndexingEverywhere,
) -> Result<Vec<TreeEntry>, ScratchError> {
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
        if hidden_or_heavy_name(&name) {
            continue;
        }
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
        if blocklisted_entry(indexing, directory, &path, is_dir) {
            continue;
        }
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

fn slice_lines(text: &str, line_start: Option<usize>, line_end: Option<usize>) -> String {
    if line_start.is_none() && line_end.is_none() {
        return text.to_string();
    }
    let start = line_start.unwrap_or(1);
    let end = line_end.unwrap_or(usize::MAX);
    text.split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            (line_number >= start && line_number <= end).then_some(line)
        })
        .collect()
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

async fn read_file_core(
    path: &Path,
    line_start: Option<usize>,
    line_end: Option<usize>,
) -> Result<ReadResponse, ScratchError> {
    validate_line_range(line_start, line_end)?;
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| io_error(StatusCode::NOT_FOUND, "inspect file", path, error))?;
    if !metadata.is_file() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Path '{}' is not a file", path.display()),
        ));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| io_error(StatusCode::INTERNAL_SERVER_ERROR, "read file", path, error))?;
    let binary = bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0);
    let text = if binary {
        None
    } else {
        std::str::from_utf8(&bytes).ok()
    };
    let language = language_for_path(path);
    let path = path.to_string_lossy().to_string();
    if let Some(text) = text {
        let (content, truncated) = truncate_content(slice_lines(text, line_start, line_end));
        return Ok(ReadResponse {
            path,
            content,
            language,
            size: metadata.len(),
            truncated,
            line_start,
            line_end,
            mtime_ms: mtime_ms(&metadata),
            binary: false,
        });
    }
    Ok(ReadResponse {
        path,
        content: String::new(),
        language,
        size: metadata.len(),
        truncated: false,
        line_start,
        line_end,
        mtime_ms: mtime_ms(&metadata),
        binary: true,
    })
}

pub async fn handle_v1_files_tree(
    State(app): State<AppState>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, ScratchError> {
    let gcx = app.gcx.clone();
    let limit = clamped_tree_limit(query.max_entries);
    let raw_path = query.path.unwrap_or_default();
    if raw_path.is_empty() {
        let entries = workspace_root_entries(gcx).await?;
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
    let entries = list_dir_core(&path, &indexing).await?;
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
    check_file_privacy_for_send(app.gcx.clone(), &requested)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    check_file_privacy_for_send(app.gcx, &path)
        .await
        .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    Ok(Json(
        read_file_core(&path, query.line_start, query.line_end).await?,
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use hyper::StatusCode;
    use serde_json::Value;
    use tower::ServiceExt;

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
    async fn empty_tree_path_returns_workspace_roots() {
        let temp = tempfile::tempdir().unwrap();
        let alpha = temp.path().join("alpha");
        let beta = temp.path().join("Beta");
        tokio::fs::create_dir_all(&alpha).await.unwrap();
        tokio::fs::create_dir_all(&beta).await.unwrap();
        let (_gcx, router) = test_router(&[&beta, &alpha]).await;

        let (status, response) = get_json(
            router,
            query_uri("/v1/files/tree", &[("path", String::new())]),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["path"], "");
        assert_eq!(response["truncated"], false);
        assert_eq!(response["entries"][0]["name"], "alpha");
        assert_eq!(response["entries"][0]["kind"], "dir");
        assert_eq!(response["entries"][0]["size"], Value::Null);
        assert_eq!(response["entries"][1]["name"], "Beta");
    }

    #[tokio::test]
    async fn tree_is_lazy_sorted_filtered_and_capped() {
        let workspace = tempfile::tempdir().unwrap();
        for directory in ["BDir", "adir", ".git", ".hidden", "node_modules", "target"] {
            tokio::fs::create_dir_all(workspace.path().join(directory))
                .await
                .unwrap();
        }
        tokio::fs::write(workspace.path().join("z.txt"), "zzz")
            .await
            .unwrap();
        tokio::fs::write(workspace.path().join("A.txt"), "a")
            .await
            .unwrap();
        let nested = workspace.path().join("adir").join("nested.txt");
        tokio::fs::write(&nested, "nested").await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/tree",
                &[
                    ("path", workspace.path().to_string_lossy().to_string()),
                    ("max_entries", "3".to_string()),
                ],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["truncated"], true);
        assert_eq!(response["entries"].as_array().unwrap().len(), 3);
        assert_eq!(response["entries"][0]["name"], "adir");
        assert_eq!(response["entries"][1]["name"], "BDir");
        assert_eq!(response["entries"][2]["name"], "A.txt");
        assert!(response.to_string().contains("nested.txt") == false);
        assert_eq!(clamped_tree_limit(Some(MAX_ENTRIES + 1)), MAX_ENTRIES);
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
        let content = format!("{}é-tail", "a".repeat(MAX_CONTENT_BYTES - 1));
        tokio::fs::write(&path, content.as_bytes()).await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router,
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
    }

    #[tokio::test]
    async fn binary_file_returns_metadata_without_content() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("image.bin");
        let bytes = [1_u8, 2, 0, 3, 4];
        tokio::fs::write(&path, bytes).await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["content"], "");
        assert_eq!(response["binary"], true);
        assert_eq!(response["size"], bytes.len());
        assert_eq!(response["truncated"], false);
        assert!(response["mtime_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn read_slices_inclusive_line_range_before_capping() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("lines.rs");
        tokio::fs::write(&path, "one\ntwo\nthree\nfour\n")
            .await
            .unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[
                    ("path", path.to_string_lossy().to_string()),
                    ("line_start", "2".to_string()),
                    ("line_end", "3".to_string()),
                ],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["content"], "two\nthree\n");
        assert_eq!(response["line_start"], 2);
        assert_eq!(response["line_end"], 3);
        assert_eq!(response["language"], "rust");
        assert_eq!(response.get("binary"), None);
    }

    #[tokio::test]
    async fn invalid_utf8_is_binary() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("invalid.txt");
        tokio::fs::write(&path, [0xff, 0xfe, b'a']).await.unwrap();
        let (_gcx, router) = test_router(&[workspace.path()]).await;

        let (status, response) = get_json(
            router,
            query_uri(
                "/v1/files/read",
                &[("path", path.to_string_lossy().to_string())],
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["binary"], true);
        assert_eq!(response["content"], "");
    }
}
