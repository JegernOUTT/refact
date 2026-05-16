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
    pub total_tokens: usize,
    pub total_cost_usd: f64,
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
    pub top_conversations: Vec<TopConversation>,
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
        total_tokens: usize,
        total_cost_usd: f64,
    }
    let mut by_mode_map: HashMap<String, ModeAcc> = HashMap::new();

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
                total_tokens: 0,
                total_cost_usd: 0.0,
            });
        mode_acc.total_calls += 1;
        mode_acc.total_tokens += event.total_tokens;
        mode_acc.total_cost_usd += event.cost_usd.unwrap_or(0.0);

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
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
        })
        .collect();
    by_mode.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| cmp_f64_desc(a.total_cost_usd, b.total_cost_usd))
            .then_with(|| b.total_calls.cmp(&a.total_calls))
            .then_with(|| a.mode.cmp(&b.mode))
    });

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
        },
        by_model,
        by_provider,
        by_day,
        by_mode,
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
}
