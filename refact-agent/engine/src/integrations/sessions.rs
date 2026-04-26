use std::{any::Any, sync::Arc};
use std::time::Duration;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use std::future::Future;

use crate::global_context::GlobalContext;

const STOP_SESSION_TIMEOUT: Duration = Duration::from_secs(5);

pub trait IntegrationSession: Any + Send + Sync {
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn is_expired(&self) -> bool;

    fn try_stop(
        &mut self,
        self_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    ) -> Box<dyn Future<Output = String> + Send>;
}

pub fn get_session_hashmap_key(integration_name: &str, base_key: &str) -> String {
    format!("{} ⚡ {}", integration_name, base_key)
}

async fn remove_expired_sessions(gcx: Arc<ARwLock<GlobalContext>>) {
    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .integration_sessions
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
        let mut gcx_locked = gcx.write().await;
        for (key, expired_session) in &expired_entries {
            let should_remove = gcx_locked
                .integration_sessions
                .get(key)
                .map(|current| Arc::ptr_eq(current, expired_session))
                .unwrap_or(false);
            if should_remove {
                gcx_locked.integration_sessions.remove(key);
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
    // sessions still keeps a reference on all sessions, just in case a destructor is called in the block above
}

pub async fn remove_expired_sessions_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
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

pub async fn stop_sessions(gcx: Arc<ARwLock<GlobalContext>>) {
    let sessions = {
        let mut gcx_locked = gcx.write().await;
        let sessions = gcx_locked
            .integration_sessions
            .iter()
            .map(|(_, session)| Arc::clone(session))
            .collect::<Vec<_>>();
        gcx_locked.integration_sessions.clear();
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
