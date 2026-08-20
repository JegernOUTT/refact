use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::global_context::GlobalContext;

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

pub fn unknown_tool_names(declared: &[String], registered: &HashSet<String>) -> Vec<String> {
    declared
        .iter()
        .filter(|name| !registered.contains(name.as_str()))
        .cloned()
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
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

pub async fn warn_unknown_mode_tools(
    gcx: Arc<GlobalContext>,
    mode_id: &str,
    declared: &[String],
    registered: &HashSet<String>,
) {
    let unknown = unknown_tool_names(declared, registered);
    if unknown.is_empty() {
        return;
    }
    let listed = summarize_tools(&unknown);
    if !claim_first_report(format!("unknown-tools:{}:{}", mode_id, unknown.join(","))) {
        return;
    }
    let message = format!(
        "Mode '{}' lists {} tool name(s) that are not registered, so they are silently unavailable and the model will get \"tool not found\" if it calls them: {}. Fix the spelling or remove them from the mode YAML.",
        mode_id,
        unknown.len(),
        listed
    );
    tracing::warn!("{}", message);
    notify_buddy(gcx, &message).await;
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
    notify_buddy(gcx, &message).await;
}

async fn notify_buddy(gcx: Arc<GlobalContext>, message: &str) {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    crate::buddy::actor::report_error_persisted(
        app,
        BUDDY_ERROR_TYPE,
        message,
        Some(BUDDY_SOURCE),
        None,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn unknown_tool_names_reports_only_unregistered_sorted_and_deduped() {
        let registered: HashSet<String> = strings(&["cat", "shell"]).into_iter().collect();
        let declared = strings(&["shell", "glob", "cat", "delegate", "glob"]);

        assert_eq!(
            unknown_tool_names(&declared, &registered),
            strings(&["delegate", "glob"])
        );
    }

    #[test]
    fn unknown_tool_names_is_empty_when_every_declared_tool_exists() {
        let registered: HashSet<String> = strings(&["cat", "shell"]).into_iter().collect();

        assert!(unknown_tool_names(&strings(&["cat"]), &registered).is_empty());
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
