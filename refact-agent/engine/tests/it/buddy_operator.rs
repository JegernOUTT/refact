//! Isolated fixture transport and real Buddy resource persistence coverage.
//!
//! `REFACT_SKIP_GUI_BUILD=1 cargo test --test it -- buddy_operator:: --nocapture`
//!
//! Whole-worker acceptance: `sh tools/dev/buddy-operator-test.sh TEST_EXE REFACT_LSP`.
//! Live IDE parity remains a separate gate.

use crate::buddy_operator_common::BuddyOperatorHarness;
use crate::buddy_operator_common::fixtures::{FixtureEndpoints, FixtureTransport};
use serde_json::json;

#[tokio::test]
async fn whole_worker_entry() {
    let result = crate::buddy_operator_common::worker::entry().await;
    if std::env::var_os("B02_MODE").is_some() {
        result.expect("whole-worker acceptance must succeed, never skip");
    } else {
        assert!(result.unwrap_err().to_string().contains("B02 REFUSED"));
        eprintln!("B02 confinement refusal tested; full scenario requires tools/dev/buddy-operator-test.sh");
    }
}

#[tokio::test]
async fn fixture_provider_result_becomes_real_persisted_buddy_draft() {
    let harness = BuddyOperatorHarness::new();
    let mut endpoints = FixtureEndpoints::start(harness.project_root().to_path_buf());
    let transport = FixtureTransport::new([endpoints.address]);
    std::fs::write(
        harness.user_config_root().join("provider.json"),
        json!({
            "endpoint": endpoints.url("/v1"), "api_key": "scratch-only-not-a-secret"
        })
        .to_string(),
    )
    .unwrap();
    let catalog: serde_json::Value = transport
        .request(reqwest::Method::GET, &endpoints.url("/v1/models"), None)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result: serde_json::Value = transport.request(reqwest::Method::POST, &endpoints.url("/v1/chat/completions"), Some(json!({
        "model": catalog["data"][0]["id"], "messages": [{"role":"user","content":"Produce a scratch skill"}]
    }))).await.unwrap().json().await.unwrap();
    let mut service = harness.service();
    let draft = service
        .prepare_draft(
            DraftKind::Skill,
            "Fixture skill".into(),
            result["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .into(),
            "Isolated provider output".into(),
        )
        .unwrap()
        .commit(&std::sync::Arc::new(tokio::sync::Mutex::new(None)))
        .await
        .unwrap();
    save_drafts(harness.project_root(), &service.draft_store.snapshot())
        .await
        .unwrap();
    drop(service);
    endpoints.shutdown().await;
    let restored = harness.restored_service().await;
    assert_eq!(
        restored.draft_store.get(&draft.id).unwrap().title,
        "Fixture skill"
    );
    assert!(
        std::fs::read_to_string(harness.project_root().join(".refact/buddy/drafts.json"))
            .unwrap()
            .contains("fixture-skill")
    );
    assert_eq!(endpoints.calls(), ["models", "completion"]);
    assert!(transport.denied().is_empty());
}

use refact_lsp::buddy::{
    drafts::{DraftStore, DraftTarget, DraftValidationError},
    storage::{load_drafts, save_drafts},
    types::DraftKind,
};
#[tokio::test]
async fn isolated_draft_create_get_delete_has_real_persisted_effect() {
    let mut harness = BuddyOperatorHarness::new();
    let root = harness.project_root().to_path_buf();
    let before = root.join(".refact/buddy/drafts.json");
    assert!(
        !before.exists(),
        "scratch project starts without Buddy drafts"
    );
    let mut service = harness.service();
    let draft = service
        .prepare_draft(
            DraftKind::Skill,
            "Scratch skill".to_string(),
            "---\nname: scratch-skill\n---\n# Scratch\n".to_string(),
            "created by the isolated operator smoke test".to_string(),
        )
        .expect("valid draft is prepared through real Buddy service")
        .commit(&std::sync::Arc::new(tokio::sync::Mutex::new(None)))
        .await
        .expect("valid draft is created through real Buddy service");
    assert_eq!(
        service
            .draft_store
            .get(&draft.id)
            .map(|stored| &stored.title),
        Some(&"Scratch skill".to_string()),
        "create/get effect is observable before persistence"
    );

    save_drafts(&root, &service.draft_store.snapshot())
        .await
        .expect("persist scratch draft");
    assert!(
        before.is_file(),
        "draft effect is contained below scratch root"
    );
    let saved = tokio::fs::read_to_string(&before)
        .await
        .expect("read scratch draft");
    assert!(saved.contains(&draft.id));
    assert!(saved.contains("scratch-skill"));

    let restored = harness.restored_service().await;
    assert_eq!(restored.draft_store.snapshot().len(), 1);
    assert_eq!(
        restored
            .draft_store
            .get(&draft.id)
            .map(|stored| &stored.title),
        Some(&"Scratch skill".to_string()),
        "new Buddy service restores the persisted draft"
    );

    assert!(service.delete_draft(&draft.id).is_some());
    assert!(service.draft_store.get(&draft.id).is_none());
    save_drafts(&root, &service.draft_store.snapshot())
        .await
        .expect("persist scratch deletion");
    assert!(
        load_drafts(&root).await.is_empty(),
        "delete effect survives reload from scratch filesystem"
    );

    harness.shutdown();
    assert!(harness.is_shutdown());
}

#[test]
fn isolated_draft_rejects_stale_or_wrong_target_without_filesystem_effect() {
    let mut harness = BuddyOperatorHarness::new();
    let drafts_path = harness.project_root().join(".refact/buddy/drafts.json");
    let mut drafts = DraftStore::new();
    let draft = drafts.create(
        DraftKind::Skill,
        "Bound skill".to_string(),
        "name: intended-skill\n".to_string(),
        "must only apply to the intended target".to_string(),
    );

    let rejected =
        drafts.get_validated(&draft.id, DraftKind::Skill, DraftTarget::Id("other-skill"));
    assert!(matches!(
        rejected,
        Err(DraftValidationError::TargetMismatch { .. })
    ));
    assert!(drafts_path.parent().is_some());
    assert!(
        !drafts_path.exists(),
        "validation rejection occurs before any resource write"
    );

    assert!(
        drafts.delete("missing-draft").is_none(),
        "stale id is rejected"
    );
    assert!(
        drafts.get(&draft.id).is_some(),
        "rejected target leaves draft intact"
    );
    harness.shutdown();
}
