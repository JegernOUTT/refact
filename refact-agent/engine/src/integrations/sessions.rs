pub use refact_integrations::sessions::{IntegrationSession, get_session_hashmap_key};

use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use std::time::Duration;

use crate::global_context::GlobalContext;

const STOP_SESSION_TIMEOUT: Duration = Duration::from_secs(5);

async fn remove_expired_sessions(gcx: Arc<GlobalContext>) {
    let sessions = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions
            .iter()
            .map(|(key, session)| (key.to_string(), session.clone()))
            .collect::<Vec<_>>()
    };

    let mut expired_entries: Vec<(String, Arc<AMutex<Box<dyn IntegrationSession>>>)> = Vec::new();
    for (key, session) in &sessions {
        let is_expired = {
            let session_locked = session.lock().await;
            session_locked.is_expired()
        };
        if is_expired {
            expired_entries.push((key.clone(), session.clone()));
        }
    }

    if !expired_entries.is_empty() {
        let integration_sessions = gcx.integration_sessions.clone();
        let mut integration_sessions = integration_sessions.lock().await;
        for (key, expired_session) in &expired_entries {
            let should_remove = integration_sessions
                .get(key)
                .map(|current| Arc::ptr_eq(current, expired_session))
                .unwrap_or(false);
            if should_remove {
                integration_sessions.remove(key);
            }
        }
    }

    let mut futures = Vec::new();
    for (_, session) in expired_entries {
        let future = {
            let mut session_locked = session.lock().await;
            session_locked.try_stop(session.clone())
        };
        let future = Box::into_pin(future);
        futures.push(future);
    }
    futures::future::join_all(futures).await;
}

pub async fn remove_expired_sessions_background_task(gcx: Arc<GlobalContext>) {
    loop {
        let shutdown_flag = gcx.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Session expiry: shutdown detected, stopping");
                return;
            }
        }
        remove_expired_sessions(gcx.clone()).await;
    }
}

pub async fn stop_sessions(gcx: Arc<GlobalContext>) {
    let sessions = {
        let integration_sessions = gcx.integration_sessions.clone();
        let mut integration_sessions = integration_sessions.lock().await;
        let sessions = integration_sessions
            .iter()
            .map(|(_, session)| Arc::clone(session))
            .collect::<Vec<_>>();
        integration_sessions.clear();
        sessions
    };
    let mut futures = Vec::new();
    for session in sessions {
        let future = Box::into_pin(session.lock().await.try_stop(session.clone()));
        futures.push(tokio::time::timeout(STOP_SESSION_TIMEOUT, future));
    }
    let results = futures::future::join_all(futures).await;
    for result in results {
        if result.is_err() {
            tracing::warn!(
                "stop_sessions: a session did not stop within {:?}, continuing shutdown",
                STOP_SESSION_TIMEOUT
            );
        }
    }
}
