use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Clone)]
pub struct CodeLensResponse {
    pub success: u8,
    pub code_lens: Vec<CodeLensOutput>,
}

#[derive(Serialize, Clone)]
pub struct CodeLensOutput {
    pub spath: String,
    pub line1: usize,
    pub line2: usize,
    pub debug_string: Option<String>,
}

pub struct CodeLensCacheEntry {
    pub response: CodeLensResponse,
    pub ts: f64,
}

#[derive(Default)]
pub struct CodeLensCache {
    pub store: HashMap<String, CodeLensCacheEntry>,
}

impl CodeLensCache {
    pub fn clean_up_old_entries(&mut self, now: f64) {
        self.store.retain(|_, entry| now - entry.ts <= 600.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> CodeLensResponse {
        CodeLensResponse {
            success: 1,
            code_lens: Vec::new(),
        }
    }

    #[test]
    fn clean_up_old_entries_drops_entries_older_than_600s() {
        let mut cache = CodeLensCache::default();
        cache.store.insert(
            "fresh".to_string(),
            CodeLensCacheEntry {
                response: response(),
                ts: 500.0,
            },
        );
        cache.store.insert(
            "boundary".to_string(),
            CodeLensCacheEntry {
                response: response(),
                ts: 400.0,
            },
        );
        cache.store.insert(
            "old".to_string(),
            CodeLensCacheEntry {
                response: response(),
                ts: 399.9,
            },
        );

        cache.clean_up_old_entries(1000.0);

        assert!(cache.store.contains_key("fresh"));
        assert!(cache.store.contains_key("boundary"));
        assert!(!cache.store.contains_key("old"));
    }
}
