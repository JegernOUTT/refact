use std::collections::HashMap;

pub struct AtCommandsPreviewCache {
    pub cache: HashMap<String, String>,
}

impl AtCommandsPreviewCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.cache.get(key).cloned()
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.cache.insert(key.clone(), value);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}
