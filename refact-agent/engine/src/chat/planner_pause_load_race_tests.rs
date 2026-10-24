use std::sync::{Arc, Mutex};

use tokio::sync::Barrier;

use super::types::{ChatSession, PlannerPause, SessionState};
use crate::tasks::board_pause_tests::{fixture, pause, resume};
use crate::tasks::storage;

tokio::task_local! {
    static LOAD_BARRIER: Arc<LoadBarrier>;
}

struct LoadBarrier {
    barrier: Barrier,
    reconcile_barrier: Option<Barrier>,
    pause_snapshot: Mutex<Option<Option<PlannerPause>>>,
}

pub(super) async fn before_session_insert(session: &ChatSession) {
    let Ok(hook) = LOAD_BARRIER.try_with(Arc::clone) else {
        return;
    };
    *hook.pause_snapshot.lock().unwrap() = Some(session.planner_pause.clone());
    hook.barrier.wait().await;
    hook.barrier.wait().await;
}

pub(super) async fn after_reconcile_board_read() {
    let Ok(hook) = LOAD_BARRIER.try_with(Arc::clone) else {
        return;
    };
    if let Some(barrier) = &hook.reconcile_barrier {
        barrier.wait().await;
        barrier.wait().await;
    }
}

async fn load_during_lifecycle_change(
    initially_paused: bool,
    change_during_reconcile: bool,
) -> (Option<PlannerPause>, Option<PlannerPause>) {
    let f = fixture(SessionState::Idle).await;
    if initially_paused {
        pause(&f).await;
    }
    let (chat_id, meta) = {
        let mut session = f.agent.lock().await;
        session.increment_version();
        (
            session.chat_id.clone(),
            session.thread.task_meta.clone().unwrap(),
        )
    };
    crate::chat::trajectories::maybe_save_trajectory(f.app.clone(), f.agent.clone()).await;
    let trajectory =
        crate::chat::trajectories::load_trajectory_for_chat(f.app.gcx.clone(), &chat_id)
            .await
            .expect("fixture must persist the agent trajectory before loading");
    assert_eq!(trajectory.thread.task_meta, Some(meta.clone()));
    f.app.chat.sessions.write().await.remove(&chat_id);

    let hook = Arc::new(LoadBarrier {
        barrier: Barrier::new(2),
        reconcile_barrier: change_during_reconcile.then(|| Barrier::new(2)),
        pause_snapshot: Mutex::new(None),
    });
    let loader = tokio::spawn({
        let app = f.app.clone();
        let chat_id = chat_id.clone();
        let hook = hook.clone();
        async move {
            LOAD_BARRIER
                .scope(hook, async move {
                    crate::chat::get_or_create_session_with_trajectory(
                        app.clone(),
                        &app.chat.sessions,
                        &chat_id,
                    )
                    .await
                })
                .await
        }
    });
    hook.barrier.wait().await;
    assert_eq!(
        hook.pause_snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_some(),
        initially_paused,
    );
    assert!(!f.app.chat.sessions.read().await.contains_key(&chat_id));
    if initially_paused {
        resume(&f).await;
    } else {
        pause(&f).await;
    }
    assert!(!f.app.chat.sessions.read().await.contains_key(&chat_id));
    hook.barrier.wait().await;

    if let Some(barrier) = &hook.reconcile_barrier {
        barrier.wait().await;
        assert!(f.app.chat.sessions.read().await.contains_key(&chat_id));
        if initially_paused {
            pause(&f).await;
        } else {
            resume(&f).await;
        }
        barrier.wait().await;
    }

    let loaded = loader.await.unwrap();
    assert!(Arc::ptr_eq(
        f.app.chat.sessions.read().await.get(&chat_id).unwrap(),
        &loaded,
    ));
    let board = storage::load_board(f.app.gcx.clone(), &meta.task_id)
        .await
        .unwrap();
    let card = board.get_card(meta.card_id.as_deref().unwrap()).unwrap();
    let persisted = card.paused_at.as_ref().map(|since| PlannerPause {
        since: since.clone(),
        by: card.paused_by.clone(),
    });
    let session = loaded.lock().await;
    (session.planner_pause.clone(), persisted)
}

#[tokio::test]
async fn resume_during_session_load_does_not_leave_agent_gated() {
    let (live, persisted) = load_during_lifecycle_change(true, false).await;
    assert!(persisted.is_none(), "resume must clear persisted paused_at");
    assert!(
        live.is_none(),
        "resumed card left the loaded agent planner-paused"
    );
}

#[tokio::test]
async fn pause_during_session_load_gates_the_loaded_session() {
    let (live, persisted) = load_during_lifecycle_change(false, false).await;
    let persisted = persisted.expect("pause must persist paused_at");
    assert_eq!(persisted.by.as_deref(), Some("planner-test"));
    assert_eq!(
        live,
        Some(persisted),
        "loaded agent must match the card's planner pause"
    );
}

#[tokio::test]
async fn resume_during_reconcile_does_not_restore_a_stale_pause() {
    let (live, persisted) = load_during_lifecycle_change(false, true).await;
    assert!(persisted.is_none());
    assert!(live.is_none(), "reconcile overwrote a newer live resume");
}

#[tokio::test]
async fn pause_during_reconcile_is_not_cleared_by_a_stale_board_read() {
    let (live, persisted) = load_during_lifecycle_change(true, true).await;
    let persisted = persisted.expect("pause must persist paused_at");
    assert_eq!(persisted.by.as_deref(), Some("planner-test"));
    assert_eq!(
        live,
        Some(persisted),
        "reconcile overwrote a newer live pause"
    );
}
