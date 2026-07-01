use std::sync::Arc;

use serde::{Deserialize, Serialize};

use refact_codegraph::Counts;

use crate::global_context::GlobalContext;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CodeGraphStatus {
    pub counts: Counts,
    pub queued: usize,
    pub state: String,
}

pub async fn get_codegraph_status(gcx: Arc<GlobalContext>) -> Option<CodeGraphStatus> {
    let service = gcx.codegraph.lock().await.clone()?;
    let counts = service.counts().await.unwrap_or_default();
    Some(CodeGraphStatus {
        counts,
        queued: service.queue_len(),
        state: "working".to_string(),
    })
}
