// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::collections::VecDeque;
use std::path::Path;

use crate::vendored::markdown_stream::MarkdownStreamCollector;

use super::chunking::{AdaptiveChunkingPolicy, DrainPlan};
use super::commit_tick::drain_with_policy;
use super::table_holdback::{TableHoldbackScanner, TableHoldbackState};

pub trait CommitDrain {
    fn run_commit_tick(&mut self) -> Option<String>;
}

#[derive(Debug, Clone)]
pub struct StreamController {
    collector: MarkdownStreamCollector,
    committed: String,
    buffered: String,
    queue: VecDeque<String>,
    held: String,
    table_scanner: TableHoldbackScanner,
    policy: AdaptiveChunkingPolicy,
}

impl StreamController {
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        Self {
            collector: MarkdownStreamCollector::new(width, cwd),
            committed: String::new(),
            buffered: String::new(),
            queue: VecDeque::new(),
            held: String::new(),
            table_scanner: TableHoldbackScanner::default(),
            policy: AdaptiveChunkingPolicy::default(),
        }
    }

    pub fn clear(&mut self) {
        self.collector.clear();
        self.committed.clear();
        self.buffered.clear();
        self.queue.clear();
        self.held.clear();
        self.table_scanner.reset();
        self.policy.reset();
    }

    pub fn replace_committed(&mut self, content: &str) {
        self.clear();
        self.committed.push_str(content);
        self.table_scanner.replace_prefix(content);
        self.assert_holdback_boundary();
    }

    pub fn push_delta(&mut self, delta: &str) {
        self.collector.push_delta(delta);
        if let Some(source) = self.collector.commit_complete_source() {
            self.ingest_complete_source(&source);
        }
    }

    pub fn committed(&self) -> &str {
        &self.committed
    }

    pub fn live(&self) -> String {
        let mut live = self.queue.iter().map(String::as_str).collect::<String>();
        live.push_str(&self.held);
        live.push_str(self.collector.pending_source());
        live
    }

    pub fn visible(&self) -> String {
        let mut visible = self.committed.clone();
        visible.push_str(&self.live());
        visible
    }

    pub fn queued_lines(&self) -> usize {
        self.queue.len()
    }

    pub fn stable_lines_ready(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn finalize(&mut self) -> String {
        let remainder = self.collector.finalize_and_drain_source();
        if !remainder.is_empty() {
            self.buffered.push_str(&remainder);
        }
        self.queue.clear();
        self.held.clear();
        let buffered = std::mem::take(&mut self.buffered);
        self.committed.push_str(&buffered);
        let out = self.committed.clone();
        self.clear();
        out
    }

    fn ingest_complete_source(&mut self, source: &str) {
        self.buffered.push_str(source);
        self.table_scanner.push_source_chunk(source);
        let stable_len = self.stable_buffer_len();
        self.rebuild_queue(stable_len);
    }

    fn stable_buffer_len(&self) -> usize {
        self.assert_holdback_boundary();
        let stable_len = match self.table_scanner.state() {
            TableHoldbackState::None => self.buffered.len(),
            TableHoldbackState::PendingHeader { header_start }
            | TableHoldbackState::Confirmed {
                table_start: header_start,
            } => header_start.saturating_sub(self.committed.len()),
        }
        .min(self.buffered.len());
        previous_char_boundary(&self.buffered, stable_len)
    }

    fn rebuild_queue(&mut self, stable_len: usize) {
        self.queue.clear();
        self.held.clear();
        let stable_len =
            previous_char_boundary(&self.buffered, stable_len.min(self.buffered.len()));
        debug_assert!(self.buffered.is_char_boundary(stable_len));
        let (stable, held) = self.buffered.split_at(stable_len);
        enqueue_stable_source(&mut self.queue, stable);
        self.held.push_str(held);
    }

    fn assert_holdback_boundary(&self) {
        debug_assert_eq!(
            self.table_scanner.source_offset(),
            self.committed.len().saturating_add(self.buffered.len()),
            "table scanner offset must match committed + buffered bytes"
        );
        let boundary = match self.table_scanner.state() {
            TableHoldbackState::None => return,
            TableHoldbackState::PendingHeader { header_start } => header_start,
            TableHoldbackState::Confirmed { table_start } => table_start,
        };
        debug_assert!(
            self.absolute_boundary_is_valid(boundary),
            "invalid table holdback boundary {boundary} for committed {} buffered {}",
            self.committed.len(),
            self.buffered.len()
        );
    }

    fn absolute_boundary_is_valid(&self, boundary: usize) -> bool {
        if boundary <= self.committed.len() {
            return self.committed.is_char_boundary(boundary);
        }
        let relative = boundary - self.committed.len();
        relative <= self.buffered.len() && self.buffered.is_char_boundary(relative)
    }

    fn drain(&mut self, plan: DrainPlan) -> Option<String> {
        let count = match plan {
            DrainPlan::Single => 1,
            DrainPlan::Batch(count) => count,
        };
        if count == 0 || self.queue.is_empty() {
            return None;
        }
        let mut drained = String::new();
        for _ in 0..count {
            let Some(line) = self.queue.pop_front() else {
                break;
            };
            self.committed.push_str(&line);
            drained.push_str(&line);
        }
        if !drained.is_empty() {
            let drain_len = drained.len().min(self.buffered.len());
            let drain_len = previous_char_boundary(&self.buffered, drain_len);
            self.buffered.drain(..drain_len);
            self.assert_holdback_boundary();
        }
        (!drained.is_empty()).then_some(drained)
    }
}

fn previous_char_boundary(source: &str, index: usize) -> usize {
    let mut index = index.min(source.len());
    while !source.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

fn split_at_char_boundary(source: &str, index: usize) -> (&str, &str) {
    source.split_at(previous_char_boundary(source, index))
}

fn enqueue_stable_source(queue: &mut VecDeque<String>, source: &str) {
    if source.is_empty() {
        return;
    }
    if let Some((table_start, table_end)) = first_table_range(source) {
        let table_start = previous_char_boundary(source, table_start);
        let table_end = previous_char_boundary(source, table_end);
        let (before, table_and_after) = split_at_char_boundary(source, table_start);
        let (table, after) =
            split_at_char_boundary(table_and_after, table_end.saturating_sub(table_start));
        for line in before.split_inclusive('\n') {
            if !line.is_empty() {
                queue.push_back(line.to_string());
            }
        }
        if !table.is_empty() {
            queue.push_back(table.to_string());
        }
        enqueue_stable_source(queue, after);
    } else {
        for line in source.split_inclusive('\n') {
            if !line.is_empty() {
                queue.push_back(line.to_string());
            }
        }
    }
}

fn first_table_range(source: &str) -> Option<(usize, usize)> {
    let mut previous: Option<(usize, &str)> = None;
    let mut offset = 0usize;
    let mut table_start = None;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(start) = table_start {
            offset = offset.saturating_add(line.len());
            if trimmed.is_empty() {
                return Some((start, offset));
            }
            continue;
        }
        if let Some((previous_start, previous_line)) = previous {
            if is_table_header(previous_line) && is_table_delimiter(trimmed) {
                table_start = Some(previous_start);
            }
        }
        previous = (!trimmed.is_empty()).then_some((offset, trimmed));
        offset = offset.saturating_add(line.len());
    }
    table_start.map(|start| (start, source.len()))
}

fn is_table_header(line: &str) -> bool {
    table_segments(line).is_some_and(|segments| {
        segments.len() >= 2 && segments.iter().any(|segment| !segment.trim().is_empty())
    })
}

fn is_table_delimiter(line: &str) -> bool {
    table_segments(line).is_some_and(|segments| {
        segments.len() >= 2
            && segments.iter().all(|segment| {
                let segment = segment.trim();
                !segment.is_empty()
                    && segment
                        .chars()
                        .all(|ch| ch == '-' || ch == ':' || ch.is_ascii_whitespace())
                    && segment.chars().filter(|ch| *ch == '-').count() >= 3
            })
    })
}

fn table_segments(line: &str) -> Option<Vec<&str>> {
    let mut line = line.trim();
    if !line.contains('|') {
        return None;
    }
    if let Some(rest) = line.strip_prefix('|') {
        line = rest;
    }
    if let Some(rest) = line.strip_suffix('|') {
        line = rest;
    }
    let segments = line.split('|').collect::<Vec<_>>();
    (segments.len() >= 2).then_some(segments)
}

impl CommitDrain for StreamController {
    fn run_commit_tick(&mut self) -> Option<String> {
        let queued_lines = self.queued_lines();
        let mut policy = std::mem::take(&mut self.policy);
        let drained = drain_with_policy(&mut policy, queued_lines, |plan| self.drain(plan));
        self.policy = policy;
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_markdown;

    fn controller() -> StreamController {
        StreamController::new(None, std::path::Path::new("."))
    }

    #[test]
    fn commit_tick_drains_one_complete_line() {
        let mut stream = controller();
        stream.push_delta("one\ntwo\n");
        assert_eq!(stream.committed(), "");
        assert_eq!(stream.queued_lines(), 2);
        assert_eq!(stream.run_commit_tick(), Some("one\n".to_string()));
        assert_eq!(stream.committed(), "one\n");
        assert_eq!(stream.live(), "two\n");
    }

    #[test]
    fn table_rows_hold_until_finalize() {
        let mut stream = controller();
        stream.push_delta("| A | B |\n");
        assert_eq!(stream.run_commit_tick(), None);
        assert_eq!(stream.visible(), "| A | B |\n");
        stream.push_delta("| --- | --- |\n");
        stream.push_delta("| one | two |\n");
        assert_eq!(stream.run_commit_tick(), None);
        assert_eq!(stream.committed(), "");
        assert!(stream.live().contains("| one | two |"));
        assert_eq!(
            stream.finalize(),
            "| A | B |\n| --- | --- |\n| one | two |\n"
        );
    }

    #[test]
    fn split_code_fence_finalizes_to_full_source() {
        let mut stream = controller();
        stream.push_delta("```rust\nfn");
        assert_eq!(stream.committed(), "");
        assert!(stream.visible().contains("```rust"));
        stream.push_delta(" main() {}\n```\n");
        while stream.run_commit_tick().is_some() {}
        assert_eq!(stream.finalize(), "```rust\nfn main() {}\n```\n");
    }

    #[test]
    fn unicode_split_across_delta_boundary_is_preserved() {
        let mut stream = controller();
        stream.push_delta("hello ");
        stream.push_delta("🦀\n");
        assert_eq!(stream.run_commit_tick(), Some("hello 🦀\n".to_string()));
        assert_eq!(stream.committed(), "hello 🦀\n");
    }

    #[test]
    fn replace_committed_keeps_utf8_boundary_before_table() {
        let mut stream = controller();
        stream.replace_committed("1234567");
        stream.push_delta("café — \n| A | B |\n");
        assert_eq!(stream.run_commit_tick(), Some("café — \n".to_string()));
        assert_eq!(stream.committed(), "1234567café — \n");
        assert_eq!(stream.live(), "| A | B |\n");
    }

    #[test]
    fn replace_committed_continues_like_single_shot_stream() {
        let prefix = "Restored prefix\n";
        let tail = "café — \n| A | B |\n| --- | --- |\n| one | two |\n\nafter\n";
        let full = format!("{prefix}{tail}");

        let mut resumed = controller();
        resumed.replace_committed(prefix);
        for chunk in utf8_chunks(tail, 4) {
            resumed.push_delta(chunk);
            while resumed.run_commit_tick().is_some() {}
        }
        let resumed = resumed.finalize();

        let mut single_shot = controller();
        for chunk in utf8_chunks(&full, 4) {
            single_shot.push_delta(chunk);
            while single_shot.run_commit_tick().is_some() {}
        }
        let single_shot = single_shot.finalize();

        assert_eq!(resumed, full);
        assert_eq!(resumed, single_shot);
        assert_eq!(render_text(&resumed), render_text(&full));
    }

    #[test]
    fn snapshot_resume_mid_table_holds_until_complete() {
        let snapshot = "café — \n| A | B |\n";
        let rest = "| --- | --- |\n| one | two |\n\nafter\n";
        let mut stream = controller();
        stream.replace_committed(snapshot);

        stream.push_delta("| --- | --- |\n");
        assert_eq!(stream.run_commit_tick(), None);
        stream.push_delta("| one | two |\n");
        assert_eq!(stream.run_commit_tick(), None);
        stream.push_delta("\nafter\n");
        while stream.run_commit_tick().is_some() {}

        assert_eq!(stream.finalize(), format!("{snapshot}{rest}"));
    }

    #[test]
    fn streamed_final_matches_non_streamed_render_for_fixture_corpus() {
        let corpus = [
            "# Title\nhello `world`\n",
            "```rust\nfn main() {}\n```\n",
            "| A | B |\n| --- | --- |\n| one | two |\n",
            "Loose list:\n1. One\n2. Two\n",
        ];
        for source in corpus {
            let mut stream = controller();
            for chunk in utf8_chunks(source, 3) {
                stream.push_delta(chunk);
                stream.run_commit_tick();
            }
            let streamed = stream.finalize();
            assert_eq!(streamed, source);
            assert_eq!(render_text(&streamed), render_text(source));
        }
    }

    #[test]
    fn completed_table_commits_as_one_queue_item_before_following_text() {
        let mut stream = controller();
        stream.push_delta("intro\n| A | B |\n| --- | --- |\n| one | two |\n\nafter\n");
        assert_eq!(stream.queued_lines(), 3);
        assert_eq!(stream.run_commit_tick(), Some("intro\n".to_string()));
        assert_eq!(
            stream.run_commit_tick(),
            Some("| A | B |\n| --- | --- |\n| one | two |\n\n".to_string())
        );
        assert_eq!(stream.run_commit_tick(), Some("after\n".to_string()));
    }

    fn utf8_chunks(source: &str, max_bytes: usize) -> Vec<&str> {
        let mut out = Vec::new();
        let mut start = 0;
        while start < source.len() {
            let mut end = (start + max_bytes).min(source.len());
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            out.push(&source[start..end]);
            start = end;
        }
        out
    }

    fn render_text(source: &str) -> Vec<String> {
        render_markdown(source, None)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }
}
