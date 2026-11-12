use refact_apps_core::{AppStore, StateScope};
use serde_json::json;

#[tokio::test]
async fn create_and_clone_publish_complete_directories_without_staging_leftovers() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let original = store.create("original", "Original").await.unwrap();
    let cloned = store.clone_app("original", "cloned").await.unwrap();
    assert_ne!(original.manifest.app_id, cloned.manifest.app_id);
    for app in [&original, &cloned] {
        assert_eq!(
            store.read_file(&app.slug, "ui/app.js").await.unwrap(),
            b"'use strict';\n"
        );
        let published = store.publish(&app.slug, &app.revision).await.unwrap();
        assert_eq!(published.manifest.app_id, app.manifest.app_id);
        assert_eq!(published.revision, app.revision);
    }
    assert_eq!(
        store.create("original", "Duplicate").await.unwrap_err(),
        "destination exists"
    );
    let mut names = std::fs::read_dir(project.path().join(".refact/apps"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [".store.lock", "cloned", "original"].map(std::ffi::OsString::from)
    );
}

#[tokio::test]
async fn atomic_resource_replacement_leaves_only_the_committed_file() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let app = store.create("atomic", "Atomic").await.unwrap();
    let updated = store
        .write_file(
            "atomic",
            "ui/app.js",
            b"replacement();".to_vec(),
            &app.revision,
        )
        .await
        .unwrap();
    assert_ne!(updated.revision, app.revision);
    assert_eq!(
        store.read_file("atomic", "ui/app.js").await.unwrap(),
        b"replacement();"
    );
    let mut names = std::fs::read_dir(project.path().join(".refact/apps/atomic/ui"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["app.js", "index.html", "style.css"].map(std::ffi::OsString::from)
    );
}

#[tokio::test]
async fn stale_state_writer_cannot_overwrite_persisted_value() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    store.create("counter", "Counter").await.unwrap();
    let first = store
        .write_state("counter", StateScope::Shared, 0, json!({"count":41}))
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    let reopened = AppStore::open(project.path()).await.unwrap();
    assert!(
        reopened
            .write_state("counter", StateScope::Shared, 0, json!({"count":99}))
            .await
            .is_err(),
        "stale state CAS accepted"
    );
    let loaded = reopened
        .read_state("counter", StateScope::Shared)
        .await
        .unwrap();
    assert_eq!((loaded.revision, loaded.value), (1, json!({"count":41})));
}

#[tokio::test]
async fn rollback_restores_full_bundle_and_survives_invalid_draft() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let app = store.create("weather", "Weather").await.unwrap();
    let app = store
        .write_file(
            "weather",
            "sources/feed.json",
            br#"{"feed":"first"}"#.to_vec(),
            &app.revision,
        )
        .await
        .unwrap();
    let first = store.publish("weather", &app.revision).await.unwrap();
    let app = store
        .write_file("weather", "ui/app.js", b"second();".to_vec(), &app.revision)
        .await
        .unwrap();
    let app = store
        .write_file(
            "weather",
            "sources/feed.json",
            br#"{"feed":"second"}"#.to_vec(),
            &app.revision,
        )
        .await
        .unwrap();
    let second = store.publish("weather", &app.revision).await.unwrap();
    assert_ne!(first.revision, second.revision);
    std::fs::remove_file(project.path().join(".refact/apps/weather/ui/index.html")).unwrap();
    let mut edited_manifest = app.manifest.clone();
    edited_manifest.app_id = uuid::Uuid::new_v4();
    std::fs::write(
        project.path().join(".refact/apps/weather/app.json"),
        serde_json::to_vec(&edited_manifest).unwrap(),
    )
    .unwrap();
    let reopened = AppStore::open(project.path()).await.unwrap();
    assert_eq!(
        reopened.get_by_id(app.manifest.app_id).await.unwrap().slug,
        "weather"
    );
    let saved = reopened
        .write_state("weather", StateScope::Shared, 0, json!({"live":true}))
        .await
        .unwrap();
    assert_eq!(
        reopened
            .read_state("weather", StateScope::Shared)
            .await
            .unwrap()
            .value,
        saved.value
    );
    assert_eq!(
        reopened.published("weather").await.unwrap().files,
        second.files
    );
    let restored = reopened
        .rollback("weather", &first.revision, Some(&second.revision))
        .await
        .unwrap();
    assert_eq!(
        restored.files, first.files,
        "rollback did not select the full original bundle"
    );
    assert_eq!(
        reopened.published("weather").await.unwrap().files,
        first.files
    );
}

#[tokio::test]
async fn tampered_revision_is_rejected_without_switching_active() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let app = store.create("weather", "Weather").await.unwrap();
    let first = store.publish("weather", &app.revision).await.unwrap();
    let path = project.path().join(format!(
        ".refact/apps/weather/.local/revisions/{}.json",
        first.revision
    ));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["files"]["ui/app.js"] = json!([88]);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        store.published("weather").await.is_err(),
        "tampered immutable bundle served"
    );
}

#[tokio::test]
async fn stale_author_revision_cannot_publish_or_overwrite() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let old = store.create("editor", "Editor").await.unwrap();
    let current = store
        .write_file("editor", "ui/app.js", b"fresh();".to_vec(), &old.revision)
        .await
        .unwrap();
    assert!(
        store
            .write_file("editor", "ui/app.js", b"stale();".to_vec(), &old.revision)
            .await
            .is_err(),
        "stale file write accepted"
    );
    assert!(
        store.publish("editor", &old.revision).await.is_err(),
        "stale publication accepted"
    );
    assert_eq!(
        store.get("editor").await.unwrap().revision,
        current.revision
    );
    assert_eq!(
        store.read_file("editor", "ui/app.js").await.unwrap(),
        b"fresh();"
    );
}

#[tokio::test]
async fn duplicate_identity_quarantines_both_apps() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let a = store.create("one", "One").await.unwrap();
    let b = store.clone_app("one", "two").await.unwrap();
    assert_ne!(a.manifest.app_id, b.manifest.app_id);
    std::fs::write(
        project.path().join(".refact/apps/two/app.json"),
        serde_json::to_vec(&a.manifest).unwrap(),
    )
    .unwrap();
    let list = store.list().await.unwrap();
    assert!(list.apps.is_empty(), "ambiguous UUID remained discoverable");
    assert_eq!(list.diagnostics.len(), 2);
    assert!(store.get_by_id(a.manifest.app_id).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_project_storage_cannot_escape() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"do not disclose").unwrap();
    std::os::unix::fs::symlink(outside.path(), project.path().join(".refact")).unwrap();
    assert!(
        AppStore::open(project.path()).await.is_err(),
        "symlink project storage accepted"
    );
    assert!(!outside.path().join("apps").exists());
}

#[tokio::test]
async fn private_runtime_state_is_not_author_readable_or_published() {
    let project = tempfile::tempdir().unwrap();
    let store = AppStore::open(project.path()).await.unwrap();
    let app = store.create("private", "Private").await.unwrap();
    store
        .write_state(
            "private",
            StateScope::Shared,
            0,
            json!({"private":"runtime"}),
        )
        .await
        .unwrap();
    assert!(
        store
            .read_file("private", ".local/state/shared.json")
            .await
            .is_err(),
        "private state exposed to file author API"
    );
    let published = store.publish("private", &app.revision).await.unwrap();
    assert!(
        !published
            .files
            .keys()
            .any(|path| path.starts_with(".local")),
        "private state entered published bundle"
    );
}
