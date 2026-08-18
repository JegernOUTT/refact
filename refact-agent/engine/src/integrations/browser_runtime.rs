use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use tokio::sync::Mutex as AMutex;
use tracing::{info, warn};

pub use refact_browser::*;

pub fn get_browser_profile_dir(gcx_cache_dir: &PathBuf, thread_id: &str) -> PathBuf {
    gcx_cache_dir.join("browser_profiles").join(thread_id)
}

fn workspace_roots(app: &crate::app_state::AppState) -> Vec<PathBuf> {
    app.gcx
        .documents_state
        .workspace_folders
        .lock()
        .map(|folders| folders.clone())
        .unwrap_or_default()
}

pub async fn register_browser_runtime(
    app: crate::app_state::AppState,
    runtime: BrowserRuntime,
) -> String {
    let mut runtime = runtime;
    runtime.set_allowed_roots(workspace_roots(&app));
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

pub const RELAUNCH_SETTLE: Duration = Duration::from_millis(800);

pub const RELAUNCH_WARNING: &str =
    "browser session was dead; relaunched and retried (open tabs lost, cookies/localStorage kept)";

pub fn relaunch_resume_warning(resume_index: usize) -> String {
    format!(
        "browser session was dead; relaunched and resumed from step {resume_index} — open tabs were lost, cookies/storage persist via profile"
    )
}

static RELAUNCH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

fn relaunch_lock_for_chat(chat_id: &str) -> Arc<AMutex<()>> {
    let locks = RELAUNCH_LOCKS.get_or_init(Default::default);
    let mut locks = locks.lock().unwrap_or_else(|error| error.into_inner());
    locks.retain(|key, lock| key == chat_id || Arc::strong_count(lock) > 1);
    locks.entry(chat_id.to_string()).or_default().clone()
}

#[derive(Debug, Clone)]
pub struct RuntimeRecoveryPlan {
    pub runtime_id: String,
    pub chat_id: String,
    pub profile_dir: PathBuf,
    pub launch_options: BrowserLaunchOptions,
}

pub async fn ensure_frame_emitter(
    app: crate::app_state::AppState,
    chat_id: &str,
    runtime_id: &str,
) {
    let runtime_arc = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes.get(runtime_id).cloned()
    };
    let Some(runtime_arc) = runtime_arc else {
        return;
    };
    let should_spawn = {
        let mut rt = runtime_arc.lock().await;
        if rt.frame_emitter_active {
            false
        } else {
            rt.frame_emitter_active = true;
            true
        }
    };
    if should_spawn {
        tokio::spawn(
            crate::http::routers::v1::v1_browser::browser_frame_emission_task(
                app.gcx.clone(),
                chat_id.to_string(),
                runtime_id.to_string(),
            ),
        );
    }
}

pub async fn relaunch_runtime_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
    window_bounds: Option<refact_chat_api::WindowBounds>,
) -> Result<String, String> {
    let options = BrowserLaunchOptions {
        window_bounds: window_bounds.or(launch_options.window_bounds.clone()),
        ..launch_options
    };

    let relaunch_lock = relaunch_lock_for_chat(chat_id);
    let _relaunch_guard = relaunch_lock.lock().await;

    let previous_emitter_active = match find_runtime_by_chat_id(app.clone(), chat_id).await {
        Some((runtime_id, runtime_arc)) => {
            let (emitter_active, usable) = {
                let mut rt = runtime_arc.lock().await;
                let same_mode = rt.launch_options.headless == options.headless;
                let usable = same_mode && tokio::task::block_in_place(|| rt.check_connection());
                (rt.frame_emitter_active, usable)
            };
            if usable {
                info!(
                    "BrowserRuntime {} is already live for chat {}, reusing it instead of relaunching",
                    runtime_id, chat_id
                );
                return Ok(runtime_id);
            }
            drop(runtime_arc);
            let removed = remove_browser_runtime(app.clone(), &runtime_id).await;
            drop(removed);
            tokio::time::sleep(RELAUNCH_SETTLE).await;
            emitter_active
        }
        None => false,
    };

    let mode = options.mode_label();
    let mut runtime = BrowserRuntime::launch(profile_dir, options)
        .map_err(|e| format!("Failed to relaunch browser in {} mode: {}", mode, e))?;
    runtime.reattach(chat_id);
    let runtime_id = register_browser_runtime(app.clone(), runtime).await;

    let runtime_arc = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes.get(&runtime_id).cloned()
    };
    if let Some(runtime_arc) = runtime_arc {
        let mut rt = runtime_arc.lock().await;
        if let Err(e) = setup_recording_for_runtime(&mut rt) {
            warn!(
                "Browser recording setup failed after {} relaunch (non-fatal): {}",
                mode, e
            );
        }
    }

    if previous_emitter_active {
        ensure_frame_emitter(app, chat_id, &runtime_id).await;
    }

    info!(
        "BrowserRuntime {} relaunched ({}) for chat {}",
        runtime_id, mode, chat_id
    );
    Ok(runtime_id)
}

pub async fn browser_snapshot_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<crate::chat::types::BrowserSnapshot> {
    let (runtime_id, runtime_arc) = find_runtime_by_chat_id(app, chat_id).await?;
    let mut rt = runtime_arc.lock().await;
    refact_browser::adopt_new_tabs(&mut rt, None);
    let tabs = rt
        .list_tab_infos()
        .into_iter()
        .map(|t| crate::chat::types::BrowserTabInfo {
            tab_id: t.id,
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

struct RuntimeHealth {
    chat_id: Option<String>,
    was_connected: bool,
    still_connected: bool,
    idle_expired: bool,
    idle_timeout: Duration,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
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
        let mut to_relaunch: Vec<RuntimeRecoveryPlan> = Vec::new();
        for rid in &runtime_ids {
            let runtime_arc = {
                let browser_runtimes = app.integrations.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                match browser_runtimes.get(rid) {
                    Some(arc) => arc.clone(),
                    None => continue,
                }
            };

            let RuntimeHealth {
                chat_id,
                was_connected,
                still_connected,
                idle_expired,
                idle_timeout,
                profile_dir,
                launch_options,
            } = {
                let mut rt = runtime_arc.lock().await;
                tokio::task::block_in_place(|| {
                    refact_browser::adopt_new_tabs(&mut rt, None);
                    let was_connected = rt.is_connected;
                    let still_connected = rt.check_connection();
                    RuntimeHealth {
                        chat_id: rt.attached_chat_id.clone(),
                        was_connected,
                        still_connected,
                        idle_expired: rt.is_idle_expired(),
                        idle_timeout: rt.idle_timeout,
                        profile_dir: rt.profile_dir.clone(),
                        launch_options: rt.launch_options.clone(),
                    }
                })
            };

            if was_connected && !still_connected {
                info!(
                    "BrowserRuntime {} (chat {:?}) lost connection",
                    rid, chat_id
                );
            }

            if chat_id.is_some() && idle_expired {
                warn!(
                    "BrowserRuntime {} idle timeout ({:?}) for chat {:?}",
                    rid, idle_timeout, chat_id
                );
                to_remove.push(rid.clone());
                continue;
            }

            if !still_connected {
                match chat_id {
                    Some(chat_id) => to_relaunch.push(RuntimeRecoveryPlan {
                        runtime_id: rid.clone(),
                        chat_id,
                        profile_dir,
                        launch_options,
                    }),
                    None => to_remove.push(rid.clone()),
                }
            }
        }

        for rid in to_remove {
            remove_browser_runtime(app.clone(), &rid).await;
        }

        for plan in to_relaunch {
            warn!(
                "BrowserRuntime {} is dead while attached to chat {}, relaunching",
                plan.runtime_id, plan.chat_id
            );
            if let Err(error) = relaunch_runtime_for_chat(
                app.clone(),
                &plan.chat_id,
                plan.profile_dir.clone(),
                plan.launch_options.clone(),
                None,
            )
            .await
            {
                warn!(
                    "Failed to relaunch dead BrowserRuntime {} for chat {}: {}",
                    plan.runtime_id, plan.chat_id, error
                );
                remove_browser_runtime(app.clone(), &plan.runtime_id).await;
            }
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
    fn monitor_evicts_dead_runtimes_regardless_of_chat_attachment() {
        let source = include_str!("browser_runtime.rs");
        let monitor = source
            .split_once("pub async fn browser_monitor_background_task(")
            .unwrap()
            .1
            .split_once("\n#[cfg(test)]")
            .unwrap()
            .0;

        assert!(
            !monitor.contains("if !still_connected && rt.attached_chat_id.is_none()"),
            "dead runtimes attached to a chat are still leaked forever"
        );
        assert!(monitor.contains("if !still_connected {"));
        assert!(monitor.contains("Some(chat_id) => to_relaunch.push(RuntimeRecoveryPlan {"));
        assert!(monitor.contains("None => to_remove.push(rid.clone()),"));
        assert!(monitor.contains("relaunch_runtime_for_chat("));
        assert!(
            monitor.contains("tokio::task::block_in_place(|| {"),
            "sync CDP calls still run on a worker thread while holding the runtime mutex"
        );
    }

    #[test]
    fn concurrent_relaunches_for_one_chat_share_a_single_guard() {
        let first = relaunch_lock_for_chat("chat-relaunch-guard");
        let second = relaunch_lock_for_chat("chat-relaunch-guard");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(
            &first,
            &relaunch_lock_for_chat("other-chat-relaunch-guard")
        ));

        let guard = first.try_lock().expect("first relaunch takes the guard");
        assert!(
            second.try_lock().is_err(),
            "a second relaunch for the same chat must wait instead of launching another browser"
        );
        drop(guard);
        assert!(second.try_lock().is_ok());
    }

    #[test]
    fn relaunch_reuses_a_live_runtime_in_the_requested_mode() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        for step in [
            "let relaunch_lock = relaunch_lock_for_chat(chat_id);",
            "let _relaunch_guard = relaunch_lock.lock().await;",
            "let same_mode = rt.launch_options.headless == options.headless;",
            "tokio::task::block_in_place(|| rt.check_connection())",
            "return Ok(runtime_id);",
        ] {
            assert!(helper.contains(step), "relaunch guard lost step: {step}");
        }
        assert!(
            helper.find("let _relaunch_guard").unwrap()
                < helper.find("remove_browser_runtime(").unwrap(),
            "the per-chat guard must be held across remove, launch and register"
        );
    }

    #[test]
    fn resume_warning_names_the_step_the_retry_starts_from() {
        let warning = relaunch_resume_warning(2);
        assert!(warning.contains("relaunched and resumed from step 2"));
        assert!(warning.contains("open tabs were lost"));
        assert!(warning.contains("cookies/storage persist via profile"));
    }

    #[test]
    fn relaunch_sequence_lives_only_in_the_shared_helper() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        for step in [
            "remove_browser_runtime(app.clone(), &runtime_id)",
            "tokio::time::sleep(RELAUNCH_SETTLE)",
            "BrowserRuntime::launch(profile_dir, options)",
            "runtime.reattach(chat_id)",
            "register_browser_runtime(app.clone(), runtime)",
            "setup_recording_for_runtime(&mut rt)",
            "ensure_frame_emitter(app, chat_id, &runtime_id)",
        ] {
            assert!(helper.contains(step), "relaunch helper lost step: {step}");
        }

        let router = include_str!("../http/routers/v1/v1_browser.rs");
        assert!(router.contains("relaunch_runtime_for_chat("));
        assert!(
            !router.contains("Failed to relaunch browser in {} mode"),
            "router still duplicates the relaunch sequence"
        );

        let controller = include_str!("browser_controller.rs");
        assert!(controller.contains("relaunch_runtime_for_chat("));
        assert!(!controller.contains("BrowserRuntime::launch("));
    }

    #[test]
    fn relaunch_preserves_profile_launch_options_and_window_bounds() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        assert!(helper
            .contains("window_bounds: window_bounds.or(launch_options.window_bounds.clone())"));
        assert!(helper.contains("..launch_options"));
        assert!(RELAUNCH_WARNING.contains("relaunched and retried"));
        assert!(RELAUNCH_WARNING.contains("open tabs lost"));
    }

    #[test]
    fn setup_chrome_session_cannot_hand_back_the_dead_runtime() {
        let session = include_str!("../tools/tool_chrome.rs")
            .split_once("async fn setup_chrome_session(")
            .unwrap()
            .1;
        let unhealthy = session
            .split_once("if runtime_healthy {")
            .unwrap()
            .1
            .split_once("find_runtime_by_chat_id(")
            .unwrap()
            .0;

        assert!(
            unhealthy.contains("remove_browser_runtime("),
            "dead runtime is not evicted before re-resolving it by chat id"
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
