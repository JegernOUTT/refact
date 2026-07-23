//! Execution output has two storage paths. Every process writes to a bounded transcript used by
//! streaming UIs and background/service reads. Foreground processes also write to a raw capture
//! before transcript eviction so final shell/cmdline output filters can see lines outside the
//! transcript window. Raw foreground capture is explicitly capped at 16 MiB stdout and 4 MiB
//! stderr; when a cap is hit the captured stream ends with an `[X bytes elided]` marker.
//! TTY processes additionally keep a combined raw buffer of unnormalized PTY bytes (escape
//! sequences and carriage returns intact, arrival order preserved) that byte-offset cursors can
//! stream from; it shares the 16 MiB cap and freezes at the cap instead of evicting.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::spill::{SpillTarget, SpillWriter};
use crate::types::{
    current_timestamp_ms, ExecOutputChunk, ExecOutputStream, ExecProcessId, ExecReadResult,
};

pub const DEFAULT_MAX_BYTES: usize = 512 * 1024;
pub const DEFAULT_SPILL_THRESHOLD_BYTES: usize = 256 * 1024;
pub const FOREGROUND_STDOUT_CAPTURE_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const FOREGROUND_STDERR_CAPTURE_MAX_BYTES: usize = 4 * 1024 * 1024;

fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub struct ExecTranscript {
    process_id: ExecProcessId,
    chunks: VecDeque<ExecOutputChunk>,
    next_seq: u64,
    total_bytes_appended: usize,
    total_lines_appended: u64,
    dropped_chunks: u64,
    dropped_bytes: usize,
    truncated_chunks: u64,
    current_bytes: usize,
    max_bytes: usize,
    spill_threshold_bytes: usize,
    spill: Option<Arc<Mutex<SpillState>>>,
    disk_log_path: Option<PathBuf>,
    last_spill_error: Option<String>,
    pre_spill_buffer: String,
    spill_started: bool,
}

struct SpillState {
    target: SpillTarget,
    writer: Option<SpillWriter>,
    disabled: bool,
}

#[derive(Clone)]
pub(crate) struct SpillAppend {
    state: Arc<Mutex<SpillState>>,
    text: String,
}

#[derive(Clone)]
struct ExecRawStreamCapture {
    text: String,
    max_bytes: usize,
    elided_bytes: usize,
    hit_limit: bool,
}

impl ExecRawStreamCapture {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::new(),
            max_bytes: max_bytes.max(1),
            elided_bytes: 0,
            hit_limit: false,
        }
    }

    fn append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.hit_limit {
            self.elided_bytes = self.elided_bytes.saturating_add(text.len());
            return;
        }
        let remaining = self.max_bytes.saturating_sub(self.text.len());
        if remaining == 0 {
            self.hit_limit = true;
            self.elided_bytes = self.elided_bytes.saturating_add(text.len());
            return;
        }
        let captured = truncate_to_char_boundary(text, remaining);
        self.text.push_str(captured);
        if captured.len() < text.len() {
            self.hit_limit = true;
        }
        self.elided_bytes = self
            .elided_bytes
            .saturating_add(text.len().saturating_sub(captured.len()));
    }

    fn text_with_marker(&self) -> String {
        let mut text = self.text.clone();
        if self.elided_bytes > 0 {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!("[{} bytes elided]\n", self.elided_bytes));
        }
        text
    }
}

pub struct ExecRawCapture {
    process_id: ExecProcessId,
    stdout: ExecRawStreamCapture,
    stderr: ExecRawStreamCapture,
    combined: Option<ExecRawStreamCapture>,
}

impl ExecRawCapture {
    pub fn foreground(process_id: ExecProcessId) -> Self {
        Self {
            process_id,
            stdout: ExecRawStreamCapture::new(FOREGROUND_STDOUT_CAPTURE_MAX_BYTES),
            stderr: ExecRawStreamCapture::new(FOREGROUND_STDERR_CAPTURE_MAX_BYTES),
            combined: None,
        }
    }

    pub fn with_combined_capture(mut self) -> Self {
        self.combined = Some(ExecRawStreamCapture::new(
            FOREGROUND_STDOUT_CAPTURE_MAX_BYTES,
        ));
        self
    }

    pub fn append(&mut self, stream: &ExecOutputStream, text: &str) {
        match stream {
            ExecOutputStream::Stdout | ExecOutputStream::Combined => self.stdout.append(text),
            ExecOutputStream::Stderr => self.stderr.append(text),
        }
    }

    pub fn append_combined_raw(&mut self, text: &str) {
        if let Some(combined) = self.combined.as_mut() {
            combined.append(text);
        }
    }

    pub fn read_combined_since(
        &self,
        offset: u64,
        max_bytes: Option<usize>,
    ) -> Option<ExecRawRead> {
        let combined = self.combined.as_ref()?;
        let text = &combined.text;
        let len = text.len();
        let mut start = offset.min(len as u64) as usize;
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        if let Some(max_bytes) = max_bytes {
            let cap_start = len.saturating_sub(max_bytes);
            if cap_start > start {
                start = cap_start;
                while start < len && !text.is_char_boundary(start) {
                    start += 1;
                }
            }
        }
        Some(ExecRawRead {
            process_id: self.process_id.clone(),
            start_offset: start as u64,
            new_offset: len as u64,
            text: text[start..].to_string(),
        })
    }

    pub fn read(&self) -> ExecRawOutput {
        ExecRawOutput {
            process_id: self.process_id.clone(),
            stdout: self.stdout.text_with_marker(),
            stderr: self.stderr.text_with_marker(),
            stdout_captured_bytes: self.stdout.text.len(),
            stderr_captured_bytes: self.stderr.text.len(),
            stdout_elided_bytes: self.stdout.elided_bytes,
            stderr_elided_bytes: self.stderr.elided_bytes,
            stdout_max_bytes: self.stdout.max_bytes,
            stderr_max_bytes: self.stderr.max_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRawOutput {
    pub process_id: ExecProcessId,
    pub stdout: String,
    pub stderr: String,
    pub stdout_captured_bytes: usize,
    pub stderr_captured_bytes: usize,
    pub stdout_elided_bytes: usize,
    pub stderr_elided_bytes: usize,
    pub stdout_max_bytes: usize,
    pub stderr_max_bytes: usize,
}

impl ExecRawOutput {
    pub fn is_truncated(&self) -> bool {
        self.stdout_elided_bytes > 0 || self.stderr_elided_bytes > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRawRead {
    pub process_id: ExecProcessId,
    pub start_offset: u64,
    pub new_offset: u64,
    pub text: String,
}

impl ExecTranscript {
    fn normalize_max_bytes(max_bytes: usize) -> usize {
        max_bytes.max(1)
    }

    pub fn new(process_id: ExecProcessId, max_bytes: usize) -> Self {
        Self::new_with_spill(process_id, max_bytes, None, DEFAULT_SPILL_THRESHOLD_BYTES)
    }

    pub fn new_with_spill(
        process_id: ExecProcessId,
        max_bytes: usize,
        chat_id: Option<String>,
        spill_threshold_bytes: usize,
    ) -> Self {
        let spill = chat_id.as_deref().and_then(|chat_id| {
            SpillTarget::new(chat_id, &process_id)
                .map(|target| {
                    Arc::new(Mutex::new(SpillState {
                        target,
                        writer: None,
                        disabled: false,
                    }))
                })
                .map_err(|error| tracing::warn!("exec spill disabled for {process_id}: {error}"))
                .ok()
        });
        Self {
            process_id,
            chunks: VecDeque::new(),
            next_seq: 0,
            total_bytes_appended: 0,
            total_lines_appended: 0,
            dropped_chunks: 0,
            dropped_bytes: 0,
            truncated_chunks: 0,
            current_bytes: 0,
            max_bytes: Self::normalize_max_bytes(max_bytes),
            spill_threshold_bytes,
            spill,
            disk_log_path: None,
            last_spill_error: None,
            pre_spill_buffer: String::new(),
            spill_started: false,
        }
    }

    pub fn with_default_max(process_id: ExecProcessId) -> Self {
        Self::new(process_id, DEFAULT_MAX_BYTES)
    }

    pub fn append(&mut self, stream: ExecOutputStream, text: String) -> u64 {
        self.append_chunk_to_ring(stream, text).seq
    }

    pub(crate) fn append_chunk(
        &mut self,
        stream: ExecOutputStream,
        text: String,
    ) -> (ExecOutputChunk, Option<SpillAppend>) {
        let spill_append = self.prepare_spill_append(&text);
        (self.append_chunk_to_ring(stream, text), spill_append)
    }

    fn append_chunk_to_ring(&mut self, stream: ExecOutputStream, text: String) -> ExecOutputChunk {
        let seq = self.next_seq;
        if text.is_empty() {
            return ExecOutputChunk {
                process_id: self.process_id.clone(),
                seq,
                stream,
                text,
                timestamp_ms: current_timestamp_ms(),
            };
        }
        self.next_seq += 1;

        let line_count = text.lines().count().max(1) as u64;
        self.total_bytes_appended += text.len();
        self.total_lines_appended += line_count;

        let (final_text, was_truncated) = if text.len() > self.max_bytes {
            let truncated = truncate_to_char_boundary(&text, self.max_bytes).to_string();
            (truncated, true)
        } else {
            (text, false)
        };

        if was_truncated {
            self.truncated_chunks += 1;
        }

        let chunk_bytes = final_text.len();

        while !self.chunks.is_empty() && self.current_bytes + chunk_bytes > self.max_bytes {
            if let Some(evicted) = self.chunks.pop_front() {
                self.current_bytes -= evicted.text.len();
                self.dropped_chunks += 1;
                self.dropped_bytes += evicted.text.len();
            }
        }

        self.current_bytes += chunk_bytes;
        let chunk = ExecOutputChunk {
            process_id: self.process_id.clone(),
            seq,
            stream,
            text: final_text,
            timestamp_ms: current_timestamp_ms(),
        };
        self.chunks.push_back(chunk.clone());
        chunk
    }

    fn prepare_spill_append(&mut self, text: &str) -> Option<SpillAppend> {
        if text.is_empty() {
            return None;
        }
        let state = self.spill.as_ref()?.clone();
        if self.spill_started {
            return Some(SpillAppend {
                state,
                text: text.to_string(),
            });
        }
        if self.total_bytes_appended.saturating_add(text.len()) <= self.spill_threshold_bytes {
            self.pre_spill_buffer.push_str(text);
            return None;
        }
        self.spill_started = true;
        let mut full_text = std::mem::take(&mut self.pre_spill_buffer);
        full_text.push_str(text);
        Some(SpillAppend {
            state,
            text: full_text,
        })
    }

    pub(crate) fn record_spill_result(&mut self, result: &Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                self.disk_log_path = Some(path.clone());
                self.last_spill_error = None;
            }
            Err(error) => {
                self.last_spill_error = Some(error.clone());
            }
        }
    }

    pub fn read_since(&self, since_seq: u64) -> Vec<&ExecOutputChunk> {
        self.chunks.iter().filter(|c| c.seq >= since_seq).collect()
    }

    pub fn read(&self, since_seq: u64, limit: Option<usize>) -> ExecReadResult {
        let mut chunks = self
            .read_since(since_seq)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(limit) = limit {
            chunks.truncate(limit);
        }
        let next_seq = chunks
            .last()
            .map(|chunk| chunk.seq + 1)
            .unwrap_or(since_seq);
        ExecReadResult {
            process_id: self.process_id.clone(),
            found: true,
            since_seq,
            next_seq,
            latest_seq: self.next_seq,
            chunks,
            total_bytes_appended: self.total_bytes_appended,
            total_lines_appended: self.total_lines_appended,
            dropped_chunks: self.dropped_chunks,
            dropped_bytes: self.dropped_bytes,
            truncated_chunks: self.truncated_chunks,
            current_bytes: self.current_bytes,
            max_bytes: self.max_bytes,
            chunk_count: self.chunks.len(),
            is_truncated: self.is_truncated(),
            disk_log_path: self.disk_log_path.clone(),
        }
    }

    pub fn process_id(&self) -> &ExecProcessId {
        &self.process_id
    }

    pub fn total_bytes_appended(&self) -> usize {
        self.total_bytes_appended
    }

    pub fn total_lines_appended(&self) -> u64 {
        self.total_lines_appended
    }

    pub fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks
    }

    pub fn dropped_bytes(&self) -> usize {
        self.dropped_bytes
    }

    pub fn truncated_chunks(&self) -> u64 {
        self.truncated_chunks
    }

    pub fn is_truncated(&self) -> bool {
        self.dropped_chunks > 0 || self.truncated_chunks > 0
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn disk_log_path(&self) -> Option<&PathBuf> {
        self.disk_log_path.as_ref()
    }

    pub fn last_spill_error(&self) -> Option<&str> {
        self.last_spill_error.as_deref()
    }

    #[doc(hidden)]
    pub(crate) fn set_spill_target_for_test(&mut self, target: SpillTarget) {
        self.spill = Some(Arc::new(Mutex::new(SpillState {
            target,
            writer: None,
            disabled: false,
        })));
    }
}

impl SpillAppend {
    pub(crate) async fn write(self) -> Result<PathBuf, String> {
        let mut state = self.state.lock_owned().await;
        if state.disabled {
            return Err("exec spill disabled after previous failure".to_string());
        }
        if state.writer.is_none() {
            let target = state.target.clone();
            state = create_spill_writer(state, target).await?;
        }
        let Some(writer) = state.writer.as_mut() else {
            state.disabled = true;
            return Err("exec spill writer unavailable".to_string());
        };
        if let Err(error) = writer.write_line(&self.text).await {
            state.disabled = true;
            return Err(error);
        }
        Ok(writer.path().clone())
    }
}

async fn create_spill_writer(
    mut state: OwnedMutexGuard<SpillState>,
    target: SpillTarget,
) -> Result<OwnedMutexGuard<SpillState>, String> {
    match SpillWriter::create(&target).await {
        Ok(writer) => {
            state.writer = Some(writer);
            Ok(state)
        }
        Err(error) => {
            state.disabled = true;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::SpillTarget;

    fn make_transcript(max_bytes: usize) -> ExecTranscript {
        ExecTranscript::new(ExecProcessId("exec_test".to_string()), max_bytes)
    }

    #[test]
    fn test_append_stdout_chunk() {
        let mut t = make_transcript(1024);
        let seq = t.append(ExecOutputStream::Stdout, "hello stdout".to_string());
        assert_eq!(seq, 0);

        let chunks = t.read_since(0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello stdout");
        assert_eq!(chunks[0].stream, ExecOutputStream::Stdout);
        assert_eq!(chunks[0].seq, 0);
    }

    #[test]
    fn test_append_stderr_chunk() {
        let mut t = make_transcript(1024);
        let seq = t.append(ExecOutputStream::Stderr, "error output".to_string());
        assert_eq!(seq, 0);

        let chunks = t.read_since(0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "error output");
        assert_eq!(chunks[0].stream, ExecOutputStream::Stderr);
    }

    #[test]
    fn test_append_preserves_stream_identity() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "out1".to_string());
        t.append(ExecOutputStream::Stderr, "err1".to_string());
        t.append(ExecOutputStream::Combined, "combined".to_string());

        let chunks = t.read_since(0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].stream, ExecOutputStream::Stdout);
        assert_eq!(chunks[1].stream, ExecOutputStream::Stderr);
        assert_eq!(chunks[2].stream, ExecOutputStream::Combined);
    }

    #[test]
    fn test_monotonic_sequence_numbers() {
        let mut t = make_transcript(4096);
        let s0 = t.append(ExecOutputStream::Stdout, "a".to_string());
        let s1 = t.append(ExecOutputStream::Stdout, "b".to_string());
        let s2 = t.append(ExecOutputStream::Stderr, "c".to_string());
        let s3 = t.append(ExecOutputStream::Stdout, "d".to_string());

        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
        assert_eq!(t.next_seq(), 4);
    }

    #[test]
    fn test_read_since_cursor_all() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "a".to_string());
        t.append(ExecOutputStream::Stdout, "b".to_string());
        t.append(ExecOutputStream::Stdout, "c".to_string());

        let all = t.read_since(0);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_read_since_cursor_partial() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "a".to_string());
        t.append(ExecOutputStream::Stdout, "b".to_string());
        t.append(ExecOutputStream::Stdout, "c".to_string());

        let from_second = t.read_since(1);
        assert_eq!(from_second.len(), 2);
        assert_eq!(from_second[0].text, "b");
        assert_eq!(from_second[1].text, "c");
    }

    #[test]
    fn test_read_since_cursor_none() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "a".to_string());

        let empty = t.read_since(1);
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_read_since_cursor_beyond_appended() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "only".to_string());

        let nothing = t.read_since(100);
        assert_eq!(nothing.len(), 0);
    }

    #[test]
    fn test_read_result_limit_and_cursor() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "a".to_string());
        t.append(ExecOutputStream::Stdout, "b".to_string());
        t.append(ExecOutputStream::Stdout, "c".to_string());

        let result = t.read(0, Some(2));
        assert!(result.found);
        assert_eq!(result.chunks.len(), 2);
        assert_eq!(result.next_seq, 2);
        assert_eq!(result.latest_seq, 3);
        assert_eq!(result.chunk_count, 3);
    }

    #[test]
    fn test_read_result_empty_limit() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "a".to_string());

        let result = t.read(0, Some(0));
        assert!(result.chunks.is_empty());
        assert_eq!(result.next_seq, 0);
        assert_eq!(result.latest_seq, 1);
    }

    #[test]
    fn test_byte_limit_eviction() {
        let mut t = make_transcript(20);
        t.append(ExecOutputStream::Stdout, "12345678".to_string());
        t.append(ExecOutputStream::Stdout, "abcdefgh".to_string());
        assert!(!t.is_truncated(), "no eviction yet");

        t.append(ExecOutputStream::Stdout, "XXXXXXXXX".to_string());

        assert!(t.is_truncated(), "should have evicted chunks");
        assert!(t.dropped_chunks() > 0);
        assert!(t.dropped_bytes() > 0);
    }

    #[test]
    fn test_byte_limit_total_tracking_persists_after_eviction() {
        let mut t = make_transcript(10);
        t.append(ExecOutputStream::Stdout, "12345".to_string());
        t.append(ExecOutputStream::Stdout, "67890".to_string());
        t.append(ExecOutputStream::Stdout, "abcde".to_string());

        assert_eq!(
            t.total_bytes_appended(),
            15,
            "total should include evicted bytes"
        );
        assert!(t.dropped_bytes() > 0);
    }

    #[test]
    fn test_long_chunk_truncation() {
        let max = 20;
        let mut t = make_transcript(max);
        let long_text = "a".repeat(100);
        t.append(ExecOutputStream::Stdout, long_text);

        assert_eq!(t.truncated_chunks(), 1);
        assert!(t.is_truncated());
        assert_eq!(t.chunk_count(), 1);
        assert!(t.current_bytes() <= max);
    }

    #[test]
    fn test_long_chunk_with_multibyte_utf8_truncation() {
        let max = 5;
        let mut t = make_transcript(max);
        let text = "αβγδε".to_string();
        t.append(ExecOutputStream::Stdout, text);

        let chunks = t.read_since(0);
        assert_eq!(chunks.len(), 1);
        let chunk_bytes = chunks[0].text.len();
        assert!(
            chunk_bytes <= max,
            "chunk bytes ({chunk_bytes}) should be <= max ({max})"
        );
        assert!(
            std::str::from_utf8(chunks[0].text.as_bytes()).is_ok(),
            "text must be valid UTF-8"
        );
    }

    #[test]
    fn test_current_bytes_stays_within_limit() {
        let max = 50;
        let mut t = make_transcript(max);
        for i in 0..20 {
            t.append(ExecOutputStream::Stdout, format!("line {i:03}"));
        }
        assert!(
            t.current_bytes() <= max,
            "current_bytes {} should not exceed max {max}",
            t.current_bytes()
        );
    }

    #[test]
    fn test_total_lines_appended_counts_lines() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "line1\nline2\nline3".to_string());
        t.append(ExecOutputStream::Stderr, "err1".to_string());

        assert_eq!(t.total_lines_appended(), 4);
    }

    #[test]
    fn test_total_lines_empty_text() {
        let mut t = make_transcript(4096);
        t.append(ExecOutputStream::Stdout, "".to_string());
        assert_eq!(t.total_lines_appended(), 0);
    }

    #[test]
    fn test_empty_appends_do_not_store_chunks() {
        let mut t = make_transcript(4096);
        for _ in 0..10_000 {
            t.append(ExecOutputStream::Stdout, "".to_string());
        }

        assert_eq!(t.chunk_count(), 0);
        assert_eq!(t.current_bytes(), 0);
        assert_eq!(t.next_seq(), 0);
        assert!(t.read_since(0).is_empty());
    }

    #[test]
    fn test_chunk_process_id_matches_transcript() {
        let pid = ExecProcessId("exec_test_pid".to_string());
        let mut t = ExecTranscript::new(pid.clone(), 4096);
        t.append(ExecOutputStream::Stdout, "hello".to_string());

        let chunks = t.read_since(0);
        assert_eq!(chunks[0].process_id, pid);
        assert_eq!(t.process_id(), &pid);
    }

    #[test]
    fn test_zero_byte_limit_is_clamped() {
        let mut t = make_transcript(0);
        t.append(ExecOutputStream::Stdout, "abcdef".to_string());

        assert_eq!(t.max_bytes(), 1);
        assert!(t.current_bytes() <= 1);
        assert!(t.is_truncated());
        let read = t.read(0, None);
        assert_eq!(read.max_bytes, 1);
        assert!(read.current_bytes <= 1);
    }

    #[tokio::test]
    async fn spill_failure_retains_in_memory_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join("not-a-dir");
        std::fs::write(&blocker, "block").unwrap();
        let target = SpillTarget::with_root(
            blocker,
            "chat-a",
            &ExecProcessId("exec_spill_fail".to_string()),
        );
        let mut t = ExecTranscript::new_with_spill(
            ExecProcessId("exec_spill_fail".to_string()),
            4096,
            Some("chat-a".to_string()),
            1,
        );
        t.set_spill_target_for_test(target);

        let (chunk, spill_append) =
            t.append_chunk(ExecOutputStream::Stdout, "retained output".to_string());
        let result = spill_append.expect("spill append").write().await;
        t.record_spill_result(&result);

        assert!(result.is_err());
        assert_eq!(chunk.seq, 0);
        assert_eq!(t.read(0, None).chunks, vec![chunk]);
        assert!(t.last_spill_error().is_some());
        assert!(t.disk_log_path().is_none());
    }

    #[tokio::test]
    async fn spill_backfills_output_before_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_backfill_pre".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-bp", &process_id);
        let mut t = ExecTranscript::new_with_spill(
            process_id.clone(),
            4096,
            Some("chat-bp".to_string()),
            20,
        );
        t.set_spill_target_for_test(target);

        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "hello".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, " world".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "!!!".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "crosses_here".to_string());
        let result = sa.expect("spill fires at threshold crossing").write().await;
        t.record_spill_result(&result);
        assert!(result.is_ok());

        let persisted = std::fs::read_to_string(t.disk_log_path().unwrap()).unwrap();
        assert_eq!(persisted, "hello world!!!crosses_here");
    }

    #[tokio::test]
    async fn spill_backfills_even_if_ring_evicted_before_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_backfill_ring".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-br", &process_id);
        let mut t =
            ExecTranscript::new_with_spill(process_id.clone(), 12, Some("chat-br".to_string()), 25);
        t.set_spill_target_for_test(target);

        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "chunk_one".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "chunk_two".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "big_chunk".to_string());
        let result = sa
            .expect("spill fires when threshold crossed")
            .write()
            .await;
        t.record_spill_result(&result);
        assert!(result.is_ok());

        let persisted = std::fs::read_to_string(t.disk_log_path().unwrap()).unwrap();
        assert_eq!(persisted, "chunk_onechunk_twobig_chunk");
        assert!(t.dropped_chunks() > 0, "ring must have evicted chunks");
    }

    #[tokio::test]
    async fn spill_backfill_happens_once() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_backfill_once".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-bo", &process_id);
        let mut t =
            ExecTranscript::new_with_spill(process_id.clone(), 12, Some("chat-bo".to_string()), 25);
        t.set_spill_target_for_test(target);

        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "chunk_one".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "chunk_two".to_string());
        assert!(sa.is_none());
        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "big_chunk".to_string());
        let result = sa.expect("first threshold crossing").write().await;
        t.record_spill_result(&result);
        assert!(result.is_ok());

        let (_c, sa) = t.append_chunk(ExecOutputStream::Stdout, "extra".to_string());
        let result = sa.expect("subsequent chunk also spills").write().await;
        t.record_spill_result(&result);
        assert!(result.is_ok());

        let persisted = std::fs::read_to_string(t.disk_log_path().unwrap()).unwrap();
        assert_eq!(persisted, "chunk_onechunk_twobig_chunkextra");
        assert_eq!(
            persisted.matches("chunk_one").count(),
            1,
            "pre-threshold content must not be duplicated"
        );
    }

    #[test]
    fn test_not_truncated_when_within_limit() {
        let mut t = make_transcript(1024);
        t.append(ExecOutputStream::Stdout, "small".to_string());
        assert!(!t.is_truncated());
        assert_eq!(t.dropped_chunks(), 0);
        assert_eq!(t.dropped_bytes(), 0);
        assert_eq!(t.truncated_chunks(), 0);
    }

    fn make_combined_capture() -> ExecRawCapture {
        ExecRawCapture::foreground(ExecProcessId("exec_raw".to_string())).with_combined_capture()
    }

    #[test]
    fn combined_raw_preserves_crlf_and_escape_bytes() {
        let mut capture = make_combined_capture();
        capture.append_combined_raw("a\r\n\x1b[31mred\x1b[0m\r\n");

        let read = capture.read_combined_since(0, None).unwrap();
        assert_eq!(read.text, "a\r\n\x1b[31mred\x1b[0m\r\n");
        assert_eq!(read.start_offset, 0);
        assert_eq!(read.new_offset, read.text.len() as u64);
    }

    #[test]
    fn combined_raw_cursor_increments_and_drains() {
        let mut capture = make_combined_capture();
        capture.append_combined_raw("first\r\n");
        let first = capture.read_combined_since(0, None).unwrap();
        assert_eq!(first.text, "first\r\n");

        capture.append_combined_raw("second\r\n");
        let second = capture.read_combined_since(first.new_offset, None).unwrap();
        assert_eq!(second.text, "second\r\n");
        assert_eq!(second.start_offset, first.new_offset);

        let empty = capture
            .read_combined_since(second.new_offset, None)
            .unwrap();
        assert_eq!(empty.text, "");
        assert_eq!(empty.new_offset, second.new_offset);
    }

    #[test]
    fn combined_raw_offset_clamps_to_char_boundary() {
        let mut capture = make_combined_capture();
        capture.append_combined_raw("α\r\n");

        let mid_char = capture.read_combined_since(1, None).unwrap();
        assert_eq!(mid_char.start_offset, 0);
        assert_eq!(mid_char.text, "α\r\n");

        let beyond = capture.read_combined_since(100, None).unwrap();
        assert_eq!(beyond.text, "");
        assert_eq!(beyond.new_offset, "α\r\n".len() as u64);
    }

    #[test]
    fn combined_raw_tail_cap_skips_forward_on_char_boundary() {
        let mut capture = make_combined_capture();
        capture.append_combined_raw("xβtail");

        let read = capture.read_combined_since(0, Some(5)).unwrap();
        assert_eq!(read.text, "tail");
        assert_eq!(read.start_offset, "xβ".len() as u64);
        assert_eq!(read.new_offset, "xβtail".len() as u64);
    }

    #[test]
    fn combined_raw_tail_cap_never_starts_before_cursor() {
        let mut capture = make_combined_capture();
        capture.append_combined_raw("0123456789");

        let read = capture.read_combined_since(8, Some(4)).unwrap();
        assert_eq!(read.start_offset, 8);
        assert_eq!(read.text, "89");
    }

    #[test]
    fn combined_raw_absent_without_combined_capture() {
        let capture = ExecRawCapture::foreground(ExecProcessId("exec_plain".to_string()));
        assert!(capture.read_combined_since(0, None).is_none());
    }

    #[test]
    fn combined_raw_does_not_leak_into_stream_captures() {
        let mut capture = make_combined_capture();
        capture.append(&ExecOutputStream::Combined, "clean");
        capture.append_combined_raw("raw\r\n");

        let output = capture.read();
        assert_eq!(output.stdout, "clean");
        assert_eq!(output.stderr, "");
        let raw = capture.read_combined_since(0, None).unwrap();
        assert_eq!(raw.text, "raw\r\n");
    }
}
