use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::stats::event::LlmCallEvent;

pub async fn stats_writer_task(
    gcx: Arc<GlobalContext>,
    receiver: tokio::sync::mpsc::Receiver<LlmCallEvent>,
) {
    let stats_dir_fn = Arc::new(move || {
        let gcx = gcx.clone();
        Box::pin(async move { crate::stats::get_stats_dir(gcx).await })
            as std::pin::Pin<Box<dyn std::future::Future<Output = std::path::PathBuf> + Send>>
    });
    refact_stats::writer::stats_writer_task(stats_dir_fn, receiver).await;
}

#[cfg(test)]
mod tests {
    use super::*;
}
