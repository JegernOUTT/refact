use super::{OperatorStore, Stored};
use refact_buddy_core::operator_types::{ArtifactRevision, ProjectIdentity, WorkItem};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn project(root: &Path) -> ProjectIdentity {
    ProjectIdentity {
        project_id: "p".into(),
        canonical_root: dunce::canonicalize(root)
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
        item.last_material_change_at =
            (chrono::DateTime::parse_from_rfc3339("2026-09-05T00:00:00Z").unwrap()
                + chrono::Duration::seconds(index))
            .to_rfc3339();
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
    let mut previous_key: Option<(chrono::DateTime<chrono::FixedOffset>, String)> = None;
    loop {
        let page = store
            .search_work_items(cursor, 50, Some("needs_you"), None)
            .unwrap();
        assert!(!page.entries.is_empty());
        for entry in &page.entries {
            let key = (
                chrono::DateTime::parse_from_rfc3339(&entry.value.last_material_change_at).unwrap(),
                entry.value.id.clone(),
            );
            assert!(previous_key.as_ref().is_none_or(
                |previous| key.0 < previous.0 || (key.0 == previous.0 && key.1 < previous.1)
            ));
            previous_key = Some(key);
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
    updated.last_material_change_at = "2026-09-06T00:00:00Z".into();
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
fn non_material_writes_between_pages_do_not_duplicate_or_skip_rows() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut revisions = Vec::new();
    for index in 0..6 {
        let id = format!("work-{index}");
        let mut item = work(&id, &id, "needs_attention", &project);
        item.last_material_change_at = format!("2026-09-05T00:{:02}:00Z", 5 - index);
        revisions.push(store.upsert_work_item(&item, None).unwrap());
    }

    let first = store
        .search_work_items(None, 2, Some("needs_you"), None)
        .unwrap();
    assert_eq!(ids(first.entries.clone()), ["work-0", "work-1"]);
    let cursor = first.next_after.unwrap();
    let mut cursor_item = first.entries[1].value.clone();
    cursor_item.current_artifact_revision = Some(3);
    cursor_item.title = "Cursor row updated without a material change".into();
    store
        .upsert_work_item(&cursor_item, Some(first.entries[1].revision))
        .unwrap();
    let mut older_item = revisions[5].value.clone();
    older_item.title = "Old row updated without a material change".into();
    store
        .upsert_work_item(&older_item, Some(revisions[5].revision))
        .unwrap();

    let mut cursor = Some(cursor);
    let mut actual = ids(first.entries);
    while let Some(after) = cursor {
        let page = store
            .search_work_items(Some(after), 2, Some("needs_you"), None)
            .unwrap();
        actual.extend(ids(page.entries));
        cursor = page.next_after;
    }
    assert_eq!(
        actual,
        (0..6)
            .map(|index| format!("work-{index}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn desk_order_compares_timestamps_by_instant_across_offsets() {
    let root = TempDir::new().unwrap();
    let (project, mut store) = open_store(&root);
    let mut older = work("older-offset", "Older", "needs_attention", &project);
    older.last_material_change_at = "2026-09-05T02:00:00+02:00".into();
    store.upsert_work_item(&older, None).unwrap();
    let mut newer = work("newer-offset", "Newer", "needs_attention", &project);
    newer.last_material_change_at = "2026-09-05T01:00:01+01:00".into();
    store.upsert_work_item(&newer, None).unwrap();
    let mut latest = work("latest-offset", "Latest", "needs_attention", &project);
    latest.last_material_change_at = "2026-09-05T00:00:02Z".into();
    store.upsert_work_item(&latest, None).unwrap();

    let page = store
        .search_work_items(None, 10, Some("needs_you"), None)
        .unwrap();
    assert_eq!(
        ids(page.entries),
        ["latest-offset", "newer-offset", "older-offset"]
    );
}
