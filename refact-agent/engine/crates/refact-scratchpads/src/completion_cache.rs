use refact_core::chat_types::CodeCompletionPost;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use ropey::Rope;

const CACHE_ENTRIES: usize = 500;
const CACHE_KEY_CHARS: usize = 5000;

#[derive(Debug, Clone)]
pub struct CompletionSaveToCache {
    pub cache_arc: Arc<StdRwLock<CompletionCache>>,
    pub cache_key: (String, String),
    pub completion0_text: String,
    pub completion0_finish_reason: String,
    pub model: String,
}

impl CompletionSaveToCache {
    pub fn new(cache_arc: Arc<StdRwLock<CompletionCache>>, post: &CodeCompletionPost) -> Self {
        CompletionSaveToCache {
            cache_arc: cache_arc.clone(),
            cache_key: cache_key_from_post(post),
            completion0_text: String::new(),
            completion0_finish_reason: String::new(),
            model: post.model.clone(),
        }
    }
}

#[derive(Debug)]
pub struct CompletionCache {
    pub map: HashMap<(String, String), serde_json::Value>,
    pub in_added_order: Vec<(String, String)>,
    pub generation: u64,
}

impl CompletionCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            in_added_order: Vec::new(),
            generation: 0,
        }
    }

    pub fn bump_generation(&mut self) -> u64 {
        self.map.clear();
        self.in_added_order.clear();
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

pub fn cache_generation(cache: Arc<StdRwLock<CompletionCache>>) -> u64 {
    cache.read().unwrap().generation()
}

pub fn cache_bump_generation(cache: Arc<StdRwLock<CompletionCache>>) -> u64 {
    cache.write().unwrap().bump_generation()
}

pub fn cache_get(
    cache: Arc<StdRwLock<CompletionCache>>,
    key: (String, String),
) -> Option<serde_json::Value> {
    let cache_locked = cache.write().unwrap();
    if let Some(value) = cache_locked.map.get(&key) {
        return Some(value.clone());
    }
    None
}

pub fn cache_put(
    cache: Arc<StdRwLock<CompletionCache>>,
    new_key: (String, String),
    value: serde_json::Value,
) {
    let mut cache_locked = cache.write().unwrap();
    while cache_locked.in_added_order.len() > CACHE_ENTRIES {
        let old_key = cache_locked.in_added_order.remove(0);
        cache_locked.map.remove(&old_key);
    }
    let mut new_key_copy = new_key.clone();
    let k0_chars = new_key_copy.0.chars();
    if k0_chars.clone().count() > CACHE_KEY_CHARS {
        new_key_copy.0 = k0_chars
            .clone()
            .skip(k0_chars.count() - CACHE_KEY_CHARS)
            .collect();
    }
    cache_locked
        .map
        .entry(new_key_copy.clone())
        .or_insert(value);
    cache_locked.in_added_order.push(new_key_copy.clone());
}

pub fn cache_key_from_post(post: &CodeCompletionPost) -> (String, String) {
    let text_maybe = post.inputs.sources.get(&post.inputs.cursor.file);
    if let None = text_maybe {
        return (
            format!(
                "dummy1-{}:{}",
                post.inputs.cursor.line, post.inputs.cursor.character
            ),
            "".to_string(),
        );
    }
    let rope = Rope::from_str(text_maybe.unwrap());
    let cursor_line_maybe = rope.get_line(post.inputs.cursor.line as usize);
    if let None = cursor_line_maybe {
        return (
            format!(
                "dummy2-{}:{}",
                post.inputs.cursor.line, post.inputs.cursor.character
            ),
            "".to_string(),
        );
    }
    let mut cursor_line = cursor_line_maybe.unwrap();
    let cpos = post.inputs.cursor.character as usize;
    if cpos < cursor_line.len_chars() {
        cursor_line = cursor_line.slice(..cpos);
    }
    let mut before_iter = rope.lines_at(post.inputs.cursor.line as usize).reversed();
    let mut linesvec = Vec::<String>::new();
    let mut bytes = 0;
    loop {
        let line_maybe = before_iter.next();
        if let None = line_maybe {
            break;
        }
        let line = line_maybe.unwrap();
        let line_str = line.to_string();
        linesvec.push(line_str.replace("\r", ""));
        bytes += line.len_chars();
        if bytes > CACHE_KEY_CHARS {
            break;
        }
    }
    linesvec.reverse();
    let mut key = "".to_string();
    key.push_str(&linesvec.join(""));
    key.push_str(&cursor_line.to_string());
    let chars = key.chars();

    if chars.clone().count() > CACHE_KEY_CHARS {
        key = chars.skip(key.len() - CACHE_KEY_CHARS).collect();
    }
    return (key, cache_part2_from_post(post));
}

pub fn cache_part2_from_post(post: &CodeCompletionPost) -> String {
    let line_mode = if post.inputs.multiline {
        "multiline".to_string()
    } else {
        "singleline".to_string()
    };
    serde_json::json!({
        "generation": post.cache_generation,
        "salt": post.cache_salt,
        "model": post.model,
        "line_mode": line_mode,
        "parameters": post.parameters,
        "use_ast": post.use_ast,
        "use_vecdb": post.use_vecdb,
        "rag_tokens_n": post.rag_tokens_n,
    })
    .to_string()
}

impl Drop for CompletionSaveToCache {
    fn drop(&mut self) {
        if self.completion0_finish_reason.is_empty() {
            return;
        }
        let mut believe_chars = self.completion0_text.len();
        if self.completion0_finish_reason == "length" {
            believe_chars = believe_chars.checked_sub(10).unwrap_or(0);
        } else {
            believe_chars += 1;
        }
        for char_num in 0..believe_chars {
            let code_completion_ahead: String =
                self.completion0_text.chars().skip(char_num).collect();
            let cache_key_ahead: (String, String) = (
                self.cache_key.0.clone()
                    + &self
                        .completion0_text
                        .chars()
                        .take(char_num)
                        .collect::<String>(),
                self.cache_key.1.clone(),
            );
            cache_put(
                self.cache_arc.clone(),
                cache_key_ahead,
                serde_json::json!(
                    {
                        "choices": [{
                            "index": 0,
                            "code_completion": code_completion_ahead,
                            "finish_reason": self.completion0_finish_reason,
                        }],
                        "model": self.model,
                        "cached": true,
                    }
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{CodeCompletionInputs, CursorPosition, SamplingParameters};

    fn test_post() -> CodeCompletionPost {
        let mut sources = HashMap::new();
        sources.insert(
            "/tmp/main.rs".to_string(),
            "fn main() {\n    pri\n}".to_string(),
        );
        CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources,
                cursor: CursorPosition {
                    file: "/tmp/main.rs".to_string(),
                    line: 1,
                    character: 7,
                },
                multiline: false,
            },
            parameters: SamplingParameters {
                max_new_tokens: 50,
                ..Default::default()
            },
            model: "custom/model".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: false,
            rag_tokens_n: 128,
            cache_salt: "salt-a".to_string(),
            cache_generation: 1,
        }
    }

    #[test]
    fn completion_cache_key_changes_with_generation() {
        let mut post = test_post();
        let key1 = cache_key_from_post(&post);
        post.cache_generation += 1;
        let key2 = cache_key_from_post(&post);

        assert_ne!(key1.1, key2.1);
    }

    #[test]
    fn completion_cache_key_changes_with_model_salt_and_rag_settings() {
        let mut post = test_post();
        let key1 = cache_key_from_post(&post);
        post.cache_salt = "salt-b".to_string();
        let key2 = cache_key_from_post(&post);
        post.cache_salt = "salt-a".to_string();
        post.use_vecdb = true;
        let key3 = cache_key_from_post(&post);

        assert_ne!(key1.1, key2.1);
        assert_ne!(key1.1, key3.1);
    }

    #[test]
    fn completion_cache_bump_clears_entries() {
        let cache = Arc::new(StdRwLock::new(CompletionCache::new()));
        let post = test_post();
        let key = cache_key_from_post(&post);
        cache_put(cache.clone(), key.clone(), serde_json::json!({"value": 1}));

        assert!(cache_get(cache.clone(), key).is_some());
        assert_eq!(cache_bump_generation(cache.clone()), 1);
        assert!(cache.read().unwrap().map.is_empty());
        assert!(cache.read().unwrap().in_added_order.is_empty());
    }
}
