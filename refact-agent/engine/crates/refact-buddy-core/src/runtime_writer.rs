use std::path::PathBuf;

use tokio::sync::mpsc;
use tracing::warn;

use crate::runtime_queue::RuntimeQueue;
use crate::storage::{append_runtime_record, compact_runtime_queue, RuntimeQueueRecord};

/// Single producer/consumer for the runtime_queue.jsonl log. Funneling all
/// mutations through one task gives us a strict total order on disk that
/// matches the in-memory mutation order, which is what makes restart-replay
/// correct in the face of concurrent backend events.
#[derive(Debug)]
pub enum RuntimeQueueWriteOp {
    Append(RuntimeQueueRecord),
    Compact(RuntimeQueue),
}

pub async fn run_runtime_queue_writer(
    project_root: PathBuf,
    mut rx: mpsc::UnboundedReceiver<RuntimeQueueWriteOp>,
) {
    while let Some(op) = rx.recv().await {
        match op {
            RuntimeQueueWriteOp::Append(record) => {
                if let Err(err) = append_runtime_record(&project_root, &record).await {
                    warn!("buddy: failed to persist runtime queue record: {}", err);
                }
            }
            RuntimeQueueWriteOp::Compact(queue) => {
                if let Err(err) = compact_runtime_queue(&project_root, &queue).await {
                    warn!("buddy: failed to compact runtime queue: {}", err);
                }
            }
        }
    }
}
