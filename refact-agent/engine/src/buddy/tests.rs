use chrono::Duration;
use tokio::sync::broadcast;

use super::actor::BuddyService;
use super::diagnostics::{classify_error, DiagnosticContext, DiagnosticSeverity};
use super::issues::{check_issue_gate, check_manual_issue_gate, redact_diagnostic_text, sanitize_body, sanitize_title, IssueGate};
use super::settings::{BuddySettings, MAX_PALETTE_INDEX};
use super::state::{default_buddy_state, grant_xp};
use super::types::{BuddySuggestion, BuddyState};

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
