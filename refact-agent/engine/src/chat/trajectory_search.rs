use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use refact_chat_api::chat_local_types::{TrajectorySourceIdentity, SEGMENT_SUMMARY_KIND};
use refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE;
use serde_json::Value;

const MESSAGES_PER_WINDOW: usize = 4;
const WINDOW_STEP: usize = 3;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const TITLE_BOOST: f64 = 1.5;
const SNIPPET_CHARS: usize = 300;
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "about", "what", "where", "when",
    "why", "how", "can", "could", "should", "would", "please", "find", "search", "look", "need",
    "want", "have", "does", "doesn", "work", "was", "were", "are", "is", "it", "to", "of", "in",
    "on", "an", "or", "be", "by", "as", "at", "do",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimits {
    pub max_trajectories: usize,
    pub max_chars_per_trajectory: usize,
    pub max_file_bytes: u64,
    pub max_reads_per_query: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_trajectories: 1_000,
            max_chars_per_trajectory: 12_000,
            max_file_bytes: 32 * 1024 * 1024,
            max_reads_per_query: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrajectoryHit {
    pub trajectory_id: String,
    pub title: String,
    pub message_start: usize,
    pub message_end: usize,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone)]
struct IndexedMessage {
    index: usize,
    role: String,
    text: String,
}

#[derive(Debug, Clone)]
struct Window {
    start: usize,
    end: usize,
    messages: std::ops::Range<usize>,
    term_freqs: HashMap<String, u32>,
    len: u32,
}

#[derive(Debug, Clone)]
struct IndexedDoc {
    id: String,
    title: String,
    title_terms: HashSet<String>,
    messages: Vec<IndexedMessage>,
    windows: Vec<Window>,
    indexed_chars: usize,
}

#[derive(Debug, Clone)]
struct Entry {
    modified: Option<SystemTime>,
    size: u64,
    doc: Option<IndexedDoc>,
}

#[derive(Debug, Default)]
pub struct TrajectorySearchIndex {
    entries: HashMap<PathBuf, Entry>,
}

fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|token| token.to_lowercase())
        .filter(|token| token.chars().count() >= 2 && !STOP_WORDS.contains(&token.as_str()))
}

pub fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokens(query)
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

fn message_text(msg: &Value) -> String {
    if let Some(content) = msg.get("content").and_then(Value::as_str) {
        return content.to_string();
    }
    if let Some(parts) = msg.get("content").and_then(Value::as_array) {
        return parts
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("m_content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
        return tool_calls
            .iter()
            .filter_map(|tc| {
                tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
            })
            .map(|name| format!("[tool: {}]", name))
            .collect::<Vec<_>>()
            .join(" ");
    }
    String::new()
}

fn is_segment_summary(msg: &Value) -> bool {
    let kind = |v: Option<&Value>| {
        v.and_then(|c| c.get("kind"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    kind(msg.get("extra").and_then(|e| e.get("compression")))
        .or_else(|| kind(msg.get("compression")))
        .as_deref()
        == Some(SEGMENT_SUMMARY_KIND)
}

fn skip_message(msg: &Value, role: &str) -> bool {
    msg.as_object()
        .is_some_and(|extra| refact_core::chat_types::is_never_model_visible_parts(role, extra))
        || matches!(
            role,
            "system" | "context_file" | "cd_instruction" | COMPRESSION_REPORT_ROLE
        )
        || (role == "assistant" && is_segment_summary(msg))
}

fn truncate_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

fn parse_doc(path: &Path, bytes: &[u8], limits: &SearchLimits) -> Option<IndexedDoc> {
    let stem = path.file_stem()?.to_str()?;
    super::trajectories::validate_trajectory_id(stem).ok()?;
    let json: Value = serde_json::from_slice(bytes).ok()?;
    if json.get("id").and_then(Value::as_str) != Some(stem) {
        return None;
    }
    match TrajectorySourceIdentity::from_json(&json) {
        Ok(TrajectorySourceIdentity::Buddy) | Err(_) => return None,
        Ok(_) => {}
    }
    if super::trajectories::is_internal_trace_link_type(
        json.get("link_type").and_then(Value::as_str),
    ) {
        return None;
    }
    let raw_messages = json.get("messages").and_then(Value::as_array)?;
    let title = json
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Untitled")
        .to_string();
    let mut messages = Vec::new();
    let mut indexed_chars = 0usize;
    for (index, msg) in raw_messages.iter().enumerate() {
        let remaining = limits
            .max_chars_per_trajectory
            .saturating_sub(indexed_chars);
        if remaining == 0 {
            break;
        }
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if skip_message(msg, &role) {
            continue;
        }
        let text = message_text(msg);
        let text = truncate_chars(text.trim(), remaining);
        if text.is_empty() {
            continue;
        }
        indexed_chars += text.chars().count();
        messages.push(IndexedMessage {
            index,
            role,
            text: text.to_string(),
        });
    }
    let mut windows = Vec::new();
    let mut first = 0;
    while first < messages.len() {
        let last = (first + MESSAGES_PER_WINDOW).min(messages.len());
        let mut term_freqs: HashMap<String, u32> = HashMap::new();
        let mut len = 0u32;
        for message in &messages[first..last] {
            for token in tokens(&message.text) {
                *term_freqs.entry(token).or_default() += 1;
                len += 1;
            }
        }
        windows.push(Window {
            start: messages[first].index,
            end: messages[last - 1].index,
            messages: first..last,
            term_freqs,
            len,
        });
        if last == messages.len() {
            break;
        }
        first += WINDOW_STEP;
    }
    Some(IndexedDoc {
        id: stem.to_string(),
        title_terms: tokens(&title).collect(),
        title,
        messages,
        windows,
        indexed_chars,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshStats {
    pub read: usize,
    pub pending: usize,
    pub over_cap: usize,
}

fn is_candidate_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    !name.starts_with('.') && name != "index.json" && name.ends_with(".json")
}

fn list_candidates(dirs: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
    let mut found = Vec::new();
    let mut push_files = |dir: &Path, nested: &mut Vec<PathBuf>| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                nested.push(path);
            } else if meta.is_file() && is_candidate_file(&path) {
                found.push((path, meta.modified().ok(), meta.len()));
            }
        }
    };
    for dir in dirs {
        let mut nested = Vec::new();
        push_files(dir, &mut nested);
        for folder in nested {
            push_files(&folder, &mut Vec::new());
        }
    }
    found
}

impl TrajectorySearchIndex {
    pub fn refresh(&mut self, dirs: &[PathBuf], limits: &SearchLimits) -> RefreshStats {
        let mut candidates = list_candidates(dirs);
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut stats = RefreshStats {
            over_cap: candidates.len().saturating_sub(limits.max_trajectories),
            ..Default::default()
        };
        candidates.truncate(limits.max_trajectories);
        let keep: HashSet<&PathBuf> = candidates.iter().map(|(path, _, _)| path).collect();
        self.entries.retain(|path, _| keep.contains(path));
        for (path, modified, size) in &candidates {
            let fresh = self
                .entries
                .get(path)
                .is_some_and(|entry| entry.modified == *modified && entry.size == *size);
            if fresh {
                continue;
            }
            if stats.read >= limits.max_reads_per_query {
                stats.pending += 1;
                continue;
            }
            stats.read += 1;
            let doc = if *size > limits.max_file_bytes {
                None
            } else {
                std::fs::read(path)
                    .ok()
                    .and_then(|bytes| parse_doc(path, &bytes, limits))
            };
            self.entries.insert(
                path.clone(),
                Entry {
                    modified: *modified,
                    size: *size,
                    doc,
                },
            );
        }
        stats
    }

    pub fn indexed_chars(&self) -> usize {
        self.entries
            .values()
            .filter_map(|entry| entry.doc.as_ref())
            .map(|doc| doc.indexed_chars)
            .sum()
    }

    pub fn indexed_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.doc.is_some())
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort();
        paths
    }

    pub fn search(&self, terms: &[String], top_n: usize) -> Vec<TrajectoryHit> {
        let docs: Vec<&IndexedDoc> = self
            .entries
            .values()
            .filter_map(|entry| entry.doc.as_ref())
            .collect();
        let total_windows: usize = docs.iter().map(|doc| doc.windows.len()).sum();
        if total_windows == 0 || terms.is_empty() || top_n == 0 {
            return Vec::new();
        }
        let total_len: u64 = docs
            .iter()
            .flat_map(|doc| doc.windows.iter())
            .map(|window| window.len as u64)
            .sum();
        let avg_len = (total_len as f64 / total_windows as f64).max(1.0);
        let n = total_windows as f64;
        let idf: Vec<f64> = terms
            .iter()
            .map(|term| {
                let df = docs
                    .iter()
                    .flat_map(|doc| doc.windows.iter())
                    .filter(|window| window.term_freqs.contains_key(term))
                    .count() as f64;
                (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
            })
            .collect();
        let mut best: HashMap<&str, TrajectoryHit> = HashMap::new();
        for doc in docs {
            let title_score: f64 = terms
                .iter()
                .zip(&idf)
                .filter(|(term, _)| doc.title_terms.contains(*term))
                .map(|(_, idf)| TITLE_BOOST * idf)
                .sum();
            let mut doc_best: Option<(f64, &Window)> = None;
            for window in &doc.windows {
                let norm = BM25_K1 * (1.0 - BM25_B + BM25_B * window.len as f64 / avg_len);
                let body: f64 = terms
                    .iter()
                    .zip(&idf)
                    .filter_map(|(term, idf)| {
                        let tf = *window.term_freqs.get(term)? as f64;
                        Some(idf * tf * (BM25_K1 + 1.0) / (tf + norm))
                    })
                    .sum();
                if doc_best.is_none_or(|(score, _)| body > score) {
                    doc_best = Some((body, window));
                }
            }
            let Some((body, window)) = doc_best else {
                continue;
            };
            let score = body + title_score;
            if score <= 0.0 {
                continue;
            }
            let hit = TrajectoryHit {
                trajectory_id: doc.id.clone(),
                title: doc.title.clone(),
                message_start: window.start,
                message_end: window.end,
                score,
                snippet: snippet(doc, window, terms),
            };
            match best.get(doc.id.as_str()) {
                Some(existing) if existing.score >= score => {}
                _ => {
                    best.insert(doc.id.as_str(), hit);
                }
            }
        }
        let mut hits: Vec<TrajectoryHit> = best.into_values().collect();
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.trajectory_id.cmp(&b.trajectory_id))
        });
        hits.truncate(top_n);
        hits
    }
}

fn snippet(doc: &IndexedDoc, window: &Window, terms: &[String]) -> String {
    let messages = &doc.messages[window.messages.clone()];
    let matching: Vec<&IndexedMessage> = messages
        .iter()
        .filter(|message| tokens(&message.text).any(|token| terms.contains(&token)))
        .collect();
    let shown = if matching.is_empty() {
        messages.iter().take(1).collect()
    } else {
        matching
    };
    shown
        .into_iter()
        .map(|message| {
            let lower = message.text.to_lowercase();
            let first_hit = terms
                .iter()
                .filter_map(|term| lower.find(term.as_str()))
                .min()
                .unwrap_or(0);
            let chars_before = lower[..first_hit].chars().count();
            let skip = chars_before.saturating_sub(SNIPPET_CHARS / 4);
            let body: String = message
                .text
                .chars()
                .skip(skip)
                .take(SNIPPET_CHARS)
                .collect();
            let prefix = if skip > 0 { "…" } else { "" };
            let suffix = if message.text.chars().count() > skip + SNIPPET_CHARS {
                "…"
            } else {
                ""
            };
            format!(
                "[msg {}] {}: {}{}{}",
                message.index,
                message.role,
                prefix,
                body.replace('\n', " "),
                suffix
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn global_index() -> &'static Mutex<TrajectorySearchIndex> {
    static INDEX: OnceLock<Mutex<TrajectorySearchIndex>> = OnceLock::new();
    INDEX.get_or_init(|| Mutex::new(TrajectorySearchIndex::default()))
}

pub async fn search_saved_trajectories(
    dirs: Vec<PathBuf>,
    terms: Vec<String>,
    top_n: usize,
    limits: SearchLimits,
) -> Result<(Vec<TrajectoryHit>, RefreshStats), String> {
    tokio::task::spawn_blocking(move || {
        let mut index = global_index()
            .lock()
            .map_err(|_| "trajectory search index is poisoned".to_string())?;
        let stats = index.refresh(&dirs, &limits);
        Ok((index.search(&terms, top_n), stats))
    })
    .await
    .map_err(|error| format!("trajectory search task failed: {error}"))?
}

#[cfg(test)]
#[path = "trajectory_search_tests.rs"]
mod tests;
