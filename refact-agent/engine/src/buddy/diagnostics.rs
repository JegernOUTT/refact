use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

pub use refact_buddy_core::diagnostics::{
    collect_diagnostics_from_error, diagnostic_id, diagnostic_signature, DiagnosticContext,
    DiagnosticSeverity,
};
pub(crate) use refact_buddy_core::diagnostics::classify_error;

use crate::global_context::GlobalContext;

pub async fn collect_diagnostics(
    _gcx: Arc<ARwLock<GlobalContext>>,
    error: &str,
) -> DiagnosticContext {
    collect_diagnostics_from_error(error)
}
