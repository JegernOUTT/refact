use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_state::AppState;

#[derive(Clone, Serialize, Deserialize)]
pub struct EngineIdentity {
    pub schema_version: u32,
    pub instance_id: String,
    pub pid: u32,
    pub managed_instance_digest: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LiveChat {
    pub id: String,
    pub title: String,
    pub mode: String,
    pub session_state: String,
    pub paused: bool,
    pub project_root: Option<String>,
    pub updated_at_ms: u64,
    pub kind: String,
}

#[derive(Serialize, Deserialize)]
pub struct LiveSnapshot {
    pub schema_version: u32,
    pub instance_id: String,
    pub coverage: String,
    pub observed_at_ms: u64,
    pub chats: Vec<LiveChat>,
}

pub(crate) fn observation_capability(secret: &str, instance_digest: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<sha2_observation::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts arbitrary keys");
    mac.update(b"refact-engine-observation-v1\0");
    mac.update(instance_digest.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn instance_digest(app: &AppState) -> String {
    format!(
        "{:x}",
        Sha256::digest(app.gcx.cmdline.ping_message.as_bytes())
    )
}

fn guard(
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: &HeaderMap,
    token: Option<&str>,
    port: u16,
) -> Result<(), StatusCode> {
    if !peer.is_some_and(|ConnectInfo(addr)| addr.ip().is_loopback())
        || headers.contains_key(header::ORIGIN)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let valid_host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<hyper::http::uri::Authority>().ok())
        .is_some_and(|a| matches!(a.host(), "127.0.0.1" | "[::1]") && a.port_u16() == Some(port));
    if !valid_host || headers.get("sec-fetch-site").is_some_and(|v| v != "none") {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(token) = token.filter(|t| !t.is_empty()) {
        if !headers
            .get("x-refact-observation-token")
            .and_then(|v| v.to_str().ok())
            .filter(|_| headers.get_all("x-refact-observation-token").iter().count() == 1)
            .is_some_and(|value| crate::daemon::auth::token_matches(value, token))
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(())
}

pub async fn identity(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Result<Json<EngineIdentity>, StatusCode> {
    let capability = app
        .gcx
        .cmdline
        .daemon_auth_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|secret| observation_capability(secret, &instance_digest(&app)));
    guard(
        peer,
        &headers,
        capability.as_deref(),
        app.gcx.cmdline.http_port,
    )?;
    Ok(Json(EngineIdentity {
        schema_version: 1,
        instance_id: super::buddy_native_approvals::native_engine_identity().into(),
        pid: std::process::id(),
        managed_instance_digest: (!app.gcx.cmdline.project_id.is_empty()).then(|| {
            format!(
                "{:x}",
                Sha256::digest(app.gcx.cmdline.ping_message.as_bytes())
            )
        }),
    }))
}

pub(crate) fn project(session: &crate::chat::types::ChatSession, now: u64) -> Option<LiveChat> {
    if session.closed {
        return None;
    }
    let kind = if session.thread.buddy_meta.is_some()
        || session.thread.mode.eq_ignore_ascii_case("buddy")
    {
        "buddy"
    } else if crate::chat::trajectories::is_internal_trace_link_type(
        session.thread.link_type.as_deref(),
    ) {
        "internal"
    } else if session.thread.parent_id.is_some() {
        "subagent"
    } else {
        "chat"
    };
    Some(LiveChat {
        id: session.chat_id.clone(),
        title: session.thread.title.chars().take(512).collect(),
        mode: session.thread.mode.chars().take(64).collect(),
        session_state: session.runtime.state.to_string(),
        paused: session.runtime.paused,
        project_root: session
            .work_project_root
            .as_ref()
            .map(|p| p.to_string_lossy().chars().take(4096).collect()),
        updated_at_ms: now.saturating_sub(
            session
                .last_activity
                .elapsed()
                .as_millis()
                .min(u64::MAX as u128) as u64,
        ),
        kind: kind.into(),
    })
}

pub async fn live(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Result<Json<LiveSnapshot>, StatusCode> {
    let capability = app
        .gcx
        .cmdline
        .daemon_auth_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|secret| observation_capability(secret, &instance_digest(&app)));
    guard(
        peer,
        &headers,
        capability.as_deref(),
        app.gcx.cmdline.http_port,
    )?;
    let loaded = {
        let sessions = app
            .chat
            .sessions
            .try_read()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if sessions.len() > 500 {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        sessions.values().cloned().collect::<Vec<_>>()
    };
    let now = crate::daemon::state::now_ms();
    let mut chats = Vec::new();
    for session in loaded {
        let session = session
            .try_lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if session.chat_id.len() > 1024 {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        if let Some(chat) = project(&session, now) {
            chats.push(chat);
        }
    }
    chats.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(LiveSnapshot {
        schema_version: 1,
        instance_id: super::buddy_native_approvals::native_engine_identity().into(),
        coverage: "loaded_sessions".into(),
        observed_at_ms: now,
        chats,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn live_handler_reads_only_loaded_sessions_and_refuses_contention() {
        use crate::chat::types::ChatSession;
        use refact_chat_api::SessionState;
        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        Arc::get_mut(&mut gcx).unwrap().cmdline.http_port = 18799;
        let app = AppState::from_gcx(gcx).await;
        let states = [
            SessionState::Idle,
            SessionState::Generating,
            SessionState::ExecutingTools,
            SessionState::WaitingIde,
            SessionState::WaitingUserInput,
            SessionState::Paused,
        ];
        let mut original = Vec::new();
        for (index, state) in states.into_iter().enumerate() {
            let id = format!("loaded-{index}");
            let mut session = ChatSession::new(id.clone());
            session.runtime.state = state;
            session.thread.title = format!("Loaded {index}");
            original.push((id.clone(), session.event_seq, session.last_activity));
            app.chat
                .sessions
                .write()
                .await
                .insert(id, Arc::new(tokio::sync::Mutex::new(session)));
        }
        let mut closed = ChatSession::new("closed".into());
        closed.closed = true;
        app.chat
            .sessions
            .write()
            .await
            .insert("closed".into(), Arc::new(tokio::sync::Mutex::new(closed)));
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:18799".parse().unwrap());
        let peer = || Some(ConnectInfo("127.0.0.1:42".parse().unwrap()));
        let snapshot = live(State(app.clone()), peer(), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(
            snapshot
                .chats
                .iter()
                .map(|c| (&*c.id, &*c.session_state))
                .collect::<Vec<_>>(),
            vec![
                ("loaded-0", "idle"),
                ("loaded-1", "generating"),
                ("loaded-2", "executing_tools"),
                ("loaded-3", "waiting_ide"),
                ("loaded-4", "waiting_user_input"),
                ("loaded-5", "paused")
            ]
        );
        let sessions = app.chat.sessions.read().await;
        assert_eq!(sessions.len(), 7);
        for (id, seq, activity) in original {
            let session = sessions[&id].lock().await;
            assert_eq!(session.event_seq, seq);
            assert_eq!(session.last_activity, activity);
        }
        let busy = sessions["loaded-0"].clone();
        drop(sessions);
        let _busy = busy.lock().await;
        assert_eq!(
            live(State(app.clone()), peer(), headers.clone())
                .await
                .err(),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
        drop(_busy);
        let _writing = app.chat.sessions.write().await;
        assert_eq!(
            live(State(app.clone()), peer(), headers).await.err(),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    #[test]
    fn observation_capability_is_instance_scoped_and_not_a_bearer() {
        let secret = "installation-secret";
        let capability = observation_capability(secret, "spawn-one");
        assert_eq!(capability.len(), 64);
        assert_ne!(capability, secret);
        assert_ne!(capability, observation_capability(secret, "spawn-two"));
        assert_ne!(
            capability,
            observation_capability("other-secret", "spawn-one")
        );
        let peer = || Some(ConnectInfo("127.0.0.1:42".parse().unwrap()));
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8001".parse().unwrap());
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {secret}").parse().unwrap(),
        );
        assert_eq!(
            guard(peer(), &headers, Some(&capability), 8001),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert("x-refact-observation-token", capability.parse().unwrap());
        assert!(guard(peer(), &headers, Some(&capability), 8001).is_ok());
        for host in ["evil.example:8001", "localhost:8001", "127.0.0.1:8002"] {
            headers.insert(header::HOST, host.parse().unwrap());
            assert_eq!(
                guard(peer(), &headers, None, 8001),
                Err(StatusCode::FORBIDDEN)
            );
        }
        headers.insert(header::HOST, "127.0.0.1:8001".parse().unwrap());
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert_eq!(
            guard(peer(), &headers, None, 8001),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn passive_projection_retains_states_identity_and_excludes_payloads() {
        use refact_chat_api::SessionState;
        let mut session = crate::chat::types::ChatSession::new("x".repeat(300));
        session.thread.title = "<img src=x onerror=alert(1)>".into();
        session.thread.mode = "task_agent".into();
        let seq = session.event_seq;
        for (state, expected) in [
            (SessionState::Idle, "idle"),
            (SessionState::Generating, "generating"),
            (SessionState::ExecutingTools, "executing_tools"),
            (SessionState::WaitingIde, "waiting_ide"),
            (SessionState::WaitingUserInput, "waiting_user_input"),
            (SessionState::Paused, "paused"),
            (SessionState::Completed, "completed"),
            (SessionState::Error, "error"),
            (SessionState::Interrupted, "interrupted"),
        ] {
            session.runtime.state = state;
            let row = project(&session, 1000).unwrap();
            assert_eq!(row.session_state, expected);
            assert_eq!(row.id, session.chat_id);
            assert_eq!(row.kind, "chat");
            let value = serde_json::to_value(row).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 8);
            assert_eq!(value["title"], session.thread.title);
            assert!(value.get("messages").is_none());
            assert_eq!(session.event_seq, seq);
        }
        session.thread.parent_id = Some("parent".into());
        assert_eq!(project(&session, 1000).unwrap().kind, "subagent");
        session.thread.link_type = Some("internal:title_generation".into());
        assert_eq!(project(&session, 1000).unwrap().kind, "internal");
        session.thread.mode = "buddy".into();
        assert_eq!(project(&session, 1000).unwrap().kind, "buddy");
        session.closed = true;
        assert!(project(&session, 1000).is_none());
    }
    #[test]
    fn observation_guard_requires_peer_and_rejects_all_browser_origins() {
        let peer = || Some(ConnectInfo("127.0.0.1:1234".parse().unwrap()));
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8001".parse().unwrap());
        assert_eq!(
            guard(None, &headers, None, 8001),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            guard(peer(), &headers, Some("secret"), 8001),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert("x-refact-observation-token", "secret".parse().unwrap());
        assert!(guard(peer(), &headers, Some("secret"), 8001).is_ok());
        headers.insert(header::ORIGIN, "http://127.0.0.1:8001".parse().unwrap());
        assert_eq!(
            guard(peer(), &headers, Some("secret"), 8001),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
