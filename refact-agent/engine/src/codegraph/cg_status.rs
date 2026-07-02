use std::sync::Arc;

use serde::{Deserialize, Serialize};

use refact_codegraph::Counts;

use crate::global_context::GlobalContext;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CodeGraphStatus {
    pub counts: Counts,
    pub queued: usize,
    pub state: String,
    pub error: String,
}

pub async fn get_codegraph_status(gcx: Arc<GlobalContext>) -> CodeGraphStatus {
    let service = gcx.codegraph.lock().await.clone();
    let error = gcx.codegraph_error.lock().unwrap().clone();
    let Some(service) = service else {
        let state = if error.is_empty() {
            "turned_off"
        } else {
            "error"
        };
        return CodeGraphStatus {
            counts: Counts::default(),
            queued: 0,
            state: state.to_string(),
            error,
        };
    };

    let queued = service.queue_len();
    let counts = match service.counts().await {
        Ok(counts) => counts,
        Err(err) => {
            let error = if error.is_empty() {
                err
            } else {
                format!("{}; {}", error, err)
            };
            return CodeGraphStatus {
                counts: Counts::default(),
                queued,
                state: "error".to_string(),
                error,
            };
        }
    };

    let state = if !error.is_empty() {
        "error"
    } else if queued > 0 || !service.is_initial_index_done() {
        "indexing"
    } else {
        "working"
    };

    CodeGraphStatus {
        counts,
        queued,
        state: state.to_string(),
        error,
    }
}
