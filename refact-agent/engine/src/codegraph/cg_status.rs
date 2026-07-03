use std::sync::Arc;

use serde::{Deserialize, Serialize};

use refact_codegraph::Counts;

use crate::global_context::GlobalContext;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CodeGraphStatus {
    pub counts: Counts,
    pub queued: usize,
    pub throughput_files_per_min: f64,
    pub eta_seconds: Option<u64>,
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
            throughput_files_per_min: 0.0,
            eta_seconds: None,
            state: state.to_string(),
            error,
        };
    };

    let queued = service.queue_len();
    let throughput_files_per_min = service.throughput_files_per_min();
    let eta_seconds = eta_seconds(queued, throughput_files_per_min);
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
                throughput_files_per_min,
                eta_seconds,
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
        throughput_files_per_min,
        eta_seconds,
        state: state.to_string(),
        error,
    }
}

fn eta_seconds(queued: usize, throughput_files_per_min: f64) -> Option<u64> {
    if queued == 0 || throughput_files_per_min <= f64::EPSILON {
        None
    } else {
        Some(((queued as f64 / throughput_files_per_min) * 60.0).ceil() as u64)
    }
}
