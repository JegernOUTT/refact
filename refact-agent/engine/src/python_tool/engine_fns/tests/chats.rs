use super::*;
use crate::chat::types::ChatSession;

async fn persist(app: &AppState, session: &ChatSession) {
    crate::chat::trajectories::save_trajectory_snapshot(
        app.gcx.clone(),
        crate::chat::trajectories::trajectory_snapshot_from_session(session),
    )
    .await
    .unwrap();
}

fn other_chat(dir: &tempfile::TempDir, id: &str, zone: &str) -> ChatSession {
    let mut session = ChatSession::new(id.into());
    session.thread.title = format!("{id} title");
    session.thread.model = "model".into();
    session
        .messages
        .push(ChatMessage::new("user".into(), "read the file".into()));
    let mut tool = ChatMessage::new("tool".into(), "zone text".into());
    tool.tool_call_id = "t1".into();
    crate::privacy::records::attach_record(
        &mut tool,
        refact_privacy::FileRecord {
            path: dir
                .path()
                .join("project/zone_a.txt")
                .to_string_lossy()
                .into_owned(),
            zone: zone.into(),
            attribution: refact_privacy::Attribution::Declared,
        },
    );
    session.messages.push(tool);
    session.trajectory_version = 1;
    session
}

async fn insert(app: &AppState, session: ChatSession) {
    let id = session.chat_id.clone();
    app.chat
        .sessions
        .write()
        .await
        .insert(id, Arc::new(tokio::sync::Mutex::new(session)));
}

#[tokio::test]
async fn chats_live_reports_busy_and_skips_closed_and_internal() {
    let (_dir, app, _) = fixture().await;
    let mut generating = ChatSession::new("zone-live-a".into());
    generating.runtime.state = refact_chat_api::SessionState::Generating;
    insert(&app, generating).await;
    let mut closed = ChatSession::new("zone-live-closed".into());
    closed.closed = true;
    insert(&app, closed).await;
    let busy = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        "zone-live-busy".into(),
    )));
    app.chat
        .sessions
        .write()
        .await
        .insert("zone-live-busy".into(), busy.clone());
    let guard = busy.lock().await;
    let out = result(&run(&app, "(await engine('chats.live'))['data']").await);
    drop(guard);
    let rows = out["chats"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|r| r["id"] == id).cloned();
    assert_eq!(find("zone-live-a").unwrap()["state"], "generating", "{out}");
    assert_eq!(find("zone-live-busy").unwrap()["state"], "busy");
    assert!(find("zone-live-closed").is_none());
    assert_eq!(out["truncated"], false);
}

#[tokio::test]
async fn chats_list_pages_saved_chats_and_rejects_bad_cursors() {
    let (dir, app, _) = fixture().await;
    for id in ["zone-saved-a", "zone-saved-b"] {
        persist(&app, &other_chat(&dir, id, "normal")).await;
    }
    let m = run(
        &app,
        "p = (await engine('chats.list', limit=1, displayable_only=False))['data']\nbad = None\ntry:\n    await engine('chats.list', cursor='###')\nexcept TypeError as e:\n    bad = str(e)\n[p, bad]",
    )
    .await;
    let out = result(&m);
    assert!(out[0]["total_count"].as_u64().unwrap() >= 2, "{out}");
    assert_eq!(out[0]["items"].as_array().unwrap().len(), 1);
    assert_eq!(out[0]["has_more"], true);
    assert!(out[1].as_str().unwrap().contains("cursor"), "{out}");
}

#[tokio::test]
async fn chats_activity_returns_the_route_rows() {
    let (_dir, app, _) = fixture().await;
    let out = result(&run(&app, "(await engine('chats.activity'))['data']").await);
    let expected = serde_json::to_value(
        crate::http::routers::v1::chat_activity::activity(app.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(out, expected);
}

#[tokio::test]
async fn chats_read_carries_records_and_refuses_this_chat() {
    let (dir, app, _) = fixture().await;
    persist(&app, &other_chat(&dir, "zone-saved-read", "zone_secret")).await;
    let mut loaded = other_chat(&dir, "zone-loaded-read", "normal");
    loaded.work_project_root = Some(dir.path().join("project"));
    loaded.derived_privacy_zones.write().unwrap().insert(
        dir.path().join("project/derived.txt"),
        crate::privacy::records::DerivedZone {
            zone: "zone_derived".into(),
            origin: None,
        },
    );
    insert(&app, loaded).await;
    let m = run(
        &app,
        "a = (await engine('chats.read', chat_id='zone-saved-read'))['data']\nb = (await engine('chats.read', chat_id='zone-loaded-read'))['data']\nerrs = []\nfor cid in ['python-test', 'zone-missing-chat']:\n    try:\n        await engine('chats.read', chat_id=cid)\n        errs.append('ok')\n    except TypeError as e:\n        errs.append('TypeError')\n[a, b, errs]",
    )
    .await;
    let out = result(&m);
    assert_eq!(out[0]["loaded"], false, "{out}");
    assert_eq!(out[0]["messages"][1]["content"], "zone text");
    assert_eq!(out[1]["loaded"], true);
    assert_eq!(out[2][0], "TypeError");
    let files = m
        .extra
        .get("privacy")
        .map(|p| p["files"].clone())
        .unwrap_or_default()
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        files.iter().any(|f| f["zone"] == "zone_secret"),
        "{:?}",
        m.extra
    );
    assert!(
        files.iter().any(|f| f["zone"] == "zone_derived"),
        "{:?}",
        m.extra
    );
    let failed = run(
        &app,
        "(await engine('chats.read', chat_id='zone-missing-chat'))['ok']",
    )
    .await;
    assert_eq!(result(&failed), false);
}

#[tokio::test]
async fn chats_read_digest_pins_the_whole_history_and_its_file() {
    let (dir, app, _) = fixture().await;
    let mut chat = other_chat(&dir, "zone-digest", "normal");
    persist(&app, &chat).await;
    let read = |app: AppState, args: &'static str| async move {
        result(
            &run(
                &app,
                &format!(
                    "(await engine('chats.read', chat_id='zone-digest'{args}))['data']['digest']"
                ),
            )
            .await,
        )
    };
    let saved = read(app.clone(), "").await;
    assert!(saved.as_str().unwrap().starts_with("h1-"), "{saved}");
    assert_eq!(
        read(app.clone(), ", start=1, limit=1").await,
        saved,
        "a page of the history pins the whole history"
    );
    assert_eq!(read(app.clone(), "").await, saved, "the pin is stable");
    chat.messages[0].content =
        crate::call_validation::ChatContent::SimpleText("read it again".into());
    persist(&app, &chat).await;
    let changed = read(app.clone(), "").await;
    assert_ne!(changed, saved, "the pin follows the history");
    chat.work_project_root = Some(dir.path().join("project"));
    chat.messages[1]
        .extra
        .insert("privacy_ref".into(), json!("r"));
    insert(&app, chat).await;
    assert_ne!(
        read(app.clone(), "").await,
        changed,
        "the pin of a loaded chat covers every field of its messages"
    );
}
