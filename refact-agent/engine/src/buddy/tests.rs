use std::sync::Arc;
use crate::app_state::AppState;
use chrono::Duration;
use tokio::sync::broadcast;
use crate::tasks::types::{BoardCard, TaskMeta, TaskStatus};

use super::actor::BuddyService;
use super::diagnostics::{DiagnosticContext, DiagnosticSeverity};
use super::issues::{
    issue_dedupe_text, issue_title_and_body, mcp_issue_args, prepare_issue_content,
    validate_issue_binary, IssueProvider,
};
use super::settings::{AutonomyLevel, BuddySettings, HumorLevel};
use super::state::{add_activity, default_buddy_state};
use super::types::{
    BuddyAction, BuddyActivity, BuddyFact, BuddyFactKind, BuddyJobState, BuddyOnboarding,
    BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks, BuddyPage, BuddyPriority,
    BuddyPulse, BuddySuggestion, BuddyState, CustomizationKind, DefaultsKind, DraftKind,
    InvestigationContext, MarketKind, OpportunityStatus, PulseScope, BuddyWorkflowSummary,
};

fn make_service_with_events() -> (BuddyService, broadcast::Receiver<super::events::BuddyEvent>) {
    let (tx, rx) = broadcast::channel(16);
    let svc = BuddyService::new(
        std::env::temp_dir().join(format!("buddy-test-{}", uuid::Uuid::new_v4())),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );
    (svc, rx)
}

fn make_service() -> BuddyService {
    make_service_with_events().0
}

#[test]
fn invalid_workflow_id_failure_report_is_rejected_before_state_or_event() {
    let (mut svc, mut rx) = make_service_with_events();
    let report = super::workflows::WorkflowFailureReport {
        workflow_id: "../bad".to_string(),
        category: super::workflows::WorkflowFailureCategory::ModelUnavailable,
        summary: "Model unavailable".to_string(),
        detail: "model not found".to_string(),
        chat_id: None,
    };

    assert!(svc.record_workflow_failure_report(report).is_none());
    assert!(svc.state.workflow_summaries.is_empty());
    assert!(svc.state.recent_activities.is_empty());
    assert!(svc.runtime_queue.items.is_empty());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_settings_update_rejects_invalid_digest_without_mutating() {
    let app = make_gcx_with_buddy().await;
    let before = {
        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        lock.as_ref().unwrap().settings.clone()
    };
    let req: crate::http::routers::v1::buddy::BuddySettingsRequest =
        serde_json::from_str(r#"{"daily_digest_hour": 24, "quiet_mode": true}"#).unwrap();

    let err = crate::http::routers::v1::buddy::handle_v1_buddy_settings_update(
        axum::extract::State(app.clone()),
        axum::Json(req),
    )
    .await
    .unwrap_err();

    assert_eq!(err.status_code, hyper::StatusCode::BAD_REQUEST);
    let after = {
        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        lock.as_ref().unwrap().settings.clone()
    };
    assert_eq!(after.daily_digest_hour, before.daily_digest_hour);
    assert_eq!(after.quiet_mode, before.quiet_mode);
}

#[tokio::test]
async fn test_operator_enabled_patch_round_trip_preserves_unknown_and_unrelated_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".refact/buddy/settings.json");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(
        &path,
        br#"{"enabled":true,"auto_diagnostics":true,"auto_issue_creation":false,"quiet_hours_start":9,"future_field":{"kept":[1,2]}}"#,
    )
    .await
    .unwrap();

    let loaded = super::settings::load_settings(root.path()).await;
    assert!(loaded.operator_enabled);
    assert_eq!(loaded.quiet_hours_start, 9);

    let req: crate::http::routers::v1::buddy::BuddySettingsRequest =
        serde_json::from_str(r#"{"operator_enabled":false}"#).unwrap();
    let mut next = loaded;
    req.apply_to_settings(&mut next);
    assert!(!next.operator_enabled);
    assert_eq!(next.quiet_hours_start, 9);
    super::settings::save_settings(root.path(), &next)
        .await
        .unwrap();

    let document: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    assert_eq!(document["future_field"]["kept"], serde_json::json!([1, 2]));
    assert_eq!(document["operator_enabled"], serde_json::json!(false));
    assert_eq!(document["quiet_hours_start"], serde_json::json!(9));

    let reloaded = super::settings::load_settings(root.path()).await;
    assert!(!reloaded.operator_enabled);
    assert_eq!(reloaded.quiet_hours_start, 9);
    assert_eq!(reloaded.enabled, next.enabled);
    assert_eq!(reloaded.autonomy_level, next.autonomy_level);
}

#[tokio::test]
async fn test_settings_update_hot_applies_and_emits_event() {
    let app = make_gcx_with_buddy().await;
    let mut rx = {
        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        lock.as_ref().unwrap().events_tx.subscribe()
    };
    let req: crate::http::routers::v1::buddy::BuddySettingsRequest = serde_json::from_str(
        r#"{
            "chat_reactions_enabled": false,
            "quiet_mode": true,
            "humor_level": "normal",
            "autonomy_level": "safe_auto",
            "daily_digest_hour": null,
            "observers": { "chat_pattern": false, "mcp_auth": false }
        }"#,
    )
    .unwrap();

    let response = crate::http::routers::v1::buddy::handle_v1_buddy_settings_update(
        axum::extract::State(app.clone()),
        axum::Json(req),
    )
    .await
    .unwrap();
    let returned: BuddySettings = serde_json::from_value(response.0).unwrap();

    assert!(!returned.chat_reactions_enabled);
    assert!(returned.quiet_mode);
    assert_eq!(returned.humor_level, HumorLevel::Normal);
    assert_eq!(returned.autonomy_level, AutonomyLevel::SafeAuto);
    assert_eq!(returned.daily_digest_hour, None);
    assert!(!returned.observers.chat_pattern);
    assert!(!returned.observers.mcp_auth);

    let (project_root, settings) = {
        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().unwrap();
        (svc.project_root.clone(), svc.settings.clone())
    };
    assert_eq!(
        settings.chat_reactions_enabled,
        returned.chat_reactions_enabled
    );
    assert_eq!(settings.quiet_mode, returned.quiet_mode);
    assert_eq!(settings.daily_digest_hour, None);
    let persisted = super::settings::load_settings(&project_root).await;
    assert_eq!(
        persisted.chat_reactions_enabled,
        returned.chat_reactions_enabled
    );
    assert_eq!(persisted.daily_digest_hour, None);

    let mut saw_settings_changed = false;
    while let Ok(event) = rx.try_recv() {
        if let super::events::BuddyEvent::SettingsChanged { settings } = event {
            assert!(!settings.chat_reactions_enabled);
            assert!(settings.quiet_mode);
            saw_settings_changed = true;
        }
    }
    assert!(saw_settings_changed);
}

fn make_issue_ctx(message: &str) -> DiagnosticContext {
    DiagnosticContext {
        error_type: "test".to_string(),
        error_message: message.to_string(),
        source_file: Some("src/main.rs".to_string()),
        tool_name: None,
        chat_id: None,
        model_id: None,
        collected_at: chrono::Utc::now().to_rfc3339(),
        severity: DiagnosticSeverity::High,
        occurrences: None,
    }
}

#[test]
fn issue_title_redacts_api_key_password_token_secret_authorization() {
    let ctx = make_issue_ctx(
        "api_key=VALUEAPI apikey=VALUEAPINOSCORE token=VALUETOKEN secret=VALUESECRET password=VALUEPASSWORD Authorization: VALUEAUTH",
    );
    let (title, body) = issue_title_and_body(&ctx);
    for raw in [
        "VALUEAPI",
        "VALUEAPINOSCORE",
        "VALUETOKEN",
        "VALUESECRET",
        "VALUEPASSWORD",
        "VALUEAUTH",
    ] {
        assert!(
            !title.contains(raw),
            "raw secret leaked in title: {}",
            title
        );
        assert!(!body.contains(raw), "raw secret leaked in body: {}", body);
    }
    assert!(title.contains("[REDACTED]"));
    assert!(body.contains("[REDACTED]"));
}

#[test]
fn mcp_issue_prepare_respects_rate_limit_when_not_manual() {
    let ctx = make_issue_ctx("rate limited issue error");
    let err = prepare_issue_content(
        &ctx,
        Some("Title"),
        Some("Body"),
        true,
        true,
        false,
        Some(std::time::Instant::now()),
        &[],
    )
    .unwrap_err();
    assert!(err.contains("rate limit active"));

    let prepared = prepare_issue_content(
        &ctx,
        Some("Title"),
        Some("Body"),
        true,
        false,
        true,
        Some(std::time::Instant::now()),
        &[],
    )
    .unwrap();
    assert_eq!(prepared.dedupe_text, issue_dedupe_text(&ctx));
}

#[test]
fn mcp_issue_prepare_sanitizes_args_title_body() {
    let ctx = make_issue_ctx("panic with token=CTXSECRET");
    let raw_title = format!("Crash token=TITLESECRET\n{}", "x".repeat(200));
    let raw_body = format!(
        "Body Bearer BODYSECRET api_key=BODYAPI with ```\n{}",
        "y".repeat(5000)
    );
    let prepared = prepare_issue_content(
        &ctx,
        Some(&raw_title),
        Some(&raw_body),
        true,
        true,
        false,
        None,
        &[],
    )
    .unwrap();
    let args = mcp_issue_args(
        "detected",
        "project",
        &prepared.title,
        &prepared.body,
        vec!["bug".to_string()],
    );
    assert_eq!(args["owner"], "detected");
    assert_eq!(args["repo"], "project");
    assert!(!prepared.title.contains('\n'));
    assert!(!prepared.title.contains('\r'));
    assert!(prepared.title.chars().count() <= 120);
    assert!(prepared.body.chars().count() <= 4000);
    assert!(!prepared.body.contains("```"));
    let serialized = serde_json::to_string(&args).unwrap();
    for raw in ["TITLESECRET", "BODYSECRET", "BODYAPI", "CTXSECRET"] {
        assert!(!serialized.contains(raw), "raw secret leaked: {}", raw);
    }
    assert!(!serialized.contains("smallcloudai/refact"));
}

#[test]
fn validate_issue_binary_rejects_absolute_path() {
    assert!(validate_issue_binary("/tmp/gh").is_err());
    assert!(validate_issue_binary("C:\\tmp\\gh").is_err());
}

#[test]
fn validate_issue_binary_rejects_unknown_command() {
    assert!(validate_issue_binary("evilbin").is_err());
    assert!(validate_issue_binary("gh.exe").is_err());
}

#[test]
fn validate_issue_binary_accepts_gh_and_glab() {
    assert_eq!(validate_issue_binary("gh").unwrap(), "gh");
    assert_eq!(validate_issue_binary("glab").unwrap(), "glab");
}

#[test]
fn issue_provider_debug_redacts_tokens() {
    let provider = IssueProvider::GitHub {
        binary: "gh".to_string(),
        token: "ghp_AbCdEfGhIj1234567890".to_string(),
    };
    let rendered = format!("{:?}", provider);
    assert!(!rendered.contains("ghp_AbCdEfGhIj1234567890"));
    assert!(rendered.contains("[REDACTED]"));
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn investigation_log_tail_reads_bounded_and_redacts() {
    use crate::http::routers::v1::buddy_opportunities::read_recent_log_lines;
    let _guard = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.get_logs_max_tail_bytes = 256 * 1024;
    });
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("refact.log");
    let mut content = String::from("old secret token=old-secret\n");
    content.push_str(&"x".repeat(300 * 1024));
    content.push_str("\nrecent token=recent-secret\nlast line\n");
    tokio::fs::write(&log_path, content).await.unwrap();

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    app.runtime.cmdline = Arc::new(crate::global_context::CommandLine {
        logs_to_file: log_path.to_string_lossy().into_owned(),
        ..(*app.runtime.cmdline).clone()
    });
    let tail = read_recent_log_lines(&app, 5).await.unwrap();
    assert!(!tail.contains("old secret"));
    assert!(!tail.contains("recent-secret"));
    assert!(tail.contains("token=[REDACTED]"));
    assert!(tail.contains("last line"));
}

#[tokio::test]
async fn diagnostic_metadata_is_redacted_before_storage() {
    let mut svc = make_service();
    svc.add_diagnostic(DiagnosticContext {
        error_type: "frontend".to_string(),
        error_message: "Bearer secret-token in /home/alice/project/gcx.ts".to_string(),
        source_file: Some("/home/alice/project/gcx.ts?token=secret".to_string()),
        tool_name: Some("tool?api_key=secret".to_string()),
        chat_id: None,
        model_id: None,
        collected_at: chrono::Utc::now().to_rfc3339(),
        severity: DiagnosticSeverity::High,
        occurrences: None,
    });

    let stored = svc.recent_diagnostics.first().unwrap();
    assert_eq!(stored.source_file.as_deref(), Some("[REDACTED_PATH]"));
    assert_eq!(stored.tool_name.as_deref(), Some("tool?api_key=[REDACTED]"));
    assert!(!stored.error_message.contains("secret-token"));
    let event = svc.runtime_queue.items.front().unwrap();
    assert_eq!(event.source, "[REDACTED_PATH]");
}

// =============================================================================
// Runtime queue persistence — JSONL log invariants
// =============================================================================

/// Enqueue an event then dismiss it; the JSONL log must replay to a queue
/// where the event is still present and `dismissed = true`.
#[tokio::test]
async fn test_runtime_queue_dismissal_survives_restart() {
    use super::actor::make_runtime_event;
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let mut ev = make_runtime_event(
        "error",
        "boom",
        "frontend/window_error",
        "key-1",
        "failed",
        Some("high"),
    );
    let id = ev.id.clone();
    append_runtime_record(root, &RuntimeQueueRecord::Event { event: ev.clone() })
        .await
        .unwrap();

    // Dismissal is recorded as a fresh Event with dismissed=true.
    ev.dismissed = true;
    append_runtime_record(root, &RuntimeQueueRecord::Event { event: ev.clone() })
        .await
        .unwrap();

    let queue = load_runtime_queue(root).await;
    let restored = queue
        .items
        .iter()
        .find(|e| e.id == id)
        .expect("event missing");
    assert!(restored.dismissed, "dismissal must survive replay");
}

/// Fill the queue past its cap, write a `Removed` tombstone for an evicted
/// event, then verify replay matches the in-memory survivors.
#[tokio::test]
async fn test_runtime_queue_eviction_tombstones_replay() {
    use super::actor::make_runtime_event;
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let mut victim = make_runtime_event(
        "signal",
        "doomed",
        "src",
        "victim-key",
        "started",
        Some("low"),
    );
    victim.priority = "low".to_string();
    let victim_id = victim.id.clone();
    append_runtime_record(
        root,
        &RuntimeQueueRecord::Event {
            event: victim.clone(),
        },
    )
    .await
    .unwrap();

    let survivor = make_runtime_event(
        "signal",
        "kept",
        "src",
        "survivor-key",
        "started",
        Some("normal"),
    );
    let survivor_id = survivor.id.clone();
    append_runtime_record(
        root,
        &RuntimeQueueRecord::Event {
            event: survivor.clone(),
        },
    )
    .await
    .unwrap();

    // Eviction tombstone for the victim.
    append_runtime_record(
        root,
        &RuntimeQueueRecord::Removed {
            id: victim_id.clone(),
        },
    )
    .await
    .unwrap();

    let queue = load_runtime_queue(root).await;
    assert!(
        !queue.items.iter().any(|e| e.id == victim_id),
        "evicted event must not resurrect"
    );
    assert!(
        queue.items.iter().any(|e| e.id == survivor_id),
        "survivor must remain"
    );
}

/// Two writes for the same id; on disk, the LATER record (in file order) wins.
/// Because all production writes are funneled through one writer task, file
/// order matches in-memory order — this test pins down the contract.
#[tokio::test]
async fn test_runtime_queue_replay_uses_latest_event() {
    use super::actor::make_runtime_event;
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let mut ev = make_runtime_event(
        "error",
        "first title",
        "src",
        "race-key",
        "started",
        Some("normal"),
    );
    let id = ev.id.clone();
    append_runtime_record(root, &RuntimeQueueRecord::Event { event: ev.clone() })
        .await
        .unwrap();

    ev.title = "second title".to_string();
    ev.status = "completed".to_string();
    append_runtime_record(root, &RuntimeQueueRecord::Event { event: ev.clone() })
        .await
        .unwrap();

    let queue = load_runtime_queue(root).await;
    let restored = queue.items.iter().find(|e| e.id == id).expect("missing");
    assert_eq!(restored.title, "second title");
    assert_eq!(restored.status, "completed");
}

/// `now_playing` is its own JSONL record kind. Setting it, then clearing it,
/// must survive a round-trip.
#[tokio::test]
async fn test_runtime_queue_now_playing_persists() {
    use super::actor::make_runtime_event;
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let np = make_runtime_event(
        "streaming",
        "live",
        "chat",
        "np-key",
        "started",
        Some("normal"),
    );
    let np_id = np.id.clone();
    append_runtime_record(
        root,
        &RuntimeQueueRecord::NowPlaying {
            event: Some(np.clone()),
        },
    )
    .await
    .unwrap();

    let queue = load_runtime_queue(root).await;
    assert_eq!(
        queue.now_playing.as_ref().map(|e| e.id.clone()),
        Some(np_id.clone()),
        "now_playing must round-trip"
    );

    append_runtime_record(root, &RuntimeQueueRecord::NowPlaying { event: None })
        .await
        .unwrap();
    let queue = load_runtime_queue(root).await;
    assert!(
        queue.now_playing.is_none(),
        "clearing now_playing must persist"
    );
}

#[test]
fn buddy_service_expire_runtime_events_persists_removed_tombstones() {
    use super::actor::{make_runtime_event, RuntimeQueueWriteOp};
    use super::storage::RuntimeQueueRecord;

    let (events_tx, _) = broadcast::channel(16);
    let (queue_tx, mut queue_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut svc = BuddyService::new(
        std::env::temp_dir().join(format!("buddy-test-{}", uuid::Uuid::new_v4())),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        events_tx,
        Some(queue_tx),
    );
    let mut event = make_runtime_event(
        "signal",
        "expired",
        "src",
        "expired-key",
        "completed",
        Some("normal"),
    );
    event.id = "expired-runtime-event".to_string();
    event.created_at = "2024-01-01T00:00:00Z".to_string();
    event.ttl_ms = Some(1000);
    svc.enqueue_runtime_event(event);

    let now = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:02Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let removed = svc.expire_runtime_events_at(now);

    assert_eq!(removed, vec!["expired-runtime-event".to_string()]);
    assert!(svc.runtime_queue.items.is_empty());
    let mut saw_tombstone = false;
    while let Ok(op) = queue_rx.try_recv() {
        if let RuntimeQueueWriteOp::Append(RuntimeQueueRecord::Removed { id }) = op {
            if id == "expired-runtime-event" {
                saw_tombstone = true;
            }
        }
    }
    assert!(saw_tombstone);
}

#[test]
fn buddy_service_stale_runtime_prune_persists_removed_tombstones() {
    use super::actor::{make_runtime_event, RuntimeQueueWriteOp};
    use super::storage::RuntimeQueueRecord;

    let (events_tx, _) = broadcast::channel(16);
    let (queue_tx, mut queue_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut svc = BuddyService::new(
        std::env::temp_dir().join(format!("buddy-test-{}", uuid::Uuid::new_v4())),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        events_tx,
        Some(queue_tx),
    );
    let mut stale = make_runtime_event(
        "chat_completed",
        "old completed chat",
        "chat",
        "stale-runtime-key",
        "completed",
        Some("normal"),
    );
    stale.id = "stale-runtime-event".to_string();
    stale.created_at = "2024-01-01T00:00:00Z".to_string();
    stale.ttl_ms = None;
    svc.enqueue_runtime_event(stale);

    let now = chrono::DateTime::parse_from_rfc3339("2024-01-01T02:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let removed = svc.expire_runtime_events_at(now);

    assert_eq!(removed, vec!["stale-runtime-event".to_string()]);
    assert!(svc.runtime_queue.items.is_empty());
    let mut saw_event_append = false;
    let mut saw_tombstone = false;
    while let Ok(op) = queue_rx.try_recv() {
        match op {
            RuntimeQueueWriteOp::Append(RuntimeQueueRecord::Event { event }) => {
                if event.id == "stale-runtime-event" {
                    saw_event_append = true;
                }
            }
            RuntimeQueueWriteOp::Append(RuntimeQueueRecord::Removed { id }) => {
                if id == "stale-runtime-event" {
                    saw_tombstone = true;
                }
            }
            _ => {}
        }
    }
    assert!(saw_event_append);
    assert!(saw_tombstone);
}

/// Backward-compat: legacy logs that contain bare `BuddyRuntimeEvent` JSON
/// objects (no `kind` tag) must still load successfully.
#[tokio::test]
async fn test_runtime_queue_loads_legacy_format() {
    use super::actor::make_runtime_event;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join(".refact/buddy/runtime_queue.jsonl");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();

    let ev = make_runtime_event(
        "error",
        "legacy",
        "src",
        "legacy-key",
        "failed",
        Some("high"),
    );
    let id = ev.id.clone();
    let line = format!("{}\n", serde_json::to_string(&ev).unwrap());
    tokio::fs::write(&path, line).await.unwrap();

    let queue = super::storage::load_runtime_queue(root).await;
    assert!(
        queue.items.iter().any(|e| e.id == id),
        "legacy bare-event line must be readable"
    );
}

#[tokio::test]
async fn test_stale_runtime_load_pruning_compacts_removed_records() {
    use super::actor::make_runtime_event;
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut stale = make_runtime_event(
        "chat_completed",
        "old completed chat",
        "chat",
        "stale-load-key",
        "completed",
        Some("normal"),
    );
    stale.id = "stale-load-event".to_string();
    stale.created_at = (chrono::Utc::now() - Duration::hours(2)).to_rfc3339();
    stale.ttl_ms = None;
    append_runtime_record(root, &RuntimeQueueRecord::Event { event: stale })
        .await
        .unwrap();

    let queue = load_runtime_queue(root).await;

    assert!(queue.items.is_empty());
    let content = tokio::fs::read_to_string(root.join(".refact/buddy/runtime_queue.jsonl"))
        .await
        .unwrap();
    assert!(!content.contains("stale-load-event"));
}

// =============================================================================
// Redaction strength
// =============================================================================

#[tokio::test]
async fn generated_buddy_speech_with_secret_uses_fallback() {
    let (service, _renderer) =
        crate::buddy::voice_service::test_voice_service_with_responses(vec![Some(
            "Tiny alarm: Bearer sk-VERYSECRET1234567890 is wobbling.".to_string(),
        )]);
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app =
        crate::app_state::AppState::from_gcx(crate::global_context::tests::make_test_gcx().await)
            .await;
    let fallback = "Quest complete: safe fallback sparkle.".to_string();

    let speech = super::actor::render_buddy_speech(
        app,
        super::types::BuddyPersonalityProfile::default(),
        "Pixel".to_string(),
        BuddyPulse::default(),
        Some("secret_speech_test".to_string()),
        "workflow summary".to_string(),
        crate::buddy::voice_service::SpeechIntent::QuestComplete,
        fallback.clone(),
    )
    .await;

    assert_eq!(speech.text, fallback);
    assert!(!speech.text.contains("VERYSECRET"));
    assert!(!speech.text.contains("[REDACTED"));
}

#[tokio::test]
async fn normal_generated_buddy_speech_is_preserved() {
    let generated = "Tiny boots on, checking the path.";
    let (service, _renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
        vec![Some(generated.to_string())],
    );
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app =
        crate::app_state::AppState::from_gcx(crate::global_context::tests::make_test_gcx().await)
            .await;

    let speech = super::actor::render_buddy_speech(
        app,
        super::types::BuddyPersonalityProfile::default(),
        "Pixel".to_string(),
        BuddyPulse::default(),
        Some("normal_speech_test".to_string()),
        "workflow summary".to_string(),
        crate::buddy::voice_service::SpeechIntent::QuestAccept,
        "fallback should not be used".to_string(),
    )
    .await;

    assert_eq!(speech.text, generated);
}

#[tokio::test]
async fn generated_buddy_speech_with_secret_fallback_uses_static_safe_text() {
    let (service, _renderer) =
        crate::buddy::voice_service::test_voice_service_with_responses(vec![Some(
            "Speech Bearer sk-GENERATEDSECRET1234567890".to_string(),
        )]);
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app =
        crate::app_state::AppState::from_gcx(crate::global_context::tests::make_test_gcx().await)
            .await;

    let speech = super::actor::render_buddy_speech(
        app,
        super::types::BuddyPersonalityProfile::default(),
        "Pixel".to_string(),
        BuddyPulse::default(),
        Some("secret_fallback_speech_test".to_string()),
        "workflow summary".to_string(),
        crate::buddy::voice_service::SpeechIntent::QuestComplete,
        "Fallback Bearer sk-FALLBACKSECRET1234567890".to_string(),
    )
    .await;

    assert_eq!(
        speech.text,
        "Tiny gremlin update: something needs attention."
    );
    assert!(!speech.text.contains("GENERATEDSECRET"));
    assert!(!speech.text.contains("FALLBACKSECRET"));
    assert!(!speech.text.contains("[REDACTED"));
}

#[tokio::test]
async fn runtime_event_secret_fallbacks_use_static_safe_text() {
    let (service, _renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
        vec![Some("Runtime token=GENERATEDSECRET".to_string())],
    );
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app =
        crate::app_state::AppState::from_gcx(crate::global_context::tests::make_test_gcx().await)
            .await;

    let (title, description) = super::actor::render_buddy_runtime_event(
        app,
        super::types::BuddyPersonalityProfile::default(),
        "Pixel".to_string(),
        BuddyPulse::default(),
        Some("secret_fallback_runtime_test".to_string()),
        "Workflow Bearer sk-DESCRIPTIONGENERATED1234567890".to_string(),
        "completed",
        "Fallback title Bearer sk-TITLESECRET1234567890".to_string(),
        Some("Fallback description token=DESCRIPTIONSECRET".to_string()),
    )
    .await;

    assert_eq!(title, "Buddy update");
    assert_eq!(description.as_deref(), Some("Buddy has an update ready."));
    let rendered = format!("{} {}", title, description.unwrap_or_default());
    assert!(!rendered.contains("GENERATEDSECRET"));
    assert!(!rendered.contains("DESCRIPTIONGENERATED"));
    assert!(!rendered.contains("TITLESECRET"));
    assert!(!rendered.contains("DESCRIPTIONSECRET"));
    assert!(!rendered.contains("[REDACTED"));
}

// =============================================================================
// FactStore tests
// =============================================================================

fn make_fact(key: &str, kind: BuddyFactKind, seen_at: chrono::DateTime<chrono::Utc>) -> BuddyFact {
    BuddyFact {
        kind,
        key: key.to_string(),
        source: "test",
        payload: serde_json::json!({"k": key}),
        seen_at,
        confidence: 1.0,
    }
}

fn make_opportunity(id: &str, cooldown_key: &str) -> BuddyOpportunity {
    let now = chrono::Utc::now();
    BuddyOpportunity {
        id: id.to_string(),
        kind: BuddyOpportunityKind::TaskHealth,
        summary: "test".to_string(),
        priority: BuddyPriority::Normal,
        confidence: 0.9,
        fact_keys: vec![],
        cooldown_key: cooldown_key.to_string(),
        cooldown_secs: 1800,
        status: OpportunityStatus::New,
        proposed_actions: vec![],
        humor: None,
        humor_allowed: false,
        related: BuddyOpportunityLinks::default(),
        created_at: now,
        expires_at: now + Duration::hours(1),
        resolved_at: None,
    }
}

fn push_opportunity(queue: &mut super::opportunities::OpportunityQueue, opp: BuddyOpportunity) {
    queue.push_with_cooldown(
        opp,
        super::opportunities::DEFAULT_COOLDOWN.num_seconds() as u64,
    );
}

// =============================================================================
// OpportunityQueue tests
// =============================================================================

// =============================================================================
// DraftStore tests
// =============================================================================

// =============================================================================
// PulseBuilder tests
// =============================================================================

fn serialized_tag<T: serde::Serialize>(value: &T, tag: &str) -> String {
    serde_json::to_value(value)
        .expect("serialize")
        .get(tag)
        .and_then(|v| v.as_str())
        .expect("tag")
        .to_string()
}

#[test]
fn schema_contract_buddy_page_variants() {
    let cases = vec![
        (BuddyPage::Buddy, "buddy"),
        (BuddyPage::Stats, "stats"),
        (BuddyPage::Customization, "customization"),
        (BuddyPage::Providers, "providers"),
        (BuddyPage::DefaultModels, "default_models"),
        (BuddyPage::Integrations, "integrations"),
        (BuddyPage::Extensions, "extensions"),
        (BuddyPage::MarketplaceHub, "marketplace_hub"),
        (BuddyPage::Marketplace, "marketplace"),
        (BuddyPage::SkillsMarketplace, "skills_marketplace"),
        (BuddyPage::CommandsMarketplace, "commands_marketplace"),
        (BuddyPage::DelegatesMarketplace, "delegates_marketplace"),
        (BuddyPage::TasksList, "tasks_list"),
        (
            BuddyPage::TaskWorkspace {
                task_id: "task-1".to_string(),
            },
            "task_workspace",
        ),
        (BuddyPage::KnowledgeGraph, "knowledge_graph"),
        (BuddyPage::Worktrees, "worktrees"),
        (
            BuddyPage::SetupMode {
                mode: "setup_mcp".to_string(),
            },
            "setup_mode",
        ),
    ];
    for (page, expected) in cases {
        let json = serde_json::to_value(&page).expect("serialize");
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some(expected));
        if expected == "task_workspace" {
            assert_eq!(json.get("task_id").and_then(|v| v.as_str()), Some("task-1"));
        }
        if expected == "setup_mode" {
            assert_eq!(json.get("mode").and_then(|v| v.as_str()), Some("setup_mcp"));
        }
    }
}

#[test]
fn schema_contract_buddy_action_variants() {
    let actions = vec![
        (
            BuddyAction::OpenPage {
                page: BuddyPage::Buddy,
            },
            "open_page",
        ),
        (
            BuddyAction::LaunchInvestigationChat {
                preload: InvestigationContext {
                    fact_keys: vec![],
                    diagnostic_ids: vec![],
                    memory_key: None,
                    log_excerpt: String::new(),
                    config_summary: String::new(),
                    initial_user_message: "investigate".to_string(),
                },
            },
            "launch_investigation_chat",
        ),
        (
            BuddyAction::DraftSkill {
                draft_id: "d1".to_string(),
                label: "Skill".to_string(),
            },
            "draft_skill",
        ),
        (
            BuddyAction::DraftCommand {
                draft_id: "d2".to_string(),
                label: "Command".to_string(),
            },
            "draft_command",
        ),
        (
            BuddyAction::DraftDelegate {
                draft_id: "d3".to_string(),
                label: "Delegate".to_string(),
            },
            "draft_delegate",
        ),
        (
            BuddyAction::DraftMode {
                draft_id: "d4".to_string(),
                label: "Mode".to_string(),
            },
            "draft_mode",
        ),
        (
            BuddyAction::DraftAgentsMdPatch {
                content: String::new(),
            },
            "draft_agents_md_patch",
        ),
        (
            BuddyAction::DraftDefaultsChange {
                defaults_kind: DefaultsKind::ChatModel,
                patch: serde_json::json!({}),
            },
            "draft_defaults_change",
        ),
        (
            BuddyAction::DraftCustomizationChange {
                customization_kind: CustomizationKind::Delegate,
                id: "delegate-1".to_string(),
                patch: serde_json::json!({}),
            },
            "draft_customization_change",
        ),
        (
            BuddyAction::OfferMarketplaceInstall {
                market_kind: MarketKind::Delegate,
                item_id: "item-1".to_string(),
            },
            "offer_marketplace_install",
        ),
        (
            BuddyAction::CreatePulseReport {
                scope: PulseScope::All,
            },
            "create_pulse_report",
        ),
        (BuddyAction::Dismiss, "dismiss"),
    ];
    for (action, expected) in actions {
        assert_eq!(serialized_tag(&action, "kind"), expected);
    }
}

#[test]
fn should_observe_thread_skips_buddy_task_setup_and_subchats() {
    let thread = crate::chat::types::ThreadParams::default();
    assert!(super::chat_reactions::should_observe_thread(&thread));

    let mut thread = crate::chat::types::ThreadParams::default();
    thread.buddy_meta = Some(refact_buddy_core::types::BuddyThreadMeta {
        is_buddy_chat: true,
        buddy_chat_kind: "investigation".to_string(),
        workflow_id: None,
        source_chat_id: None,
        source_project: None,
        source_created_at: None,
    });
    assert!(!super::chat_reactions::should_observe_thread(&thread));

    let mut thread = crate::chat::types::ThreadParams::default();
    thread.task_meta = Some(crate::chat::types::TaskMeta::default());
    assert!(!super::chat_reactions::should_observe_thread(&thread));

    for mode in ["buddy", "setup", "task_agent", "task_planner"] {
        let mut thread = crate::chat::types::ThreadParams::default();
        thread.mode = mode.to_string();
        assert!(!super::chat_reactions::should_observe_thread(&thread));
    }

    let mut thread = crate::chat::types::ThreadParams::default();
    thread.mode = "setup_mcp".to_string();
    assert!(!super::chat_reactions::should_observe_thread(&thread));

    let mut thread = crate::chat::types::ThreadParams::default();
    thread.parent_id = Some("parent".to_string());
    assert!(!super::chat_reactions::should_observe_thread(&thread));

    let mut thread = crate::chat::types::ThreadParams::default();
    thread.parent_id = Some("parent".to_string());
    thread.link_type = Some("branch".to_string());
    assert!(super::chat_reactions::should_observe_thread(&thread));
}

// =============================================================================
// Policy tests
// =============================================================================

fn make_opp_with_priority(id: &str, priority: BuddyPriority) -> BuddyOpportunity {
    let mut opp = make_opportunity(id, id);
    opp.priority = priority;
    opp
}

#[test]
fn policy_quiet_mode_drops_non_critical() {
    use super::policy::{evaluate, PolicyDecision};
    use super::opportunities::OpportunityQueue;
    let mut settings = BuddySettings::default();
    settings.quiet_mode = true;
    let queue = OpportunityQueue::new();

    let normal_opp = make_opp_with_priority("opp-normal", BuddyPriority::Normal);
    let result = evaluate(&normal_opp, &settings, &queue);
    assert!(matches!(
        result,
        PolicyDecision::Drop {
            reason: "quiet_mode"
        }
    ));

    let critical_opp = make_opp_with_priority("opp-critical", BuddyPriority::Critical);
    let result = evaluate(&critical_opp, &settings, &queue);
    assert!(matches!(result, PolicyDecision::Surface { .. }));
}

#[test]
fn policy_unread_cap_drops() {
    use super::policy::{evaluate, PolicyDecision};
    use super::opportunities::{OpportunityQueue, MAX_UNREAD};
    let settings = BuddySettings::default();
    let mut queue = OpportunityQueue::new();
    for i in 0..MAX_UNREAD {
        push_opportunity(
            &mut queue,
            make_opportunity(&format!("pre-{}", i), &format!("ck-pre-{}", i)),
        );
    }
    assert_eq!(queue.unread_count(), MAX_UNREAD);
    let opp = make_opp_with_priority("new-opp", BuddyPriority::Normal);
    let result = evaluate(&opp, &settings, &queue);
    assert!(matches!(
        result,
        PolicyDecision::Drop {
            reason: "unread_cap"
        }
    ));
}

#[test]
fn policy_dismissed_24h_drops() {
    use super::policy::{evaluate, PolicyDecision};
    use super::opportunities::OpportunityQueue;
    let settings = BuddySettings::default();
    let mut queue = OpportunityQueue::new();
    let opp = make_opportunity("opp-dm", "key-dm");
    push_opportunity(&mut queue, opp.clone());
    queue.dismiss("opp-dm");
    let new_opp = make_opportunity("opp-new", "key-dm");
    let result = evaluate(&new_opp, &settings, &queue);
    assert!(matches!(
        result,
        PolicyDecision::Drop {
            reason: "dismissed_24h"
        }
    ));
}

#[test]
fn policy_cooldown_drops() {
    use super::policy::{evaluate, PolicyDecision};
    use super::opportunities::OpportunityQueue;
    let settings = BuddySettings::default();
    let mut queue = OpportunityQueue::new();
    push_opportunity(&mut queue, make_opportunity("opp-cd", "cooldown-key"));
    assert!(queue.cooldown_active("cooldown-key"));
    let new_opp = make_opportunity("opp-cd2", "cooldown-key");
    let result = evaluate(&new_opp, &settings, &queue);
    assert!(matches!(
        result,
        PolicyDecision::Drop { reason: "cooldown" }
    ));
}

// =============================================================================
// Humor tests
// =============================================================================

use std::sync::atomic::{AtomicU32, Ordering};

struct CountingGenerator {
    count: std::sync::Arc<AtomicU32>,
    lines: Vec<String>,
}

#[async_trait::async_trait]
impl super::humor::HumorGenerator for CountingGenerator {
    async fn generate(
        &self,
        _kind: BuddyFactKind,
        _summary: String,
        _gcx: AppState,
    ) -> Vec<String> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.lines.clone()
    }
}

struct EmptyGenerator;

#[async_trait::async_trait]
impl super::humor::HumorGenerator for EmptyGenerator {
    async fn generate(
        &self,
        _kind: BuddyFactKind,
        _summary: String,
        _gcx: AppState,
    ) -> Vec<String> {
        vec![]
    }
}

struct SlowTimeoutGenerator;

#[async_trait::async_trait]
impl super::humor::HumorGenerator for SlowTimeoutGenerator {
    async fn generate(
        &self,
        _kind: BuddyFactKind,
        _summary: String,
        _gcx: AppState,
    ) -> Vec<String> {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        vec!["too late".to_string()]
    }
}

// =============================================================================
// Observer tests
// =============================================================================

struct SleepObserver {
    id: &'static str,
}

#[async_trait::async_trait]
impl super::observers::BuddyObserver for SleepObserver {
    fn id(&self) -> &'static str {
        self.id
    }

    fn cadence_seconds(&self) -> u64 {
        1
    }

    fn requires_setting(&self, _settings: &BuddySettings) -> bool {
        true
    }

    async fn observe(
        &self,
        _gcx: AppState,
        _ctx: &super::observers::ObserverContext,
    ) -> Vec<BuddyFact> {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        vec![make_fact(
            self.id,
            BuddyFactKind::TaskStuck,
            chrono::Utc::now(),
        )]
    }
}

#[test]
fn ephemeral_debug_renders_placeholder() {
    use super::observers::Ephemeral;
    let e = Ephemeral::new("secret".to_string());
    assert_eq!(format!("{:?}", e), "<ephemeral>");
    assert_eq!(e.as_ref(), "secret");
}

fn chat_msg(role: &str, content: &str) -> crate::call_validation::ChatMessage {
    crate::call_validation::ChatMessage {
        role: role.to_string(),
        content: crate::call_validation::ChatContent::SimpleText(content.to_string()),
        ..Default::default()
    }
}

#[test]
fn chat_pattern_no_secret_leakage() {
    use super::observers::chat_pattern::run_chat_pattern_observer_sync;
    let messages = vec![
        chat_msg("user", "my key is sk-FAKEKEYABCDEFGHIJKL12345"),
        chat_msg("user", "actually use Bearer abcdef-leak-token instead"),
        chat_msg("user", "wait try again"),
        chat_msg("user", "sorry undo"),
    ];
    let facts = run_chat_pattern_observer_sync(&messages, "chat-1");
    let serialized = serde_json::to_string(&facts).unwrap();
    assert!(!serialized.contains("sk-FAKEKEYABCDEFGHIJKL12345"));
    assert!(!serialized.contains("Bearer abcdef-leak-token"));
    assert!(!serialized.contains("actually"));
    assert!(!serialized.contains("undo"));
    assert!(facts
        .iter()
        .any(|f| matches!(f.kind, BuddyFactKind::ChatRetryStreak)));
}

// =============================================================================
// T-7: Detector rules + Actor wiring + Pulse aggregation
// =============================================================================

// =============================================================================
// T-8: HTTP routes unit tests
// =============================================================================

// =============================================================================
// T-10: Tool tests — open_view, create_draft, launch_investigation, buddy_yaml
// =============================================================================

// =============================================================================
// T-10/T-14: Editor draft_id integration tests
// =============================================================================

async fn make_gcx_with_buddy() -> AppState {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let buddy_arc = app.buddy.buddy.clone();
    *buddy_arc.lock().await = Some(make_service());
    app
}

async fn draft_exists(gcx: &AppState, id: &str) -> bool {
    let buddy_arc = gcx.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    lock.as_ref()
        .and_then(|svc| svc.draft_store.get(id))
        .is_some()
}

async fn add_draft_to_gcx(gcx: &AppState, kind: DraftKind, title: &str, content: &str) -> String {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().expect("buddy service must exist");
    svc.create_draft(kind, title.to_string(), content.to_string(), String::new())
        .expect("draft must be created")
        .id
}

#[tokio::test]
async fn ext_skill_save_with_command_draft_fails_and_keeps_draft() {
    use axum::extract::Path;
    use hyper::StatusCode;

    let app = make_gcx_with_buddy().await;
    let _gcx = app.gcx.clone();
    let draft_id = add_draft_to_gcx(
        &app,
        DraftKind::Command,
        "Command Draft",
        "---\ndescription: Command\n---\nBody",
    )
    .await;
    let body = serde_json::to_vec(&serde_json::json!({
        "raw_content": "---\nname: target_skill\ndescription: Target\n---\nBody",
        "draft_id": draft_id.clone(),
        "scope": "global"
    }))
    .unwrap();
    let response = crate::http::routers::v1::ext_management::handle_v1_ext_skill_put(
        axum::extract::State(app.clone()),
        Path("target_skill".to_string()),
        hyper::body::Bytes::from(body),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(draft_exists(&app, &draft_id).await);
}

#[tokio::test]
async fn ext_skill_save_with_mismatched_draft_target_keeps_draft() {
    use axum::extract::Path;
    use hyper::StatusCode;

    let app = make_gcx_with_buddy().await;
    let _gcx = app.gcx.clone();
    let draft_id = add_draft_to_gcx(
        &app,
        DraftKind::Skill,
        "Other Skill",
        "---\nname: other_skill\ndescription: Other\n---\nBody",
    )
    .await;
    let body = serde_json::to_vec(&serde_json::json!({
        "raw_content": "---\nname: target_skill\ndescription: Target\n---\nBody",

        "draft_id": draft_id.clone(),
        "scope": "global"
    }))
    .unwrap();
    let response = crate::http::routers::v1::ext_management::handle_v1_ext_skill_put(
        axum::extract::State(app.clone()),
        Path("target_skill".to_string()),
        hyper::body::Bytes::from(body),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(draft_exists(&app, &draft_id).await);
}

#[tokio::test]
async fn registry_validation_error_does_not_consume_draft() {
    use hyper::{Body, Request, StatusCode};
    use tower::ServiceExt;

    let app = make_gcx_with_buddy().await;
    let _gcx = app.gcx.clone();
    let config_dir = app.paths.config_dir.clone();
    tokio::fs::create_dir_all(config_dir.join("modes"))
        .await
        .unwrap();
    tokio::fs::write(config_dir.join("modes/broken.yaml"), "schema_version: [")
        .await
        .unwrap();
    let draft_id = add_draft_to_gcx(
        &app,
        DraftKind::Mode,
        "Mode Draft",
        "schema_version: 1\nid: valid_mode\ntitle: Valid\nprompt: ok\n",
    )
    .await;
    let body = serde_json::to_vec(&serde_json::json!({
        "config": {
            "schema_version": 1,
            "id": "valid_mode",
            "title": "Valid",
            "prompt": "ok"
        },
        "scope": "global",
        "draft_id": draft_id.clone()
    }))
    .unwrap();
    let app_state = app.clone();
    let router = crate::http::routers::v1::make_v1_router(app_state.clone()).with_state(app_state);
    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/customization/modes/valid_mode")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["ok"], false);
    assert!(draft_exists(&app, &draft_id).await);
}

#[tokio::test]
async fn concurrent_ext_atomic_writes_use_unique_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("commands").join("same.md");
    let first = crate::http::routers::v1::ext_management::write_file_atomic(&path, "first");
    let second = crate::http::routers::v1::ext_management::write_file_atomic(&path, "second");
    let (a, b) = tokio::join!(first, second);
    assert!(a.is_ok());
    assert!(b.is_ok());
    let final_content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(final_content == "first" || final_content == "second");
    let mut entries = tokio::fs::read_dir(path.parent().unwrap()).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(!name.ends_with(".tmp"));
    }
}

// =============================================================================
// T-19: Privacy hardening + Opportunity lifecycle correctness
// =============================================================================

#[tokio::test]
async fn diagnostic_persisted_jsonl_is_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();
    let (tx, _rx) = broadcast::channel(16);
    let mut svc = BuddyService::new(
        root.to_path_buf(),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );
    svc.report_error(
        "test",
        "connection failed: Bearer sk-LEAK_THIS_TOKEN",
        None,
        None,
    );
    let jsonl_path = root.join(".refact/buddy/diagnostics.jsonl");
    let mut content = String::new();
    for _ in 0..100 {
        content = tokio::fs::read_to_string(&jsonl_path)
            .await
            .unwrap_or_default();
        if !content.is_empty() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    assert!(
        !content.contains("sk-LEAK_THIS_TOKEN"),
        "secret must not be in jsonl: {}",
        content
    );
    assert!(
        content.contains("[REDACTED"),
        "redaction marker must be in jsonl: {}",
        content
    );
}

// =============================================================================
// Observer↔Detector schema contract tests
// =============================================================================

#[test]
fn terminal_opp_retention_uses_resolved_at() {
    use super::opportunities::OpportunityQueue;
    let now = chrono::Utc::now();

    // Opp with old created_at but resolved recently — must NOT be evicted at now+23h
    let mut opp1 = make_opportunity("opp-rt1", "ck-rt1");
    opp1.created_at = now - Duration::hours(48);
    opp1.expires_at = now - Duration::hours(47);
    opp1.status = OpportunityStatus::Dismissed;
    opp1.resolved_at = Some(now);

    let mut q1 = OpportunityQueue::new();
    q1.items.push(opp1);
    q1.expire_old(now + Duration::minutes(23 * 60 + 59));
    assert!(
        q1.get("opp-rt1").is_some(),
        "opp resolved now must survive at now+23h59m"
    );

    // Same opp — must be evicted at now+24h01m
    let mut opp2 = make_opportunity("opp-rt2", "ck-rt2");
    opp2.created_at = now - Duration::hours(48);
    opp2.expires_at = now - Duration::hours(47);
    opp2.status = OpportunityStatus::Completed;
    opp2.resolved_at = Some(now);

    let mut q2 = OpportunityQueue::new();
    q2.items.push(opp2);
    q2.expire_old(now + Duration::minutes(24 * 60 + 1));
    assert!(
        q2.get("opp-rt2").is_none(),
        "opp resolved now must be evicted at now+24h01m"
    );
}

// =============================================================================
// F-A: Humor off-lock + task health semantics + pulse completeness
// =============================================================================

// =============================================================================
// F-C: Opportunity resolution correctness
// =============================================================================

#[tokio::test]
async fn defaults_update_with_valid_draft_consumes_after_save() {
    use crate::providers::config::ProviderDefaults;
    use crate::providers::http::handle_v1_defaults_update;
    use hyper::body::Bytes;
    use hyper::StatusCode;

    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        crate::test_tmp::scratch_dir("cache"),
        dir.path().to_path_buf(),
    )
    .await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;

    let mut svc = make_service();
    let draft = svc
        .create_draft(
            DraftKind::DefaultsModel,
            "Default Models".to_string(),
            r#"{"chat":{"model":"openai/gpt-4o"}}"#.to_string(),
            String::new(),
        )
        .unwrap();
    let draft_id = draft.id.clone();
    *app.buddy.buddy.lock().await = Some(svc);

    let body = serde_json::json!({
        "chat": { "model": "openai/gpt-4o" },
        "chat_light": { "model": "openai/gpt-4o-mini" },
        "chat_thinking": {},
        "chat_buddy": {},
        "draft_id": draft_id.clone(),
    });
    let response = handle_v1_defaults_update(
        axum::extract::State(app.clone()),
        Bytes::from(body.to_string()),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let saved = ProviderDefaults::load(dir.path()).await.unwrap();
    assert_eq!(saved.chat.model.as_deref(), Some("openai/gpt-4o"));
    assert_eq!(
        saved.chat_light.model.as_deref(),
        Some("openai/gpt-4o-mini")
    );
    let buddy_arc = app.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    assert!(lock.as_ref().unwrap().draft_store.get(&draft_id).is_none());
}

#[tokio::test]
async fn defaults_update_wrong_draft_kind_returns_conflict_and_keeps_draft() {
    use crate::providers::http::handle_v1_defaults_update;
    use hyper::body::Bytes;
    use hyper::StatusCode;

    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        crate::test_tmp::scratch_dir("cache"),
        dir.path().to_path_buf(),
    )
    .await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;

    let mut svc = make_service();
    let draft = svc
        .create_draft(
            DraftKind::Skill,
            "Skill Draft".to_string(),
            "---\nname: skill\n---\nbody".to_string(),
            String::new(),
        )
        .unwrap();
    let draft_id = draft.id.clone();
    *app.buddy.buddy.lock().await = Some(svc);

    let body = serde_json::json!({
        "chat": { "model": "openai/gpt-4o" },
        "chat_light": {},
        "chat_thinking": {},
        "chat_buddy": {},
        "draft_id": draft_id.clone(),
    });
    let err = handle_v1_defaults_update(
        axum::extract::State(app.clone()),
        Bytes::from(body.to_string()),
    )
    .await
    .unwrap_err();

    assert_eq!(err.status_code, StatusCode::CONFLICT);
    let buddy_arc = app.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    assert!(lock.as_ref().unwrap().draft_store.get(&draft_id).is_some());
}

#[tokio::test]
async fn defaults_update_parse_invalid_draft_returns_422_and_keeps_draft() {
    use crate::providers::http::handle_v1_defaults_update;
    use hyper::body::Bytes;
    use hyper::StatusCode;

    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        crate::test_tmp::scratch_dir("cache"),
        dir.path().to_path_buf(),
    )
    .await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;

    let mut svc = make_service();
    let draft = svc
        .create_draft(
            DraftKind::DefaultsModel,
            "Default Models".to_string(),
            r#"{"chat_default_model":"openai/gpt-4o"}"#.to_string(),
            String::new(),
        )
        .unwrap();
    let draft_id = draft.id.clone();
    *app.buddy.buddy.lock().await = Some(svc);

    let body = serde_json::json!({
        "chat": { "model": "openai/gpt-4o" },
        "chat_light": {},
        "chat_thinking": {},
        "chat_buddy": {},
        "draft_id": draft_id.clone(),
    });
    let err = handle_v1_defaults_update(
        axum::extract::State(app.clone()),
        Bytes::from(body.to_string()),
    )
    .await
    .unwrap_err();

    assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
    let buddy_arc = app.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    assert!(lock.as_ref().unwrap().draft_store.get(&draft_id).is_some());
}

#[tokio::test]
async fn accept_after_dismiss_returns_409() {
    use axum::extract::Path;
    use crate::http::routers::v1::buddy_opportunities::{
        handle_v1_buddy_opportunity_accept, handle_v1_buddy_opportunity_dismiss, AcceptRequest,
    };
    use hyper::StatusCode;

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let mut svc = make_service();
    let mut opp = make_opportunity("opp-dismiss-then-accept", "ck-dismiss-then-accept");
    opp.proposed_actions = vec![BuddyAction::OpenPage {
        page: BuddyPage::Buddy,
    }];
    svc.add_opportunity(opp);
    *app.buddy.buddy.lock().await = Some(svc);

    let _ = handle_v1_buddy_opportunity_dismiss(
        axum::extract::State(app.clone()),
        Path("opp-dismiss-then-accept".to_string()),
        None,
    )
    .await
    .unwrap();
    let accept_err = handle_v1_buddy_opportunity_accept(
        axum::extract::State(app.clone()),
        Path("opp-dismiss-then-accept".to_string()),
        Some(axum::extract::Json(AcceptRequest { action_index: 0 })),
    )
    .await
    .unwrap_err();
    assert_eq!(accept_err.status_code, StatusCode::CONFLICT);
    let dismiss_err = handle_v1_buddy_opportunity_dismiss(
        axum::extract::State(app.clone()),
        Path("opp-dismiss-then-accept".to_string()),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(dismiss_err.status_code, StatusCode::CONFLICT);
}

#[tokio::test]
async fn concurrent_accepts_only_one_succeeds() {
    use axum::extract::Path;
    use crate::http::routers::v1::buddy_opportunities::{
        handle_v1_buddy_opportunity_accept, AcceptRequest,
    };
    use hyper::StatusCode;

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let mut svc = make_service();
    let mut opp = make_opportunity("opp-concurrent-accept", "ck-concurrent-accept");
    opp.proposed_actions = vec![BuddyAction::OpenPage {
        page: BuddyPage::Buddy,
    }];
    svc.add_opportunity(opp);
    *app.buddy.buddy.lock().await = Some(svc);

    let app1 = app.clone();
    let app2 = app.clone();
    let task1 = tokio::spawn(async move {
        match handle_v1_buddy_opportunity_accept(
            axum::extract::State(app1.clone()),
            Path("opp-concurrent-accept".to_string()),
            Some(axum::extract::Json(AcceptRequest { action_index: 0 })),
        )
        .await
        {
            Ok(_) => StatusCode::OK,
            Err(err) => err.status_code,
        }
    });
    let task2 = tokio::spawn(async move {
        match handle_v1_buddy_opportunity_accept(
            axum::extract::State(app2.clone()),
            Path("opp-concurrent-accept".to_string()),
            Some(axum::extract::Json(AcceptRequest { action_index: 0 })),
        )
        .await
        {
            Ok(_) => StatusCode::OK,
            Err(err) => err.status_code,
        }
    });
    let (status1, status2) = tokio::join!(task1, task2);
    let mut statuses = vec![status1.unwrap(), status2.unwrap()];
    statuses.sort_by_key(|s| s.as_u16());
    assert_eq!(statuses, vec![StatusCode::OK, StatusCode::CONFLICT]);
}

#[tokio::test]
async fn failed_dispatch_leaves_opportunity_retryable_and_clears_claim() {
    use axum::extract::Path;
    use crate::http::routers::v1::buddy_opportunities::{
        handle_v1_buddy_opportunity_accept, AcceptRequest,
    };
    use hyper::StatusCode;

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let mut svc = make_service();
    let mut opp = make_opportunity("opp-dispatch-fails", "ck-dispatch-fails");
    opp.proposed_actions = vec![BuddyAction::DraftCustomizationChange {
        customization_kind: CustomizationKind::Mode,
        id: "mode-dispatch-fails".to_string(),
        patch: serde_json::json!("not-an-object"),
    }];
    svc.add_opportunity(opp);
    *app.buddy.buddy.lock().await = Some(svc);

    let err = handle_v1_buddy_opportunity_accept(
        axum::extract::State(app.clone()),
        Path("opp-dispatch-fails".to_string()),
        Some(axum::extract::Json(AcceptRequest { action_index: 0 })),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);

    {
        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().unwrap();
        let opp = svc.opportunity_queue.get("opp-dispatch-fails").unwrap();
        assert_eq!(opp.status, OpportunityStatus::New);
        assert!(!svc.is_opportunity_accept_claimed("opp-dispatch-fails"));
    }

    let err = handle_v1_buddy_opportunity_accept(
        axum::extract::State(app.clone()),
        Path("opp-dispatch-fails".to_string()),
        Some(axum::extract::Json(AcceptRequest { action_index: 0 })),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn opportunity_expiry_persists_and_noops_when_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();
    let (tx, _rx) = broadcast::channel(16);
    let mut svc = BuddyService::new(
        root.to_path_buf(),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );
    let mut opp = make_opportunity("opp-expiry-persist", "ck-expiry-persist");
    opp.expires_at = chrono::Utc::now() - Duration::seconds(1);
    svc.add_opportunity(opp);
    svc.dirty = false;

    svc.expire_opportunities();
    assert!(svc.dirty);
    super::state::save_state(root, &svc.state).await.unwrap();
    let loaded = super::state::load_state(root).await;
    assert_eq!(
        loaded
            .opportunities
            .iter()
            .find(|o| o.id == "opp-expiry-persist")
            .map(|o| o.status),
        Some(OpportunityStatus::Expired)
    );

    svc.dirty = false;
    svc.expire_opportunities();
    assert!(!svc.dirty);
}

// =============================================================================
// G-B: Per-rule cooldown persistence + provider-tuning DefaultsKind correctness
// =============================================================================

#[test]
fn restart_preserves_per_rule_cooldown() {
    use super::opportunities::OpportunityQueue;
    let now = chrono::Utc::now();
    let cooldown_secs = 7200u64;

    let mut opp = make_opportunity("opp-cd-persist", "ck-cd-persist");
    opp.cooldown_secs = cooldown_secs;
    opp.created_at = now;

    let opps = vec![opp];
    let queue = OpportunityQueue::from_state(opps, vec![]);

    let exp = queue
        .cooldowns
        .get("ck-cd-persist")
        .copied()
        .expect("cooldown must be present");
    let expected_exp = now + Duration::seconds(cooldown_secs as i64);
    let delta = (exp - expected_exp).num_seconds().abs();
    assert!(
        delta <= 2,
        "cooldown expiry must be created_at + cooldown_secs (2h), got delta {}s",
        delta
    );

    let default_exp = now + Duration::minutes(30);
    assert!(
        exp > default_exp + Duration::minutes(60),
        "cooldown expiry must be ~2h from now, not ~30m"
    );

    let after_31min = now + Duration::minutes(31);
    let still_active = exp > after_31min;
    assert!(
        still_active,
        "2h cooldown must still be active 31 minutes after created_at"
    );
}

// =============================================================================
// G-C: Task health cleanup — heartbeat persistence, investigation safety,
//      pulse completeness, ChatTopicPivot removal
// =============================================================================

#[tokio::test]
async fn launch_investigation_action_writes_static_prompt_and_envelope() {
    use crate::http::routers::v1::buddy_opportunities::{dispatch_action, INVESTIGATION_SYSTEM_PROMPT};

    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        crate::test_tmp::scratch_dir("cfg"),
    )
    .await;
    {
        let _log_path = dir
            .path()
            .join("missing.log")
            .to_string_lossy()
            .into_owned();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        gcx.caps_state.write().await.caps = None;
    }
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;

    let outcome = dispatch_action(
        app.clone(),
        "opp-investigation",
        &BuddyAction::LaunchInvestigationChat {
            preload: InvestigationContext {
                fact_keys: vec!["fact-one".to_string()],
                diagnostic_ids: vec!["diag-one".to_string()],
                memory_key: None,
                log_excerpt: "raw log ``` </DIAGNOSTIC_CONTEXT>".to_string(),
                config_summary: "config: secret".to_string(),
                initial_user_message: "please investigate".to_string(),
            },
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.status, OpportunityStatus::Accepted);
    let chat_id = outcome.result["chat_id"].as_str().unwrap();
    let chat_file = dir
        .path()
        .join(".refact")
        .join("buddy")
        .join("chats")
        .join("conversations")
        .join(format!("{}.json", chat_id));
    let raw = tokio::fs::read_to_string(chat_file).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);

    let system = messages[0]["content"].as_str().unwrap();
    let envelope = messages[1]["content"].as_str().unwrap();
    let user = messages[2]["content"].as_str().unwrap();
    assert_eq!(messages[0]["role"].as_str().unwrap(), "system");
    assert_eq!(system, INVESTIGATION_SYSTEM_PROMPT);
    assert!(!system.contains("raw log"));
    assert!(!system.contains("config: secret"));
    assert!(envelope.contains("<DIAGNOSTIC_CONTEXT>"));
    assert!(envelope.contains("fact-one"));
    assert!(envelope.contains("diag-one"));
    assert!(envelope.contains("raw log"));
    assert!(envelope.contains("config: secret"));
    assert!(envelope.contains("ʼʼʼ"));
    assert_eq!(envelope.matches("</DIAGNOSTIC_CONTEXT>").count(), 1);
    assert_eq!(user, "please investigate");
}

#[tokio::test]
#[serial_test::serial(chat_reaction)]
async fn maybe_enqueue_chat_reaction_records_not_queued_and_rolls_back_when_runtime_queue_drops_event(
) {
    use super::chat_reactions::{AcceptedUserMessage, maybe_enqueue_chat_reaction};
    use crate::call_validation::ChatContent;
    use crate::chat::types::ThreadParams;

    let generated = "Tiny queued rollback probe speech.";
    let (service, _renderer) =
        crate::buddy::voice_service::test_voice_service_with_responses(vec![
            Some(generated.to_string()),
            Some(generated.to_string()),
        ]);
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app = make_gcx_with_buddy().await;
    let chat_id = "not-queued-rollback-chat".to_string();
    let content = "please design a private customer architecture carefully".to_string();

    {
        let mut lock = app.buddy.buddy.lock().await;
        lock.as_mut().unwrap().force_next_runtime_enqueue_drop = true;
    }

    maybe_enqueue_chat_reaction(
        app.clone(),
        AcceptedUserMessage {
            chat_id: chat_id.clone(),
            thread: ThreadParams::default(),
            content: ChatContent::SimpleText(content.clone()),
        },
    )
    .await;

    let first_debug = wait_for_chat_reaction_debug_attempt(&app, "not_queued").await;
    let first_attempt = first_debug
        .recent_attempts
        .iter()
        .rev()
        .find(|attempt| attempt.result == "not_queued")
        .expect("not_queued attempt must be recorded");
    assert_eq!(first_attempt.chat_id, chat_id);
    assert_eq!(first_attempt.signal_type.as_deref(), Some("speech_insight"));
    assert_eq!(first_attempt.queued, Some(false));
    assert!(first_attempt.event_id.is_none());
    assert_eq!(first_debug.counts_by_result.get("not_queued"), Some(&1));
    assert!(first_debug.counts_by_result.get("emitted").is_none());
    assert!(!first_debug
        .recent_attempts
        .iter()
        .any(|attempt| attempt.result == "emitted"));
    let first_debug_json = serde_json::to_string(&first_debug).unwrap();
    assert!(!first_debug_json.contains("private customer architecture"));
    assert!(!first_debug_json.contains(generated));
    {
        let lock = app.buddy.buddy.lock().await;
        let runtime_queue = &lock.as_ref().unwrap().runtime_queue;
        assert!(runtime_queue.items.is_empty());
        assert!(runtime_queue.now_playing.is_none());
    }

    maybe_enqueue_chat_reaction(
        app.clone(),
        AcceptedUserMessage {
            chat_id: chat_id.clone(),
            thread: ThreadParams::default(),
            content: ChatContent::SimpleText(content),
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_insight").await;
    assert_eq!(ev.chat_id.as_deref(), Some(chat_id.as_str()));
    assert_eq!(ev.speech_text.as_deref(), Some(generated));
    let second_debug = wait_for_chat_reaction_debug_attempt(&app, "emitted").await;
    assert_eq!(second_debug.counts_by_result.get("not_queued"), Some(&1));
    assert_eq!(second_debug.counts_by_result.get("emitted"), Some(&1));
    let emitted = second_debug
        .recent_attempts
        .iter()
        .rev()
        .find(|attempt| attempt.result == "emitted")
        .expect("retry must emit after rollback");
    assert_eq!(emitted.chat_id, chat_id);
    assert_eq!(emitted.event_id.as_deref(), Some(ev.id.as_str()));
    assert_eq!(emitted.queued, Some(true));
}

#[tokio::test]
async fn chat_reaction_runtime_queue_jsonl_round_trips_frontend_fields() {
    use super::chat_reactions::{build_reaction_event, ChatReaction, ChatReactionKind};
    use super::storage::{append_runtime_record, load_runtime_queue, RuntimeQueueRecord};
    use crate::buddy::types::BuddyBubblePolicy;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let reaction = ChatReaction {
        kind: ChatReactionKind::Humor,
        text: "Pixel gremlin put a tiny hat on this iteration.".to_string(),
    };
    let event = build_reaction_event(
        "chat-jsonl-reaction",
        "can you tweak that and make it simpler without quoting raw text",
        &reaction,
    );

    append_runtime_record(
        root,
        &RuntimeQueueRecord::Event {
            event: event.clone(),
        },
    )
    .await
    .unwrap();

    let raw = tokio::fs::read_to_string(root.join(".refact/buddy/runtime_queue.jsonl"))
        .await
        .unwrap();
    let line: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
    let json_event = &line["event"];
    assert_eq!(line["kind"].as_str(), Some("event"));
    assert_eq!(json_event["source"].as_str(), Some("chat_reactions"));
    assert_eq!(json_event["chat_id"].as_str(), Some("chat-jsonl-reaction"));
    assert_eq!(json_event["signal_type"].as_str(), Some("speech_humor"));
    assert_eq!(
        json_event["speech_text"].as_str(),
        Some(reaction.text.as_str())
    );
    assert_eq!(json_event["ttl_ms"].as_u64(), Some(90_000));
    assert_eq!(json_event["bubble_policy"].as_str(), Some("ambient"));

    let queue = load_runtime_queue(root).await;
    let restored = queue
        .items
        .front()
        .expect("chat reaction event must replay");
    assert_eq!(restored.id, event.id);
    assert_eq!(restored.source, "chat_reactions");
    assert_eq!(restored.chat_id.as_deref(), Some("chat-jsonl-reaction"));
    assert_eq!(
        restored.speech_text.as_deref(),
        Some(reaction.text.as_str())
    );
    assert_eq!(restored.ttl_ms, Some(90_000));
    assert_eq!(restored.bubble_policy, Some(BuddyBubblePolicy::Ambient));
    assert!(restored.controls.is_empty());
}

async fn wait_for_runtime_event(
    app: &AppState,
    signal_type: &str,
) -> super::types::BuddyRuntimeEvent {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            {
                let lock = app.buddy.buddy.lock().await;
                if let Some(event) = lock.as_ref().and_then(|svc| {
                    svc.runtime_queue
                        .items
                        .iter()
                        .find(|event| event.signal_type == signal_type)
                        .cloned()
                }) {
                    return event;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("runtime event {signal_type} did not arrive"))
}

async fn wait_for_chat_reaction_debug_attempt(
    app: &AppState,
    result: &str,
) -> super::snapshot::ChatReactionDebug {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            {
                let lock = app.buddy.buddy.lock().await;
                if let Some(debug) = lock.as_ref().map(|svc| svc.chat_reaction_debug.snapshot()) {
                    if debug
                        .recent_attempts
                        .iter()
                        .any(|attempt| attempt.result == result)
                    {
                        return debug;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("chat reaction debug attempt {result} did not arrive"))
}

#[test]
fn chat_reaction_limiter_allows_moderate_periodic_reactions_during_hour() {
    use super::chat_reactions::{
        ChatReactionKind, ChatReactionLimiter, ChatReactionSkipReason, GLOBAL_HOURLY_CAP,
        PER_CHAT_COOLDOWN_SECS,
    };

    assert_eq!(PER_CHAT_COOLDOWN_SECS, 180);
    assert_eq!(GLOBAL_HOURLY_CAP, 20);

    let mut limiter = ChatReactionLimiter::new();
    let now = chrono::Utc::now();
    for i in 0..GLOBAL_HOURLY_CAP {
        let allowed = limiter.try_allow_kind(
            "moderate-chat",
            ChatReactionKind::Ambient,
            now + Duration::seconds(i64::from(i) * PER_CHAT_COOLDOWN_SECS),
        );
        assert!(allowed.is_ok(), "moderate reaction {i} must be allowed");
    }

    assert_eq!(
        limiter.try_allow_kind(
            "another-chat",
            ChatReactionKind::Ambient,
            now + Duration::seconds(3599),
        ),
        Err(ChatReactionSkipReason::RateLimited)
    );
    assert!(limiter
        .try_allow_kind(
            "moderate-chat",
            ChatReactionKind::Ambient,
            now + Duration::seconds(3601),
        )
        .is_ok());
}

#[test]
fn bug_reaction_not_blocked_by_ambient_cooldown() {
    use super::chat_reactions::{ChatReactionKind, ChatReactionLimiter, ChatReactionSkipReason};

    let mut limiter = ChatReactionLimiter::new();
    let now = chrono::Utc::now();
    assert!(limiter
        .try_allow_kind("bug-independent-chat", ChatReactionKind::Ambient, now)
        .is_ok());
    assert!(limiter
        .try_allow_kind(
            "bug-independent-chat",
            ChatReactionKind::BugCandidate,
            now + Duration::seconds(10),
        )
        .is_ok());
    assert_eq!(
        limiter.try_allow_kind(
            "bug-independent-chat",
            ChatReactionKind::Ambient,
            now + Duration::seconds(10),
        ),
        Err(ChatReactionSkipReason::RateLimited)
    );
}

#[tokio::test]
async fn maybe_enqueue_chat_activity_reaction_records_not_queued_and_rolls_back_when_runtime_queue_drops_event(
) {
    use super::chat_reactions::{ChatActivityCompletion, maybe_enqueue_chat_activity_reaction};
    use crate::chat::types::ThreadParams;

    let app = make_gcx_with_buddy().await;
    let chat_id = "not-queued-activity-rollback-chat".to_string();
    let mut rx = {
        let mut lock = app.buddy.buddy.lock().await;
        let svc = lock.as_mut().unwrap();
        svc.force_next_runtime_enqueue_drop = true;
        svc.events_tx.subscribe()
    };

    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: chat_id.clone(),
            thread: ThreadParams::default(),
        },
    )
    .await;

    let first_debug = wait_for_chat_reaction_debug_attempt(&app, "not_queued").await;
    let first_attempt = first_debug
        .recent_attempts
        .iter()
        .rev()
        .find(|attempt| attempt.result == "not_queued")
        .expect("not_queued activity attempt must be recorded");
    assert_eq!(first_attempt.chat_id, chat_id);
    assert_eq!(
        first_attempt.signal_type.as_deref(),
        Some("speech_chat_reaction")
    );
    assert_eq!(first_attempt.queued, Some(false));
    assert!(first_attempt.event_id.is_none());
    assert_eq!(first_debug.counts_by_result.get("not_queued"), Some(&1));
    assert!(first_debug.counts_by_result.get("emitted").is_none());
    assert!(!first_debug
        .recent_attempts
        .iter()
        .any(|attempt| attempt.result == "emitted"));
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, super::events::BuddyEvent::RuntimeEvent { .. }),
            "dropped activity reaction must not broadcast a runtime event"
        );
    }
    {
        let lock = app.buddy.buddy.lock().await;
        let svc = lock.as_ref().unwrap();
        assert!(svc.runtime_queue.items.is_empty());
        assert!(svc.runtime_queue.now_playing.is_none());
    }

    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: chat_id.clone(),
            thread: ThreadParams::default(),
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_chat_reaction").await;
    assert_eq!(ev.chat_id.as_deref(), Some(chat_id.as_str()));
    let second_debug = wait_for_chat_reaction_debug_attempt(&app, "emitted").await;
    assert_eq!(second_debug.counts_by_result.get("not_queued"), Some(&1));
    assert_eq!(second_debug.counts_by_result.get("emitted"), Some(&1));
    let emitted = second_debug
        .recent_attempts
        .iter()
        .rev()
        .find(|attempt| attempt.result == "emitted")
        .expect("activity retry must emit after rollback");
    assert_eq!(emitted.chat_id, chat_id);
    assert_eq!(emitted.signal_type.as_deref(), Some("speech_chat_reaction"));
    assert_eq!(emitted.event_id.as_deref(), Some(ev.id.as_str()));
    assert_eq!(emitted.queued, Some(true));
}

#[tokio::test]
async fn chat_activity_reaction_emits_after_normal_chat_activity_or_completion() {
    use super::chat_reactions::{
        ChatActivityCompletion, AMBIENT_LINES, maybe_enqueue_chat_activity_reaction,
    };
    use crate::buddy::types::BuddyBubblePolicy;
    use crate::chat::types::ThreadParams;

    let (service, renderer) =
        crate::buddy::voice_service::test_voice_service_with_responses(vec![Some(
            "voice service should not be used".to_string(),
        )]);
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app = make_gcx_with_buddy().await;

    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: "activity-reaction-chat".to_string(),
            thread: ThreadParams::default(),
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_chat_reaction").await;
    assert_eq!(ev.source, "chat_reactions");
    assert_eq!(ev.chat_id.as_deref(), Some("activity-reaction-chat"));
    assert_eq!(ev.bubble_policy, Some(BuddyBubblePolicy::Ambient));
    let speech = ev.speech_text.as_deref().unwrap_or_default();
    assert!(
        AMBIENT_LINES.contains(&speech),
        "activity reaction must use ambient fallback, got: {speech}"
    );
    assert!(renderer.intent_kinds().is_empty());
}

#[tokio::test]
async fn chat_activity_reaction_skips_planner_and_task_threads() {
    use super::chat_reactions::{ChatActivityCompletion, maybe_enqueue_chat_activity_reaction};
    use crate::chat::types::{TaskMeta as ChatTaskMeta, ThreadParams};

    let app = make_gcx_with_buddy().await;
    let mut planner = ThreadParams::default();
    planner.mode = "task_planner".to_string();
    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: "planner-activity-chat".to_string(),
            thread: planner,
        },
    )
    .await;

    let mut task = ThreadParams::default();
    task.task_meta = Some(ChatTaskMeta::default());
    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: "task-activity-chat".to_string(),
            thread: task,
        },
    )
    .await;

    let lock = app.buddy.buddy.lock().await;
    let svc = lock.as_ref().unwrap();
    assert!(svc.runtime_queue.items.is_empty());
    let debug = svc.chat_reaction_debug.snapshot();
    let filtered = debug
        .recent_attempts
        .iter()
        .filter(|attempt| attempt.skip_reason.as_deref() == Some("thread_filtered"))
        .count();
    assert_eq!(filtered, 2);
}

#[tokio::test]
async fn chat_activity_reaction_uses_safe_fallback_without_user_echo() {
    use super::chat_reactions::{
        ChatActivityCompletion, AMBIENT_LINES, maybe_enqueue_chat_activity_reaction,
    };
    use crate::chat::types::ThreadParams;

    let app = make_gcx_with_buddy().await;
    let mut thread = ThreadParams::default();
    thread.title = "Acme customer roadmap token=PRIVATESECRET".to_string();

    maybe_enqueue_chat_activity_reaction(
        app.clone(),
        ChatActivityCompletion {
            chat_id: "safe-activity-reaction-chat".to_string(),
            thread,
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_chat_reaction").await;
    let speech = ev.speech_text.as_deref().unwrap_or_default();
    assert!(AMBIENT_LINES.contains(&speech));
    let serialized = serde_json::to_string(&ev).unwrap();
    for private in ["Acme", "customer roadmap", "PRIVATESECRET"] {
        assert!(
            !serialized.contains(private),
            "private text leaked: {private}"
        );
    }
}

#[serial_test::serial(chat_reaction)]
#[tokio::test]
async fn chat_reaction_generated_text_is_redacted_before_storage() {
    use super::chat_reactions::{AcceptedUserMessage, maybe_enqueue_chat_reaction};
    use crate::call_validation::ChatContent;
    use crate::chat::types::ThreadParams;

    let (service, _renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
        vec![Some("hey use sk-FAKE12345 to test".to_string())],
    );
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app = make_gcx_with_buddy().await;

    maybe_enqueue_chat_reaction(
        app.clone(),
        AcceptedUserMessage {
            chat_id: "redacted-reaction-chat".to_string(),
            thread: ThreadParams::default(),
            content: ChatContent::SimpleText(
                "design a security architecture review today".to_string(),
            ),
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_insight").await;
    let speech = ev.speech_text.as_deref().unwrap_or_default();
    assert!(!speech.contains("FAKE12345"));
    assert!(!speech.contains("sk-FAKE12345"));
}

#[tokio::test]
#[serial_test::serial(chat_reaction)]
async fn maybe_enqueue_chat_reaction_hard_settings_block_events() {
    use super::chat_reactions::{AcceptedUserMessage, maybe_enqueue_chat_reaction};
    use crate::call_validation::ChatContent;
    use crate::chat::types::ThreadParams;

    let blockers: [fn(&mut BuddySettings); 3] = [
        |settings| settings.chat_reactions_enabled = false,
        |settings| settings.enabled = false,
        |settings| settings.quiet_mode = true,
    ];
    for configure in blockers {
        let app = make_gcx_with_buddy().await;
        {
            let mut lock = app.buddy.buddy.lock().await;
            configure(&mut lock.as_mut().unwrap().settings);
        }

        maybe_enqueue_chat_reaction(
            app.clone(),
            AcceptedUserMessage {
                chat_id: "settings-blocked-reaction-chat".to_string(),
                thread: ThreadParams::default(),
                content: ChatContent::SimpleText(
                    "please implement the schema cleanup for this api response".to_string(),
                ),
            },
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let lock = app.buddy.buddy.lock().await;
        assert!(
            lock.as_ref().unwrap().runtime_queue.items.is_empty(),
            "hard settings gate must block live chat reaction events"
        );
    }
}

#[test]
fn runtime_event_coalesce_broadcasts_stored_id() {
    use super::state::default_buddy_state;
    use super::settings::BuddySettings;

    let (tx, mut rx) = broadcast::channel(16);
    let mut svc = BuddyService::new(
        std::env::temp_dir().join(format!("buddy-test-{}", uuid::Uuid::new_v4())),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );

    let mut ev_a = super::actor::make_runtime_event("signal", "A", "src", "k", "started", None);
    ev_a.id = "id-A".to_string();
    svc.enqueue_runtime_event(ev_a);

    let broadcast_a = rx.try_recv().unwrap();
    match broadcast_a {
        super::events::BuddyEvent::RuntimeEvent { event } => {
            assert_eq!(event.id, "id-A");
        }
        other => panic!("expected RuntimeEvent, got {:?}", other),
    }

    let mut ev_b = super::actor::make_runtime_event("signal", "B", "src", "k", "started", None);
    ev_b.id = "id-B".to_string();
    svc.enqueue_runtime_event(ev_b);

    let broadcast_b = rx.try_recv().unwrap();
    match broadcast_b {
        super::events::BuddyEvent::RuntimeEvent { event } => {
            assert_eq!(
                event.id, "id-A",
                "coalesced broadcast must carry the stored id"
            );
        }
        other => panic!("expected RuntimeEvent, got {:?}", other),
    }

    assert!(
        svc.dismiss_runtime_event_by_id("id-A"),
        "dismiss by id-A must succeed after coalesce"
    );
}

#[tokio::test]
#[serial_test::serial(chat_reaction)]
async fn maybe_enqueue_chat_reaction_rolls_back_limiter_when_enqueue_skipped() {
    use super::chat_reactions::{AcceptedUserMessage, maybe_enqueue_chat_reaction};
    use crate::call_validation::ChatContent;
    use crate::chat::types::ThreadParams;

    let (service, _renderer) =
        crate::buddy::voice_service::test_voice_service_with_delayed_responses(
            vec![
                Some("first insight".to_string()),
                Some("second insight".to_string()),
            ],
            std::time::Duration::from_millis(100),
        );
    let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
    let app = make_gcx_with_buddy().await;
    let chat_id = "rollback-reaction-chat".to_string();

    maybe_enqueue_chat_reaction(
        app.clone(),
        AcceptedUserMessage {
            chat_id: chat_id.clone(),
            thread: ThreadParams::default(),
            content: ChatContent::SimpleText(
                "please design a cache architecture for services".to_string(),
            ),
        },
    )
    .await;

    {
        let mut lock = app.buddy.buddy.lock().await;
        lock.as_mut().unwrap().settings.chat_reactions_enabled = false;
    }

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let rolled_back = {
                let lock = app.buddy.buddy.lock().await;
                let svc = lock.as_ref().unwrap();
                svc.chat_reaction_debug
                    .snapshot()
                    .last_skip_reason
                    .as_deref()
                    == Some("settings_disabled")
                    && svc.runtime_queue.items.is_empty()
            };
            if rolled_back {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first reaction should skip after settings are disabled");

    {
        let mut lock = app.buddy.buddy.lock().await;
        lock.as_mut().unwrap().settings.chat_reactions_enabled = true;
    }

    maybe_enqueue_chat_reaction(
        app.clone(),
        AcceptedUserMessage {
            chat_id,
            thread: ThreadParams::default(),
            content: ChatContent::SimpleText(
                "please design a cache architecture for services again".to_string(),
            ),
        },
    )
    .await;

    let ev = wait_for_runtime_event(&app, "speech_insight").await;
    assert_eq!(ev.speech_text.as_deref(), Some("second insight"));
}

#[test]
fn muted_chat_strips_runtime_event_speech() {
    let mut svc = make_service();
    svc.settings.quiet_hours_mode = crate::buddy::settings::QuietHoursMode::Off;
    svc.settings.muted_chat_ids = vec!["chat-mute".to_string()];
    let mut event = crate::buddy::actor::make_runtime_event(
        "speech_insight",
        "Chat: insight",
        "chat_reactions",
        "chat_reaction:chat-mute:insight:xyz",
        "info",
        None,
    );
    event.speech_text = Some("quiet please".to_string());
    event.chat_id = Some("chat-mute".to_string());

    let stored = svc.enqueue_runtime_event_with_stored(event).unwrap();
    assert_eq!(stored.speech_text, None);

    let mut other = crate::buddy::actor::make_runtime_event(
        "speech_insight",
        "Chat: insight",
        "chat_reactions",
        "chat_reaction:other:insight:xyz",
        "info",
        None,
    );
    other.speech_text = Some("loud and proud".to_string());
    other.chat_id = Some("other".to_string());
    let stored_other = svc.enqueue_runtime_event_with_stored(other).unwrap();
    assert!(stored_other.speech_text.is_some());
}

#[test]
fn llm_spend_tracks_daily_budget() {
    let mut svc = make_service();
    svc.settings.daily_llm_token_budget = Some(1000);
    assert!(!svc.llm_budget_exhausted());
    svc.record_workflow_telemetry("job_a", 2, 700, 200, true);
    assert!(!svc.llm_budget_exhausted());
    svc.record_workflow_telemetry("job_b", 1, 100, 50, false);
    assert!(svc.llm_budget_exhausted());
    svc.settings.daily_llm_token_budget = None;
    assert!(!svc.llm_budget_exhausted());
}
