pub use refact_buddy_core::diagnostics::{
    collect_diagnostics_from_error, diagnostic_id, diagnostic_signature, DiagnosticContext,
    DiagnosticSeverity,
};
pub(crate) use refact_buddy_core::diagnostics::classify_error;

use crate::app_state::AppState;

pub async fn collect_diagnostics(_gcx: AppState, error: &str) -> DiagnosticContext {
    collect_diagnostics_from_error(error)
}

pub async fn collect_diagnostics_with_model(
    _gcx: AppState,
    error: &str,
    model_id: Option<String>,
) -> DiagnosticContext {
    let mut ctx = collect_diagnostics_from_error(error);
    ctx.model_id = model_id;
    ctx
}
