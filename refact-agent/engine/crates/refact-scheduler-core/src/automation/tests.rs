use super::*;
use serde_json::json;

#[test]
fn finite_delivery_failures_survive_reopen_without_ack_or_wake_or_recheckpoint() {
    let root = crate::test_support::tempdir();
    let j = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let request = source(&j);
    j.ingest(&request).unwrap();
    let d = j.pending_deliveries(10, 100).unwrap().pop().unwrap();
    for attempt in 1..=8 {
        assert!(j
            .retry_delivery(d.event_seq, &d.target, 100 + attempt, "Bearer raw-secret")
            .unwrap());
        assert_eq!(
            j.pending_deliveries(10, 200).unwrap().len(),
            usize::from(attempt < 8)
        );
    }
    assert!(!j
        .retry_delivery(d.event_seq, &d.target, 300, "again")
        .unwrap());
    assert!(!j.acknowledge_delivery(d.event_seq, &d.target).unwrap());
    assert_eq!(j.ingest(&request).unwrap(), IngestOutcome::Duplicate);
    let db = rusqlite::Connection::open(journal_path(root.path()).unwrap()).unwrap();
    let state: (u32, bool, bool, String) = db
        .query_row("SELECT attempts,acked,failed,error FROM outbox", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!((state.0, state.1, state.2), (8, false, true));
    assert!(!state.3.contains("raw-secret"));
    drop(j);
    let j = AutomationJournal::open(root.path()).unwrap();
    let summary = j.wake_summary_at(200).unwrap();
    assert_eq!(summary.pending_delivery, 0);
    assert_eq!(summary.failed_delivery, 1);
    assert_eq!(summary.next_wake_ms, None);
    assert!(j
        .pending_deliveries_for_targets(&[d.target.clone()], 10, 200)
        .unwrap()
        .is_empty());
    assert_eq!(j.checkpoint("app", "source").unwrap(), request.cursor);
    assert!(j.failed_deliveries(10).unwrap()[0].payload_retained);
    j.activate_owner(&lease).unwrap();
    assert!(!j
        .retry_failed_delivery(d.event_seq, "wrong-target", 400)
        .unwrap());
    assert!(j
        .retry_failed_delivery(d.event_seq, &d.target, 400)
        .unwrap());
    assert!(j.pending_deliveries(10, 399).unwrap().is_empty());
    assert_eq!(j.pending_deliveries(10, 400).unwrap()[0].attempts, 0);
    // Retry attempts are not occurrence ordinals: a distinct upstream identity
    // inserts exactly one new event, while a duplicate inserts none.
    let mut next = request.clone();
    next.event_id = "event-2".into();
    let outcome = j.ingest(&next).unwrap();
    let IngestOutcome::Inserted {
        event_seq,
    } = outcome
    else {
        panic!("new event deduplicated");
    };
    assert!(event_seq > d.event_seq);
    assert_eq!(j.ingest(&next).unwrap(), IngestOutcome::Duplicate);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn deadletter_retention_preserves_metadata_and_pending_payload_with_bounded_admission() {
    let root = crate::test_support::tempdir();
    let j = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut request = source(&j);
    j.ingest(&request).unwrap();
    let pending = j.pending_deliveries(1, 100).unwrap()[0].clone();
    let mut db = rusqlite::Connection::open(journal_path(root.path()).unwrap()).unwrap();
    let tx = db.transaction().unwrap();
    // Seed the capacity boundary once; exercise production pruning below.
    tx.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4100) INSERT INTO events(app,source,digest,producer,event_id,data,at_ms) SELECT 'cron','generic','digest','run','seed-'||x,'{\"terminal\":true}',0 FROM n;
        INSERT INTO outbox(event_seq,target,data,next_ms,attempts,failed,failed_definition) SELECT seq,'dead',data,0,8,1,'generic' FROM events WHERE app='cron';").unwrap();
    tx.commit().unwrap();
    assert!(!j.acknowledge_delivery(-1, "missing").unwrap());
    assert_eq!(
        j.failed_delivery_count_for_definition("generic").unwrap(),
        4100
    );
    assert_eq!(j.wake_summary_at(100).unwrap().pending_delivery, 1);
    assert_eq!(j.pending_deliveries(1, 100).unwrap()[0].data, pending.data);
    let retired: i64 = db
        .query_row(
            "SELECT event_seq FROM outbox WHERE failed=1 AND data='null' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!j.retry_failed_delivery(retired, "dead", 100).unwrap());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events WHERE app='cron'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4096
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM outbox WHERE failed=1 AND acked=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<12283) INSERT INTO outbox(event_seq,target,data,next_ms,attempts,failed) SELECT -x,'dead','null',0,8,1 FROM n;").unwrap();
    request.event_id = "blocked-at-control-cap".into();
    request.cursor = Some("must-not-commit".into());
    assert!(j
        .ingest(&request)
        .unwrap_err()
        .contains("control record capacity"));
    assert_eq!(
        j.checkpoint("app", "source").unwrap().as_deref(),
        Some("cursor-1")
    );
}

#[test]
fn explicit_agent_enqueue_rejection_releases_started_claim_but_not_command_or_unknown() {
    let root = crate::test_support::tempdir();
    let j = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let agent = job("agent");
    j.put_legacy_job(&agent).unwrap();
    let digest = legacy_job_digest(&agent).unwrap();
    let claim = j
        .claim_run("agent", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    assert!(j.mark_effect_started(&claim).unwrap());
    assert!(j.release_rejected_agent_enqueue(&claim).unwrap());
    let claim = j
        .claim_run("agent", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    assert!(j.mark_effect_started(&claim).unwrap());
    assert!(j
        .finish_run(&claim, RunStatus::Unknown, None, None)
        .unwrap());
    assert!(!j.release_rejected_agent_enqueue(&claim).unwrap());
    let mut command = job("command");
    command.action = crate::types::Action::Command {
        argv: vec!["echo".into()],
        target: crate::types::AgentTarget::Isolated,
        cwd: None,
        env: None,
        timeout_secs: None,
    };
    j.put_legacy_job(&command).unwrap();
    let claim = j
        .claim_run(
            "command",
            "one",
            &legacy_job_digest(&command).unwrap(),
            lease.epoch(),
            false,
        )
        .unwrap()
        .unwrap();
    assert!(j.mark_effect_started(&claim).unwrap());
    assert!(!j.release_rejected_agent_enqueue(&claim).unwrap());
}

#[test]
fn succeeded_occurrence_lookup_is_owner_and_current_authority_fenced() {
    let root = crate::test_support::tempdir();
    let j = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut agent = job("accepted");
    j.put_legacy_job(&agent).unwrap();
    let digest = legacy_job_digest(&agent).unwrap();
    let claim = j
        .claim_run("accepted", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    assert!(!j
        .is_run_succeeded("accepted", "one", &digest, lease.epoch())
        .unwrap());
    assert!(j
        .finish_run(&claim, RunStatus::Succeeded, None, None)
        .unwrap());
    assert!(j
        .is_run_succeeded("accepted", "one", &digest, lease.epoch())
        .unwrap());
    assert!(j
        .is_run_succeeded("accepted", "one", &digest, "stale")
        .is_err());
    let held = j
        .claim_run("accepted", "two", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    j.finish_run(&held, RunStatus::Unknown, None, None).unwrap();
    assert!(!j
        .is_run_succeeded("accepted", "one", &digest, lease.epoch())
        .unwrap());
    agent.enabled = false;
    j.put_legacy_job(&agent).unwrap();
    assert!(!j
        .is_run_succeeded("accepted", "one", &digest, lease.epoch())
        .unwrap());
}

#[test]
fn legacy_outbox_schema_migrates_exhausted_attempts_to_honest_failure() {
    let root = crate::test_support::tempdir();
    let j = AutomationJournal::open(root.path()).unwrap();
    drop(j);
    let db = rusqlite::Connection::open(journal_path(root.path()).unwrap()).unwrap();
    db.execute_batch("DROP TABLE outbox; CREATE TABLE outbox(event_seq INTEGER NOT NULL,target TEXT NOT NULL,data TEXT NOT NULL,next_ms INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,acked INTEGER NOT NULL DEFAULT 0,error TEXT,PRIMARY KEY(event_seq,target)); INSERT INTO outbox VALUES(1,'old','{}',0,99,0,'old');").unwrap();
    let j = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    assert_eq!(j.wake_summary_at(100).unwrap().failed_delivery, 1);
    assert_eq!(j.wake_summary_at(100).unwrap().pending_delivery, 0);
    assert_eq!(
        db.query_row("SELECT acked FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn job(id: &str) -> crate::types::Job {
    let mut j = crate::types::Job::new_cron_agent_chat(
        "* * * * *".into(),
        "test".into(),
        "test".into(),
        true,
        true,
        1,
    );
    j.id = id.into();
    j
}
fn source(j: &AutomationJournal) -> IngestRequest {
    j.reconcile_app_source("app", "source", "digest", &json!({"driver":"manual"}))
        .unwrap();
    j.grant_source(&GrantRecord {
        app_id: "app".into(),
        source_id: "source".into(),
        config_digest: "digest".into(),
        endpoint_grant: None,
        credential_refs: vec![],
        granted: true,
    })
    .unwrap();
    j.set_source_control(
        "app",
        "source",
        &SourceControl {
            demand: true,
            ..Default::default()
        },
    )
    .unwrap();
    IngestRequest {
        app_id: "app".into(),
        source_id: "source".into(),
        source_generation: j
            .source_state("app", "source")
            .unwrap()
            .unwrap()
            .source_generation,
        producer_id: "endpoint".into(),
        producer_epoch: "upstream-1".into(),
        event_id: "event-1".into(),
        cursor: Some("cursor-1".into()),
        source_config_digest: "digest".into(),
        data: json!({"value":42}),
        deliveries: vec!["view".into()],
        at_ms: 100,
    }
}
#[test]
fn sql_ingest_commits_event_latest_checkpoint_and_outbox_once() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let r = source(&j);
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
    assert_eq!(j.ingest(&r).unwrap(), IngestOutcome::Duplicate);
    assert_eq!(j.latest("app", "source").unwrap(), Some(r.data));
    assert_eq!(
        j.checkpoint("app", "source").unwrap().as_deref(),
        Some("cursor-1")
    );
    assert_eq!(j.pending_deliveries(10, 100).unwrap().len(), 1);
}
#[test]
fn large_ingest_and_delivery_preserve_exact_payload_dedup_and_denial() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut r = source(&j);
    r.data = json!({"history":"exact-data".repeat(40000)});
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
    assert_eq!(j.ingest(&r).unwrap(), IngestOutcome::Duplicate);
    let deliveries = j.pending_deliveries(10, 100).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].data, r.data);
    assert_eq!(j.latest("app", "source").unwrap().as_ref(), Some(&r.data));
    drop(j);
    let j = AutomationJournal::open(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    assert_eq!(j.pending_deliveries(10, 100).unwrap()[0].data, r.data);
    assert_eq!(j.checkpoint("app", "source").unwrap(), r.cursor);
    j.grant_source(&GrantRecord {
        app_id: "app".into(),
        source_id: "source".into(),
        config_digest: "digest".into(),
        endpoint_grant: None,
        credential_refs: vec![],
        granted: false,
    })
    .unwrap();
    r.event_id = "denied".into();
    r.cursor = Some("denied-cursor".into());
    assert!(j.ingest(&r).is_err());
    assert_eq!(
        j.checkpoint("app", "source").unwrap().as_deref(),
        Some("cursor-1")
    );
}

#[test]
fn malformed_delivery_rolls_back_every_ingest_write() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut r = source(&j);
    r.deliveries.push("../../escape".into());
    assert!(j.ingest(&r).is_err());
    assert_eq!(j.latest("app", "source").unwrap(), None);
    assert_eq!(j.checkpoint("app", "source").unwrap(), None);
    assert!(j.pending_deliveries(10, 100).unwrap().is_empty());
    r.deliveries.pop();
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
}
#[test]
fn upstream_identity_not_local_generation_controls_deduplication() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut r = source(&j);
    j.ingest(&r).unwrap();
    j.set_source_control(
        "app",
        "source",
        &SourceControl {
            demand: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(j.ingest(&r).is_err());
    r.source_generation = j
        .source_state("app", "source")
        .unwrap()
        .unwrap()
        .source_generation;
    assert_eq!(j.ingest(&r).unwrap(), IngestOutcome::Duplicate);
    r.producer_epoch = "upstream-2".into();
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
}
#[test]
fn unkeyed_events_are_best_effort_not_accidentally_deduplicated() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut r = source(&j);
    r.event_id.clear();
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
    assert!(matches!(
        j.ingest(&r).unwrap(),
        IngestOutcome::Inserted { .. }
    ));
}
#[test]
fn semantic_reconcile_preserves_grant_but_changed_spec_revokes() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let r = source(&j);
    let epoch = r.source_generation.clone();
    assert_eq!(
        j.reconcile_app_source("app", "source", "digest", &json!({}))
            .unwrap(),
        ReconcileStatus::Unchanged
    );
    assert_eq!(
        j.source_state("app", "source")
            .unwrap()
            .unwrap()
            .source_generation,
        epoch
    );
    assert_eq!(
        j.reconcile_app_source("app", "source", "new-digest", &json!({}))
            .unwrap(),
        ReconcileStatus::NeedsApproval
    );
    assert!(!j.source_state("app", "source").unwrap().unwrap().approved);
    assert!(j.ingest(&r).is_err());
}
#[test]
fn delivery_ack_and_retry_do_not_reexecute_or_recheckpoint() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let r = source(&j);
    j.ingest(&r).unwrap();
    let d = j.pending_deliveries(1, 100).unwrap().pop().unwrap();
    assert!(j
        .retry_delivery(d.event_seq, &d.target, 200, "offline")
        .unwrap());
    assert!(j.pending_deliveries(1, 199).unwrap().is_empty());
    assert_eq!(j.pending_deliveries(1, 200).unwrap()[0].attempts, 1);
    assert!(j.acknowledge_delivery(d.event_seq, &d.target).unwrap());
    assert!(!j.acknowledge_delivery(d.event_seq, &d.target).unwrap());
    assert_eq!(j.checkpoint("app", "source").unwrap(), r.cursor);
}
#[test]
fn claims_exclude_duplicate_and_started_recovery_blocks_new_occurrences() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("cron");
    j.put_legacy_job(&job).unwrap();
    let digest = legacy_job_digest(&job).unwrap();
    let c = j
        .claim_run("cron", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    assert!(j
        .claim_run("cron", "one", &digest, lease.epoch(), false)
        .unwrap()
        .is_none());
    assert!(j.mark_effect_started(&c).unwrap());
    drop(j);
    drop(lease);
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    let j = AutomationJournal::open(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    assert_eq!(j.recover_inflight(lease.epoch()).unwrap(), 1);
    assert!(j.finish_run(&c, RunStatus::Succeeded, None, None).is_err());
    assert!(j
        .claim_run("cron", "new-key", &digest, lease.epoch(), false)
        .unwrap()
        .is_none());
    assert_eq!(j.wake_summary().unwrap().recovery, 1);
}
#[test]
fn never_started_recovery_releases_claim_without_replaying_side_effect() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("cron");
    j.put_legacy_job(&job).unwrap();
    let digest = legacy_job_digest(&job).unwrap();
    let c = j
        .claim_run("cron", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    drop(j);
    drop(lease);
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    let j = AutomationJournal::open(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    assert_eq!(j.recover_inflight(lease.epoch()).unwrap(), 0);
    assert!(j.mark_effect_started(&c).is_err());
    assert!(j
        .claim_run("cron", "one", &digest, lease.epoch(), false)
        .unwrap()
        .is_some());
}
#[test]
fn changed_job_revokes_old_effect_admission() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let mut job = job("cron");
    j.put_legacy_job(&job).unwrap();
    let digest = legacy_job_digest(&job).unwrap();
    let c = j
        .claim_run("cron", "one", &digest, lease.epoch(), false)
        .unwrap()
        .unwrap();
    job.enabled = false;
    j.put_legacy_job(&job).unwrap();
    assert!(!j.mark_effect_started(&c).unwrap());
    assert!(j
        .claim_run("cron", "two", &digest, lease.epoch(), false)
        .unwrap()
        .is_none());
}
#[test]
fn sessions_are_fenced_and_never_loaded_as_durable() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("session");
    j.put_session_job(&job, lease.epoch()).unwrap();
    assert!(j.load_legacy_jobs().unwrap().is_empty());
    let digest = legacy_job_digest(&job).unwrap();
    assert!(j
        .claim_run("session", "one", &digest, "stale-owner", false)
        .is_err());
    assert!(j
        .claim_run("session", "one", &digest, lease.epoch(), false)
        .unwrap()
        .is_some());
    j.recover_inflight(lease.epoch()).unwrap();
    assert!(j
        .claim_run("session", "two", &digest, "stale-owner", false)
        .is_err());
}
#[test]
fn lease_shared_in_process_but_independent_owner_denied() {
    #[cfg(unix)]
    {
        const MARKER: &str = "REFACT_LEASE_FORK_TEST_CHILD";
        if std::env::var_os(MARKER).is_none() {
            struct TestProcess(std::process::Child);
            impl Drop for TestProcess {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let mut process = TestProcess(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "automation::tests::lease_shared_in_process_but_independent_owner_denied",
                        "--test-threads=1",
                        "--nocapture",
                    ])
                    .env(MARKER, "1")
                    .spawn()
                    .unwrap(),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if let Some(status) = process.0.try_wait().unwrap() {
                    assert!(status.success(), "isolated lease test failed: {status}");
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "isolated lease test timeout"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    let t = crate::test_support::tempdir();
    let a = ProjectAutomationLease::acquire(t.path()).unwrap();
    let b = ProjectAutomationLease::acquire(t.path()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_err());
    assert!(std::sync::Arc::ptr_eq(
        &a.capacity("actions", 8),
        &b.capacity("actions", 8)
    ));
    drop(a);
    drop(b);
    assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_ok());

    #[cfg(unix)]
    {
        struct ForkChild {
            ready: [libc::c_int; 2],
            release: [libc::c_int; 2],
            pid: libc::pid_t,
            status: Option<libc::c_int>,
        }
        impl ForkChild {
            fn close(fd: &mut libc::c_int) {
                if *fd >= 0 {
                    unsafe { libc::close(*fd) };
                    *fd = -1;
                }
            }
            fn running(&mut self) -> bool {
                if self.status.is_some() {
                    return false;
                }
                loop {
                    let mut status = 0;
                    let rc = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
                    if rc == self.pid {
                        self.status = Some(status);
                        return false;
                    }
                    if rc == 0 {
                        return true;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        self.pid = -1;
                    }
                    assert_eq!(error.raw_os_error(), Some(libc::EINTR), "waitpid: {error}");
                }
            }
            fn finish(&mut self) {
                Self::close(&mut self.release[1]);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while self.running() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fork child exit timeout"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let status = self.status.unwrap();
                assert!(libc::WIFEXITED(status));
                assert_eq!(libc::WEXITSTATUS(status), 0);
            }
        }
        impl Drop for ForkChild {
            fn drop(&mut self) {
                for fd in self.ready.iter_mut().chain(self.release.iter_mut()) {
                    Self::close(fd);
                }
                if self.pid > 0 && self.status.is_none() {
                    unsafe { libc::kill(self.pid, libc::SIGKILL) };
                    loop {
                        let rc = unsafe { libc::waitpid(self.pid, std::ptr::null_mut(), 0) };
                        if rc >= 0
                            || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                        {
                            break;
                        }
                    }
                }
            }
        }
        for child_drops_lease in [false, true] {
            let mut lease = Some(ProjectAutomationLease::acquire_exclusive(t.path()).unwrap());
            assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_err());
            let mut child = ForkChild {
                ready: [-1; 2],
                release: [-1; 2],
                pid: -1,
                status: None,
            };
            assert_eq!(unsafe { libc::pipe(child.ready.as_mut_ptr()) }, 0);
            assert_eq!(unsafe { libc::pipe(child.release.as_mut_ptr()) }, 0);
            child.pid = unsafe { libc::fork() };
            assert!(child.pid >= 0);
            if child.pid == 0 {
                unsafe {
                    libc::close(child.ready[0]);
                    libc::close(child.release[1]);
                    if child_drops_lease {
                        drop(lease.take());
                    }
                    let byte = b'R';
                    loop {
                        let rc = libc::write(child.ready[1], (&byte as *const u8).cast(), 1);
                        if rc == 1 {
                            break;
                        }
                        if rc < 0
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                        {
                            continue;
                        }
                        libc::_exit(11);
                    }
                    let mut byte = 0u8;
                    loop {
                        let rc = libc::read(child.release[0], (&mut byte as *mut u8).cast(), 1);
                        if rc < 0
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                        {
                            continue;
                        }
                        libc::_exit(if rc == 0 { 0 } else { 12 });
                    }
                }
            }
            ForkChild::close(&mut child.ready[1]);
            ForkChild::close(&mut child.release[0]);
            let mut fd = libc::pollfd {
                fd: child.ready[0],
                events: libc::POLLIN,
                revents: 0,
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                assert!(!remaining.is_zero(), "fork child READY timeout");
                let rc = unsafe { libc::poll(&mut fd, 1, remaining.as_millis().max(1) as i32) };
                if rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                assert_eq!(rc, 1, "fork child READY unavailable");
                break;
            }
            assert_ne!(fd.revents & libc::POLLIN, 0);
            let mut byte = 0u8;
            loop {
                let rc = unsafe { libc::read(child.ready[0], (&mut byte as *mut u8).cast(), 1) };
                if rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                assert_eq!(rc, 1);
                break;
            }
            assert_eq!(byte, b'R');
            assert!(child.running());
            if child_drops_lease {
                assert!(
                    ProjectAutomationLease::acquire_exclusive(t.path()).is_err(),
                    "child dropping its inherited lease must not unlock the live parent owner"
                );
            } else {
                assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_err());
                drop(lease.take());
                let replacement = ProjectAutomationLease::acquire_exclusive(t.path())
                    .expect("parent drop must release ownership while the fork child retains its descriptor");
                assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_err());
                drop(replacement);
            }
            assert!(child.running());
            child.finish();
            drop(lease);
            assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_ok());
        }
    }
}
#[test]
fn observer_does_not_create_and_cannot_write() {
    let t = crate::test_support::tempdir();
    assert!(AutomationJournal::open_observer(t.path()).is_err());
    assert!(!t.path().join(".refact").exists());
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let o = AutomationJournal::open_observer(t.path()).unwrap();
    assert_eq!(o.wake_summary().unwrap().pending_delivery, 0);
    assert!(o.put_session_job(&job("no"), "epoch").is_err());
    assert!(j.load_legacy_jobs().unwrap().is_empty());
}
#[test]
fn paused_expired_unapproved_and_archived_sources_cannot_keep_daemon_awake() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    source(&j);
    let mut c = SourceControl {
        background: true,
        resume_after_restart: true,
        expires_at_ms: Some(200),
        next_wake_ms: Some(150),
        ..Default::default()
    };
    j.set_source_control("app", "source", &c).unwrap();
    assert!(j.wake_summary_at(100).unwrap().background_active);
    assert!(!j.wake_summary_at(200).unwrap().background_active);
    c.paused = true;
    j.set_source_control("app", "source", &c).unwrap();
    assert!(!j.wake_summary_at(100).unwrap().background_active);
    j.remove_source("app", "source").unwrap();
    assert!(j.list_sources().unwrap().is_empty());
    assert!(!j.wake_summary_at(100).unwrap().background_active);
}
#[test]
fn migration_retires_json_preserves_fields_and_places_retry_review_hold() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    let mut job = job("legacy");
    job.retry_attempts = 2;
    job.fire_count = 7;
    job.last_delivery_error = Some("offline".into());
    std::fs::write(
        t.path().join(".refact/scheduled_tasks.json"),
        serde_json::to_vec(&vec![&job]).unwrap(),
    )
    .unwrap();
    assert_eq!(j.migrate_legacy_jobs(t.path(), &lease).unwrap(), 1);
    assert!(!t.path().join(".refact/scheduled_tasks.json").exists());
    assert!(j.legacy_authority_retired().unwrap());
    assert_eq!(j.load_legacy_jobs().unwrap(), vec![job]);
    assert_eq!(j.wake_summary().unwrap().recovery, 1);
    assert_eq!(j.migrate_legacy_jobs(t.path(), &lease).unwrap(), 0);
}
#[test]
fn catalog_cas_and_external_deletion_are_authoritative() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("portable");
    j.put_legacy_job(&job).unwrap();
    let c = AutomationCatalog::open(t.path()).unwrap();
    let doc = c.read().unwrap();
    assert_eq!(doc.legacy_jobs, vec![job]);
    assert!(c.compare_and_swap_legacy_jobs(0, vec![]).is_err());
    c.compare_and_swap_legacy_jobs(doc.revision, vec![])
        .unwrap();
    assert!(j.load_legacy_jobs().unwrap().is_empty());
}
#[cfg(unix)]
#[test]
fn symlinked_storage_components_and_hardlinks_are_refused() {
    use std::os::unix::fs::symlink;
    let t = crate::test_support::tempdir();
    let outside = crate::test_support::tempdir();
    symlink(outside.path(), t.path().join(".refact")).unwrap();
    assert!(AutomationJournal::open(t.path()).is_err());
    // Private execution ownership does not traverse project-controlled .refact.
    assert!(ProjectAutomationLease::acquire(t.path()).is_ok());
    std::fs::remove_file(t.path().join(".refact")).unwrap();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    drop(j);
    std::fs::hard_link(
        journal_path(t.path()).unwrap(),
        outside.path().join("alias"),
    )
    .unwrap();
    assert!(AutomationJournal::open(t.path()).is_err());
}

#[test]
fn session_collision_cannot_overwrite_durable_or_other_session_authority() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let original = job("authority");
    j.put_legacy_job(&original).unwrap();
    let mut attacker = original.clone();
    attacker.enabled = false;
    assert!(j.put_session_job(&attacker, "other").is_err());
    assert_eq!(j.load_legacy_jobs().unwrap(), vec![original]);
    attacker.id = "session-only".into();
    j.put_session_job(&attacker, "first").unwrap();
    assert!(j.put_session_job(&attacker, "second").is_err());
}
#[test]
fn observer_migration_refuses_before_retiring_legacy_authority() {
    let t = crate::test_support::tempdir();
    let _j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    let old = t.path().join(".refact/scheduled_tasks.json");
    std::fs::write(&old, b"[]").unwrap();
    let observer = AutomationJournal::open_observer(t.path()).unwrap();
    assert!(observer.migrate_legacy_jobs(t.path(), &lease).is_err());
    assert!(old.exists());
    assert!(!t
        .path()
        .join(".refact/scheduled_tasks.migrated.json")
        .exists());
    assert!(observer.load_legacy_jobs().unwrap().is_empty());
}
#[test]
fn atomic_completion_rejects_invalid_payload_without_finishing_claim() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("atomic");
    j.put_legacy_job(&job).unwrap();
    let claim = j
        .claim_run(
            "atomic",
            "one",
            &legacy_job_digest(&job).unwrap(),
            lease.epoch(),
            false,
        )
        .unwrap()
        .unwrap();
    assert!(j.mark_effect_started(&claim).unwrap());
    let prepared = json!({"prepared":"large-delivery".repeat(30000)});
    assert!(j
        .complete_run_and_enqueue_delivery(
            &claim,
            RunStatus::Succeeded,
            None,
            None,
            Some((&prepared, "invalid/target"))
        )
        .is_err());
    assert_eq!(j.run_delivery_id(&claim).unwrap(), None);
    assert!(j
        .complete_run_and_enqueue_delivery(
            &claim,
            RunStatus::Succeeded,
            Some(&json!({"ok":true})),
            None,
            Some((&prepared, "cron-delivery"))
        )
        .unwrap());
    let seq = j.run_delivery_id(&claim).unwrap().unwrap();
    assert_eq!(
        j.pending_deliveries(1, u64::MAX / 2).unwrap()[0].data,
        prepared
    );
    assert_eq!(
        j.pending_deliveries(1, u64::MAX / 2).unwrap()[0].event_seq,
        seq
    );
    assert!(!j
        .complete_run_and_enqueue_delivery(&claim, RunStatus::Succeeded, None, None, None)
        .unwrap());
}
#[test]
fn activation_checks_root_identity_and_retains_the_execution_lease() {
    let t = crate::test_support::tempdir();
    let other = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let wrong = ProjectAutomationLease::acquire(other.path()).unwrap();
    assert!(j.activate_owner(&wrong).is_err());
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    drop(lease);
    assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_err());
    drop(j);
    assert!(ProjectAutomationLease::acquire_exclusive(t.path()).is_ok());
}
#[test]
fn cron_wake_summary_includes_interval_once_cron_and_explicit_manual() {
    let cases = [
        (
            "interval",
            crate::types::Trigger::Interval {
                every_ms: 1000,
            },
            Some(1001),
        ),
        (
            "once",
            crate::types::Trigger::Once {
                at_ms: 500,
            },
            Some(500),
        ),
        (
            "cron",
            crate::types::Trigger::Cron {
                expr: "* * * * *".into(),
                tz: Some("UTC".into()),
            },
            Some(60000),
        ),
        ("manual", crate::types::Trigger::Manual, None),
    ];
    for (id, trigger, expected) in cases {
        let t = crate::test_support::tempdir();
        let j = AutomationJournal::open(t.path()).unwrap();
        let mut job = job(id);
        job.trigger = trigger;
        j.put_legacy_job(&job).unwrap();
        assert_eq!(
            j.wake_summary_at(100).unwrap().next_wake_ms,
            expected,
            "{id}"
        );
        job.enabled = false;
        job.trigger_at_ms = Some(250);
        j.put_legacy_job(&job).unwrap();
        assert_eq!(j.wake_summary_at(100).unwrap().next_wake_ms, Some(250));
    }
    use chrono::TimeZone;
    let utc_ms = |month, hour, minute| {
        chrono::Utc
            .with_ymd_and_hms(2026, month, 1, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    };
    let default_tz = chrono_tz::Australia::Sydney;
    let mut mismatches = Vec::new();
    for (month, local_hour_utc) in [(1, 0), (7, 1)] {
        for recurring in [false, true] {
            for zone in [None, Some("UTC")] {
                let t = crate::test_support::tempdir();
                let j = AutomationJournal::open(t.path()).unwrap();
                let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
                j.activate_owner(&lease).unwrap();
                let mut imported = job("legacy-timezone");
                imported.created_at_ms = utc_ms(month, 0, 0);
                imported.recurring = recurring;
                imported.auto_expire_after_ms = 0;
                imported.trigger = crate::types::Trigger::Cron {
                    expr: "1 11 * * *".into(),
                    tz: zone.map(str::to_string),
                };
                std::fs::write(
                    t.path().join(".refact/scheduled_tasks.json"),
                    serde_json::to_vec(&vec![&imported]).unwrap(),
                )
                .unwrap();
                assert_eq!(j.migrate_legacy_jobs(t.path(), &lease).unwrap(), 1);
                assert!(j.legacy_authority_retired().unwrap());
                assert_eq!(j.load_legacy_jobs().unwrap(), vec![imported.clone()]);
                let now = imported.created_at_ms + 1_000;
                let utc_expected = utc_ms(month, 11, 1);
                let expected = if zone.is_some() {
                    utc_expected
                } else {
                    utc_ms(month, local_hour_utc, 1)
                };
                assert!(expected > now);
                assert_eq!(
                    crate::schedule::next_run_ms(&imported, imported.created_at_ms, default_tz,),
                    Some(expected)
                );
                if recurring {
                    let state = crate::schedule::recurring_missed_grace_state(
                        &imported,
                        imported.created_at_ms,
                        expected,
                        default_tz,
                        Default::default(),
                    )
                    .unwrap();
                    assert_eq!(state.due_ms, Some(expected));
                    assert!(state.should_fire);
                }
                let observer = AutomationJournal::open_observer(t.path()).unwrap();
                assert_eq!(
                    observer.wake_summary_at(now).unwrap().next_wake_ms,
                    Some(utc_expected),
                    "UTC compatibility wrapper"
                );
                for at in [now, expected] {
                    let actual = observer
                        .wake_summary_at_in_timezone(at, default_tz)
                        .unwrap()
                        .next_wake_ms;
                    let label =
                        format!("month={month} recurring={recurring} zone={zone:?} at={at}");
                    eprintln!("{label}: actual={actual:?} expected={expected}");
                    if actual != Some(expected) {
                        mismatches.push(label);
                    }
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "timezone wake mismatches: {mismatches:?}"
    );
}
#[test]
fn project_database_leaf_is_not_authoritative_and_runtime_is_private() {
    let t = crate::test_support::tempdir();
    std::fs::create_dir_all(t.path().join(".refact/automation")).unwrap();
    std::fs::write(
        t.path().join(".refact/automation/runtime.sqlite"),
        b"attacker-supplied database",
    )
    .unwrap();
    let j = AutomationJournal::open(t.path()).unwrap();
    assert_eq!(j.wake_summary().unwrap().pending_delivery, 0);
    let path = journal_path(t.path()).unwrap();
    assert!(!path.starts_with(t.path()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o077,
            0
        );
    }
}

#[test]
fn external_security_edit_is_desired_state_not_automatic_execution_grant() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let original = job("edit");
    j.put_legacy_job(&original).unwrap();
    let c = AutomationCatalog::open(t.path()).unwrap();
    let mut doc = c.read().unwrap();
    doc.legacy_jobs[0].action = crate::types::Action::Command {
        argv: vec!["changed".into()],
        target: crate::types::AgentTarget::Isolated,
        cwd: None,
        env: None,
        timeout_secs: None,
    };
    let changed = doc.legacy_jobs[0].clone();
    c.compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)
        .unwrap();
    j.load_legacy_jobs().unwrap();
    assert!(j
        .claim_run(
            "edit",
            "external",
            &legacy_job_digest(&changed).unwrap(),
            lease.epoch(),
            false
        )
        .unwrap()
        .is_none());
    assert_eq!(j.wake_summary().unwrap().recovery, 1);
}
#[test]
fn sql_trigger_failure_proves_atomic_completion_rollback() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("failure");
    j.put_legacy_job(&job).unwrap();
    let c = j
        .claim_run(
            "failure",
            "one",
            &legacy_job_digest(&job).unwrap(),
            lease.epoch(),
            false,
        )
        .unwrap()
        .unwrap();
    j.mark_effect_started(&c).unwrap();
    let db = rusqlite::Connection::open(journal_path(t.path()).unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(FAIL,'injected delivery failure'); END;").unwrap();
    assert!(j
        .complete_run_and_enqueue_delivery(
            &c,
            RunStatus::Succeeded,
            None,
            None,
            Some((&json!({"output":1}), "delivery"))
        )
        .is_err());
    assert_eq!(j.run_delivery_id(&c).unwrap(), None);
    db.execute_batch("DROP TRIGGER fail_outbox").unwrap();
    assert!(j
        .complete_run_and_enqueue_delivery(
            &c,
            RunStatus::Succeeded,
            None,
            None,
            Some((&json!({"output":1}), "delivery"))
        )
        .unwrap());
}
#[test]
fn pending_delivery_capacity_keeps_checkpoint_and_effect_admission_unchanged() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let request = source(&j);
    let db = rusqlite::Connection::open(journal_path(t.path()).unwrap()).unwrap();
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO outbox(event_seq,target,data,next_ms) SELECT -x,'blocked','{}',0 FROM n;").unwrap();
    assert!(j.ingest(&request).is_err());
    assert_eq!(j.checkpoint("app", "source").unwrap(), None);
    assert_eq!(j.latest("app", "source").unwrap(), None);
    let job = job("bounded");
    j.put_legacy_job(&job).unwrap();
    let c = j
        .claim_run(
            "bounded",
            "one",
            &legacy_job_digest(&job).unwrap(),
            lease.epoch(),
            false,
        )
        .unwrap()
        .unwrap();
    assert!(j.mark_effect_started(&c).is_err());
    assert_eq!(j.wake_summary().unwrap().pending_delivery, 4096);
}

#[test]
fn project_lock_unlink_cannot_create_second_private_runtime_owner() {
    let root = crate::test_support::tempdir();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    std::fs::create_dir_all(root.path().join(".refact/apps")).unwrap();
    let old = root.path().join(".refact/apps/.automation.lock");
    std::fs::write(&old, b"project controlled legacy inode").unwrap();
    std::fs::remove_file(&old).unwrap();
    std::fs::write(&old, b"replacement inode").unwrap();
    assert!(ProjectAutomationLease::acquire_exclusive(root.path()).is_err());
    drop(lease);
    assert!(ProjectAutomationLease::acquire_exclusive(root.path()).is_ok());
}
fn generic_definition() -> Definition {
    Definition {
        schema_version: 1,
        id: "generic".into(),
        revision: 1,
        owner: DefinitionOwner::Project,
        description: "generic".into(),
        enabled: true,
        trigger: AutomationTrigger::Continuous,
        action: AutomationAction::Transform {
            input_channel: "in".into(),
            output_channel: "out".into(),
            select_fields: vec![],
            max_output_bytes: 1024,
        },
        deliveries: vec![],
        lifecycle: Lifecycle::OnDemand,
        policy: ExecutionPolicy::default(),
    }
}
#[test]
fn generic_approval_claim_effect_completion_and_revocation_share_run_ledger() {
    let root = crate::test_support::tempdir();
    let journal = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    journal.activate_owner(&lease).unwrap();
    let definition = generic_definition();
    let catalog = AutomationCatalog::open(root.path()).unwrap();
    catalog
        .compare_and_swap(0, vec![definition.clone()])
        .unwrap();
    assert!(journal
        .claim_definition_run("generic", "one", lease.epoch())
        .unwrap()
        .is_none());
    journal
        .approve_definition("generic", &definition.security_digest().unwrap(), true)
        .unwrap();
    let claim = journal
        .claim_definition_run("generic", "one", lease.epoch())
        .unwrap()
        .unwrap();
    assert!(journal
        .claim_definition_run("generic", "one", lease.epoch())
        .unwrap()
        .is_none());
    assert!(journal.mark_effect_started(&claim).unwrap());
    assert!(journal
        .complete_run_and_enqueue_delivery(
            &claim,
            RunStatus::Succeeded,
            None,
            None,
            Some((&json!({"generic":true}), "output"))
        )
        .unwrap());
    assert!(journal.run_delivery_id(&claim).unwrap().is_some());
    journal
        .approve_definition("generic", &definition.security_digest().unwrap(), false)
        .unwrap();
    assert!(journal
        .claim_definition_run("generic", "two", lease.epoch())
        .unwrap()
        .is_none());
}
#[test]
fn cron_only_completion_and_ack_retention_is_bounded_without_any_source_ingest() {
    let root = crate::test_support::tempdir();
    let journal = AutomationJournal::open(root.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(root.path()).unwrap();
    journal.activate_owner(&lease).unwrap();
    let job = job("retention");
    journal.put_legacy_job(&job).unwrap();
    let digest = legacy_job_digest(&job).unwrap();
    let mut db = rusqlite::Connection::open(journal_path(root.path()).unwrap()).unwrap();
    let tx = db.transaction().unwrap();
    for n in 0..4098 {
        tx.execute(
            "INSERT INTO runs(run_id,job_id,occurrence,digest,owner_epoch,idempotent,status,effect_started) VALUES(?1,'retention',?1,?2,?3,1,'succeeded',1)",
            rusqlite::params![format!("seed-{n}"), digest, lease.epoch()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO events(app,source,digest,producer,event_id,data,at_ms) VALUES('cron','retention',?1,'run',?2,?3,0)",
            rusqlite::params![digest, format!("seed-{n}"), json!({"n":n}).to_string()],
        )
        .unwrap();
        let seq = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO outbox(event_seq,target,data,next_ms,acked) VALUES(?1,'delivery',?2,0,?3)",
            rusqlite::params![seq, json!({"n":n}).to_string(), n != 0],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4098
    );
    let pending = journal.pending_deliveries(10, u64::MAX / 2).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].data, json!({"n":0}));
    let pending_seq = pending[0].event_seq;
    for n in 4098..4100 {
        let claim = journal
            .claim_run("retention", &n.to_string(), &digest, lease.epoch(), true)
            .unwrap()
            .unwrap();
        assert!(journal.mark_effect_started(&claim).unwrap());
        assert!(journal
            .complete_run_and_enqueue_delivery(
                &claim,
                RunStatus::Succeeded,
                None,
                None,
                Some((&json!({"n":n}), "delivery"))
            )
            .unwrap());
        let seq = journal.run_delivery_id(&claim).unwrap().unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4098
        );
        journal.acknowledge_delivery(seq, "delivery").unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4097
        );
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4097
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4097
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4096
    );
    let deliveries = journal.pending_deliveries(10, u64::MAX / 2).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].event_seq, pending_seq);
    assert_eq!(deliveries[0].data, json!({"n":0}));
}

#[test]
fn byte_policies_default_unlimited_and_preserve_positive_digest_covered_opt_ins() {
    let mut definition = generic_definition();
    definition.policy = ExecutionPolicy::default();
    assert_eq!(definition.policy.max_event_bytes, 0);
    definition.action = AutomationAction::Transform {
        input_channel: "input".into(),
        output_channel: "output".into(),
        select_fields: vec!["history".into()],
        max_output_bytes: 0,
    };
    definition.validate().unwrap();
    let digest = definition.security_digest().unwrap();
    definition.policy.max_event_bytes = 500000;
    if let AutomationAction::Transform {
        max_output_bytes, ..
    } = &mut definition.action
    {
        *max_output_bytes = 500000;
    }
    definition.validate().unwrap();
    assert_ne!(definition.security_digest().unwrap(), digest);
    let mut persisted = serde_json::to_value(&definition).unwrap();
    let restored: Definition = serde_json::from_value(persisted.clone()).unwrap();
    assert_eq!(
        restored.security_digest().unwrap(),
        definition.security_digest().unwrap()
    );
    assert_eq!(restored.policy.max_event_bytes, 500000);
    persisted["policy"]
        .as_object_mut()
        .unwrap()
        .remove("max_event_bytes");
    persisted["action"]
        .as_object_mut()
        .unwrap()
        .remove("max_output_bytes");
    let unset: Definition = serde_json::from_value(persisted).unwrap();
    assert_eq!(unset.policy.max_event_bytes, 0);
    assert!(matches!(
        unset.action,
        AutomationAction::Transform {
            max_output_bytes: 0,
            ..
        }
    ));
}

#[test]
fn explicit_run_delivery_accepts_large_payload_once_and_rejects_stale_claim() {
    let t = crate::test_support::tempdir();
    let j = AutomationJournal::open(t.path()).unwrap();
    let lease = ProjectAutomationLease::acquire(t.path()).unwrap();
    j.activate_owner(&lease).unwrap();
    let job = job("large-run-delivery");
    j.put_legacy_job(&job).unwrap();
    let claim = j
        .claim_run(
            &job.id,
            "one",
            &legacy_job_digest(&job).unwrap(),
            lease.epoch(),
            false,
        )
        .unwrap()
        .unwrap();
    let payload = json!({"history":"run-data".repeat(50000)});
    let seq = j.enqueue_run_delivery(&claim, &payload, "view").unwrap();
    assert_eq!(
        j.enqueue_run_delivery(&claim, &payload, "view").unwrap(),
        seq
    );
    let deliveries = j.pending_deliveries(10, 0).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].data, payload);
    let mut stale = claim.clone();
    stale.run_id = "missing-run".into();
    assert!(j.enqueue_run_delivery(&stale, &payload, "view").is_err());
    assert_eq!(j.pending_deliveries(10, 0).unwrap().len(), 1);
}
