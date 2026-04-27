use axum::Extension;
use axum::response::Result;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::buddy::types::BuddyPulse;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

pub async fn handle_v1_buddy_pulse(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<BuddyPulse>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let pulse = lock.as_ref().map(|svc| svc.pulse.clone()).unwrap_or_default();
    Ok(axum::Json(pulse))
}
