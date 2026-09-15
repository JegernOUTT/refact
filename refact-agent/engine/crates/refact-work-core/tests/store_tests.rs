use refact_work_core::contracts::{WorkOrigin, WorkPhase};
use refact_work_core::identity::{CheckoutKey, ProjectKey, RunKey, WorkKey};
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::filesystem::{classify_mounts, FilesystemKind};
use refact_work_core::store::records::{
    AccessKind, ActivityObservationRecord, DeliveryOutboxRecord, DeliveryState,
    EngineInstanceRecord, IntentState, LeaseAcquisition, LeaseRecord, PresenceMark,
    QueuedWorkRecord, RunIntentRecord,
};
use refact_work_core::store::schema;
use refact_work_core::store::{CoordinationStore, StoreError};
use std::path::Path;

fn settings() -> CoordinationSettings {
    CoordinationSettings::default()
}

async fn open(path: &Path) -> CoordinationStore {
    CoordinationStore::open_on_filesystem(path, &settings(), FilesystemKind::Local)
        .await
        .unwrap()
}

fn engine_instance(project: &str, instance_id: &str) -> EngineInstanceRecord {
    EngineInstanceRecord {
        project: ProjectKey::new(project),
        instance_id: instance_id.to_string(),
        pid: 4242,
        boot_id: format!("boot-{project}"),
        process_started_at_ms: 1_700_000_000_000,
        presence_mark: PresenceMark::Active,
        host_id: format!("host-{project}"),
        heartbeat_at_ms: 1_700_000_005_000,
    }
}

fn run_intent(project: &str, work_id: &str, generation: i64) -> RunIntentRecord {
    RunIntentRecord {
        project: ProjectKey::new(project),
        run: RunKey::new(WorkKey::trajectory(work_id), 1),
        generation,
        state: IntentState::Persisted,
        updated_at_ms: 1_700_000_010_000,
    }
}

fn queued_work(project: &str, work_id: &str) -> QueuedWorkRecord {
    QueuedWorkRecord {
        project: ProjectKey::new(project),
        work: WorkKey::background_agent(work_id),
        root: CheckoutKey::new(format!("root-{project}")),
        origin: WorkOrigin::Subchat,
        phase: WorkPhase::Admitted,
        enqueued_at_ms: 1_700_000_020_000,
        payload: format!("{{\"work\":\"{work_id}\"}}"),
    }
}

fn lease(project: &str, lease_key: &str) -> LeaseRecord {
    LeaseRecord {
        project: ProjectKey::new(project),
        lease_key: lease_key.to_string(),
        holder_instance: format!("engine-{project}"),
        owner_token: format!("owner-{project}"),
        fencing_generation: 7,
        acquired_at_ms: 1_700_000_030_000,
        expires_at_ms: 1_700_000_090_000,
    }
}

fn outbox(project: &str, message_id: &str) -> DeliveryOutboxRecord {
    DeliveryOutboxRecord {
        project: ProjectKey::new(project),
        message_id: message_id.to_string(),
        recipient: format!("agent-{project}"),
        state: DeliveryState::Pending,
        attempts: 2,
        created_at_ms: 1_700_000_040_000,
        updated_at_ms: 1_700_000_041_000,
        payload: format!("{{\"id\":\"{message_id}\"}}"),
    }
}

fn observation(project: &str, record_id: &str, observed_at_ms: i64) -> ActivityObservationRecord {
    ActivityObservationRecord {
        project: ProjectKey::new(project),
        record_id: record_id.to_string(),
        work: WorkKey::trajectory("traj-1"),
        path: format!("src/{record_id}.rs"),
        access: AccessKind::Write,
        content_hash: Some(format!("hash-{record_id}")),
        observed_at_ms,
        file_key: format!("/{project}/src/{record_id}.rs"),
        file_alias: Some(format!("id:66:{record_id}")),
        instance_id: format!("engine-{project}"),
        tool_call: Some("patch".to_string()),
        content_hash_before: Some(format!("before-{record_id}")),
        checkout: Some(format!("/{project}")),
    }
}

#[tokio::test]
async fn open_migrate_and_reopen_at_the_same_version_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");

    let store = open(&path).await;
    assert_eq!(
        store.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    let access = store.authorize(ProjectKey::new("alpha"));
    store
        .put_lease(&access, &lease("alpha", "build"))
        .await
        .unwrap();
    store.close().await.unwrap();

    let reopened = open(&path).await;
    assert_eq!(
        reopened.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    let access = reopened.authorize(ProjectKey::new("alpha"));
    let stored = reopened.get_lease(&access, "build").await.unwrap().unwrap();
    assert_eq!(stored, lease("alpha", "build"));

    let third = open(&path).await;
    assert_eq!(
        third.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
}

#[tokio::test]
async fn older_schema_migrates_forward_and_keeps_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    assert!(schema::SCHEMA_VERSION > 1);
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(schema::MIGRATIONS[0]).unwrap();
        conn.execute(
            "INSERT INTO leases (project, lease_key, owner_token, fencing_generation, \
             acquired_at_ms, expires_at_ms) VALUES ('alpha', 'build', 'owner-alpha', 7, \
             1700000030000, 1700000090000)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
    }

    let store = open(&path).await;
    assert_eq!(
        store.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );

    let access = store.authorize(ProjectKey::new("alpha"));
    assert_eq!(
        store.get_lease(&access, "build").await.unwrap().unwrap(),
        LeaseRecord {
            holder_instance: String::new(),
            ..lease("alpha", "build")
        },
        "a row written before holders were recorded migrates with an empty holder"
    );
    let observed = observation("alpha", "obs-1", 1_700_000_050_000);
    store
        .put_activity_observation(&access, &observed)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_activity_observation(&access, "obs-1")
            .await
            .unwrap()
            .unwrap(),
        observed
    );
}

#[tokio::test]
async fn newer_schema_fails_closed_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    open(&path).await.close().await.unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO leases (project, lease_key, owner_token, fencing_generation, \
             acquired_at_ms, expires_at_ms) VALUES ('alpha', 'build', 'owner-alpha', 7, 1, 2)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", schema::SCHEMA_VERSION + 5)
            .unwrap();
    }

    let err = CoordinationStore::open_on_filesystem(&path, &settings(), FilesystemKind::Local)
        .await
        .unwrap_err();
    match err {
        StoreError::SchemaTooNew { found, supported } => {
            assert_eq!(found, schema::SCHEMA_VERSION + 5);
            assert_eq!(supported, schema::SCHEMA_VERSION);
        }
        other => panic!("expected SchemaTooNew, got {other}"),
    }

    let conn = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM leases", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, schema::SCHEMA_VERSION + 5);
}

#[tokio::test]
async fn project_scope_blocks_cross_project_reads_and_mutations() {
    let store = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let alpha = store.authorize(ProjectKey::new("alpha"));
    let beta = store.authorize(ProjectKey::new("beta"));

    store
        .put_engine_instance(&beta, &engine_instance("beta", "engine-1"))
        .await
        .unwrap();
    store
        .put_lease(&beta, &lease("beta", "shared-key"))
        .await
        .unwrap();
    store
        .put_lease(&alpha, &lease("alpha", "shared-key"))
        .await
        .unwrap();

    assert!(store
        .get_engine_instance(&alpha, "engine-1")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .list_engine_instances(&alpha)
        .await
        .unwrap()
        .is_empty());
    assert!(!store
        .delete_engine_instance(&alpha, "engine-1")
        .await
        .unwrap());
    assert!(store
        .get_engine_instance(&beta, "engine-1")
        .await
        .unwrap()
        .is_some());

    let alpha_lease = store
        .get_lease(&alpha, "shared-key")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alpha_lease.owner_token, "owner-alpha");
    let beta_lease = store.get_lease(&beta, "shared-key").await.unwrap().unwrap();
    assert_eq!(beta_lease.owner_token, "owner-beta");

    let err = store
        .put_lease(&alpha, &lease("beta", "shared-key"))
        .await
        .unwrap_err();
    match err {
        StoreError::Unauthorized {
            requested,
            authorized,
        } => {
            assert_eq!(requested, "beta");
            assert_eq!(authorized, "alpha");
        }
        other => panic!("expected Unauthorized, got {other}"),
    }
    assert_eq!(
        store.get_lease(&beta, "shared-key").await.unwrap().unwrap(),
        lease("beta", "shared-key")
    );

    assert!(!store.delete_lease(&alpha, "missing").await.unwrap());
    assert_eq!(store.list_leases(&alpha).await.unwrap().len(), 1);
    assert_eq!(store.list_leases(&beta).await.unwrap().len(), 1);
}

#[tokio::test]
async fn access_token_from_another_store_is_refused() {
    let one = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let two = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let foreign = one.authorize(ProjectKey::new("alpha"));

    let err = two.list_leases(&foreign).await.unwrap_err();
    assert!(matches!(err, StoreError::ForeignAccessToken), "got {err}");
    let err = two
        .put_lease(&foreign, &lease("alpha", "build"))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::ForeignAccessToken), "got {err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_serialize_without_lost_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    let one = open(&path).await;
    let two = open(&path).await;
    let access_one = one.authorize(ProjectKey::new("alpha"));
    let access_two = two.authorize(ProjectKey::new("alpha"));

    let writer_one = async {
        for index in 0..50 {
            one.put_outbox_message(&access_one, &outbox("alpha", &format!("a-{index}")))
                .await
                .unwrap();
        }
    };
    let writer_two = async {
        for index in 0..50 {
            two.put_outbox_message(&access_two, &outbox("alpha", &format!("b-{index}")))
                .await
                .unwrap();
        }
    };
    tokio::join!(writer_one, writer_two);
    assert_eq!(
        one.list_outbox_messages(&access_one).await.unwrap().len(),
        100
    );

    one.put_run_intent(&access_one, &run_intent("alpha", "run-1", 0))
        .await
        .unwrap();
    let run = RunKey::new(WorkKey::trajectory("run-1"), 1);
    let (first, second) = tokio::join!(
        one.compare_and_set_run_intent(&access_one, &run, 0, IntentState::Dispatched, 111),
        two.compare_and_set_run_intent(&access_two, &run, 0, IntentState::Acknowledged, 222),
    );

    assert_eq!(
        u8::from(first.is_ok()) + u8::from(second.is_ok()),
        1,
        "exactly one compare-and-set must win"
    );
    let loser = if first.is_err() {
        first.unwrap_err()
    } else {
        second.unwrap_err()
    };
    assert!(
        matches!(
            loser,
            StoreError::GenerationConflict {
                expected: 0,
                found: 1
            }
        ),
        "got {loser}"
    );

    let settled = one
        .get_run_intent(&access_one, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.generation, 1);
    assert_eq!(
        two.get_run_intent(&access_two, &run)
            .await
            .unwrap()
            .unwrap(),
        settled
    );
}

#[tokio::test]
async fn compare_and_set_reports_a_missing_intent() {
    let store = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let access = store.authorize(ProjectKey::new("alpha"));
    let run = RunKey::new(WorkKey::trajectory("absent"), 3);
    let err = store
        .compare_and_set_run_intent(&access, &run, 0, IntentState::Terminal, 1)
        .await
        .unwrap_err();
    match err {
        StoreError::NotFound { kind, id } => {
            assert_eq!(kind, "run intent");
            assert_eq!(id, run.to_string());
        }
        other => panic!("expected NotFound, got {other}"),
    }
}

#[tokio::test]
async fn corrupt_database_reports_a_specific_error_and_destroys_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    let garbage = "this file is not a database, it is a poem\n".repeat(128);
    std::fs::write(&path, &garbage).unwrap();

    let err = CoordinationStore::open_on_filesystem(&path, &settings(), FilesystemKind::Local)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Corrupt { .. }), "got {err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), garbage);
}

#[tokio::test]
async fn every_record_type_round_trips_all_fields() {
    let store = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let access = store.authorize(ProjectKey::new("alpha"));

    let instance = engine_instance("alpha", "engine-1");
    store.put_engine_instance(&access, &instance).await.unwrap();
    assert_eq!(
        store
            .get_engine_instance(&access, "engine-1")
            .await
            .unwrap()
            .unwrap(),
        instance
    );

    let intent = run_intent("alpha", "run-1", 4);
    store.put_run_intent(&access, &intent).await.unwrap();
    assert_eq!(
        store
            .get_run_intent(&access, &intent.run)
            .await
            .unwrap()
            .unwrap(),
        intent
    );

    let queued = queued_work("alpha", "agent-1");
    store.put_queued_work(&access, &queued).await.unwrap();
    assert_eq!(
        store
            .get_queued_work(&access, &queued.work)
            .await
            .unwrap()
            .unwrap(),
        queued
    );

    let held = lease("alpha", "build");
    store.put_lease(&access, &held).await.unwrap();
    assert_eq!(
        store.get_lease(&access, "build").await.unwrap().unwrap(),
        held
    );

    let message = outbox("alpha", "msg-1");
    store.put_outbox_message(&access, &message).await.unwrap();
    assert_eq!(
        store
            .get_outbox_message(&access, "msg-1")
            .await
            .unwrap()
            .unwrap(),
        message
    );

    let observed = observation("alpha", "obs-1", 1_700_000_050_000);
    store
        .put_activity_observation(&access, &observed)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_activity_observation(&access, "obs-1")
            .await
            .unwrap()
            .unwrap(),
        observed
    );

    let mut without_hash = observation("alpha", "obs-2", 1_700_000_060_000);
    without_hash.content_hash = None;
    store
        .put_activity_observation(&access, &without_hash)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_activity_observation(&access, "obs-2")
            .await
            .unwrap()
            .unwrap(),
        without_hash
    );

    assert!(store
        .delete_queued_work(&access, &queued.work)
        .await
        .unwrap());
    assert!(store.delete_lease(&access, "build").await.unwrap());
    assert!(store.delete_outbox_message(&access, "msg-1").await.unwrap());
    assert!(store
        .delete_activity_observation(&access, "obs-1")
        .await
        .unwrap());
    assert!(store
        .delete_engine_instance(&access, "engine-1")
        .await
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_lease_acquirers_produce_exactly_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    let one = open(&path).await;
    let two = open(&path).await;
    let access_one = one.authorize(ProjectKey::new("alpha"));
    let access_two = two.authorize(ProjectKey::new("alpha"));

    let (first, second) = tokio::join!(
        one.try_acquire_lease(&access_one, "build", "engine-1", "token-1", 100, 1_100),
        two.try_acquire_lease(&access_two, "build", "engine-2", "token-2", 100, 1_100),
    );
    let acquired = [first.unwrap(), second.unwrap()];
    let winners = acquired
        .iter()
        .filter(|outcome| matches!(outcome, LeaseAcquisition::Acquired(_)))
        .count();

    assert_eq!(winners, 1, "exactly one acquirer may own the lease");
    let held = one.get_lease(&access_one, "build").await.unwrap().unwrap();
    assert_eq!(held.fencing_generation, 1);
    for outcome in &acquired {
        match outcome {
            LeaseAcquisition::Acquired(record) | LeaseAcquisition::Held(record) => {
                assert_eq!(record, &held)
            }
        }
    }
}

#[tokio::test]
async fn a_fenced_write_from_a_replaced_holder_never_lands() {
    let store = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let access = store.authorize(ProjectKey::new("alpha"));
    let run = RunKey::new(WorkKey::trajectory("run-1"), 1);
    store
        .put_run_intent(&access, &run_intent("alpha", "run-1", 0))
        .await
        .unwrap();
    let LeaseAcquisition::Acquired(old) = store
        .try_acquire_lease(&access, "run-1", "engine-1", "token-1", 100, 1_100)
        .await
        .unwrap()
    else {
        panic!("a free lease must be acquirable");
    };
    let renewed = store
        .renew_lease(&access, &old.authority(), 200, 2_100)
        .await
        .unwrap();
    assert_eq!(renewed.fencing_generation, old.fencing_generation);

    let taken = store
        .take_over_lease(
            &access,
            "run-1",
            old.fencing_generation,
            "engine-2",
            "token-2",
            3_000,
            4_000,
        )
        .await
        .unwrap();
    let stale = store
        .compare_and_set_run_intent_fenced(
            &access,
            &old.authority(),
            3_100,
            &run,
            0,
            IntentState::Dispatched,
            3_100,
        )
        .await
        .unwrap_err();
    let fresh = store
        .compare_and_set_run_intent_fenced(
            &access,
            &taken.authority(),
            3_100,
            &run,
            0,
            IntentState::Acknowledged,
            3_100,
        )
        .await
        .unwrap();

    assert_eq!(taken.fencing_generation, old.fencing_generation + 1);
    assert!(
        matches!(
            stale,
            StoreError::LeaseFenced {
                presented: 1,
                held: 2,
                ..
            }
        ),
        "got {stale}"
    );
    assert_eq!(fresh.state, IntentState::Acknowledged);
    assert!(!store
        .release_lease(&access, &old.authority())
        .await
        .unwrap());
    assert!(store
        .release_lease(&access, &taken.authority())
        .await
        .unwrap());
}

#[tokio::test]
async fn prune_enforces_retention_per_project() {
    let mut tight = settings();
    tight.store_max_activity_observations = 3;
    let store = CoordinationStore::open_in_memory(&tight).await.unwrap();
    let alpha = store.authorize(ProjectKey::new("alpha"));
    let beta = store.authorize(ProjectKey::new("beta"));

    for index in 0..10i64 {
        store
            .put_activity_observation(
                &alpha,
                &observation("alpha", &format!("obs-{index}"), index),
            )
            .await
            .unwrap();
    }
    for index in 0..2i64 {
        store
            .put_activity_observation(&beta, &observation("beta", &format!("obs-{index}"), index))
            .await
            .unwrap();
    }

    let report = store.prune(&alpha).await.unwrap();
    assert_eq!(report.activity_observations, 7);
    assert_eq!(report.total(), 7);
    let kept = store.list_activity_observations(&alpha).await.unwrap();
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0].record_id, "obs-9");
    assert_eq!(
        store.list_activity_observations(&beta).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn file_provenance_lookup_deletion_and_eviction_stay_project_scoped() {
    let store = CoordinationStore::open_in_memory(&settings())
        .await
        .unwrap();
    let alpha = store.authorize(ProjectKey::new("alpha"));
    let beta = store.authorize(ProjectKey::new("beta"));
    let key = "/alpha/src/obs-1.rs";

    for project in ["alpha", "beta"] {
        let access = if project == "alpha" { &alpha } else { &beta };
        let mut record = observation(project, "obs-1", 1_700_000_050_000);
        record.file_key = key.to_string();
        store
            .put_activity_observation(access, &record)
            .await
            .unwrap();
    }
    let mut older = observation("alpha", "obs-0", 1_700_000_040_000);
    older.file_key = key.to_string();
    store
        .put_activity_observation(&alpha, &older)
        .await
        .unwrap();

    let found = store
        .last_activity_for_file(&alpha, key, AccessKind::Write, 0, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.record_id, "obs-1");
    assert_eq!(found.instance_id, "engine-alpha");
    assert_eq!(found.content_hash_before.as_deref(), Some("before-obs-1"));
    assert!(store
        .last_activity_for_file(&alpha, key, AccessKind::Write, 0, Some("traj-1"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .last_activity_for_file(&alpha, key, AccessKind::Read, 0, None)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .last_activity_for_file(&alpha, key, AccessKind::Write, 1_700_000_050_001, None)
        .await
        .unwrap()
        .is_none());

    assert_eq!(
        store.prune_activity_observations(&alpha, 1).await.unwrap(),
        1
    );
    assert_eq!(
        store
            .list_activity_observations(&alpha)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .delete_activity_observations(&alpha, Some(key))
            .await
            .unwrap(),
        1
    );
    assert!(store
        .last_activity_for_file(&alpha, key, AccessKind::Write, 0, None)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .last_activity_for_file(&beta, key, AccessKind::Write, 0, None)
            .await
            .unwrap()
            .unwrap()
            .project,
        ProjectKey::new("beta")
    );
    assert_eq!(
        store
            .delete_activity_observations(&beta, None)
            .await
            .unwrap(),
        1
    );
    assert!(store
        .list_activity_observations(&beta)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn non_local_filesystem_is_refused_unless_explicitly_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("net").join("coordination.sqlite");
    let kind = FilesystemKind::NonLocal {
        fs_type: "nfs4".to_string(),
        mount_point: "/mnt/team".to_string(),
    };

    let err = CoordinationStore::open_on_filesystem(&path, &settings(), kind.clone())
        .await
        .unwrap_err();
    match err {
        StoreError::NonLocalFilesystem { fs_type, .. } => assert_eq!(fs_type, "nfs4"),
        other => panic!("expected NonLocalFilesystem, got {other}"),
    }
    assert!(!path.exists());

    let mut permissive = settings();
    permissive.store_allow_non_local_filesystem = true;
    let store = CoordinationStore::open_on_filesystem(&path, &permissive, kind)
        .await
        .unwrap();
    assert_eq!(store.warnings().len(), 1);
    assert!(path.exists());
}

#[tokio::test]
async fn invalid_settings_are_rejected_at_open() {
    let mut invalid = settings();
    invalid.store_busy_timeout_ms = 0;
    let err = CoordinationStore::open_in_memory(&invalid)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Settings(_)), "got {err}");

    let mut invalid = settings();
    invalid.store_max_leases = 0;
    let err = CoordinationStore::open_in_memory(&invalid)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Settings(_)), "got {err}");
}

#[test]
fn classify_mounts_picks_the_deepest_matching_mount_point() {
    let mounts = "/dev/sda1 / ext4 rw,relatime 0 0\n\
                  server:/export /home/dev/shared nfs4 rw 0 0\n\
                  tmpfs /tmp tmpfs rw 0 0\n";

    assert_eq!(
        classify_mounts(mounts, Path::new("/home/dev/project/db.sqlite")),
        FilesystemKind::Local
    );
    assert_eq!(
        classify_mounts(mounts, Path::new("/tmp/db.sqlite")),
        FilesystemKind::Local
    );
    match classify_mounts(mounts, Path::new("/home/dev/shared/db.sqlite")) {
        FilesystemKind::NonLocal {
            fs_type,
            mount_point,
        } => {
            assert_eq!(fs_type, "nfs4");
            assert_eq!(mount_point, "/home/dev/shared");
        }
        other => panic!("expected NonLocal, got {other:?}"),
    }
    assert_eq!(
        classify_mounts("", Path::new("/home/dev/db.sqlite")),
        FilesystemKind::Unknown
    );
}

#[test]
fn classify_mounts_decodes_escaped_mount_points() {
    let mounts = "/dev/sda1 / ext4 rw 0 0\n\
                  //server/team /mnt/team\\040share cifs rw 0 0\n";
    match classify_mounts(mounts, Path::new("/mnt/team share/db.sqlite")) {
        FilesystemKind::NonLocal {
            fs_type,
            mount_point,
        } => {
            assert_eq!(fs_type, "cifs");
            assert_eq!(mount_point, "/mnt/team share");
        }
        other => panic!("expected NonLocal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_peer_connection_predating_the_wal_switch_does_not_cost_the_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordination.sqlite");
    let peer = rusqlite::Connection::open(&path).unwrap();
    let before: String = peer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, "delete");

    let store = CoordinationStore::open_on_filesystem(&path, &settings(), FilesystemKind::Local)
        .await
        .expect("a peer holding the database open must not cost this engine its durability");

    let observer = rusqlite::Connection::open(&path).unwrap();
    let mode: String = observer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_first_opens_of_one_store_all_succeed() {
    for attempt in 0..20 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let mut opening = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            opening.push(tokio::spawn(async move {
                CoordinationStore::open_on_filesystem(&path, &settings(), FilesystemKind::Local)
                    .await
            }));
        }
        let mut held = Vec::new();
        for opened in opening {
            match opened.await.unwrap() {
                Ok(store) => held.push(store),
                Err(error) => panic!(
                    "attempt {attempt}: a store another connection is still holding open must \
                     not cost this one its durability: {error}"
                ),
            }
        }
    }
}
