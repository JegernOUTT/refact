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

fn split_line_if_needed(
    line: &str,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
) -> Vec<String> {
    if let Some(tokenizer) = tokenizer {
        tokenizer.encode(line, false).map_or_else(
            |_| split_without_tokenizer(line, tokens_limit),
            |tokens| {
                let ids = tokens.get_ids();
                if ids.len() <= tokens_limit {
                    vec![line.to_string()]
                } else {
                    ids.chunks(tokens_limit)
                        .filter_map(|chunk| tokenizer.decode(chunk, true).ok())
                        .collect()
                }
            },
        )
    } else {
        split_without_tokenizer(line, tokens_limit)
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
) -> Vec<SplitResult> {
    let (top_row, bottom_row) = top_bottom_rows;
    let mut chunks: Vec<SplitResult> = Vec::new();
    let mut accum: VecDeque<(String, usize)> = Default::default();
    let mut current_tok_n = 0;
    let lines = text.split("\n").collect::<Vec<&str>>();

    {
        let mut line_idx: usize = 0;
        while line_idx < lines.len() {
            let line = lines[line_idx];
            let line_tok_n = count_text_tokens_with_fallback(tokenizer.clone(), line);

            if !accum.is_empty() && current_tok_n + line_tok_n > tokens_limit {
                let current_line = accum.iter().map(|(line, _)| line).join("\n");
                let start_line = if use_symbol_range_always {
                    top_row as u64
                } else {
                    accum.front().unwrap().1 as u64
                };
                let end_line = if use_symbol_range_always {
                    bottom_row as u64
                } else {
                    accum.back().unwrap().1 as u64
                };
                for chunked_line in
                    split_line_if_needed(&current_line, tokenizer.clone(), tokens_limit)
                {
                    chunks.push(SplitResult {
                        file_path: file_path.clone(),
                        window_text: chunked_line.clone(),
                        window_text_hash: official_text_hashing_function(&chunked_line),
                        start_line,
                        end_line,
                        symbol_path: symbol_path.clone(),
                    });
                }
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
                let current_line = accum.iter().map(|(line, _)| line).join("\n");
                let start_line = if use_symbol_range_always {
                    top_row as u64
                } else {
                    accum.front().unwrap().1 as u64
                };
                let end_line = if use_symbol_range_always {
                    bottom_row as u64
                } else {
                    accum.back().unwrap().1 as u64
                };
                for chunked_line in
                    split_line_if_needed(&current_line, tokenizer.clone(), tokens_limit)
                {
                    chunks.push(SplitResult {
                        file_path: file_path.clone(),
                        window_text: chunked_line.clone(),
                        window_text_hash: official_text_hashing_function(&chunked_line),
                        start_line,
                        end_line,
                        symbol_path: symbol_path.clone(),
                    });
                }
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
        let current_line = accum.iter().map(|(line, _)| line).join("\n");
        let start_line = if use_symbol_range_always {
            top_row as u64
        } else {
            accum.front().unwrap().1 as u64
        };
        let end_line = if use_symbol_range_always {
            bottom_row as u64
        } else {
            accum.back().unwrap().1 as u64
        };
        for chunked_line in split_line_if_needed(&current_line, tokenizer.clone(), tokens_limit) {
            chunks.push(SplitResult {
                file_path: file_path.clone(),
                window_text: chunked_line.clone(),
                window_text_hash: official_text_hashing_function(&chunked_line),
                start_line,
                end_line,
                symbol_path: symbol_path.clone(),
            });
        }
    }

    chunks
        .into_iter()
        .filter(|c| !c.window_text.is_empty())
        .collect()
}
