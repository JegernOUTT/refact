//! Urgency orders admission; it never widens a cap, never touches a running
//! run, and never starves a class. Measured in admission opportunities.
use refact_work_core::{
    AdmissionRequest, AdmissionScheduler, CheckoutKey, CoordinationSettings, Permit, ProjectKey,
    TicketSnapshot, TicketState, WorkKey, WorkPriority, WorkScope,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

type Wait = Pin<Box<dyn Future<Output = Result<Permit, refact_work_core::AdmissionError>>>>;

fn scheduler(settings: CoordinationSettings) -> AdmissionScheduler {
    AdmissionScheduler::new(settings)
}

fn caps(engine: usize, project: usize, root: usize) -> CoordinationSettings {
    CoordinationSettings {
        max_active_engine: engine,
        max_active_project: project,
        max_active_root: root,
        ..CoordinationSettings::default()
    }
}

fn request(id: &str, project: &str, root: &str) -> AdmissionRequest {
    AdmissionRequest::new(
        WorkKey::trajectory(id).run(1),
        WorkScope::new(ProjectKey::new(project), CheckoutKey::new(root)),
    )
}

fn reserve(
    scheduler: &AdmissionScheduler,
    id: &str,
    root: &str,
    priority: WorkPriority,
) -> refact_work_core::Ticket {
    scheduler
        .reserve(request(id, "project", root).with_priority(priority))
        .expect("reserve")
}

async fn admit(
    scheduler: &AdmissionScheduler,
    id: &str,
    root: &str,
    priority: WorkPriority,
) -> Permit {
    reserve(scheduler, id, root, priority)
        .acquire()
        .await
        .expect("permit")
}

async fn pending(wait: &mut Wait) {
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

async fn ready(mut wait: Wait) -> Permit {
    std::future::poll_fn(|cx| match wait.as_mut().poll(cx) {
        Poll::Ready(result) => Poll::Ready(result.unwrap()),
        Poll::Pending => panic!("scheduler selected a ticket without delivering its permit"),
    })
    .await
}

fn active(scheduler: &AdmissionScheduler) -> String {
    scheduler
        .snapshot()
        .tickets
        .into_iter()
        .find(|ticket| ticket.state == TicketState::Active)
        .expect("one admission opportunity must be used")
        .run
        .work
        .id
}

fn ticket(scheduler: &AdmissionScheduler, id: &str) -> Option<TicketSnapshot> {
    scheduler
        .snapshot()
        .tickets
        .into_iter()
        .find(|ticket| ticket.run.work.id == id)
}

#[tokio::test]
async fn an_urgent_run_outranks_a_lower_class_run_that_queued_first() {
    let scheduler = scheduler(caps(1, 1, 1));
    let holder = admit(&scheduler, "holder", "r", WorkPriority::Normal).await;
    let mut waits = BTreeMap::new();
    for (id, priority) in [
        ("batch", WorkPriority::Speculative),
        ("chore", WorkPriority::Normal),
        ("typing", WorkPriority::UserBlocking),
    ] {
        let mut wait: Wait = Box::pin(reserve(&scheduler, id, "r", priority).acquire());
        pending(&mut wait).await;
        waits.insert(id.to_string(), wait);
    }
    holder.release();
    for expected in ["typing", "chore", "batch"] {
        assert_eq!(
            active(&scheduler),
            expected,
            "the ladder, not arrival order, decides who goes next"
        );
        ready(waits.remove(expected).unwrap()).await.release();
    }
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn urgency_reaches_across_roots_without_spending_the_preempted_roots_turn() {
    let scheduler = scheduler(caps(1, 1, 1));
    let holder = admit(&scheduler, "holder", "chat", WorkPriority::Normal).await;
    let mut waits = BTreeMap::new();
    for (id, root, priority) in [
        ("board-1", "board", WorkPriority::Normal),
        ("board-2", "board", WorkPriority::Normal),
        ("typing", "chat", WorkPriority::UserBlocking),
    ] {
        let mut wait: Wait = Box::pin(reserve(&scheduler, id, root, priority).acquire());
        pending(&mut wait).await;
        waits.insert(id.to_string(), wait);
    }
    holder.release();
    // The rotation owed "board" this turn; urgency took it, so the rotation
    // still owes it and "board" is served next.
    for expected in ["typing", "board-1", "board-2"] {
        assert_eq!(active(&scheduler), expected);
        ready(waits.remove(expected).unwrap()).await.release();
    }
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn a_continuous_user_blocking_stream_leaves_speculative_its_minimum_share() {
    let scheduler = scheduler(CoordinationSettings {
        max_priority_burst: 2,
        ..caps(1, 1, 1)
    });
    let mut held = Some(admit(&scheduler, "seed", "r", WorkPriority::Normal).await);
    let mut waits = BTreeMap::new();
    let mut batch: Wait =
        Box::pin(reserve(&scheduler, "batch", "r", WorkPriority::Speculative).acquire());
    pending(&mut batch).await;
    waits.insert("batch".to_string(), batch);
    let mut admitted = Vec::new();
    for round in 0..8 {
        let id = format!("typing-{round}");
        let mut wait: Wait =
            Box::pin(reserve(&scheduler, &id, "r", WorkPriority::UserBlocking).acquire());
        pending(&mut wait).await;
        waits.insert(id, wait);
        held.take().unwrap().release();
        let id = active(&scheduler);
        held = Some(ready(waits.remove(&id).unwrap()).await);
        admitted.push(id.clone());
        if id == "batch" {
            break;
        }
    }
    let position = admitted
        .iter()
        .position(|id| id == "batch")
        .unwrap_or_else(|| {
            panic!("an endless user-blocking stream starved speculative work: {admitted:?}")
        });
    assert!(
        position < scheduler.settings().max_priority_burst + 1,
        "every class keeps one opportunity in every max_priority_burst + 1: {admitted:?}"
    );
    drop(waits);
    drop(held);
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn urgency_raises_no_engine_project_or_root_cap() {
    for (settings, root) in [
        (caps(1, 8, 8), "other"),
        (caps(8, 1, 8), "other"),
        (caps(8, 8, 1), "held"),
    ] {
        let scheduler = scheduler(settings);
        let holder = admit(&scheduler, "holder", "held", WorkPriority::Speculative).await;
        let mut urgent: Wait =
            Box::pin(reserve(&scheduler, "typing", root, WorkPriority::UserBlocking).acquire());
        pending(&mut urgent).await;
        let snapshot = scheduler.snapshot();
        assert_eq!(
            snapshot.active, 1,
            "the most urgent class still waits for a permit to exist"
        );
        assert!(snapshot.active <= snapshot.settings.max_active_engine);
        for count in snapshot.active_by_project.values() {
            assert!(*count <= snapshot.settings.max_active_project);
        }
        for count in snapshot.active_by_root.values() {
            assert!(*count <= snapshot.settings.max_active_root);
        }
        assert_eq!(
            ticket(&scheduler, "typing").map(|t| t.state),
            Some(TicketState::Pending)
        );
        holder.release();
        ready(urgent).await.release();
        assert_eq!(scheduler.snapshot().outstanding, 0);
    }
}

#[tokio::test]
async fn urgency_never_cancels_or_unseats_a_running_lower_class_run() {
    let scheduler = scheduler(caps(1, 1, 1));
    let running = admit(&scheduler, "batch", "r", WorkPriority::Speculative).await;
    let mut urgent: Wait =
        Box::pin(reserve(&scheduler, "typing", "r", WorkPriority::UserBlocking).acquire());
    pending(&mut urgent).await;
    let snapshot = scheduler.snapshot();
    assert_eq!((snapshot.active, snapshot.evicted), (1, 0));
    assert_eq!(
        ticket(&scheduler, "batch").map(|t| t.state),
        Some(TicketState::Active),
        "a permit already granted is never taken back by a more urgent arrival"
    );
    pending(&mut urgent).await;

    // The lower-class run finishes on its own terms, and only then does the
    // urgent one start.
    running.release();
    let permit = ready(urgent).await;
    assert_eq!(permit.run().work.id, "typing");
    permit.release();
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn residency_eviction_ignores_priority_and_keeps_the_evicted_class() {
    let scheduler = scheduler(CoordinationSettings {
        max_parked_residency: 1,
        ..caps(1, 1, 1)
    });
    let urgent = admit(&scheduler, "typing", "r", WorkPriority::UserBlocking)
        .await
        .park()
        .expect("park");
    let later = admit(&scheduler, "batch", "r", WorkPriority::Speculative)
        .await
        .park()
        .expect("park");
    let checkpoint = ticket(&scheduler, "typing").expect("checkpoint");
    assert_eq!(
        checkpoint.state,
        TicketState::Evicted,
        "residency is memory, not order: urgency buys no resident parked slot"
    );
    assert_eq!(checkpoint.priority, WorkPriority::UserBlocking);
    assert_eq!(
        ticket(&scheduler, "batch").map(|t| t.state),
        Some(TicketState::Parked)
    );

    let permit = urgent.acquire().await.expect("an evicted run resumes");
    assert_eq!(permit.run().work.id, "typing");
    assert_eq!(
        ticket(&scheduler, "typing").map(|t| t.priority),
        Some(WorkPriority::UserBlocking),
        "a checkpoint keeps the class it was assigned"
    );
    permit.release();
    drop(later);
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn the_class_is_assigned_or_defaulted_and_visible_in_the_projection() {
    let scheduler = scheduler(CoordinationSettings {
        default_priority: WorkPriority::Speculative,
        ..caps(2, 2, 2)
    });
    let inherited = scheduler
        .reserve(request("inherited", "project", "r"))
        .expect("reserve")
        .acquire()
        .await
        .expect("permit");
    let assigned = admit(&scheduler, "assigned", "r", WorkPriority::UserBlocking).await;
    assert_eq!(
        ticket(&scheduler, "inherited").map(|t| t.priority),
        Some(WorkPriority::Speculative),
        "an unassigned run takes the configured default class"
    );
    assert_eq!(
        ticket(&scheduler, "assigned").map(|t| t.priority),
        Some(WorkPriority::UserBlocking)
    );
    let rendered = serde_json::to_string(&scheduler.snapshot()).unwrap();
    assert!(
        rendered.contains("\"priority\":\"user_blocking\""),
        "{rendered}"
    );
    assert!(
        rendered.contains("\"priority\":\"speculative\""),
        "{rendered}"
    );
    inherited.release();
    assigned.release();
    assert_eq!(scheduler.snapshot().outstanding, 0);
}
