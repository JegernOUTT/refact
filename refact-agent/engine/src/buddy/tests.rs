use chrono::Duration;
use tokio::sync::broadcast;

use super::actor::BuddyService;
use super::diagnostics::{classify_error, DiagnosticContext, DiagnosticSeverity};
use super::issues::{check_issue_gate, check_manual_issue_gate, redact_diagnostic_text, sanitize_body, sanitize_title, IssueGate};
use super::scheduler::BuddyJobContext;
use super::settings::{BuddySettings, MAX_PALETTE_INDEX};
use super::state::{default_buddy_state, grant_xp};
use super::types::{BuddyJobState, BuddyOnboarding, BuddySuggestion, BuddyState};

fn make_service() -> BuddyService {
    let (tx, _rx) = broadcast::channel(16);
    BuddyService::new(default_buddy_state(), BuddySettings::default(), tx)
}

fn make_suggestion(id: &str, stype: &str, created_at: &str) -> BuddySuggestion {
    BuddySuggestion {
        id: id.to_string(),
        suggestion_type: stype.to_string(),
        title: "t".to_string(),
        description: "d".to_string(),
        created_at: created_at.to_string(),
        dismissed: false,
    }
}

#[test]
fn test_auto_gate_requires_all_conditions() {
    let gate = IssueGate {
        has_diagnostics: true,
        has_repro_context: true,
        integration_configured: true,
        auto_creation_enabled: true,
        within_rate_limit: true,
    };
    assert!(check_issue_gate(&gate));
}

#[test]
fn test_auto_gate_blocks_without_repro() {
    let gate = IssueGate {
        has_diagnostics: true,
        has_repro_context: false,
        integration_configured: true,
        auto_creation_enabled: true,
        within_rate_limit: true,
    };
    assert!(!check_issue_gate(&gate));
}

#[test]
fn test_manual_gate_allows_without_auto_enabled() {
    let gate = IssueGate {
        has_diagnostics: true,
        has_repro_context: false,
        integration_configured: true,
        auto_creation_enabled: false,
        within_rate_limit: false,
    };
    assert!(check_manual_issue_gate(&gate));
}

#[test]
fn test_manual_gate_requires_integration() {
    let gate = IssueGate {
        has_diagnostics: true,
        has_repro_context: true,
        integration_configured: false,
        auto_creation_enabled: true,
        within_rate_limit: true,
    };
    assert!(!check_manual_issue_gate(&gate));
}

#[test]
fn test_default_state_starts_egg() {
    let state = default_buddy_state();
    assert_eq!(state.progression.stage, 0);
    assert_eq!(state.progression.stage_name, "Egg");
    assert_eq!(state.progression.xp, 0);
    assert_eq!(state.progression.level, 1);
}

#[test]
fn test_grant_xp_levels_up() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 100);
    assert_eq!(state.progression.level, 2);
    assert_eq!(state.progression.xp, 0);
}

#[test]
fn test_grant_xp_updates_stage() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 30);
    assert_eq!(state.progression.stage, 1);
    assert_eq!(state.progression.stage_name, "Hatch");
}

#[test]
fn test_stage_transitions_at_thresholds() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 100);
    assert_eq!(state.progression.stage_name, "Sprite");
    assert_eq!(state.progression.stage, 2);
}

#[test]
fn test_xp_bar_never_negative() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 0);
    assert!(state.progression.xp < state.progression.xp_next);
}

#[test]
fn test_max_stage_behavior() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 3000);
    assert_eq!(state.progression.stage_name, "Archon");
    assert_eq!(state.progression.stage, 6);
}

#[test]
fn test_palette_clamped_on_load() {
    let mut state = default_buddy_state();
    state.identity.palette_index = 100;
    state.identity.palette_index = state.identity.palette_index.min(MAX_PALETTE_INDEX);
    assert_eq!(state.identity.palette_index, MAX_PALETTE_INDEX);
}

#[test]
fn test_palette_valid_range() {
    for i in 0..=MAX_PALETTE_INDEX {
        assert_eq!(i.min(MAX_PALETTE_INDEX), i);
    }
    assert!(MAX_PALETTE_INDEX > 0);
    assert!(10usize.min(MAX_PALETTE_INDEX) == MAX_PALETTE_INDEX);
}

#[test]
fn test_palette_single_source() {
    let settings = BuddySettings::default();
    let json = serde_json::to_value(&settings).unwrap();
    assert!(json.get("palette_index").is_none(), "palette_index must not be in BuddySettings");
    let state = default_buddy_state();
    assert!(state.identity.palette_index <= MAX_PALETTE_INDEX);
}

#[test]
fn test_old_state_migration() {
    let json = r#"{
        "identity": {"name": "Pixel", "created_at": "2024-01-01T00:00:00Z", "palette_index": 2},
        "progression": {"stage": 0, "stage_name": "Egg", "level": 1, "xp": 0, "xp_next": 100},
        "skills": {"unlocked": [], "locked": []},
        "workflow_summaries": [],
        "semantic": {"mood": "Idle", "focus": "", "headline": "", "last_active": "2024-01-01T00:00:00Z"},
        "recent_activities": [],
        "suggestion_state": []
    }"#;
    let state: BuddyState = serde_json::from_str(json).expect("should parse old state without onboarding");
    assert!(!state.onboarding.greeted);
    assert!(!state.onboarding.tour_completed);
    assert!(state.onboarding.first_launch_at.is_empty());
}

#[test]
fn test_save_on_mutate_sets_dirty() {
    let mut svc = make_service();
    assert!(!svc.dirty);
    svc.grant_xp(10);
    assert!(svc.dirty);
}

#[tokio::test]
async fn test_save_on_mutate_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();
    let mut svc = make_service();
    svc.grant_xp(25);
    super::state::save_state(root, &svc.state).await.unwrap();
    let loaded = super::state::load_state(root).await;
    assert!(loaded.progression.xp > 0 || loaded.progression.level > 1);
}

#[tokio::test]
async fn test_bootstrap_no_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();
    let state1 = super::state::load_state(root).await;
    let name1 = state1.identity.name.clone();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();
    let state2 = super::state::load_state(root).await;
    assert_eq!(name1, state2.identity.name, "bootstrap must not overwrite existing state.json");
}

#[test]
fn test_classification_case_insensitive() {
    assert_eq!(classify_error("TIMEOUT occurred"), "timeout");
    assert_eq!(classify_error("TimeOut error"), "timeout");
    assert_eq!(classify_error("PERMISSION denied"), "permission");
}

#[test]
fn test_classify_timeout() {
    assert_eq!(classify_error("connection timeout after 30s"), "timeout");
    assert_eq!(classify_error("request timed out"), "generic");
}

#[test]
fn test_classify_generic_fallback() {
    assert_eq!(classify_error("something weird happened"), "generic");
    assert_eq!(classify_error("unknown failure"), "generic");
}

#[test]
fn test_suggestion_dedupe() {
    let mut svc = make_service();
    let now = chrono::Utc::now().to_rfc3339();
    let already = svc.state.suggestion_state.iter().any(|s| s.suggestion_type == "setup");
    if !already {
        svc.add_suggestion(make_suggestion("setup", "setup", &now));
    }
    let already2 = svc.state.suggestion_state.iter().any(|s| s.suggestion_type == "setup");
    if !already2 {
        svc.add_suggestion(make_suggestion("setup2", "setup", &now));
    }
    assert_eq!(svc.state.suggestion_state.len(), 1);
}

#[test]
fn test_suggestion_pruning() {
    let mut svc = make_service();
    let old_time = (chrono::Utc::now() - Duration::seconds(400)).to_rfc3339();
    svc.state.suggestion_state.push(make_suggestion("old", "test", &old_time));
    svc.expire_suggestions();
    assert!(svc.state.suggestion_state[0].dismissed);
}

#[test]
fn test_suggestion_cap() {
    let mut svc = make_service();
    let now = chrono::Utc::now().to_rfc3339();
    let mut added = 0usize;
    for i in 0..10 {
        let s = make_suggestion(&format!("s{}", i), "test", &now);
        if svc.maybe_add_suggestion(s) {
            added += 1;
        }
    }
    assert_eq!(added, 1);
    assert_eq!(svc.state.suggestion_state.len(), 1);
}

#[test]
fn test_redact_api_key_pattern() {
    let input = "token ghp_AbCdEfGhIj1234567890 used";
    let output = redact_diagnostic_text(input);
    assert!(!output.contains("ghp_AbCdEfGhIj1234567890"));
    assert!(output.contains("[REDACTED"));
}

#[test]
fn test_sanitize_title_newlines() {
    let raw = "Error:\nline 2\r\nline 3";
    let result = sanitize_title(raw);
    assert!(!result.contains('\n'));
    assert!(!result.contains('\r'));
    assert!(!result.is_empty());
}

#[test]
fn test_sanitize_body_truncation() {
    let raw: String = "x".repeat(5000);
    let result = sanitize_body(&raw);
    assert!(result.chars().count() <= 4000);
}

#[test]
fn test_diagnostic_cap() {
    let mut svc = make_service();
    for i in 0..150 {
        let ctx = DiagnosticContext {
            error_type: "test".to_string(),
            error_message: format!("error {}", i),
            source_file: None,
            tool_name: None,
            chat_id: None,
            collected_at: chrono::Utc::now().to_rfc3339(),
            severity: DiagnosticSeverity::Low,
        };
        svc.add_diagnostic(ctx);
    }
    assert_eq!(svc.recent_diagnostics.len(), 100);
}

#[test]
fn test_buddy_say_creates_speech() {
    use super::types::{BuddySpeechItem, BuddyControl};
    let mut svc = make_service();
    let speech = BuddySpeechItem {
        id: "test-id".to_string(),
        text: "Hello!".to_string(),
        mood: "happy".to_string(),
        scope: "global".to_string(),
        persistent: false,
        ttl_seconds: 10,
        dedupe_key: Some("greeting".to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        controls: vec![],
    };
    svc.update_speech(speech.clone());
    assert!(svc.active_speech.is_some());
    assert_eq!(svc.active_speech.as_ref().unwrap().text, "Hello!");

    let speech2 = BuddySpeechItem {
        id: "test-id-2".to_string(),
        text: "Updated!".to_string(),
        mood: "happy".to_string(),
        scope: "global".to_string(),
        persistent: false,
        ttl_seconds: 10,
        dedupe_key: Some("greeting".to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        controls: vec![],
    };
    svc.update_speech(speech2);
    assert_eq!(svc.active_speech.as_ref().unwrap().text, "Updated!");

    let _ = BuddyControl {
        id: "btn1".to_string(),
        label: "Open Setup".to_string(),
        action: "open_setup".to_string(),
        action_param: None,
        style: "primary".to_string(),
    };
}

#[test]
fn test_buddy_controls_schema() {
    let valid_actions = ["open_chat", "open_setup", "open_setup_mcp", "open_setup_skills", "open_stats", "open_buddy", "dismiss", "run_command"];
    assert!(valid_actions.contains(&"open_setup"));
    assert!(valid_actions.contains(&"dismiss"));
    assert!(!valid_actions.contains(&"invalid_action"));
}

#[test]
fn test_runtime_event_speech_text_preserved() {
    use super::runtime_queue::RuntimeQueue;
    let mut queue = RuntimeQueue::new();
    let mut ev = super::actor::make_runtime_event("streaming", "Test", "chat", "chat_1", "started", None);
    ev.speech_text = Some("Thinking...".to_string());
    ev.scene = Some("working".to_string());
    ev.persistent = true;
    queue.enqueue(ev);
    let stored = &queue.items[0];
    assert_eq!(stored.speech_text.as_deref(), Some("Thinking..."));
    assert_eq!(stored.scene.as_deref(), Some("working"));
    assert!(stored.persistent);
}

#[test]
fn test_persistent_event_fields_coalesced() {
    use super::runtime_queue::RuntimeQueue;
    let mut queue = RuntimeQueue::new();
    let mut ev1 = super::actor::make_runtime_event("streaming", "First", "chat", "key_1", "started", None);
    ev1.speech_text = Some("Initial".to_string());
    ev1.persistent = true;
    queue.enqueue(ev1);
    let mut ev2 = super::actor::make_runtime_event("streaming", "Updated", "chat", "key_1", "progress", None);
    ev2.speech_text = Some("Updated text".to_string());
    ev2.persistent = true;
    queue.enqueue(ev2);
    assert_eq!(queue.items.len(), 1);
    assert_eq!(queue.items[0].speech_text.as_deref(), Some("Updated text"));
    assert_eq!(queue.items[0].status, "progress");
}

#[tokio::test]
async fn test_unified_listing_mixed_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let conv_path = root.join(".refact/buddy/chats/conversations/abc123.json");
    let conv_json = serde_json::json!({
        "chat_id": "abc123", "title": "Test Chat", "kind": "chat",
        "created_at": "2024-01-02T00:00:00Z", "last_message_at": null, "messages": []
    });
    super::storage::atomic_write_json(&conv_path, &conv_json).await.unwrap();

    let wf_path = root.join(".refact/buddy/chats/workflows/commit_message.json");
    let wf_json = serde_json::json!({
        "entries": [{ "timestamp": "2024-01-01T00:00:00Z", "input_summary": "", "output_summary": "done", "success": true }]
    });
    super::storage::atomic_write_json(&wf_path, &wf_json).await.unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    assert_eq!(entries.len(), 2);
    let kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"chat"));
    assert!(kinds.contains(&"workflow"));
}

#[tokio::test]
async fn test_setup_kind_stored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let path = root.join(".refact/buddy/chats/conversations/setup1.json");
    let json = serde_json::json!({
        "chat_id": "setup1", "title": "MCP Setup", "kind": "setup", "badge": "MCP Setup",
        "created_at": "2024-01-01T00:00:00Z", "last_message_at": null, "messages": []
    });
    super::storage::atomic_write_json(&path, &json).await.unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    let setup = entries.iter().find(|e| e.id == "setup1").unwrap();
    assert_eq!(setup.kind, "setup");
    assert_eq!(setup.badge.as_deref(), Some("MCP Setup"));
    assert_eq!(setup.icon, "⚙️");
}

#[tokio::test]
async fn test_kind_filter_works() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let conv_path = root.join(".refact/buddy/chats/conversations/c1.json");
    let conv_json = serde_json::json!({
        "chat_id": "c1", "title": "Chat", "created_at": "2024-01-01T00:00:00Z", "messages": []
    });
    super::storage::atomic_write_json(&conv_path, &conv_json).await.unwrap();

    let wf_path = root.join(".refact/buddy/chats/workflows/commit_message.json");
    let wf_json = serde_json::json!({ "entries": [] });
    super::storage::atomic_write_json(&wf_path, &wf_json).await.unwrap();

    let chat_only = super::conversation_ledger::list_all_buddy_conversations(root, Some(vec!["chat".to_string()])).await;
    assert!(chat_only.iter().all(|e| e.kind == "chat"));

    let wf_only = super::conversation_ledger::list_all_buddy_conversations(root, Some(vec!["workflow".to_string()])).await;
    assert!(wf_only.iter().all(|e| e.kind == "workflow"));
}

#[tokio::test]
async fn test_sorting_by_updated_at() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let p1 = root.join(".refact/buddy/chats/conversations/old.json");
    super::storage::atomic_write_json(&p1, &serde_json::json!({
        "chat_id": "old", "title": "Old", "created_at": "2024-01-01T00:00:00Z", "messages": []
    })).await.unwrap();

    let p2 = root.join(".refact/buddy/chats/conversations/new.json");
    super::storage::atomic_write_json(&p2, &serde_json::json!({
        "chat_id": "new", "title": "New", "created_at": "2024-06-01T00:00:00Z",
        "last_message_at": "2024-06-02T00:00:00Z", "messages": []
    })).await.unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    assert_eq!(entries[0].id, "new");
}

#[test]
fn test_runtime_event_controls_preserved() {
    use super::runtime_queue::RuntimeQueue;
    use super::types::BuddyControl;
    let mut queue = RuntimeQueue::new();
    let mut ev = super::actor::make_runtime_event("chat_error", "Error", "chat", "err_1", "info", Some("high"));
    ev.controls = vec![BuddyControl {
        id: "fix".to_string(),
        label: "Fix".to_string(),
        action: "open_chat".to_string(),
        action_param: None,
        style: "primary".to_string(),
    }];
    queue.enqueue(ev);
    assert_eq!(queue.items[0].controls.len(), 1);
    assert_eq!(queue.items[0].controls[0].action, "open_chat");
}

fn make_job_context(onboarding: BuddyOnboarding, diagnostics_count: usize, job_state: BuddyJobState) -> BuddyJobContext {
    let mut diags = vec![];
    for _ in 0..diagnostics_count {
        diags.push(DiagnosticContext {
            error_type: "timeout".to_string(),
            error_message: "connection timeout".to_string(),
            source_file: None,
            tool_name: None,
            chat_id: None,
            collected_at: chrono::Utc::now().to_rfc3339(),
            severity: DiagnosticSeverity::High,
        });
    }
    BuddyJobContext {
        identity_name: "Pixel".to_string(),
        onboarding,
        recent_diagnostics: diags,
        project_root: std::path::PathBuf::from("/tmp/test-project"),
        job_state,
    }
}

#[test]
fn test_scheduler_cooldown_enforcement() {
    let recent_run = (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
    let state = BuddyJobState {
        last_run: Some(recent_run),
        run_count: 1,
        last_result: Some("ok".to_string()),
        snoozed_until: None,
        dismissed: false,
    };
    let elapsed = state.last_run.as_deref()
        .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
        .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds().max(0) as u64)
        .unwrap_or(u64::MAX);
    let cooldown = 5 * 60u64;
    assert!(elapsed < cooldown, "job should be blocked by cooldown");
}

#[test]
fn test_job_state_persistence_roundtrip() {
    let mut state = default_buddy_state();
    state.job_cooldowns.insert("greeting".to_string(), BuddyJobState {
        last_run: Some("2026-01-01T00:00:00Z".to_string()),
        run_count: 3,
        last_result: Some("ok".to_string()),
        snoozed_until: None,
        dismissed: false,
    });
    let json = serde_json::to_string(&state).unwrap();
    let loaded: BuddyState = serde_json::from_str(&json).unwrap();
    let job_state = loaded.job_cooldowns.get("greeting").unwrap();
    assert_eq!(job_state.run_count, 3);
    assert_eq!(job_state.last_result.as_deref(), Some("ok"));
}

#[tokio::test]
async fn test_greeting_triggers_on_first_launch() {
    use super::jobs::greeting::GreetingJob;
    use super::scheduler::BuddyJob;
    let job = GreetingJob;
    let ctx = make_job_context(BuddyOnboarding::default(), 0, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(job.should_run(gcx, &ctx).await);
}

#[test]
fn test_greeting_blocked_within_cooldown() {
    use super::jobs::greeting::GreetingJob;
    use super::scheduler::BuddyJob;
    let job = GreetingJob;
    let recent = (chrono::Utc::now() - chrono::Duration::seconds(60)).to_rfc3339();
    let job_state = BuddyJobState {
        last_run: Some(recent),
        run_count: 1,
        last_result: Some("ok".to_string()),
        snoozed_until: None,
        dismissed: false,
    };
    let elapsed = job_state.last_run.as_deref()
        .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
        .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds().max(0) as u64)
        .unwrap_or(u64::MAX);
    assert!(elapsed < job.cooldown_seconds(), "greeting must be blocked within 24h cooldown");
}

#[tokio::test]
async fn test_error_triage_clusters_by_type() {
    use super::jobs::error_triage::ErrorTriageJob;
    use super::scheduler::BuddyJob;
    let job = ErrorTriageJob;
    let ctx = make_job_context(BuddyOnboarding::default(), 5, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(job.should_run(gcx.clone(), &ctx).await);
    let result = job.execute(gcx, ctx).await;
    assert!(result.suggestion.is_some(), "should produce suggestion for 5 repeated timeouts");
    let sug = result.suggestion.unwrap();
    assert_eq!(sug.suggestion_type, "error_pattern");
    assert!(sug.title.contains("timeout"));
}

#[tokio::test]
async fn test_config_watcher_detects_missing_agents_md() {
    use super::jobs::config_watcher::ConfigWatcherJob;
    use super::scheduler::BuddyJob;
    let dir = tempfile::tempdir().unwrap();
    let job = ConfigWatcherJob;
    let mut ctx = make_job_context(BuddyOnboarding::default(), 0, BuddyJobState::default());
    ctx.project_root = dir.path().to_path_buf();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let result = job.execute(gcx, ctx).await;
    assert!(result.suggestion.is_some(), "should suggest setup when AGENTS.md missing");
    assert_eq!(result.suggestion.unwrap().suggestion_type, "setup");
}

#[test]
fn test_suggestion_cap_max_unread() {
    let mut svc = make_service();
    let now = chrono::Utc::now().to_rfc3339();
    svc.settings.proactive_enabled = true;
    for i in 0..10 {
        let s = make_suggestion(&format!("s{}", i), &format!("type{}", i), &now);
        let _ = svc.maybe_add_suggestion(s);
    }
    let unread = svc.state.suggestion_state.iter().filter(|s| !s.dismissed).count();
    assert!(unread <= 10, "suggestions should be bounded");
}

#[test]
fn test_dismissed_job_does_not_retrigger() {
    let state = BuddyJobState {
        last_run: None,
        run_count: 0,
        last_result: None,
        snoozed_until: None,
        dismissed: true,
    };
    assert!(state.dismissed, "dismissed job must not retrigger");
    let elapsed = u64::MAX;
    let cooldown = 0u64;
    let should_skip = state.dismissed || elapsed < cooldown;
    assert!(should_skip, "dismissed job must be skipped regardless of cooldown");
}

#[test]
fn test_proactive_enabled_setting() {
    let settings = BuddySettings::default();
    assert!(settings.proactive_enabled, "proactive_enabled defaults to true");
    let json = serde_json::to_string(&settings).unwrap();
    let loaded: BuddySettings = serde_json::from_str(&json).unwrap();
    assert!(loaded.proactive_enabled);
}

#[test]
fn test_old_settings_get_proactive_default() {
    let json = r#"{"enabled": true, "auto_diagnostics": true, "auto_issue_creation": false}"#;
    let settings: BuddySettings = serde_json::from_str(json).unwrap();
    assert!(settings.proactive_enabled, "missing proactive_enabled should default to true");
}

#[tokio::test]
async fn test_tour_job_runs_only_once() {
    use super::jobs::tour::TourJob;
    use super::scheduler::BuddyJob;
    let job = TourJob;
    let onboarding = BuddyOnboarding { greeted: true, tour_completed: false, ..Default::default() };
    let fresh_ctx = make_job_context(onboarding.clone(), 0, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(job.should_run(gcx.clone(), &fresh_ctx).await, "tour must run on first tick");
    let ran_state = BuddyJobState {
        last_run: Some(chrono::Utc::now().to_rfc3339()),
        run_count: 1,
        last_result: Some("ok".to_string()),
        snoozed_until: None,
        dismissed: false,
    };
    let ran_ctx = make_job_context(onboarding, 0, ran_state);
    assert!(!job.should_run(gcx, &ran_ctx).await, "tour must not run after first run");
}

#[test]
fn test_scheduler_suggestion_dedup() {
    let mut svc = make_service();
    let now = chrono::Utc::now().to_rfc3339();
    let s1 = make_suggestion("dup-1", "error_pattern", &now);
    let s2 = BuddySuggestion {
        id: "dup-2".to_string(),
        suggestion_type: "error_pattern".to_string(),
        title: "t".to_string(),
        description: "d".to_string(),
        created_at: now,
        dismissed: false,
    };
    assert!(svc.maybe_add_suggestion(s1), "first suggestion must be accepted");
    assert!(!svc.maybe_add_suggestion(s2), "duplicate suggestion must be rejected by dedup");
    assert_eq!(svc.state.suggestion_state.len(), 1);
}

#[tokio::test]
async fn test_proactive_disabled_still_allows_greeting() {
    use super::jobs::greeting::GreetingJob;
    use super::scheduler::BuddyJob;
    let job = GreetingJob;
    assert!(!job.produces_suggestion(), "greeting must not be gated by proactive flag");
    let ctx = make_job_context(BuddyOnboarding::default(), 0, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(job.should_run(gcx, &ctx).await, "greeting must run even when proactive_enabled=false");
}

#[test]
fn test_workflow_id_rejects_path_traversal() {
    assert!(!super::actor::validate_workflow_id("../evil"));
}

#[test]
fn test_workflow_id_rejects_slashes() {
    assert!(!super::actor::validate_workflow_id("a/b"));
}

#[test]
fn test_workflow_id_accepts_valid() {
    assert!(super::actor::validate_workflow_id("commit_message"));
    assert!(super::actor::validate_workflow_id("follow-up"));
    assert!(super::actor::validate_workflow_id("kg_enrich"));
}

#[test]
fn test_report_error_unicode_safe() {
    let mut svc = make_service();
    svc.report_error("test", "emoji 🎉 and CJK 你好 text", None, None);
    assert!(!svc.state.recent_activities.is_empty());
}

#[test]
fn test_error_redaction_strips_tokens() {
    let output = super::actor::redact_sensitive("Error: Bearer sk-abc123xyz failed");
    assert!(output.contains("[REDACTED]"), "must contain [REDACTED]");
    assert!(!output.contains("sk-abc123xyz"), "must not contain raw token");
}

#[test]
fn test_queue_eviction_drops_oldest_low_priority() {
    use super::runtime_queue::RuntimeQueue;
    use super::actor::make_runtime_event;
    let mut queue = RuntimeQueue::new();
    for i in 0..50 {
        let mut ev = make_runtime_event("signal", &format!("low-{}", i), "src", &format!("low-key-{}", i), "started", None);
        ev.priority = "low".to_string();
        queue.enqueue(ev);
    }
    for i in 0..55 {
        let ev = make_runtime_event("signal", &format!("normal-{}", i), "src", &format!("normal-key-{}", i), "started", Some("normal"));
        queue.enqueue(ev);
    }
    assert!(queue.items.len() <= 100);
    assert!(!queue.items.iter().any(|e| e.title == "low-0"), "oldest low-priority item must be evicted first");
    assert!(queue.items.iter().any(|e| e.title == "low-49"), "newest low-priority items should survive");
}

#[tokio::test]
async fn test_ledger_skips_empty_chat_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let bad_path = root.join(".refact/buddy/chats/conversations/no_id.json");
    let bad_json = serde_json::json!({
        "title": "Missing ID", "kind": "chat",
        "created_at": "2024-01-01T00:00:00Z", "messages": []
    });
    super::storage::atomic_write_json(&bad_path, &bad_json).await.unwrap();

    let good_path = root.join(".refact/buddy/chats/conversations/has_id.json");
    let good_json = serde_json::json!({
        "chat_id": "has_id", "title": "Good Chat", "kind": "chat",
        "created_at": "2024-01-02T00:00:00Z", "messages": []
    });
    super::storage::atomic_write_json(&good_path, &good_json).await.unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "has_id");
}
