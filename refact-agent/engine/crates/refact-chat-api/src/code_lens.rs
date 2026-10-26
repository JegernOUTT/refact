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
