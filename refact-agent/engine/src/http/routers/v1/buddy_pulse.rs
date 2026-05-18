use axum::response::Result;
use axum::extract::State;

use crate::app_state::AppState;
use crate::buddy::types::BuddyPulse;
use crate::custom_error::ScratchError;

pub async fn handle_v1_buddy_pulse(
    State(app): State<AppState>,
) -> Result<axum::Json<BuddyPulse>, ScratchError> {
    let gcx = app.gcx.clone();
    let buddy_arc = gcx.buddy.clone();
    let lock = buddy_arc.lock().await;
    let pulse = lock
        .as_ref()
        .map(|svc| svc.pulse.clone())
        .unwrap_or_default();
    Ok(axum::Json(pulse))
}
