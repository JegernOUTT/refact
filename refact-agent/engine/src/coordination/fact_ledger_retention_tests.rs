use std::sync::Arc;

use refact_work_core::{CoordinationSettings, CoordinationStore, ProjectKey};

use super::fact_ledger::{self, FactLedger};

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
