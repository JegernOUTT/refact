use std::path::PathBuf;
use serde_json::Value;

use refact_core::chunk_utils::official_text_hashing_function;
use refact_core::memory_plane::{MemoryPlaneFileKind, MemoryPlaneRoots};
use refact_core::vecdb_types::SplitResult;

use crate::vdb_thread::vecdb_trajectory_split_bytes;

const MESSAGES_PER_CHUNK: usize = 4;
const OVERLAP_MESSAGES: usize = 1;
const MAX_PARTS_PER_MESSAGE: usize = 8;
const CHARS_PER_TOKEN: usize = 4;
const LLM_SEGMENT_SUMMARY_KIND: &str = "llm_segment_summary";
// Keep in sync with refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE;
// refact-vecdb intentionally has no dependency on the chat-history crate.
const COMPRESSION_REPORT_ROLE: &str = "compression_report";

pub struct TrajectoryFileSplitter {
    max_tokens: usize,
    split_bytes: usize,
}

#[derive(Debug, Clone)]
struct ExtractedMessage {
    index: usize,
    role: String,
    content: String,
    part: usize,
    total_parts: usize,
}

struct MessageChunk {
    text: String,
    start_msg: usize,
    end_msg: usize,
}

impl TrajectoryFileSplitter {
    pub fn new(max_tokens: usize) -> Self {
        let split_bytes = vecdb_trajectory_split_bytes()
            .min(max_tokens.saturating_mul(CHARS_PER_TOKEN))
            .max(1);
        Self {
            max_tokens,
            split_bytes,
        }
    }

    pub fn split_bytes(&self) -> usize {
        self.split_bytes
    }

    pub async fn split(&self, text: &str, path: &PathBuf) -> Result<Vec<SplitResult>, String> {
        let trajectory: Value = serde_json::from_str(text)
            .map_err(|e| format!("Failed to parse trajectory JSON: {}", e))?;

        let trajectory_id = trajectory
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let title = trajectory
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();
        let messages = trajectory
            .get("messages")
            .and_then(|v| v.as_array())
            .ok_or("No messages array")?;

        let extracted = self.extract_messages(messages);
        if extracted.is_empty() {
            return Ok(vec![]);
        }

        let mut results = Vec::new();

        let indexed_messages = extracted
            .iter()
            .map(|message| message.index)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let metadata_text = format!(
            "Trajectory: {}\nTitle: {}\nMessages: {}",
            trajectory_id, title, indexed_messages
        );
        results.push(SplitResult {
            file_path: path.clone(),
            window_text: metadata_text.clone(),
            window_text_hash: official_text_hashing_function(&metadata_text),
            start_line: 0,
            end_line: 0,
            symbol_path: format!("traj:{}:meta", trajectory_id),
        });

        for chunk in self.chunk_messages(&extracted) {
            results.push(SplitResult {
                file_path: path.clone(),
                window_text: chunk.text.clone(),
                window_text_hash: official_text_hashing_function(&chunk.text),
                start_line: chunk.start_msg as u64,
                end_line: chunk.end_msg as u64,
                symbol_path: format!(
                    "traj:{}:msg:{}-{}",
                    trajectory_id, chunk.start_msg, chunk.end_msg
                ),
            });
        }

        Ok(results)
    }

    fn extract_messages(&self, messages: &[Value]) -> Vec<ExtractedMessage> {
        messages
            .iter()
            .enumerate()
            .filter_map(|(idx, msg)| {
                let role = msg
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                if self.should_skip_message(msg, &role) {
                    return None;
                }
                let content = self.extract_content(msg);
                if content.trim().is_empty() {
                    return None;
                }
                let parts = self.split_message_content(&content);
                let total_parts = parts.len();
                Some(
                    parts
                        .into_iter()
                        .enumerate()
                        .map(move |(part, content)| ExtractedMessage {
                            index: idx,
                            role: role.clone(),
                            content,
                            part,
                            total_parts,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect()
    }

    fn split_message_content(&self, content: &str) -> Vec<String> {
        if content.len() <= self.split_bytes {
            return vec![content.to_string()];
        }
        let mut parts = Vec::new();
        let mut start = 0;
        while start < content.len() && parts.len() < MAX_PARTS_PER_MESSAGE {
            let mut end = start.saturating_add(self.split_bytes).min(content.len());
            while end > start && !content.is_char_boundary(end) {
                end -= 1;
            }
            if end == start {
                break;
            }
            parts.push(content[start..end].to_string());
            start = end;
        }
        if start < content.len() {
            let marker = format!(
                "... (truncated: showing {} of {} bytes; raise vecdb_trajectory_split_bytes in settings)",
                start,
                content.len()
            );
            match parts.last_mut() {
                Some(last) => last.push_str(&marker),
                None => parts.push(marker),
            }
        }
        parts
    }

    fn should_skip_message(&self, msg: &Value, role: &str) -> bool {
        if role == COMPRESSION_REPORT_ROLE
            || matches!(role, "context_file" | "cd_instruction" | "system")
        {
            return true;
        }
        role == "assistant" && message_compression_kind(msg) == Some(LLM_SEGMENT_SUMMARY_KIND)
    }

    fn extract_content(&self, msg: &Value) -> String {
        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            return content.to_string();
        }
        if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
            return content_arr
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| item.get("m_content").and_then(|t| t.as_str()))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            let names: Vec<_> = tool_calls
                .iter()
                .filter_map(|tc| {
                    tc.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                })
                .map(|s| format!("[tool: {}]", s))
                .collect();
            if !names.is_empty() {
                return names.join(" ");
            }
        }
        String::new()
    }

    fn chunk_messages(&self, messages: &[ExtractedMessage]) -> Vec<MessageChunk> {
        if messages.is_empty() {
            return vec![];
        }
        let mut chunks = Vec::new();
        let mut i = 0;
        while i < messages.len() {
            let end_idx = (i + MESSAGES_PER_CHUNK).min(messages.len());
            let chunk_messages = &messages[i..end_idx];
            let text = self.format_chunk(chunk_messages);
            let estimated_tokens = text.len() / CHARS_PER_TOKEN;
            if estimated_tokens > self.max_tokens && chunk_messages.len() > 1 {
                for msg in chunk_messages {
                    chunks.push(MessageChunk {
                        text: self.format_chunk(&[msg.clone()]),
                        start_msg: msg.index,
                        end_msg: msg.index,
                    });
                }
            } else {
                chunks.push(MessageChunk {
                    text,
                    start_msg: chunk_messages.first().map(|m| m.index).unwrap_or(0),
                    end_msg: chunk_messages.last().map(|m| m.index).unwrap_or(0),
                });
            }
            i += MESSAGES_PER_CHUNK.saturating_sub(OVERLAP_MESSAGES).max(1);
        }
        chunks
    }

    fn format_chunk(&self, messages: &[ExtractedMessage]) -> String {
        messages
            .iter()
            .flat_map(|msg| {
                let role = match msg.role.as_str() {
                    "user" => "USER",
                    "assistant" => "ASSISTANT",
                    "tool" => "TOOL_RESULT",
                    "system" => "SYSTEM",
                    _ => &msg.role,
                };
                let header = if msg.total_parts > 1 {
                    format!("[{} part {}/{}]:", role, msg.part + 1, msg.total_parts)
                } else {
                    format!("[{}]:", role)
                };
                vec![header, msg.content.clone(), String::new()]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn message_compression_kind(msg: &Value) -> Option<&str> {
    msg.get("extra")
        .and_then(|extra| extra.get("compression"))
        .and_then(|compression| compression.get("kind"))
        .and_then(|kind| kind.as_str())
        .or_else(|| {
            msg.get("compression")
                .and_then(|compression| compression.get("kind"))
                .and_then(|kind| kind.as_str())
        })
}

pub fn is_trajectory_file(path: &PathBuf, roots: &MemoryPlaneRoots) -> bool {
    roots.classify_file(path) == Some(MemoryPlaneFileKind::TrajectoryJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    static SPLIT_BYTES_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_split_bytes() -> std::sync::MutexGuard<'static, ()> {
        SPLIT_BYTES_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn split_texts(results: &[SplitResult]) -> Vec<String> {
        results
            .iter()
            .map(|result| result.window_text.clone())
            .collect()
    }

    #[tokio::test]
    async fn split_skips_source_preserving_compression_report_and_internal_summary_pair() {
        let trajectory = json!({
            "id": "traj-compression",
            "title": "Compression artifacts",
            "messages": [
                {"role": "user", "content": "keep indexed user goal"},
                {"role": "assistant", "content": "normal assistant remains indexed"},
                {
                    "role": "compression_report",
                    "content": "compression report must not be indexed",
                    "extra": {
                        "compression_report": {
                            "kind": "chat_compression_report",
                            "compression_kind": "llm_segment_summary",
                            "insert_mode": "source_preserving"
                        }
                    }
                },
                {
                    "role": "assistant",
                    "content": "internal llm segment summary must not be indexed",
                    "extra": {
                        "compression": {
                            "kind": "llm_segment_summary",
                            "insert_mode": "source_preserving"
                        }
                    }
                },
                {"role": "assistant", "content": "normal assistant after summary remains indexed"}
            ]
        });
        let splitter = TrajectoryFileSplitter::new(10_000);
        let results = splitter
            .split(
                &trajectory.to_string(),
                &PathBuf::from(".refact/trajectories/traj-compression.json"),
            )
            .await
            .unwrap();
        let text = split_texts(&results).join("\n");

        assert!(text.contains("keep indexed user goal"));
        assert!(text.contains("normal assistant remains indexed"));
        assert!(text.contains("normal assistant after summary remains indexed"));
        assert!(!text.contains("compression report must not be indexed"));
        assert!(!text.contains("internal llm segment summary must not be indexed"));
    }

    #[tokio::test]
    async fn split_skips_legacy_top_level_llm_segment_summary_shape() {
        let trajectory = json!({
            "id": "traj-legacy-compression",
            "title": "Legacy compression artifacts",
            "messages": [
                {
                    "role": "assistant",
                    "content": "legacy top-level summary must not be indexed",
                    "compression": {"kind": "llm_segment_summary"}
                },
                {
                    "role": "assistant",
                    "content": "malformed compression metadata remains normal assistant",
                    "extra": {"compression": "not an object"}
                }
            ]
        });
        let splitter = TrajectoryFileSplitter::new(10_000);
        let results = splitter
            .split(
                &trajectory.to_string(),
                &PathBuf::from(".refact/trajectories/traj-legacy-compression.json"),
            )
            .await
            .unwrap();
        let text = split_texts(&results).join("\n");

        assert!(!text.contains("legacy top-level summary must not be indexed"));
        assert!(text.contains("malformed compression metadata remains normal assistant"));
    }

    #[tokio::test]
    async fn long_message_is_split_into_multiple_indexed_chunks() {
        let _guard = lock_split_bytes();
        let tail = "TAIL_MARKER_UNIQUE";
        let content = format!("{}{}", "a".repeat(40_000), tail);
        let trajectory = json!({
            "id": "traj-long",
            "title": "Long message",
            "messages": [{"role": "user", "content": content}]
        });
        let splitter = TrajectoryFileSplitter::new(4_096);
        let results = splitter
            .split(
                &trajectory.to_string(),
                &PathBuf::from(".refact/trajectories/traj-long.json"),
            )
            .await
            .unwrap();
        let text = split_texts(&results).join("\n");

        assert_eq!(splitter.split_bytes(), 16_384);
        assert!(results.len() > 2);
        assert!(text.contains(tail));
        assert!(text.contains("part 1/3"));
        assert!(text.contains("part 3/3"));
        assert!(!text.contains("truncated: showing"));
    }

    #[tokio::test]
    async fn truncation_marker_reports_shown_and_total_bytes() {
        let _guard = lock_split_bytes();
        let total = super::MAX_PARTS_PER_MESSAGE * 16_384 + 5_000;
        let content = "b".repeat(total);
        let trajectory = json!({
            "id": "traj-huge",
            "title": "Huge message",
            "messages": [{"role": "user", "content": content}]
        });
        let splitter = TrajectoryFileSplitter::new(1_000_000);
        let results = splitter
            .split(
                &trajectory.to_string(),
                &PathBuf::from(".refact/trajectories/traj-huge.json"),
            )
            .await
            .unwrap();
        let text = split_texts(&results).join("\n");

        let shown = super::MAX_PARTS_PER_MESSAGE * splitter.split_bytes();
        assert!(text.contains(&format!(
            "... (truncated: showing {} of {} bytes; raise vecdb_trajectory_split_bytes in settings)",
            shown, total
        )));
    }

    #[test]
    fn install_vecdb_trajectory_split_bytes_changes_effective_split_size() {
        let _guard = lock_split_bytes();
        let original = crate::vdb_thread::vecdb_trajectory_split_bytes();
        assert_eq!(
            original,
            crate::vdb_thread::DEFAULT_VECDB_TRAJECTORY_SPLIT_BYTES
        );
        assert_eq!(
            TrajectoryFileSplitter::new(1_000_000).split_bytes(),
            crate::vdb_thread::DEFAULT_VECDB_TRAJECTORY_SPLIT_BYTES
        );

        crate::vdb_thread::install_vecdb_trajectory_split_bytes(4_096);
        assert_eq!(TrajectoryFileSplitter::new(1_000_000).split_bytes(), 4_096);

        crate::vdb_thread::install_vecdb_trajectory_split_bytes(original);
        assert_eq!(
            TrajectoryFileSplitter::new(1_000_000).split_bytes(),
            original
        );
    }

    #[test]
    fn recognizes_project_task_and_global_trajectories() {
        let roots = MemoryPlaneRoots::new(
            vec![PathBuf::from("/w")],
            Some(PathBuf::from("/home/u/.config/refact/knowledge")),
            Some(PathBuf::from("/home/u/.config/refact/trajectories")),
        );

        assert!(is_trajectory_file(
            &std::path::PathBuf::from("/w/.refact/trajectories/abc.json"),
            &roots
        ));
        assert!(is_trajectory_file(
            &std::path::PathBuf::from("/w/.refact/tasks/T-1/trajectories/planner/abc.json"),
            &roots
        ));
        assert!(is_trajectory_file(
            &std::path::PathBuf::from("/home/u/.config/refact/trajectories/abc.json"),
            &roots
        ));
        assert!(!is_trajectory_file(
            &std::path::PathBuf::from("/w/.refact/buddy/chats/conversations/abc.json"),
            &roots
        ));
        assert!(!is_trajectory_file(
            &std::path::PathBuf::from("/w/.refact/trajectories/abc.md"),
            &roots
        ));
        assert!(!is_trajectory_file(
            &std::path::PathBuf::from("/w/.refact/knowledge/note.md"),
            &roots
        ));
        assert!(!is_trajectory_file(
            &std::path::PathBuf::from("/w/src/tasks/T-1/trajectories/abc.json"),
            &roots
        ));
    }
}
