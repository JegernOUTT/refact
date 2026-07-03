use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration, Local, Timelike, Utc};
use regex::Regex;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::user_action::UserAction;

const USER_ACTIVITY_CAPACITY: usize = 200;
const TEXT_CAP: usize = 80;
const REDACTION_SCAN_MULTIPLIER: usize = 4;
const REDACTION_SCAN_EXTRA_CHARS: usize = 4_096;
const TRUNCATED_MARKER: &str = "\n...[truncated]";

#[derive(Debug)]
pub struct UserActivityRing {
    buf: VecDeque<UserAction>,
    capacity: usize,
    project_root: PathBuf,
    persisted_len: AtomicUsize,
}

impl UserActivityRing {
    pub fn new(project_root: PathBuf, capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity,
            project_root,
            persisted_len: AtomicUsize::new(0),
        }
    }

    pub async fn load(project_root: &Path) -> Self {
        let mut ring = Self::new(project_root.to_path_buf(), USER_ACTIVITY_CAPACITY);
        let path = activity_path(project_root);
        let Ok(content) = fs::read_to_string(path).await else {
            return ring;
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(action) = serde_json::from_str::<UserAction>(trimmed) {
                ring.push(action);
            }
        }
        ring.persisted_len.store(ring.buf.len(), Ordering::SeqCst);
        ring
    }

    pub fn push(&mut self, action: UserAction) {
        if self.capacity == 0 {
            self.buf.clear();
            self.persisted_len.store(0, Ordering::SeqCst);
            return;
        }
        self.buf.push_back(redact_action(action));
        let mut evicted = 0usize;
        while self.buf.len() > self.capacity {
            self.buf.pop_front();
            evicted += 1;
        }
        if evicted > 0 {
            let persisted = self.persisted_len.load(Ordering::SeqCst);
            self.persisted_len
                .store(persisted.saturating_sub(evicted), Ordering::SeqCst);
        }
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<UserAction> {
        self.buf.iter().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn last_n(&self, n: usize) -> Vec<UserAction> {
        self.buf
            .iter()
            .skip(self.buf.len().saturating_sub(n))
            .cloned()
            .collect()
    }

    pub fn last_hours(&self, hours: u32) -> Vec<UserAction> {
        let cutoff = Utc::now() - Duration::hours(hours as i64);
        self.buf
            .iter()
            .filter(|action| action.ts() >= cutoff)
            .cloned()
            .collect()
    }

    pub async fn persist(&self) -> Result<(), String> {
        let persisted = self
            .persisted_len
            .load(Ordering::SeqCst)
            .min(self.buf.len());
        let actions = self.buf.iter().skip(persisted).collect::<Vec<_>>();
        if actions.is_empty() {
            return Ok(());
        }
        let path = activity_path(&self.project_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut content = String::new();
        for action in actions {
            let line = serde_json::to_string(action).map_err(|e| e.to_string())?;
            content.push_str(&line);
            content.push('\n');
        }
        file.write_all(content.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        file.flush().await.map_err(|e| e.to_string())?;
        self.persisted_len.store(self.buf.len(), Ordering::SeqCst);
        Ok(())
    }
}

pub fn time_of_day_pattern(actions: &[UserAction]) -> String {
    if actions.is_empty() {
        return "no recent activity".to_string();
    }
    let mut hours = [0usize; 24];
    for action in actions {
        let local = action.ts().with_timezone(&Local);
        hours[local.hour() as usize] += 1;
    }
    let mut windows = (0usize..24)
        .map(|start| {
            let count = hours[start] + hours[(start + 1) % 24] + hours[(start + 2) % 24];
            (start, count)
        })
        .filter(|(_, count)| *count > 0)
        .collect::<Vec<_>>();
    windows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut selected = Vec::new();
    for (start, _) in windows {
        if selected
            .iter()
            .all(|existing| !windows_overlap(*existing, start))
        {
            selected.push(start);
        }
        if selected.len() == 2 {
            break;
        }
    }
    selected.sort_unstable();
    match selected.as_slice() {
        [] => "no recent activity".to_string(),
        [start] => format!("mostly active {}", format_window(*start)),
        [first, second] => format!(
            "mostly active {} and {}",
            format_window(*first),
            format_window(*second)
        ),
        _ => unreachable!(),
    }
}

fn activity_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/user_activity.jsonl")
}

fn redact_action(action: UserAction) -> UserAction {
    match action {
        UserAction::FileOpened { path, ts } => UserAction::FileOpened {
            path: redact_text(&path),
            ts,
        },
        UserAction::SnippetSelected { path, lines, ts } => UserAction::SnippetSelected {
            path: redact_text(&path),
            lines,
            ts,
        },
        UserAction::ToolApproved {
            tool_name,
            chat_id,
            ts,
        } => UserAction::ToolApproved {
            tool_name: redact_text(&tool_name),
            chat_id: redact_text(&chat_id),
            ts,
        },
        UserAction::ToolRejected {
            tool_name,
            chat_id,
            ts,
        } => UserAction::ToolRejected {
            tool_name: redact_text(&tool_name),
            chat_id: redact_text(&chat_id),
            ts,
        },
        UserAction::CommandRun {
            command_preview,
            chat_id,
            ts,
        } => UserAction::CommandRun {
            command_preview: redact_text(&command_preview),
            chat_id: redact_text(&chat_id),
            ts,
        },
        UserAction::WorkspaceChanged {
            folders_added,
            folders_removed,
            ts,
        } => UserAction::WorkspaceChanged {
            folders_added: folders_added.into_iter().map(|x| redact_text(&x)).collect(),
            folders_removed: folders_removed
                .into_iter()
                .map(|x| redact_text(&x))
                .collect(),
            ts,
        },
        UserAction::CommitMade {
            sha,
            message_first_line,
            files,
            ts,
        } => UserAction::CommitMade {
            sha: redact_text(&sha),
            message_first_line: redact_text(&message_first_line),
            files,
            ts,
        },
        UserAction::TaskFailed {
            task_id,
            reason_short,
            ts,
        } => UserAction::TaskFailed {
            task_id: redact_text(&task_id),
            reason_short: redact_text(&reason_short),
            ts,
        },
        UserAction::ChatStarted {
            chat_id,
            first_user_text_preview,
            ts,
        } => UserAction::ChatStarted {
            chat_id: redact_text(&chat_id),
            first_user_text_preview: redact_text(&first_user_text_preview),
            ts,
        },
    }
}

fn redact_text(text: &str) -> String {
    let redacted = redact_and_cap_text(text, TEXT_CAP);
    aws_key_regex()
        .replace_all(&redacted, "[REDACTED_AWS_KEY]")
        .into_owned()
}

fn redact_and_cap_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let scan_cap = redaction_scan_cap(max_chars);
    let (scan_text, truncated) =
        refact_core::string_utils::bounded_redaction_window(text, scan_cap);
    let mut redacted = refact_core::string_utils::redact_sensitive(scan_text);
    if truncated {
        redacted.push_str(TRUNCATED_MARKER);
    }
    cap_text(&redacted, max_chars)
}

fn redaction_scan_cap(max_chars: usize) -> usize {
    max_chars
        .saturating_mul(REDACTION_SCAN_MULTIPLIER)
        .max(max_chars.saturating_add(REDACTION_SCAN_EXTRA_CHARS))
}

fn cap_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let marker_chars = TRUNCATED_MARKER.chars().count();
    if max_chars <= marker_chars {
        return TRUNCATED_MARKER.chars().take(max_chars).collect();
    }
    let keep = max_chars - marker_chars;
    let prefix: String = text.chars().take(keep).collect();
    format!("{}{}", prefix.trim_end(), TRUNCATED_MARKER)
}

fn aws_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b").unwrap())
}

fn windows_overlap(a: usize, b: usize) -> bool {
    let a_hours = [a % 24, (a + 1) % 24, (a + 2) % 24];
    let b_hours = [b % 24, (b + 1) % 24, (b + 2) % 24];
    a_hours.iter().any(|hour| b_hours.contains(hour))
}

fn format_window(start: usize) -> String {
    format!("{:02}–{:02}", start, start + 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone};

    #[test]
    fn ring_holds_200_evicts_oldest() {
        let mut ring = UserActivityRing::new(PathBuf::from("/tmp/project"), 200);
        for idx in 0..201 {
            ring.push(UserAction::ChatStarted {
                chat_id: format!("chat-{idx}"),
                first_user_text_preview: "hello".to_string(),
                ts: Utc::now(),
            });
        }
        let actions = ring.snapshot();
        assert_eq!(actions.len(), 200);
        match actions.first().unwrap() {
            UserAction::ChatStarted { chat_id, .. } => assert_eq!(chat_id, "chat-1"),
            _ => panic!("unexpected action"),
        }
    }

    #[tokio::test]
    async fn ring_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut ring = UserActivityRing::new(dir.path().to_path_buf(), 200);
        ring.push(UserAction::FileOpened {
            path: "src/main.rs".to_string(),
            ts: Utc::now(),
        });
        ring.push(UserAction::CommandRun {
            command_preview: "cargo check".to_string(),
            chat_id: "chat-1".to_string(),
            ts: Utc::now(),
        });
        ring.persist().await.unwrap();
        ring.persist().await.unwrap();

        let reloaded = UserActivityRing::load(dir.path()).await;
        let actions = reloaded.snapshot();
        assert_eq!(actions.len(), 2);
        assert_eq!(
            tokio::fs::read_to_string(activity_path(dir.path()))
                .await
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn time_of_day_pattern_buckets_correctly() {
        let actions = [9, 10, 11, 19, 20, 21]
            .into_iter()
            .map(|hour| UserAction::FileOpened {
                path: format!("file-{hour}"),
                ts: local_ts(hour),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            time_of_day_pattern(&actions),
            "mostly active 09–12 and 19–22"
        );
    }

    #[test]
    fn redacts_secrets_in_paths() {
        let mut ring = UserActivityRing::new(PathBuf::from("/tmp/project"), 200);
        ring.push(UserAction::FileOpened {
            path: "src/AKIA1234567890ABCDEF/config.rs".to_string(),
            ts: Utc::now(),
        });
        let actions = ring.snapshot();
        match &actions[0] {
            UserAction::FileOpened { path, .. } => {
                assert!(!path.contains("AKIA1234567890ABCDEF"));
                assert!(path.contains("[REDACTED_AWS_KEY]"));
            }
            _ => panic!("unexpected action"),
        }
    }

    #[test]
    fn last_hours_returns_correct_window() {
        let mut ring = UserActivityRing::new(PathBuf::from("/tmp/project"), 200);
        ring.push(UserAction::FileOpened {
            path: "old.rs".to_string(),
            ts: Utc::now() - Duration::hours(25),
        });
        ring.push(UserAction::FileOpened {
            path: "new.rs".to_string(),
            ts: Utc::now() - Duration::hours(2),
        });
        let actions = ring.last_hours(24);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            UserAction::FileOpened { path, .. } => assert_eq!(path, "new.rs"),
            _ => panic!("unexpected action"),
        }
    }

    fn local_ts(hour: u32) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(2024, 1, 2, hour, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }
}
