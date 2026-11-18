use std::sync::Arc;
use tokenizers::Tokenizer;

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
