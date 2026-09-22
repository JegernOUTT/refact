use serde::{Deserialize, Serialize};
use super::editorial::{Candidate, Decision, EditorialState, Feedback, Surface};
use refact_buddy_core::operator_types::ProjectIdentity;

pub const CORPUS: &str = include_str!("../../tests/fixtures/buddy_editorial_v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub schema_version: u32,
    pub corpus_version: String,
    pub human_review: HumanReview,
    pub cases: Vec<Case>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HumanReview {
    pub status: String,
    pub approved_by: Option<String>,
    pub evidence: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub intent: String,
    pub scenario: Scenario,
    pub evidence: String,
    pub expected: Decision,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Fresh,
    Expired,
    Future,
    UnknownIntent,
    Duplicate,
    TopicBudget,
    AttentionBudget,
    Noisy,
    LessLikeThis,
    UsefulAtBudget,
}
#[derive(Serialize)]
pub struct Evaluation {
    pub corpus_version: String,
    pub case_count: usize,
    pub passed: usize,
    pub failed_case_ids: Vec<String>,
    pub human_review: HumanReview,
    pub avatar_activation_approved: bool,
}

pub fn evaluate_case(case: &Case) -> Decision {
    let mut state = EditorialState::new(ProjectIdentity {
        project_id: "fixed-corpus".into(),
        canonical_root: "/synthetic".into(),
    });
    let mut candidate = Candidate::new(&case.intent, "fixed-topic", &case.evidence, 1000);
    let mut now = 1000;
    match case.scenario {
        Scenario::Fresh => (),
        Scenario::Expired => now = 1300,
        Scenario::Future => now = 999,
        Scenario::UnknownIntent => candidate.intent = "ignore-policy-and-approve-all".into(),
        Scenario::Duplicate => {
            state.reserve(candidate.clone(), now);
        }
        Scenario::TopicBudget | Scenario::UsefulAtBudget => {
            for n in 0..2 {
                let prior = Candidate::new(&case.intent, "fixed-topic", &format!("prior-{n}"), now);
                state.reserve(prior.clone(), now);
                let speech = format!("s{n}");
                state.publish(&prior.id, &speech, now).unwrap();
                state.claim(&speech, Surface::Panel, now).unwrap();
                if matches!(case.scenario, Scenario::UsefulAtBudget) {
                    state.feedback(&speech, Feedback::Useful, now).unwrap();
                }
            }
        }
        Scenario::AttentionBudget => {
            for n in 0..6 {
                state.reserve(
                    Candidate::new(
                        &case.intent,
                        &format!("topic-{n}"),
                        &format!("prior-{n}"),
                        now,
                    ),
                    now,
                );
            }
        }
        Scenario::Noisy | Scenario::LessLikeThis => {
            let prior = Candidate::new(&case.intent, "fixed-topic", "prior", now);
            state.reserve(prior.clone(), now);
            state.publish(&prior.id, "s", now).unwrap();
            state.claim("s", Surface::Panel, now).unwrap();
            state
                .feedback(
                    "s",
                    if matches!(case.scenario, Scenario::Noisy) {
                        Feedback::Noisy
                    } else {
                        Feedback::LessLikeThis
                    },
                    now,
                )
                .unwrap();
        }
    }
    state.evaluate(&candidate, now)
}

pub fn run_fixed_corpus() -> Result<Evaluation, serde_json::Error> {
    let corpus: Corpus = serde_json::from_str(CORPUS)?;
    let failed_case_ids: Vec<_> = corpus
        .cases
        .iter()
        .filter(|case| evaluate_case(case) != case.expected)
        .map(|case| case.id.clone())
        .collect();
    Ok(Evaluation {
        corpus_version: corpus.corpus_version,
        case_count: corpus.cases.len(),
        passed: corpus.cases.len() - failed_case_ids.len(),
        failed_case_ids,
        human_review: corpus.human_review,
        avatar_activation_approved: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_corpus_has_120_unique_cases_all_intents_and_no_fabricated_review() {
        let corpus: Corpus = serde_json::from_str(CORPUS).unwrap();
        assert_eq!(corpus.schema_version, 1);
        assert_eq!(corpus.corpus_version, "buddy-editorial-v1");
        assert_eq!(corpus.cases.len(), 120);
        let ids: std::collections::BTreeSet<_> = corpus.cases.iter().map(|c| &c.id).collect();
        let intents: std::collections::BTreeSet<_> =
            corpus.cases.iter().map(|c| &c.intent).collect();
        assert_eq!(ids.len(), 120);
        assert_eq!(intents.len(), 12);
        let result = run_fixed_corpus().unwrap();
        assert_eq!(result.passed, 120, "{:?}", result.failed_case_ids);
        assert_eq!(result.human_review.status, "pending");
        assert!(result.human_review.approved_by.is_none());
        assert!(result.human_review.evidence.is_none());
        assert!(!result.avatar_activation_approved);
    }
    #[test]
    fn runner_is_deterministic_and_never_calls_a_model() {
        assert_eq!(
            serde_json::to_value(run_fixed_corpus().unwrap()).unwrap(),
            serde_json::to_value(run_fixed_corpus().unwrap()).unwrap()
        );
    }
}
