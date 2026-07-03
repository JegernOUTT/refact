use axum::extract::{Query, State};
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::tools::tool_buddy_get_logs::{
    is_log_candidate, read_bounded_log_tail, redact_sensitive, resolve_log_dir,
};

const DEFAULT_TAIL_LINES: usize = 200;
const MAX_TAIL_LINES: usize = 2000;
const BUNDLE_LOG_TAIL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct BugReportContextResponse {
    engine_version: &'static str,
    os: String,
    http_port: u16,
    cache_dir: String,
    config_dir: String,
    workspace_roots: Vec<String>,
    log_paths: BugReportLogPaths,
    bundle_default_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BugReportLogPaths {
    engine_log_target: String,
    engine_log_exists: bool,
    daemon_log_file: String,
    daemon_log_exists: bool,
    daemon_logs_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct BugReportLogsQuery {
    source: Option<String>,
    tail: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct BugReportLogsResponse {
    source: String,
    path: String,
    exists: bool,
    lines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    read_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecentErrorLine {
    source: String,
    level: String,
    message: String,
}

#[derive(Debug, Serialize)]
pub struct BugReportErrorsResponse {
    errors: Vec<RecentErrorLine>,
}

#[derive(Debug, Deserialize)]
pub struct BugReportBundleRequest {
    dest_dir: Option<String>,
    redact: Option<bool>,
    webui_lines: Option<Vec<String>>,
    ide_lines: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct BugReportBundleResponse {
    path: String,
    size_bytes: u64,
    files: Vec<BugReportBundleFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BugReportBundleFile {
    name: String,
    size_bytes: u64,
}

#[derive(Debug, Clone)]
struct ResolvedLogSource {
    source: &'static str,
    path: PathBuf,
    exists: bool,
}

#[derive(Debug, Clone)]
struct BugReportBundleInput {
    dest_dir: PathBuf,
    context: BugReportContextResponse,
    engine_log: ResolvedLogSource,
    daemon_log: ResolvedLogSource,
    redact: bool,
    webui_lines: Option<Vec<String>>,
    ide_lines: Option<Vec<String>>,
}

fn clamp_tail_lines(tail: Option<usize>) -> usize {
    tail.unwrap_or(DEFAULT_TAIL_LINES).clamp(1, MAX_TAIL_LINES)
}

async fn newest_log_candidate_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    let mut files: Vec<(PathBuf, Option<std::time::SystemTime>, String)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        if !is_log_candidate(&name) {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            if meta.is_file() {
                files.push((path, meta.modified().ok(), name));
            }
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
    files.into_iter().next().map(|(path, _, _)| path)
}

async fn resolve_engine_log(logs_to_file: String, cache_dir: &Path) -> ResolvedLogSource {
    if !logs_to_file.is_empty() {
        let path = PathBuf::from(logs_to_file);
        let exists = tokio::fs::metadata(&path)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false);
        return ResolvedLogSource {
            source: "engine",
            path,
            exists,
        };
    }

    let dir = resolve_log_dir(cache_dir);
    if let Some(path) = newest_log_candidate_in_dir(&dir).await {
        return ResolvedLogSource {
            source: "engine",
            path,
            exists: true,
        };
    }
    ResolvedLogSource {
        source: "engine",
        path: dir,
        exists: false,
    }
}

async fn resolve_daemon_log() -> ResolvedLogSource {
    let path = crate::daemon::paths::daemon_log_path();
    let exists = tokio::fs::metadata(&path)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false);
    ResolvedLogSource {
        source: "daemon",
        path,
        exists,
    }
}

async fn resolve_log_source(
    source: &str,
    logs_to_file: String,
    cache_dir: &Path,
) -> Result<ResolvedLogSource, ScratchError> {
    match source {
        "engine" => Ok(resolve_engine_log(logs_to_file, cache_dir).await),
        "daemon" => Ok(resolve_daemon_log().await),
        _ => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "source must be engine or daemon".to_string(),
        )),
    }
}

async fn build_context_response(app: &AppState) -> BugReportContextResponse {
    let gcx = app.gcx.clone();
    let logs_to_file = gcx.cmdline.logs_to_file.clone();
    let http_port = gcx.cmdline.http_port;
    let cache_dir = gcx.cache_dir.clone();
    let config_dir = gcx.config_dir.clone();
    let workspace_roots = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();

    let engine = resolve_engine_log(logs_to_file, &cache_dir).await;
    let daemon = resolve_daemon_log().await;
    build_context_response_from_parts(
        http_port,
        cache_dir,
        config_dir,
        workspace_roots,
        engine,
        daemon,
    )
}

fn build_context_response_from_parts(
    http_port: u16,
    cache_dir: PathBuf,
    config_dir: PathBuf,
    workspace_roots: Vec<String>,
    engine: ResolvedLogSource,
    daemon: ResolvedLogSource,
) -> BugReportContextResponse {
    BugReportContextResponse {
        engine_version: env!("CARGO_PKG_VERSION"),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        http_port,
        cache_dir: cache_dir.to_string_lossy().to_string(),
        config_dir: config_dir.to_string_lossy().to_string(),
        workspace_roots,
        log_paths: BugReportLogPaths {
            engine_log_target: engine.path.to_string_lossy().to_string(),
            engine_log_exists: engine.exists,
            daemon_log_file: daemon.path.to_string_lossy().to_string(),
            daemon_log_exists: daemon.exists,
            daemon_logs_dir: daemon
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy()
                .to_string(),
        },
        bundle_default_dir: cache_dir.join("bug-reports").to_string_lossy().to_string(),
    }
}

async fn read_redacted_lines(
    source: &ResolvedLogSource,
    tail: usize,
) -> (bool, Vec<String>, Option<String>) {
    if !source.exists || !source.path.is_file() {
        return (false, Vec::new(), None);
    }
    match read_bounded_log_tail(&source.path).await {
        Ok(text) => (
            true,
            text.lines()
                .rev()
                .take(tail)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(redact_sensitive)
                .collect(),
            None,
        ),
        Err(e) => (true, Vec::new(), Some(redact_sensitive(&e))),
    }
}

fn level_for_line(line: &str) -> Option<&'static str> {
    let has_error = line
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == "ERROR");
    if has_error {
        return Some("error");
    }
    let has_warn = line
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == "WARN");
    if has_warn {
        return Some("warn");
    }
    None
}

async fn scan_recent_errors(sources: &[ResolvedLogSource], cap: usize) -> Vec<RecentErrorLine> {
    let mut errors = Vec::new();
    let mut warns = Vec::new();
    for source in sources {
        if !source.exists || !source.path.is_file() {
            continue;
        }
        let Ok(text) = read_bounded_log_tail(&source.path).await else {
            continue;
        };
        for line in text.lines().rev() {
            if let Some(level) = level_for_line(line) {
                let item = RecentErrorLine {
                    source: source.source.to_string(),
                    level: level.to_string(),
                    message: redact_sensitive(line),
                };
                if level == "error" {
                    errors.push(item);
                } else {
                    warns.push(item);
                }
            }
        }
    }
    errors.into_iter().chain(warns).take(cap).collect()
}

async fn read_tail_bytes(path: &Path, max_bytes: u64) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("failed to read log file {:?}: {}", path, e))?;
    let len = file
        .metadata()
        .await
        .map_err(|e| format!("failed to stat log file {:?}: {}", path, e))?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|e| format!("failed to seek log file {:?}: {}", path, e))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .map_err(|e| format!("failed to read log file {:?}: {}", path, e))?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[pos + 1..].to_string();
        }
    }
    Ok(text)
}

fn expand_dest_dir(dest_dir: Option<String>, cache_dir: &Path) -> Result<PathBuf, ScratchError> {
    let path = match dest_dir {
        Some(dest) => {
            let expanded = if let Some(rest) = dest.strip_prefix("~/") {
                home::home_dir()
                    .ok_or_else(|| {
                        ScratchError::new(
                            StatusCode::BAD_REQUEST,
                            "cannot resolve home directory".to_string(),
                        )
                    })?
                    .join(rest)
            } else {
                PathBuf::from(dest)
            };
            if !expanded.is_absolute() {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "dest_dir must be absolute".to_string(),
                ));
            }
            if expanded
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "dest_dir must not contain '..'".to_string(),
                ));
            }
            expanded
        }
        None => cache_dir.join("bug-reports"),
    };
    Ok(path)
}

fn maybe_redact(text: String, redact: bool) -> String {
    if redact {
        redact_sensitive(&text)
    } else {
        text
    }
}

async fn build_bug_report_bundle(
    input: BugReportBundleInput,
) -> Result<BugReportBundleResponse, ScratchError> {
    tokio::fs::create_dir_all(&input.dest_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let dest_dir = tokio::fs::canonicalize(&input.dest_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if input.engine_log.exists && input.engine_log.path.is_file() {
        match read_tail_bytes(&input.engine_log.path, BUNDLE_LOG_TAIL_BYTES).await {
            Ok(text) => entries.push(("engine.log".to_string(), maybe_redact(text, input.redact))),
            Err(e) => notes.push(format!("engine.log skipped: {}", e)),
        }
    }
    if input.daemon_log.exists && input.daemon_log.path.is_file() {
        match read_tail_bytes(&input.daemon_log.path, BUNDLE_LOG_TAIL_BYTES).await {
            Ok(text) => entries.push(("daemon.log".to_string(), maybe_redact(text, input.redact))),
            Err(e) => notes.push(format!("daemon.log skipped: {}", e)),
        }
    }
    if let Some(lines) = input.webui_lines {
        if !lines.is_empty() {
            entries.push((
                "webui.log".to_string(),
                maybe_redact(format!("{}\n", lines.join("\n")), input.redact),
            ));
        }
    }
    if let Some(lines) = input.ide_lines {
        if !lines.is_empty() {
            entries.push((
                "ide.log".to_string(),
                maybe_redact(format!("{}\n", lines.join("\n")), input.redact),
            ));
        }
    }

    let system_info = serde_json::to_string_pretty(&input.context)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    entries.push((
        "system_info.json".to_string(),
        maybe_redact(system_info, input.redact),
    ));

    let errors = scan_recent_errors(&[input.engine_log, input.daemon_log], 50).await;
    let recent_errors = errors
        .into_iter()
        .map(|err| format!("[{}] {}", err.source, err.message))
        .collect::<Vec<_>>()
        .join("\n");
    entries.push((
        "recent_errors.txt".to_string(),
        maybe_redact(
            if recent_errors.is_empty() {
                String::new()
            } else {
                format!("{}\n", recent_errors)
            },
            input.redact,
        ),
    ));
    if !notes.is_empty() {
        entries.push((
            "bundle_notes.txt".to_string(),
            maybe_redact(format!("{}\n", notes.join("\n")), input.redact),
        ));
    }

    let filename = format!(
        "refact-bug-{}.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let zip_path = dest_dir.join(filename);
    let zip_path_for_write = zip_path.clone();
    let files: Vec<BugReportBundleFile> = entries
        .iter()
        .map(|(name, text)| BugReportBundleFile {
            name: name.clone(),
            size_bytes: text.len() as u64,
        })
        .collect();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = std::fs::File::create(&zip_path_for_write)
            .map_err(|e| format!("failed to create zip: {}", e))?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, text) in entries {
            zip.start_file(name, options)
                .map_err(|e| format!("failed to start zip entry: {}", e))?;
            zip.write_all(text.as_bytes())
                .map_err(|e| format!("failed to write zip entry: {}", e))?;
        }
        zip.finish()
            .map_err(|e| format!("failed to finish zip: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let size_bytes = tokio::fs::metadata(&zip_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .len();

    Ok(BugReportBundleResponse {
        path: zip_path.to_string_lossy().to_string(),
        size_bytes,
        files,
    })
}

pub async fn handle_v1_bug_report_context(
    State(app): State<AppState>,
) -> Result<Json<BugReportContextResponse>, ScratchError> {
    Ok(Json(build_context_response(&app).await))
}

pub async fn handle_v1_bug_report_logs(
    State(app): State<AppState>,
    Query(query): Query<BugReportLogsQuery>,
) -> Result<Json<BugReportLogsResponse>, ScratchError> {
    let source = query.source.unwrap_or_else(|| "engine".to_string());
    let tail = clamp_tail_lines(query.tail);
    let gcx = app.gcx.clone();
    let resolved =
        resolve_log_source(&source, gcx.cmdline.logs_to_file.clone(), &gcx.cache_dir).await?;
    let (exists, lines, read_error) = read_redacted_lines(&resolved, tail).await;
    Ok(Json(BugReportLogsResponse {
        source,
        path: resolved.path.to_string_lossy().to_string(),
        exists,
        lines,
        read_error,
    }))
}

pub async fn handle_v1_bug_report_errors(
    State(app): State<AppState>,
) -> Result<Json<BugReportErrorsResponse>, ScratchError> {
    let gcx = app.gcx.clone();
    let engine = resolve_engine_log(gcx.cmdline.logs_to_file.clone(), &gcx.cache_dir).await;
    let daemon = resolve_daemon_log().await;
    Ok(Json(BugReportErrorsResponse {
        errors: scan_recent_errors(&[engine, daemon], 50).await,
    }))
}

pub async fn handle_v1_bug_report_bundle(
    State(app): State<AppState>,
    Json(req): Json<BugReportBundleRequest>,
) -> Result<Json<BugReportBundleResponse>, ScratchError> {
    let gcx = app.gcx.clone();
    let cache_dir = gcx.cache_dir.clone();
    let dest_dir = expand_dest_dir(req.dest_dir, &cache_dir)?;
    let context = build_context_response(&app).await;
    let engine = resolve_engine_log(gcx.cmdline.logs_to_file.clone(), &cache_dir).await;
    let daemon = resolve_daemon_log().await;
    let response = build_bug_report_bundle(BugReportBundleInput {
        dest_dir,
        context,
        engine_log: engine,
        daemon_log: daemon,
        redact: req.redact.unwrap_or(true),
        webui_lines: req.webui_lines,
        ide_lines: req.ide_lines,
    })
    .await?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[tokio::test]
    async fn errors_scan_newest_first_errors_before_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.log");
        tokio::fs::write(
            &path,
            concat!(
                "2026 INFO hello\n",
                "2026 WARN older warn\n",
                "2026 ERROR older error\n",
                "2026 WARN newest warn\n",
                "2026 ERROR newest error\n",
            ),
        )
        .await
        .unwrap();
        let source = ResolvedLogSource {
            source: "engine",
            path,
            exists: true,
        };

        let errors = scan_recent_errors(&[source], 3).await;

        assert_eq!(errors[0].level, "error");
        assert!(errors[0].message.contains("newest error"));
        assert_eq!(errors[1].level, "error");
        assert!(errors[1].message.contains("older error"));
        assert_eq!(errors[2].level, "warn");
        assert!(errors[2].message.contains("newest warn"));
    }

    #[tokio::test]
    async fn bundle_contains_redacted_engine_log_and_system_info() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let dest_dir = dir.path().join("bundles");
        tokio::fs::create_dir_all(&cache_dir).await.unwrap();
        let engine_path = cache_dir.join("engine.log");
        tokio::fs::write(&engine_path, "api_key=sk-abc123defgh456ijklmn0000\n")
            .await
            .unwrap();
        let engine = ResolvedLogSource {
            source: "engine",
            path: engine_path,
            exists: true,
        };
        let daemon = ResolvedLogSource {
            source: "daemon",
            path: cache_dir.join("missing-daemon.log"),
            exists: false,
        };
        let context = build_context_response_from_parts(
            8001,
            cache_dir.clone(),
            dir.path().join("config"),
            Vec::new(),
            engine.clone(),
            daemon.clone(),
        );

        let response = build_bug_report_bundle(BugReportBundleInput {
            dest_dir,
            context,
            engine_log: engine,
            daemon_log: daemon,
            redact: true,
            webui_lines: None,
            ide_lines: None,
        })
        .await
        .unwrap();

        assert!(Path::new(&response.path).exists());
        let file = std::fs::File::open(&response.path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut engine_log = String::new();
        archive
            .by_name("engine.log")
            .unwrap()
            .read_to_string(&mut engine_log)
            .unwrap();
        assert!(!engine_log.contains("sk-abc123defgh456ijklmn0000"));
        assert!(engine_log.contains("[REDACTED]"));
        archive.by_name("system_info.json").unwrap();
    }

    #[test]
    fn tail_clamp_bounds() {
        assert_eq!(clamp_tail_lines(Some(0)), 1);
        assert_eq!(clamp_tail_lines(Some(99_999)), 2000);
        assert_eq!(clamp_tail_lines(None), 200);
    }

    #[test]
    fn dest_dir_validation() {
        let cache_dir = Path::new("/tmp/refact-cache");
        assert_eq!(
            expand_dest_dir(None, cache_dir).unwrap(),
            cache_dir.join("bug-reports")
        );
        #[cfg(unix)]
        {
            assert_eq!(
                expand_dest_dir(Some("/tmp/bundles".to_string()), cache_dir).unwrap(),
                PathBuf::from("/tmp/bundles")
            );
        }
        assert!(expand_dest_dir(Some("relative/dir".to_string()), cache_dir).is_err());
        assert!(expand_dest_dir(Some("/tmp/ok/../escape".to_string()), cache_dir).is_err());
    }
}
