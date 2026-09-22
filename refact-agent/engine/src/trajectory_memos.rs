use std::path::{Path, PathBuf};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::time::{Duration as TokioDuration, Instant};
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::buddy::memory_lifecycle::{
    compute_content_hash, compute_idempotency_key, load_memory_doc_snapshots_from_knowledge_dirs,
    MemoryCandidate, MemoryCandidateStatus, MemoryDocSnapshot, MemoryLifecycleOp, MemoryOpType,
    MemorySource,
};
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::trajectories::extract_text_with_image_placeholders_from_json;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::memories::extract_file_paths;
use crate::subchat::{run_subchat_once, TraceParent};
use crate::yaml_configs::customization_registry::get_subagent_config;

const ABANDONED_THRESHOLD_HOURS: i64 = 2;
const CHECK_INTERVAL_SECS: u64 = 300;
const TRAJECTORIES_FOLDER: &str = ".refact/trajectories";
const SUBAGENT_ID: &str = "memo_extraction";
const MAX_TRAJECTORY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MEMO_CONTENT_CHARS: usize = 4_000;
const MAX_MEMO_EVIDENCE_CHARS: usize = 1_000;
const MAX_TRAJECTORIES_PER_SWEEP: usize = 25;
const MAX_EXTRACTION_ATTEMPTS: u64 = 3;
const RATE_LIMIT_BREAKER_THRESHOLD: usize = 3;
const BREAKER_COOLDOWNS_SECS: &[u64] = &[300, 900, 2700, 10_800, 21_600];

#[derive(Debug)]
struct TrajectoryCandidate {
    path: PathBuf,
    updated_at: DateTime<Utc>,
}

struct MemoBreaker {
    consecutive_rate_limited: usize,
    open_until: Option<Instant>,
    escalation: usize,
}

impl Default for MemoBreaker {
    fn default() -> Self {
        Self {
            consecutive_rate_limited: 0,
            open_until: None,
            escalation: 0,
        }
    }
}

impl MemoBreaker {
    fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|open_until| now < open_until)
    }

    fn close_if_expired(&mut self, now: Instant) -> bool {
        if self.open_until.is_some_and(|open_until| now >= open_until) {
            self.open_until = None;
            true
        } else {
            false
        }
    }

    fn record_success(&mut self) {
        self.consecutive_rate_limited = 0;
        self.open_until = None;
        self.escalation = 0;
    }

    fn record_failure(
        &mut self,
        category: &crate::buddy::workflows::WorkflowFailureCategory,
        now: Instant,
    ) -> Option<TokioDuration> {
        if *category != crate::buddy::workflows::WorkflowFailureCategory::ProviderRateLimit {
            self.consecutive_rate_limited = 0;
            return None;
        }

        self.consecutive_rate_limited += 1;
        if self.consecutive_rate_limited < RATE_LIMIT_BREAKER_THRESHOLD {
            return None;
        }

        let cooldown = TokioDuration::from_secs(BREAKER_COOLDOWNS_SECS[self.escalation]);
        self.open_until = Some(now + cooldown);
        self.escalation = (self.escalation + 1).min(BREAKER_COOLDOWNS_SECS.len() - 1);
        Some(cooldown)
    }
}

struct MemoExtractionRequest {
    gcx: Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    current_title: String,
    is_title_generated: bool,
    trajectory_id: String,
    root_chat_id: String,
}

type MemoExtractionFuture = Pin<Box<dyn Future<Output = Result<ExtractionResult, String>> + Send>>;
type MemoExtractor = dyn Fn(MemoExtractionRequest) -> MemoExtractionFuture + Send + Sync;

enum MemoProcessingOutcome {
    Extracted,
    Skipped,
    Failed(MemoProcessingFailure),
}

struct MemoProcessingFailure {
    category: crate::buddy::workflows::WorkflowFailureCategory,
    error: String,
}

pub async fn trajectory_memos_background_task(gcx: Arc<GlobalContext>) {
    let extractor: Arc<MemoExtractor> = Arc::new(extract_memos_for_trajectory);
    trajectory_memos_background_task_inner(gcx, MemoBreaker::default(), extractor).await;
}

async fn trajectory_memos_background_task_inner(
    gcx: Arc<GlobalContext>,
    mut breaker: MemoBreaker,
    extractor: Arc<MemoExtractor>,
) {
    loop {
        let shutdown_flag = gcx.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Trajectory memos: shutdown detected, stopping");
                return;
            }
        }

        let now = Instant::now();
        if breaker.close_if_expired(now) {
            info!("trajectory_memos: rate-limit breaker closed");
        }
        if breaker.is_open(now) {
            continue;
        }

        if let Err(e) =
            process_abandoned_trajectories(gcx.clone(), &mut breaker, extractor.as_ref()).await
        {
            warn!("trajectory_memos: error processing trajectories: {}", e);
        }
    }
}

async fn process_abandoned_trajectories(
    gcx: Arc<GlobalContext>,
    breaker: &mut MemoBreaker,
    extractor: &MemoExtractor,
) -> Result<(), String> {
    process_abandoned_trajectories_with_enabled(gcx, breaker, extractor, trajectory_memos_enabled())
        .await
}

async fn process_abandoned_trajectories_with_enabled(
    gcx: Arc<GlobalContext>,
    breaker: &mut MemoBreaker,
    extractor: &MemoExtractor,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    if project_dirs.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    let threshold = now - Duration::hours(ABANDONED_THRESHOLD_HOURS);
    let candidates = collect_trajectory_candidates(project_dirs, &threshold).await?;

    for candidate in candidates.into_iter().take(MAX_TRAJECTORIES_PER_SWEEP) {
        let Some(outcome) = process_trajectory_until_shutdown(
            gcx.clone(),
            candidate.path.clone(),
            &threshold,
            extractor,
        )
        .await
        else {
            return Ok(());
        };

        match outcome {
            MemoProcessingOutcome::Extracted => {
                breaker.record_success();
                info!(
                    "trajectory_memos: extracted memos from {}",
                    candidate.path.display()
                );
            }
            MemoProcessingOutcome::Skipped => {}
            MemoProcessingOutcome::Failed(failure) => {
                record_extraction_failure(&candidate.path, &failure).await?;
                if let Some(cooldown) = breaker.record_failure(&failure.category, Instant::now()) {
                    warn!(
                        cooldown_secs = cooldown.as_secs(),
                        "trajectory_memos: rate-limit breaker opened after {} consecutive provider rate-limit failures",
                        RATE_LIMIT_BREAKER_THRESHOLD
                    );
                    return Ok(());
                }
                warn!(
                    "trajectory_memos: failed to process {}: {}",
                    candidate.path.display(),
                    failure.error
                );
            }
        }
    }

    Ok(())
}

fn trajectory_memos_enabled() -> bool {
    trajectory_memos_enabled_from_env(
        std::env::var("REFACT_DISABLE_TRAJECTORY_MEMOS")
            .ok()
            .as_deref(),
    )
}

fn trajectory_memos_enabled_from_env(value: Option<&str>) -> bool {
    value.map_or(true, |value| {
        let value = value.trim();
        value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false")
    })
}

async fn collect_trajectory_candidates(
    project_dirs: Vec<PathBuf>,
    threshold: &DateTime<Utc>,
) -> Result<Vec<TrajectoryCandidate>, String> {
    let mut candidates = Vec::new();

    for workspace_root in project_dirs {
        let trajectories_dir = workspace_root.join(TRAJECTORIES_FOLDER);
        if !trajectories_dir.exists() {
            continue;
        }

        for entry in WalkDir::new(&trajectories_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if !entry.file_type().is_file() || !is_trajectory_path(path) || is_memo_state_path(path)
            {
                continue;
            }
            if memo_state_path(&path).exists() {
                continue;
            }

            let metadata = fs::metadata(&path)
                .await
                .map_err(|error| error.to_string())?;
            if metadata.len() > MAX_TRAJECTORY_BYTES {
                mark_oversize_trajectory(&path, metadata.len()).await?;
                continue;
            }

            let content = fs::read_to_string(&path)
                .await
                .map_err(|error| error.to_string())?;
            let mut trajectory: Value = match serde_json::from_str(&content) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(updated_at) = eligible_trajectory_updated_at(&trajectory, threshold) else {
                continue;
            };

            if is_buddy_system_trajectory(&trajectory) {
                mark_trajectory_skipped(&path, &mut trajectory, "skipped: buddy system trajectory")
                    .await?;
                continue;
            }
            let Some(messages) = trajectory.get("messages").and_then(Value::as_array) else {
                continue;
            };
            if build_chat_messages(messages).is_empty() {
                mark_trajectory_skipped(&path, &mut trajectory, "skipped: invalid chat history")
                    .await?;
                continue;
            }

            candidates.push(TrajectoryCandidate {
                path: path.to_path_buf(),
                updated_at,
            });
        }
    }

    candidates.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(candidates)
}

fn is_trajectory_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json")
}

fn is_memo_state_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".memo-state.json"))
}

fn memo_state_path(path: &Path) -> PathBuf {
    path.with_extension("memo-state.json")
}

fn eligible_trajectory_updated_at(
    trajectory: &Value,
    threshold: &DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if trajectory
        .get("memo_extracted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || trajectory.get("memo_extraction_abandoned").is_some()
    {
        return None;
    }

    let updated_at = trajectory
        .get("updated_at")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))?;
    if updated_at >= *threshold {
        return None;
    }

    let messages = trajectory.get("messages").and_then(Value::as_array)?;
    (messages.len() >= 10).then_some(updated_at)
}

async fn mark_oversize_trajectory(path: &Path, size_bytes: u64) -> Result<(), String> {
    let state = json!({
        "memo_extraction_skipped": {
            "reason": "oversize",
            "size_bytes": size_bytes,
            "recorded_at": Utc::now().to_rfc3339(),
        }
    });
    write_json_atomically(&memo_state_path(path), &state).await
}

async fn record_extraction_failure(
    path: &Path,
    failure: &MemoProcessingFailure,
) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|error| error.to_string())?;
    let mut trajectory: Value =
        serde_json::from_str(&content).map_err(|error| error.to_string())?;
    let trajectory_object = trajectory.as_object_mut().ok_or("Invalid trajectory")?;
    let attempts = trajectory_object
        .get("memo_extraction_attempt")
        .and_then(|attempt| attempt.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    let attempt = json!({
        "count": attempts,
        "last_attempt_at": Utc::now().to_rfc3339(),
        "last_error_class": failure.category.as_str(),
    });
    trajectory_object.insert("memo_extraction_attempt".to_string(), attempt.clone());
    if attempts >= MAX_EXTRACTION_ATTEMPTS {
        trajectory_object.insert("memo_extraction_abandoned".to_string(), attempt);
    }
    write_trajectory_json(path, &trajectory).await
}

async fn process_trajectory_until_shutdown(
    gcx: Arc<GlobalContext>,
    path: PathBuf,
    threshold: &DateTime<Utc>,
    extractor: &MemoExtractor,
) -> Option<MemoProcessingOutcome> {
    let shutdown_flag = gcx.shutdown_flag.clone();
    tokio::select! {
        outcome = process_single_trajectory_with_extractor(gcx, path, threshold, extractor) => Some(outcome),
        _ = wait_for_shutdown(shutdown_flag) => None,
    }
}

async fn wait_for_shutdown(shutdown_flag: Arc<std::sync::atomic::AtomicBool>) {
    while !shutdown_flag.load(Ordering::SeqCst) {
        tokio::time::sleep(TokioDuration::from_millis(200)).await;
    }
}

async fn process_single_trajectory(
    gcx: Arc<GlobalContext>,
    path: PathBuf,
    threshold: &DateTime<Utc>,
) -> Result<bool, String> {
    match process_single_trajectory_with_extractor(
        gcx,
        path,
        threshold,
        &extract_memos_for_trajectory,
    )
    .await
    {
        MemoProcessingOutcome::Extracted => Ok(true),
        MemoProcessingOutcome::Skipped => Ok(false),
        MemoProcessingOutcome::Failed(failure) => Err(failure.error),
    }
}

async fn process_single_trajectory_with_extractor(
    gcx: Arc<GlobalContext>,
    path: PathBuf,
    threshold: &DateTime<Utc>,
    extractor: &MemoExtractor,
) -> MemoProcessingOutcome {
    let result = process_single_trajectory_inner(gcx, path, threshold, extractor).await;
    match result {
        Ok(true) => MemoProcessingOutcome::Extracted,
        Ok(false) => MemoProcessingOutcome::Skipped,
        Err(error) => MemoProcessingOutcome::Failed(MemoProcessingFailure {
            category: crate::buddy::workflows::WorkflowFailureCategory::classify(&error),
            error,
        }),
    }
}

async fn process_single_trajectory_inner(
    gcx: Arc<GlobalContext>,
    path: PathBuf,
    threshold: &DateTime<Utc>,
    extractor: &MemoExtractor,
) -> Result<bool, String> {
    let metadata = fs::metadata(&path).await.map_err(|e| e.to_string())?;
    if metadata.len() > MAX_TRAJECTORY_BYTES {
        return Ok(false);
    }

    let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    let mut trajectory: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };

    if trajectory
        .get("memo_extracted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || trajectory.get("memo_extraction_abandoned").is_some()
    {
        return Ok(false);
    }

    if is_buddy_system_trajectory(&trajectory) {
        mark_trajectory_skipped(&path, &mut trajectory, "skipped: buddy system trajectory").await?;
        return Ok(false);
    }

    let updated_at = trajectory
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let is_abandoned = match updated_at {
        Some(dt) => dt < *threshold,
        None => false,
    };

    if !is_abandoned {
        return Ok(false);
    }

    let messages = trajectory
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or("No messages")?;

    if messages.len() < 10 {
        return Ok(false);
    }

    let trajectory_id = trajectory
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let root_chat_id = trajectory
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| trajectory_id.clone());
    let current_title = trajectory
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled")
        .to_string();

    let is_title_generated = trajectory_title_is_generated(&trajectory);

    let chat_messages = build_chat_messages(messages);
    if chat_messages.is_empty() {
        mark_trajectory_skipped(&path, &mut trajectory, "skipped: invalid chat history").await?;
        return Ok(false);
    }

    let extraction = extractor(MemoExtractionRequest {
        gcx,
        messages: chat_messages,
        current_title: current_title.clone(),
        is_title_generated,
        trajectory_id: trajectory_id.clone(),
        root_chat_id: root_chat_id.clone(),
    })
    .await?;

    persist_extraction_result(
        &path,
        &mut trajectory,
        &extraction,
        &trajectory_id,
        &root_chat_id,
        &current_title,
        is_title_generated,
        Utc::now(),
    )
    .await?;

    Ok(true)
}

fn extract_memos_for_trajectory(request: MemoExtractionRequest) -> MemoExtractionFuture {
    Box::pin(async move {
        let MemoExtractionRequest {
            gcx,
            messages,
            current_title,
            is_title_generated,
            trajectory_id,
            root_chat_id,
        } = request;
        let gcx2 = gcx.clone();
        let title2 = current_title.clone();
        let trajectory_id2 = trajectory_id.clone();
        let root_chat_id2 = root_chat_id.clone();
        crate::buddy::workflows::buddy_wrap_workflow(
            crate::app_state::AppState::from_gcx(gcx).await,
            "memo_extraction",
            "🧠",
            8,
            |r: &ExtractionResult| format!("Memory extracted: {} memos", r.memos.len()),
            move || async move {
                extract_memos_and_meta(
                    gcx2,
                    messages,
                    &title2,
                    is_title_generated,
                    Some(trajectory_id2.as_str()),
                    Some(root_chat_id2.as_str()),
                )
                .await
            },
        )
        .await
    })
}

async fn persist_extraction_result(
    path: &Path,
    trajectory: &mut Value,
    extraction: &ExtractionResult,
    trajectory_id: &str,
    root_chat_id: &str,
    current_title: &str,
    is_title_generated: bool,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let memo_title = extraction
        .meta
        .as_ref()
        .filter(|_| is_title_generated)
        .map(|m| m.title.clone())
        .unwrap_or_else(|| current_title.to_string());
    let mut candidates =
        memory_candidates_from_extraction(extraction, trajectory_id, root_chat_id, &memo_title);
    let existing_docs = load_existing_memo_docs(path).await;
    candidates = dedupe_memo_candidates(candidates, &existing_docs, root_chat_id);
    let ops = memory_ops_from_candidates(&candidates, now);
    if !ops.is_empty() {
        enqueue_trajectory_memory_ops(path, &ops).await?;
    }

    let traj_obj = trajectory.as_object_mut().ok_or("Invalid trajectory")?;
    if let Some(ref meta) = extraction.meta {
        traj_obj.insert("overview".to_string(), Value::String(meta.overview.clone()));
        if is_title_generated && !meta.title.is_empty() {
            traj_obj.insert("title".to_string(), Value::String(meta.title.clone()));
            info!(
                "trajectory_memos: updated title '{}' -> '{}' for {}",
                current_title, meta.title, trajectory_id
            );
        }
    }
    if ops.is_empty() {
        traj_obj.insert(
            "memo_extraction_skip_reason".to_string(),
            Value::String("skipped: no reusable memory candidates".to_string()),
        );
    }
    traj_obj.insert("memo_extracted".to_string(), Value::Bool(true));
    write_trajectory_json(path, trajectory).await
}

async fn write_trajectory_json(path: &Path, trajectory: &Value) -> Result<(), String> {
    write_json_atomically(path, trajectory).await
}

async fn write_json_atomically(path: &Path, value: &Value) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, path).await.map_err(|e| e.to_string())
}

async fn mark_trajectory_skipped(
    path: &Path,
    trajectory: &mut Value,
    reason: &str,
) -> Result<(), String> {
    let traj_obj = trajectory.as_object_mut().ok_or("Invalid trajectory")?;
    traj_obj.insert("memo_extracted".to_string(), Value::Bool(true));
    traj_obj.insert(
        "memo_extraction_skip_reason".to_string(),
        Value::String(reason.to_string()),
    );
    write_trajectory_json(path, trajectory).await
}

fn is_buddy_system_trajectory(trajectory: &Value) -> bool {
    trajectory
        .get("buddy_meta")
        .and_then(|meta| {
            let is_buddy = meta
                .get("is_buddy_chat")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let kind = meta
                .get("buddy_chat_kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            (is_buddy && kind == "system").then_some(())
        })
        .is_some()
}

fn trajectory_title_is_generated(trajectory: &Value) -> bool {
    trajectory
        .get("extra")
        .and_then(|e| e.get("isTitleGenerated"))
        .or_else(|| trajectory.get("isTitleGenerated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn build_chat_messages(messages: &[Value]) -> Vec<ChatMessage> {
    let msgs: Vec<ChatMessage> = messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(|v| v.as_str())?;
            if role != "user" && role != "assistant" {
                return None;
            }

            let content = msg
                .get("content")
                .and_then(extract_text_with_image_placeholders_from_json)?;

            if content.trim().is_empty() {
                return None;
            }

            Some(ChatMessage {
                role: role.to_string(),
                content: ChatContent::SimpleText(content.chars().take(3000).collect()),
                ..Default::default()
            })
        })
        .collect();

    let start = msgs
        .iter()
        .position(|m| m.role == "user")
        .unwrap_or(msgs.len());
    let trimmed = msgs[start..].to_vec();
    match crate::chat::history_limit::validate_chat_history(&trimmed) {
        Ok(valid) => valid,
        Err(e) => {
            warn!("trajectory_memos: skipping invalid chat history: {}", e);
            vec![]
        }
    }
}

struct ExtractedMemo {
    memo_type: String,
    content: String,
}

struct TrajectoryMeta {
    overview: String,
    title: String,
}

struct ExtractionResult {
    meta: Option<TrajectoryMeta>,
    memos: Vec<ExtractedMemo>,
}

async fn extract_memos_and_meta(
    gcx: Arc<GlobalContext>,
    mut messages: Vec<ChatMessage>,
    current_title: &str,
    is_title_generated: bool,
    parent_chat_id: Option<&str>,
    parent_root_chat_id: Option<&str>,
) -> Result<ExtractionResult, String> {
    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let extraction_prompt = subagent_config
        .messages
        .user_template
        .as_ref()
        .ok_or_else(|| {
            format!(
                "messages.user_template not defined for subagent '{}'",
                SUBAGENT_ID
            )
        })?;

    let title_hint = if is_title_generated {
        format!("\n\nNote: The current title \"{}\" was auto-generated. Please provide a better descriptive title.", current_title)
    } else {
        String::new()
    };

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(format!("{}{}", extraction_prompt, title_hint)),
        ..Default::default()
    });

    let result = run_subchat_once(
        gcx,
        SUBAGENT_ID,
        messages,
        TraceParent::from_parts(parent_chat_id, parent_root_chat_id),
    )
    .await
    .map_err(|e| e.to_string())?;

    let response_text = result
        .messages
        .last()
        .and_then(|m| match &m.content {
            ChatContent::SimpleText(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let mut meta: Option<TrajectoryMeta> = None;
    let mut memos: Vec<ExtractedMemo> = Vec::new();

    for line in response_text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }

        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let (Some(overview), Some(title)) = (
            parsed.get("overview").and_then(|v| v.as_str()),
            parsed.get("title").and_then(|v| v.as_str()),
        ) {
            meta = Some(TrajectoryMeta {
                overview: overview.to_string(),
                title: title.to_string(),
            });
            continue;
        }

        if let (Some(memo_type), Some(content)) = (
            parsed.get("type").and_then(|v| v.as_str()),
            parsed.get("content").and_then(|v| v.as_str()),
        ) {
            if memos.len() < 10 {
                memos.push(ExtractedMemo {
                    memo_type: memo_type.to_string(),
                    content: content.to_string(),
                });
            }
        }
    }

    Ok(ExtractionResult { meta, memos })
}

fn memory_candidates_from_extraction(
    extraction: &ExtractionResult,
    trajectory_id: &str,
    root_chat_id: &str,
    memo_title: &str,
) -> Vec<MemoryCandidate> {
    let mut candidates = Vec::new();
    for (idx, memo) in extraction.memos.iter().enumerate() {
        let content = sanitize_candidate_text(
            &format!(
                "{}\n\n---\nSource: trajectory `{}`",
                memo.content, trajectory_id
            ),
            MAX_MEMO_CONTENT_CHARS,
        );
        if content.is_empty() {
            continue;
        }
        let memo_type = normalize_memo_type(&memo.memo_type);
        let mut tags = vec![
            memo_type.clone(),
            "trajectory".to_string(),
            "memory".to_string(),
        ];
        if memo_type == "preference" {
            tags.push("preference".to_string());
        }
        let related_files = extract_file_paths(&content);
        let content_hash = compute_content_hash(&content);
        let source_message_range = trajectory_message_range(idx, extraction.memos.len());
        let source_id = format!(
            "{}:{}:{}:{}",
            trajectory_id,
            source_message_range,
            root_chat_id,
            candidate_hash(&[trajectory_id, &source_message_range, &content_hash])
        );
        candidates.push(
            MemoryCandidate {
                candidate_id: deterministic_candidate_id(trajectory_id, idx, &content_hash),
                source: MemorySource::Trajectory,
                title: format!("[{}] {}", memo_type, memo_title),
                content,
                tags,
                kind: memo_kind(&memo_type).to_string(),
                related_files,
                source_id: Some(source_id),
                source_message_range: Some(source_message_range),
                confidence: memo_confidence(&memo_type),
                status: candidate_status(MemorySource::Trajectory, memo_confidence(&memo_type)),
                content_hash,
                review_after_days: 0,
                ..Default::default()
            }
            .normalized(),
        );
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            seen.insert(compute_idempotency_key(
                &candidate.idempotency_input(MemoryOpType::CreateMemory),
            ))
        })
        .collect()
}

fn memory_ops_from_candidates(
    candidates: &[MemoryCandidate],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    candidates
        .iter()
        .map(|candidate| {
            let evidence = sanitize_candidate_text(
                &format!(
                    "trajectory memo candidate source={} type={} confidence={:.2}",
                    candidate.source_id.as_deref().unwrap_or_default(),
                    candidate.kind,
                    candidate.confidence
                ),
                MAX_MEMO_EVIDENCE_CHARS,
            );
            candidate.clone().into_create_memory_op(
                candidate.candidate_id.clone(),
                evidence,
                now.to_rfc3339(),
            )
        })
        .collect()
}

async fn load_existing_memo_docs(path: &Path) -> Vec<MemoryDocSnapshot> {
    let Some(project_root) = trajectory_project_root(path) else {
        return Vec::new();
    };
    load_memory_doc_snapshots_from_knowledge_dirs(&[project_root.join(KNOWLEDGE_FOLDER_NAME)]).await
}

fn normalized_memo_title(title: &str) -> String {
    let collapsed = title
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let stripped = if let Some(rest) = collapsed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            rest[end + 1..].trim_start()
        } else {
            collapsed.as_str()
        }
    } else {
        collapsed.as_str()
    };
    stripped.to_lowercase()
}

fn source_id_has_root(source_id: &str, root_chat_id: &str) -> bool {
    if root_chat_id.is_empty() {
        return false;
    }
    let Some((prefix, hash)) = source_id.rsplit_once(':') else {
        return false;
    };
    let Some((trajectory_range, root)) = prefix.rsplit_once(':') else {
        return false;
    };
    !hash.is_empty() && trajectory_range.contains(":memo:") && root == root_chat_id
}

fn dedupe_memo_candidates(
    candidates: Vec<MemoryCandidate>,
    existing_docs: &[MemoryDocSnapshot],
    root_chat_id: &str,
) -> Vec<MemoryCandidate> {
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            let title = normalized_memo_title(&candidate.title);
            if !seen.insert((title.clone(), candidate.content_hash.clone())) {
                return false;
            }
            !existing_docs.iter().any(|doc| {
                normalized_memo_title(&doc.title) == title
                    && (doc.content_hash == candidate.content_hash
                        || doc
                            .source_id
                            .as_deref()
                            .map(|source_id| source_id_has_root(source_id, root_chat_id))
                            .unwrap_or(false))
            })
        })
        .collect()
}

async fn enqueue_trajectory_memory_ops(
    path: &Path,
    ops: &[MemoryLifecycleOp],
) -> Result<(), String> {
    let project_root = trajectory_project_root(path).ok_or_else(|| {
        format!(
            "trajectory path is not under a project .refact directory: {}",
            path.display()
        )
    })?;
    for op in ops {
        crate::buddy::storage::enqueue_memory_op(&project_root, op.clone()).await?;
    }
    Ok(())
}

fn trajectory_project_root(path: &Path) -> Option<PathBuf> {
    let mut parent = path.parent();
    while let Some(dir) = parent {
        if dir.file_name().and_then(|name| name.to_str()) == Some("trajectories") {
            let refact_dir = dir.parent()?;
            if refact_dir.file_name().and_then(|name| name.to_str()) == Some(".refact") {
                return refact_dir.parent().map(Path::to_path_buf);
            }
            return None;
        }
        parent = dir.parent();
    }
    None
}

fn sanitize_candidate_text(text: &str, max_chars: usize) -> String {
    let redacted = crate::buddy::actor::redact_sensitive(text);
    let collapsed = redacted
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    crate::llm::safe_truncate(&collapsed, max_chars)
        .trim()
        .to_string()
}

fn normalize_memo_type(memo_type: &str) -> String {
    match memo_type.trim().to_lowercase().replace('-', "_").as_str() {
        "pattern" | "tool_pattern" | "tool_patterns" => "pattern".to_string(),
        "preference" | "preferences" => "preference".to_string(),
        "decision" | "decisions" => "decision".to_string(),
        "bug" | "bugs" | "bug_fix" | "bug_fixed" => "bug".to_string(),
        "lesson" | "lessons" | "learning" => "lesson".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "lesson".to_string(),
    }
}

fn memo_kind(memo_type: &str) -> &'static str {
    match memo_type {
        "preference" => "preference",
        "decision" => "decision",
        "pattern" => "pattern",
        "bug" => "bug_fix",
        _ => "lesson",
    }
}

fn memo_confidence(memo_type: &str) -> f32 {
    match memo_type {
        "preference" => 0.78,
        "decision" => 0.74,
        "pattern" => 0.72,
        "bug" => 0.70,
        _ => 0.68,
    }
}

fn candidate_status(source: MemorySource, confidence: f32) -> MemoryCandidateStatus {
    if !source.is_autonomous() && confidence >= 0.85 {
        MemoryCandidateStatus::Promoted
    } else {
        MemoryCandidateStatus::Proposed
    }
}

fn trajectory_message_range(idx: usize, total: usize) -> String {
    let total = total.max(1);
    format!("memo:{}-{}", idx, total - 1)
}

fn deterministic_candidate_id(trajectory_id: &str, idx: usize, content_hash: &str) -> String {
    format!(
        "memcand_traj_{}",
        candidate_hash(&[trajectory_id, &idx.to_string(), content_hash])
    )
}

fn candidate_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn scripted_extractor(
        outcomes: Vec<Result<(), String>>,
        calls: Arc<AtomicUsize>,
    ) -> Arc<MemoExtractor> {
        let outcomes = Arc::new(std::sync::Mutex::new(VecDeque::from(outcomes)));
        Arc::new(move |_| {
            let outcomes = outcomes.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                match outcomes.lock().unwrap().pop_front().unwrap_or(Ok(())) {
                    Ok(()) => Ok(ExtractionResult {
                        meta: None,
                        memos: vec![],
                    }),
                    Err(error) => Err(error),
                }
            })
        })
    }

    async fn memo_test_workspace(count: usize) -> (tempfile::TempDir, Arc<GlobalContext>) {
        let workspace = tempfile::tempdir().unwrap();
        let trajectories = workspace.path().join(TRAJECTORIES_FOLDER);
        tokio::fs::create_dir_all(&trajectories).await.unwrap();
        for index in 0..count {
            let trajectory = trajectory_json(&format!("trajectory-{index}"), enough_messages());
            tokio::fs::write(
                trajectories.join(format!("trajectory-{index}.json")),
                serde_json::to_string(&trajectory).unwrap(),
            )
            .await
            .unwrap();
        }
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        (workspace, gcx)
    }

    fn recording_extractor(processed: Arc<std::sync::Mutex<Vec<String>>>) -> Arc<MemoExtractor> {
        Arc::new(move |request| {
            let processed = processed.clone();
            Box::pin(async move {
                processed.lock().unwrap().push(request.trajectory_id);
                Ok(ExtractionResult {
                    meta: None,
                    memos: vec![],
                })
            })
        })
    }

    #[tokio::test]
    async fn sweep_processes_at_most_the_batch_cap() {
        let (_workspace, gcx) = memo_test_workspace(100).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(vec![], calls.clone());
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories(gcx.clone(), &mut breaker, extractor.as_ref())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), MAX_TRAJECTORIES_PER_SWEEP);

        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), MAX_TRAJECTORIES_PER_SWEEP * 2);
    }

    #[tokio::test]
    async fn sweep_is_oldest_first() {
        let (workspace, gcx) = memo_test_workspace(3).await;
        let trajectories = workspace.path().join(TRAJECTORIES_FOLDER);
        for (id, updated_at) in [
            ("trajectory-0", "2026-05-03T00:00:00Z"),
            ("trajectory-1", "2026-05-01T00:00:00Z"),
            ("trajectory-2", "2026-05-02T00:00:00Z"),
        ] {
            let mut trajectory = trajectory_json(id, enough_messages());
            trajectory["updated_at"] = Value::String(updated_at.to_string());
            tokio::fs::write(
                trajectories.join(format!("{id}.json")),
                serde_json::to_string(&trajectory).unwrap(),
            )
            .await
            .unwrap();
        }
        let processed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let extractor = recording_extractor(processed.clone());
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();

        assert_eq!(
            *processed.lock().unwrap(),
            vec![
                "trajectory-1".to_string(),
                "trajectory-2".to_string(),
                "trajectory-0".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn file_is_abandoned_after_three_failed_attempts() {
        let (workspace, gcx) = memo_test_workspace(1).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(
            vec![
                Err("model unavailable".to_string()),
                Err("model unavailable".to_string()),
                Err("model unavailable".to_string()),
            ],
            calls.clone(),
        );
        let mut breaker = MemoBreaker::default();

        for _ in 0..MAX_EXTRACTION_ATTEMPTS {
            process_abandoned_trajectories(gcx.clone(), &mut breaker, extractor.as_ref())
                .await
                .unwrap();
        }
        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            MAX_EXTRACTION_ATTEMPTS as usize
        );
        let path = workspace
            .path()
            .join(TRAJECTORIES_FOLDER)
            .join("trajectory-0.json");
        let trajectory: Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(trajectory.get("memo_extracted").is_none());
        assert_eq!(
            trajectory["memo_extraction_abandoned"]["count"].as_u64(),
            Some(MAX_EXTRACTION_ATTEMPTS)
        );
        assert_eq!(
            trajectory["memo_extraction_abandoned"]["last_error_class"].as_str(),
            Some("model_unavailable")
        );
    }

    #[tokio::test]
    async fn oversize_file_is_marked_and_not_rescanned() {
        let (workspace, gcx) = memo_test_workspace(0).await;
        let path = workspace
            .path()
            .join(TRAJECTORIES_FOLDER)
            .join("oversize.json");
        tokio::fs::write(&path, "x".repeat(MAX_TRAJECTORY_BYTES as usize + 1))
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(vec![], calls.clone());
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories(gcx.clone(), &mut breaker, extractor.as_ref())
            .await
            .unwrap();
        let marker_path = memo_state_path(&path);
        let marker: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&marker_path).await.unwrap()).unwrap();
        assert_eq!(
            marker["memo_extraction_skipped"]["reason"].as_str(),
            Some("oversize")
        );

        let before = tokio::fs::read(&marker_path).await.unwrap();
        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(marker_path).await.unwrap(), before);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disabled_setting_performs_no_work() {
        let (_workspace, gcx) = memo_test_workspace(1).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(vec![], calls.clone());
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories_with_enabled(gcx, &mut breaker, extractor.as_ref(), false)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!trajectory_memos_enabled_from_env(Some("1")));
        assert!(trajectory_memos_enabled_from_env(Some("false")));
        assert!(trajectory_memos_enabled_from_env(None));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_mid_sweep() {
        let (_workspace, gcx) = memo_test_workspace(2).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor: Arc<MemoExtractor> = {
            let calls = calls.clone();
            Arc::new(move |_| {
                let calls = calls.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<Result<ExtractionResult, String>>().await
                })
            })
        };
        let shutdown = gcx.shutdown_flag.clone();
        let mut breaker = MemoBreaker::default();
        let task = tokio::spawn(async move {
            process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
                .await
                .unwrap();
        });

        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        shutdown.store(true, Ordering::SeqCst);
        tokio::time::advance(TokioDuration::from_millis(200)).await;
        task.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn breaker_and_cap_compose() {
        let (_workspace, gcx) = memo_test_workspace(MAX_TRAJECTORIES_PER_SWEEP + 5).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(
            vec![
                Err("OpenAI API error (429): rate limit reached".to_string()),
                Err("OpenAI API error (429): rate limit reached".to_string()),
                Err("OpenAI API error (429): rate limit reached".to_string()),
            ],
            calls.clone(),
        );
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), RATE_LIMIT_BREAKER_THRESHOLD);
        assert!(breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn breaker_opens_after_three_consecutive_rate_limits() {
        let (_workspace, gcx) = memo_test_workspace(4).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(
            vec![
                Err("OpenAI API error (429): rate limit reached".to_string()),
                Err("OpenAI API error (429): rate limit reached".to_string()),
                Err("OpenAI API error (429): rate limit reached".to_string()),
                Err("OpenAI API error (429): rate limit reached".to_string()),
            ],
            calls.clone(),
        );
        let mut breaker = MemoBreaker::default();

        process_abandoned_trajectories(gcx, &mut breaker, extractor.as_ref())
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn breaker_makes_no_llm_calls_while_open() {
        let (_workspace, gcx) = memo_test_workspace(1).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(vec![Ok(())], calls.clone());
        let mut breaker = MemoBreaker::default();
        breaker.open_until = Some(Instant::now() + TokioDuration::from_secs(900));
        let shutdown = gcx.shutdown_flag.clone();
        let task = tokio::spawn(trajectory_memos_background_task_inner(
            gcx, breaker, extractor,
        ));

        tokio::task::yield_now().await;
        tokio::time::advance(TokioDuration::from_secs(CHECK_INTERVAL_SECS)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(TokioDuration::from_secs(CHECK_INTERVAL_SECS)).await;
        tokio::task::yield_now().await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        shutdown.store(true, Ordering::SeqCst);
        tokio::time::advance(TokioDuration::from_millis(200)).await;
        task.await.unwrap();
    }

    #[test]
    fn success_resets_the_breaker() {
        let mut breaker = MemoBreaker::default();
        let now = Instant::now();
        let rate_limit = crate::buddy::workflows::WorkflowFailureCategory::ProviderRateLimit;

        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert_eq!(
            breaker.record_failure(&rate_limit, now),
            Some(TokioDuration::from_secs(300))
        );
        breaker.record_success();

        assert_eq!(breaker.consecutive_rate_limited, 0);
        assert_eq!(breaker.escalation, 0);
        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert_eq!(
            breaker.record_failure(&rate_limit, now),
            Some(TokioDuration::from_secs(300))
        );
    }

    #[test]
    fn non_rate_limit_failure_does_not_open_the_breaker() {
        let mut breaker = MemoBreaker::default();
        let now = Instant::now();
        let rate_limit = crate::buddy::workflows::WorkflowFailureCategory::ProviderRateLimit;
        let parse_error = crate::buddy::workflows::WorkflowFailureCategory::InvalidRequest;

        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert!(breaker.record_failure(&rate_limit, now).is_none());
        assert!(breaker.record_failure(&parse_error, now).is_none());

        assert_eq!(breaker.consecutive_rate_limited, 0);
        assert!(!breaker.is_open(now));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_is_prompt_while_the_breaker_is_open() {
        let (_workspace, gcx) = memo_test_workspace(1).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let extractor = scripted_extractor(vec![Ok(())], calls.clone());
        let mut breaker = MemoBreaker::default();
        breaker.open_until = Some(Instant::now() + TokioDuration::from_secs(21_600));
        let shutdown = gcx.shutdown_flag.clone();
        let task = tokio::spawn(trajectory_memos_background_task_inner(
            gcx, breaker, extractor,
        ));

        tokio::task::yield_now().await;
        shutdown.store(true, Ordering::SeqCst);
        tokio::time::advance(TokioDuration::from_millis(200)).await;
        task.await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cooldown_escalates_and_caps() {
        let mut breaker = MemoBreaker::default();
        let rate_limit = crate::buddy::workflows::WorkflowFailureCategory::ProviderRateLimit;
        let now = Instant::now();
        let mut cooldowns = Vec::new();

        for _ in 0..RATE_LIMIT_BREAKER_THRESHOLD {
            if let Some(cooldown) = breaker.record_failure(&rate_limit, now) {
                cooldowns.push(cooldown.as_secs());
            }
        }
        for _ in 0..5 {
            cooldowns.push(breaker.record_failure(&rate_limit, now).unwrap().as_secs());
        }

        assert_eq!(cooldowns, vec![300, 900, 2700, 10_800, 21_600, 21_600]);
    }

    #[test]
    fn provenance_root_matching_is_exact_not_substring() {
        assert!(source_id_has_root(
            "trajectory:memo:0-0:root-a:hash",
            "root-a"
        ));
        assert!(!source_id_has_root(
            "trajectory:memo:0-0:root-ab:hash",
            "root-a"
        ));
        assert!(!source_id_has_root("root-a:memo:0-0:other:hash", "root-a"));
        assert!(!source_id_has_root("malformed-root-a", "root-a"));
        assert!(!source_id_has_root("trajectory:memo:0-0:root-a:hash", ""));
    }

    fn extraction_with_memo(memo_type: &str, content: &str) -> ExtractionResult {
        ExtractionResult {
            meta: Some(TrajectoryMeta {
                overview: "Implemented a fix".to_string(),
                title: "Fix Summary".to_string(),
            }),
            memos: vec![ExtractedMemo {
                memo_type: memo_type.to_string(),
                content: content.to_string(),
            }],
        }
    }

    fn stale_threshold() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn trajectory_json(id: &str, messages: Vec<Value>) -> Value {
        serde_json::json!({
            "id": id,
            "title": "Untitled",
            "model": "test",
            "mode": "agent",
            "root_chat_id": id,
            "updated_at": "2026-05-01T00:00:00Z",
            "messages": messages
        })
    }

    fn enough_messages() -> Vec<Value> {
        (0..10)
            .map(|idx| {
                if idx % 2 == 0 {
                    serde_json::json!({"role": "user", "content": format!("Question {idx}")})
                } else {
                    serde_json::json!({"role": "assistant", "content": format!("Answer {idx}")})
                }
            })
            .collect()
    }

    #[test]
    fn trajectory_extraction_builds_proposed_candidate_ops_with_source_id() {
        let extraction = extraction_with_memo(
            "decision",
            "Use bounded queue persistence for trajectory memories in src/lib.rs.",
        );
        let candidates = memory_candidates_from_extraction(
            &extraction,
            "trajectory-a",
            "root-a",
            "Queue Persistence",
        );
        let ops = memory_ops_from_candidates(&candidates, stale_threshold());

        assert_eq!(ops.len(), 1);
        let op = &ops[0];
        assert_eq!(op.source, MemorySource::Trajectory);
        assert_eq!(op.op_type, MemoryOpType::CreateMemory);
        assert_eq!(
            op.status,
            crate::buddy::memory_lifecycle::MemoryOpStatus::Pending
        );
        assert!(!op.requires_approval);
        assert_eq!(
            op.payload.source_id.as_deref().unwrap().split(':').next(),
            Some("trajectory-a")
        );
        assert_eq!(op.payload.kind.as_deref(), Some("decision"));
        assert_eq!(op.payload.source_message_range.as_deref(), Some("memo:0-0"));
        assert_eq!(op.payload.review_after.as_deref(), Some("2026-06-01"));
        assert!(op
            .payload
            .tags
            .as_ref()
            .unwrap()
            .contains(&"trajectory".to_string()));
        assert!(op
            .payload
            .tags
            .as_ref()
            .unwrap()
            .contains(&"memory".to_string()));
        assert_eq!(
            op.payload.related_files.as_ref().unwrap(),
            &vec!["src/lib.rs".to_string()]
        );
    }

    #[tokio::test]
    async fn trajectory_json_persists_queued_memory_candidate_ops() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let trajectories_dir = root.join(TRAJECTORIES_FOLDER);
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join("trajectory-a.json");
        let mut trajectory = trajectory_json("trajectory-a", enough_messages());
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();

        let extraction =
            extraction_with_memo("lesson", "Use the memory ops queue for distilled lessons.");
        persist_extraction_result(
            &path,
            &mut trajectory,
            &extraction,
            "trajectory-a",
            "root-a",
            "Queue Persistence",
            false,
            stale_threshold(),
        )
        .await
        .unwrap();

        let state = crate::buddy::storage::load_memory_ops(root).await;
        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.ops[0].source, MemorySource::Trajectory);
        assert!(state.ops[0]
            .payload
            .source_id
            .as_deref()
            .unwrap()
            .starts_with("trajectory-a:memo:0-0:root-a:"));
        let updated: Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(updated["memo_extracted"].as_bool(), Some(true));
    }

    #[test]
    fn duplicate_trajectory_rerun_produces_same_idempotency_key() {
        let extraction = extraction_with_memo("pattern", "Use tokio::fs for async file writes.");
        let first = memory_ops_from_candidates(
            &memory_candidates_from_extraction(&extraction, "trajectory-a", "root-a", "Title"),
            stale_threshold(),
        );
        let second = memory_ops_from_candidates(
            &memory_candidates_from_extraction(&extraction, "trajectory-a", "root-a", "Title"),
            stale_threshold(),
        );

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].idempotency_key, second[0].idempotency_key);
        assert_eq!(first[0].op_id, second[0].op_id);
    }

    #[test]
    fn normalized_memo_title_strips_type_prefix_and_whitespace() {
        assert_eq!(
            normalized_memo_title(" [decision]   Queue   Persistence \n Plan "),
            "queue persistence plan"
        );
        assert_eq!(
            normalized_memo_title("Queue Persistence"),
            "queue persistence"
        );
    }

    #[test]
    fn memo_title_dedupe_skips_existing_and_batch_duplicates() {
        let extraction = ExtractionResult {
            meta: None,
            memos: vec![
                ExtractedMemo {
                    memo_type: "decision".to_string(),
                    content: "Use bounded queues.".to_string(),
                },
                ExtractedMemo {
                    memo_type: "lesson".to_string(),
                    content: "Prefer bounded queues.".to_string(),
                },
                ExtractedMemo {
                    memo_type: "pattern".to_string(),
                    content: "Ship idempotent writes.".to_string(),
                },
            ],
        };
        let candidates = memory_candidates_from_extraction(
            &extraction,
            "trajectory-a",
            "root-a",
            "Queue Persistence",
        );
        let existing = vec![MemoryDocSnapshot {
            title: "[decision] Queue Persistence".to_string(),
            content_hash: "different".to_string(),
            source_id: Some("trajectory-old:memo:0-0:root-a:hash".to_string()),
            ..Default::default()
        }];
        let deduped = dedupe_memo_candidates(candidates, &existing, "root-a");

        assert_eq!(deduped.len(), 0);

        let extraction = ExtractionResult {
            meta: None,
            memos: vec![
                ExtractedMemo {
                    memo_type: "decision".to_string(),
                    content: "Use bounded queues.".to_string(),
                },
                ExtractedMemo {
                    memo_type: "lesson".to_string(),
                    content: "Use bounded queues.".to_string(),
                },
                ExtractedMemo {
                    memo_type: "pattern".to_string(),
                    content: "Prefer bounded queues.".to_string(),
                },
            ],
        };
        let candidates = memory_candidates_from_extraction(
            &extraction,
            "trajectory-a",
            "root-a",
            "Queue Persistence",
        );
        let deduped = dedupe_memo_candidates(candidates, &[], "other-root");

        assert_eq!(deduped.len(), 2);
        assert!(deduped
            .iter()
            .all(|candidate| normalized_memo_title(&candidate.title) == "queue persistence"));
    }

    #[tokio::test]
    async fn huge_and_malformed_trajectories_skip_safely() {
        let dir = tempfile::tempdir().unwrap();
        let huge = dir.path().join("huge.json");
        let malformed = dir.path().join("malformed.json");
        tokio::fs::write(
            &huge,
            format!(
                "{{\"pad\":\"{}\"}}",
                "x".repeat(MAX_TRAJECTORY_BYTES as usize + 1)
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(&malformed, "{").await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        assert!(
            !process_single_trajectory(gcx.clone(), huge.clone(), &stale_threshold())
                .await
                .unwrap()
        );
        assert!(
            !process_single_trajectory(gcx, malformed.clone(), &stale_threshold())
                .await
                .unwrap()
        );
        assert!(!tokio::fs::read_to_string(huge)
            .await
            .unwrap()
            .contains("memo_extracted"));
        assert_eq!(tokio::fs::read_to_string(malformed).await.unwrap(), "{");
    }

    #[tokio::test]
    async fn buddy_system_trajectories_are_marked_skipped_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("buddy.json");
        let mut trajectory = trajectory_json("buddy-chat", enough_messages());
        trajectory["buddy_meta"] = serde_json::json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "system",
            "workflow_id": "buddy_security_whisperer"
        });
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        assert!(
            !process_single_trajectory(gcx, path.clone(), &stale_threshold())
                .await
                .unwrap()
        );
        let updated: Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(updated["memo_extracted"].as_bool(), Some(true));
        assert_eq!(
            updated["memo_extraction_skip_reason"].as_str(),
            Some("skipped: buddy system trajectory")
        );
    }

    #[tokio::test]
    async fn memo_extracted_is_not_set_if_queueing_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.json");
        let mut trajectory = trajectory_json("chat", enough_messages());
        trajectory["overview"] = Value::String("overview".to_string());
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();

        let extraction = extraction_with_memo("lesson", "Queue through memory ops.");
        let err = persist_extraction_result(
            &path,
            &mut trajectory,
            &extraction,
            "chat",
            "chat",
            "Chat",
            false,
            stale_threshold(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("trajectory path is not under a project .refact directory"));
        let unchanged: Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(unchanged.get("memo_extracted").is_none());
    }

    #[test]
    fn candidate_content_gets_normalized_tags_kind_review_after_status_proposed() {
        let extraction = extraction_with_memo(
            "preference",
            "User prefers concise answers and not token=secret in logs.",
        );
        let candidates = memory_candidates_from_extraction(&extraction, "chat", "chat", "Prefs");

        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.status, MemoryCandidateStatus::Proposed);
        assert_eq!(candidate.kind, "preference");
        assert_eq!(candidate.review_after_days, 30);
        assert!(candidate.tags.contains(&"memory".to_string()));
        assert!(candidate.tags.contains(&"preference".to_string()));
        assert!(!candidate.content.contains("secret"));
    }
}
