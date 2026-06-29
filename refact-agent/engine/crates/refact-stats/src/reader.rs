use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::event::{LlmCallEvent, canonicalize_mode_for_stats};

const RECENT_STATS_MIN_TAIL_BYTES: u64 = 64 * 1024;
const RECENT_STATS_MAX_TAIL_BYTES: u64 = 2 * 1024 * 1024;
const RECENT_STATS_BYTES_PER_EVENT: u64 = 4 * 1024;

#[allow(dead_code)]
pub fn read_all_stats_events(stats_dir: &Path) -> Vec<LlmCallEvent> {
    read_stats_events_filtered(stats_dir, None, None)
}

pub fn read_stats_events_filtered(
    stats_dir: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<LlmCallEvent> {
    read_stats_events_from_dirs(&[stats_dir.to_path_buf()], from, to)
}

pub fn read_stats_events_from_dirs(
    stats_dirs: &[PathBuf],
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<LlmCallEvent> {
    let mut seen_ids = HashSet::new();
    let mut all_events = Vec::new();
    for stats_dir in stats_dirs {
        let dir_events = read_stats_events_from_single_dir(stats_dir, from, to);
        merge_events(&mut all_events, &mut seen_ids, dir_events);
    }
    sort_events(&mut all_events);
    all_events
}

pub fn read_recent_stats_events_from_dirs(
    stats_dirs: &[PathBuf],
    max_events: usize,
) -> Vec<LlmCallEvent> {
    if max_events == 0 {
        return Vec::new();
    }
    let mut seen_ids = HashSet::new();
    let mut all_events = Vec::new();
    for stats_dir in stats_dirs {
        let dir_events = read_recent_stats_events_from_single_dir(stats_dir, max_events);
        merge_events(&mut all_events, &mut seen_ids, dir_events);
    }
    sort_events(&mut all_events);
    if all_events.len() > max_events {
        all_events.drain(0..all_events.len() - max_events);
    }
    all_events
}

fn merge_events(
    all_events: &mut Vec<LlmCallEvent>,
    seen_ids: &mut HashSet<String>,
    events: Vec<LlmCallEvent>,
) {
    let mut batch_seen_ids = HashSet::new();
    for event in events {
        if event.id.is_empty() {
            all_events.push(event);
            continue;
        }
        if seen_ids.contains(&event.id) || !batch_seen_ids.insert(event.id.clone()) {
            continue;
        }
        all_events.push(event);
    }
    seen_ids.extend(batch_seen_ids);
}

fn sort_events(events: &mut Vec<LlmCallEvent>) {
    events.sort_by(|a, b| {
        a.ts_start
            .cmp(&b.ts_start)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.chat_id.cmp(&b.chat_id))
    });
}

fn read_stats_events_from_single_dir(
    stats_dir: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<LlmCallEvent> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(stats_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .collect(),
        Err(_) => return vec![],
    };
    files.sort();

    let mut events = Vec::new();
    for path in &files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("stats reader: failed to read {:?}: {}", path, e);
                continue;
            }
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<LlmCallEvent>(line) {
                Ok(mut event) => {
                    event.mode = canonicalize_mode_for_stats(&event.mode);
                    if let Some(from) = from {
                        if event.ts_start.get(..10).unwrap_or("") < from.get(..10).unwrap_or("") {
                            continue;
                        }
                    }
                    if let Some(to) = to {
                        if event.ts_start.get(..10).unwrap_or("") > to.get(..10).unwrap_or("") {
                            continue;
                        }
                    }
                    events.push(event);
                }
                Err(e) => {
                    warn!("stats reader: skipping malformed line in {:?}: {}", path, e);
                }
            }
        }
    }
    events
}

fn read_recent_stats_events_from_single_dir(
    stats_dir: &Path,
    max_events: usize,
) -> Vec<LlmCallEvent> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(stats_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .collect(),
        Err(_) => return vec![],
    };
    files.sort();

    let mut seen_ids = HashSet::new();
    let mut events = Vec::new();
    let tail_bytes = recent_tail_window(max_events);
    for path in &files {
        let content = match read_file_tail_to_string(path, tail_bytes) {
            Ok(c) => c,
            Err(e) => {
                warn!("stats reader: failed to read {:?}: {}", path, e);
                continue;
            }
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<LlmCallEvent>(line) {
                Ok(mut event) => {
                    event.mode = canonicalize_mode_for_stats(&event.mode);
                    if !event.id.is_empty() && !seen_ids.insert(event.id.clone()) {
                        continue;
                    }
                    events.push(event);
                }
                Err(e) => {
                    warn!("stats reader: skipping malformed line in {:?}: {}", path, e);
                }
            }
        }
    }
    sort_events(&mut events);
    if events.len() > max_events {
        events.drain(0..events.len() - max_events);
    }
    events
}

fn recent_tail_window(max_events: usize) -> u64 {
    (max_events as u64)
        .saturating_mul(RECENT_STATS_BYTES_PER_EVENT)
        .clamp(RECENT_STATS_MIN_TAIL_BYTES, RECENT_STATS_MAX_TAIL_BYTES)
}

fn read_file_tail_to_string(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len <= max_bytes {
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        return Ok(content);
    }
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start - 1))?;
    let mut previous = [0u8; 1];
    file.read_exact(&mut previous)?;
    let partial_first_line = previous[0] != b'\n';
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut content = String::from_utf8_lossy(&bytes).into_owned();
    if partial_first_line {
        if let Some(newline) = content.find('\n') {
            content.drain(..=newline);
        } else {
            content.clear();
        }
    }
    Ok(content)
}

fn cmp_f64_desc(a: f64, b: f64) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}

#[derive(serde::Serialize)]
pub struct DateRange {
    pub from: String,
    pub to: String,
}

#[derive(serde::Serialize)]
pub struct StatsTotals {
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cache_read_tokens: usize,
    pub total_cache_creation_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub total_conversations: usize,
    pub total_messages_sent: usize,
    pub total_tasks: usize,
    pub total_agents: usize,
    pub retried_calls: usize,
    pub total_retries: usize,
    pub active_days: usize,
}

#[derive(serde::Serialize)]
pub struct StatsByModel {
    pub model_id: String,
    pub provider: String,
    pub model: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cache_read_tokens: usize,
    pub total_cache_creation_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct StatsByProvider {
    pub provider: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cache_read_tokens: usize,
    pub total_cache_creation_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct StatsByDay {
    pub date: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cache_read_tokens: usize,
    pub total_cache_creation_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct StatsByMode {
    pub mode: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cache_read_tokens: usize,
    pub total_cache_creation_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub conversations: usize,
}

#[derive(serde::Serialize)]
pub struct StatsByTaskRole {
    pub role: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: usize,
    pub total_completion_tokens: usize,
    pub total_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub tasks: usize,
    pub agents: usize,
    pub conversations: usize,
}

#[derive(serde::Serialize)]
pub struct StatsByAgent {
    pub agent_id: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_tokens: usize,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub tasks: usize,
    pub last_active: String,
    pub primary_mode: String,
}

#[derive(serde::Serialize)]
pub struct StatsByTask {
    pub task_id: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_tokens: usize,
    pub total_cost_usd: f64,
    pub agents: usize,
    pub cards: usize,
    pub last_active: String,
}

#[derive(serde::Serialize)]
pub struct StatsByHour {
    pub hour: u8,
    pub total_calls: usize,
    pub total_tokens: usize,
}

#[derive(serde::Serialize)]
pub struct StatsCountItem {
    pub key: String,
    pub count: usize,
}

#[derive(serde::Serialize)]
pub struct StatsErrors {
    pub failed_calls: usize,
    pub retried_calls: usize,
    pub total_retries: usize,
    pub by_finish_reason: Vec<StatsCountItem>,
    pub by_category: Vec<StatsCountItem>,
}

#[derive(serde::Serialize)]
pub struct TopConversation {
    pub chat_id: String,
    pub total_calls: usize,
    pub total_tokens: usize,
    pub total_cost_usd: f64,
    pub model_id: String,
}

#[derive(serde::Serialize)]
pub struct StatsSummary {
    pub date_range: DateRange,
    pub totals: StatsTotals,
    pub by_model: Vec<StatsByModel>,
    pub by_provider: Vec<StatsByProvider>,
    pub by_day: Vec<StatsByDay>,
    pub by_mode: Vec<StatsByMode>,
    pub by_task_role: Vec<StatsByTaskRole>,
    pub by_agent: Vec<StatsByAgent>,
    pub by_task: Vec<StatsByTask>,
    pub by_hour: Vec<StatsByHour>,
    pub errors: StatsErrors,
    pub top_conversations: Vec<TopConversation>,
}

pub fn categorize_error(msg: &str) -> &'static str {
    let m = msg.to_lowercase();
    if m.contains("context length")
        || m.contains("context_length")
        || m.contains("maximum context")
        || m.contains("context window")
        || m.contains("too many tokens")
        || m.contains("reduce the length")
    {
        "context_length"
    } else if m.contains("rate limit")
        || m.contains("rate_limit")
        || m.contains("429")
        || m.contains("quota")
        || m.contains("overloaded")
        || m.contains("capacity")
    {
        "rate_limit"
    } else if m.contains("timeout") || m.contains("timed out") || m.contains("deadline") {
        "timeout"
    } else if m.contains("cancel") || m.contains("abort") {
        "cancelled"
    } else if m.contains("401")
        || m.contains("403")
        || m.contains("unauthorized")
        || m.contains("api key")
        || m.contains("permission")
        || m.contains("authentication")
    {
        "auth"
    } else if m.contains("network")
        || m.contains("connection")
        || m.contains("connect")
        || m.contains("dns")
        || m.contains("socket")
        || m.contains("reset by peer")
    {
        "network"
    } else if m.contains("500")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
        || m.contains("server error")
        || m.contains("internal error")
        || m.contains("bad gateway")
    {
        "server"
    } else {
        "other"
    }
}

pub fn aggregate_summary(
    events: &[LlmCallEvent],
    from: Option<&str>,
    to: Option<&str>,
) -> StatsSummary {
    let actual_from = events
        .iter()
        .map(|e| e.ts_start.as_str())
        .min()
        .unwrap_or("")
        .to_string();
    let actual_to = events
        .iter()
        .map(|e| e.ts_start.as_str())
        .max()
        .unwrap_or("")
        .to_string();

    let date_range = DateRange {
        from: from.map(|s| s.to_string()).unwrap_or(actual_from),
        to: to.map(|s| s.to_string()).unwrap_or(actual_to),
    };

    let mut total_prompt_tokens = 0usize;
    let mut total_completion_tokens = 0usize;
    let mut total_tokens = 0usize;
    let mut total_cache_read_tokens = 0usize;
    let mut total_cache_creation_tokens = 0usize;
    let mut total_cost_usd = 0.0f64;
    let mut total_duration_ms = 0u64;
    let mut successful_calls = 0usize;
    let mut total_messages_sent = 0usize;
    let mut retried_calls = 0usize;
    let mut total_retries = 0usize;
    let mut task_ids: HashSet<String> = HashSet::new();
    let mut agent_ids: HashSet<String> = HashSet::new();
    let mut finish_reason_map: HashMap<String, usize> = HashMap::new();
    let mut error_category_map: HashMap<String, usize> = HashMap::new();
    let mut by_hour_arr: [(usize, usize); 24] = [(0, 0); 24];

    struct ModelAcc {
        provider: String,
        model: String,
        total_calls: usize,
        successful_calls: usize,
        total_prompt_tokens: usize,
        total_completion_tokens: usize,
        total_tokens: usize,
        total_cache_read_tokens: usize,
        total_cache_creation_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
    }
    let mut by_model_map: HashMap<String, ModelAcc> = HashMap::new();

    struct ProviderAcc {
        total_calls: usize,
        successful_calls: usize,
        total_prompt_tokens: usize,
        total_completion_tokens: usize,
        total_tokens: usize,
        total_cache_read_tokens: usize,
        total_cache_creation_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
    }
    let mut by_provider_map: HashMap<String, ProviderAcc> = HashMap::new();

    struct DayAcc {
        total_calls: usize,
        successful_calls: usize,
        total_prompt_tokens: usize,
        total_completion_tokens: usize,
        total_tokens: usize,
        total_cache_read_tokens: usize,
        total_cache_creation_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
    }
    let mut by_day_map: HashMap<String, DayAcc> = HashMap::new();

    struct ModeAcc {
        total_calls: usize,
        successful_calls: usize,
        total_prompt_tokens: usize,
        total_completion_tokens: usize,
        total_tokens: usize,
        total_cache_read_tokens: usize,
        total_cache_creation_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
        conversations: HashSet<String>,
    }
    let mut by_mode_map: HashMap<String, ModeAcc> = HashMap::new();

    struct TaskRoleAcc {
        total_calls: usize,
        successful_calls: usize,
        total_prompt_tokens: usize,
        total_completion_tokens: usize,
        total_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
        tasks: HashSet<String>,
        agents: HashSet<String>,
        conversations: HashSet<String>,
    }
    let mut by_task_role_map: HashMap<String, TaskRoleAcc> = HashMap::new();

    struct AgentAcc {
        total_calls: usize,
        successful_calls: usize,
        total_tokens: usize,
        total_cost_usd: f64,
        total_duration_ms: u64,
        tasks: HashSet<String>,
        last_active: String,
        mode_counts: HashMap<String, usize>,
    }
    let mut by_agent_map: HashMap<String, AgentAcc> = HashMap::new();

    struct TaskAcc {
        total_calls: usize,
        successful_calls: usize,
        total_tokens: usize,
        total_cost_usd: f64,
        agents: HashSet<String>,
        cards: HashSet<String>,
        last_active: String,
    }
    let mut by_task_map: HashMap<String, TaskAcc> = HashMap::new();

    struct ConvAcc {
        total_calls: usize,
        total_tokens: usize,
        total_cost_usd: f64,
        model_id: String,
    }
    let mut conv_map: HashMap<String, ConvAcc> = HashMap::new();

    for event in events {
        total_prompt_tokens += event.prompt_tokens;
        total_completion_tokens += event.completion_tokens;
        total_tokens += event.total_tokens;
        total_cache_read_tokens += event.cache_read_tokens.unwrap_or(0);
        total_cache_creation_tokens += event.cache_creation_tokens.unwrap_or(0);
        total_cost_usd += event.cost_usd.unwrap_or(0.0);
        total_duration_ms += event.duration_ms;
        total_messages_sent += event.messages_count;
        if event.success {
            successful_calls += 1;
        }

        let model_acc = by_model_map
            .entry(event.model_id.clone())
            .or_insert_with(|| ModelAcc {
                provider: event.provider.clone(),
                model: event.model.clone(),
                total_calls: 0,
                successful_calls: 0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_cost_usd: 0.0,
                total_duration_ms: 0,
            });
        model_acc.total_calls += 1;
        if event.success {
            model_acc.successful_calls += 1;
        }
        model_acc.total_prompt_tokens += event.prompt_tokens;
        model_acc.total_completion_tokens += event.completion_tokens;
        model_acc.total_tokens += event.total_tokens;
        model_acc.total_cache_read_tokens += event.cache_read_tokens.unwrap_or(0);
        model_acc.total_cache_creation_tokens += event.cache_creation_tokens.unwrap_or(0);
        model_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
        model_acc.total_duration_ms += event.duration_ms;

        let provider_acc = by_provider_map
            .entry(event.provider.clone())
            .or_insert_with(|| ProviderAcc {
                total_calls: 0,
                successful_calls: 0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_cost_usd: 0.0,
                total_duration_ms: 0,
            });
        provider_acc.total_calls += 1;
        if event.success {
            provider_acc.successful_calls += 1;
        }
        provider_acc.total_prompt_tokens += event.prompt_tokens;
        provider_acc.total_completion_tokens += event.completion_tokens;
        provider_acc.total_tokens += event.total_tokens;
        provider_acc.total_cache_read_tokens += event.cache_read_tokens.unwrap_or(0);
        provider_acc.total_cache_creation_tokens += event.cache_creation_tokens.unwrap_or(0);
        provider_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
        provider_acc.total_duration_ms += event.duration_ms;

        let day = event.ts_start.get(..10).unwrap_or("").to_string();
        let day_acc = by_day_map.entry(day).or_insert_with(|| DayAcc {
            total_calls: 0,
            successful_calls: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cost_usd: 0.0,
            total_duration_ms: 0,
        });
        day_acc.total_calls += 1;
        if event.success {
            day_acc.successful_calls += 1;
        }
        day_acc.total_prompt_tokens += event.prompt_tokens;
        day_acc.total_completion_tokens += event.completion_tokens;
        day_acc.total_tokens += event.total_tokens;
        day_acc.total_cache_read_tokens += event.cache_read_tokens.unwrap_or(0);
        day_acc.total_cache_creation_tokens += event.cache_creation_tokens.unwrap_or(0);
        day_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
        day_acc.total_duration_ms += event.duration_ms;

        let mode_acc = by_mode_map
            .entry(canonicalize_mode_for_stats(&event.mode))
            .or_insert_with(|| ModeAcc {
                total_calls: 0,
                successful_calls: 0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_cost_usd: 0.0,
                total_duration_ms: 0,
                conversations: HashSet::new(),
            });
        mode_acc.total_calls += 1;
        if event.success {
            mode_acc.successful_calls += 1;
        }
        mode_acc.total_prompt_tokens += event.prompt_tokens;
        mode_acc.total_completion_tokens += event.completion_tokens;
        mode_acc.total_tokens += event.total_tokens;
        mode_acc.total_cache_read_tokens += event.cache_read_tokens.unwrap_or(0);
        mode_acc.total_cache_creation_tokens += event.cache_creation_tokens.unwrap_or(0);
        mode_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
        mode_acc.total_duration_ms += event.duration_ms;
        mode_acc.conversations.insert(event.chat_id.clone());

        let conv_acc = conv_map
            .entry(event.chat_id.clone())
            .or_insert_with(|| ConvAcc {
                total_calls: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
                model_id: event.model_id.clone(),
            });
        conv_acc.total_calls += 1;
        conv_acc.total_tokens += event.total_tokens;
        conv_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
        conv_acc.model_id = event.model_id.clone();

        if let Some(task_id) = event.task_id.as_ref().filter(|s| !s.is_empty()) {
            task_ids.insert(task_id.clone());
            let task_acc = by_task_map
                .entry(task_id.clone())
                .or_insert_with(|| TaskAcc {
                    total_calls: 0,
                    successful_calls: 0,
                    total_tokens: 0,
                    total_cost_usd: 0.0,
                    agents: HashSet::new(),
                    cards: HashSet::new(),
                    last_active: String::new(),
                });
            task_acc.total_calls += 1;
            if event.success {
                task_acc.successful_calls += 1;
            }
            task_acc.total_tokens += event.total_tokens;
            task_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
            if let Some(agent_id) = event.agent_id.as_ref().filter(|s| !s.is_empty()) {
                task_acc.agents.insert(agent_id.clone());
            }
            if let Some(card_id) = event.card_id.as_ref().filter(|s| !s.is_empty()) {
                task_acc.cards.insert(card_id.clone());
            }
            if event.ts_end > task_acc.last_active {
                task_acc.last_active = event.ts_end.clone();
            }
        }

        if let Some(role) = event.task_role.as_ref().filter(|s| !s.is_empty()) {
            let role_acc = by_task_role_map
                .entry(role.clone())
                .or_insert_with(|| TaskRoleAcc {
                    total_calls: 0,
                    successful_calls: 0,
                    total_prompt_tokens: 0,
                    total_completion_tokens: 0,
                    total_tokens: 0,
                    total_cost_usd: 0.0,
                    total_duration_ms: 0,
                    tasks: HashSet::new(),
                    agents: HashSet::new(),
                    conversations: HashSet::new(),
                });
            role_acc.total_calls += 1;
            if event.success {
                role_acc.successful_calls += 1;
            }
            role_acc.total_prompt_tokens += event.prompt_tokens;
            role_acc.total_completion_tokens += event.completion_tokens;
            role_acc.total_tokens += event.total_tokens;
            role_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
            role_acc.total_duration_ms += event.duration_ms;
            role_acc.conversations.insert(event.chat_id.clone());
            if let Some(task_id) = event.task_id.as_ref().filter(|s| !s.is_empty()) {
                role_acc.tasks.insert(task_id.clone());
            }
            if let Some(agent_id) = event.agent_id.as_ref().filter(|s| !s.is_empty()) {
                role_acc.agents.insert(agent_id.clone());
            }
        }

        if let Some(agent_id) = event.agent_id.as_ref().filter(|s| !s.is_empty()) {
            agent_ids.insert(agent_id.clone());
            let agent_acc = by_agent_map
                .entry(agent_id.clone())
                .or_insert_with(|| AgentAcc {
                    total_calls: 0,
                    successful_calls: 0,
                    total_tokens: 0,
                    total_cost_usd: 0.0,
                    total_duration_ms: 0,
                    tasks: HashSet::new(),
                    last_active: String::new(),
                    mode_counts: HashMap::new(),
                });
            agent_acc.total_calls += 1;
            if event.success {
                agent_acc.successful_calls += 1;
            }
            agent_acc.total_tokens += event.total_tokens;
            agent_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);
            agent_acc.total_duration_ms += event.duration_ms;
            if let Some(task_id) = event.task_id.as_ref().filter(|s| !s.is_empty()) {
                agent_acc.tasks.insert(task_id.clone());
            }
            *agent_acc
                .mode_counts
                .entry(canonicalize_mode_for_stats(&event.mode))
                .or_insert(0) += 1;
            if event.ts_end > agent_acc.last_active {
                agent_acc.last_active = event.ts_end.clone();
            }
        }

        if let Ok(hour) = event.ts_start.get(11..13).unwrap_or("").parse::<usize>() {
            if hour < 24 {
                by_hour_arr[hour].0 += 1;
                by_hour_arr[hour].1 += event.total_tokens;
            }
        }

        if event.attempt_n > 1 {
            retried_calls += 1;
            total_retries += event.attempt_n - 1;
        }

        let finish_key = event
            .finish_reason
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "none".to_string());
        *finish_reason_map.entry(finish_key).or_insert(0) += 1;

        if !event.success {
            let category = match event.error_message.as_deref() {
                Some(msg) if !msg.is_empty() => categorize_error(msg),
                _ => "unknown",
            };
            *error_category_map.entry(category.to_string()).or_insert(0) += 1;
        }
    }

    let total_calls = events.len();
    let failed_calls = total_calls - successful_calls;
    let avg_duration_ms = if total_calls > 0 {
        total_duration_ms / total_calls as u64
    } else {
        0
    };
    let total_conversations = conv_map.len();

    let mut by_model: Vec<StatsByModel> = by_model_map
        .into_iter()
        .map(|(model_id, acc)| StatsByModel {
            model_id,
            provider: acc.provider,
            model: acc.model,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            failed_calls: acc.total_calls - acc.successful_calls,
            total_prompt_tokens: acc.total_prompt_tokens,
            total_completion_tokens: acc.total_completion_tokens,
            total_tokens: acc.total_tokens,
            total_cache_read_tokens: acc.total_cache_read_tokens,
            total_cache_creation_tokens: acc.total_cache_creation_tokens,
            total_cost_usd: acc.total_cost_usd,
            total_duration_ms: acc.total_duration_ms,
            avg_duration_ms: if acc.total_calls > 0 {
                acc.total_duration_ms / acc.total_calls as u64
            } else {
                0
            },
        })
        .collect();
    by_model.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    let mut by_provider: Vec<StatsByProvider> = by_provider_map
        .into_iter()
        .map(|(provider, acc)| StatsByProvider {
            provider,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            failed_calls: acc.total_calls - acc.successful_calls,
            total_prompt_tokens: acc.total_prompt_tokens,
            total_completion_tokens: acc.total_completion_tokens,
            total_tokens: acc.total_tokens,
            total_cache_read_tokens: acc.total_cache_read_tokens,
            total_cache_creation_tokens: acc.total_cache_creation_tokens,
            total_cost_usd: acc.total_cost_usd,
            total_duration_ms: acc.total_duration_ms,
        })
        .collect();
    by_provider.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.provider.cmp(&b.provider))
    });

    let mut by_day: Vec<StatsByDay> = by_day_map
        .into_iter()
        .map(|(date, acc)| StatsByDay {
            date,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            total_prompt_tokens: acc.total_prompt_tokens,
            total_completion_tokens: acc.total_completion_tokens,
            total_tokens: acc.total_tokens,
            total_cache_read_tokens: acc.total_cache_read_tokens,
            total_cache_creation_tokens: acc.total_cache_creation_tokens,
            total_cost_usd: acc.total_cost_usd,
            total_duration_ms: acc.total_duration_ms,
        })
        .collect();
    by_day.sort_by(|a, b| a.date.cmp(&b.date));

    let mut by_mode: Vec<StatsByMode> = by_mode_map
        .into_iter()
        .map(|(mode, acc)| StatsByMode {
            mode,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            failed_calls: acc.total_calls - acc.successful_calls,
            total_prompt_tokens: acc.total_prompt_tokens,
            total_completion_tokens: acc.total_completion_tokens,
            total_tokens: acc.total_tokens,
            total_cache_read_tokens: acc.total_cache_read_tokens,
            total_cache_creation_tokens: acc.total_cache_creation_tokens,
            total_cost_usd: acc.total_cost_usd,
            total_duration_ms: acc.total_duration_ms,
            avg_duration_ms: if acc.total_calls > 0 {
                acc.total_duration_ms / acc.total_calls as u64
            } else {
                0
            },
            conversations: acc.conversations.len(),
        })
        .collect();
    by_mode.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.mode.cmp(&b.mode))
    });

    let mut by_task_role: Vec<StatsByTaskRole> = by_task_role_map
        .into_iter()
        .map(|(role, acc)| StatsByTaskRole {
            role,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            failed_calls: acc.total_calls - acc.successful_calls,
            total_prompt_tokens: acc.total_prompt_tokens,
            total_completion_tokens: acc.total_completion_tokens,
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
            total_duration_ms: acc.total_duration_ms,
            avg_duration_ms: if acc.total_calls > 0 {
                acc.total_duration_ms / acc.total_calls as u64
            } else {
                0
            },
            tasks: acc.tasks.len(),
            agents: acc.agents.len(),
            conversations: acc.conversations.len(),
        })
        .collect();
    by_task_role.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.role.cmp(&b.role))
    });

    let mut by_agent: Vec<StatsByAgent> = by_agent_map
        .into_iter()
        .map(|(agent_id, acc)| {
            let primary_mode = acc
                .mode_counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(mode, _)| mode.clone())
                .unwrap_or_default();
            StatsByAgent {
                agent_id,
                total_calls: acc.total_calls,
                successful_calls: acc.successful_calls,
                failed_calls: acc.total_calls - acc.successful_calls,
                total_tokens: acc.total_tokens,
                total_cost_usd: acc.total_cost_usd,
                total_duration_ms: acc.total_duration_ms,
                tasks: acc.tasks.len(),
                last_active: acc.last_active,
                primary_mode,
            }
        })
        .collect();
    by_agent.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
    by_agent.truncate(20);

    let mut by_task: Vec<StatsByTask> = by_task_map
        .into_iter()
        .map(|(task_id, acc)| StatsByTask {
            task_id,
            total_calls: acc.total_calls,
            successful_calls: acc.successful_calls,
            failed_calls: acc.total_calls - acc.successful_calls,
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
            agents: acc.agents.len(),
            cards: acc.cards.len(),
            last_active: acc.last_active,
        })
        .collect();
    by_task.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    by_task.truncate(20);

    let by_hour: Vec<StatsByHour> = by_hour_arr
        .iter()
        .enumerate()
        .map(|(hour, (calls, tokens))| StatsByHour {
            hour: hour as u8,
            total_calls: *calls,
            total_tokens: *tokens,
        })
        .collect();

    let mut by_finish_reason: Vec<StatsCountItem> = finish_reason_map
        .into_iter()
        .map(|(key, count)| StatsCountItem { key, count })
        .collect();
    by_finish_reason.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));

    let mut by_category: Vec<StatsCountItem> = error_category_map
        .into_iter()
        .map(|(key, count)| StatsCountItem { key, count })
        .collect();
    by_category.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));

    let errors = StatsErrors {
        failed_calls,
        retried_calls,
        total_retries,
        by_finish_reason,
        by_category,
    };

    let total_tasks = task_ids.len();
    let total_agents = agent_ids.len();
    let active_days = by_day.len();

    let mut top_conversations: Vec<TopConversation> = conv_map
        .into_iter()
        .map(|(chat_id, acc)| TopConversation {
            chat_id,
            total_calls: acc.total_calls,
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
            model_id: acc.model_id,
        })
        .collect();
    top_conversations.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.chat_id.cmp(&b.chat_id))
    });
    top_conversations.truncate(10);

    StatsSummary {
        date_range,
        totals: StatsTotals {
            total_calls,
            successful_calls,
            failed_calls,
            total_prompt_tokens,
            total_completion_tokens,
            total_tokens,
            total_cache_read_tokens,
            total_cache_creation_tokens,
            total_cost_usd,
            total_duration_ms,
            avg_duration_ms,
            total_conversations,
            total_messages_sent,
            total_tasks,
            total_agents,
            retried_calls,
            total_retries,
            active_days,
        },
        by_model,
        by_provider,
        by_day,
        by_mode,
        by_task_role,
        by_agent,
        by_task,
        by_hour,
        errors,
        top_conversations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::LlmCallEvent;
    use std::io::Write;

    fn make_event(i: u64, success: bool) -> LlmCallEvent {
        LlmCallEvent {
            id: format!("test-id-{}", i),
            ts_start: format!("2026-02-{:02}T00:00:00Z", i + 1),
            ts_end: format!("2026-02-{:02}T00:00:01Z", i + 1),
            duration_ms: 1000 + i * 100,
            chat_id: format!("chat-{}", i),
            root_chat_id: None,
            mode: "agent".to_string(),
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
            model_id: "anthropic/claude-3".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-3".to_string(),
            messages_count: 3,
            tools_count: 0,
            max_tokens: 4096,
            temperature: Some(0.0),
            success,
            error_message: if success {
                None
            } else {
                Some("timeout".to_string())
            },
            finish_reason: if success {
                Some("stop".to_string())
            } else {
                None
            },
            attempt_n: 1,
            retry_reason: None,
            prompt_tokens: 100,
            completion_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 150,
            cost_usd: Some(0.001),
        }
    }

    #[test]
    fn test_reader_parses_valid_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let event = make_event(1, true);
        let line = serde_json::to_string(&event).unwrap();
        std::fs::write(&file_path, format!("{}\n", line)).unwrap();

        let events = read_all_stats_events(dir.path());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].chat_id, "chat-1");
    }

    #[test]
    fn test_reader_skips_invalid_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let event = make_event(1, true);
        let valid_line = serde_json::to_string(&event).unwrap();
        let mut second = make_event(2, true);
        second.id = "test-id-2".to_string();
        let second_line = serde_json::to_string(&second).unwrap();
        let content = format!("{}\nthis is not json\n{}\n", valid_line, second_line);
        std::fs::write(&file_path, &content).unwrap();

        let events = read_all_stats_events(dir.path());
        assert_eq!(
            events.len(),
            2,
            "should parse 2 valid lines, skip 1 invalid"
        );
    }

    #[test]
    fn test_summary_aggregation() {
        let events = vec![
            make_event(1, true),
            make_event(2, true),
            make_event(3, false),
        ];
        let summary = aggregate_summary(&events, None, None);
        assert_eq!(summary.totals.total_calls, 3);
        assert_eq!(summary.totals.successful_calls, 2);
        assert_eq!(summary.totals.failed_calls, 1);
        assert_eq!(summary.totals.total_prompt_tokens, 300);
        assert_eq!(summary.totals.total_completion_tokens, 150);
        assert_eq!(summary.totals.total_tokens, 450);
        assert_eq!(summary.totals.total_conversations, 3);
        assert_eq!(summary.totals.total_messages_sent, 9);
        assert!((summary.totals.total_cost_usd - 0.003).abs() < 1e-9);
        assert_eq!(summary.by_model.len(), 1);
        assert_eq!(summary.by_model[0].total_calls, 3);
        assert_eq!(summary.by_model[0].successful_calls, 2);
        assert_eq!(summary.by_model[0].failed_calls, 1);
        assert_eq!(summary.by_mode.len(), 1);
        assert_eq!(summary.by_mode[0].total_calls, 3);
        assert_eq!(summary.top_conversations.len(), 3);
        assert_eq!(summary.top_conversations[0].total_calls, 1);
    }

    #[test]
    fn test_filter_by_date_range() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1u64..=5 {
            let event = make_event(i, true);
            let line = serde_json::to_string(&event).unwrap();
            writeln!(file, "{}", line).unwrap();
        }
        let events = read_stats_events_filtered(dir.path(), Some("2026-02-03"), Some("2026-02-05"));
        assert_eq!(
            events.len(),
            3,
            "should include events on days 3, 4, and 5 (inclusive)"
        );
    }

    #[test]
    fn test_date_filter_inclusive_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1u64..=5 {
            let event = make_event(i, true);
            let line = serde_json::to_string(&event).unwrap();
            writeln!(file, "{}", line).unwrap();
        }
        let events = read_stats_events_filtered(dir.path(), Some("2026-02-03"), Some("2026-02-03"));
        assert_eq!(
            events.len(),
            1,
            "should include exactly the event on the boundary date"
        );
        assert_eq!(events[0].chat_id, "chat-2");
    }

    #[test]
    fn test_date_filter_date_only_to() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let mut file = std::fs::File::create(&file_path).unwrap();
        let mut event = make_event(1, true);
        event.ts_start = "2026-02-05T23:59:59Z".to_string();
        let line = serde_json::to_string(&event).unwrap();
        writeln!(file, "{}", line).unwrap();
        let events = read_stats_events_filtered(dir.path(), None, Some("2026-02-05"));
        assert_eq!(
            events.len(),
            1,
            "event at 23:59:59 on to-date should be included"
        );
    }

    #[test]
    fn test_read_stats_events_from_dirs_merges_workspace_and_config_dirs() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        let workspace_file = workspace_dir.path().join("00000001.jsonl");
        let config_file = config_dir.path().join("00000001.jsonl");

        let mut workspace_event = make_event(1, true);
        workspace_event.id = "workspace-event".to_string();
        workspace_event.chat_id = "workspace-chat".to_string();
        workspace_event.ts_start = "2026-02-02T00:00:00Z".to_string();

        let mut config_event = make_event(2, true);
        config_event.id = "config-event".to_string();
        config_event.chat_id = "config-chat".to_string();
        config_event.ts_start = "2026-02-03T00:00:00Z".to_string();

        std::fs::write(
            &workspace_file,
            format!("{}\n", serde_json::to_string(&workspace_event).unwrap()),
        )
        .unwrap();
        std::fs::write(
            &config_file,
            format!("{}\n", serde_json::to_string(&config_event).unwrap()),
        )
        .unwrap();

        let events = read_stats_events_from_dirs(
            &[
                workspace_dir.path().to_path_buf(),
                config_dir.path().to_path_buf(),
            ],
            None,
            None,
        );

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].chat_id, "workspace-chat");
        assert_eq!(events[1].chat_id, "config-chat");
    }

    #[test]
    fn test_read_stats_events_from_dirs_dedupes_duplicate_event_ids() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        let mut first = make_event(1, true);
        first.id = "duplicate-id".to_string();
        first.chat_id = "workspace-chat".to_string();
        first.ts_start = "2026-02-02T00:00:00Z".to_string();

        let mut duplicate = first.clone();
        duplicate.chat_id = "config-chat".to_string();

        let mut unique = make_event(2, true);
        unique.id = "unique-id".to_string();
        unique.chat_id = "unique-chat".to_string();
        unique.ts_start = "2026-02-03T00:00:00Z".to_string();

        std::fs::write(
            workspace_dir.path().join("00000001.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&first).unwrap(),
                serde_json::to_string(&unique).unwrap()
            ),
        )
        .unwrap();
        std::fs::write(
            config_dir.path().join("00000001.jsonl"),
            format!("{}\n", serde_json::to_string(&duplicate).unwrap()),
        )
        .unwrap();

        let events = read_stats_events_from_dirs(
            &[
                workspace_dir.path().to_path_buf(),
                config_dir.path().to_path_buf(),
            ],
            None,
            None,
        );

        assert_eq!(events.len(), 2);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.id == "duplicate-id")
                .count(),
            1
        );
        assert!(events.iter().any(|event| event.chat_id == "workspace-chat"));
        assert!(events.iter().any(|event| event.chat_id == "unique-chat"));
    }

    #[test]
    fn test_read_stats_events_dedupes_duplicate_event_ids_within_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");

        let mut first = make_event(1, true);
        first.id = "duplicate-id".to_string();
        first.chat_id = "first-chat".to_string();
        first.ts_start = "2026-02-02T00:00:00Z".to_string();

        let mut duplicate = first.clone();
        duplicate.chat_id = "duplicate-chat".to_string();
        duplicate.ts_start = "2026-02-03T00:00:00Z".to_string();

        std::fs::write(
            &file_path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&first).unwrap(),
                serde_json::to_string(&duplicate).unwrap()
            ),
        )
        .unwrap();

        let events = read_all_stats_events(dir.path());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].chat_id, "first-chat");
    }

    #[test]
    fn test_read_recent_stats_events_from_dirs_is_bounded_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 1u64..=5 {
            let event = make_event(i, true);
            let line = serde_json::to_string(&event).unwrap();
            writeln!(file, "{}", line).unwrap();
        }
        let mut duplicate = make_event(5, true);
        duplicate.chat_id = "duplicate-chat".to_string();
        writeln!(file, "{}", serde_json::to_string(&duplicate).unwrap()).unwrap();

        let events = read_recent_stats_events_from_dirs(&[dir.path().to_path_buf()], 3);

        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|event| event.ts_start.as_str() >= "2026-02-04T00:00:00Z"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.id == "test-id-5")
                .count(),
            1
        );
    }

    #[test]
    fn test_read_recent_stats_events_tail_reads_large_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("00000001.jsonl");
        let mut file = std::fs::File::create(&file_path).unwrap();

        let mut old = make_event(1, true);
        old.id = "old-id".to_string();
        old.chat_id = "old-chat".to_string();
        writeln!(file, "{}", serde_json::to_string(&old).unwrap()).unwrap();
        writeln!(
            file,
            "{}",
            "x".repeat((RECENT_STATS_MIN_TAIL_BYTES as usize) + 1024)
        )
        .unwrap();
        for i in 10u64..=11 {
            let mut event = make_event(i, true);
            event.id = format!("tail-id-{i}");
            event.chat_id = format!("tail-chat-{i}");
            writeln!(file, "{}", serde_json::to_string(&event).unwrap()).unwrap();
        }

        let events = read_recent_stats_events_from_dirs(&[dir.path().to_path_buf()], 2);

        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.id.starts_with("tail-id-")));
        assert!(!events.iter().any(|event| event.id == "old-id"));
    }

    #[test]
    fn test_read_recent_stats_events_dedupes_across_dirs() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        let mut first = make_event(1, true);
        first.id = "duplicate-id".to_string();
        first.chat_id = "workspace-chat".to_string();

        let mut duplicate = first.clone();
        duplicate.chat_id = "config-chat".to_string();

        std::fs::write(
            workspace_dir.path().join("00000001.jsonl"),
            format!("{}\n", serde_json::to_string(&first).unwrap()),
        )
        .unwrap();
        std::fs::write(
            config_dir.path().join("00000001.jsonl"),
            format!("{}\n", serde_json::to_string(&duplicate).unwrap()),
        )
        .unwrap();

        let events = read_recent_stats_events_from_dirs(
            &[
                workspace_dir.path().to_path_buf(),
                config_dir.path().to_path_buf(),
            ],
            10,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].chat_id, "workspace-chat");
    }

    #[test]
    fn test_empty_stats_dir() {
        let dir = tempfile::tempdir().unwrap();
        let events = read_all_stats_events(dir.path());
        assert!(events.is_empty());
        let summary = aggregate_summary(&events, None, None);
        assert_eq!(summary.totals.total_calls, 0);
        assert!(summary.by_model.is_empty());
    }

    #[test]
    fn test_by_day_total_tokens_uses_total_tokens_field() {
        let mut e = make_event(1, true);
        e.prompt_tokens = 100;
        e.completion_tokens = 50;
        e.total_tokens = 200;
        let events = vec![e];
        let summary = aggregate_summary(&events, None, None);
        assert_eq!(
            summary.by_day[0].total_tokens, 200,
            "by_day.total_tokens should use event.total_tokens, not prompt+completion"
        );
    }

    #[test]
    fn test_summary_cache_tokens_by_day_and_provider() {
        let mut e1 = make_event(1, true);
        e1.cache_read_tokens = Some(200);
        e1.cache_creation_tokens = Some(100);

        let mut e2 = make_event(1, true);
        e2.id = "test-id-1b".to_string();
        e2.chat_id = "chat-1b".to_string();
        e2.cache_read_tokens = Some(50);
        e2.cache_creation_tokens = None;

        let mut e3 = make_event(2, true);
        e3.provider = "openai".to_string();
        e3.model_id = "openai/gpt-4".to_string();
        e3.model = "gpt-4".to_string();
        e3.cache_read_tokens = None;
        e3.cache_creation_tokens = Some(300);

        let events = vec![e1, e2, e3];
        let summary = aggregate_summary(&events, None, None);

        let anthropic = summary
            .by_provider
            .iter()
            .find(|p| p.provider == "anthropic")
            .unwrap();
        assert_eq!(anthropic.total_cache_read_tokens, 250);
        assert_eq!(anthropic.total_cache_creation_tokens, 100);

        let openai = summary
            .by_provider
            .iter()
            .find(|p| p.provider == "openai")
            .unwrap();
        assert_eq!(openai.total_cache_read_tokens, 0);
        assert_eq!(openai.total_cache_creation_tokens, 300);

        let day1 = summary
            .by_day
            .iter()
            .find(|d| d.date == "2026-02-02")
            .unwrap();
        assert_eq!(day1.total_cache_read_tokens, 250);
        assert_eq!(day1.total_cache_creation_tokens, 100);

        let day2 = summary
            .by_day
            .iter()
            .find(|d| d.date == "2026-02-03")
            .unwrap();
        assert_eq!(day2.total_cache_read_tokens, 0);
        assert_eq!(day2.total_cache_creation_tokens, 300);
    }

    #[test]
    fn test_summary_normalizes_legacy_mode_names() {
        let mut uppercase = make_event(1, true);
        uppercase.mode = "TASK_AGENT".to_string();
        let mut lowercase = make_event(2, true);
        lowercase.mode = "task_agent".to_string();

        let summary = aggregate_summary(&[uppercase, lowercase], None, None);

        assert_eq!(summary.by_mode.len(), 1);
        assert_eq!(summary.by_mode[0].mode, "task_agent");
        assert_eq!(summary.by_mode[0].total_calls, 2);
    }

    #[test]
    fn test_summary_canonicalizes_no_tools_and_explore_modes() {
        let mut no_tools = make_event(1, true);
        no_tools.mode = "NO_TOOLS".to_string();
        let mut explore = make_event(2, true);
        explore.mode = "explore".to_string();

        let summary = aggregate_summary(&[no_tools, explore], None, None);

        assert_eq!(summary.by_mode.len(), 1);
        assert_eq!(summary.by_mode[0].mode, "explore");
        assert_eq!(summary.by_mode[0].total_calls, 2);
    }

    #[test]
    fn test_summary_sorting_has_stable_tie_breakers() {
        let mut z = make_event(1, true);
        z.model_id = "z/model".to_string();
        z.provider = "z".to_string();
        z.model = "model".to_string();
        z.chat_id = "z-chat".to_string();
        z.mode = "z_mode".to_string();

        let mut a = make_event(2, true);
        a.model_id = "a/model".to_string();
        a.provider = "a".to_string();
        a.model = "model".to_string();
        a.chat_id = "a-chat".to_string();
        a.mode = "a_mode".to_string();

        let summary = aggregate_summary(&[z, a], None, None);

        assert_eq!(summary.by_model[0].model_id, "a/model");
        assert_eq!(summary.by_provider[0].provider, "a");
        assert_eq!(summary.by_mode[0].mode, "a_mode");
        assert_eq!(summary.top_conversations[0].chat_id, "a-chat");
    }

    #[test]
    fn test_by_mode_enriched_fields() {
        let mut e1 = make_event(1, true);
        e1.mode = "agent".to_string();
        e1.prompt_tokens = 100;
        e1.completion_tokens = 40;
        e1.cache_read_tokens = Some(20);
        let mut e2 = make_event(2, false);
        e2.mode = "agent".to_string();
        e2.chat_id = "chat-1".to_string();
        let summary = aggregate_summary(&[e1, e2], None, None);
        let agent = summary.by_mode.iter().find(|m| m.mode == "agent").unwrap();
        assert_eq!(agent.total_calls, 2);
        assert_eq!(agent.successful_calls, 1);
        assert_eq!(agent.failed_calls, 1);
        assert_eq!(agent.conversations, 1, "both events share chat-1");
        assert!(agent.total_prompt_tokens >= 100);
        assert!(agent.avg_duration_ms > 0);
    }

    #[test]
    fn test_task_role_agent_task_aggregation() {
        let mut planner = make_event(1, true);
        planner.task_id = Some("T-1".to_string());
        planner.task_role = Some("planner".to_string());
        planner.agent_id = Some("agent-a".to_string());
        planner.card_id = Some("card-1".to_string());

        let mut agent = make_event(2, true);
        agent.task_id = Some("T-1".to_string());
        agent.task_role = Some("agent".to_string());
        agent.agent_id = Some("agent-b".to_string());
        agent.card_id = Some("card-2".to_string());

        let mut agent2 = make_event(3, false);
        agent2.task_id = Some("T-1".to_string());
        agent2.task_role = Some("agent".to_string());
        agent2.agent_id = Some("agent-b".to_string());
        agent2.card_id = Some("card-2".to_string());

        let summary = aggregate_summary(&[planner, agent, agent2], None, None);

        assert_eq!(summary.totals.total_tasks, 1);
        assert_eq!(summary.totals.total_agents, 2);

        let role_agent = summary
            .by_task_role
            .iter()
            .find(|r| r.role == "agent")
            .unwrap();
        assert_eq!(role_agent.total_calls, 2);
        assert_eq!(role_agent.successful_calls, 1);
        assert_eq!(role_agent.failed_calls, 1);
        assert_eq!(role_agent.agents, 1);
        assert_eq!(role_agent.tasks, 1);

        let task = summary.by_task.iter().find(|t| t.task_id == "T-1").unwrap();
        assert_eq!(task.total_calls, 3);
        assert_eq!(task.agents, 2);
        assert_eq!(task.cards, 2);

        let agent_b = summary
            .by_agent
            .iter()
            .find(|a| a.agent_id == "agent-b")
            .unwrap();
        assert_eq!(agent_b.total_calls, 2);
        assert_eq!(agent_b.tasks, 1);
        assert_eq!(agent_b.primary_mode, "agent");
    }

    #[test]
    fn test_errors_and_retries_aggregation() {
        let mut ok = make_event(1, true);
        ok.finish_reason = Some("stop".to_string());

        let mut timeout = make_event(2, false);
        timeout.finish_reason = None;
        timeout.error_message = Some("request timed out".to_string());
        timeout.attempt_n = 2;

        let mut ctx = make_event(3, false);
        ctx.error_message = Some("maximum context length exceeded".to_string());
        ctx.attempt_n = 3;

        let summary = aggregate_summary(&[ok, timeout, ctx], None, None);

        assert_eq!(summary.errors.failed_calls, 2);
        assert_eq!(summary.errors.retried_calls, 2);
        assert_eq!(summary.errors.total_retries, 3, "(2-1)+(3-1)=3");
        assert_eq!(summary.totals.retried_calls, 2);
        assert_eq!(summary.totals.total_retries, 3);

        let timeout_cat = summary
            .errors
            .by_category
            .iter()
            .find(|c| c.key == "timeout")
            .unwrap();
        assert_eq!(timeout_cat.count, 1);
        let ctx_cat = summary
            .errors
            .by_category
            .iter()
            .find(|c| c.key == "context_length")
            .unwrap();
        assert_eq!(ctx_cat.count, 1);

        let stop = summary
            .errors
            .by_finish_reason
            .iter()
            .find(|r| r.key == "stop")
            .unwrap();
        assert_eq!(stop.count, 1);
        let none = summary
            .errors
            .by_finish_reason
            .iter()
            .find(|r| r.key == "none")
            .unwrap();
        assert_eq!(none.count, 2, "both failed events have no finish_reason");
    }

    #[test]
    fn test_by_hour_has_24_buckets_and_counts() {
        let mut e1 = make_event(1, true);
        e1.ts_start = "2026-02-01T09:15:00Z".to_string();
        e1.total_tokens = 100;
        let mut e2 = make_event(2, true);
        e2.ts_start = "2026-02-02T09:45:00Z".to_string();
        e2.total_tokens = 50;
        let mut e3 = make_event(3, true);
        e3.ts_start = "2026-02-03T23:00:00Z".to_string();

        let summary = aggregate_summary(&[e1, e2, e3], None, None);
        assert_eq!(summary.by_hour.len(), 24);
        assert_eq!(summary.by_hour[9].total_calls, 2);
        assert_eq!(summary.by_hour[9].total_tokens, 150);
        assert_eq!(summary.by_hour[23].total_calls, 1);
        assert_eq!(summary.totals.active_days, 3);
    }

    #[test]
    fn test_categorize_error_buckets() {
        assert_eq!(categorize_error("Request timed out after 60s"), "timeout");
        assert_eq!(categorize_error("429 Too Many Requests"), "rate_limit");
        assert_eq!(
            categorize_error("maximum context length is 200000 tokens"),
            "context_length"
        );
        assert_eq!(categorize_error("401 Unauthorized: invalid api key"), "auth");
        assert_eq!(categorize_error("connection reset by peer"), "network");
        assert_eq!(categorize_error("502 Bad Gateway"), "server");
        assert_eq!(categorize_error("something weird happened"), "other");
    }
}
