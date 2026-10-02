use std::sync::Arc;

use refact_work_core::store::records::{ClaimRecord, ClaimStatus};
use refact_work_core::{CoordinationSettings, CoordinationStore, ProjectKey};

use super::fact_ledger::{self, ClaimLanding, FactLedger};

struct Fixture {
    _temp: tempfile::TempDir,
    ledger: FactLedger,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        CoordinationStore::open(
            &temp.path().join("coordination.sqlite"),
            &CoordinationSettings::default(),
        )
        .await
        .unwrap(),
    );
    Fixture {
        _temp: temp,
        ledger: FactLedger::new(store, ProjectKey::new("alpha")),
    }
}

fn claim(claim_id: &str, assertion: &str, author: &str) -> ClaimRecord {
    fact_ledger::tests::claim(claim_id, assertion, author, 600)
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn recording_a_claim_prunes_settled_claims_past_retention() {
    let _settings = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.claim_retention_ms = 1;
    });
    let fixture = fixture().await;
    fact_ledger::tests::seed_claim(
        &fixture.ledger,
        claim("c-settled", "old conclusion", "agent-a"),
        ClaimStatus::Retracted,
        1_500,
    )
    .await;

    let incoming = claim("c-new", "new conclusion", "agent-b");
    let landed = fixture
        .ledger
        .record(&incoming, &fact_ledger::tests::test_policy())
        .await
        .unwrap();

    assert!(
        matches!(landed, ClaimLanding::Recorded { .. }),
        "{landed:?}"
    );
    assert!(fixture.ledger.get("c-settled").await.unwrap().is_none());
    assert!(fixture.ledger.get("c-new").await.unwrap().is_some());
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn standing_claim_survives_regardless_of_age() {
    let fixture = fixture().await;
    let mut standing = claim("c-standing", "still standing", "agent-a");
    standing.predicate = "an unrelated question".to_string();
    fact_ledger::tests::seed_claim(&fixture.ledger, standing, ClaimStatus::Active, 1_500).await;

    let incoming = claim("c-new", "new conclusion", "agent-b");
    fixture
        .ledger
        .record(&incoming, &fact_ledger::tests::test_policy())
        .await
        .unwrap();

    let stored = fixture.ledger.get("c-standing").await.unwrap().unwrap();
    assert_eq!(stored.status, ClaimStatus::Active);
}
