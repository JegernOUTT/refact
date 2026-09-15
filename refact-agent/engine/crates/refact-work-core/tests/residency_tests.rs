//! Parked residency is bounded by checkpoint eviction, which is never a cancel.
use refact_work_core::{
    AdmissionError, AdmissionRequest, AdmissionScheduler, CoordinationSettings, Permit, RunKey,
    Ticket, TicketSnapshot, TicketState, WorkKey, WorkScope,
};

fn scheduler(residency: usize) -> AdmissionScheduler {
    AdmissionScheduler::new(CoordinationSettings {
        max_active_engine: 1,
        max_active_project: 1,
        max_active_root: 1,
        max_parked_residency: residency,
        ..CoordinationSettings::default()
    })
}

fn request(id: &str) -> AdmissionRequest {
    AdmissionRequest::new(
        RunKey::new(WorkKey::trajectory(id), 1),
        WorkScope::single("project"),
    )
}

async fn admit(scheduler: &AdmissionScheduler, id: &str) -> Permit {
    scheduler
        .reserve(request(id))
        .expect("reserve")
        .acquire()
        .await
        .expect("permit")
}

async fn park(scheduler: &AdmissionScheduler, id: &str) -> Ticket {
    admit(scheduler, id).await.park().expect("park")
}

fn ticket(scheduler: &AdmissionScheduler, id: &str) -> Option<TicketSnapshot> {
    scheduler
        .snapshot()
        .tickets
        .into_iter()
        .find(|ticket| ticket.run.work.id == id)
}

async fn poll_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn residency_bound_checkpoints_the_least_valuable_parked_run() {
    let scheduler = scheduler(2);
    let old = park(&scheduler, "old").await;
    let mid = park(&scheduler, "mid").await;
    let snapshot = scheduler.snapshot();
    assert_eq!((snapshot.parked, snapshot.evicted), (2, 0));

    let new = park(&scheduler, "new").await;
    let snapshot = scheduler.snapshot();
    assert_eq!((snapshot.parked, snapshot.evicted), (2, 1));
    assert_eq!(
        ticket(&scheduler, "old").map(|t| t.state),
        Some(TicketState::Evicted),
        "the run idle longest since its last park is the least valuable"
    );
    for resident in ["mid", "new"] {
        assert_eq!(
            ticket(&scheduler, resident).map(|t| t.state),
            Some(TicketState::Parked)
        );
    }
    assert_eq!(
        snapshot.outstanding, 3,
        "a checkpoint is retained work, not discarded work"
    );
    drop((old, mid, new));
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn an_evicted_run_resumes_from_its_checkpoint_with_age_and_accounting_intact() {
    let scheduler = scheduler(1);
    let old = park(&scheduler, "old").await;
    let newer = park(&scheduler, "newer").await;
    let checkpoint = ticket(&scheduler, "old").expect("checkpoint");
    assert_eq!(checkpoint.state, TicketState::Evicted);
    assert_eq!((checkpoint.park_count, checkpoint.evict_count), (1, 1));

    let blocker = admit(&scheduler, "blocker").await;
    let mut young = Box::pin(scheduler.reserve(request("young")).unwrap().acquire());
    poll_pending(young.as_mut()).await;
    let mut resume = Box::pin(old.acquire());
    poll_pending(resume.as_mut()).await;
    blocker.release();

    let permit = resume.await.expect("an evicted run resumes");
    assert_eq!(
        permit.run().work.id,
        "old",
        "a checkpoint keeps the original queue age it was accepted with"
    );
    let resumed = ticket(&scheduler, "old").expect("resumed");
    assert_eq!(resumed.state, TicketState::Active);
    assert_eq!((resumed.park_count, resumed.evict_count), (1, 1));
    assert_eq!(resumed.seq, checkpoint.seq);
    assert_eq!(resumed.depth, checkpoint.depth);
    assert_eq!(resumed.scope, checkpoint.scope);
    assert_eq!(scheduler.snapshot().evicted, 0);

    drop(young);
    permit.release();
    drop(newer);
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn eviction_is_reported_distinctly_from_cancellation() {
    let scheduler = scheduler(2);
    let evicted = park(&scheduler, "evicted").await;
    let cancelled = park(&scheduler, "cancelled").await;
    let resident = park(&scheduler, "resident").await;
    assert_eq!(
        ticket(&scheduler, "evicted").map(|t| t.state),
        Some(TicketState::Evicted)
    );

    assert!(scheduler.cancel(&request("cancelled").run));
    assert!(
        ticket(&scheduler, "cancelled").is_none(),
        "a cancelled run leaves no checkpoint behind"
    );
    assert_eq!(cancelled.acquire().await.unwrap_err(), AdmissionError::Gone);

    let permit = evicted
        .acquire()
        .await
        .expect("an evicted run resumes where a cancelled one is gone");
    assert_eq!(permit.run().work.id, "evicted");
    let snapshot = scheduler.snapshot();
    assert_eq!((snapshot.active, snapshot.evicted), (1, 0));
    permit.release();
    drop(resident);
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn lowering_the_residency_bound_drains_into_checkpoints_and_kills_nothing() {
    let scheduler = scheduler(4);
    let first = park(&scheduler, "first").await;
    let second = park(&scheduler, "second").await;
    let third = park(&scheduler, "third").await;
    let running = admit(&scheduler, "running").await;

    let mut settings = scheduler.settings();
    settings.max_parked_residency = 1;
    scheduler.configure(settings).expect("configure");

    let snapshot = scheduler.snapshot();
    assert_eq!(
        (snapshot.parked, snapshot.evicted, snapshot.active),
        (1, 2, 1)
    );
    assert_eq!(
        ticket(&scheduler, "third").map(|t| t.state),
        Some(TicketState::Parked),
        "the most recently parked run is the most valuable one to keep resident"
    );
    assert_eq!(
        snapshot.outstanding, 4,
        "a lowered live bound drains parked work, it never terminates it"
    );

    running.release();
    for ticket in [first, second, third] {
        ticket
            .acquire()
            .await
            .expect("every drained run resumes")
            .release();
    }
    assert_eq!(scheduler.snapshot().outstanding, 0);
}
