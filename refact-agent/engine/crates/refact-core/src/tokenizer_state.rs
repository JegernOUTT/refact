use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, Weak};

use tokenizers::Tokenizer;
use tokio::sync::Mutex as AMutex;

#[derive(Hash, PartialEq, Eq)]
pub enum TokenizerLoadKey {
    Model(String),
    Cache(PathBuf),
}

#[derive(Clone)]
pub struct TokenizerState {
    pub map: HashMap<String, Option<Arc<Tokenizer>>>,
    pub download_locks: Arc<StdMutex<HashMap<TokenizerLoadKey, Weak<AMutex<()>>>>>,
}

impl TokenizerState {
    pub fn download_lock(&self, model_id: &str) -> Arc<AMutex<()>> {
        self.load_lock(TokenizerLoadKey::Model(model_id.to_string()))
    }

    pub fn cache_lock(&self, path: &Path) -> Arc<AMutex<()>> {
        self.load_lock(TokenizerLoadKey::Cache(path.to_path_buf()))
    }

    fn load_lock(&self, key: TokenizerLoadKey) -> Arc<AMutex<()>> {
        let mut locks = self.download_locks.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() != 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AMutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenizerLoadKey, TokenizerState};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Mutex as AsyncMutex;

    #[test]
    fn cache_lock_reuses_paths_without_colliding_with_model_keys() {
        let state = TokenizerState {
            map: HashMap::new(),
            download_locks: Arc::new(Mutex::new(HashMap::new())),
        };
        let model = state.download_lock("same-path");
        let cache = state.cache_lock(Path::new("same-path"));
        let same = state.cache_lock(Path::new("same-path"));
        assert!(!Arc::ptr_eq(&model, &cache));
        assert!(Arc::ptr_eq(&cache, &same));
    }

    #[test]
    fn download_lock_registry_prunes_dead_entries_and_preserves_live_identity() {
        let state = TokenizerState {
            map: HashMap::new(),
            download_locks: Arc::new(Mutex::new(HashMap::new())),
        };
        let live = state.download_lock("model/live");
        let dead = Arc::new(AsyncMutex::new(()));
        state.download_locks.lock().unwrap().insert(
            TokenizerLoadKey::Model("model/dead".into()),
            Arc::downgrade(&dead),
        );
        drop(dead);

        let same_live = state.download_lock("model/live");
        assert!(Arc::ptr_eq(&live, &same_live));
        let live_key = TokenizerLoadKey::Model("model/live".into());
        let dead_key = TokenizerLoadKey::Model("model/dead".into());
        let registry = state.download_locks.lock().unwrap();
        assert!(registry.contains_key(&live_key));
        assert!(!registry.contains_key(&dead_key));
        assert!(registry.get(&live_key).unwrap().upgrade().is_some());
        drop(registry);

        drop(live);
        drop(same_live);
        let replacement = state.download_lock("model/live");
        assert!(state.download_locks.lock().unwrap()[&live_key]
            .upgrade()
            .is_some());
        drop(replacement);
    }
}
