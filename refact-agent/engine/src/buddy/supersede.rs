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
pub(crate) fn with_open_pass_disabled<R>(operation: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            OPEN_PASS_ENABLED.with(|enabled| enabled.set(self.0));
        }
    }
    OPEN_PASS_ENABLED.with(|enabled| {
        let previous = enabled.replace(false);
        let _reset = Reset(previous);
        operation()
    })
}

#[cfg(test)]
pub(crate) fn with_interruption_after_groups<R>(groups: usize, operation: impl FnOnce() -> R) -> R {
    struct Reset(Option<usize>);
    impl Drop for Reset {
        fn drop(&mut self) {
            INTERRUPT_AFTER_GROUPS.with(|value| value.set(self.0));
        }
    }
    INTERRUPT_AFTER_GROUPS.with(|value| {
        let previous = value.replace(Some(groups));
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

    #[test]
    fn assessment_failure_assessment_selects_old_rows_independent_of_order() {
        let candidates = [
            candidate("a1", RunStatus::Inconclusive, "2026-01-01T00:00:00Z"),
            candidate("f2", RunStatus::Failed, "2026-01-02T00:00:00Z"),
            candidate("a3", RunStatus::Inconclusive, "2026-01-03T00:00:00Z"),
        ];
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let ordered: Vec<_> = order.map(|index| candidates[index].clone()).into();
            let mut retired = retire_ids(&ordered);
            retired.sort();
            assert_eq!(retired, vec!["a1", "f2"]);
        }
    }

    #[test]
    fn newer_failure_does_not_supersede_assessment() {
        let candidates = [
            candidate("a1", RunStatus::Inconclusive, "2026-01-01T00:00:00Z"),
            candidate("f2", RunStatus::Failed, "2026-01-02T00:00:00Z"),
        ];
        assert!(retire_ids(&candidates).is_empty());
        assert!(retire_ids(&[candidates[1].clone(), candidates[0].clone()]).is_empty());
    }

    #[test]
    fn ties_use_larger_work_id_and_ineligible_rows_never_retire() {
        let mut candidates = [
            candidate("a", RunStatus::Inconclusive, "2026-01-01T00:00:00Z"),
            candidate("b", RunStatus::Inconclusive, "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(retire_ids(&candidates), vec!["a"]);
        candidates[1].eligible = false;
        assert!(retire_ids(&candidates).is_empty());
    }

    #[test]
    fn real_security_whisperer_coverage_counts_share_a_class() {
        let coverages = [5, 5, 10, 10, 11, 11, 11, 11, 11, 13, 0, 6, 7, 8]
            .map(|count| {
                format!(
                    "Secret-pattern scan of staged and working-tree contents of every changed path under [REDACTED_PATH] (text files up to 65536 bytes), plus {count} recent diagnostics"
                )
            });
        let expected = "Secret-pattern scan of staged and working-tree contents of every changed path under [REDACTED_PATH] (text files up to # bytes), plus # recent diagnostics";
        assert!(coverages
            .iter()
            .all(|coverage| coverage_class(coverage) == expected));
    }

    #[test]
    fn coverage_class_preserves_non_digit_text() {
        let redacted = "source under [REDACTED_PATH] plus 11 diagnostics";
        let other_root = "source under /other/root plus 8 diagnostics";
        assert_ne!(coverage_class(redacted), coverage_class(other_root));
        assert_ne!(
            coverage_class("knowledge base"),
            coverage_class("source tree")
        );
        assert_ne!(coverage_class("coverage-a"), coverage_class("coverage-b"));
    }
}
