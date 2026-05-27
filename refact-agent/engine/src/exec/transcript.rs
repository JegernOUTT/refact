use std::collections::VecDeque;

use crate::exec::types::{
    current_timestamp_ms, ExecOutputChunk, ExecOutputStream, ExecProcessId, ExecReadResult,
};

pub const DEFAULT_MAX_BYTES: usize = 512 * 1024;

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
}

impl ExecTranscript {
    fn normalize_max_bytes(max_bytes: usize) -> usize {
        max_bytes.max(1)
    }

    pub fn new(process_id: ExecProcessId, max_bytes: usize) -> Self {
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
        }
    }

    pub fn with_default_max(process_id: ExecProcessId) -> Self {
        Self::new(process_id, DEFAULT_MAX_BYTES)
    }

    pub fn append(&mut self, stream: ExecOutputStream, text: String) -> u64 {
        self.append_chunk(stream, text).seq
    }

    pub(crate) fn append_chunk(
        &mut self,
        stream: ExecOutputStream,
        text: String,
    ) -> ExecOutputChunk {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_not_truncated_when_within_limit() {
        let mut t = make_transcript(1024);
        t.append(ExecOutputStream::Stdout, "small".to_string());
        assert!(!t.is_truncated());
        assert_eq!(t.dropped_chunks(), 0);
        assert_eq!(t.dropped_bytes(), 0);
        assert_eq!(t.truncated_chunks(), 0);
    }
}
