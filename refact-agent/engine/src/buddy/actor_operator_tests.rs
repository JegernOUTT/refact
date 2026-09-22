use super::*;
use crate::buddy::outcome_runtime::OutcomeRuntime;
use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};

fn service(root: &Path) -> BuddyService {
    BuddyService::new(
        root.to_path_buf(),
        super::super::state::default_buddy_state(),
        BuddySettings::default(),
        vec![],
        RuntimeQueue::new(),
        broadcast::channel(64).0,
        None,
    )
}

fn runtime(root: &Path) -> OutcomeRuntime {
    OutcomeRuntime::open(
        root,
        ProjectIdentity {
            project_id: "actor-fixture".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        },
        WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        },
    )
    .unwrap()
}

struct ReleaseOnDrop(std::sync::mpsc::Sender<()>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn speech() -> BuddySpeechItem {
    serde_json::from_value(serde_json::json!({
        "id": "speech", "text": "Hello", "created_at": Utc::now().to_rfc3339()
    }))
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn reopen_succeeds_once_the_lock_is_released() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let holder = super::super::operator_runtime::open_project_runtime(&root).unwrap();
    let mut service = service(&root);
    service.enable_operator_from_settings();
    assert!(service.outcome_runtime.is_none());
    assert!(matches!(
        service
            .operator_open_failure
            .as_ref()
            .map(|failure| &failure.reason),
        Some(OperatorOpenFailureReason::AlreadyOwned(Some(_)))
    ));
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(service)));

    tokio::time::advance(OPERATOR_REOPEN_DELAYS[0]).await;
    for _ in 0..10_000 {
        tick_operator_reopen(&buddy).await;
        if buddy
            .lock()
            .await
            .as_ref()
            .unwrap()
            .operator_open_failure
            .as_ref()
            .is_some_and(|failure| failure.attempts == 2)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        buddy
            .lock()
            .await
            .as_ref()
            .unwrap()
            .operator_open_failure
            .as_ref()
            .unwrap()
            .attempts,
        2
    );

    holder.shutdown().await;
    tokio::time::advance(OPERATOR_REOPEN_DELAYS[1] - Duration::from_secs(1)).await;
    tick_operator_reopen(&buddy).await;
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .operator_reopen_task
        .is_none());

    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..10_000 {
        tick_operator_reopen(&buddy).await;
        if buddy
            .lock()
            .await
            .as_ref()
            .unwrap()
            .outcome_runtime
            .is_some()
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let reopened = buddy.lock().await;
    let service = reopened.as_ref().unwrap();
    assert!(service.outcome_runtime.is_some());
    assert!(service.operator_open_failure.is_none());
}

#[tokio::test(start_paused = true)]
async fn manual_reopen_succeeds_immediately_once_the_lock_is_released() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let holder = super::super::operator_runtime::open_project_runtime(&root).unwrap();
    let mut service = service(&root);
    service.enable_operator_from_settings();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(service)));

    holder.shutdown().await;

    assert!(reopen_operator_now(&buddy).await);
    let service = buddy.lock().await;
    assert!(service.as_ref().unwrap().outcome_runtime.is_some());
    assert!(service.as_ref().unwrap().operator_open_failure.is_none());
}

#[tokio::test(start_paused = true)]
async fn manual_reopen_is_rate_limited_without_resetting_the_backoff() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let holder = super::super::operator_runtime::open_project_runtime(&root).unwrap();
    let mut service = service(&root);
    service.enable_operator_from_settings();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(service)));
    let initial = {
        let guard = buddy.lock().await;
        let failure = guard
            .as_ref()
            .unwrap()
            .operator_open_failure
            .as_ref()
            .unwrap();
        (failure.attempts, failure.retry_stage, failure.next_retry_at)
    };

    assert!(!reopen_operator_now(&buddy).await);
    assert!(!reopen_operator_now(&buddy).await);
    let guard = buddy.lock().await;
    let failure = guard
        .as_ref()
        .unwrap()
        .operator_open_failure
        .as_ref()
        .unwrap();
    assert_eq!(failure.attempts, initial.0);
    assert_eq!(failure.retry_stage, initial.1);
    assert_eq!(failure.next_retry_at, initial.2);
    assert!(matches!(
        failure.reason,
        OperatorOpenFailureReason::AlreadyOwned(Some(_))
    ));
    drop(guard);
    holder.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn reopen_backoff_does_not_busy_loop() {
    let now = tokio::time::Instant::now();
    let mut failure = OperatorOpenFailure::new(
        OperatorOpenFailureReason::Other {
            kind: OperatorOpenFailureKind::IoError,
            detail: "contended".into(),
        },
        now,
    );
    assert!(!failure.is_due(now));
    for (attempt, delay) in OPERATOR_REOPEN_DELAYS.iter().enumerate() {
        tokio::time::advance(*delay - Duration::from_secs(1)).await;
        assert!(!failure.is_due(tokio::time::Instant::now()));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(failure.is_due(tokio::time::Instant::now()));
        failure.record_failure(
            OperatorOpenFailureReason::Other {
                kind: OperatorOpenFailureKind::IoError,
                detail: format!("failure-{attempt}"),
            },
            tokio::time::Instant::now(),
        );
    }
    assert_eq!(failure.retry_stage, OPERATOR_REOPEN_DELAYS.len() - 1);
    assert_eq!(failure.attempts, OPERATOR_REOPEN_DELAYS.len() as u64 + 1);
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(15 * 60 - 1)).await;
        assert!(!failure.is_due(tokio::time::Instant::now()));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(failure.is_due(tokio::time::Instant::now()));
        failure.record_failure(
            OperatorOpenFailureReason::Other {
                kind: OperatorOpenFailureKind::IoError,
                detail: "still contended".into(),
            },
            tokio::time::Instant::now(),
        );
    }
    assert_eq!(failure.retry_stage, OPERATOR_REOPEN_DELAYS.len() - 1);
    assert_eq!(failure.attempts, OPERATOR_REOPEN_DELAYS.len() as u64 + 4);
}

#[test]
fn rollout_stays_closed_and_editorial_preserves_explicit_user_speech() {
    let root = tempfile::tempdir().unwrap();
    let mut svc = service(root.path());
    assert!(svc.outcome_runtime.is_none());
    assert!(!svc.operator_features.storage);
    assert!(!svc.operator_features.editorial);
    svc.settings.enabled = true;
    svc.operator_features.editorial = true;
    assert!(!svc.update_speech(speech()));
    assert!(svc.active_speech.is_none());
    assert!(svc.update_speech_user_initiated(speech()));
    let mut event = make_runtime_event(
        "warning",
        "Native warning",
        "native",
        "warning",
        "blocked",
        None,
    );
    event.speech_text = Some("Cosmetic bypass".into());
    let stored = svc.enqueue_runtime_event_with_stored(event).unwrap();
    assert!(stored.speech_text.is_none());
    assert_eq!(stored.title, "Native warning");
    assert_eq!(stored.status, "blocked");
}

#[tokio::test]
async fn drafts_survive_failed_storage_then_expire_only_after_publication() {
    let root = tempfile::tempdir().unwrap();
    let mut svc = service(root.path());
    let draft = svc
        .create_draft(
            super::super::types::DraftKind::Command,
            "Proposal".into(),
            "name: proposal".into(),
            "Review this change".into(),
        )
        .unwrap();
    svc.operator_features.storage = true;
    let mut events = svc.events_tx.subscribe();
    assert!(events.try_recv().is_err());
    let future = Utc::now() + chrono::Duration::hours(3);
    assert!(svc.expire_drafts(future).is_empty());
    assert!(svc.delete_draft(&draft.id).is_none());
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    assert!(publish_operator_inputs(&buddy).await.is_err());
    let store = runtime(root.path());
    buddy.lock().await.as_mut().unwrap().outcome_runtime = Some(store.clone());
    publish_operator_inputs(&buddy).await.unwrap();
    assert!(matches!(
        events.try_recv().unwrap(),
        BuddyEvent::DraftCreated { .. }
    ));
    let mut guard = buddy.lock().await;
    let svc = guard.as_mut().unwrap();
    assert_eq!(svc.expire_drafts(future), vec![draft.id]);
    assert_eq!(svc.snapshot().opportunities.len(), 1);
    drop(guard);
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    publish_operator_inputs(&buddy).await.unwrap();
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    store.shutdown().await;
}

#[tokio::test]
async fn work_projection_not_legacy_queue_owns_snapshot_and_lifecycle() {
    let root = tempfile::tempdir().unwrap();
    let mut svc = service(root.path());
    let suggestion = BuddySuggestion {
        id: "legacy".into(),
        suggestion_type: "setup".into(),
        title: "Set up".into(),
        description: "Review configuration".into(),
        created_at: Utc::now().to_rfc3339(),
        dismissed: false,
        controls: vec![],
        quest: None,
    };
    svc.add_suggestion(suggestion.clone());
    let mut opportunity = operator_suggestion(&suggestion);
    opportunity.expires_at = Utc::now() - chrono::Duration::hours(1);
    svc.add_opportunity(opportunity.clone());
    svc.operator_features.storage = true;
    let store = runtime(root.path());
    svc.outcome_runtime = Some(store.clone());
    assert!(svc.snapshot().opportunities.is_empty());
    assert!(!svc.resolve_opportunity(&opportunity.id, OpportunityStatus::Completed));
    svc.dismiss_suggestion("legacy");
    svc.expire_suggestions();
    svc.expire_opportunities();
    assert!(!svc.state.suggestion_state[0].dismissed);
    assert_eq!(
        svc.opportunity_queue.snapshot()[0].status,
        OpportunityStatus::New
    );
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    publish_operator_inputs(&buddy).await.unwrap();
    let snapshot = buddy.lock().await.as_ref().unwrap().snapshot();
    assert_eq!(snapshot.opportunities.len(), 1);
    assert_ne!(snapshot.opportunities[0].id, opportunity.id);
    assert!(snapshot.opportunities[0].proposed_actions.is_empty());
    assert!(snapshot.state.suggestion_state.is_empty());
    store.shutdown().await;
}

#[tokio::test]
async fn durable_admission_rechecks_material_and_preserves_read_snooze() {
    let root = tempfile::tempdir().unwrap();
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    let suggestion = BuddySuggestion {
        id: "stable-source".into(),
        suggestion_type: "setup".into(),
        title: "Set up".into(),
        description: "Original assessment".into(),
        created_at: Utc::now().to_rfc3339(),
        dismissed: false,
        controls: vec![],
        quest: None,
    };
    let mut input = operator_suggestion(&suggestion);
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    assert!(
        BuddyService::surface_opportunity_durable(&buddy, input.clone(), 0)
            .await
            .is_err()
    );
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .state
        .opportunities
        .is_empty());
    let store = runtime(root.path());
    buddy.lock().await.as_mut().unwrap().outcome_runtime = Some(store.clone());
    assert!(
        BuddyService::surface_opportunity_durable(&buddy, input.clone(), 0)
            .await
            .unwrap()
    );
    let first = store
        .list_work_items(None, 10)
        .await
        .unwrap()
        .entries
        .remove(0);
    let id = first.value.id.clone();
    let artifact = first.value.current_artifact_id.clone().unwrap();
    let read = store
        .mark_work_read(id.clone(), artifact.clone(), 1)
        .await
        .unwrap();
    let snooze = store
        .snooze_work(id.clone(), artifact.clone(), 1, Some(60))
        .await
        .unwrap();
    input.id = "regenerated-card-id".into();
    input.created_at = Utc::now();
    assert!(
        BuddyService::surface_opportunity_durable(&buddy, input.clone(), 0)
            .await
            .unwrap()
    );
    assert_eq!(
        store.get_work_item(id.clone()).await.unwrap().unwrap(),
        first
    );
    assert_eq!(
        store.get_read_state(id.clone()).await.unwrap(),
        Some(read.clone())
    );
    assert_eq!(store.get_work_snooze(id.clone()).await.unwrap(), snooze);
    input.summary = "Materially changed assessment".into();
    assert!(BuddyService::surface_opportunity_durable(&buddy, input, 0)
        .await
        .unwrap());
    let changed = store.get_work_item(id.clone()).await.unwrap().unwrap();
    assert_eq!(changed.value.current_artifact_revision, Some(2));
    assert_eq!(changed.value.state, first.value.state);
    assert_eq!(store.get_read_state(id.clone()).await.unwrap(), Some(read));
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    assert_eq!(
        store
            .list_work_outcomes(id, None, 10)
            .await
            .unwrap()
            .entries
            .len(),
        2
    );
    store.shutdown().await;
}

#[tokio::test]
async fn durable_draft_has_no_success_or_editor_before_commit() {
    let root = tempfile::tempdir().unwrap();
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    let mut events = svc.events_tx.subscribe();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    assert!(BuddyService::create_draft_durable(
        &buddy,
        super::super::types::DraftKind::Command,
        "Proposal".into(),
        "name: proposal".into(),
        "Review".into()
    )
    .await
    .is_err());
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .draft_store
        .snapshot()
        .is_empty());
    assert!(events.try_recv().is_err());
    let store = runtime(root.path());
    buddy.lock().await.as_mut().unwrap().outcome_runtime = Some(store.clone());
    assert!(BuddyService::create_draft_durable(
        &buddy,
        super::super::types::DraftKind::Command,
        "Oversized".into(),
        "x".repeat(32769),
        String::new()
    )
    .await
    .is_err());
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .draft_store
        .snapshot()
        .is_empty());
    let draft = BuddyService::create_draft_durable(
        &buddy,
        super::super::types::DraftKind::Command,
        "Proposal".into(),
        "name: proposal".into(),
        "Review".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    assert!(matches!(
        events.try_recv().unwrap(),
        BuddyEvent::DraftCreated { .. }
    ));
    buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .expire_drafts(Utc::now() + chrono::Duration::hours(3));
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .draft_store
        .get(&draft.id)
        .is_none());
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    store.shutdown().await;
}

#[tokio::test]
async fn blocked_admission_releases_actor_and_stale_commit_never_publishes() {
    let root = tempfile::tempdir().unwrap();
    let store = runtime(root.path());
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    svc.outcome_runtime = Some(store.clone());
    let mut events = svc.events_tx.subscribe();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = ReleaseOnDrop(release_tx);
    let blocker_runtime = store.clone();
    let blocker = tokio::spawn(async move {
        blocker_runtime
            .with_store(move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
            .unwrap();
    });
    entered_rx.await.unwrap();
    let admission = buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .prepare_draft(
            super::super::types::DraftKind::Command,
            "Blocked proposal".into(),
            "name: blocked".into(),
            "Review".into(),
        )
        .unwrap();
    let commit = admission.commit(&buddy);
    tokio::pin!(commit);
    assert!(futures::poll!(&mut commit).is_pending());
    // This is a deterministic lock assertion, not a timing/sleep race.
    let mut guard = buddy
        .try_lock()
        .expect("DB admission cannot own actor mutex");
    let svc = guard.as_mut().unwrap();
    assert!(svc.draft_store.snapshot().is_empty());
    assert!(events.try_recv().is_err());
    svc.mute_rule_for_cooldown_key("native-decision");
    assert!(svc
        .prepare_draft(
            super::super::types::DraftKind::Command,
            "Backpressure".into(),
            "name: busy".into(),
            "Review".into()
        )
        .is_err());
    drop(release);
    blocker.await.unwrap();
    // Keep actor locked until durable commit is observable, then replace owner.
    // Polling the repository is bounded and never reads the whole history.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if !store
                .list_work_items(None, 1)
                .await
                .unwrap()
                .entries
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("admission commits while actor is locked by native decision");
    let mut replacement = service(root.path());
    replacement.operator_features.storage = true;
    replacement.outcome_runtime = Some(store.clone());
    let stopped = guard
        .take()
        .expect("native stop can take actor while DB admission is pending");
    assert!(stopped.draft_store.snapshot().is_empty());
    *guard = Some(replacement);
    drop(guard);
    assert!(commit.await.unwrap_err().contains("replaced"));
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .draft_store
        .snapshot()
        .is_empty());
    assert!(events.try_recv().is_err());
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    store.shutdown().await;
}

#[tokio::test]
async fn cancelled_admission_and_failed_memory_commit_preserve_input() {
    let root = tempfile::tempdir().unwrap();
    let store = runtime(root.path());
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    svc.outcome_runtime = Some(store.clone());
    let previous = svc.memory_ops.clone();
    let mut replacement = previous.clone();
    replacement.total_records = 42;
    let admission = svc.prepare_memory_ops(replacement.clone()).unwrap();
    assert_eq!(svc.memory_ops, previous);
    drop(admission);
    let admission = svc.prepare_memory_ops(replacement).unwrap();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    store.shutdown().await;
    assert!(admission.commit(&buddy).await.is_err());
    assert_eq!(buddy.lock().await.as_ref().unwrap().memory_ops, previous);
    assert!(buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .prepare_memory_ops(previous)
        .is_ok());
}

#[tokio::test]
async fn maintenance_driver_stops_while_repository_is_blocked() {
    let root = tempfile::tempdir().unwrap();
    let store = runtime(root.path());
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    svc.outcome_runtime = Some(store.clone());
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = ReleaseOnDrop(release_tx);
    let runtime = store.clone();
    let maintenance = async move {
        runtime
            .with_store(move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
            .unwrap();
    };
    let driver = drive_maintenance(&buddy, shutdown.clone(), maintenance);
    tokio::pin!(driver);
    tokio::select! {
        _ = &mut driver => panic!("maintenance exited before stop"),
        _ = entered_rx => {},
    }
    assert!(buddy.try_lock().is_ok());
    shutdown.store(true, Ordering::SeqCst);
    // The production driver must stop before the repository blocker is released.
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(3), &mut driver).await;
    drop(release);
    assert!(
        stopped.is_ok(),
        "native stop must not wait for database maintenance"
    );
    store.shutdown().await;
}

#[tokio::test]
async fn production_maintenance_driver_stops_while_repository_is_blocked() {
    let root = tempfile::tempdir().unwrap();
    let store = runtime(root.path());
    let mut svc = service(root.path());
    svc.operator_features.storage = true;
    svc.outcome_runtime = Some(store.clone());
    let mut events = svc.events_tx.subscribe();
    let admission = svc
        .prepare_draft(
            super::super::types::DraftKind::Command,
            "Pending".into(),
            "name: pending".into(),
            "Review".into(),
        )
        .unwrap();
    let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = ReleaseOnDrop(release_tx);
    let blocker_runtime = store.clone();
    let blocker = tokio::spawn(async move {
        blocker_runtime
            .with_store(move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
            .unwrap();
    });
    entered_rx.await.unwrap();
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let acknowledged = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let driver = drive_maintenance(&buddy, shutdown.clone(), async {
            admission.commit(&buddy).await.unwrap();
            acknowledged.store(true, Ordering::SeqCst);
        });
        tokio::pin!(driver);
        assert!(futures::poll!(&mut driver).is_pending());
        let native_event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::select! {
                _ = &mut driver => panic!("maintenance exited before stop"),
                event = events.recv() => event.unwrap(),
            }
        })
        .await
        .expect("native tick progresses while repository is blocked");
        let BuddyEvent::StateUpdated { state } = native_event else {
            panic!("expected native pet tick, got {native_event:?}");
        };
        assert_eq!(state.pet.evolution.open_seconds, PET_DECAY_INTERVAL_SECS);
        let guard = buddy.try_lock().expect("native tick releases actor mutex");
        let svc = guard.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::to_value(svc.presentation_state()).unwrap()
        );
        assert!(state.opportunities.is_empty());
        assert!(svc.draft_store.snapshot().is_empty());
        assert!(events.try_recv().is_err());
        drop(guard);
        shutdown.store(true, Ordering::SeqCst);
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut driver)
            .await
            .expect("production loop exits without DB progress");
    }
    assert!(!acknowledged.load(Ordering::SeqCst));
    assert!(events.try_recv().is_err());
    // Cancellation retains the sole admission permit in the repository closure.
    assert!(buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .prepare_memory_ops(MemoryOpsState::default())
        .is_err());
    drop(release);
    blocker.await.unwrap();
    let admission_gate = buddy.lock().await.as_ref().unwrap().admission_gate.clone();
    drop(
        tokio::time::timeout(std::time::Duration::from_secs(2), admission_gate.lock())
            .await
            .expect("cancelled repository admission releases capacity after commit"),
    );
    assert_eq!(
        store.list_work_items(None, 10).await.unwrap().entries.len(),
        1
    );
    store.shutdown().await;
    assert!(buddy
        .lock()
        .await
        .as_ref()
        .unwrap()
        .draft_store
        .snapshot()
        .is_empty());
    assert!(events.try_recv().is_err());
}
