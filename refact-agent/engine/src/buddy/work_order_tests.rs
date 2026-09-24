use super::{OperatorStore, Stored};
use crate::buddy::operator_store::OperatorStoreError;
use refact_buddy_core::operator_types::{ArtifactRevision, ProjectIdentity, WorkItem};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn project(root: &Path) -> ProjectIdentity {
    ProjectIdentity {
        project_id: "p".into(),
        canonical_root: fs::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    }
}

fn work(id: &str, title: &str, state: &str, project: &ProjectIdentity) -> WorkItem {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
    ))
    .unwrap();
    let item = &mut value["work_item"];
    item["id"] = serde_json::json!(id);
    item["title"] = serde_json::json!(title);
    item["state"] = serde_json::json!(state);
    item["project"] = serde_json::to_value(project).unwrap();
    serde_json::from_value(item.clone()).unwrap()
}

fn artifact(id: &str, run_id: &str, project: &ProjectIdentity) -> ArtifactRevision {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
    ))
    .unwrap();
    let artifact = &mut value["artifact_revision"];
    artifact["artifact_id"] = serde_json::json!(id);
    artifact["revision"] = serde_json::json!(1);
    artifact["project"] = serde_json::to_value(project).unwrap();
    artifact["source_run_ids"] = serde_json::json!([run_id]);
    artifact["provenance"]["source_run_ids"] = serde_json::json!([run_id]);
    serde_json::from_value(artifact.clone()).unwrap()
}

fn open_store(root: &TempDir) -> (ProjectIdentity, OperatorStore) {
    let project = project(root.path());
    let store = OperatorStore::open(root.path(), &project).unwrap();
    (project, store)
}

fn ids(page: Vec<Stored<WorkItem>>) -> Vec<String> {
    page.into_iter().map(|record| record.value.id).collect()
}

#[test]
fn newest_first_pages_cover_needs_you_once_with_exclusive_cursors() {
    use refact_buddy_core::operator_types::WorkItemState;

    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut expected = Vec::new();
    let mut previous_seq = None;
    for index in 0..120 {
        let (state, state_name) = match index {
            0..=107 => (WorkItemState::NeedsAttention, "needs_attention"),
            108..=110 => (WorkItemState::Working, "working"),
            111..=114 => (WorkItemState::Delivered, "delivered"),
            _ => (WorkItemState::Watching, "watching"),
        };
        let id = format!("work-{index:03}");
        let mut item = work(&id, &format!("Item {index}"), state_name, &project);
        item.state = state;
        let stored = store.upsert_work_item(&item, None).unwrap();
        assert!(previous_seq.is_none_or(|previous| stored.change_seq > previous));
        previous_seq = Some(stored.change_seq);
        if index <= 107 {
            expected.push((id, stored.change_seq));
        }
    }
    expected.reverse();

    let mut cursor = None;
    let mut actual = Vec::new();
    let mut previous = None;
    loop {
        let page = store
            .search_work_items(cursor, 50, Some("needs_you"), None)
            .unwrap();
        assert!(!page.entries.is_empty());
        for entry in &page.entries {
            assert!(previous.is_none_or(|seq| entry.change_seq < seq));
            previous = Some(entry.change_seq);
            actual.push((entry.value.id.clone(), entry.change_seq));
        }
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(actual, expected);
}

#[test]
fn updating_a_returned_item_moves_it_to_page_one_without_repeating_it_later() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut inserted = Vec::new();
    for index in 0..6 {
        let item = work(
            &format!("work-{index}"),
            &format!("Item {index}"),
            "needs_attention",
            &project,
        );
        inserted.push(store.upsert_work_item(&item, None).unwrap());
    }

    let first = store
        .search_work_items(None, 3, Some("needs_you"), None)
        .unwrap();
    let cursor = first.next_after.unwrap();
    let updated_id = first.entries[1].value.id.clone();
    let mut updated = first.entries[1].value.clone();
    updated.title = "Updated after first page".into();
    let changed = store
        .upsert_work_item(&updated, Some(first.entries[1].revision))
        .unwrap();
    assert!(changed.change_seq > inserted.last().unwrap().change_seq);

    let mut cursor = Some(cursor);
    let mut later = Vec::new();
    while let Some(after) = cursor {
        let page = store
            .search_work_items(Some(after), 3, Some("needs_you"), None)
            .unwrap();
        later.extend(ids(page.entries));
        cursor = page.next_after;
    }
    assert!(!later.contains(&updated_id));

    let fresh = store
        .search_work_items(None, 3, Some("needs_you"), None)
        .unwrap();
    assert_eq!(fresh.entries[0].value.id, updated_id);
    assert_eq!(fresh.entries[0].change_seq, changed.change_seq);
}

#[test]
fn read_and_snooze_changes_do_not_reorder_work_items() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut selected = None;
    for index in 0..4 {
        let id = format!("work-{index}");
        let run_id = format!("run-{index}");
        let artifact_id = format!("artifact-{index}");
        let report = artifact(&artifact_id, &run_id, &project);
        store.append_artifact_revision(&report).unwrap();
        let mut item = work(&id, &format!("Item {index}"), "needs_attention", &project);
        item.current_artifact_id = Some(artifact_id.clone());
        item.current_artifact_revision = Some(1);
        store.upsert_work_item(&item, None).unwrap();
        if index == 1 {
            selected = Some((id, artifact_id));
        }
    }
    let before = ids(store
        .search_work_items(None, 10, Some("needs_you"), None)
        .unwrap()
        .entries);
    let (id, artifact_id) = selected.unwrap();
    store.mark_work_read(&id, &artifact_id, 1).unwrap();
    store.snooze_work(&id, &artifact_id, 1, Some(60)).unwrap();
    let after = ids(store
        .search_work_items(None, 10, Some("needs_you"), None)
        .unwrap()
        .entries);
    assert_eq!(after, before);
}

#[test]
fn view_search_and_attention_filters_keep_newest_first_order() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut expected = Vec::new();
    for index in 0..12 {
        let is_match = matches!(index, 2 | 5 | 9);
        let is_needs_you = index % 2 == 1;
        let id = format!("work-{index:02}");
        let state = if is_needs_you {
            "needs_attention"
        } else {
            "working"
        };
        let title = if is_match {
            format!("Needle item {index}")
        } else {
            format!("Other item {index}")
        };
        let item = work(&id, &title, state, &project);
        let saved = store.upsert_work_item(&item, None).unwrap();
        if is_match && is_needs_you {
            expected.push((id, saved.change_seq));
        }
    }
    expected.reverse();

    let page = store
        .search_work_items_with_attention(
            None,
            50,
            Some("needs_you"),
            Some("needle"),
            Some("unread"),
        )
        .unwrap();
    let actual: Vec<_> = page
        .entries
        .into_iter()
        .map(|entry| (entry.value.id, entry.change_seq))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(page.next_after, None);
}

#[test]
fn desk_cursor_does_not_change_ascending_list_change_feed() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    for index in 0..3 {
        let item = work(
            &format!("work-{index}"),
            &format!("Item {index}"),
            "needs_attention",
            &project,
        );
        store.upsert_work_item(&item, None).unwrap();
    }

    let first = store.list_work_items(None, 2).unwrap();
    assert!(first.entries[0].change_seq < first.entries[1].change_seq);
    let second = store.list_work_items(first.next_after, 2).unwrap();
    assert!(first.entries[1].change_seq < second.entries[0].change_seq);
    assert_eq!(second.next_after, None);
}

#[test]
fn filtered_desk_queries_reject_invalid_filters() {
    let root = TempDir::new().unwrap();
    let (_project, store) = open_store(&root);
    assert!(matches!(
        store.search_work_items_with_attention(None, 10, None, None, Some("unknown")),
        Err(OperatorStoreError::InvalidInput(_))
    ));
}
