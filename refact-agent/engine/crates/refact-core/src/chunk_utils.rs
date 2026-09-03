use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools;
use ropey::Rope;
use tokenizers::Tokenizer;

use crate::vecdb_types::SplitResult;

pub fn official_text_hashing_function(s: &str) -> String {
    let digest = md5::compute(s);
    format!("{:x}", digest)
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4 + 1
}

pub fn count_text_tokens(tokenizer: Option<Arc<Tokenizer>>, text: &str) -> Result<usize, String> {
    match tokenizer {
        Some(tokenizer) => match tokenizer.encode_fast(text, false) {
            Ok(tokens) => Ok(tokens.len()),
            Err(e) => Err(format!("Encoding error: {e}")),
        },
        None => Ok(estimate_tokens(text)),
    }
}

pub fn count_text_tokens_with_fallback(tokenizer: Option<Arc<Tokenizer>>, text: &str) -> usize {
    count_text_tokens(tokenizer, text).unwrap_or_else(|_| estimate_tokens(text))
}

#[derive(Debug, Clone, Default)]
pub struct ChunkSplit {
    pub chunks: Vec<SplitResult>,
    pub dropped_chunks: usize,
}

impl ChunkSplit {
    pub fn drop_notice(&self) -> Option<String> {
        if self.dropped_chunks == 0 {
            return None;
        }
        Some(format!(
            "chunking incomplete: {} of {} chunks were dropped because the tokenizer failed to \
             decode them, the affected text is not in the index",
            self.dropped_chunks,
            self.dropped_chunks + self.chunks.len()
        ))
    }
}

#[derive(Debug, Default)]
struct SplitLines {
    lines: Vec<String>,
    dropped: usize,
}

fn split_line_if_needed(
    line: &str,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
) -> SplitLines {
    if let Some(tokenizer) = tokenizer {
        tokenizer.encode(line, false).map_or_else(
            |_| SplitLines {
                lines: split_without_tokenizer(line, tokens_limit),
                dropped: 0,
            },
            |tokens| {
                let ids = tokens.get_ids();
                if ids.len() <= tokens_limit {
                    return SplitLines {
                        lines: vec![line.to_string()],
                        dropped: 0,
                    };
                }
                let mut lines = Vec::new();
                let mut dropped = 0;
                for chunk in ids.chunks(tokens_limit) {
                    match tokenizer.decode(chunk, true) {
                        Ok(decoded) => lines.push(decoded),
                        Err(_) => dropped += 1,
                    }
                }
                SplitLines { lines, dropped }
            },
        )
    } else {
        SplitLines {
            lines: split_without_tokenizer(line, tokens_limit),
            dropped: 0,
        }
    }
}

fn split_without_tokenizer(line: &str, tokens_limit: usize) -> Vec<String> {
    if count_text_tokens(None, line).is_ok_and(|tokens| tokens <= tokens_limit) {
        vec![line.to_string()]
    } else {
        Rope::from_str(line)
            .chars()
            .collect::<Vec<_>>()
            .chunks(tokens_limit)
            .map(|chunk| chunk.iter().collect())
            .collect()
    }
}

pub fn get_chunks(
    text: &String,
    file_path: &PathBuf,
    symbol_path: &String,
    top_bottom_rows: (usize, usize),
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
    intersection_lines: usize,
    use_symbol_range_always: bool,
) -> ChunkSplit {
    let (top_row, bottom_row) = top_bottom_rows;
    let mut chunks: Vec<SplitResult> = Vec::new();
    let mut dropped_chunks = 0usize;
    let mut accum: VecDeque<(String, usize)> = Default::default();
    let mut current_tok_n = 0;
    let lines = text.split("\n").collect::<Vec<&str>>();

    let push_window = |chunks: &mut Vec<SplitResult>,
                       dropped_chunks: &mut usize,
                       accum: &VecDeque<(String, usize)>| {
        let current_line = accum.iter().map(|(line, _)| line).join("\n");
        let start_line = match (use_symbol_range_always, accum.front()) {
            (false, Some((_, row))) => *row as u64,
            _ => top_row as u64,
        };
        let end_line = match (use_symbol_range_always, accum.back()) {
            (false, Some((_, row))) => *row as u64,
            _ => bottom_row as u64,
        };
        let split = split_line_if_needed(&current_line, tokenizer.clone(), tokens_limit);
        *dropped_chunks += split.dropped;
        for chunked_line in split.lines {
            chunks.push(SplitResult {
                file_path: file_path.clone(),
                window_text: chunked_line.clone(),
                window_text_hash: official_text_hashing_function(&chunked_line),
                start_line,
                end_line,
                symbol_path: symbol_path.clone(),
            });
        }
    };

    {
        let mut line_idx: usize = 0;
        while line_idx < lines.len() {
            let line = lines[line_idx];
            let line_tok_n = count_text_tokens_with_fallback(tokenizer.clone(), line);

            if !accum.is_empty() && current_tok_n + line_tok_n > tokens_limit {
                push_window(&mut chunks, &mut dropped_chunks, &accum);
                accum.clear();
                current_tok_n = 0;
                if intersection_lines > 0 {
                    line_idx = line_idx.saturating_sub(intersection_lines);
                }
            } else {
                current_tok_n += line_tok_n;
                accum.push_back((line.to_string(), line_idx + top_row));
                line_idx += 1;
            }
        }
    }

    if !accum.is_empty() {
        let mut line_idx: i64 = (lines.len() - 1) as i64;
        accum.clear();
        current_tok_n = 0;
        while line_idx >= 0 {
            let line = lines[line_idx as usize];
            let text_orig_tok_n = count_text_tokens_with_fallback(tokenizer.clone(), line);
            if !accum.is_empty() && current_tok_n + text_orig_tok_n > tokens_limit {
                push_window(&mut chunks, &mut dropped_chunks, &accum);
                accum.clear();
                break;
            } else {
                current_tok_n += text_orig_tok_n;
                accum.push_front((line.to_string(), line_idx as usize + top_row));
                line_idx -= 1;
            }
        }
    }

    if !accum.is_empty() {
        push_window(&mut chunks, &mut dropped_chunks, &accum);
    }

    ChunkSplit {
        chunks: chunks
            .into_iter()
            .filter(|c| !c.window_text.is_empty())
            .collect(),
        dropped_chunks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wordlevel_tokenizer() -> Arc<Tokenizer> {
        let json = r#"{
            "version": "1.0",
            "truncation": null,
            "padding": null,
            "added_tokens": [],
            "normalizer": null,
            "pre_tokenizer": {"type": "WhitespaceSplit"},
            "post_processor": null,
            "decoder": null,
            "model": {
                "type": "WordLevel",
                "vocab": {"[UNK]": 0, "a": 1, "b": 2, "c": 3},
                "unk_token": "[UNK]"
            }
        }"#;
        Arc::new(Tokenizer::from_bytes(json.as_bytes()).unwrap())
    }

    fn sample_chunk(text: &str) -> SplitResult {
        SplitResult {
            file_path: PathBuf::from("/tmp/file.md"),
            window_text: text.to_string(),
            window_text_hash: official_text_hashing_function(text),
            start_line: 0,
            end_line: 0,
            symbol_path: String::new(),
        }
    }

    #[test]
    fn drop_notice_is_none_at_zero_dropped() {
        let split = ChunkSplit {
            chunks: vec![sample_chunk("kept")],
            dropped_chunks: 0,
        };
        assert!(split.drop_notice().is_none());

        let empty = ChunkSplit::default();
        assert!(empty.drop_notice().is_none());
    }

    #[test]
    fn drop_notice_quantifies_dropped_chunks() {
        let split = ChunkSplit {
            chunks: vec![sample_chunk("kept")],
            dropped_chunks: 2,
        };
        let notice = split.drop_notice().unwrap();
        assert!(notice.contains("chunking incomplete: 2 of 3 chunks were dropped"));
    }

    #[test]
    fn get_chunks_empty_input_yields_no_chunks_and_zero_dropped() {
        let split = get_chunks(
            &String::new(),
            &PathBuf::from("/tmp/file.md"),
            &String::new(),
            (0, 0),
            None,
            10,
            0,
            false,
        );
        assert!(split.chunks.is_empty());
        assert_eq!(split.dropped_chunks, 0);
        assert!(split.drop_notice().is_none());
    }

    #[test]
    fn get_chunks_without_tokenizer_splits_long_line_without_drops() {
        let split = get_chunks(
            &"abcdefgh".repeat(10),
            &PathBuf::from("/tmp/file.md"),
            &String::new(),
            (0, 0),
            None,
            4,
            0,
            false,
        );
        assert!(!split.chunks.is_empty());
        assert_eq!(split.dropped_chunks, 0);
        assert!(split.drop_notice().is_none());
    }

    #[test]
    fn get_chunks_with_tokenizer_decodes_oversized_line_without_drops() {
        let tokenizer = wordlevel_tokenizer();
        let split = get_chunks(
            &"a b c a b c".to_string(),
            &PathBuf::from("/tmp/file.md"),
            &String::new(),
            (0, 0),
            Some(tokenizer),
            2,
            0,
            false,
        );
        assert_eq!(split.dropped_chunks, 0);
        assert!(split.drop_notice().is_none());
        assert!(split.chunks.len() >= 3);
        let rejoined = split
            .chunks
            .iter()
            .map(|chunk| chunk.window_text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined.split_whitespace().count(), 6);
    }

    #[test]
    fn split_lines_dropped_field_flows_into_drop_notice_wording() {
        let split = SplitLines {
            lines: vec!["kept".to_string()],
            dropped: 2,
        };
        let chunk_split = ChunkSplit {
            chunks: split.lines.iter().map(|line| sample_chunk(line)).collect(),
            dropped_chunks: split.dropped,
        };
        assert_eq!(
            chunk_split.drop_notice().unwrap(),
            "chunking incomplete: 2 of 3 chunks were dropped because the tokenizer failed to \
             decode them, the affected text is not in the index"
        );
    }
}
