// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::render::MarkdownRenderer;
use crate::table_detect::{is_table_delimiter_line, is_table_header_line};
use crate::text_safety::sanitize_tool_text;
use crate::vendored::markdown_stream::MarkdownStreamCollector;
use crate::vendored::terminal_hyperlinks::HyperlinkLine;

use super::chunking::{AdaptiveChunkingPolicy, DrainPlan, QueueSnapshot};
use super::commit_tick::drain_with_policy;
use super::table_holdback::{TableHoldbackScanner, TableHoldbackState};

pub trait CommitDrain {
    fn run_commit_tick(&mut self) -> Option<String>;
}

#[derive(Debug, Clone)]
struct QueuedLine {
    text: String,
    queued_at: Instant,
    line_count: usize,
}

#[derive(Debug, Clone)]
struct StablePrefixLenCache {
    source_start: usize,
    width: Option<usize>,
    stable_prefix_len: usize,
}

#[derive(Debug, Clone)]
pub struct StreamController {
    collector: MarkdownStreamCollector,
    committed: String,
    raw_source: String,
    rendered_lines: Vec<HyperlinkLine>,
    enqueued_stable_len: usize,
    emitted_stable_len: usize,
    enqueued_source_len: usize,
    queue: VecDeque<QueuedLine>,
    table_scanner: TableHoldbackScanner,
    policy: AdaptiveChunkingPolicy,
    stable_prefix_len_cache: Option<StablePrefixLenCache>,
    width: Option<usize>,
}

impl StreamController {
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        Self {
            collector: MarkdownStreamCollector::new(width, cwd),
            committed: String::new(),
            raw_source: String::new(),
            rendered_lines: Vec::new(),
            enqueued_stable_len: 0,
            emitted_stable_len: 0,
            enqueued_source_len: 0,
            queue: VecDeque::new(),
            table_scanner: TableHoldbackScanner::default(),
            policy: AdaptiveChunkingPolicy::default(),
            stable_prefix_len_cache: None,
            width,
        }
    }

    pub fn clear(&mut self) {
        self.collector.clear();
        self.committed.clear();
        self.raw_source.clear();
        self.rendered_lines.clear();
        self.enqueued_stable_len = 0;
        self.emitted_stable_len = 0;
        self.enqueued_source_len = 0;
        self.queue.clear();
        self.table_scanner.reset();
        self.policy.reset();
        self.stable_prefix_len_cache = None;
    }

    pub fn replace_committed(&mut self, content: &str) {
        let sanitized = sanitize_tool_text(content);
        self.replace_sanitized_committed(&sanitized);
    }

    pub(crate) fn replace_sanitized_committed(&mut self, content: &str) {
        self.clear();
        self.committed.push_str(content);
        self.raw_source.push_str(content);
        self.enqueued_source_len = self.raw_source.len();
        self.table_scanner.replace_prefix(content);
        self.recompute_streaming_render();
        self.enqueued_stable_len = self.rendered_lines.len();
        self.emitted_stable_len = self.rendered_lines.len();
        self.assert_holdback_boundary();
    }

    pub fn set_width(&mut self, width: Option<usize>) {
        if self.width == width {
            return;
        }
        let had_pending_queue = !self.queue.is_empty();
        let had_live_tail = self.has_tail();
        self.width = width;
        self.collector.set_width(width);
        if self.raw_source.is_empty() {
            return;
        }
        self.recompute_streaming_render();
        self.emitted_stable_len = self.emitted_stable_len.min(self.rendered_lines.len());
        if had_pending_queue
            && self.emitted_stable_len == self.rendered_lines.len()
            && self.emitted_stable_len > 0
        {
            self.emitted_stable_len -= 1;
        }
        self.queue.clear();
        if self.emitted_stable_len > 0 && !had_pending_queue && !had_live_tail {
            self.enqueued_stable_len = self.rendered_lines.len();
            self.enqueued_source_len = self.raw_source.len();
            return;
        }
        self.rebuild_stable_queue_from_render();
    }

    pub fn push_delta(&mut self, delta: &str) {
        let sanitized = sanitize_tool_text(delta);
        self.push_sanitized_delta(&sanitized);
    }

    pub(crate) fn push_sanitized_delta(&mut self, delta: &str) {
        self.collector.push_delta(delta);
        if let Some(source) = self.collector.commit_complete_source() {
            self.ingest_complete_source(&source);
        } else {
            self.recompute_streaming_render();
        }
    }

    pub fn committed(&self) -> &str {
        &self.committed
    }

    pub fn live(&self) -> String {
        let mut live =
            self.raw_source[self.enqueued_source_len.min(self.raw_source.len())..].to_string();
        live.push_str(self.collector.pending_source());
        live
    }

    pub fn visible(&self) -> String {
        let mut visible = self.committed.clone();
        for line in &self.queue {
            visible.push_str(&line.text);
        }
        visible.push_str(&self.live());
        visible
    }

    pub fn current_tail_lines(&self) -> Vec<HyperlinkLine> {
        let start = self.enqueued_stable_len.min(self.rendered_lines.len());
        self.rendered_lines[start..].to_vec()
    }

    pub fn has_live_tail(&self) -> bool {
        self.has_tail()
    }

    pub fn queued_lines(&self) -> usize {
        self.queue.iter().map(|line| line.line_count).sum()
    }

    pub fn stable_lines_ready(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.queue
            .front()
            .map(|line| now.saturating_duration_since(line.queued_at))
    }

    pub fn finalize(&mut self) -> String {
        let remainder = self.collector.finalize_and_drain_source();
        if !remainder.is_empty() {
            self.raw_source.push_str(&remainder);
            self.table_scanner.push_source_chunk(&remainder);
        }
        let out = if self.raw_source.is_empty() {
            self.committed.clone()
        } else {
            self.raw_source.clone()
        };
        self.clear();
        out
    }

    pub fn drain_all_stable(&mut self) -> Option<String> {
        self.drain(DrainPlan::Batch(usize::MAX))
    }

    fn ingest_complete_source(&mut self, source: &str) {
        self.raw_source.push_str(source);
        self.table_scanner.push_source_chunk(source);
        self.recompute_streaming_render();
        self.sync_stable_queue();
    }

    fn render_source(&self, source: &str) -> Vec<HyperlinkLine> {
        if source.is_empty() {
            Vec::new()
        } else {
            MarkdownRenderer::new(self.width).render_with_links(source)
        }
    }

    fn recompute_streaming_render(&mut self) {
        let mut source = self.raw_source.clone();
        source.push_str(self.collector.pending_source());
        self.rendered_lines = self.render_source(&source);
    }

    fn has_tail(&self) -> bool {
        self.enqueued_stable_len < self.rendered_lines.len()
    }

    fn target_stable_source_len(&self) -> usize {
        let source_len = self.raw_source.len();
        let target = match self.table_scanner.state() {
            TableHoldbackState::None => source_len,
            TableHoldbackState::PendingHeader { header_start }
            | TableHoldbackState::Confirmed {
                table_start: header_start,
            } => header_start,
        };
        previous_char_boundary(&self.raw_source, target.min(source_len))
    }

    fn compute_target_stable_len(&mut self) -> usize {
        let target_source_len = self.target_stable_source_len();
        self.stable_prefix_len_for_source_start(target_source_len)
            .max(self.emitted_stable_len)
    }

    fn sync_stable_queue(&mut self) -> bool {
        let target_source_len = self.target_stable_source_len();
        let target_stable_len = self.compute_target_stable_len();
        if target_source_len < self.enqueued_source_len {
            self.queue.clear();
            if self.committed.len() < target_source_len {
                self.enqueue_source_range(self.committed.len(), target_source_len);
            }
            self.enqueued_source_len = target_source_len;
            self.enqueued_stable_len = target_stable_len;
            return !self.queue.is_empty();
        }
        if target_source_len == self.enqueued_source_len {
            self.enqueued_stable_len = target_stable_len;
            return false;
        }
        self.enqueue_source_range(self.enqueued_source_len, target_source_len);
        self.enqueued_source_len = target_source_len;
        self.enqueued_stable_len = target_stable_len;
        true
    }

    fn rebuild_stable_queue_from_render(&mut self) {
        let target_source_len = self.target_stable_source_len();
        let target_stable_len = self.compute_target_stable_len();
        self.queue.clear();
        if self.committed.len() < target_source_len {
            self.enqueue_source_range(self.committed.len(), target_source_len);
        }
        self.enqueued_source_len = target_source_len;
        self.enqueued_stable_len = target_stable_len;
    }

    fn enqueue_source_range(&mut self, start: usize, end: usize) {
        if start >= end || start >= self.raw_source.len() {
            return;
        }
        let start = previous_char_boundary(&self.raw_source, start);
        let end = previous_char_boundary(&self.raw_source, end.min(self.raw_source.len()));
        let mut chunks = VecDeque::new();
        enqueue_stable_source(&mut chunks, &self.raw_source[start..end]);
        let now = Instant::now();
        for text in chunks {
            let line_count = text
                .split_inclusive('\n')
                .filter(|line| !line.is_empty())
                .count()
                .max(1);
            self.queue.push_back(QueuedLine {
                text,
                queued_at: now,
                line_count,
            });
        }
    }

    fn stable_prefix_len_for_source_start(&mut self, source_start: usize) -> usize {
        if let Some(cache) = &self.stable_prefix_len_cache {
            if cache.source_start == source_start && cache.width == self.width {
                return cache.stable_prefix_len;
            }
        }
        let stable_prefix_len = self.render_source(&self.raw_source[..source_start]).len();
        self.stable_prefix_len_cache = Some(StablePrefixLenCache {
            source_start,
            width: self.width,
            stable_prefix_len,
        });
        stable_prefix_len
    }

    fn assert_holdback_boundary(&self) {
        debug_assert_eq!(
            self.table_scanner.source_offset(),
            self.raw_source.len(),
            "table scanner offset must match raw source bytes"
        );
        let boundary = match self.table_scanner.state() {
            TableHoldbackState::None => return,
            TableHoldbackState::PendingHeader { header_start } => header_start,
            TableHoldbackState::Confirmed { table_start } => table_start,
        };
        debug_assert!(
            boundary <= self.raw_source.len() && self.raw_source.is_char_boundary(boundary),
            "invalid table holdback boundary {boundary} for source {}",
            self.raw_source.len()
        );
    }

    fn drain(&mut self, plan: DrainPlan) -> Option<String> {
        let count = match plan {
            DrainPlan::Single => 1,
            DrainPlan::Batch(count) => count,
        };
        if count == 0 || self.queue.is_empty() {
            return None;
        }
        let mut remaining = count;
        let mut drained = String::new();
        while remaining > 0 {
            let Some(line) = self.queue.pop_front() else {
                break;
            };
            remaining = remaining.saturating_sub(line.line_count.max(1));
            self.committed.push_str(&line.text);
            drained.push_str(&line.text);
        }
        if !drained.is_empty() {
            self.emitted_stable_len = self.render_source(&self.committed).len();
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
            if is_table_header_line(previous_line) && is_table_delimiter_line(trimmed) {
                table_start = Some(previous_start);
            }
        }
        previous = (!trimmed.is_empty()).then_some((offset, trimmed));
        offset = offset.saturating_add(line.len());
    }
    table_start.map(|start| (start, source.len()))
}

impl CommitDrain for StreamController {
    fn run_commit_tick(&mut self) -> Option<String> {
        let now = Instant::now();
        let snapshot = QueueSnapshot {
            queued_lines: self.queued_lines(),
            oldest_age: self.oldest_queued_age(now),
        };
        let mut policy = std::mem::take(&mut self.policy);
        let drained = drain_with_policy(&mut policy, snapshot, now, |plan| self.drain(plan));
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
        assert_eq!(stream.live(), "");
        assert_eq!(stream.visible(), "one\ntwo\n");
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
        assert_eq!(stream.visible(), "```rust\nfn");
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
    fn streamed_model_text_is_escape_inert_in_source_and_rendered_lines() {
        let mut stream = controller();
        stream.push_delta(injected_model_text());

        let tail = stream.current_tail_lines();
        assert!(!tail.is_empty());
        assert_hyperlink_lines_escape_inert(&tail);
        assert_escape_inert(&stream.visible());
        assert_model_text_survives(&stream.visible());

        stream.push_delta("\n");
        while let Some(drained) = stream.run_commit_tick() {
            assert_escape_inert(&drained);
        }
        let finalized = stream.finalize();

        assert_escape_inert(&finalized);
        assert_model_text_survives(&finalized);

        let mut restored = controller();
        restored.replace_committed(injected_model_text());
        assert_escape_inert(&restored.visible());
        assert_hyperlink_lines_escape_inert(&restored.current_tail_lines());
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
        assert_eq!(stream.queued_lines(), 6);
        assert_eq!(stream.run_commit_tick(), Some("intro\n".to_string()));
        assert_eq!(
            stream.run_commit_tick(),
            Some("| A | B |\n| --- | --- |\n| one | two |\n\n".to_string())
        );
        assert_eq!(stream.run_commit_tick(), Some("after\n".to_string()));
    }

    #[test]
    fn oldest_queued_age_tracks_first_stable_line() {
        let mut stream = controller();
        stream.push_delta("one\ntwo\n");
        let age = stream.oldest_queued_age(Instant::now() + Duration::from_millis(150));
        assert!(age.is_some_and(|age| age >= Duration::from_millis(100)));
    }

    #[test]
    fn set_width_rebuilds_queue_without_reemitting_emitted_lines() {
        let mut stream = StreamController::new(Some(12), std::path::Path::new("."));
        stream.push_delta("alpha beta gamma\ndelta\n");
        assert!(stream.run_commit_tick().is_some());
        let emitted = stream.committed().to_string();
        stream.set_width(Some(24));
        while stream.run_commit_tick().is_some() {}
        assert!(stream.committed().starts_with(&emitted));
        assert_eq!(stream.finalize(), "alpha beta gamma\ndelta\n");
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

    fn injected_model_text() -> &'static str {
        "lead \u{1b}[31mred \u{1b}[2Jclear\u{7} bell \u{009b}31mcsi \u{1b}]8;;http://evil\u{7}TEXT\u{1b}]8;;\u{7} tail"
    }

    fn assert_hyperlink_lines_escape_inert(lines: &[HyperlinkLine]) {
        for line in lines {
            for span in &line.line.spans {
                assert_escape_inert(span.content.as_ref());
            }
        }
    }

    fn assert_escape_inert(text: &str) {
        assert!(!text.as_bytes().contains(&0x1b), "raw ESC in {text:?}");
        assert!(!text.as_bytes().contains(&0x07), "raw BEL in {text:?}");
        assert!(!text.as_bytes().contains(&0x9b), "raw CSI byte in {text:?}");
        assert!(!text.contains('\u{009b}'), "raw CSI char in {text:?}");
        assert!(!text.contains("http://evil"), "raw OSC8 URL in {text:?}");
    }

    fn assert_model_text_survives(text: &str) {
        for fragment in ["lead", "red", "clear", "bell", "csi", "TEXT", "tail"] {
            assert!(text.contains(fragment), "missing {fragment:?} in {text:?}");
        }
    }
}
