use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::stats::event::{canonicalize_mode_for_stats, LlmCallEvent};
use crate::stats::reader::{aggregate_summary, read_stats_events_from_dirs};

const SUMMARY_CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Deserialize, Eq, Hash, PartialEq)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct StatsEventsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub chat_id: Option<String>,
    pub success: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(serde::Serialize)]
struct EventsResponse<'a> {
    events: &'a [LlmCallEvent],
    total: usize,
    limit: usize,
    offset: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StatsFingerprint {
    file_count: usize,
    total_size: u64,
    max_modified_ns: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SummaryCacheKey {
    stats_dirs: Vec<PathBuf>,
    fingerprint: StatsFingerprint,
    from: Option<String>,
    to: Option<String>,
}

impl Hash for SummaryCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.stats_dirs.hash(state);
        self.fingerprint.hash(state);
        self.from.hash(state);
        self.to.hash(state);
    }
}

struct SummaryCacheEntry {
    inserted: Instant,
    body: String,
}

type SummaryCache = HashMap<SummaryCacheKey, SummaryCacheEntry>;

fn summary_cache() -> &'static Mutex<SummaryCache> {
    static CACHE: OnceLock<Mutex<SummaryCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scratch_internal(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, message.into())
}

fn json_response(body: String) -> Result<Response<Body>, ScratchError> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| scratch_internal(format!("response error: {}", e)))
}

fn stats_fingerprint(stats_dirs: &[PathBuf]) -> StatsFingerprint {
    let mut fingerprint = StatsFingerprint {
        file_count: 0,
        total_size: 0,
        max_modified_ns: 0,
    };
    for stats_dir in stats_dirs {
        let Ok(entries) = std::fs::read_dir(stats_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            fingerprint.file_count += 1;
            fingerprint.total_size = fingerprint.total_size.saturating_add(metadata.len());
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            fingerprint.max_modified_ns = fingerprint.max_modified_ns.max(modified_ns);
        }
    }
    fingerprint
}

fn cached_summary_body<F>(
    cache: &Mutex<SummaryCache>,
    key: SummaryCacheKey,
    compute: F,
) -> Result<String, ScratchError>
where
    F: FnOnce() -> Result<String, ScratchError>,
{
    let now = Instant::now();
    if let Ok(cache) = cache.lock() {
        if let Some(entry) = cache.get(&key) {
            if now.duration_since(entry.inserted) < SUMMARY_CACHE_TTL {
                return Ok(entry.body.clone());
            }
        }
    }

    let body = compute()?;
    if let Ok(mut cache) = cache.lock() {
        cache.retain(|_, entry| now.duration_since(entry.inserted) < SUMMARY_CACHE_TTL);
        cache.insert(
            key,
            SummaryCacheEntry {
                inserted: now,
                body: body.clone(),
            },
        );
    }
    Ok(body)
}

fn sequence_number(path: &Path) -> Option<u32> {
    path.file_stem()?.to_str()?.parse().ok()
}

fn stats_files_newest_first(stats_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for (dir_index, stats_dir) in stats_dirs.iter().enumerate() {
        let Ok(entries) = std::fs::read_dir(stats_dir) else {
            continue;
        };
        files.extend(entries.flatten().filter_map(|entry| {
            let path = entry.path();
            sequence_number(&path).map(|sequence| (sequence, dir_index, path))
        }));
    }
    files.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| right.2.cmp(&left.2))
    });
    files.into_iter().map(|(_, _, path)| path).collect()
}

fn event_matches(event: &LlmCallEvent, params: &StatsEventsQuery) -> bool {
    let event_day = event.ts_start.get(..10).unwrap_or("");
    if params
        .from
        .as_deref()
        .is_some_and(|from| event_day < from.get(..10).unwrap_or(""))
    {
        return false;
    }
    if params
        .to
        .as_deref()
        .is_some_and(|to| event_day > to.get(..10).unwrap_or(""))
    {
        return false;
    }
    if params
        .model
        .as_deref()
        .is_some_and(|model| !event.model_id.starts_with(model))
    {
        return false;
    }
    if params
        .provider
        .as_deref()
        .is_some_and(|provider| event.provider != provider)
    {
        return false;
    }
    if params
        .chat_id
        .as_deref()
        .is_some_and(|chat_id| event.chat_id != chat_id)
    {
        return false;
    }
    !params
        .success
        .is_some_and(|success| event.success != success)
}

fn scan_events_newest_first<F>(
    stats_dirs: &[PathBuf],
    params: &StatsEventsQuery,
    target: usize,
    mut file_opened: F,
) -> Vec<LlmCallEvent>
where
    F: FnMut(&Path),
{
    if target == 0 {
        return Vec::new();
    }
    let mut events = Vec::with_capacity(target);
    let mut seen_ids = HashSet::new();
    for path in stats_files_newest_first(stats_dirs) {
        if events.len() >= target {
            break;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        file_opened(&path);
        for line in content.lines().rev() {
            let Ok(mut event) = serde_json::from_str::<LlmCallEvent>(line.trim()) else {
                continue;
            };
            event.mode = canonicalize_mode_for_stats(&event.mode);
            if !event_matches(&event, params) {
                continue;
            }
            if !event.id.is_empty() && !seen_ids.insert(event.id.clone()) {
                continue;
            }
            events.push(event);
            if events.len() >= target {
                break;
            }
        }
    }
    events
}

pub async fn handle_v1_stats_llm_summary(
    State(app): State<AppState>,
    Query(params): Query<StatsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let stats_dirs = crate::stats::get_stats_dirs_for_read(app.gcx.clone()).await;
    let body = tokio::task::spawn_blocking(move || {
        let key = SummaryCacheKey {
            stats_dirs: stats_dirs.clone(),
            fingerprint: stats_fingerprint(&stats_dirs),
            from: params.from.clone(),
            to: params.to.clone(),
        };
        cached_summary_body(summary_cache(), key, || {
            let events = read_stats_events_from_dirs(
                &stats_dirs,
                params.from.as_deref(),
                params.to.as_deref(),
            );
            serde_json::to_string(&aggregate_summary(
                &events,
                params.from.as_deref(),
                params.to.as_deref(),
            ))
            .map_err(|e| scratch_internal(format!("serialization error: {}", e)))
        })
    })
    .await
    .map_err(|e| scratch_internal(format!("stats summary task failed: {}", e)))??;
    json_response(body)
}

pub async fn handle_v1_stats_llm_events(
    State(app): State<AppState>,
    Query(params): Query<StatsEventsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let stats_dirs = crate::stats::get_stats_dirs_for_read(app.gcx.clone()).await;
    let limit = params.limit.unwrap_or(100).min(1000);
    let offset = params.offset.unwrap_or(0);
    let target = offset.saturating_add(limit);
    let events = tokio::task::spawn_blocking(move || {
        scan_events_newest_first(&stats_dirs, &params, target, |_| {})
    })
    .await
    .map_err(|e| scratch_internal(format!("stats events task failed: {}", e)))?;
    let total = events.len();
    let end = offset.saturating_add(limit).min(total);
    let page = if offset < end {
        &events[offset..end]
    } else {
        &[]
    };
    let resp = EventsResponse {
        events: page,
        total,
        limit,
        offset,
    };
    let body = serde_json::to_string(&resp)
        .map_err(|e| scratch_internal(format!("serialization error: {}", e)))?;
    json_response(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_event(id: &str) -> LlmCallEvent {
        LlmCallEvent {
            id: id.to_string(),
            ts_start: "2026-01-01T00:00:00Z".to_string(),
            ts_end: "2026-01-01T00:00:01Z".to_string(),
            duration_ms: 1,
            chat_id: "chat".to_string(),
            root_chat_id: None,
            mode: "agent".to_string(),
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
            model_id: "provider/model".to_string(),
            provider: "provider".to_string(),
            model: "model".to_string(),
            messages_count: 1,
            tools_count: 0,
            max_tokens: 1,
            temperature: None,
            success: true,
            error_message: None,
            finish_reason: Some("stop".to_string()),
            attempt_n: 1,
            retry_reason: None,
            prompt_tokens: 1,
            completion_tokens: 1,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 2,
            cost_usd: None,
        }
    }

    fn write_event(path: &Path, event: &LlmCallEvent) {
        let line = serde_json::to_string(event).unwrap_or_default();
        let _ = std::fs::write(path, format!("{}\n", line));
    }

    #[test]
    fn events_stop_after_newest_file_satisfies_limit() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let path = dir.path();
        let _ = std::fs::write(path.join("00000001.jsonl"), "invalid old data\n");
        let _ = std::fs::write(path.join("00000002.jsonl"), "invalid old data\n");
        write_event(&path.join("00000003.jsonl"), &make_event("new"));
        let opened = AtomicUsize::new(0);
        let params = StatsEventsQuery {
            from: None,
            to: None,
            model: None,
            provider: None,
            chat_id: None,
            success: None,
            limit: Some(1),
            offset: None,
        };

        let events = scan_events_newest_first(&[path.to_path_buf()], &params, 1, |_| {
            opened.fetch_add(1, Ordering::Relaxed);
        });

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "new");
        assert_eq!(opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn summary_cache_hits_then_misses_after_append() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let file = dir.path().join("00000001.jsonl");
        write_event(&file, &make_event("one"));
        let cache = Mutex::new(HashMap::new());
        let computes = AtomicUsize::new(0);
        let make_key = || SummaryCacheKey {
            stats_dirs: vec![dir.path().to_path_buf()],
            fingerprint: stats_fingerprint(&[dir.path().to_path_buf()]),
            from: None,
            to: None,
        };
        let compute = || {
            computes.fetch_add(1, Ordering::Relaxed);
            Ok("body".to_string())
        };

        let _ = cached_summary_body(&cache, make_key(), compute);
        let _ = cached_summary_body(&cache, make_key(), compute);
        assert_eq!(computes.load(Ordering::Relaxed), 1);

        use std::io::Write;
        if let Ok(mut output) = std::fs::OpenOptions::new().append(true).open(&file) {
            let _ = writeln!(output, "{{}}");
        }
        let _ = cached_summary_body(&cache, make_key(), compute);
        assert_eq!(computes.load(Ordering::Relaxed), 2);
    }
}
