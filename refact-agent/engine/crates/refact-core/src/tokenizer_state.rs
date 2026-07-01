use std::collections::HashMap;
use std::sync::Arc;

use tokenizers::Tokenizer;
use tokio::sync::Mutex as AMutex;

#[derive(Clone)]
pub struct TokenizerState {
    pub map: HashMap<String, Option<Arc<Tokenizer>>>,
    pub download_lock: Arc<AMutex<bool>>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Mutex as AMutex;

    use super::TokenizerState;

    #[test]
    fn tokenizer_state_can_start_empty() {
        let state = TokenizerState {
            map: HashMap::new(),
            download_lock: Arc::new(AMutex::new(false)),
        };

        assert!(state.map.is_empty());
    }
}
