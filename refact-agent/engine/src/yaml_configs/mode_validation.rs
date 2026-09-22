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

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn vecdb_gated_tools() -> HashMap<String, ToolGating> {
        HashMap::from([(
            "search_semantic".to_string(),
            ToolGating {
                capability: "vector database",
                unavailable_reason: "which is not enabled on this engine",
                enablement: "start it with --vecdb",
            },
        )])
    }

    #[test]
    fn unknown_tool_names_reports_only_unregistered_sorted_and_deduped() {
        let registered: HashSet<String> = strings(&["cat", "shell"]).into_iter().collect();
        let declared = strings(&["shell", "glob", "cat", "delegate", "glob"]);

        assert_eq!(
            unknown_tool_names(&declared, &registered, &HashMap::new()),
            strings(&["delegate", "glob"])
        );
    }

    #[test]
    fn unknown_tool_names_is_empty_when_every_declared_tool_exists() {
        let registered: HashSet<String> = strings(&["cat", "shell"]).into_iter().collect();

        assert!(unknown_tool_names(&strings(&["cat"]), &registered, &HashMap::new()).is_empty());
    }

    #[test]
    fn unknown_tool_name_still_reports_as_a_typo() {
        let diagnostics = mode_tool_diagnostics(
            "agent",
            &strings(&["definitely_missing"]),
            &HashSet::new(),
            &HashMap::new(),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].1,
            crate::buddy::diagnostics::DiagnosticSeverity::High
        );
        assert_eq!(
            diagnostics[0].0,
            "Mode 'agent' lists 1 tool name(s) that are not registered, so they are silently unavailable and the model will get \"tool not found\" if it calls them: definitely_missing. Fix the spelling or remove them from the mode YAML."
        );
    }

    #[test]
    fn vecdb_gated_tool_reports_as_gated_not_typo() {
        for mode_id in ["agent", "task_planner", "task_agent"] {
            let diagnostics = mode_tool_diagnostics(
                mode_id,
                &strings(&["search_semantic"]),
                &HashSet::new(),
                &vecdb_gated_tools(),
            );

            assert_eq!(diagnostics.len(), 1);
            assert_eq!(
                diagnostics[0].1,
                crate::buddy::diagnostics::DiagnosticSeverity::Low
            );
            assert!(diagnostics[0].0.contains("search_semantic"));
            assert!(diagnostics[0].0.contains("vector database"));
            assert!(diagnostics[0].0.contains("start it with --vecdb"));
            assert!(diagnostics[0].0.contains("This is not a typo"));
            assert!(!diagnostics[0].0.contains("Fix the spelling"));
        }
    }

    #[test]
    fn vecdb_enabled_produces_no_diagnostic_for_search_semantic() {
        let registered = HashSet::from(["search_semantic".to_string()]);

        assert!(mode_tool_diagnostics(
            "agent",
            &strings(&["search_semantic"]),
            &registered,
            &HashMap::new(),
        )
        .is_empty());
    }

    #[test]
    fn mixed_mode_reports_each_category_separately() {
        let diagnostics = mode_tool_diagnostics(
            "agent",
            &strings(&["search_semantic", "definitely_missing"]),
            &HashSet::new(),
            &vecdb_gated_tools(),
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().any(|(message, severity)| {
            *severity == crate::buddy::diagnostics::DiagnosticSeverity::High
                && message.contains("definitely_missing")
                && message.contains("Fix the spelling")
        }));
        assert!(diagnostics.iter().any(|(message, severity)| {
            *severity == crate::buddy::diagnostics::DiagnosticSeverity::Low
                && message.contains("search_semantic")
                && message.contains("This is not a typo")
                && !message.contains("Fix the spelling")
        }));
    }

    #[test]
    fn dropped_tool_names_lists_base_tools_missing_after_override() {
        let base = strings(&["cat", "delegate", "glob", "shell"]);
        let resolved = strings(&["cat", "shell"]);

        assert_eq!(
            dropped_tool_names(&base, &resolved),
            strings(&["delegate", "glob"])
        );
    }

    #[test]
    fn dropped_tool_names_is_empty_when_override_only_adds() {
        let base = strings(&["cat"]);
        let resolved = strings(&["cat", "glob"]);

        assert!(dropped_tool_names(&base, &resolved).is_empty());
    }

    #[test]
    fn summarize_tools_truncates_long_lists() {
        let many: Vec<String> = (0..MAX_LISTED_TOOLS + 3)
            .map(|i| format!("tool{}", i))
            .collect();
        let summary = summarize_tools(&many);

        assert!(summary.ends_with("and 3 more"));
        assert!(summary.contains("tool0"));
        assert!(!summary.contains("tool14"));
    }

    #[test]
    fn claim_first_report_deduplicates_repeated_problems() {
        let key = format!("test-key-{}", uuid::Uuid::new_v4());

        assert!(claim_first_report(key.clone()));
        assert!(!claim_first_report(key));
    }
}
