//! Isolated fixture transport and real Buddy resource persistence coverage.
//!
//! `REFACT_SKIP_GUI_BUILD=1 cargo test --test buddy_operator -- --nocapture`
//!
//! Whole-worker acceptance: `sh tools/dev/buddy-operator-test.sh TEST_EXE REFACT_LSP`.
//! Live IDE parity remains a separate gate.

mod buddy_operator_common;

use buddy_operator_common::BuddyOperatorHarness;
use buddy_operator_common::fixtures::{FixtureEndpoints, FixtureTransport};
use serde_json::json;

#[tokio::test]
async fn whole_worker_entry() {
    let result = buddy_operator_common::worker::entry().await;
    if std::env::var_os("B02_MODE").is_some() {
        result.expect("whole-worker acceptance must succeed, never skip");
    } else {
        assert!(result.unwrap_err().to_string().contains("B02 REFUSED"));
        eprintln!("B02 confinement refusal tested; full scenario requires tools/dev/buddy-operator-test.sh");
    }
}

#[test]
fn whole_worker_refuses_missing_shared_or_routed_namespace() {
    use buddy_operator_common::worker::require_namespace as validate_namespace;
    assert!(validate_namespace(None, "net:[2]", &["lo".into()]).is_err());
    assert!(validate_namespace(Some("net:[1]"), "net:[1]", &["lo".into()]).is_err());
    assert!(validate_namespace(Some("net:[1]"), "net:[2]", &["lo".into(), "eth0".into()]).is_err());
    assert!(validate_namespace(Some("net:[1]"), "net:[2]", &["lo".into()]).is_ok());
}

#[tokio::test]
async fn process_fixture_refuses_partial_or_unusable_confinement_before_launch() {
    use refact_sandbox::{Enforcement, ExecSandboxSpec, SandboxError, SandboxProvider};
    struct Unsupported(Enforcement);
    impl SandboxProvider for Unsupported {
        fn name(&self) -> &'static str {
            "injected-unsupported"
        }
        fn probe(&self) -> Enforcement {
            self.0
        }
        fn confine(
            &self,
            _: &ExecSandboxSpec,
            _: &str,
            _: &[String],
        ) -> Result<(String, Vec<String>), SandboxError> {
            panic!("unsupported provider must never construct a launch command")
        }
    }
    let harness = BuddyOperatorHarness::new();
    for capability in [Enforcement::Partial, Enforcement::Unusable] {
        let result = buddy_operator_common::fixtures::run_confined_process_with_provider(
            &Unsupported(capability),
            harness.project_root(),
            harness.user_config_root(),
        )
        .await;
        assert!(result
            .unwrap_err()
            .contains("full bubblewrap filesystem/network confinement is required"));
        assert!(!harness.project_root().join("process-effect.txt").exists());
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

#[tokio::test]
async fn fixture_mcp_effect_survives_endpoint_restart_and_unknown_tool_cannot_write() {
    let harness = BuddyOperatorHarness::new();
    let mut endpoints = FixtureEndpoints::start(harness.project_root().to_path_buf());
    let transport = FixtureTransport::new([endpoints.address]);
    for (id, method, params) in [
        (
            1,
            "initialize",
            json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"operator-test","version":"1"}}),
        ),
        (2, "tools/list", json!({})),
        (
            3,
            "tools/call",
            json!({"name":"write_scratch","arguments":{}}),
        ),
    ] {
        let result: serde_json::Value = transport
            .request(
                reqwest::Method::POST,
                &endpoints.url("/mcp"),
                Some(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})),
            )
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result["id"], id);
        assert!(result.get("error").is_none());
    }
    let marker = harness.project_root().join("mcp-effect.txt");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "fixture MCP effect\n"
    );
    endpoints.shutdown().await;
    let mut restarted = FixtureEndpoints::start(harness.project_root().to_path_buf());
    let transport = FixtureTransport::new([restarted.address]);
    let response: serde_json::Value = transport.request(reqwest::Method::POST, &restarted.url("/mcp"), Some(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"write_scratch","arguments":{"path":"../escape"}}})))
        .await.unwrap().json().await.unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        "fixture MCP effect\n"
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn fixture_transport_denies_nonallowlisted_destinations_before_connect_and_never_follows_redirect(
) {
    let harness = BuddyOperatorHarness::new();
    let mut endpoints = FixtureEndpoints::start(harness.project_root().to_path_buf());
    let transport = FixtureTransport::new([endpoints.address]);
    let forbidden = [
        "https://example.invalid/private",
        "http://203.0.113.1:80/private",
        "http://127.0.0.1:1/private",
        "http://localhost:80/private",
    ];
    for url in forbidden {
        assert!(transport
            .request(reqwest::Method::GET, url, None)
            .await
            .unwrap_err()
            .contains("before DNS/connect"));
    }
    let wrong_route = endpoints.url("/not-allowed");
    assert!(transport
        .request(reqwest::Method::GET, &wrong_route, None)
        .await
        .is_err());
    let redirect = transport
        .request(reqwest::Method::GET, &endpoints.url("/redirect"), None)
        .await
        .unwrap();
    assert_eq!(redirect.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
    let mut expected: Vec<String> = forbidden.into_iter().map(String::from).collect();
    expected.push(wrong_route);
    assert_eq!(transport.denied(), expected);
    assert!(endpoints.calls().is_empty());
    endpoints.shutdown().await;
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
