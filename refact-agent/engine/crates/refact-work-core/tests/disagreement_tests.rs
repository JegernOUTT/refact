use refact_work_core::identity::{ProjectKey, WorkKey};
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{
    AdjudicationRefusal, AdjudicationVerdict, DisagreementPosition, DisagreementRecord,
    DisagreementStatus, EvidenceKind, EvidenceRef,
};
use refact_work_core::store::{CoordinationStore, DisagreementFiling};

const REVISION: &str = "sha-1111";

fn evidence(kind: EvidenceKind, reference: &str) -> EvidenceRef {
    EvidenceRef::new(kind, reference)
}

fn position(
    position_id: &str,
    author: &str,
    statement: &str,
    evidence_refs: Vec<EvidenceRef>,
    revision: &str,
) -> DisagreementPosition {
    DisagreementPosition {
        position_id: position_id.to_string(),
        author: WorkKey::background_agent(author),
        run_generation: 1,
        statement: statement.to_string(),
        evidence: evidence_refs,
        asserted_at_ms: 1_700_000_000_000,
        subject_revision: revision.to_string(),
    }
}

fn record(record_id: &str) -> DisagreementRecord {
    DisagreementRecord {
        project: ProjectKey::new("alpha"),
        record_id: record_id.to_string(),
        claim: "the retry budget is exhausted before the breaker opens".to_string(),
        subject: "src/providers/retry.rs".to_string(),
        positions: vec![
            position(
                "for",
                "agent-a",
                "the budget runs out first",
                vec![evidence(EvidenceKind::TestRun, "retry::budget_exhausted")],
                REVISION,
            ),
            position(
                "against",
                "agent-b",
                "the breaker opens first",
                vec![evidence(EvidenceKind::File, "src/providers/retry.rs:88")],
                REVISION,
            ),
        ],
        status: DisagreementStatus::Open,
        adjudication: None,
        open_reason: None,
        filed_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_000_000,
    }
}

async fn store(settings: &CoordinationSettings) -> CoordinationStore {
    CoordinationStore::open_in_memory(settings).await.unwrap()
}

#[test]
fn adjudication_keeps_the_rejected_position_readable_and_cites_its_evidence() {
    let mut filed = record("d-1");
    let decisive = evidence(EvidenceKind::TestRun, "retry::budget_exhausted");

    let status = filed
        .resolve(
            "reviewer",
            AdjudicationVerdict::Decided {
                position_id: "for".to_string(),
                decisive_evidence: decisive.clone(),
                rationale: "the run reproduces the exhaustion".to_string(),
            },
            REVISION,
            1_700_000_100_000,
        )
        .unwrap();

    assert_eq!(status, DisagreementStatus::Adjudicated);
    let adjudication = filed.adjudication.as_ref().unwrap();
    assert_eq!(adjudication.decisive_evidence, decisive);
    assert_eq!(adjudication.resolved_position, "for");
    assert_eq!(filed.positions.len(), 2, "no position may be deleted");
    let rejected = filed.rejected_positions();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].position_id, "against");
    assert_eq!(rejected[0].statement, "the breaker opens first");
    assert!(!rejected[0].evidence.is_empty());
}

#[test]
fn evidence_the_winning_position_never_presented_cannot_decide_it() {
    let mut filed = record("d-2");

    let refusal = filed
        .resolve(
            "reviewer",
            AdjudicationVerdict::Decided {
                position_id: "for".to_string(),
                decisive_evidence: evidence(EvidenceKind::File, "src/providers/retry.rs:88"),
                rationale: "borrowed from the other side".to_string(),
            },
            REVISION,
            1_700_000_100_000,
        )
        .unwrap_err();

    assert_eq!(
        refusal,
        AdjudicationRefusal::EvidenceNotPresented("for".to_string())
    );
    assert_eq!(filed.status, DisagreementStatus::Open);
    assert!(filed.adjudication.is_none());
}

#[test]
fn a_revision_that_moved_makes_the_record_stale_rather_than_adjudicated() {
    let mut filed = record("d-4");
    assert!(!filed.is_stale_against(REVISION));
    assert_eq!(filed.asserted_revision(), Some(REVISION));
    assert!(filed.is_stale_against("sha-2222"));

    let refusal = filed
        .resolve(
            "reviewer",
            AdjudicationVerdict::Decided {
                position_id: "for".to_string(),
                decisive_evidence: evidence(EvidenceKind::TestRun, "retry::budget_exhausted"),
                rationale: "still convincing".to_string(),
            },
            "sha-2222",
            1_700_000_100_000,
        )
        .unwrap_err();

    assert_eq!(
        refusal,
        AdjudicationRefusal::Stale {
            asserted_against: REVISION.to_string(),
            current: "sha-2222".to_string(),
        }
    );
    assert_eq!(filed.status, DisagreementStatus::Stale);
    assert!(filed.adjudication.is_none());
    assert_ne!(filed.status, DisagreementStatus::Adjudicated);
    assert_eq!(
        filed.resolve(
            "reviewer",
            AdjudicationVerdict::Unresolvable {
                reason: "re-argue it".to_string()
            },
            "sha-2222",
            1_700_000_200_000,
        ),
        Err(AdjudicationRefusal::NotOpen(DisagreementStatus::Stale))
    );
}

#[tokio::test]
async fn a_second_filing_never_overwrites_the_positions_already_argued() {
    let settings = CoordinationSettings::default();
    let store = store(&settings).await;
    let access = store.authorize(ProjectKey::new("alpha"));

    let first = record("d-shared");
    assert!(matches!(
        store.file_disagreement(&access, &first).await.unwrap(),
        DisagreementFiling::Filed(_)
    ));

    let mut later = record("d-shared");
    later.claim = "the retry budget is fine".to_string();
    later.positions = vec![position(
        "only-mine",
        "agent-c",
        "there was never a dispute",
        vec![],
        REVISION,
    )];
    let outcome = store.file_disagreement(&access, &later).await.unwrap();

    let DisagreementFiling::Exists(existing) = outcome else {
        panic!("a filed disagreement must not be replaced by a later writer");
    };
    assert_eq!(existing.claim, first.claim);
    assert_eq!(existing.positions, first.positions);
    let stored = store
        .get_disagreement(&access, "d-shared")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.positions, first.positions);
}

#[tokio::test]
async fn writing_a_resolution_cannot_rewrite_either_position() {
    let settings = CoordinationSettings::default();
    let store = store(&settings).await;
    let access = store.authorize(ProjectKey::new("alpha"));
    let filed = record("d-resolve");
    store.file_disagreement(&access, &filed).await.unwrap();

    let mut resolved = filed.clone();
    resolved
        .resolve(
            "reviewer",
            AdjudicationVerdict::Decided {
                position_id: "against".to_string(),
                decisive_evidence: evidence(EvidenceKind::File, "src/providers/retry.rs:88"),
                rationale: "the line settles the order".to_string(),
            },
            REVISION,
            1_700_000_100_000,
        )
        .unwrap();
    resolved.positions[0].statement = "rewritten by the winner".to_string();
    assert!(store
        .put_disagreement_resolution(&access, &resolved)
        .await
        .unwrap());

    let stored = store
        .get_disagreement(&access, "d-resolve")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, DisagreementStatus::Adjudicated);
    assert_eq!(
        stored.positions, filed.positions,
        "a resolution write must leave both arguments untouched"
    );
    assert_eq!(
        stored.adjudication.as_ref().unwrap().decisive_evidence,
        evidence(EvidenceKind::File, "src/providers/retry.rs:88")
    );
    assert_eq!(stored.rejected_positions()[0].position_id, "for");
}

#[tokio::test]
async fn the_open_record_cap_refuses_a_filing_instead_of_evicting_a_dispute() {
    let mut settings = CoordinationSettings::default();
    settings.disagreement_max_open = 2;
    let store = store(&settings).await;
    let access = store.authorize(ProjectKey::new("alpha"));

    store
        .file_disagreement(&access, &record("d-open-1"))
        .await
        .unwrap();
    store
        .file_disagreement(&access, &record("d-open-2"))
        .await
        .unwrap();
    let refused = store
        .file_disagreement(&access, &record("d-open-3"))
        .await
        .unwrap();

    assert_eq!(refused, DisagreementFiling::OpenLimit(2));
    assert_eq!(
        store
            .list_disagreements(&access, Some(DisagreementStatus::Open))
            .await
            .unwrap()
            .len(),
        2
    );

    let mut settled = record("d-open-1");
    settled.withdraw(1_700_000_100_000);
    store
        .put_disagreement_resolution(&access, &settled)
        .await
        .unwrap();
    assert!(matches!(
        store
            .file_disagreement(&access, &record("d-open-3"))
            .await
            .unwrap(),
        DisagreementFiling::Filed(_)
    ));
}
