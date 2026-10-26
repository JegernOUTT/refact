use std::collections::HashMap;
use std::sync::Arc;

use tokenizers::Tokenizer;
use tokio::sync::Mutex as AMutex;

#[derive(Clone)]
pub struct TokenizerState {
    pub map: HashMap<String, Option<Arc<Tokenizer>>>,
    pub download_lock: Arc<AMutex<bool>>,
}
