use std::sync::atomic::Ordering;

use super::chats_risky::{
    approve, build, deny, exists, message, moved_then_allowed, other, refused, save, until,
};
use super::*;
use crate::chat::types::ChatSession;

async fn loaded(app: &AppState, id: &str) -> bool {
    app.chat.sessions.read().await.contains_key(id)
}

#[tokio::test]
async fn delete_closes_and_deletes_a_loaded_idle_chat() {
    let (_dir, app, _) = fixture().await;
    drop(other(&app, "zone-dl", |_| {}).await);
    let (request, out) = approve(
        &app,
        "(await engine('chats.delete', chat_id='zone-dl'))['data']",
    )
    .await;
    let line = request.engine.unwrap().items[0]
        .old
        .clone()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(line.contains("open in the engine"), "{line}");
    assert_eq!(out, json!({"chat_id":"zone-dl","deleted":true}));
    assert!(!loaded(&app, "zone-dl").await, "the chat stayed open");
    assert!(!exists(&app, "zone-dl").await);
}

#[tokio::test]
async fn delete_closes_an_idle_chat_with_a_parked_queue_processor() {
    let (_dir, app, _) = fixture().await;
    let viewer = other(&app, "zone-dq", |_| {}).await;
    let running = viewer.lock().await.queue_processor_running.clone();
    running.store(true, Ordering::SeqCst);
    let worker = tokio::spawn(crate::chat::process_command_queue(
        app.clone(),
        viewer.clone(),
        running,
    ));
    until(&viewer, |s| {
        s.queue_processor_counters
            .empty_locks
            .load(Ordering::Relaxed)
            > 0
    })
    .await;

    assert_eq!(
        refused(&app, "await engine('chats.delete', chat_id='zone-dq')").await,
        "PermissionError"
    );
    drop(viewer);

    let (_, out) = approve(
        &app,
        "(await engine('chats.delete', chat_id='zone-dq'))['data']",
    )
    .await;
    assert_eq!(out, json!({"chat_id":"zone-dq","deleted":true}));
    assert!(!loaded(&app, "zone-dq").await);
    assert!(!exists(&app, "zone-dq").await);
    tokio::time::timeout(Duration::from_secs(20), worker)
        .await
        .expect("the queue processor did not exit after deletion")
        .unwrap();
}

#[tokio::test]
async fn a_chat_open_in_a_viewer_is_refused_and_kept() {
    let (_dir, app, _) = fixture().await;
    let viewer = other(&app, "zone-dv", |_| {}).await;
    assert_eq!(
        refused(&app, "await engine('chats.delete', chat_id='zone-dv')").await,
        "PermissionError"
    );
    assert!(loaded(&app, "zone-dv").await);
    assert!(exists(&app, "zone-dv").await);
    drop(viewer);
}

#[tokio::test]
async fn cascade_deletes_the_subtree_children_first_and_lists_every_chat() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-dc", |_| {})).await;
    save(
        &app,
        &build("zone-dc-child", |s| {
            s.thread.parent_id = Some("zone-dc".into())
        }),
    )
    .await;
    drop(
        other(&app, "zone-dc-grandchild", |s| {
            s.thread.parent_id = Some("zone-dc-child".into())
        })
        .await,
    );
    let tree = ["zone-dc-grandchild", "zone-dc-child", "zone-dc"];
    assert_eq!(
        refused(&app, "await engine('chats.delete', chat_id='zone-dc')").await,
        "PermissionError",
        "children without cascade stay refused"
    );
    let engine = deny(
        &app,
        "await engine('chats.delete', chat_id='zone-dc', cascade=True)",
    )
    .await
    .engine
    .unwrap();
    let labels: Vec<&str> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "chat zone-dc-grandchild",
            "chat zone-dc-child",
            "chat zone-dc"
        ]
    );
    assert!(
        engine.warning.clone().unwrap().starts_with("These 3 chats"),
        "{:?}",
        engine.warning
    );
    for id in tree {
        assert!(exists(&app, id).await, "deny deleted {id}");
    }
    let (_, out) = approve(
        &app,
        "(await engine('chats.delete', chat_id='zone-dc', cascade=True))['data']",
    )
    .await;
    assert_eq!(out["chats"], json!(tree), "{out}");
    for id in tree {
        assert!(!exists(&app, id).await, "{id} was kept");
    }
    assert!(!loaded(&app, "zone-dc-grandchild").await);
}

#[tokio::test]
async fn a_tree_with_a_chat_it_must_not_delete_is_refused_whole() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-dt", |_| {})).await;
    save(
        &app,
        &build("zone-dt-free", |s| {
            s.thread.parent_id = Some("zone-dt".into())
        }),
    )
    .await;
    let task_child = other(&app, "zone-dt-task", |s| {
        s.thread.parent_id = Some("zone-dt".into())
    })
    .await;
    task_child.lock().await.thread.task_meta =
        Some(serde_json::from_value(json!({"task_id":"t","role":"agent"})).unwrap());
    drop(task_child);
    assert_eq!(
        refused(
            &app,
            "await engine('chats.delete', chat_id='zone-dt', cascade=True)"
        )
        .await,
        "PermissionError"
    );
    for id in ["zone-dt", "zone-dt-free", "zone-dt-task"] {
        assert!(exists(&app, id).await, "{id}");
    }

    save(&app, &build("zone-da", |_| {})).await;
    session(&app).await.lock().await.thread.parent_id = Some("zone-da".into());
    assert_eq!(
        refused(
            &app,
            "await engine('chats.delete', chat_id='zone-da', cascade=True)"
        )
        .await,
        "PermissionError",
        "an ancestor of this chat"
    );
    assert!(exists(&app, "zone-da").await);
}

#[tokio::test]
async fn a_child_added_between_card_and_answer_is_refused() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-dm", |_| {})).await;
    let app2 = app.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.delete', chat_id='zone-dm', cascade=True)",
        async move {
            save(
                &app2,
                &build("zone-dm-late", |s| {
                    s.thread.parent_id = Some("zone-dm".into())
                }),
            )
            .await;
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    assert!(exists(&app, "zone-dm").await && exists(&app, "zone-dm-late").await);
}

/// K4: each member is checked again right before it is deleted; the first one that changed stops
/// the delete, and the error lists what was deleted and what was kept.
#[tokio::test]
async fn a_member_that_changes_during_the_delete_stops_it() {
    let (_dir, app, _) = fixture().await;
    drop(other(&app, "zone-ds", |_| {}).await);
    save(
        &app,
        &build("zone-ds-child", |s| {
            s.thread.parent_id = Some("zone-ds".into())
        }),
    )
    .await;
    let hook: fn(&mut ChatSession) = |s| {
        s.messages
            .push(message("zone-ds-late", "user", "zone late"))
    };
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .push(("delete:zone-ds".into(), hook));
    let (_, out) = approve(
        &app,
        "r = await engine('chats.delete', chat_id='zone-ds', cascade=True)\n[r['ok'], r.get('error')]",
    )
    .await;
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .retain(|(key, _)| key != "delete:zone-ds");
    assert_eq!(out[0], false, "{out}");
    let error = out[1].as_str().unwrap();
    assert!(
        error.contains("deleted: zone-ds-child")
            && error.contains("stopped at chat zone-ds: it changed since the preview")
            && error.contains("kept: zone-ds"),
        "{error}"
    );
    assert!(!exists(&app, "zone-ds-child").await);
    assert!(exists(&app, "zone-ds").await);
    assert!(loaded(&app, "zone-ds").await);
}

/// K4: a chat saved twice is ambiguous, so it is refused instead of deleting one copy.
#[tokio::test]
async fn a_chat_saved_twice_is_refused() {
    let (dir, app, _) = fixture().await;
    save(&app, &build("zone-dd", |_| {})).await;
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-dd")
        .await
        .unwrap();
    let copy = dir
        .path()
        .join("project/.refact/trajectories/zone-dd/zone-dd.json");
    assert_ne!(copy, path);
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&path, &copy).unwrap();
    assert_eq!(
        refused(&app, "await engine('chats.delete', chat_id='zone-dd')").await,
        "PermissionError"
    );
    assert!(path.exists() && copy.exists());
}

fn delete_gate(id: &str) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
    let (entered, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    crate::python_tool::engine_fns::chats::DELETE_GATE
        .lock()
        .unwrap()
        .push((id.into(), entered.clone(), release.clone()));
    (entered, release)
}

fn open_delete_gate(release: &tokio::sync::Notify) {
    release.notify_one();
}

async fn approved_delete_at_gate(
    app: &AppState,
    id: &str,
) -> (
    tokio::task::JoinHandle<ChatMessage>,
    Arc<tokio::sync::Notify>,
) {
    let (entered, release) = delete_gate(id);
    let task = spawn(
        app,
        &format!("r = await engine('chats.delete', chat_id='{id}')\n[r['ok'], r['data'], r.get('error')]"),
    );
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), entered.notified())
        .await
        .expect("the delete did not reach its last check");
    (task, release)
}

/// T2: from its last check to its unlink a member is reserved: a viewer, a command, a restore
/// or a branch cannot open it.
#[tokio::test]
async fn a_chat_being_deleted_cannot_be_opened_or_restored() {
    let (_dir, app, _) = fixture().await;
    drop(other(&app, "zone-dr", |_| {}).await);
    let (task, release) = approved_delete_at_gate(&app, "zone-dr").await;
    let sessions = app.chat.sessions.clone();
    let opened = crate::chat::session_runtime::get_or_create_session_with_trajectory(
        app.clone(),
        &sessions,
        "zone-dr",
    )
    .await;
    assert!(
        matches!(
            opened,
            Err(crate::chat::session_runtime::SessionOpenError::Busy)
        ),
        "a viewer or a command opened a chat that is being deleted"
    );
    assert!(matches!(
        crate::chat::session_runtime::restore_session_if_trajectory_exists(
            app.clone(),
            &sessions,
            "zone-dr"
        )
        .await,
        crate::chat::session_runtime::SessionRestore::Refused(_)
    ));
    open_delete_gate(&release);
    let out = result(&finish(task).await);
    assert_eq!(out[0], true, "{out}");
    assert!(!exists(&app, "zone-dr").await);
    assert!(!loaded(&app, "zone-dr").await);
}

/// T2: the final close uses the C25 idle gate and the real viewer count, so a wake or a viewer
/// that appears after the last check keeps the chat.
#[tokio::test]
async fn the_close_of_a_delete_refuses_a_wake_or_a_viewer_that_came_after_its_check() {
    let (_dir, app, _) = fixture().await;
    for (id, wake) in [("zone-dw", true), ("zone-dh", false)] {
        drop(other(&app, id, |_| {}).await);
        let (task, release) = approved_delete_at_gate(&app, id).await;
        let session = app.chat.sessions.read().await[id].clone();
        let viewer = if wake {
            session
                .lock()
                .await
                .delivery_wake_sources
                .insert("external");
            None
        } else {
            Some(session.lock().await.subscribe())
        };
        drop(session);
        open_delete_gate(&release);
        let out = result(&finish(task).await);
        assert_eq!(out[0], false, "{id}: {out}");
        assert_eq!(out[1]["chats"][0]["deleted"], false, "{id}: {out}");
        assert!(
            out[2].as_str().unwrap().contains("could not be closed"),
            "{id}: {out}"
        );
        assert!(exists(&app, id).await, "{id}");
        assert!(loaded(&app, id).await, "{id}");
        drop(viewer);
    }
}

/// T2: the unlink is bound to the file the last check saw: a file another engine replaced
/// meanwhile is kept, and a workspace deleted before the file changed is reported as such.
#[tokio::test]
async fn a_delete_never_unlinks_a_file_that_changed_after_its_check() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-du", |_| {})).await;
    let (task, release) = approved_delete_at_gate(&app, "zone-du").await;
    let mut newer = build("zone-du", |_| {});
    newer
        .messages
        .push(message("zone-du-late", "user", "zone written elsewhere"));
    save(&app, &newer).await;
    open_delete_gate(&release);
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    let row = &out[1]["chats"][0];
    assert_eq!(row["deleted"], false, "{out}");
    assert_eq!(row["workspace_deleted"], false, "{out}");
    assert!(
        row["reason"].as_str().unwrap().contains("changed on disk"),
        "{out}"
    );
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-du")
        .await
        .unwrap();
    assert_eq!(saved.messages.len(), 6, "the newer file was deleted");

    save(&app, &build("zone-dp", |_| {})).await;
    let (deleted, gone) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    crate::chat::trajectories::test_workspace_deleted_hooks()
        .lock()
        .unwrap()
        .insert("zone-dp".into(), (deleted.clone(), gone.clone()));
    let task = spawn(
        &app,
        "r = await engine('chats.delete', chat_id='zone-dp')\n[r['ok'], r['data'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), deleted.notified())
        .await
        .expect("the delete did not reach its workspace");
    let mut newer = build("zone-dp", |_| {});
    newer.thread.title = "zone renamed meanwhile".into();
    save(&app, &newer).await;
    gone.notify_one();
    let m = finish(task).await;
    let out = result(&m);
    assert_eq!(out[0], false, "{out}");
    let row = &out[1]["chats"][0];
    assert_eq!(row["workspace_deleted"], true, "{out}");
    assert!(
        row["reason"]
            .as_str()
            .unwrap()
            .contains("Python workspace was deleted"),
        "{out}"
    );
    assert!(exists(&app, "zone-dp").await);
    assert_eq!(
        m.extra["python"]["engine_changes"][0]["items"][0]["new"],
        "its Python workspace deleted, its file kept",
        "{}",
        m.extra["python"]
    );
}

/// T4: a cascade that stops midway is a result: ok false, every member listed as deleted or kept
/// with its reason, and the deleted ones recorded as the change this run made.
#[tokio::test]
async fn a_stopped_cascade_lists_every_chat_and_records_what_it_deleted() {
    let (_dir, app, _) = fixture().await;
    drop(other(&app, "zone-dx", |_| {}).await);
    save(
        &app,
        &build("zone-dx-child", |s| {
            s.thread.parent_id = Some("zone-dx".into())
        }),
    )
    .await;
    let hook: fn(&mut ChatSession) = |s| {
        s.messages
            .push(message("zone-dx-late", "user", "zone late"))
    };
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .push(("delete:zone-dx".into(), hook));
    let task = spawn(
        &app,
        "r = await engine('chats.delete', chat_id='zone-dx', cascade=True)\n[r['ok'], r['data'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .retain(|(key, _)| key != "delete:zone-dx");
    let out = result(&m);
    assert_eq!(out[0], false, "{out}");
    assert_eq!(
        out[1]["chats"],
        json!([
            {"id":"zone-dx-child","deleted":true},
            {"id":"zone-dx","deleted":false,"reason":"it changed since the preview","workspace_deleted":false}
        ]),
        "{out}"
    );
    assert!(
        out[2]
            .as_str()
            .unwrap()
            .contains("deleted: zone-dx-child; stopped at chat zone-dx"),
        "{out}"
    );
    let changes = &m.extra["python"]["engine_changes"];
    assert_eq!(m.extra["python"]["engine_changes_total"], 1, "{changes}");
    let items = changes[0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{changes}");
    assert_eq!(items[0]["label"], "chat zone-dx-child");
    assert_eq!(items[0]["new"], "deleted");
    assert!(!exists(&app, "zone-dx-child").await);
    assert!(exists(&app, "zone-dx").await);
}
