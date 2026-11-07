use refact_work_core::{
    AdmissionError, AdmissionRequest, AdmissionScheduler, CoordinationSettings, Permit, RunKey,
    RuntimeKind, TicketState, WorkKey, WorkScope,
};
use std::time::Duration;
use tokio::time::timeout;

fn settings(engine: usize, project: usize, root: usize) -> CoordinationSettings {
    CoordinationSettings {
        max_active_engine: engine,
        max_active_project: project,
        max_active_root: root,
        ..CoordinationSettings::default()
    }
}

fn request(runtime: RuntimeKind, id: &str, project: &str, root: &str) -> AdmissionRequest {
    AdmissionRequest::new(
        RunKey::new(WorkKey::new(runtime, id), 1),
        WorkScope::new(project.into_project(), root.into_root()),
    )
}

trait KeyExt {
    fn into_project(self) -> refact_work_core::ProjectKey;
    fn into_root(self) -> refact_work_core::CheckoutKey;
}

impl KeyExt for &str {
    fn into_project(self) -> refact_work_core::ProjectKey {
        refact_work_core::ProjectKey::new(self)
    }
    fn into_root(self) -> refact_work_core::CheckoutKey {
        refact_work_core::CheckoutKey::new(self)
    }
}

async fn acquire_now(sched: &AdmissionScheduler, req: AdmissionRequest) -> Permit {
    let ticket = sched.reserve(req).expect("reserve");
    timeout(Duration::from_secs(1), ticket.acquire())
        .await
        .expect("no timeout")
        .expect("permit")
}

async fn poll_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn repeated_aged_resumes_cannot_starve_fresh_work_in_the_same_root() {
    let sched = AdmissionScheduler::new(CoordinationSettings {
        max_resume_burst: 2,
        ..settings(1, 1, 1)
    });
    let mut old = Vec::new();
    for id in ["old1", "old2", "old3"] {
        old.push(
            acquire_now(&sched, request(RuntimeKind::Trajectory, id, "p", "r"))
                .await
                .park()
                .unwrap(),
        );
    }
    let holder = acquire_now(&sched, request(RuntimeKind::Trajectory, "holder", "p", "r")).await;
    let mut fresh = Box::pin(
        sched
            .reserve(request(RuntimeKind::Trajectory, "fresh", "p", "r"))
            .unwrap()
            .acquire(),
    );
    poll_pending(fresh.as_mut()).await;
    let mut waits = Vec::new();
    for ticket in old {
        let mut wait = Box::pin(ticket.acquire());
        poll_pending(wait.as_mut()).await;
        waits.push(Some(wait));
    }
    holder.release();
    for index in [0, 1] {
        timeout(Duration::from_secs(1), waits[index].take().unwrap())
            .await
            .unwrap()
            .unwrap()
            .release();
    }
    let fresh = timeout(Duration::from_secs(1), fresh)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.run().work.id, "fresh");
    fresh.release();
    timeout(Duration::from_secs(1), waits[2].take().unwrap())
        .await
        .unwrap()
        .unwrap()
        .release();
}

#[tokio::test]
async fn parked_residency_does_not_consume_new_queue_admission() {
    let sched = AdmissionScheduler::new(CoordinationSettings {
        max_pending: 1,
        max_outstanding: 8,
        ..settings(1, 1, 1)
    });
    let parent = acquire_now(&sched, request(RuntimeKind::Trajectory, "parent", "p", "r"))
        .await
        .park()
        .unwrap();
    let child = acquire_now(
        &sched,
        request(RuntimeKind::BackgroundAgent, "child", "p", "r"),
    )
    .await;
    let pending = sched
        .reserve(request(RuntimeKind::BackgroundAgent, "next", "p", "r"))
        .unwrap();
    assert!(matches!(
        sched.reserve(request(RuntimeKind::Trajectory, "extra", "p", "r")),
        Err(AdmissionError::QueueFull { .. })
    ));
    drop(pending);
    child.release();
    parent.acquire().await.unwrap().release();
    assert_eq!(sched.snapshot().outstanding, 0);
}

#[tokio::test]
async fn engine_cap_applies_across_runtimes() {
    let sched = AdmissionScheduler::new(settings(2, 16, 16));
    let _p1 = acquire_now(&sched, request(RuntimeKind::Trajectory, "t1", "p1", "r1")).await;
    let _p2 = acquire_now(
        &sched,
        request(RuntimeKind::BackgroundAgent, "b1", "p2", "r2"),
    )
    .await;

    let ticket = sched
        .reserve(request(RuntimeKind::Trajectory, "t2", "p3", "r3"))
        .unwrap();
    assert!(
        timeout(Duration::from_millis(150), ticket.acquire())
            .await
            .is_err(),
        "third acquire must wait: engine cap is shared by both runtimes"
    );

    let snap = sched.snapshot();
    assert_eq!(snap.active, 2);
    assert_eq!(
        snap.active_by_runtime.get(&RuntimeKind::Trajectory),
        Some(&1)
    );
    assert_eq!(
        snap.active_by_runtime.get(&RuntimeKind::BackgroundAgent),
        Some(&1)
    );
}

#[tokio::test]
async fn project_cap_is_enforced_below_engine_cap() {
    let sched = AdmissionScheduler::new(settings(8, 2, 8));
    let _p1 = acquire_now(
        &sched,
        request(RuntimeKind::Trajectory, "a", "shared", "r1"),
    )
    .await;
    let _p2 = acquire_now(
        &sched,
        request(RuntimeKind::Trajectory, "b", "shared", "r2"),
    )
    .await;

    let blocked = sched
        .reserve(request(RuntimeKind::Trajectory, "c", "shared", "r3"))
        .unwrap();
    assert!(timeout(Duration::from_millis(150), blocked.acquire())
        .await
        .is_err());

    let other = acquire_now(&sched, request(RuntimeKind::Trajectory, "d", "other", "r4")).await;
    assert_eq!(other.scope().project.as_str(), "other");
    assert_eq!(sched.snapshot().active, 3);
}

#[tokio::test]
async fn root_cap_is_enforced_below_engine_cap() {
    let sched = AdmissionScheduler::new(settings(8, 8, 1));
    let _p1 = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p1", "root")).await;

    let blocked = sched
        .reserve(request(RuntimeKind::BackgroundAgent, "b", "p2", "root"))
        .unwrap();
    assert!(timeout(Duration::from_millis(150), blocked.acquire())
        .await
        .is_err());
    assert_eq!(
        sched.snapshot().active_by_root.get(&"root".into_root()),
        Some(&1)
    );
}

#[tokio::test]
async fn panicking_setup_releases_permit_and_ticket_exactly_once() {
    let sched = std::sync::Arc::new(AdmissionScheduler::new(settings(1, 1, 1)));
    let permit = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let ticket = sched
        .reserve(request(RuntimeKind::BackgroundAgent, "b", "p", "r"))
        .unwrap();
    assert_eq!(sched.snapshot().active, 1);
    assert_eq!(sched.snapshot().reserved, 1);
    let setup = tokio::spawn(async move {
        let _permit = permit;
        let _ticket = ticket;
        panic!("setup failed after admission");
    });
    let error = setup.await.unwrap_err();
    assert!(error.is_panic());
    let snap = sched.snapshot();
    assert_eq!(snap.active, 0);
    assert_eq!(snap.reserved, 0);
    assert_eq!(snap.outstanding, 0);
    let next = timeout(
        Duration::from_secs(5),
        acquire_now(&sched, request(RuntimeKind::BackgroundAgent, "b", "p", "r")),
    )
    .await
    .expect("capacity returns after a panicking owner");
    assert_eq!(sched.snapshot().active, 1);
    drop(next);
    assert_eq!(sched.snapshot().active, 0);
}

#[tokio::test]
async fn dropping_a_pending_acquire_future_frees_the_queue_slot() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let held = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let ticket = sched
        .reserve(request(RuntimeKind::Trajectory, "b", "p", "r"))
        .unwrap();
    {
        let fut = ticket.acquire();
        assert!(timeout(Duration::from_millis(80), fut).await.is_err());
    }
    assert_eq!(sched.snapshot().pending, 0);
    assert_eq!(sched.snapshot().outstanding, 1);
    drop(held);
    assert_eq!(sched.snapshot().active, 0);
}

#[tokio::test]
async fn cancel_wakes_the_waiter_with_cancelled() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let _held = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let req = request(RuntimeKind::BackgroundAgent, "b", "p", "r");
    let run = req.run.clone();
    let ticket = sched.reserve(req).unwrap();
    let waiter = tokio::spawn(ticket.acquire());

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(sched.cancel(&run));
    assert!(!sched.cancel(&run));

    let err = timeout(Duration::from_secs(1), waiter)
        .await
        .expect("no timeout")
        .expect("join")
        .unwrap_err();
    assert_eq!(err, AdmissionError::Cancelled);
    assert_eq!(sched.snapshot().pending, 0);
}

#[tokio::test]
async fn cancelling_an_active_run_retains_capacity_until_runner_drops_permit() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let req = request(RuntimeKind::Trajectory, "a", "p", "r");
    let run = req.run.clone();
    let permit = acquire_now(&sched, req).await;
    assert!(sched.cancel(&run));
    assert!(!sched.cancel(&run));
    assert_eq!(sched.snapshot().active, 1);
    drop(permit);
    assert_eq!(sched.snapshot().active, 0);
    assert_eq!(sched.snapshot().outstanding, 0);
}

#[tokio::test]
async fn duplicate_run_is_rejected_while_tracked() {
    let sched = AdmissionScheduler::new(settings(4, 4, 4));
    let permit = acquire_now(&sched, request(RuntimeKind::Trajectory, "dup", "p", "r")).await;
    let err = sched
        .reserve(request(RuntimeKind::Trajectory, "dup", "p", "r"))
        .unwrap_err();
    assert!(matches!(err, AdmissionError::Duplicate { .. }));

    sched
        .reserve(request(RuntimeKind::BackgroundAgent, "dup", "p", "r"))
        .expect("same id under a different runtime is a different work key");

    permit.release();
    sched
        .reserve(request(RuntimeKind::Trajectory, "dup", "p", "r"))
        .expect("re-reservable after release");
}

#[tokio::test]
async fn park_releases_capacity_and_keeps_the_work() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let permit = acquire_now(&sched, request(RuntimeKind::Trajectory, "owner", "p", "r")).await;
    let parked = permit.park().expect("park");
    assert_eq!(parked.state(), Some(TicketState::Parked));

    let snap = sched.snapshot();
    assert_eq!(snap.active, 0);
    assert_eq!(snap.parked, 1);
    assert_eq!(snap.outstanding, 1);
    assert!(sched
        .reserve(request(RuntimeKind::Trajectory, "owner", "p", "r"))
        .is_err());

    let child = acquire_now(
        &sched,
        request(RuntimeKind::BackgroundAgent, "child", "p", "r"),
    )
    .await;
    assert_eq!(sched.snapshot().active, 1);
    child.release();

    let permit = timeout(Duration::from_secs(1), parked.acquire())
        .await
        .expect("no timeout")
        .expect("reacquire");
    assert_eq!(permit.run().work.id, "owner");
    assert_eq!(sched.snapshot().active, 1);
    assert_eq!(sched.snapshot().tickets[0].park_count, 1);
}

#[tokio::test]
async fn outstanding_full_is_rejected_explicitly() {
    let sched = AdmissionScheduler::new(CoordinationSettings {
        max_pending: 8,
        max_outstanding: 2,
        ..settings(1, 1, 1)
    });
    let _held = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let _t1 = sched
        .reserve(request(RuntimeKind::Trajectory, "b", "p", "r"))
        .unwrap();
    let err = sched
        .reserve(request(RuntimeKind::Trajectory, "c", "p", "r"))
        .unwrap_err();
    assert!(
        matches!(
            err,
            AdmissionError::OutstandingFull {
                outstanding: 2,
                max_outstanding: 2
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn refused_admission_is_terminal_and_never_leaks_a_ticket_or_a_run_identity() {
    let sched = AdmissionScheduler::new(CoordinationSettings {
        max_pending: 1,
        max_outstanding: 3,
        ..settings(1, 1, 1)
    });
    let handle = sched.handle();
    let owner = acquire_now(&sched, request(RuntimeKind::Trajectory, "owner", "p", "r")).await;
    let queued_request = request(RuntimeKind::Trajectory, "queued", "p", "r");
    let queued_run = queued_request.run.clone();
    let queued = sched.reserve(queued_request).unwrap();

    let refused = request(RuntimeKind::BackgroundAgent, "child", "p", "r");
    let child_run = refused.run.clone();
    let err = sched.reserve(refused).unwrap_err();
    assert!(
        matches!(
            err,
            AdmissionError::QueueFull {
                pending: 1,
                max_pending: 1
            }
        ),
        "{err:?}"
    );
    assert!(
        !handle.is_tracked(&child_run),
        "a refused child must not exist as a run its parent could wait on"
    );
    assert_eq!(sched.snapshot().outstanding, 2);

    // Resume and cancellation are not creation, and must never be refused
    // because the creation bound is full.
    let parked = owner.park().expect("park");
    let resumed = timeout(Duration::from_secs(1), parked.acquire())
        .await
        .expect("no timeout")
        .expect("a resume is not new queue admission");
    assert_eq!(resumed.run().work.id, "owner");
    assert!(sched.cancel(&queued_run));
    drop(queued);
    resumed.release();
    sched
        .reserve(request(RuntimeKind::BackgroundAgent, "child", "p", "r"))
        .expect("refusal is terminal for that attempt, not a permanent ban");
}

#[tokio::test]
async fn depth_limit_and_disabled_scheduler_reject_reservations() {
    let sched = AdmissionScheduler::new(CoordinationSettings {
        max_subchat_depth: 2,
        ..settings(4, 4, 4)
    });
    let err = sched
        .reserve(request(RuntimeKind::Trajectory, "deep", "p", "r").with_depth(3))
        .unwrap_err();
    assert!(matches!(
        err,
        AdmissionError::DepthExceeded {
            depth: 3,
            ..
        }
    ));
    sched
        .reserve(request(RuntimeKind::Trajectory, "ok", "p", "r").with_depth(2))
        .expect("depth at the limit is allowed");

    let sched = AdmissionScheduler::new(CoordinationSettings {
        enabled: false,
        ..settings(4, 4, 4)
    });
    assert_eq!(
        sched
            .reserve(request(RuntimeKind::Trajectory, "x", "p", "r"))
            .unwrap_err(),
        AdmissionError::Disabled
    );
}

#[tokio::test]
async fn configure_raises_cap_and_wakes_waiters() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let _held = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let ticket = sched
        .reserve(request(RuntimeKind::Trajectory, "b", "p", "r"))
        .unwrap();
    let waiter = tokio::spawn(ticket.acquire());
    tokio::time::sleep(Duration::from_millis(20)).await;

    sched.configure(settings(4, 4, 4)).expect("configure");
    let permit = timeout(Duration::from_secs(1), waiter)
        .await
        .expect("no timeout")
        .expect("join")
        .expect("permit");
    assert_eq!(permit.run().work.id, "b");
    assert_eq!(sched.snapshot().active, 2);
}

#[tokio::test]
async fn cap_reduction_never_evicts_running_work_but_stops_new_admissions() {
    let sched = AdmissionScheduler::new(settings(4, 4, 4));
    let a = acquire_now(&sched, request(RuntimeKind::Trajectory, "a", "p", "r")).await;
    let b = acquire_now(&sched, request(RuntimeKind::Trajectory, "b", "p", "r")).await;
    let c = acquire_now(&sched, request(RuntimeKind::Trajectory, "c", "p", "r")).await;
    assert_eq!(sched.snapshot().active, 3);

    sched.configure(settings(2, 2, 2)).expect("configure");
    assert_eq!(sched.snapshot().active, 3);

    let ticket = sched
        .reserve(request(RuntimeKind::Trajectory, "d", "p", "r"))
        .unwrap();
    let waiter = tokio::spawn(ticket.acquire());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!waiter.is_finished());

    a.release();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!waiter.is_finished(), "still over the reduced cap");

    b.release();
    let permit = timeout(Duration::from_secs(1), waiter)
        .await
        .expect("no timeout")
        .expect("join")
        .expect("permit");
    assert_eq!(permit.run().work.id, "d");
    assert_eq!(sched.snapshot().active, 2);
    drop(c);
    drop(permit);
    assert_eq!(sched.snapshot().active, 0);
}

#[tokio::test]
async fn parent_never_blocks_while_spawning_at_full_capacity() {
    let sched = AdmissionScheduler::new(settings(1, 1, 1));
    let parent = acquire_now(&sched, request(RuntimeKind::Trajectory, "parent", "p", "r")).await;

    let mut children = Vec::new();
    for i in 0..16 {
        children.push(
            sched
                .reserve(
                    request(
                        RuntimeKind::BackgroundAgent,
                        &format!("child-{i}"),
                        "p",
                        "r",
                    )
                    .with_depth(1),
                )
                .expect("spawn reservation must be immediate"),
        );
    }
    assert_eq!(sched.snapshot().reserved, 16);
    assert_eq!(sched.snapshot().active, 1);

    let waiters: Vec<_> = children
        .into_iter()
        .map(|t| tokio::spawn(t.acquire()))
        .collect();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(waiters.iter().all(|w| !w.is_finished()));

    let parked = parent.park().expect("parent parks while children run");
    let mut done = 0;
    for w in waiters {
        let permit = timeout(Duration::from_secs(2), w)
            .await
            .expect("no timeout")
            .expect("join")
            .expect("permit");
        done += 1;
        permit.release();
    }
    assert_eq!(done, 16);

    let permit = timeout(Duration::from_secs(1), parked.acquire())
        .await
        .expect("no timeout")
        .expect("parent resumes");
    assert_eq!(permit.run().work.id, "parent");
}

#[tokio::test]
async fn snapshot_invariants_hold_under_mixed_churn() {
    let sched = AdmissionScheduler::new(settings(3, 2, 2));
    let mut permits = Vec::new();
    let mut tickets = Vec::new();

    for i in 0..12 {
        let project = format!("p{}", i % 3);
        let root = format!("r{}", i % 4);
        let req = request(
            if i % 2 == 0 {
                RuntimeKind::Trajectory
            } else {
                RuntimeKind::BackgroundAgent
            },
            &format!("run-{i}"),
            &project,
            &root,
        );
        let ticket = sched.reserve(req).unwrap();
        if i % 3 == 0 {
            if let Ok(Ok(p)) = timeout(Duration::from_millis(50), ticket.acquire()).await {
                permits.push(p);
            }
        } else {
            tickets.push(ticket);
        }

        let snap = sched.snapshot();
        assert!(snap.active <= snap.settings.max_active_engine);
        for count in snap.active_by_project.values() {
            assert!(*count <= snap.settings.max_active_project);
        }
        for count in snap.active_by_root.values() {
            assert!(*count <= snap.settings.max_active_root);
        }
        assert_eq!(
            snap.active + snap.pending + snap.reserved + snap.parked + snap.evicted,
            snap.outstanding
        );
        assert_eq!(snap.active, snap.active_by_runtime.values().sum::<usize>());
        assert_eq!(
            snap.active,
            snap.tickets
                .iter()
                .filter(|t| t.state == TicketState::Active)
                .count()
        );
    }

    let parked: Vec<_> = permits.drain(..).map(|p| p.park().expect("park")).collect();
    let snap = sched.snapshot();
    assert_eq!(snap.active, 0);
    assert_eq!(snap.parked, parked.len());
    assert!(snap.active_by_project.is_empty());
    assert!(snap.active_by_root.is_empty());

    drop(parked);
    drop(tickets);
    let snap = sched.snapshot();
    assert_eq!(snap.outstanding, 0);
    assert_eq!(snap.active, 0);
    assert!(snap.tickets.is_empty());
}
