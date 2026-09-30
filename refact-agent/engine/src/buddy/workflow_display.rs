use refact_buddy_core::operator_types::WorkItem;

use super::operator_store::{OperatorStore, OperatorStoreError, Stored};

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

pub(crate) fn workflow_interrupted_title(workflow: &str) -> String {
    format!("{} — interrupted", workflow_display_name(workflow))
}

pub(crate) fn backfill_workflow_display(
    store: &mut OperatorStore,
) -> Result<usize, OperatorStoreError> {
    let mut after = None;
    let mut candidates: Vec<Stored<WorkItem>> = Vec::new();
    loop {
        let page = store.list_work_items(after, 200)?;
        candidates.extend(page.entries.into_iter().filter(|item| {
            item.value.subject.resource_kind == "workflow"
                && item.value.project == *store.identity()
        }));
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }

    let mut updated_count = 0;
    for stored in candidates {
        let mut updated = stored.value;
        let workflow = updated.subject.resource_id.as_str();
        let expected_scope = workflow_display_scope(workflow);
        let legacy_title = format!("{workflow} — interrupted run");
        let mut changed = false;
        if updated.scope.as_deref() != Some(expected_scope.as_str()) {
            updated.scope = Some(expected_scope);
            changed = true;
        }
        if updated.title == legacy_title {
            updated.title = workflow_interrupted_title(workflow);
            changed = true;
        }
        if changed {
            store.upsert_work_item(&updated, Some(stored.revision))?;
            updated_count += 1;
        }
    }
    Ok(updated_count)
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
