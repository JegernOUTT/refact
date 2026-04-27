/// Build a `BuddyPulse` snapshot for the given project.
///
/// Stub for T-7 — returns a default pulse with `generated_at` set to now.
/// Real metric collection and sub-pulse population will be wired in T-7.
pub async fn build_pulse(
    _gcx: std::sync::Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
    _project_root: &std::path::Path,
    _fact_store: &crate::buddy::facts::FactStore,
) -> crate::buddy::types::BuddyPulse {
    let mut p = crate::buddy::types::BuddyPulse::default();
    p.generated_at = Some(chrono::Utc::now());
    p
}
