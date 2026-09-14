//! Correctness is measured in admission opportunities, not elapsed wall time.
use refact_work_core::{
    AdmissionRequest, AdmissionScheduler, CoordinationSettings, Permit, TicketState, WorkKey,
    WorkScope,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

type Wait = Pin<Box<dyn Future<Output = Result<Permit, refact_work_core::AdmissionError>>>>;

fn scheduler() -> AdmissionScheduler {
    AdmissionScheduler::new(CoordinationSettings {
        max_active_engine: 1,
        max_active_project: 1,
        max_active_root: 1,
        max_resume_burst: 2,
        ..Default::default()
    })
}

fn reserve(scheduler: &AdmissionScheduler, id: &str, root: &str) -> refact_work_core::Ticket {
    scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::trajectory(id).run(1),
            WorkScope::new(
                refact_work_core::ProjectKey::new("project"),
                refact_work_core::CheckoutKey::new(root),
            ),
        ))
        .unwrap()
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

#[tokio::test]
async fn root_weights_apply_across_mixed_runtimes_without_widening_capacity() {
    let scheduler = scheduler();
    let mut settings = scheduler.settings();
    settings.root_weights.insert("heavy".into(), 2);
    scheduler.configure(settings).unwrap();
    let holder = reserve(&scheduler, "holder", "holder")
        .acquire()
        .await
        .unwrap();
    let mut waits = BTreeMap::new();
    for (id, root) in [
        ("a1", "heavy"),
        ("a2", "heavy"),
        ("a3", "heavy"),
        ("b1", "light"),
        ("b2", "light"),
    ] {
        let runtime = if id == "a2" {
            refact_work_core::RuntimeKind::BackgroundAgent
        } else {
            refact_work_core::RuntimeKind::Trajectory
        };
        let ticket = scheduler
            .reserve(AdmissionRequest::new(
                WorkKey::new(runtime, id).run(1),
                WorkScope::new(
                    refact_work_core::ProjectKey::new("project"),
                    refact_work_core::CheckoutKey::new(root),
                ),
            ))
            .unwrap();
        let mut wait: Wait = Box::pin(ticket.acquire());
        pending(&mut wait).await;
        waits.insert(id.to_string(), wait);
    }
    holder.release();
    for expected in ["a1", "a2", "b1", "a3", "b2"] {
        assert_eq!(scheduler.snapshot().active, 1);
        assert_eq!(active(&scheduler), expected);
        ready(waits.remove(expected).unwrap()).await.release();
    }
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn reducing_root_weight_live_does_not_keep_the_old_large_credit() {
    let scheduler = scheduler();
    let mut settings = scheduler.settings();
    settings.root_weights.insert("heavy".into(), 10);
    scheduler.configure(settings.clone()).unwrap();
    let holder = reserve(&scheduler, "holder", "heavy")
        .acquire()
        .await
        .unwrap();
    let mut waits = BTreeMap::new();
    for (id, root) in [("a1", "heavy"), ("a2", "heavy"), ("b1", "light")] {
        let mut wait: Wait = Box::pin(reserve(&scheduler, id, root).acquire());
        pending(&mut wait).await;
        waits.insert(id.to_string(), wait);
    }
    settings.root_weights.insert("heavy".into(), 1);
    scheduler.configure(settings).unwrap();
    holder.release();
    for expected in ["a1", "b1", "a2"] {
        assert_eq!(active(&scheduler), expected);
        ready(waits.remove(expected).unwrap()).await.release();
    }
}

#[tokio::test]
async fn newly_arriving_roots_do_not_cut_ahead_of_an_existing_root_forever() {
    let scheduler = scheduler();
    let holder = reserve(&scheduler, "holder", "b").acquire().await.unwrap();
    let mut victim: Wait = Box::pin(reserve(&scheduler, "victim", "a").acquire());
    pending(&mut victim).await;
    let mut waits = BTreeMap::from([("victim".to_string(), victim)]);
    let mut held = Some(holder);
    let mut admitted = Vec::new();
    for arrival in ["c", "cc", "ccc"] {
        let mut wait: Wait = Box::pin(reserve(&scheduler, arrival, arrival).acquire());
        pending(&mut wait).await;
        waits.insert(arrival.to_string(), wait);
        held.take().unwrap().release();
        let id = active(&scheduler);
        held = Some(ready(waits.remove(&id).unwrap()).await);
        admitted.push(id.clone());
        if id == "victim" {
            break;
        }
    }
    assert!(
        admitted.iter().any(|id| id == "victim"),
        "new root arrivals starved an already pending root: {admitted:?}"
    );
    drop(waits);
    drop(held);
    assert_eq!(scheduler.snapshot().outstanding, 0);
}

#[tokio::test]
async fn two_old_resumers_cannot_alternate_forever_ahead_of_another_resumer() {
    let scheduler = scheduler();
    let mut parked = Vec::new();
    for id in ["old1", "old2", "victim"] {
        parked.push(
            reserve(&scheduler, id, "r")
                .acquire()
                .await
                .unwrap()
                .park()
                .unwrap(),
        );
    }
    let holder = reserve(&scheduler, "holder", "r").acquire().await.unwrap();
    let mut waits = BTreeMap::new();
    for ticket in parked {
        let id = ticket.run().work.id.clone();
        let mut wait: Wait = Box::pin(ticket.acquire());
        pending(&mut wait).await;
        waits.insert(id, wait);
    }
    holder.release();
    let mut admitted = Vec::new();
    for _ in 0..6 {
        let id = active(&scheduler);
        let permit = ready(waits.remove(&id).unwrap()).await;
        admitted.push(id.clone());
        if id == "victim" {
            // Drop pending futures before releasing the active permit.
            waits.clear();
            permit.release();
            break;
        }
        let mut wait: Wait = Box::pin(permit.park().unwrap().acquire());
        pending(&mut wait).await;
        waits.insert(id, wait);
    }
    assert!(
        admitted.iter().any(|id| id == "victim"),
        "original-age resume preference starved another accepted resumer: {admitted:?}"
    );
    assert_eq!(scheduler.snapshot().outstanding, 0);
}
