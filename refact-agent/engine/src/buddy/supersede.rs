use chrono::{DateTime, FixedOffset};
use refact_buddy_core::operator_types::RunStatus;

#[cfg(test)]
std::thread_local! {
    static PUBLICATION_BYPASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OPEN_PASS_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static INTERRUPT_AFTER_GROUPS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

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
    #[cfg(test)]
    if PUBLICATION_BYPASS.with(std::cell::Cell::get) {
        return Vec::new();
    }
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

#[cfg(test)]
pub(crate) fn with_publication_bypass<R>(operation: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            PUBLICATION_BYPASS.with(|bypass| bypass.set(self.0));
        }
    }
    PUBLICATION_BYPASS.with(|bypass| {
        let previous = bypass.replace(true);
        let _reset = Reset(previous);
        operation()
    })
}

#[cfg(test)]
pub(crate) fn should_interrupt_after_group(groups: usize) -> bool {
    INTERRUPT_AFTER_GROUPS.with(|value| {
        if value.get() == Some(groups) {
            value.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
pub(crate) fn open_pass_enabled() -> bool {
    OPEN_PASS_ENABLED.with(std::cell::Cell::get)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, status: RunStatus, observed_at: &str) -> Candidate {
        Candidate {
            id: id.into(),
            status,
            observed_at: DateTime::parse_from_rfc3339(observed_at).unwrap(),
            eligible: true,
        }
    }
}
