use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::stats::event::LlmCallEvent;

pub fn read_all_stats_events(stats_dir: &Path) -> Vec<LlmCallEvent> {
    read_stats_events_filtered(stats_dir, None, None)
}

pub fn read_stats_events_filtered(
    stats_dir: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<LlmCallEvent> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(stats_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("jsonl")
            })
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
                Ok(event) => {
                    if let Some(from) = from {
                        if event.ts_start.as_str() < from {
                            continue;
                        }
                    }
                    if let Some(to) = to {
                        if event.ts_start.as_str() > to {
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
    pub calls: usize,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub cost_usd: f64,
    pub avg_duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct StatsByProvider {
    pub provider: String,
    pub calls: usize,
    pub total_tokens: usize,
    pub cost_usd: f64,
}

#[derive(serde::Serialize)]
pub struct StatsByDay {
    pub date: String,
    pub calls: usize,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cost_usd: f64,
}

#[derive(serde::Serialize)]
pub struct StatsByMode {
    pub mode: String,
    pub calls: usize,
    pub total_tokens: usize,
    pub cost_usd: f64,
}

#[derive(serde::Serialize)]
pub struct TopConversation {
    pub chat_id: String,
    pub calls: usize,
    pub total_tokens: usize,
    pub cost_usd: f64,
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

pub fn aggregate_summary(events: &[LlmCallEvent], from: Option<&str>, to: Option<&str>) -> StatsSummary {
    let actual_from = events.iter().map(|e| e.ts_start.as_str()).min().unwrap_or("").to_string();
    let actual_to = events.iter().map(|e| e.ts_start.as_str()).max().unwrap_or("").to_string();

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
        calls: usize,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
        cost_usd: f64,
        duration_ms: u64,
    }
    let mut by_model_map: HashMap<String, ModelAcc> = HashMap::new();

    struct ProviderAcc {
        calls: usize,
        total_tokens: usize,
        cost_usd: f64,
    }
    let mut by_provider_map: HashMap<String, ProviderAcc> = HashMap::new();

    struct DayAcc {
        calls: usize,
        prompt_tokens: usize,
        completion_tokens: usize,
        cost_usd: f64,
    }
    let mut by_day_map: HashMap<String, DayAcc> = HashMap::new();

    struct ModeAcc {
        calls: usize,
        total_tokens: usize,
        cost_usd: f64,
    }
    let mut by_mode_map: HashMap<String, ModeAcc> = HashMap::new();

    struct ConvAcc {
        calls: usize,
        total_tokens: usize,
        cost_usd: f64,
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

        let model_acc = by_model_map.entry(event.model_id.clone()).or_insert_with(|| ModelAcc {
            provider: event.provider.clone(),
            model: event.model.clone(),
            calls: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            duration_ms: 0,
        });
        model_acc.calls += 1;
        model_acc.prompt_tokens += event.prompt_tokens;
        model_acc.completion_tokens += event.completion_tokens;
        model_acc.total_tokens += event.total_tokens;
        model_acc.cost_usd += event.cost_usd.unwrap_or(0.0);
        model_acc.duration_ms += event.duration_ms;

        let provider_acc = by_provider_map.entry(event.provider.clone()).or_insert_with(|| ProviderAcc {
            calls: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        provider_acc.calls += 1;
        provider_acc.total_tokens += event.total_tokens;
        provider_acc.cost_usd += event.cost_usd.unwrap_or(0.0);

        let day = event.ts_start.get(..10).unwrap_or("").to_string();
        let day_acc = by_day_map.entry(day).or_insert_with(|| DayAcc {
            calls: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
        });
        day_acc.calls += 1;
        day_acc.prompt_tokens += event.prompt_tokens;
        day_acc.completion_tokens += event.completion_tokens;
        day_acc.cost_usd += event.cost_usd.unwrap_or(0.0);

        let mode_acc = by_mode_map.entry(event.mode.clone()).or_insert_with(|| ModeAcc {
            calls: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        mode_acc.calls += 1;
        mode_acc.total_tokens += event.total_tokens;
        mode_acc.cost_usd += event.cost_usd.unwrap_or(0.0);

        let conv_acc = conv_map.entry(event.chat_id.clone()).or_insert_with(|| ConvAcc {
            calls: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            model_id: event.model_id.clone(),
        });
        conv_acc.calls += 1;
        conv_acc.total_tokens += event.total_tokens;
        conv_acc.cost_usd += event.cost_usd.unwrap_or(0.0);
        conv_acc.model_id = event.model_id.clone();
    }

    let total_calls = events.len();
    let failed_calls = total_calls - successful_calls;
    let avg_duration_ms = if total_calls > 0 { total_duration_ms / total_calls as u64 } else { 0 };
    let total_conversations = conv_map.len();

    let mut by_model: Vec<StatsByModel> = by_model_map
        .into_iter()
        .map(|(model_id, acc)| StatsByModel {
            model_id,
            provider: acc.provider,
            model: acc.model,
            calls: acc.calls,
            prompt_tokens: acc.prompt_tokens,
            completion_tokens: acc.completion_tokens,
            total_tokens: acc.total_tokens,
            cost_usd: acc.cost_usd,
            avg_duration_ms: if acc.calls > 0 { acc.duration_ms / acc.calls as u64 } else { 0 },
        })
        .collect();
    by_model.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    let mut by_provider: Vec<StatsByProvider> = by_provider_map
        .into_iter()
        .map(|(provider, acc)| StatsByProvider {
            provider,
            calls: acc.calls,
            total_tokens: acc.total_tokens,
            cost_usd: acc.cost_usd,
        })
        .collect();
    by_provider.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    let mut by_day: Vec<StatsByDay> = by_day_map
        .into_iter()
        .map(|(date, acc)| StatsByDay {
            date,
            calls: acc.calls,
            prompt_tokens: acc.prompt_tokens,
            completion_tokens: acc.completion_tokens,
            cost_usd: acc.cost_usd,
        })
        .collect();
    by_day.sort_by(|a, b| a.date.cmp(&b.date));

    let mut by_mode: Vec<StatsByMode> = by_mode_map
        .into_iter()
        .map(|(mode, acc)| StatsByMode {
            mode,
            calls: acc.calls,
            total_tokens: acc.total_tokens,
            cost_usd: acc.cost_usd,
        })
        .collect();
    by_mode.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    let mut top_conversations: Vec<TopConversation> = conv_map
        .into_iter()
        .map(|(chat_id, acc)| TopConversation {
            chat_id,
            calls: acc.calls,
            total_tokens: acc.total_tokens,
            cost_usd: acc.cost_usd,
            model_id: acc.model_id,
        })
        .collect();
    top_conversations.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
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
    use crate::stats::event::LlmCallEvent;
    use std::io::Write;

    fn make_event(i: u64, success: bool) -> LlmCallEvent {
        LlmCallEvent {
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
            error_message: if success { None } else { Some("timeout".to_string()) },
            finish_reason: if success { Some("stop".to_string()) } else { None },
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
        let content = format!("{}\nthis is not json\n{}\n", valid_line, valid_line);
        std::fs::write(&file_path, &content).unwrap();

        let events = read_all_stats_events(dir.path());
        assert_eq!(events.len(), 2, "should parse 2 valid lines, skip 1 invalid");
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
        assert_eq!(summary.by_mode.len(), 1);
        assert_eq!(summary.top_conversations.len(), 3);
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
        let events = read_stats_events_filtered(dir.path(), Some("2026-02-03T"), Some("2026-02-05T"));
        assert_eq!(events.len(), 2, "should include events on day 3 and 4 only");
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
}
