use std::path::PathBuf;
use std::sync::Arc;

use tokenizers::Tokenizer;

use refact_ast::ast::chunk_utils::{count_text_tokens_with_fallback, get_chunks};
use refact_core::vecdb_types::SplitResult;

pub const LINES_OVERLAP: usize = 3;

pub struct FileSplitter {
    soft_window: usize,
}

impl FileSplitter {
    pub fn new(window_size: usize) -> Self {
        Self { soft_window: window_size }
    }

    pub async fn vectorization_split(
        &self,
        text: &str,
        path: &PathBuf,
        tokenizer: Option<Arc<Tokenizer>>,
        tokens_limit: usize,
    ) -> Result<Vec<SplitResult>, String> {
        let mut chunks = Vec::new();
        let mut lines_accumulator: Vec<&str> = Default::default();
        let mut token_n_accumulator = 0;
        let mut top_row: i32 = -1;
        let lines = text.split('\n').collect::<Vec<_>>();
        for (line_idx, line) in lines.iter().enumerate() {
            let text_orig_tok_n = count_text_tokens_with_fallback(tokenizer.clone(), line);
            if top_row == -1 && text_orig_tok_n != 0 {
                top_row = line_idx as i32;
            }
            if top_row == -1 {
                continue;
            }
            if token_n_accumulator + text_orig_tok_n < self.soft_window {
                lines_accumulator.push(line);
                token_n_accumulator += text_orig_tok_n;
                continue;
            }
            if line.is_empty() {
                let _line = lines_accumulator.join("\n");
                let chunks_ = get_chunks(
                    &_line,
                    path,
                    &"".to_string(),
                    (top_row as usize, line_idx - 1),
                    tokenizer.clone(),
                    tokens_limit,
                    LINES_OVERLAP,
                    false,
                );
                chunks.extend(chunks_);
                lines_accumulator.clear();
                token_n_accumulator = 0;
                top_row = -1;
            } else {
                lines_accumulator.push(line);
                token_n_accumulator += text_orig_tok_n;
            }
        }
        if !lines_accumulator.is_empty() {
            let _line = lines_accumulator.join("\n");
            let chunks_ = get_chunks(
                &_line,
                path,
                &"".to_string(),
                (top_row as usize, lines.len() - 1),
                tokenizer.clone(),
                tokens_limit,
                LINES_OVERLAP,
                false,
            );
            chunks.extend(chunks_);
        }
        Ok(chunks)
    }
}
