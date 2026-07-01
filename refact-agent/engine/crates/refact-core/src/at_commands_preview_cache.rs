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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_returns_value_and_missing_is_none() {
        let mut cache = AtCommandsPreviewCache::new();

        assert_eq!(cache.get("missing"), None);

        cache.insert("prompt".to_string(), "preview".to_string());

        assert_eq!(cache.get("prompt"), Some("preview".to_string()));
    }
}
