use crate::chat::evidence::{EvidenceDefaults, SpecAuthority, VerificationSpec};
use crate::tasks::assignment::{specs_for_validation, Assignment};
use crate::tasks::types::BoardCard;
use refact_chat_api::DeliverableContract;

fn defaults() -> EvidenceDefaults {
    EvidenceDefaults {
        timeout_secs: 60,
        output_cap_bytes: 4_000,
    }
}

fn card(instructions: &str) -> BoardCard {
    let mut card: BoardCard = serde_json::from_value(serde_json::json!({
        "id": "T-typed",
        "title": "Typed card",
        "column": "planned",
        "created_at": "2026-10-02T00:00:00Z",
        "started_at": null,
        "completed_at": null,
        "assignee": null,
        "agent_chat_id": null
    }))
    .unwrap();
    card.instructions = instructions.to_string();
    card
}

#[test]
fn card_without_typed_fields_is_unchanged() {
    let card = card("Do the work.\n\n## Verification\n- Verify: `cargo test --lib`");

    let assignment = Assignment::from_card(&card);

    assert!(assignment.deliverable.is_none());
    assert!(assignment.verify.is_empty());
    assert!(!assignment.has_typed_verification());
    assert_eq!(assignment.task, card.instructions);
}

#[test]
fn card_with_typed_fields_produces_typed_assignment() {
    let mut card = card("Do the work.");
    let deliverable = DeliverableContract {
        required_sections: vec!["Findings".to_string()],
        output_schema: None,
        evidence_required: true,
    };
    let verification = VerificationSpec::propose(
        "T-typed:declared",
        "cargo test --lib -- typed",
        SpecAuthority::CardInstructions,
        &defaults(),
    );
    card.deliverable = Some(serde_json::to_value(&deliverable).unwrap());
    card.verification = vec![serde_json::to_value(&verification).unwrap()];

    let assignment = Assignment::from_card(&card);

    assert_eq!(assignment.deliverable, Some(deliverable.clone()));
    assert_eq!(assignment.verify, vec![verification]);
    assert!(assignment.has_typed_verification());
    assert!(assignment
        .render_prompt()
        .contains(&deliverable.render_instructions()));
}

#[test]
fn typed_specs_take_precedence_in_validation() {
    let mut card = card("## Verification\n- Verify: `cargo test --lib -- from-prose`");
    let typed = VerificationSpec::propose(
        "T-typed:declared",
        "cargo test --lib -- typed-contract",
        SpecAuthority::CardInstructions,
        &defaults(),
    );
    card.verification = vec![serde_json::to_value(&typed).unwrap()];
    let assignment = Assignment::from_card(&card);

    let specs = specs_for_validation(Some(&assignment), &card, &defaults());

    assert_eq!(specs, vec![typed]);
    assert_eq!(specs[0].command, "cargo test --lib -- typed-contract");
}

#[test]
fn malformed_stored_spec_falls_back_without_panic() {
    let mut card = card("## Verification\n- Verify: `cargo test --lib -- from-prose`");
    card.verification = vec![serde_json::json!({"garbage": true})];

    let assignment = Assignment::from_card(&card);

    assert!(assignment.deliverable.is_none());
    assert!(assignment.verify.is_empty());
    assert!(!assignment.has_typed_verification());
    assert!(crate::tasks::assignment::typed_parts(&card)
        .unwrap_err()
        .contains("card T-typed verification[0]"));
}

#[test]
fn old_board_yaml_round_trips() {
    let yaml = r#"
cards:
  - id: T-old
    title: Legacy card
    column: planned
    priority: P1
    depends_on: []
    instructions: Legacy instructions
    assignee: null
    agent_chat_id: null
    status_updates: []
    created_at: '2026-10-02T00:00:00Z'
    started_at: null
    completed_at: null
"#;
    let board: crate::tasks::types::TaskBoard = serde_yaml::from_str(yaml).unwrap();
    let serialized = serde_yaml::to_string(&board).unwrap();
    let reparsed: crate::tasks::types::TaskBoard = serde_yaml::from_str(&serialized).unwrap();

    assert!(reparsed.cards[0].deliverable.is_none());
    assert!(reparsed.cards[0].verification.is_empty());
    assert!(!serialized.contains("deliverable"));
    assert!(!serialized.contains("verification"));
}
