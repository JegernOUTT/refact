//! Shipping-path tests: the tool is real, only the provider transport is a loopback fixture.
// The assistant's tool invocation is seeded as a provider transcript; start_generation
// itself dispatches it and would contact the counting server for any continuation.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::json;
use refact_review_core::*;
use crate::{
    app_state::{AppState, AppToolRegistry},
    call_validation::{ChatMessage, ChatToolCall, ChatToolFunction},
    chat::types::{ChatSession, SessionState},
    tools::tool_review::{ToolReview, ReviewToolKind},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const BODY: &str = "I propose clarifying this Tour explanation.";

struct Provider {
    address: std::net::SocketAddr,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Provider {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 4096];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&bytes[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&request[..end]).unwrap();
                        let len = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                count.fetch_add(1, Ordering::SeqCst);
                let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Ordinary continuation\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            address,
            calls,
            task,
        }
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: AppState,
    session: Arc<Mutex<ChatSession>>,
    root: std::path::PathBuf,
    review: Review,
    args: serde_json::Value,
    provider: Provider,
}

fn mutation(review: &Review, key: &str, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: review.revision,
        request_key: key.into(),
        actor: Actor {
            kind: ActorKind::Agent,
            id: "owner".into(),
        },
        at_ms: 1,
        operation,
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let oid = {
        let repo = git2::Repository::init(&root).unwrap();
        let tree = repo.index().unwrap().write_tree().unwrap();
        let sig = git2::Signature::now("Test", "test@example.test").unwrap();
        let oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "fixture",
                &repo.find_tree(tree).unwrap(),
                &[],
            )
            .unwrap();
        oid.to_string()
    };
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    let jobs = crate::tool_jobs::JobRuntime::open_durable(dir.path().join("jobs"), 16, 1)
        .await
        .unwrap();
    Arc::get_mut(&mut gcx).unwrap().tool_jobs = jobs;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
    let base = AppState::from_gcx(gcx.clone()).await;
    let app = AppState {
        tool_registry: Arc::new(AppToolRegistry::with_fixture_tool_factory(
            gcx,
            Arc::new(|| vec![Box::new(ToolReview(ReviewToolKind::Update))]),
        )),
        ..base
    };
    let provider = Provider::new().await;
    let model_id = "test/proposal-completion";
    let model = Arc::new(crate::caps::ChatModelRecord {
        base: crate::caps::BaseModelRecord {
            id: model_id.into(),
            name: "proposal-completion".into(),
            endpoint: format!("http://{}/v1/chat/completions", provider.address),
            wire_format: crate::llm::WireFormat::OpenaiChatCompletions,
            tokenizer: "fake".into(),
            n_ctx: 8192,
            ..Default::default()
        },
        supports_tools: true,
        supports_temperature: false,
        ..Default::default()
    });
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(model_id.into(), model);
    caps.defaults.chat_default_model = model_id.into();
    {
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let mut session = ChatSession::new("owner".into());
    session.work_project_root = Some(root.clone());
    session.thread.model = model_id.into();
    session.thread.mode = "agent".into();
    session.thread.include_project_info = false;
    session.add_message(ChatMessage::new(
        "system".into(),
        "Use the review tool.".into(),
    ));
    session.add_message(ChatMessage::new(
        "user".into(),
        "Suggest a clearer Tour explanation.".into(),
    ));
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("owner".into(), session.clone());
    let abort = {
        let mut locked = session.lock().await;
        locked.set_runtime_state(SessionState::ExecutingTools, None);
        locked.abort_flag.clone()
    };
    // Real context acquisition registers the investigation checkout required by request preflight.
    let ccx = crate::at_commands::at_commands::AtCommandsContext::new_with_abort(
        app.clone(),
        8192,
        10,
        false,
        vec![],
        "owner".into(),
        None,
        String::new(),
        None,
        None,
        Some(abort),
    )
    .await;
    super::context::set_context(
        Arc::new(Mutex::new(ccx)),
        super::context::SetContextRequest {
            repository: root.to_string_lossy().into(),
            base_ref: oid.clone(),
            candidate_ref: oid,
            policy: CapturePolicy::default(),
            title: "Review".into(),
            target_reference: "local".into(),
            review_id: None,
            expected_revision: None,
            request_key: "fixture".into(),
        },
    )
    .await
    .unwrap();
    let store = ReviewStore::open(&root).unwrap();
    let mut review = store.get_for_chat("owner").unwrap().unwrap();
    let candidate = review.current_candidate_id.clone().unwrap();
    let step = serde_json::from_value(json!({"id":"step","current":{"version":1,"title":"Title","body":"Original","anchors":[],"candidate_id":candidate},"history":[]})).unwrap();
    review = store
        .mutate(
            &review.id,
            mutation(
                &review,
                "tour",
                Operation::PutTourStep {
                    step,
                },
            ),
        )
        .unwrap();
    let target = TourDiscussionTarget {
        candidate_id: candidate,
        step_id: "step".into(),
        version: 1,
        block_id: None,
        block_revision: None,
    };
    let thread = Thread {
        id: "discussion".into(),
        anchors: vec![],
        annotation_id: None,
        target: Some(target.clone()),
        resolved: false,
        contributions: vec![],
    };
    review = store
        .mutate(
            &review.id,
            mutation(
                &review,
                "thread",
                Operation::PutThread {
                    thread,
                },
            ),
        )
        .unwrap();
    let args = json!({"review_id":review.id,"expected_revision":review.revision,"request_key":"proposal","operation":{"op":"propose_tour_revision","target":target,"thread_id":"discussion","change_request":"Clarify","body":BODY,"blocks":[]}});
    let mut assistant = ChatMessage::new("assistant".into(), String::new());
    assistant.tool_calls = Some(vec![ChatToolCall {
        id: "proposal-call".into(),
        index: None,
        function: ChatToolFunction {
            name: "review_update".into(),
            arguments: args.to_string(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }]);
    {
        let mut locked = session.lock().await;
        locked.add_message(assistant);
        locked.set_runtime_state(SessionState::Idle, None);
        locked.planner_pause_pending_tools = true;
    }
    Fixture {
        _dir: dir,
        app,
        session,
        root,
        review,
        args,
        provider,
    }
}

async fn generate(f: &Fixture) {
    tokio::time::timeout(TIMEOUT, async {
        crate::chat::start_generation(f.app.clone(), f.session.clone()).await;
        // Request preflight intentionally parks rather than blocking generation.
        // Drive its shipping queue consumer, never set its readiness decision ourselves.
        loop {
            let notify = f.session.lock().await.queue_notify.clone();
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if crate::chat::take_ready_review_continuation(&f.session).await {
                crate::chat::start_generation(f.app.clone(), f.session.clone()).await;
                continue;
            }
            if f.session.lock().await.review_readiness_pending.is_none() {
                break;
            }
            notified.await;
        }
    })
    .await
    .expect("real generation loop must terminate");
}

async fn assert_canonical(f: &Fixture) {
    let review = ReviewStore::open(&f.root)
        .unwrap()
        .load(&f.review.id)
        .unwrap();
    assert_eq!(review.tour_revision_proposals.len(), 1);
    assert_eq!(review.threads[0].contributions.len(), 1);
    let locked = f.session.lock().await;
    let canonical: Vec<_> = locked
        .messages
        .iter()
        .filter(|m| m.role == "assistant" && m.content.content_text_only() == BODY)
        .collect();
    assert_eq!(canonical.len(), 1, "canonical contribution must land once");
    assert_eq!(
        canonical[0].extra["review"]["contribution_id"],
        review.threads[0].contributions[0].id
    );
    let results: Vec<_> = locked
        .messages
        .iter()
        .filter(|m| m.role == "tool" && m.tool_call_id == "proposal-call")
        .collect();
    assert!(
        !results.is_empty(),
        "tool protocol response must survive completion"
    );
    assert!(results.iter().all(|m| m.tool_failed != Some(true)));
    let completed_results: Vec<_> = locked
        .messages
        .iter()
        .filter(|m| {
            (m.role == "tool" && m.tool_call_id == "proposal-call")
                || (m.role == "event"
                    && m.extra.get("tool_job_result").is_some_and(|metadata| {
                        metadata["original_role"] == "tool"
                            && metadata["tool_name"] == "review_update"
                            && metadata["tool_call_id"] == "proposal-call"
                    }))
        })
        .collect();
    assert!(
        completed_results
            .iter()
            .any(|m| m.tool_failed == Some(false)
                && m.content
                    .content_text_only()
                    .contains("pending_tour_revision_actions")),
        "actual final review result must land, not only job receipt"
    );
    assert_eq!(locked.runtime.state, SessionState::Idle);
    assert!(
        locked.delivery_wake_sources.is_empty(),
        "completed proposal must leave no automatic regeneration wake"
    );
    assert!(locked.command_queue.is_empty());
    assert!(locked.pending_deliveries.is_empty());
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn sync_proposal_shipping_path_lands_canonical_and_never_calls_provider() {
    let f = fixture().await;
    f.app
        .runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_secs(60)));
    generate(&f).await;
    assert_canonical(&f).await;
    assert_eq!(
        f.provider.calls.load(Ordering::SeqCst),
        0,
        "proposal completion must not request an assistant paraphrase"
    );
    // A later real execution must not inherit the old completion permission.
    f.session.lock().await.add_message(ChatMessage::new(
        "user".into(),
        "Now continue normally.".into(),
    ));
    generate(&f).await;
    assert_eq!(
        f.provider.calls.load(Ordering::SeqCst),
        1,
        "later human turn must reach actual provider transport"
    );
    f.app.runtime.review_readiness.shutdown().await.unwrap();
    f.app.runtime.tool_jobs.shutdown().await;
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn async_proposal_shipping_path_lands_archived_final_result_without_provider() {
    let f = fixture().await;
    f.app.runtime.tool_jobs.set_test_inline_budget(None);
    let mut events = f.app.runtime.tool_jobs.subscribe();
    generate(&f).await;
    assert_canonical(&f).await;
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let job = f
        .app
        .runtime
        .tool_jobs
        .proposal_job("owner", "proposal-call")
        .expect("real native job");
    let event = events
        .try_recv()
        .expect("forced background completion must emit event");
    assert_eq!(event.job_id, job.job_id);
    assert_eq!(event.status, crate::tool_jobs::JobStatus::Succeeded);
    assert!(event.result_available);
    let archived = f.app.runtime.tool_jobs.get("owner", &job.job_id).unwrap();
    assert!(
        archived.result.is_none(),
        "one-byte retention threshold must archive native result"
    );
    let retained = f
        .app
        .runtime
        .tool_jobs
        .get_result("owner", &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(retained.result.unwrap().messages.iter().any(|m| m
        .content
        .content_text_only()
        .contains("pending_tour_revision_actions")));
    // Replay delivery must not duplicate canonical output or reactivate completion.
    let review = ReviewStore::open(&f.root)
        .unwrap()
        .load(&f.review.id)
        .unwrap();
    super::chat_bridge::flush_read_only(&f.app, f.root.clone(), &review)
        .await
        .unwrap();
    assert_canonical(&f).await;
    assert!(
        !super::turn_completion::stop_at_boundary(&f.app, &f.session)
            .await
            .unwrap()
    );
    f.app.runtime.review_readiness.shutdown().await.unwrap();
    f.app.runtime.tool_jobs.shutdown().await;
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn failed_real_proposal_tool_continues_to_provider_instead_of_auto_stopping() {
    let mut f = fixture().await;
    f.args["expected_revision"] = json!(0);
    f.session
        .lock()
        .await
        .messages
        .last_mut()
        .unwrap()
        .tool_calls
        .as_mut()
        .unwrap()[0]
        .function
        .arguments = f.args.to_string();
    f.app
        .runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_secs(60)));
    generate(&f).await;
    assert_eq!(
        f.provider.calls.load(Ordering::SeqCst),
        1,
        "failed proposal must take normal provider continuation"
    );
    let review = ReviewStore::open(&f.root)
        .unwrap()
        .load(&f.review.id)
        .unwrap();
    assert!(review.tour_revision_proposals.is_empty());
    assert!(f
        .session
        .lock()
        .await
        .messages
        .iter()
        .any(|m| m.role == "tool"
            && m.tool_call_id == "proposal-call"
            && m.tool_failed == Some(true)));
    f.app.runtime.review_readiness.shutdown().await.unwrap();
    f.app.runtime.tool_jobs.shutdown().await;
}
