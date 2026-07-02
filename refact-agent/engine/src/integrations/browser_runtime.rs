use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex as AMutex;
use tracing::{info, warn};

pub use refact_browser::*;

pub fn get_browser_profile_dir(gcx_cache_dir: &PathBuf, thread_id: &str) -> PathBuf {
    gcx_cache_dir.join("browser_profiles").join(thread_id)
}

pub async fn register_browser_runtime(
    app: crate::app_state::AppState,
    runtime: BrowserRuntime,
) -> String {
    let runtime_id = runtime.runtime_id.clone();
    let arc = Arc::new(AMutex::new(runtime));
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .insert(runtime_id.clone(), arc);
    runtime_id
}

pub async fn remove_browser_runtime(
    app: crate::app_state::AppState,
    runtime_id: &str,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .remove(runtime_id)
}

pub async fn find_runtime_by_chat_id(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    let runtime_arcs: Vec<(String, Arc<AMutex<BrowserRuntime>>)> = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes
            .iter()
            .map(|(rid, arc)| (rid.clone(), arc.clone()))
            .collect()
    };
    for (rid, arc) in runtime_arcs {
        let rt = arc.lock().await;
        if rt.attached_chat_id.as_deref() == Some(chat_id) {
            return Some((rid, arc.clone()));
        }
    }
    None
}

pub async fn browser_snapshot_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<crate::chat::types::BrowserSnapshot> {
    let (runtime_id, runtime_arc) = find_runtime_by_chat_id(app, chat_id).await?;
    let rt = runtime_arc.lock().await;
    let tabs = rt
        .list_tab_infos()
        .into_iter()
        .map(|t| crate::chat::types::BrowserTabInfo {
            tab_id: t.tab_id,
            url: t.url,
            title: t.title,
        })
        .collect::<Vec<_>>();
    let (url, title) = match rt.get_active_tab() {
        Some(tab) => (
            Some(tab.get_url()).filter(|s| !s.is_empty()),
            Some(tab.get_title().unwrap_or_default()).filter(|s| !s.is_empty()),
        ),
        None => (None, None),
    };
    Some(crate::chat::types::BrowserSnapshot {
        runtime_id,
        connected: rt.is_connected,
        active_tab: rt.active_tab_target_id().map(|s| s.to_string()),
        url,
        title,
        tabs,
    })
}

pub async fn browser_monitor_background_task(app: crate::app_state::AppState) {
    loop {
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(10)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {
                return;
            }
        }

        let runtime_ids: Vec<String> = {
            let browser_runtimes = app.integrations.browser_runtimes.clone();
            let browser_runtimes = browser_runtimes.lock().await;
            browser_runtimes.keys().cloned().collect()
        };

        let mut to_remove = Vec::new();
        for rid in &runtime_ids {
            let runtime_arc = {
                let browser_runtimes = app.integrations.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                match browser_runtimes.get(rid) {
                    Some(arc) => arc.clone(),
                    None => continue,
                }
            };

            let mut rt = runtime_arc.lock().await;

            let was_connected = rt.is_connected;
            let still_connected = rt.check_connection();

            if was_connected && !still_connected {
                info!(
                    "BrowserRuntime {} (chat {:?}) lost connection",
                    rt.runtime_id, rt.attached_chat_id
                );
            }

            if rt.attached_chat_id.is_some() && rt.is_idle_expired() {
                warn!(
                    "BrowserRuntime {} idle timeout ({:?}) for chat {:?}",
                    rt.runtime_id, rt.idle_timeout, rt.attached_chat_id
                );
                to_remove.push(rid.clone());
            }

            if !still_connected && rt.attached_chat_id.is_none() {
                to_remove.push(rid.clone());
            }
        }

        for rid in to_remove {
            remove_browser_runtime(app.clone(), &rid).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_get_browser_profile_dir() {
        let cache_dir = PathBuf::from("/tmp/refact-cache");
        let profile = get_browser_profile_dir(&cache_dir, "thread-abc-123");
        assert_eq!(
            profile,
            PathBuf::from("/tmp/refact-cache/browser_profiles/thread-abc-123")
        );
    }

    #[test]
    fn test_get_browser_profile_dir_different_threads() {
        let cache_dir = PathBuf::from("/home/user/.cache/refact");
        let p1 = get_browser_profile_dir(&cache_dir, "thread-1");
        let p2 = get_browser_profile_dir(&cache_dir, "thread-2");
        assert_ne!(p1, p2);
        assert!(p1.to_str().unwrap().contains("thread-1"));
        assert!(p2.to_str().unwrap().contains("thread-2"));
    }
}
