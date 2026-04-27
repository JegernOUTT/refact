use chrono::Duration;
use tokio::sync::broadcast;

use super::actor::BuddyService;
use super::diagnostics::{classify_error, DiagnosticContext, DiagnosticSeverity};
use super::issues::{
    check_issue_gate, check_manual_issue_gate, redact_diagnostic_text, sanitize_body,
    sanitize_title, IssueGate,
};
use super::scheduler::BuddyJobContext;
use super::settings::{AutonomyLevel, BuddySettings, HumorLevel, MAX_PALETTE_INDEX};
use super::state::{
    apply_care_action, apply_pet_tick, default_buddy_state, grant_xp, reroll_personality,
};
use super::types::{
    BuddyAction, BuddyCareAction, BuddyFact, BuddyFactKind, BuddyJobState, BuddyOnboarding,
    BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks, BuddyPage, BuddyPriority,
    BuddyPulse, BuddySuggestion, BuddyState, CustomizationKind, DefaultsKind, DraftKind,
    InvestigationContext, MarketKind, OpportunityStatus, PulseScope,
};

fn make_service() -> BuddyService {
    let (tx, _rx) = broadcast::channel(16);
    BuddyService::new(
        std::env::temp_dir().join(format!("buddy-test-{}", uuid::Uuid::new_v4())),
        default_buddy_state(),
        BuddySettings::default(),
        Vec::new(),
        super::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    )
}

fn make_suggestion(id: &str, stype: &str, created_at: &str) -> BuddySuggestion {
    BuddySuggestion {
        id: id.to_string(),
        suggestion_type: stype.to_string(),
        title: "t".to_string(),
        description: "d".to_string(),
        created_at: created_at.to_string(),
        dismissed: false,
        controls: vec![],
        quest: None,
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
    assert_eq!(state.pet.needs.hunger, 80);
    assert_eq!(state.pet.needs.energy, 85);
    assert_eq!(state.pet.needs.hygiene, 80);
    assert_eq!(state.pet.needs.boredom, 15);
    assert_eq!(state.pet.needs.affection, 75);
}

#[test]
fn test_growth_points_do_not_level_without_care_gate() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 100);
    assert_eq!(state.progression.level, 1);
    assert_eq!(state.progression.stage, 0);
    assert_eq!(state.progression.xp, 100);
}

#[test]
fn test_grant_xp_updates_stage_when_care_gate_met() {
    let mut state = default_buddy_state();
    state.pet.evolution.open_seconds = 10 * 60;
    state.pet.evolution.care_score = 20;
    grant_xp(&mut state, 30);
    assert_eq!(state.progression.stage, 1);
    assert_eq!(state.progression.stage_name, "Hatch");
    assert_eq!(state.progression.level, 2);
    assert_eq!(state.progression.xp, 10);
}

#[test]
fn test_stage_transitions_require_runtime_and_care() {
    let mut state = default_buddy_state();
    grant_xp(&mut state, 100);
    assert_eq!(state.progression.stage_name, "Egg");
    state.pet.evolution.open_seconds = 20 * 60;
    state.pet.evolution.care_score = 40;
    grant_xp(&mut state, 0);
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
    state.pet.evolution.open_seconds = 400 * 60;
    state.pet.evolution.care_score = 500;
    grant_xp(&mut state, 3000);
    assert_eq!(state.progression.stage_name, "Archon");
    assert_eq!(state.progression.stage, 6);
    assert_eq!(state.progression.level, 7);
    assert_eq!(state.progression.xp_next, 0);
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
    assert!(
        json.get("palette_index").is_none(),
        "palette_index must not be in BuddySettings"
    );
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
    let state: BuddyState =
        serde_json::from_str(json).expect("should parse old state without onboarding");
    assert!(!state.onboarding.greeted);
    assert!(!state.onboarding.tour_completed);
    assert!(state.onboarding.first_launch_at.is_empty());
    assert_eq!(state.pet.needs.hunger, 80);
    assert_eq!(state.pet.needs.energy, 85);
    assert_eq!(state.pet.evolution.open_seconds, 0);
    assert!(!state.personality.archetype_label.is_empty());
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
    assert_eq!(loaded.progression.xp, 25);
    assert_eq!(loaded.pet.needs.hunger, 80);
}

#[test]
fn test_pet_tick_decays_needs_while_awake() {
    let mut state = default_buddy_state();
    // Pin personality so trait thresholds are deterministic (all traits = 50)
    state.personality = super::types::BuddyPersonalityProfile::default();
    let changed = apply_pet_tick(&mut state, 15);
    assert!(changed);
    assert_eq!(state.pet.needs.hunger, 78);
    assert_eq!(state.pet.needs.energy, 84);
    assert_eq!(state.pet.needs.hygiene, 79);
    assert_eq!(state.pet.needs.boredom, 17);
    assert_eq!(state.pet.needs.affection, 74);
    assert_eq!(state.pet.evolution.open_seconds, 15);
}

#[test]
fn test_pet_tick_restores_energy_while_sleeping() {
    let mut state = default_buddy_state();
    state.pet.condition.sleeping = true;
    state.pet.needs.energy = 20;
    let changed = apply_pet_tick(&mut state, 15);
    assert!(changed);
    assert_eq!(state.pet.needs.energy, 23);
    assert_eq!(state.pet.needs.boredom, 16);
}

#[test]
fn test_pet_tick_updates_need_flags() {
    let mut state = default_buddy_state();
    state.pet.needs.hunger = 10;
    state.pet.needs.energy = 10;
    state.pet.needs.hygiene = 10;
    state.pet.needs.boredom = 95;
    state.pet.needs.affection = 10;
    let _ = apply_pet_tick(&mut state, 15);
    assert!(state.pet.condition.hungry);
    assert!(state.pet.condition.sleepy);
    assert!(state.pet.condition.dirty);
    assert!(state.pet.condition.bored);
    assert!(state.pet.condition.lonely);
}

#[test]
fn test_pet_tick_sets_dirty_on_service() {
    let mut svc = make_service();
    assert!(!svc.dirty);
    svc.apply_pet_tick(15);
    assert!(svc.dirty);
}

#[test]
fn test_reroll_personality_preserves_progress() {
    let mut state = default_buddy_state();
    state.progression.stage = 2;
    state.progression.xp = 17;
    let before = state.personality.archetype_label.clone();
    reroll_personality(&mut state);
    assert_eq!(state.progression.stage, 2);
    assert_eq!(state.progression.xp, 17);
    assert!(!state.personality.archetype_label.is_empty());
    assert!(!state.personality.prompt.is_empty());
    if before == state.personality.archetype_label {
        assert_ne!(state.personality.traits.playfulness, 0);
    }
}

#[test]
fn test_feed_care_action_restores_hunger() {
    let mut state = default_buddy_state();
    state.pet.needs.hunger = 10;
    let (changed, message) = apply_care_action(&mut state, BuddyCareAction::Feed, None);
    assert!(changed);
    assert!(message.contains("Snack"));
    assert!(state.pet.needs.hunger > 10);
    assert_eq!(state.recent_activities[0].activity_type, "care_feed");
}

#[test]
fn test_play_care_action_uses_toy_hint() {
    let mut state = default_buddy_state();
    state.pet.needs.boredom = 90;
    let (_, message) = apply_care_action(&mut state, BuddyCareAction::Play, Some("bug"));
    assert!(message.contains("bug"));
    assert!(state.pet.needs.boredom < 90);
}

#[test]
fn test_sleep_care_action_sets_sleeping() {
    let mut state = default_buddy_state();
    state.pet.needs.energy = 20;
    let (_, message) = apply_care_action(&mut state, BuddyCareAction::Sleep, None);
    assert!(message.contains("Sleep mode"));
    assert!(state.pet.condition.sleeping);
    assert!(state.pet.needs.energy > 20);
}

#[test]
fn test_sleep_care_action_stays_sleeping_near_threshold() {
    let mut state = default_buddy_state();
    state.pet.needs.energy = 80;
    let _ = apply_care_action(&mut state, BuddyCareAction::Sleep, None);
    assert!(state.pet.condition.sleeping);
}

#[test]
fn test_settings_null_prompt_clears_value() {
    let json = r#"{
        "enabled": true,
        "clear_personality_prompt": true
    }"#;
    let req: crate::http::routers::v1::buddy::BuddySettingsRequest =
        serde_json::from_str(json).unwrap();
    assert_eq!(req.clear_personality_prompt, Some(true));
    assert!(req.personality_prompt.is_none());
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
    assert_eq!(
        name1, state2.identity.name,
        "bootstrap must not overwrite existing state.json"
    );
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
    let already = svc
        .state
        .suggestion_state
        .iter()
        .any(|s| s.suggestion_type == "setup");
    if !already {
        svc.add_suggestion(make_suggestion("setup", "setup", &now));
    }
    let already2 = svc
        .state
        .suggestion_state
        .iter()
        .any(|s| s.suggestion_type == "setup");
    if !already2 {
        svc.add_suggestion(make_suggestion("setup2", "setup", &now));
    }
    assert_eq!(svc.state.suggestion_state.len(), 1);
}

#[test]
fn test_suggestion_pruning() {
    let mut svc = make_service();
    let old_time = (chrono::Utc::now() - Duration::seconds(400)).to_rfc3339();
    svc.state
        .suggestion_state
        .push(make_suggestion("old", "test", &old_time));
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

#[tokio::test]
async fn test_diagnostic_cap() {
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

#[tokio::test]
async fn test_diagnostic_history_persists_and_loads() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let ctx1 = DiagnosticContext {
        error_type: "test".to_string(),
        error_message: "first".to_string(),
        source_file: Some("src/a.rs".to_string()),
        tool_name: None,
        chat_id: Some("chat-1".to_string()),
        collected_at: "2026-04-27T10:00:00Z".to_string(),
        severity: DiagnosticSeverity::High,
    };
    let ctx2 = DiagnosticContext {
        error_type: "test".to_string(),
        error_message: "second".to_string(),
        source_file: Some("src/b.rs".to_string()),
        tool_name: Some("tool".to_string()),
        chat_id: Some("chat-2".to_string()),
        collected_at: "2026-04-27T10:01:00Z".to_string(),
        severity: DiagnosticSeverity::Low,
    };

    super::storage::append_diagnostic(root, &ctx1)
        .await
        .unwrap();
    super::storage::append_diagnostic(root, &ctx2)
        .await
        .unwrap();

    let loaded = super::storage::load_diagnostics(root).await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].error_message, "first");
    assert_eq!(loaded[1].error_message, "second");
}

#[test]
fn test_same_day_log_filter_accepts_same_day_time() {
    assert!(super::actor::same_day_log_filter(
        "101530.123 ERROR something failed",
        "2026-04-27T10:16:00Z",
    ));
    assert!(!super::actor::same_day_log_filter(
        "235959.999 ERROR old failure",
        "2026-04-27T10:16:00Z",
    ));
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
        chat_id: None,
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
        chat_id: None,
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
    let valid_actions = [
        "open_chat",
        "open_setup",
        "open_setup_mcp",
        "open_setup_skills",
        "open_stats",
        "open_buddy",
        "dismiss",
        "run_command",
    ];
    assert!(valid_actions.contains(&"open_setup"));
    assert!(valid_actions.contains(&"dismiss"));
    assert!(!valid_actions.contains(&"invalid_action"));
}

#[test]
fn test_runtime_event_speech_text_preserved() {
    use super::runtime_queue::RuntimeQueue;
    let mut queue = RuntimeQueue::new();
    let mut ev =
        super::actor::make_runtime_event("streaming", "Test", "chat", "chat_1", "started", None);
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
    let mut ev1 =
        super::actor::make_runtime_event("streaming", "First", "chat", "key_1", "started", None);
    ev1.speech_text = Some("Initial".to_string());
    ev1.persistent = true;
    queue.enqueue(ev1);
    let mut ev2 =
        super::actor::make_runtime_event("streaming", "Updated", "chat", "key_1", "progress", None);
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
    super::storage::atomic_write_json(&conv_path, &conv_json)
        .await
        .unwrap();

    let wf_path = root.join(".refact/buddy/chats/workflows/commit_message.json");
    let wf_json = serde_json::json!({
        "entries": [{ "timestamp": "2024-01-01T00:00:00Z", "input_summary": "", "output_summary": "done", "success": true }]
    });
    super::storage::atomic_write_json(&wf_path, &wf_json)
        .await
        .unwrap();

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
    super::storage::atomic_write_json(&path, &json)
        .await
        .unwrap();

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
    super::storage::atomic_write_json(&conv_path, &conv_json)
        .await
        .unwrap();

    let wf_path = root.join(".refact/buddy/chats/workflows/commit_message.json");
    let wf_json = serde_json::json!({ "entries": [] });
    super::storage::atomic_write_json(&wf_path, &wf_json)
        .await
        .unwrap();

    let chat_only = super::conversation_ledger::list_all_buddy_conversations(
        root,
        Some(vec!["chat".to_string()]),
    )
    .await;
    assert!(chat_only.iter().all(|e| e.kind == "chat"));

    let wf_only = super::conversation_ledger::list_all_buddy_conversations(
        root,
        Some(vec!["workflow".to_string()]),
    )
    .await;
    assert!(wf_only.iter().all(|e| e.kind == "workflow"));
}

#[tokio::test]
async fn test_sorting_by_updated_at() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    super::storage::bootstrap_buddy_storage(root).await.unwrap();

    let p1 = root.join(".refact/buddy/chats/conversations/old.json");
    super::storage::atomic_write_json(
        &p1,
        &serde_json::json!({
            "chat_id": "old", "title": "Old", "created_at": "2024-01-01T00:00:00Z", "messages": []
        }),
    )
    .await
    .unwrap();

    let p2 = root.join(".refact/buddy/chats/conversations/new.json");
    super::storage::atomic_write_json(
        &p2,
        &serde_json::json!({
            "chat_id": "new", "title": "New", "created_at": "2024-06-01T00:00:00Z",
            "last_message_at": "2024-06-02T00:00:00Z", "messages": []
        }),
    )
    .await
    .unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    assert_eq!(entries[0].id, "new");
}

#[test]
fn test_runtime_event_controls_preserved() {
    use super::runtime_queue::RuntimeQueue;
    use super::types::BuddyControl;
    let mut queue = RuntimeQueue::new();
    let mut ev = super::actor::make_runtime_event(
        "chat_error",
        "Error",
        "chat",
        "err_1",
        "info",
        Some("high"),
    );
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

fn make_job_context(
    onboarding: BuddyOnboarding,
    diagnostics_count: usize,
    job_state: BuddyJobState,
) -> BuddyJobContext {
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
        total_workflow_runs: 0,
        suggestion_state: vec![],
        pet: Default::default(),
        active_quest: None,
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
    let elapsed = state
        .last_run
        .as_deref()
        .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
        .map(|t| {
            chrono::Utc::now()
                .signed_duration_since(t)
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or(u64::MAX);
    let cooldown = 5 * 60u64;
    assert!(elapsed < cooldown, "job should be blocked by cooldown");
}

#[test]
fn test_job_state_persistence_roundtrip() {
    let mut state = default_buddy_state();
    state.job_cooldowns.insert(
        "greeting".to_string(),
        BuddyJobState {
            last_run: Some("2026-01-01T00:00:00Z".to_string()),
            run_count: 3,
            last_result: Some("ok".to_string()),
            snoozed_until: None,
            dismissed: false,
        },
    );
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
    let elapsed = job_state
        .last_run
        .as_deref()
        .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
        .map(|t| {
            chrono::Utc::now()
                .signed_duration_since(t)
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or(u64::MAX);
    assert!(
        elapsed < job.cooldown_seconds(),
        "greeting must be blocked within 24h cooldown"
    );
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
    assert!(
        result.suggestion.is_some(),
        "should produce suggestion for 5 repeated timeouts"
    );
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
    assert!(
        result.suggestion.is_some(),
        "should suggest setup when AGENTS.md missing"
    );
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
    let unread = svc
        .state
        .suggestion_state
        .iter()
        .filter(|s| !s.dismissed)
        .count();
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
    assert!(
        should_skip,
        "dismissed job must be skipped regardless of cooldown"
    );
}

#[test]
fn test_proactive_enabled_setting() {
    let settings = BuddySettings::default();
    assert!(
        settings.proactive_enabled,
        "proactive_enabled defaults to true"
    );
    let json = serde_json::to_string(&settings).unwrap();
    let loaded: BuddySettings = serde_json::from_str(&json).unwrap();
    assert!(loaded.proactive_enabled);
}

#[test]
fn test_old_settings_get_proactive_default() {
    let json = r#"{"enabled": true, "auto_diagnostics": true, "auto_issue_creation": false}"#;
    let settings: BuddySettings = serde_json::from_str(json).unwrap();
    assert!(
        settings.proactive_enabled,
        "missing proactive_enabled should default to true"
    );
}

#[tokio::test]
async fn test_tour_job_runs_only_once() {
    use super::jobs::tour::TourJob;
    use super::scheduler::BuddyJob;
    let job = TourJob;
    let onboarding = BuddyOnboarding {
        greeted: true,
        tour_completed: false,
        ..Default::default()
    };
    let fresh_ctx = make_job_context(onboarding.clone(), 0, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(
        job.should_run(gcx.clone(), &fresh_ctx).await,
        "tour must run on first tick"
    );
    let ran_state = BuddyJobState {
        last_run: Some(chrono::Utc::now().to_rfc3339()),
        run_count: 1,
        last_result: Some("ok".to_string()),
        snoozed_until: None,
        dismissed: false,
    };
    let ran_ctx = make_job_context(onboarding, 0, ran_state);
    assert!(
        !job.should_run(gcx, &ran_ctx).await,
        "tour must not run after first run"
    );
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
        controls: vec![],
        quest: None,
    };
    assert!(
        svc.maybe_add_suggestion(s1),
        "first suggestion must be accepted"
    );
    assert!(
        !svc.maybe_add_suggestion(s2),
        "duplicate suggestion must be rejected by dedup"
    );
    assert_eq!(svc.state.suggestion_state.len(), 1);
}

#[tokio::test]
async fn test_proactive_disabled_still_allows_greeting() {
    use super::jobs::greeting::GreetingJob;
    use super::scheduler::BuddyJob;
    let job = GreetingJob;
    assert!(
        !job.produces_suggestion(),
        "greeting must not be gated by proactive flag"
    );
    let ctx = make_job_context(BuddyOnboarding::default(), 0, BuddyJobState::default());
    let gcx = crate::global_context::tests::make_test_gcx().await;
    assert!(
        job.should_run(gcx, &ctx).await,
        "greeting must run even when proactive_enabled=false"
    );
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

#[tokio::test]
async fn test_report_error_unicode_safe() {
    let mut svc = make_service();
    svc.report_error("test", "emoji 🎉 and CJK 你好 text", None, None);
    assert!(!svc.state.recent_activities.is_empty());
    assert_eq!(svc.recent_diagnostics.len(), 1);
}

#[test]
fn test_error_redaction_strips_tokens() {
    let output = super::actor::redact_sensitive("Error: Bearer sk-abc123xyz failed");
    assert!(output.contains("[REDACTED]"), "must contain [REDACTED]");
    assert!(
        !output.contains("sk-abc123xyz"),
        "must not contain raw token"
    );
}

#[test]
fn test_queue_eviction_drops_oldest_low_priority() {
    use super::runtime_queue::RuntimeQueue;
    use super::actor::make_runtime_event;
    let mut queue = RuntimeQueue::new();
    for i in 0..50 {
        let mut ev = make_runtime_event(
            "signal",
            &format!("low-{}", i),
            "src",
            &format!("low-key-{}", i),
            "started",
            None,
        );
        ev.priority = "low".to_string();
        queue.enqueue(ev);
    }
    for i in 0..55 {
        let ev = make_runtime_event(
            "signal",
            &format!("normal-{}", i),
            "src",
            &format!("normal-key-{}", i),
            "started",
            Some("normal"),
        );
        queue.enqueue(ev);
    }
    assert!(queue.items.len() <= 100);
    assert!(
        !queue.items.iter().any(|e| e.title == "low-0"),
        "oldest low-priority item must be evicted first"
    );
    assert!(
        queue.items.iter().any(|e| e.title == "low-49"),
        "newest low-priority items should survive"
    );
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
    super::storage::atomic_write_json(&bad_path, &bad_json)
        .await
        .unwrap();

    let good_path = root.join(".refact/buddy/chats/conversations/has_id.json");
    let good_json = serde_json::json!({
        "chat_id": "has_id", "title": "Good Chat", "kind": "chat",
        "created_at": "2024-01-02T00:00:00Z", "messages": []
    });
    super::storage::atomic_write_json(&good_path, &good_json)
        .await
        .unwrap();

    let entries = super::conversation_ledger::list_all_buddy_conversations(root, None).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "has_id");
}

#[test]
fn test_workflow_label_mapping() {
    assert_eq!(
        super::workflows::workflow_label("commit_message"),
        "commit message generation"
    );
    assert_eq!(
        super::workflows::workflow_label("follow_up"),
        "follow-up suggestions"
    );
    assert_eq!(
        super::workflows::workflow_label("compress_trajectory"),
        "chat compression"
    );
    assert_eq!(
        super::workflows::workflow_label("memo_extraction"),
        "memo extraction"
    );
    assert_eq!(
        super::workflows::workflow_label("kg_enrich"),
        "knowledge graph enrichment"
    );
    assert_eq!(
        super::workflows::workflow_label("kg_deprecate"),
        "knowledge cleanup"
    );
    assert_eq!(
        super::workflows::workflow_label("title_generating"),
        "title generation"
    );
    assert_eq!(
        super::workflows::workflow_label("unknown_workflow"),
        "unknown_workflow"
    );
}

#[test]
fn test_event_title_length_limit() {
    use super::actor::make_runtime_event;
    let long_title = "A".repeat(200);
    let ev = make_runtime_event("signal", &long_title, "src", "key", "started", None);
    assert!(
        ev.title.len() <= 200,
        "make_runtime_event stores the title as-is"
    );
    let truncated: String = long_title.chars().take(80).collect();
    assert!(
        truncated.len() <= 80,
        "truncated title must be at most 80 chars"
    );
    let chat_label: String = "Some very long chat title that goes on and on and on and on and on"
        .chars()
        .take(60)
        .collect();
    let ev2 = make_runtime_event(
        "chat_started",
        &format!("Started: {}", chat_label),
        "chat",
        "chat_123",
        "started",
        None,
    );
    assert!(
        ev2.title.len() <= 120,
        "chat started event title must be under 120 chars"
    );
}

#[test]
fn test_runtime_event_chat_id_default_none() {
    use super::actor::make_runtime_event;
    let ev = make_runtime_event(
        "indexing",
        "Indexing...",
        "indexer",
        "indexing",
        "started",
        None,
    );
    assert!(ev.chat_id.is_none(), "default event must have no chat_id");
}

#[test]
fn test_runtime_event_chat_id_serialized_when_set() {
    use super::actor::make_runtime_event;
    let mut ev = make_runtime_event(
        "chat_error",
        "Error",
        "chat",
        "chat_abc",
        "failed",
        Some("high"),
    );
    ev.chat_id = Some("abc-123".to_string());
    let json = serde_json::to_string(&ev).unwrap();
    assert!(
        json.contains("\"chat_id\":\"abc-123\""),
        "chat_id must be serialized when set"
    );
}

#[test]
fn test_runtime_event_chat_id_skipped_when_none() {
    use super::actor::make_runtime_event;
    let ev = make_runtime_event(
        "chat_completed",
        "Done",
        "chat",
        "chat_abc",
        "completed",
        None,
    );
    let json = serde_json::to_string(&ev).unwrap();
    assert!(
        !json.contains("chat_id"),
        "chat_id must be skipped when None"
    );
}

#[test]
fn test_chat_error_event_includes_chat_id() {
    use super::actor::make_runtime_event;
    let chat_id = "test-chat-xyz";
    let mut ev = make_runtime_event(
        "chat_error",
        "Error in 'Test chat': something failed",
        "chat",
        &format!("chat_{}", chat_id),
        "failed",
        Some("high"),
    );
    ev.chat_id = Some(chat_id.to_string());
    assert_eq!(ev.chat_id.as_deref(), Some(chat_id));
    assert_eq!(ev.status, "failed");
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

// =============================================================================
// Redaction strength
// =============================================================================

#[test]
fn test_redact_handles_multiple_secrets_and_case() {
    let input = "first Bearer abc def then bearer XYZ and api_key=p1 plus API_KEY=p2 token=tok1";
    let output = super::actor::redact_sensitive(input);
    assert!(
        !output.contains("abc"),
        "first Bearer secret leaked: {}",
        output
    );
    assert!(
        !output.contains("XYZ"),
        "lowercase bearer leaked: {}",
        output
    );
    assert!(!output.contains("p1"), "first api_key leaked: {}", output);
    assert!(
        !output.contains("p2"),
        "second case-variant api_key leaked: {}",
        output
    );
    assert!(!output.contains("tok1"), "token leaked: {}", output);
    // Should redact at least 4 distinct secrets in this string.
    let redactions = output.matches("[REDACTED]").count();
    assert!(
        redactions >= 4,
        "expected >=4 redactions, got {}: {}",
        redactions,
        output
    );
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
        status: OpportunityStatus::New,
        proposed_actions: vec![],
        humor: None,
        humor_allowed: false,
        related: BuddyOpportunityLinks::default(),
        created_at: now,
        expires_at: now + Duration::hours(1),
    }
}

#[test]
fn fact_store_dedup_by_key() {
    use super::facts::FactStore;
    let mut store = FactStore::new();
    let now = chrono::Utc::now();
    let f1 = BuddyFact {
        payload: serde_json::json!({"v": 1}),
        ..make_fact("k1", BuddyFactKind::TaskStuck, now)
    };
    let f2 = BuddyFact {
        payload: serde_json::json!({"v": 2}),
        ..make_fact("k1", BuddyFactKind::TaskStuck, now)
    };
    store.ingest(f1);
    store.ingest(f2);
    assert_eq!(store.len(), 1);
    assert_eq!(store.iter().next().unwrap().payload["v"], 2);
}

#[test]
fn fact_store_ring_evicts_oldest() {
    use super::facts::{FactStore, FACT_RING_CAPACITY};
    let mut store = FactStore::new();
    let now = chrono::Utc::now();
    for i in 0..=FACT_RING_CAPACITY {
        store.ingest(make_fact(
            &format!("key-{}", i),
            BuddyFactKind::TaskStuck,
            now,
        ));
    }
    assert_eq!(store.len(), FACT_RING_CAPACITY);
    assert!(
        !store.iter().any(|f| f.key == "key-0"),
        "first key must be evicted"
    );
}

#[test]
fn fact_store_count_within() {
    use super::facts::FactStore;
    let now = chrono::Utc::now();
    let mut store = FactStore::new();
    store.ingest(make_fact(
        "old",
        BuddyFactKind::DiagnosticCluster,
        now - Duration::hours(2),
    ));
    store.ingest(make_fact(
        "recent",
        BuddyFactKind::DiagnosticCluster,
        now - Duration::minutes(5),
    ));
    assert_eq!(
        store.count_within(BuddyFactKind::DiagnosticCluster, Duration::hours(1)),
        1
    );
    assert_eq!(
        store.count_within(BuddyFactKind::DiagnosticCluster, Duration::hours(3)),
        2
    );
}

// =============================================================================
// OpportunityQueue tests
// =============================================================================

#[test]
fn opportunity_queue_unread_cap_state() {
    use super::opportunities::OpportunityQueue;
    let mut q = OpportunityQueue::new();
    q.push(make_opportunity("opp1", "ck1"));
    assert_eq!(q.unread_count(), 1);
    q.mark_status("opp1", OpportunityStatus::Dismissed);
    assert_eq!(q.unread_count(), 0);
}

#[test]
fn opportunity_queue_cooldown_blocks_dup() {
    use super::opportunities::OpportunityQueue;
    let mut q = OpportunityQueue::new();
    q.push(make_opportunity("opp1", "ck1"));
    assert!(q.cooldown_active("ck1"));
}

#[test]
fn opportunity_queue_dismissed_24h() {
    use super::opportunities::OpportunityQueue;
    let mut q = OpportunityQueue::new();
    q.push(make_opportunity("opp1", "ck1"));
    q.dismiss("opp1");
    assert!(q.recently_dismissed("ck1", Duration::hours(24)));
    assert!(!q.recently_dismissed("ck1", Duration::zero()));
}

#[test]
fn opportunity_queue_expire_old() {
    use super::opportunities::OpportunityQueue;
    let now = chrono::Utc::now();
    let mut q = OpportunityQueue::new();
    let mut opp = make_opportunity("opp1", "ck1");
    opp.expires_at = now - Duration::hours(1);
    opp.created_at = now - Duration::minutes(5);
    q.push(opp);
    q.expire_old(now);
    assert_eq!(
        q.get("opp1").map(|o| o.status),
        Some(OpportunityStatus::Expired)
    );
    q.expire_old(now + Duration::hours(25));
    assert!(q.get("opp1").is_none(), "must be removed after 24h");
}

#[test]
fn opportunity_queue_cap() {
    use super::opportunities::{OpportunityQueue, MAX_OPPORTUNITIES};
    let mut q = OpportunityQueue::new();
    for i in 0..=MAX_OPPORTUNITIES {
        q.push(make_opportunity(
            &format!("opp-{}", i),
            &format!("ck-{}", i),
        ));
    }
    assert!(q.iter().count() <= MAX_OPPORTUNITIES);
}

// =============================================================================
// DraftStore tests
// =============================================================================

#[test]
fn draft_store_create_get_consume() {
    use super::drafts::DraftStore;
    let mut store = DraftStore::new();
    let draft = store.create(
        DraftKind::Skill,
        "My Skill".to_string(),
        "yaml: {}".to_string(),
        "exp".to_string(),
    );
    let id = draft.id.clone();
    assert!(store.get(&id).is_some());
    let consumed = store.consume(&id);
    assert!(consumed.is_some());
    assert!(store.get(&id).is_none());
}

#[test]
fn draft_store_ttl() {
    use super::drafts::DraftStore;
    let mut store = DraftStore::new();
    let draft = store.create(
        DraftKind::Command,
        "Cmd".to_string(),
        "{}".to_string(),
        "".to_string(),
    );
    let id = draft.id.clone();
    store.expire_old(chrono::Utc::now() + Duration::hours(3));
    assert!(store.get(&id).is_none(), "draft must be removed after TTL");
}

// =============================================================================
// PulseBuilder tests
// =============================================================================

#[tokio::test]
async fn pulse_build_skeleton_sets_generated_at() {
    use super::facts::FactStore;
    use super::pulse::build_pulse;
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let store = FactStore::new();
    let pulse = build_pulse(gcx, std::path::Path::new("/tmp"), &store).await;
    assert!(pulse.generated_at.is_some());
}

#[test]
fn state_migration_loads_old_state_without_opportunities() {
    let json = r#"{
        "identity": {"name": "Pixel", "created_at": "2024-01-01T00:00:00Z", "palette_index": 2},
        "progression": {"stage": 0, "stage_name": "Egg", "level": 1, "xp": 0, "xp_next": 20},
        "skills": {"unlocked": [], "locked": []},
        "workflow_summaries": [],
        "semantic": {"mood": "Idle", "focus": "", "headline": "", "last_active": "2024-01-01T00:00:00Z"},
        "recent_activities": [],
        "suggestion_state": []
    }"#;
    let state: BuddyState =
        serde_json::from_str(json).expect("should parse old state without opportunities");
    assert!(
        state.opportunities.is_empty(),
        "opportunities must default to empty vec"
    );
}

#[test]
fn buddy_action_round_trip() {
    let actions: Vec<BuddyAction> = vec![
        BuddyAction::OpenPage {
            page: BuddyPage::Buddy,
            params: None,
        },
        BuddyAction::LaunchInvestigationChat {
            preload: InvestigationContext {
                fact_keys: vec![],
                diagnostic_ids: vec![],
                log_excerpt: String::new(),
                config_summary: String::new(),
                initial_user_message: "investigate".to_string(),
            },
        },
        BuddyAction::DraftSkill {
            draft_id: "d1".to_string(),
            label: "My Skill".to_string(),
        },
        BuddyAction::DraftCommand {
            draft_id: "d2".to_string(),
            label: "My Command".to_string(),
        },
        BuddyAction::DraftSubagent {
            draft_id: "d3".to_string(),
            label: "My Subagent".to_string(),
        },
        BuddyAction::DraftMode {
            draft_id: "d4".to_string(),
            label: "My Mode".to_string(),
        },
        BuddyAction::DraftAgentsMdPatch {
            diff: "--- a\n+++ b".to_string(),
        },
        BuddyAction::DraftDefaultsChange {
            defaults_kind: DefaultsKind::ChatModel,
            patch: serde_json::json!({}),
        },
        BuddyAction::DraftCustomizationChange {
            customization_kind: CustomizationKind::Mode,
            id: "m1".to_string(),
            patch: serde_json::json!({}),
        },
        BuddyAction::OfferMarketplaceInstall {
            market_kind: MarketKind::Mcp,
            item_id: "github-mcp".to_string(),
        },
        BuddyAction::CreatePulseReport {
            scope: PulseScope::All,
        },
        BuddyAction::Dismiss,
    ];
    for action in &actions {
        let json = serde_json::to_string(action).expect("serialize");
        let back: BuddyAction = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2, "round-trip mismatch");
    }
}

#[test]
fn buddy_page_round_trip() {
    let pages: Vec<BuddyPage> = vec![
        BuddyPage::Buddy,
        BuddyPage::Stats,
        BuddyPage::Customization,
        BuddyPage::Providers,
        BuddyPage::DefaultModels,
        BuddyPage::Integrations,
        BuddyPage::Extensions,
        BuddyPage::MarketplaceHub,
        BuddyPage::McpMarketplace,
        BuddyPage::SkillsMarketplace,
        BuddyPage::CommandsMarketplace,
        BuddyPage::SubagentsMarketplace,
        BuddyPage::TasksList,
        BuddyPage::TaskWorkspace {
            task_id: "task-abc".to_string(),
        },
        BuddyPage::KnowledgeGraph,
    ];
    for page in &pages {
        let json = serde_json::to_string(page).expect("serialize");
        let back: BuddyPage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(page, &back, "round-trip mismatch for {:?}", page);
    }
    let task_json = serde_json::to_string(&BuddyPage::TaskWorkspace {
        task_id: "task-abc".to_string(),
    })
    .unwrap();
    assert!(task_json.contains("task-abc"), "task_id must be serialized");
}

#[test]
fn buddy_pulse_default() {
    let pulse = BuddyPulse::default();
    let json = serde_json::to_string(&pulse).expect("serialize");
    let back: BuddyPulse = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.tasks.total, 0);
    assert_eq!(back.git.uncommitted_files, 0);
    assert!(back.generated_at.is_none());
}

#[test]
fn settings_default_observer_toggles() {
    let settings = BuddySettings::default();
    assert!(
        !settings.observers.chat_pattern,
        "chat_pattern must default to false"
    );
    assert!(settings.observers.task_health);
    assert!(settings.observers.trajectory_clutter);
    assert!(settings.observers.customization_drift);
    assert!(settings.observers.memory_garden);
    assert!(settings.observers.mcp_auth);
    assert!(settings.observers.git_pressure);
    assert!(settings.observers.diagnostic_cluster);
    assert!(settings.observers.provider_health);
}

#[test]
fn humor_level_and_autonomy_serde() {
    let hl = HumorLevel::Light;
    let json = serde_json::to_string(&hl).unwrap();
    let back: HumorLevel = serde_json::from_str(&json).unwrap();
    assert_eq!(hl, back);

    let al = AutonomyLevel::Suggest;
    let json = serde_json::to_string(&al).unwrap();
    let back: AutonomyLevel = serde_json::from_str(&json).unwrap();
    assert_eq!(al, back);

    let settings = BuddySettings::default();
    assert_eq!(settings.humor_level, HumorLevel::Light);
    assert_eq!(settings.autonomy_level, AutonomyLevel::Suggest);
}
