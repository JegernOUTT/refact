use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::code_assistant_caps::CodeAssistantCaps;

#[derive(Clone)]
pub struct CapsState {
    pub caps: Option<Arc<CodeAssistantCaps>>,
    pub reading_lock: Arc<AMutex<bool>>,
    pub last_error: String,
    pub last_attempted_ts: u64,
    pub models_dev_startup_refresh_attempted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_state_can_be_constructed_empty() {
        let state = CapsState {
            caps: None,
            reading_lock: Arc::new(AMutex::new(false)),
            last_error: String::new(),
            last_attempted_ts: 0,
            models_dev_startup_refresh_attempted: false,
        };

        assert!(state.caps.is_none());
        assert!(state.last_error.is_empty());
        assert_eq!(state.last_attempted_ts, 0);
        assert!(!state.models_dev_startup_refresh_attempted);
    }
}
