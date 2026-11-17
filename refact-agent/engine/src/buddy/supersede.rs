use chrono::{DateTime, FixedOffset};
use refact_buddy_core::operator_types::RunStatus;

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub id: String,
    pub status: RunStatus,
    pub observed_at: DateTime<FixedOffset>,
    pub eligible: bool,
}

pub(crate) fn coverage_class(coverage: &str) -> String {
    let mut class = String::with_capacity(coverage.len());
    let mut characters = coverage.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_ascii_digit() {
            while matches!(characters.peek(), Some(next) if next.is_ascii_digit()) {
                characters.next();
            }
            class.push('#');
        } else {
            class.push(character);
        }
    }
    class
}

pub(crate) fn retire_ids(candidates: &[Candidate]) -> Vec<String> {
    candidates
        .iter()
        .filter(|older| {
            older.eligible
                && candidates.iter().any(|newer| {
                    newer.eligible
                        && newer_than(newer, older)
                        && supersedes(&newer.status, &older.status)
                })
        })
        .map(|candidate| candidate.id.clone())
        .collect()
}

fn newer_than(newer: &Candidate, older: &Candidate) -> bool {
    newer.observed_at > older.observed_at
        || (newer.observed_at == older.observed_at && newer.id > older.id)
}

fn supersedes(newer: &RunStatus, older: &RunStatus) -> bool {
    let newer_assessment = is_assessment(newer);
    let older_assessment = is_assessment(older);
    let newer_failure = is_failure(newer);
    let older_failure = is_failure(older);
    (newer_assessment && (older_assessment || older_failure)) || (newer_failure && older_failure)
}

fn is_assessment(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Completed | RunStatus::Inconclusive | RunStatus::NoChange | RunStatus::Partial
    )
}

fn is_failure(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Failed | RunStatus::Blocked | RunStatus::Cancelled
    )
}
