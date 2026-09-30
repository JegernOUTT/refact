pub(crate) fn workflow_display_name(workflow: &str) -> String {
    refact_buddy_core::autonomous_workflows::autonomous_workflow_meta(workflow)
        .map(|meta| meta.title.to_string())
        .or_else(|| {
            let label = refact_buddy_core::workflows::workflow_label(workflow);
            (label != workflow).then(|| capitalize(label))
        })
        .unwrap_or_else(|| super::outcome_runtime::workflow_display_title(workflow))
}

pub(crate) fn workflow_display_scope(workflow: &str) -> String {
    format!("Latest run of {}", workflow_display_name(workflow))
}

fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_display_name_uses_titles_or_human_workflow_labels() {
        assert_eq!(
            workflow_display_name("buddy_dependency_radar"),
            "Dependency Radar"
        );
        assert_eq!(workflow_display_name("memo_extraction"), "Memo extraction");
        assert_eq!(
            workflow_display_scope("memo_extraction"),
            "Latest run of Memo extraction"
        );
    }
}
