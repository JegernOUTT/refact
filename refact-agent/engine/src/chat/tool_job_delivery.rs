//! One runtime-owned completion bridge. Results remain native messages, never parsed as JSON.
use std::sync::Arc;
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::tool_jobs::{JobSnapshot, JobStatus, JobReceipt};
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    // Current-thread tests install per-invocation hooks; no process-global hook
    // can intercept another test or a completion bridge on a different thread.
    type LandingHook = Box<dyn FnMut(&str, &super::super::types::ChatSession)>;
    thread_local! {
        static LANDING_HOOK: RefCell<Option<LandingHook>> = RefCell::new(None);
        static LOSE_LANDING_ACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static BRIDGE_SCAN_DONE: RefCell<Option<Arc<tokio::sync::Notify>>> = RefCell::new(None);
    }
    struct HookReset;
    impl Drop for HookReset {
        fn drop(&mut self) {
            LANDING_HOOK.with(|hook| *hook.borrow_mut() = None);
            BRIDGE_SCAN_DONE.with(|hook| *hook.borrow_mut() = None);
            LOSE_LANDING_ACK.with(|lose| lose.set(false));
        }
    }
    pub(super) fn landing_hook(stage: &str, session: &super::super::types::ChatSession) {
        LANDING_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(stage, session);
            }
        });
    }

    pub(super) fn lose_landing_ack() -> bool {
        LOSE_LANDING_ACK.with(|lose| lose.replace(false))
    }

    pub(super) fn bridge_scan_done() {
        BRIDGE_SCAN_DONE.with(|hook| {
            if let Some(done) = hook.borrow().as_ref() {
                done.notify_one();
            }
        });
    }

    pub(crate) async fn policy_race_fixture(
        chat_id: &str,
    ) -> (
        tempfile::TempDir,
        AppState,
        Arc<tokio::sync::Mutex<super::super::types::ChatSession>>,
        JobReceipt,
    ) {
        policy_fixture(chat_id, false).await
    }

    async fn policy_fixture(
        chat_id: &str,
        durable: bool,
    ) -> (
        tempfile::TempDir,
        AppState,
        Arc<tokio::sync::Mutex<super::super::types::ChatSession>>,
        JobReceipt,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "model".into(),
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(project_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let mut app = AppState::from_gcx(gcx).await;
        if durable {
            app.runtime.tool_jobs = crate::tool_jobs::JobRuntime::open_durable(
                temp.path().join("durable-jobs"),
                2,
                65536,
            )
            .await
            .unwrap();
        }
        let mut session = super::super::types::ChatSession::new(chat_id.into());
        session.thread.mode = "agent".into();
        session.thread.model = "model".into();
        session.work_project_root = Some(root);
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session.clone());
        let receipt = app
            .runtime
            .tool_jobs
            .submit_background(
                crate::tool_jobs::JobBinding {
                    scope_keys: vec![],
                    chat_id: chat_id.into(),
                    tool_call_id: "call".into(),
                    tool_name: "cat".into(),
                    model: "model".into(),
                    turn_id: "turn".into(),
                },
                |_| async {
                    (
                        JobStatus::Succeeded,
                        crate::tool_jobs::JobResult {
                            had_corrections: false,
                            messages: vec![ChatMessage::new(
                                "tool".into(),
                                "native race result".into(),
                            )],
                            context_files: vec![],
                        },
                    )
                },
            )
            .await
            .unwrap();
        let snapshot = app
            .runtime
            .tool_jobs
            .wait_terminal(chat_id, &receipt.job_id)
            .await
            .unwrap();
        let mut pending = super::super::types::PendingDelivery::with_id(
            format!("tool_job:{}:{}", receipt.job_id, snapshot.revision),
            vec![],
            super::super::types::PushMode::Append,
            "tool_job.completed",
            false,
        );
        pending.deferred_job = Some(refact_core::chat_types::DeferredJobRef {
            job_id: receipt.job_id.clone(),
            revision: snapshot.revision,
            chat_id: chat_id.into(),
            tool_call_id: "call".into(),
            tool_name: "cat".into(),
            model: "model".into(),
            turn_id: "turn".into(),
        });
        session.lock().await.enqueue_delivery(pending).unwrap();
        // A successful initial clearance prevents an early return from making
        // either race test vacuously pass.
        checked_snapshot(&app, chat_id, &receipt.job_id)
            .await
            .unwrap();
        (temp, app, session, receipt)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shipping_startup_recovers_durable_result_and_lost_ack_without_submission() {
        for lost_ack in [false, true] {
            let chat = if lost_ack {
                "startup-lost-ack"
            } else {
                "startup-retained"
            };
            let (temp, mut app, session, receipt) = policy_fixture(chat, true).await;
            let snapshot = app.runtime.tool_jobs.get(chat, &receipt.job_id).unwrap();
            let delivery_id = format!("tool_job:{}:{}", receipt.job_id, snapshot.revision);
            {
                let mut session = session.lock().await;
                // The pre-restart chat has only its accepted receipt, not an
                // already-enqueued completion hint.
                session.pending_deliveries.clear();
                session.add_message(receipt_message(&receipt));
            }
            if lost_ack {
                let mut pending = super::super::types::PendingDelivery::with_id(
                    delivery_id.clone(),
                    vec![],
                    super::super::types::PushMode::Append,
                    "tool_job.completed",
                    false,
                );
                pending.deferred_job = Some(refact_core::chat_types::DeferredJobRef {
                    job_id: receipt.job_id.clone(),
                    revision: snapshot.revision,
                    chat_id: chat.into(),
                    tool_call_id: "call".into(),
                    tool_name: "cat".into(),
                    model: "model".into(),
                    turn_id: "turn".into(),
                });
                session.lock().await.enqueue_delivery(pending).unwrap();
                // Model process loss precisely after the real append and its
                // required save, before the runtime's durable ACK.
                LOSE_LANDING_ACK.with(|lose| lose.set(true));
                land_deferred_jobs(&app, &session).await;
                assert!(!LOSE_LANDING_ACK.with(|lose| lose.get()));
                assert!(session
                    .lock()
                    .await
                    .delivered_delivery_ids
                    .contains(&delivery_id));
            }
            super::super::trajectories::try_save_trajectory_with_intent(
                app.clone(),
                session.clone(),
                super::super::types::TrajectoryCommitIntent::Required,
            )
            .await
            .unwrap();
            assert_eq!(app.runtime.tool_jobs.background_terminals().len(), 1);
            app.runtime.tool_jobs.shutdown().await;
            app.chat.sessions.write().await.remove(chat);
            drop(session);
            // Dropping the last owner releases the durable journal lock before
            // reopening it, just as a fresh engine process does.
            app.runtime.tool_jobs = crate::tool_jobs::JobRuntime::new(2, 65536);
            app.runtime.tool_jobs = crate::tool_jobs::JobRuntime::open_durable(
                temp.path().join("durable-jobs"),
                2,
                65536,
            )
            .await
            .unwrap();
            let restored = super::super::session_runtime::get_or_create_session_with_trajectory(
                app.clone(),
                &app.chat.sessions,
                chat,
            )
            .await;
            assert_eq!(
                restored.lock().await.messages.len(),
                if lost_ack { 2 } else { 1 }
            );
            // Suppress model execution, not completion reconciliation. This
            // keeps the oracle local and prevents any external model request.
            restored
                .lock()
                .await
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let appended = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = appended.clone();
            let _reset = HookReset;
            LANDING_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |stage, _| {
                    if stage == "appended" {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }))
            });
            // This is the shipping startup entry point. No submit, explicit
            // ensure_bridge, or manual landing call occurs during recovery.
            let scan_done = Arc::new(tokio::sync::Notify::new());
            BRIDGE_SCAN_DONE.with(|hook| *hook.borrow_mut() = Some(scan_done.clone()));
            super::super::session_runtime::start_session_cleanup_task(app.clone());
            tokio::time::timeout(Duration::from_secs(10), scan_done.notified())
                .await
                .expect("shipping startup must reconcile retained completion");
            assert!(app.runtime.tool_jobs.background_terminals().is_empty());
            let locked = restored.lock().await;
            assert_eq!(locked.messages.len(), 2);
            assert_eq!(
                locked.messages[1].content.content_text_only(),
                "native race result"
            );
            assert_eq!(locked.messages[1].message_id, format!("{delivery_id}:0"));
            assert!(locked.delivered_delivery_ids.contains(&delivery_id));
            assert_eq!(
                appended.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!lost_ack)
            );
            drop(locked);
            let persisted =
                super::super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat)
                    .await
                    .unwrap();
            assert_eq!(
                persisted.messages.len(),
                2,
                "ACK must follow actual durable append"
            );
            app.runtime
                .shutdown_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
            app.runtime.tool_jobs.shutdown().await;
            assert!(!restored.lock().await.closed);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_readiness_completion_preserves_parked_continuation_through_bridge() {
        let chat = "private-readiness-bridge";
        let (_temp, app, session, old_receipt) = policy_race_fixture(chat).await;
        let old = app
            .runtime
            .tool_jobs
            .wait_terminal(chat, &old_receipt.job_id)
            .await
            .unwrap();
        app.runtime
            .tool_jobs
            .ack_delivered(chat, &old_receipt.job_id, old.revision)
            .await
            .unwrap();
        let receipt = app
            .runtime
            .tool_jobs
            .submit_readiness(
                crate::tool_jobs::JobBinding {
                    scope_keys: vec![],
                    chat_id: chat.into(),
                    tool_call_id: "private".into(),
                    tool_name: "review_readiness".into(),
                    model: String::new(),
                    turn_id: "original-nonce".into(),
                },
                |_| async {
                    (
                        JobStatus::Succeeded,
                        crate::tool_jobs::JobResult {
                            had_corrections: false,
                            messages: vec![],
                            context_files: vec![],
                        },
                    )
                },
            )
            .await
            .unwrap();
        let terminal = app
            .runtime
            .tool_jobs
            .wait_terminal(chat, &receipt.job_id)
            .await
            .unwrap();
        let version = {
            let mut locked = session.lock().await;
            locked.pending_deliveries.clear();
            locked.review_readiness_pending =
                Some(super::super::types::ReviewReadinessContinuation {
                    turn_nonce: "original-nonce".into(),
                    receipt: receipt.clone(),
                    command: locked.active_command.clone(),
                    trajectory_version: locked.trajectory_version,
                    thread_fingerprint: serde_json::to_value(&locked.thread).unwrap(),
                    cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    ready: true,
                });
            locked.trajectory_version
        };
        let done = Arc::new(tokio::sync::Notify::new());
        let _reset = HookReset;
        BRIDGE_SCAN_DONE.with(|hook| *hook.borrow_mut() = Some(done.clone()));
        ensure_bridge(&app);
        tokio::time::timeout(Duration::from_secs(10), done.notified())
            .await
            .unwrap();
        {
            let locked = session.lock().await;
            assert_eq!(locked.trajectory_version, version);
            assert!(locked.pending_deliveries.is_empty());
            assert!(locked.messages.is_empty());
        }
        assert!(super::super::generation::take_ready_review_continuation(&session).await);
        assert_eq!(
            session
                .lock()
                .await
                .review_readiness_resume_nonce
                .as_deref(),
            Some("original-nonce")
        );
        assert!(!super::super::generation::take_ready_review_continuation(&session).await);
        app.runtime
            .tool_jobs
            .ack_readiness(chat, &receipt.job_id, terminal.revision)
            .await
            .unwrap();
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn final_policy_guard_blocks_reload_through_real_append() {
        let (_temp, app, session, _) = policy_race_fixture("policy-held-append").await;
        let policy = app.gcx.privacy_policy_load.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let writer_start = start.clone();
        let append_policy = policy.clone();
        let (probe_tx, probe_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            writer_start.wait();
            let blocked = matches!(policy.try_write(), Err(std::sync::TryLockError::WouldBlock));
            probe_tx.send(blocked).unwrap();
            policy.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy::default());
            done_tx.send(()).unwrap();
        });
        let appended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_append = appended.clone();
        let _reset = HookReset;
        LANDING_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |stage, session| {
                if stage == "compared" {
                    start.wait();
                    assert!(
                        probe_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                        "reload writer must be excluded after final comparison"
                    );
                } else if stage == "appended" {
                    assert!(
                        matches!(
                            append_policy.try_write(),
                            Err(std::sync::TryLockError::WouldBlock)
                        ),
                        "the policy read guard must still be held at append"
                    );
                    assert_eq!(session.messages.len(), 1);
                    assert_eq!(
                        session.messages[0].content.content_text_only(),
                        "native race result"
                    );
                    assert!(session.pending_deliveries.is_empty());
                    assert!(
                        matches!(
                            done_rx.try_recv(),
                            Err(std::sync::mpsc::TryRecvError::Empty)
                        ),
                        "policy replacement cannot finish before the actual append"
                    );
                    saw_append.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }))
        });
        land_deferred_jobs(&app, &session).await;
        assert!(
            appended.load(std::sync::atomic::Ordering::SeqCst),
            "real append hook was not reached"
        );
        writer.join().unwrap();
        // Replacement after a valid append does not retroactively erase it.
        assert_eq!(session.lock().await.messages.len(), 1);
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn customization_fence_blocks_writer_and_rejects_prior_revocation() {
        let (_temp, app, session, _) = policy_race_fixture("customization-held-append").await;
        let cache = app.gcx.project_registry_cache.clone();
        let generations = app.gcx.tool_catalog_generations.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let writer_start = start.clone();
        let (probe_tx, probe_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            writer_start.wait();
            probe_tx
                .send(matches!(
                    cache.try_write(),
                    Err(std::sync::TryLockError::WouldBlock)
                ))
                .unwrap();
            let mut cache = cache.write().unwrap();
            cache.cache.clear();
            generations.advance_customization();
            done_tx.send(()).unwrap();
        });
        let appended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_append = appended.clone();
        let append_cache = app.gcx.project_registry_cache.clone();
        let reset = HookReset;
        LANDING_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |stage, session| {
                if stage == "compared" {
                    start.wait();
                    assert!(probe_rx.recv_timeout(Duration::from_secs(5)).unwrap());
                } else if stage == "appended" {
                    assert!(matches!(
                        append_cache.try_write(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    assert!(matches!(
                        done_rx.try_recv(),
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    ));
                    assert_eq!(session.messages.len(), 1);
                    assert!(session.pending_deliveries.is_empty());
                    saw_append.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }));
        });
        land_deferred_jobs(&app, &session).await;
        assert!(appended.load(std::sync::atomic::Ordering::SeqCst));
        writer.join().unwrap();
        drop(reset);
        land_deferred_jobs(&app, &session).await;
        assert_eq!(session.lock().await.messages.len(), 1);
        app.runtime.tool_jobs.shutdown().await;

        let (_temp, app, session, receipt) =
            policy_race_fixture("customization-before-guard").await;
        let saved = app.gcx.project_registry_cache.read().unwrap().cache.clone();
        let gcx = app.gcx.clone();
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_guard = reached.clone();
        let reset = HookReset;
        LANDING_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |stage, _| {
                assert_eq!(stage, "before_guard");
                let mut cache = gcx.project_registry_cache.write().unwrap();
                for cached in cache.cache.values_mut() {
                    cached
                        .registry
                        .modes
                        .get_mut("agent")
                        .unwrap()
                        .tools
                        .retain(|name| name != "cat");
                }
                gcx.tool_catalog_generations.advance_customization();
                saw_guard.store(true, std::sync::atomic::Ordering::SeqCst);
            }));
        });
        land_deferred_jobs(&app, &session).await;
        assert!(reached.load(std::sync::atomic::Ordering::SeqCst));
        drop(reset);
        {
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert_eq!(session.pending_deliveries.len(), 1);
            assert!(session.pending_deliveries[0].messages.is_empty());
            assert!(session.pending_deliveries[0].deferred_job.is_some());
        }
        land_deferred_jobs(&app, &session).await;
        assert!(session.lock().await.messages.is_empty());
        let retained = app
            .runtime
            .tool_jobs
            .get_result("customization-before-guard", &receipt.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "native race result"
        );
        {
            let mut cache = app.gcx.project_registry_cache.write().unwrap();
            cache.cache = saved;
            app.gcx.tool_catalog_generations.advance_customization();
        }
        land_deferred_jobs(&app, &session).await;
        land_deferred_jobs(&app, &session).await;
        assert_eq!(session.lock().await.messages.len(), 1);
        app.runtime.tool_jobs.shutdown().await;
    }

    async fn register_child(app: &AppState, chat: &str) -> String {
        use crate::agents::types::{BgAgentKind, CreateAgentRequest};
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: Some("parent".into()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "child".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, chat.into())
            .await
            .unwrap();
        let mut config = crate::subchat::test_support::minimal_config(chat, "parent");
        config.model = "model".into();
        config.tools = crate::subchat::ToolsPolicy::Only(vec!["cat".into()]);
        app.agent_executions
            .remember_config(&record.agent_id, config);
        record.agent_id
    }

    #[tokio::test(flavor = "current_thread")]
    async fn child_config_writer_is_blocked_through_actual_append_and_lands_once() {
        let chat = "child-config-fenced";
        let (_temp, app, session, _) = policy_race_fixture(chat).await;
        let id = register_child(&app, chat).await;
        let registry = app.agent_executions.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let writer_start = start.clone();
        let (probe_tx, probe_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            writer_start.wait();
            probe_tx
                .send(registry.config_writer_blocked_for_test())
                .unwrap();
            registry.forget_config(&id);
            done_tx.send(()).unwrap();
        });
        let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let appended = seen.clone();
        let append_registry = app.agent_executions.clone();
        let _reset = HookReset;
        LANDING_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |stage, session| {
                if stage == "compared" {
                    start.wait();
                    assert!(probe_rx.recv_timeout(Duration::from_secs(5)).unwrap());
                } else if stage == "appended" {
                    assert!(append_registry.config_writer_blocked_for_test());
                    assert_eq!(session.messages.len(), 1);
                    assert!(matches!(
                        done_rx.try_recv(),
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    ));
                    appended.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }))
        });
        land_deferred_jobs(&app, &session).await;
        assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
        writer.join().unwrap();
        land_deferred_jobs(&app, &session).await;
        assert_eq!(session.lock().await.messages.len(), 1);
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn child_config_revoked_missing_or_legacy_at_final_guard_retains_result() {
        for state in ["revoked", "missing", "legacy"] {
            let chat = format!("child-config-{state}");
            let (_temp, app, session, receipt) = policy_race_fixture(&chat).await;
            let id = register_child(&app, &chat).await;
            let registry = app.agent_executions.clone();
            let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let saw_guard = reached.clone();
            let _reset = HookReset;
            LANDING_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |stage, _| {
                    assert_eq!(stage, "before_guard");
                    if state == "missing" {
                        registry.forget_config(&id);
                    } else {
                        let mut config = registry.saved_config(&id).unwrap();
                        config.tools = if state == "legacy" {
                            crate::subchat::ToolsPolicy::All
                        } else {
                            crate::subchat::ToolsPolicy::None
                        };
                        registry.remember_config(&id, config);
                    }
                    saw_guard.store(true, std::sync::atomic::Ordering::SeqCst);
                }))
            });
            land_deferred_jobs(&app, &session).await;
            assert!(reached.load(std::sync::atomic::Ordering::SeqCst));
            let locked = session.lock().await;
            assert!(locked.messages.is_empty());
            assert_eq!(locked.pending_deliveries.len(), 1);
            assert!(locked.pending_deliveries[0].messages.is_empty());
            assert!(locked.pending_deliveries[0].deferred_job.is_some());
            drop(locked);
            let retained = app
                .runtime
                .tool_jobs
                .get_result(&chat, &receipt.job_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                retained.result.unwrap().messages[0]
                    .content
                    .content_text_only(),
                "native race result"
            );
            app.runtime.tool_jobs.shutdown().await;
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn policy_replaced_before_final_guard_retains_source_free_reference() {
        let (_temp, app, session, receipt) = policy_race_fixture("policy-before-guard").await;
        let policy = app.gcx.privacy_policy_load.clone();
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_guard = reached.clone();
        let _reset = HookReset;
        LANDING_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |stage, _| {
                if stage == "before_guard" {
                    let policy = policy.clone();
                    std::thread::spawn(move || {
                        policy.write().unwrap().policy =
                            Arc::new(refact_privacy::PrivacyPolicy::default());
                    })
                    .join()
                    .unwrap();
                    saw_guard.store(true, std::sync::atomic::Ordering::SeqCst);
                } else {
                    panic!("changed policy must fail the final pointer comparison before {stage}");
                }
            }))
        });
        land_deferred_jobs(&app, &session).await;
        assert!(reached.load(std::sync::atomic::Ordering::SeqCst));
        {
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert_eq!(session.pending_deliveries.len(), 1);
            assert!(session.pending_deliveries[0].messages.is_empty());
            assert!(session.pending_deliveries[0].deferred_job.is_some());
        }
        let retained = app
            .runtime
            .tool_jobs
            .get_result("policy-before-guard", &receipt.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "native race result"
        );
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn authority_revoked_after_enqueue_retains_reference_and_native_payload() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "deferred-revoked";
        let session = Arc::new(tokio::sync::Mutex::new(
            super::super::types::ChatSession::new(chat_id.into()),
        ));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session.clone());
        let receipt = app
            .runtime
            .tool_jobs
            .submit_background(
                crate::tool_jobs::JobBinding {
                    scope_keys: vec![],
                    chat_id: chat_id.into(),
                    tool_call_id: "call".into(),
                    tool_name: "cat".into(),
                    model: "model".into(),
                    turn_id: "turn".into(),
                },
                |_| async {
                    (
                        JobStatus::Succeeded,
                        crate::tool_jobs::JobResult {
                            had_corrections: false,
                            messages: vec![ChatMessage::new(
                                "tool".into(),
                                "retained exact native result".into(),
                            )],
                            context_files: vec![],
                        },
                    )
                },
            )
            .await
            .unwrap();
        let snapshot = app
            .runtime
            .tool_jobs
            .wait_terminal(chat_id, &receipt.job_id)
            .await
            .unwrap();
        let mut pending = super::super::types::PendingDelivery::with_id(
            format!("tool_job:{}:{}", receipt.job_id, snapshot.revision),
            vec![],
            super::super::types::PushMode::Append,
            "tool_job.completed",
            false,
        );
        pending.deferred_job = Some(refact_core::chat_types::DeferredJobRef {
            job_id: receipt.job_id.clone(),
            revision: snapshot.revision,
            chat_id: chat_id.into(),
            tool_call_id: "call".into(),
            tool_name: "cat".into(),
            model: "model".into(),
            turn_id: "turn".into(),
        });
        session.lock().await.enqueue_delivery(pending).unwrap();
        // The queue boundary is a deterministic barrier: revoke after acceptance,
        // before the async coordinator can resolve any native payload.
        session.lock().await.thread.mode = "NO_TOOLS".into();
        land_deferred_jobs(&app, &session).await;
        {
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert_eq!(session.pending_deliveries.len(), 1);
        }
        session.lock().await.closed = true;
        land_deferred_jobs(&app, &session).await;
        assert!(session.lock().await.messages.is_empty());
        app.chat.sessions.write().await.remove(chat_id);
        land_deferred_jobs(&app, &session).await;
        assert!(!app.chat.sessions.read().await.contains_key(chat_id));
        let retained = app
            .runtime
            .tool_jobs
            .get_result(chat_id, &receipt.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "retained exact native result"
        );
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn retained_terminal_scan_survives_lag_and_receipt_is_not_success() {
        let runtime = crate::tool_jobs::JobRuntime::new(2, 65536);
        let mut events = runtime.subscribe();
        let mut ids = Vec::new();
        for index in 0..2 {
            let receipt = runtime
                .submit_background(
                    crate::tool_jobs::JobBinding {
                        scope_keys: vec![],
                        chat_id: "owner".into(),
                        tool_call_id: format!("call-{index}"),
                        tool_name: "cat".into(),
                        model: "model".into(),
                        turn_id: "turn".into(),
                    },
                    |_| async {
                        (
                            JobStatus::Failed,
                            crate::tool_jobs::JobResult {
                                had_corrections: false,
                                messages: vec![ChatMessage::new(
                                    "tool".into(),
                                    "plain result, not JSON".into(),
                                )],
                                context_files: vec![],
                            },
                        )
                    },
                )
                .await
                .unwrap();
            let message = receipt_message(&receipt);
            assert_eq!(message.tool_failed, None);
            assert_eq!(message.extra["tool_job"]["status"], "accepted");
            ids.push(receipt.job_id);
        }
        for id in &ids {
            runtime.wait_terminal("owner", id).await.unwrap();
        }
        // Discard hints entirely: state reconciliation still finds both records.
        while events.try_recv().is_ok() {}
        let retained = runtime.background_terminals();
        assert_eq!(retained.len(), 2);
        assert!(retained
            .iter()
            .all(|snapshot| snapshot.status == JobStatus::Failed));
        assert!(retained
            .iter()
            .all(|snapshot| snapshot.result.as_ref().unwrap().messages[0]
                .content
                .content_text_only()
                == "plain result, not JSON"));
        runtime.shutdown().await;
    }
}

pub(crate) fn metadata(
    receipt: &JobReceipt,
    status: JobStatus,
    revision: u64,
    available: bool,
) -> serde_json::Value {
    let mut value = serde_json::to_value(receipt).expect("receipt serializes");
    value["status"] = serde_json::to_value(status).expect("status serializes");
    value["revision"] = revision.into();
    value["result_available"] = available.into();
    value
}

/// Rechecks BOTH present tool authority and present destination privacy. The retained
/// native result is not modified when a destination cannot currently receive it.
pub(crate) async fn checked_snapshot(
    app: &AppState,
    chat_id: &str,
    job_id: &str,
) -> Result<JobSnapshot, String> {
    let mut snapshot = app
        .runtime
        .tool_jobs
        .get_result(chat_id, job_id)
        .await?
        .ok_or("tool job not found")?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("chat unavailable")?;
    let thread = {
        let session = session.lock().await;
        if session.closed {
            return Err("chat closed".into());
        }
        session.thread.clone()
    };
    let catalog = app
        .tool_registry
        .acquire_tool_catalog(
            &thread.mode,
            Some(&thread.model),
            thread
                .worktree
                .as_ref()
                .map(|w| w.root.to_string_lossy().into_owned())
                .as_deref(),
        )
        .await;
    if !catalog
        .index
        .tools
        .iter()
        .any(|t| t.name == snapshot.receipt.binding.tool_name)
    {
        return Err("tool result withheld by current tool authority".into());
    }
    let context = Arc::new(tokio::sync::Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app.clone(),
            8192,
            1,
            false,
            Vec::new(),
            chat_id.into(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            thread.task_meta.clone(),
            thread.worktree.clone(),
        )
        .await,
    ));
    crate::agents::tool_authority::ensure_context_tool(
        &context,
        &snapshot.receipt.binding.tool_name,
    )
    .await?;
    for key in &snapshot.receipt.binding.scope_keys {
        if let crate::tool_jobs::ScopeKey::Worktree {
            project,
            root,
        } = key
        {
            let scope_matches = match thread.worktree.as_ref() {
                Some(worktree) => {
                    &worktree.source_workspace_root == project && &worktree.root == root
                }
                None => app
                    .gcx
                    .documents_state
                    .workspace_folders
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|configured| {
                        std::fs::canonicalize(configured)
                            .ok()
                            .is_some_and(|canonical| {
                                let canonical = dunce::simplified(&canonical);
                                canonical == project && canonical == root
                            })
                    }),
            };
            if !scope_matches {
                return Err("job execution scope no longer attached".into());
            }
        }
    }
    if let Some(result) = snapshot.result.as_mut() {
        let model = crate::agents::handoff::parent_destination(app, chat_id)
            .await
            .ok_or("result destination unresolved")?;
        crate::privacy::records::attach_declared_output_files(&app.gcx, &mut result.messages)?;
        let mut audit_messages = result.messages.clone();
        if !result.context_files.is_empty() {
            let mut files_message = ChatMessage::default();
            files_message.role = "context_file".into();
            files_message.content =
                crate::call_validation::ChatContent::ContextFiles(result.context_files.clone());
            audit_messages.push(files_message);
        }
        crate::privacy::records::attach_declared_output_files(&app.gcx, &mut audit_messages)?;
        crate::agents::launch_spec::clear_current_context(&app.gcx, &audit_messages, &model)?;
    }
    Ok(snapshot)
}

pub(crate) fn ensure_bridge(app: &AppState) {
    let mut events = app.runtime.tool_jobs.subscribe();
    let app = app.clone();
    app.runtime
        .tool_jobs
        .clone()
        .install_completion_bridge(move |mut stop| {
            tokio::spawn(async move {
                loop {
                    for snapshot in app.runtime.tool_jobs.background_terminals() {
                        let binding = &snapshot.receipt.binding;
                        let job_id = &snapshot.receipt.job_id;
                        let id = format!("tool_job:{}:{}", job_id, snapshot.revision);
                        let mut delivery = super::types::PendingDelivery::with_id(
                            id,
                            vec![],
                            super::types::PushMode::Append,
                            "tool_job.completed",
                            true,
                        );
                        // Scope keys remain private to the runtime journal. This public
                        // reference grants no authority and contains no native output.
                        delivery.deferred_job = Some(refact_core::chat_types::DeferredJobRef {
                            job_id: job_id.clone(),
                            revision: snapshot.revision,
                            chat_id: binding.chat_id.clone(),
                            tool_call_id: binding.tool_call_id.clone(),
                            tool_name: binding.tool_name.clone(),
                            model: binding.model.clone(),
                            turn_id: binding.turn_id.clone(),
                        });
                        delivery.after_tool_call_id = Some(binding.tool_call_id.clone());
                        if let Err(error) = super::delivery::deliver_to_open_chat_with_receipt(
                            app.clone(),
                            &binding.chat_id,
                            delivery,
                        )
                        .await
                        {
                            tracing::debug!(%job_id, %error, "tool job reference remains retained");
                            continue;
                        }
                        let session = app
                            .chat
                            .sessions
                            .read()
                            .await
                            .get(&binding.chat_id)
                            .cloned();
                        if let Some(session) = session {
                            land_deferred_jobs(&app, &session).await;
                            let (wake, running, notify) = {
                                let session = session.lock().await;
                                (
                                    !session.closed
                                        && !session.delivery_wake_sources.is_empty()
                                        && matches!(
                                            session.runtime.state,
                                            super::types::SessionState::Idle
                                                | super::types::SessionState::Completed
                                                | super::types::SessionState::WaitingUserInput
                                        ),
                                    session.queue_processor_running.clone(),
                                    session.queue_notify.clone(),
                                )
                            };
                            if wake && !running.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                tokio::spawn(super::queue::process_command_queue(
                                    app.clone(),
                                    session.clone(),
                                    running,
                                ));
                            } else if wake {
                                notify.notify_one();
                            }
                            // A crash after trajectory commit but before ACK is repaired
                            // by the stable delivery id, without appending output again.
                            let delivered = session
                                .lock()
                                .await
                                .delivered_delivery_ids
                                .contains(&format!("tool_job:{}:{}", job_id, snapshot.revision));
                            if delivered
                                && matches!(
                                    super::trajectories::try_save_trajectory_with_intent(
                                        app.clone(),
                                        session.clone(),
                                        super::types::TrajectoryCommitIntent::Required
                                    )
                                    .await,
                                    Ok(true)
                                )
                            {
                                let _ = app
                                    .runtime
                                    .tool_jobs
                                    .ack_delivered(&binding.chat_id, job_id, snapshot.revision)
                                    .await;
                            }
                        }
                    }
                    #[cfg(test)]
                    tests::bridge_scan_done();
                    tokio::select! {
                        event = events.recv() => match event {
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                            Err(_) => break,
                        },
                        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
                        _ = stop.changed() => break,
                        _ = crate::background_tasks::wait_for_shutdown(app.gcx.clone()) => break,
                    }
                }
            })
        });
}

/// Resolve source-free references only at an open boundary. The session/map
/// guards fence lifecycle and thread changes; native output is never placed in
/// a pending envelope, even transiently. A concurrent second resolver loses the
/// pending-id comparison and therefore cannot append twice.
pub(crate) async fn land_deferred_jobs(
    app: &AppState,
    session_arc: &Arc<tokio::sync::Mutex<super::types::ChatSession>>,
) {
    let candidates = {
        let session = session_arc.lock().await;
        if session.closed {
            return;
        }
        session
            .pending_deliveries
            .iter()
            .filter(|d| d.deferred_job.is_some() && session.delivery_envelope_boundary_open(d))
            .cloned()
            .collect::<Vec<_>>()
    };
    for pending in candidates {
        let reference = pending.deferred_job.as_ref().unwrap();
        // Resolve identity before taking synchronous config/session fences.
        let agent_id = app
            .agents
            .find_agent_id_by_child_chat_id(&reference.chat_id)
            .await;
        let (thread, allowed) = {
            let session = session_arc.lock().await;
            (
                serde_json::to_value(&session.thread).ok(),
                session.active_command.allowed_tools.clone(),
            )
        };
        let policy = app
            .gcx
            .privacy_policy_load
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .policy
            .clone();
        let customization =
            crate::yaml_configs::customization_registry::customization_generation(&app.gcx);
        let Ok(snapshot) = checked_snapshot(app, &reference.chat_id, &reference.job_id).await
        else {
            continue;
        };
        let binding = &snapshot.receipt.binding;
        if snapshot.receipt.job_id != reference.job_id
            || snapshot.revision != reference.revision
            || binding.chat_id != reference.chat_id
            || binding.tool_call_id != reference.tool_call_id
            || binding.tool_name != reference.tool_name
            || binding.model != reference.model
            || binding.turn_id != reference.turn_id
            || !matches!(
                snapshot.status,
                JobStatus::Succeeded
                    | JobStatus::Failed
                    | JobStatus::Cancelled
                    | JobStatus::Unknown
            )
        {
            continue;
        }
        let Some(mut result) = snapshot.result else {
            continue;
        };
        let Some(model) = crate::agents::handoff::parent_destination(app, &reference.chat_id).await
        else {
            continue;
        };
        if !result.context_files.is_empty() {
            let mut message = ChatMessage::default();
            message.role = "context_file".into();
            message.content =
                crate::call_validation::ChatContent::ContextFiles(result.context_files);
            result.messages.push(message);
        }
        for (index, message) in result.messages.iter_mut().enumerate() {
            let original_role = message.role.clone();
            let original_call = message.tool_call_id.clone();
            message.message_id = format!(
                "tool_job:{}:{}:{}",
                reference.job_id, reference.revision, index
            );
            message.extra.insert(
                "tool_job_result".into(),
                serde_json::json!({
                    "job_id": reference.job_id, "revision": reference.revision,
                    "tool_call_id": original_call, "original_role": original_role,
                    "actor": "tool_completion", "tool_name": reference.tool_name,
                }),
            );
            if message.role == "tool" {
                message.role = "event".into();
                message.tool_call_id.clear();
                message.extra.insert(
                    "event".into(),
                    serde_json::json!({
                        "subkind":"system_notice", "source":"tool_job.completed",
                        "payload":{"actor":"tool_completion", "job_id":reference.job_id}
                    }),
                );
            }
        }
        if result.messages.is_empty() {
            continue;
        }
        let sessions = app.chat.sessions.read().await;
        if !sessions
            .get(&reference.chat_id)
            .is_some_and(|current| Arc::ptr_eq(current, session_arc))
        {
            continue;
        }
        let mut session = session_arc.lock().await;
        if session.closed
            || serde_json::to_value(&session.thread).ok() != thread
            || session.active_command.allowed_tools != allowed
            || (!allowed.is_empty() && !allowed.contains(&reference.tool_name))
            || !session.delivery_envelope_boundary_open(&pending)
            || !session.pending_deliveries.iter().any(|d| d == &pending)
            || !Arc::ptr_eq(
                &policy,
                &app.gcx
                    .privacy_policy_load
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .policy,
            )
        {
            continue;
        }
        // No await from this fresh full native-payload clearance through append.
        if crate::privacy::records::attach_declared_output_files(&app.gcx, &mut result.messages)
            .is_err()
            || crate::agents::launch_spec::clear_current_context(&app.gcx, &result.messages, &model)
                .is_err()
        {
            continue;
        }
        #[cfg(test)]
        tests::landing_hook("before_guard", &session);
        // Preserve the existing Session -> configs lock order (custody rebind
        // and session_runtime). No await or session acquisition in this callback.
        let append = |config: Option<&crate::subchat::SubchatConfig>| {
            if agent_id.is_some()
                && !config.is_some_and(|config| {
                    !matches!(config.tools, crate::subchat::ToolsPolicy::All)
                        && crate::agents::tool_authority::allows_dispatch(
                            &config.tools,
                            &reference.tool_name,
                        )
                })
            {
                return false;
            }
            let _customization_guard = app
                .gcx
                .project_registry_cache
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if app
                .gcx
                .tool_catalog_generations
                .customization
                .load(std::sync::atomic::Ordering::Acquire)
                != customization
            {
                return false;
            }
            let current_policy = app
                .gcx
                .privacy_policy_load
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if !Arc::ptr_eq(&policy, &current_policy.policy) {
                return false;
            }
            #[cfg(test)]
            tests::landing_hook("compared", &session);
            let mut landed = pending.clone();
            landed.deferred_job = None;
            landed.messages = result.messages;
            session.pending_deliveries.retain(|d| d.id != pending.id);
            session.append_delivery(&landed);
            #[cfg(test)]
            tests::landing_hook("appended", &session);
            true
        };
        let appended = match agent_id.as_deref() {
            Some(id) => app.agent_executions.with_config_read_scope(id, append),
            None => append(None),
        };
        if !appended {
            continue;
        }
        session.emit_queue_update();
        session.mark_persisted_runtime_changed();
        let source_root = crate::coordination::scope::session_source_root(&session);
        let notify = session.queue_notify.clone();
        drop(session);
        drop(sessions);
        let save = super::delivery::schedule_delivery_save(
            app.clone(),
            session_arc.clone(),
            source_root,
            vec![(pending.id.clone(), super::types::DeliveryStatus::Delivered)],
            vec![],
        )
        .await;
        if matches!(save.await, Ok(true)) {
            #[cfg(test)]
            if tests::lose_landing_ack() {
                return;
            }
            let _ = app
                .runtime
                .tool_jobs
                .ack_delivered(&reference.chat_id, &reference.job_id, reference.revision)
                .await;
        }
        if pending.wake {
            notify.notify_one();
        }
    }
}

pub(crate) fn receipt_message(receipt: &JobReceipt) -> ChatMessage {
    let mut message = ChatMessage::new("tool".into(), format!("Accepted background tool job {}; execution is not complete. The result will arrive automatically. Do not replay the invocation.", receipt.job_id));
    message.tool_call_id = receipt.binding.tool_call_id.clone();
    message.extra.insert(
        "tool_job".into(),
        metadata(receipt, JobStatus::Accepted, 1, false),
    );
    // None is deliberate: admission is not execution success.
    message.tool_failed = None;
    message
}
