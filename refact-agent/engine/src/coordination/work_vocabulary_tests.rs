//! One vocabulary across the tool, HTTP and GUI surfaces (M10).
//!
//! The work projection names its states, freshness and disclosure once, in
//! `coordination::types`. Every other surface is a copy, and a copy drifts:
//! the GUI omitted `closed` and rejected the whole page for it, and the
//! `work_overview` schema omitted it and refused to filter on it.
//!
//! These tests pin each copy back to the enum. The tool schema is now
//! generated from `WorkStateTag::ALL`, so it cannot drift; the GUI contract is
//! a separate language and is pinned by reading the file.
//!
//! Surfaces that deliberately carry no work vocabulary, and so are not pinned
//! here: the TUI renders a single chat session and never lists project work,
//! so it has no state/freshness/disclosure of its own to keep in step. It
//! names its own `SessionState`, which the catalog already maps into this
//! vocabulary, so pinning it would assert a synonym rather than forbid one.
//!
//! The six evidence verdicts are pinned here too. They had a GUI surface all
//! along — the goal receipt card — but it re-derived the verdict by matching
//! words in the engine's reason string, so rewording a sentence changed what
//! the user was told happened. The verdict now crosses as a value and is
//! pinned to `EvidenceVerdict::ALL` below.
//!
//! Concepts that reach the tool and HTTP surfaces but have no GUI panel yet —
//! merge state and dirty-baseline refusal, `done` vs `finalized`,
//! recovery/reconciliation actions, handoff-brief signal and disagreement
//! status. They are omitted here on purpose: each belongs to the panel of its
//! own milestone, and inventing a second work-shaped list for them is the
//! parallel dashboard M10 exists to avoid.

use std::path::{Path, PathBuf};

use super::types::{Disclosure, Freshness, WorkStateTag};
use crate::chat::evidence::EvidenceVerdict;

fn gui_contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gui/src/services/refact/workTypes.ts")
}

fn gui_evidence_contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gui/src/services/refact/evidenceTypes.ts")
}

/// Collect the double-quoted literals between `start` and the next `terminator`.
fn literals_between(text: &str, start: &str, terminator: &str) -> Vec<String> {
    let rest = text
        .split_once(start)
        .unwrap_or_else(|| panic!("GUI work contract no longer declares `{start}`"))
        .1;
    let block = rest
        .split_once(terminator)
        .unwrap_or_else(|| panic!("`{start}` is not terminated by `{terminator}`"))
        .0;
    block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|value| value.to_string())
        .collect()
}

fn engine_states() -> Vec<String> {
    WorkStateTag::ALL
        .iter()
        .map(|state| state.as_str().to_string())
        .collect()
}

#[test]
fn work_state_names_round_trip_and_all_is_complete() {
    for state in WorkStateTag::ALL {
        // An exhaustive match: a new variant fails to compile here until it is
        // added to ALL, which is what every other surface is generated from.
        match state {
            WorkStateTag::Queued
            | WorkStateTag::Running
            | WorkStateTag::Parked
            | WorkStateTag::WaitingApproval
            | WorkStateTag::Idle
            | WorkStateTag::Completed
            | WorkStateTag::Failed
            | WorkStateTag::Cancelled
            | WorkStateTag::Interrupted
            | WorkStateTag::Closed
            | WorkStateTag::Unknown => {}
        }
        assert_eq!(WorkStateTag::parse(state.as_str()), Some(state));
    }
    let mut unique = engine_states();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        WorkStateTag::ALL.len(),
        "duplicate state name"
    );
}

#[test]
fn work_states_reach_the_gui_surface_under_the_engine_names() {
    let text = std::fs::read_to_string(gui_contract_path()).unwrap();
    let gui = literals_between(&text, "export const workStates = [", "] as const");
    assert_eq!(
        gui,
        engine_states(),
        "GUI work states drifted from the engine vocabulary; a state the GUI \
         does not list makes isWorkRow reject the row and blanks the panel"
    );
}

#[test]
fn work_freshness_and_disclosure_reach_the_gui_surface_unrenamed() {
    let text = std::fs::read_to_string(gui_contract_path()).unwrap();
    let engine_freshness: Vec<String> = Freshness::ALL
        .iter()
        .map(|value| value.as_str().to_string())
        .collect();
    assert_eq!(
        literals_between(&text, "export const workFreshness = [", "] as const"),
        engine_freshness,
        "GUI freshness drifted from the engine vocabulary"
    );
    assert_eq!(
        literals_between(&text, "export const workDisclosures = [", "] as const"),
        Disclosure::ALL
            .iter()
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>(),
        "GUI disclosure drifted from the engine vocabulary"
    );
}

fn engine_verdicts() -> Vec<String> {
    EvidenceVerdict::ALL
        .iter()
        .map(|verdict| verdict.label().to_string())
        .collect()
}

#[test]
fn evidence_verdict_names_round_trip_and_all_is_complete() {
    for verdict in EvidenceVerdict::ALL {
        // An exhaustive match: a seventh verdict fails to compile here until it
        // is added to ALL, which every other surface is pinned to.
        match verdict {
            EvidenceVerdict::NotRun
            | EvidenceVerdict::Passed
            | EvidenceVerdict::Failed
            | EvidenceVerdict::Blocked
            | EvidenceVerdict::Unavailable
            | EvidenceVerdict::Superseded => {}
        }
        assert_eq!(
            serde_json::from_value::<EvidenceVerdict>(serde_json::json!(verdict.label())).unwrap(),
            verdict,
            "the wire name and the label disagree for {verdict:?}"
        );
    }
    let mut unique = engine_verdicts();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        EvidenceVerdict::ALL.len(),
        "duplicate verdict name"
    );
}

#[test]
fn evidence_verdicts_reach_the_gui_surface_under_the_engine_names() {
    let text = std::fs::read_to_string(gui_evidence_contract_path()).unwrap();
    assert_eq!(
        literals_between(&text, "export const evidenceVerdicts = [", "] as const"),
        engine_verdicts(),
        "GUI evidence verdicts drifted from the engine vocabulary; a verdict the \
         GUI does not list reads as `unknown`, and a renamed one lets the goal \
         card report an outcome the engine never gave"
    );
}

#[test]
fn work_overview_tool_schema_offers_every_state_the_projection_can_report() {
    let tool = crate::tools::tool_work_overview::ToolWorkOverview {
        config_path: String::new(),
    };
    let schema = crate::tools::tools_description::Tool::tool_description(&tool).input_schema;
    let offered: Vec<String> = schema["properties"]["states"]["items"]["enum"]
        .as_array()
        .expect("states filter enumerates the state vocabulary")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        offered,
        engine_states(),
        "the tool surface must offer every state the projection can report"
    );
}
