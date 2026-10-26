use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::global_context::GlobalContext;
use crate::tools::tools_list::ToolGating;

const BUDDY_ERROR_TYPE: &str = "mode_config";
const BUDDY_SOURCE: &str = "yaml_configs/mode_validation.rs";
const MAX_LISTED_TOOLS: usize = 12;

fn reported_problems() -> &'static Mutex<HashSet<String>> {
    static REPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REPORTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn claim_first_report(key: String) -> bool {
    match reported_problems().lock() {
        Ok(mut seen) => seen.insert(key),
        Err(_) => false,
    }
}

pub fn dropped_tool_names(base_tools: &[String], resolved_tools: &[String]) -> Vec<String> {
    let kept: HashSet<&str> = resolved_tools.iter().map(|name| name.as_str()).collect();
    base_tools
        .iter()
        .filter(|name| !kept.contains(name.as_str()))
        .cloned()
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

fn summarize_tools(names: &[String]) -> String {
    if names.len() <= MAX_LISTED_TOOLS {
        return names.join(", ");
    }
    format!(
        "{}, and {} more",
        names[..MAX_LISTED_TOOLS].join(", "),
        names.len() - MAX_LISTED_TOOLS
    )
}

pub fn unknown_tool_names(
    declared: &[String],
    registered: &HashSet<String>,
    gated: &HashMap<String, ToolGating>,
) -> Vec<String> {
    declared
        .iter()
        .filter(|name| !registered.contains(name.as_str()) && !gated.contains_key(name.as_str()))
        .cloned()
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

pub fn gated_tool_names(
    declared: &[String],
    gated: &HashMap<String, ToolGating>,
) -> BTreeMap<(String, String), Vec<String>> {
    let mut grouped = BTreeMap::new();
    for name in declared.iter().collect::<BTreeSet<_>>() {
        let Some(gating) = gated.get(name.as_str()) else {
            continue;
        };
        grouped
            .entry((
                gating.capability.to_string(),
                format!("{} ({})", gating.unavailable_reason, gating.enablement),
            ))
            .or_insert_with(Vec::new)
            .push(name.clone());
    }
    grouped
}

pub fn mode_tool_diagnostics(
    mode_id: &str,
    declared: &[String],
    registered: &HashSet<String>,
    gated: &HashMap<String, ToolGating>,
) -> Vec<(String, crate::buddy::diagnostics::DiagnosticSeverity)> {
    let mut diagnostics = Vec::new();
    let unknown = unknown_tool_names(declared, registered, gated);
    if !unknown.is_empty() {
        diagnostics.push((
            format!(
                "Mode '{}' lists {} tool name(s) that are not registered, so they are silently unavailable and the model will get \"tool not found\" if it calls them: {}. Fix the spelling or remove them from the mode YAML.",
                mode_id,
                unknown.len(),
                summarize_tools(&unknown)
            ),
            crate::buddy::diagnostics::DiagnosticSeverity::High,
        ));
    }
    for ((capability, enablement), names) in gated_tool_names(declared, gated) {
        diagnostics.push((
            format!(
                "Mode '{}' lists {} tool{} that {} currently unavailable: {} need{} the {}, {}. This is not a typo — leave the mode YAML alone.",
                mode_id,
                names.len(),
                if names.len() == 1 { "" } else { "s" },
                if names.len() == 1 {
                    "exists but is"
                } else {
                    "exist but are"
                },
                summarize_tools(&names),
                if names.len() == 1 { "s" } else { "" },
                capability,
                enablement,
            ),
            crate::buddy::diagnostics::DiagnosticSeverity::Low,
        ));
    }
    diagnostics
}

pub async fn warn_unavailable_mode_tools(
    gcx: Arc<GlobalContext>,
    mode_id: &str,
    declared: &[String],
    registered: &HashSet<String>,
    gated: &HashMap<String, ToolGating>,
) {
    for (message, severity) in mode_tool_diagnostics(mode_id, declared, registered, gated) {
        let key = format!("mode-tools:{}:{}", mode_id, message);
        if !claim_first_report(key) {
            continue;
        }
        match severity {
            crate::buddy::diagnostics::DiagnosticSeverity::High => tracing::warn!("{}", message),
            _ => tracing::info!("{}", message),
        }
        notify_buddy(gcx.clone(), &message, severity).await;
    }
}

pub async fn warn_override_tool_drift(
    gcx: Arc<GlobalContext>,
    mode_id: &str,
    overlay_id: &str,
    dropped: &[String],
) {
    if dropped.is_empty() {
        return;
    }
    let listed = summarize_tools(dropped);
    if !claim_first_report(format!(
        "override-drift:{}:{}:{}",
        mode_id,
        overlay_id,
        dropped.join(",")
    )) {
        return;
    }
    let message = format!(
        "Mode overlay '{}' uses tools_replace on base mode '{}' and drops {} tool(s) the base still provides: {}. The overlay prompt may still advertise them, which produces \"tool not found\" at runtime. Prefer tools_add/tools_remove so the overlay keeps inheriting new tools.",
        overlay_id,
        mode_id,
        dropped.len(),
        listed
    );
    tracing::warn!("{}", message);
    notify_buddy(
        gcx,
        &message,
        crate::buddy::diagnostics::DiagnosticSeverity::High,
    )
    .await;
}

async fn notify_buddy(
    gcx: Arc<GlobalContext>,
    message: &str,
    severity: crate::buddy::diagnostics::DiagnosticSeverity,
) {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let buddy = app.buddy.buddy.clone();
    let mut service = buddy.lock().await;
    if let Some(service) = service.as_mut() {
        service.add_diagnostic(crate::buddy::diagnostics::DiagnosticContext {
            error_type: BUDDY_ERROR_TYPE.to_string(),
            error_message: message.to_string(),
            source_file: Some(BUDDY_SOURCE.to_string()),
            tool_name: None,
            chat_id: None,
            model_id: None,
            collected_at: chrono::Utc::now().to_rfc3339(),
            severity,
            occurrences: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
