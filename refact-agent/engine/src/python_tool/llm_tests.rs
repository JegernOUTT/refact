use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::app_state::AppState;
use crate::chat::stream_core::ChoiceFinal;
use crate::tools::tool_python::tests::{call, fixture, with_tools};

fn answer(text: &str, input: usize, output: usize) -> ChoiceFinal {
    ChoiceFinal {
        content: text.into(),
        finish_reason: Some("stop".into()),
        usage: Some(crate::call_validation::ChatUsage {
            prompt_tokens: input,
            completion_tokens: output,
            total_tokens: input + output,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn model(id: &str) -> Arc<crate::caps::ChatModelRecord> {
    Arc::new(crate::caps::ChatModelRecord {
        base: crate::caps::BaseModelRecord {
            id: id.into(),
            tokenizer: "fake".into(),
            n_ctx: 32_000,
            endpoint: "https://example.invalid/v1".into(),
            ..Default::default()
        },
        ..Default::default()
    })
}

/// One test's chat (its own id, so its model fixture is its own) and app.
struct Llm {
    dir: tempfile::TempDir,
    app: AppState,
    chat: String,
}

impl Drop for Llm {
    fn drop(&mut self) {
        crate::subchat::remove_model_fixture(&self.backend());
        crate::python_tool::llm::set_test_timeout(&self.chat, None);
    }
}

impl Llm {
    fn backend(&self) -> String {
        format!("{}-pyllm", self.chat)
    }
    fn reply(&self, responses: Vec<ChoiceFinal>, delay: Option<Duration>) {
        crate::subchat::install_model_fixture(&self.backend(), responses, delay);
    }
    fn replies(
        &self,
        replies: Vec<Result<ChoiceFinal, String>>,
        delay: Option<Duration>,
        gate: Option<Arc<tokio::sync::Semaphore>>,
    ) {
        crate::subchat::install_model_fixture_replies(&self.backend(), replies, delay, gate);
    }
    fn requests(&self) -> Vec<crate::llm::LlmRequest> {
        crate::subchat::model_fixture_requests(&self.backend())
    }
    fn outcomes(&self) -> (Vec<std::time::Instant>, usize) {
        crate::subchat::model_fixture_outcomes(&self.backend())
    }
    fn concurrency(&self) -> (usize, usize) {
        crate::subchat::model_fixture_concurrency(&self.backend())
    }
    async fn run(&self, id: &str, code: &str) -> Value {
        self.run_with_history(&[], id, code).await
    }
    async fn run_with_history(
        &self,
        history: &[crate::call_validation::ChatMessage],
        id: &str,
        code: &str,
    ) -> Value {
        python_of(&self.execute(history, call(id, code, false)).await)
    }
    async fn execute(
        &self,
        history: &[crate::call_validation::ChatMessage],
        tool_call: crate::call_validation::ChatToolCall,
    ) -> Vec<crate::call_validation::ChatMessage> {
        execute_in(
            self.app.clone(),
            self.chat.clone(),
            history.to_vec(),
            tool_call,
        )
        .await
    }
    fn spawn_run(&self, id: &str, code: &str) -> tokio::task::JoinHandle<Value> {
        let (app, chat, tool_call) = (self.app.clone(), self.chat.clone(), call(id, code, false));
        tokio::spawn(async move { python_of(&execute_in(app, chat, vec![], tool_call).await) })
    }
    fn stop(&self, id: &str) {
        self.app.python_tool.stop_run(&self.chat, id).unwrap();
    }
}

fn python_of(messages: &[crate::call_validation::ChatMessage]) -> Value {
    let mut python = messages[0].extra["python"].clone();
    python["_text"] = json!(messages[0].content.content_text_only());
    python
}

async fn execute_in(
    app: AppState,
    chat: String,
    history: Vec<crate::call_validation::ChatMessage>,
    tool_call: crate::call_validation::ChatToolCall,
) -> Vec<crate::call_validation::ChatMessage> {
    let thread = app.chat.sessions.read().await[&chat]
        .lock()
        .await
        .thread
        .clone();
    tokio::time::timeout(
        Duration::from_secs(20),
        crate::chat::tools::execute_tools(
            app,
            &[tool_call],
            &history,
            &thread,
            "agent",
            Some("model"),
            crate::chat::tools::ExecuteToolsOptions {
                structured_output: true,
                ..Default::default()
            },
        ),
    )
    .await
    .expect("python run exceeded 20 seconds")
    .0
}

/// Chat model `model`; slots: light `cheap/light`, default `big/default`, thinking
/// `deep/thinking`, each a different model so that a wrong slot fails the test.
async fn llm_fixture(chat: &str, settings: &str) -> Llm {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        settings,
    )
    .unwrap();
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in ["model", "cheap/light", "big/default", "deep/thinking"] {
        caps.chat_models.insert(id.into(), model(id));
    }
    caps.defaults.chat_default_model = "big/default".into();
    caps.defaults.chat_light_model = "cheap/light".into();
    caps.defaults.chat_thinking_model = "deep/thinking".into();
    {
        let mut state = app.gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
    }
    let mut session = crate::chat::types::ChatSession::new(chat.into());
    session.thread.model = "model".into();
    app.chat
        .sessions
        .write()
        .await
        .insert(chat.into(), Arc::new(tokio::sync::Mutex::new(session)));
    let llm = Llm {
        dir,
        app: with_tools(app),
        chat: chat.into(),
    };
    crate::subchat::remove_model_fixture(&llm.backend());
    llm
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

fn result(python: &Value) -> Value {
    let dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
    serde_json::from_slice(&std::fs::read(dir.join("result.json")).unwrap()).unwrap()
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn llm_returns_text_usage_and_records_everywhere() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-text", "").await;
        llm.reply(vec![answer("Paris", 12, 3), answer("Rome", 10, 2)], None);
        let (stats_tx, mut stats_rx) = tokio::sync::mpsc::channel(16);
        *llm.app.gcx.llm_stats_sender.lock().unwrap() = Some(stats_tx);
        let python = llm.run(
            "llm-text",
            "r = await llm('Capital of France?', system='Be brief.')\nr2 = await llm(prompt='Capital of Italy?', model='chat')\n[r, r2['text']]",
        )
        .await;
        assert_eq!(python["status"], "completed", "{python}");
        let value = result(&python);
        assert_eq!(value[0]["ok"], true);
        assert_eq!(value[0]["text"], "Paris");
        assert_eq!(value[0]["data"], Value::Null);
        assert_eq!(value[0]["model"], "cheap/light");
        assert_eq!(value[0]["usage"], json!({"input_tokens":12,"output_tokens":3}));
        assert_eq!(value[0]["error"], Value::Null);
        assert_eq!(value[1], "Rome");
        let sent = llm.requests();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].model_id, "cheap/light");
        assert_eq!(sent[1].model_id, "model");
        let roles: Vec<_> = sent[0].messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user"]);
        assert_eq!(sent[0].messages[1].content.content_text_only(), "Capital of France?");
        assert!(sent[0].tools.as_ref().is_none_or(|t| t.is_empty()));
        let calls = python["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["tool"], "llm");
        assert_eq!((calls[0]["line"].clone(), calls[0]["col"].clone()), (json!(1), json!(10)));
        assert_eq!(calls[0]["model"], "cheap/light");
        assert_eq!((calls[0]["tokens_in"].clone(), calls[0]["tokens_out"].clone()), (json!(12), json!(3)));
        let preview: Value = serde_json::from_str(calls[0]["args_preview"].as_str().unwrap()).unwrap();
        assert_eq!(preview, json!({"prompt":"Capital of France?","model":null,"schema":false}));
        assert!(calls[0]["summary"].as_str().unwrap().contains("cheap/light"));
        let totals = json!({"calls":2,"input_tokens":22,"output_tokens":5,"models":["cheap/light","model"]});
        assert_eq!(python["llm"], totals);
        assert_eq!(python["calls_total"], 2);
        let run_dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
        let saved: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
        assert_eq!(saved["llm"], totals);
        let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("calls/1_llm.json")).unwrap()).unwrap();
        assert_eq!(log["prompt"], "Capital of France?");
        assert_eq!(log["system"], "Be brief.");
        assert_eq!(log["answer"], "Paris");
        assert_eq!(log["model"], "cheap/light");
        assert_eq!(log["usage"], json!({"input_tokens":12,"output_tokens":3}));
        assert!(python["_text"].as_str().unwrap().contains("Model calls: 2 (22 input / 5 output tokens; cheap/light, model)"));
        let view = crate::python_tool::workspace_view::workspace_view(&llm.app, &llm.chat).await.unwrap();
        let entry = view.runs.iter().find(|r| Some(r.run_id.as_str()) == python["run_id"].as_str()).unwrap();
        assert_eq!((entry.calls_total, entry.llm_calls), (2, Some(2)));
        assert!(!llm.app.chat.sessions.read().await.keys().any(|id| id.contains("-pyllm-")));
        let mut metered = vec![];
        while let Ok(event) = stats_rx.try_recv() {
            metered.push(event);
        }
        assert_eq!(metered.len(), 2);
        assert!(metered.iter().all(|e| e.success && e.root_chat_id.as_deref() == Some(llm.chat.as_str())));
        assert_eq!(metered.iter().map(|e| e.prompt_tokens).sum::<usize>(), 22);
        *llm.app.gcx.llm_stats_sender.lock().unwrap() = None;
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn schema_answers_are_parsed_validated_and_retried_once() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-schema", "").await;
        llm.reply(
            vec![
                answer("```json\n{\"label\": \"bug\"}\n```", 5, 1),
                answer("{\"label\": 3}", 5, 1),
                answer("{\"label\": \"feature\"}", 5, 1),
                answer("nope", 5, 1),
                answer("still nope", 5, 1),
            ],
            None,
        );
        let python = llm.run(
            "llm-schema",
            "s = {'type':'object','required':['label'],'properties':{'label':{'type':'string'}}}\n[await llm('a', schema=s), await llm('b', schema=s), await llm('c', schema=s)]",
        )
        .await;
        let value = result(&python);
        assert_eq!(value[0]["data"], json!({"label":"bug"}));
        assert_eq!(value[1]["data"], json!({"label":"feature"}));
        assert_eq!(value[1]["usage"], json!({"input_tokens":10,"output_tokens":2}));
        assert_eq!(value[2]["ok"], false);
        assert_eq!(value[2]["text"], "still nope");
        assert!(value[2]["error"].as_str().unwrap().starts_with("response did not match schema: "));
        let sent = llm.requests();
        assert_eq!(sent.len(), 5);
        assert!(sent[0].messages[0].content.content_text_only().contains("JSON Schema"));
        assert!(sent[2].messages.last().unwrap().content.content_text_only().contains("did not match"));
        assert_eq!(python["calls"][2]["status"], "failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn bad_arguments_raise_type_error_and_nothing_is_sent() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-bad", "").await;
        llm.reply(vec![], None);
        let code = "out = []\nfor kw in [dict(), dict(prompt=''), dict(prompt=1), dict(prompt='q', temperature=1), dict(prompt='q', model='gpt'), dict(prompt='q', schema=[1]), dict(prompt='q', max_tokens=0), dict(prompt='x' * 400001)]:\n    try:\n        await llm(**kw)\n        out.append('ok')\n    except TypeError:\n        out.append('TypeError')\ntry:\n    await llm('a', 'b')\nexcept TypeError as e:\n    out.append(str(e))\nout";
        let python = llm.run("llm-bad", code).await;
        let value = result(&python);
        let items = value.as_array().unwrap();
        assert!(items[..8].iter().all(|v| v == "TypeError"), "{value}");
        assert!(items[8].as_str().unwrap().contains("positional"), "{value}");
        assert!(llm.requests().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn call_and_token_budgets_raise_runtime_error() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let code = "await llm('a')\nmsg = None\ntry:\n    await llm('b')\nexcept RuntimeError as e:\n    msg = str(e)\nmsg";
        let llm = llm_fixture("pyllm-calls", "python_max_llm_calls: 1\n").await;
        llm.reply(vec![answer("one", 1, 1)], None);
        let python = llm.run("llm-calls", code).await;
        assert!(result(&python).as_str().is_some_and(|m| m.contains("call budget")), "{python}");
        assert_eq!(llm.requests().len(), 1);
        let llm = llm_fixture("pyllm-tokens", "python_max_llm_tokens: 1000\n").await;
        llm.reply(vec![answer("big", 1400, 100)], None);
        let python = llm.run("llm-tokens", code).await;
        assert!(result(&python).as_str().is_some_and(|m| m.contains("token budget")), "{python}");
        assert_eq!(llm.requests().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn model_calls_are_not_charged_to_the_nested_call_budget() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-nested", "python_max_nested_calls: 1\n").await;
        llm.reply((0..3).map(|i| answer(&format!("a{i}"), 1, 1)).collect(), None);
        let python = llm
            .run(
                "llm-nested",
                "out = [(await llm('a'))['text'], (await llm('b'))['text']]\nout.append((await fixture())['ok'])\nout.append((await llm('c'))['text'])\ntry:\n    await fixture()\nexcept RuntimeError as e:\n    out.append(str(e))\nout",
            )
            .await;
        assert_eq!(
            result(&python),
            json!(["a0", "a1", true, "a2", "nested call budget exhausted"]),
            "{python}"
        );
        assert_eq!(llm.requests().len(), 3);
        assert_eq!(python["llm"]["calls"], 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn provider_errors_return_ok_false_with_the_error_text() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-provider-error", "").await;
        llm.replies(
            vec![
                Err("invalid_request_error: the prompt mentions a forbidden word".into()),
                Ok(answer("fine", 2, 1)),
            ],
            None,
            None,
        );
        let python = llm
            .run(
                "llm-provider-error",
                "[await llm('a', model='chat'), (await llm('b', model='chat'))['ok']]",
            )
            .await;
        assert_eq!(python["status"], "completed", "{python}");
        let value = result(&python);
        assert_eq!(value[0]["ok"], false, "{value}");
        assert!(
            value[0]["error"]
                .as_str()
                .unwrap()
                .contains("forbidden word"),
            "{value}"
        );
        assert_eq!(
            (value[0]["text"].clone(), value[0]["data"].clone()),
            (json!(""), Value::Null)
        );
        assert_eq!(value[0]["model"], "model");
        assert_eq!(value[1], true);
        assert_eq!(python["calls"][0]["status"], "failed");
        assert_eq!(llm.requests().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn timeout_returns_ok_false_and_covers_the_schema_retry() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-timeout", "").await;
        let timeout = Duration::from_secs(4);
        crate::python_tool::llm::set_test_timeout(&llm.chat, Some(timeout));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        llm.replies(
            vec![Ok(answer("not json", 4, 1)), Ok(answer("{\"n\":1}", 4, 1))],
            None,
            Some(gate.clone()),
        );
        let worker = llm.spawn_run(
            "llm-timeout",
            "await llm('a', model='chat', schema={'type':'object','required':['n']})",
        );
        until("the first attempt", || llm.requests().len() == 1).await;
        // Half of the call's time goes to the first attempt; the retry gets only the rest.
        tokio::time::sleep(timeout / 2).await;
        let released = std::time::Instant::now();
        gate.add_permits(1);
        until("the repair retry", || llm.requests().len() == 2).await;
        let python = worker.await.unwrap();
        let value = result(&python);
        assert_eq!(value["ok"], false, "{value}");
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("timed out after 4 s"),
            "{value}"
        );
        assert_eq!(value["text"], "not json");
        assert_eq!(python["calls"][0]["status"], "failed");
        assert_eq!(python["calls"][0]["tokens_in"], 8, "{python}");
        until("the abort of the timed-out retry", || {
            llm.outcomes().0.len() == 1
        })
        .await;
        let (aborted, completed) = llm.outcomes();
        assert_eq!(completed, 1);
        let retry_time = aborted[0].duration_since(released);
        assert!(
            retry_time < timeout * 7 / 8,
            "the retry got its own {timeout:?}: {retry_time:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn model_slots_resolve_to_their_own_models() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let code = "out = []\nfor m in ['default', 'thinking', 'light', 'chat']:\n    out.append((await llm('q', model=m))['model'])\nout";
        let llm = llm_fixture("pyllm-slots", "").await;
        llm.reply((0..4).map(|_| answer("x", 1, 1)).collect(), None);
        let python = llm.run("llm-slots", code).await;
        let expected = json!(["big/default", "deep/thinking", "cheap/light", "model"]);
        assert_eq!(result(&python), expected, "{python}");
        let sent: Vec<_> = llm.requests().iter().map(|r| r.model_id.clone()).collect();
        assert_eq!(json!(sent), expected);
        let llm = llm_fixture("pyllm-unset-slots", "").await;
        {
            let mut state = llm.app.gcx.caps_state.write().await;
            let mut caps = (**state.caps.as_ref().unwrap()).clone();
            caps.defaults.chat_default_model = String::new();
            caps.defaults.chat_thinking_model = String::new();
            caps.defaults.chat_light_model = String::new();
            state.caps = Some(Arc::new(caps));
        }
        llm.reply((0..4).map(|_| answer("x", 1, 1)).collect(), None);
        let python = llm.run("llm-unset-slots", code).await;
        assert_eq!(result(&python), json!(["model", "model", "model", "model"]), "{python}");
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn privacy_gate_refuses_before_sending_and_default_falls_back_to_chat_model() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-privacy", "").await;
        zone_a_policy(&llm.dir, &llm.app).await;
        llm.reply(vec![], None);
        let python = llm.run(
            "llm-refused",
            "x = open('zone_a.txt').read()\nmsg = None\ntry:\n    await llm('summarise ' + x, model='light')\nexcept PermissionError as e:\n    msg = str(e)\nmsg",
        )
        .await;
        let message = result(&python);
        let message = message.as_str().unwrap_or_else(|| panic!("{python}"));
        assert!(message.contains("\"cheap\"") && message.contains("zone_a"), "{message}");
        assert!(llm.requests().is_empty());
        assert_eq!(python["calls"][0]["status"], "denied");
        llm.reply(vec![answer("ok", 3, 1)], None);
        let python = llm.run("llm-fallback", "x = open('zone_a.txt').read()\n(await llm('summarise ' + x))['model']").await;
        assert_eq!(result(&python), "model", "{python}");
        assert_eq!(llm.requests().len(), 1);
        assert_eq!(llm.requests()[0].model_id, "model");
    })
    .await
    .unwrap();
}

/// Each reply waits for a permit; the fixture counts the requests it holds at once.
async fn max_parallel_requests(chat: &str, settings: &str, calls: usize, expected: usize) -> usize {
    let llm = llm_fixture(chat, settings).await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    llm.replies(
        (0..calls).map(|_| Ok(answer("x", 1, 1))).collect(),
        None,
        Some(gate.clone()),
    );
    let worker = llm.spawn_run(
        "llm-gather",
        &format!("import asyncio\nawait asyncio.gather(*[llm(str(i)) for i in range({calls})])"),
    );
    for released in 0..calls {
        until("the next held requests", || {
            llm.concurrency().0 == expected.min(calls - released)
        })
        .await;
        gate.add_permits(1);
        until("the released request to finish", || {
            llm.outcomes().1 == released + 1
        })
        .await;
    }
    let python = worker.await.unwrap();
    assert_eq!(python["status"], "completed", "{python}");
    llm.concurrency().1
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn gather_runs_calls_in_parallel_within_the_semaphore() {
    tokio::time::timeout(Duration::from_secs(90), async {
        assert_eq!(max_parallel_requests("pyllm-gather", "", 3, 3).await, 3);
        assert_eq!(
            max_parallel_requests("pyllm-serial", "python_max_parallel_calls: 1\n", 3, 1).await,
            1
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn stop_cancels_an_in_flight_call() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-stop", "").await;
        llm.reply(vec![answer("late", 1, 1)], Some(Duration::from_secs(30)));
        let worker = llm.spawn_run("llm-stop", "await llm('slow')");
        until("the request", || !llm.requests().is_empty()).await;
        llm.stop("llm-stop");
        let python = worker.await.unwrap();
        assert_eq!(python["status"], "cancelled");
        assert_eq!(python["calls"][0]["status"], "cancelled");
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn stop_aborts_the_provider_request_and_keeps_reported_usage() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-abort", "").await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        llm.replies(
            vec![Ok(answer("late", 7, 50)), Ok(answer("late", 9, 50))],
            None,
            Some(gate.clone()),
        );
        let worker = llm.spawn_run(
            "llm-abort",
            "import asyncio\nawait asyncio.gather(llm('a'), llm('b'))",
        );
        until("both requests held by the provider", || {
            llm.concurrency().0 == 2
        })
        .await;
        llm.stop("llm-abort");
        let python = worker.await.unwrap();
        assert_eq!(python["status"], "cancelled", "{python}");
        until("the provider to see both aborts", || {
            llm.outcomes().0.len() == 2
        })
        .await;
        assert_eq!(llm.outcomes().1, 0, "a completion arrived after Stop");
        let calls = python["calls"].as_array().unwrap();
        assert!(calls.iter().all(|c| c["status"] == "cancelled"), "{python}");
        let mut tokens_in: Vec<_> = calls
            .iter()
            .map(|c| c["tokens_in"].as_u64().unwrap())
            .collect();
        tokens_in.sort();
        assert_eq!(tokens_in, [7, 9], "{python}");
        assert_eq!(python["llm"]["input_tokens"], 16, "{python}");
        let run_dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
        let saved: Value =
            serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
        assert_eq!(saved["llm"]["input_tokens"], 16);
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn stop_during_the_schema_retry_keeps_the_first_attempt_in_the_record() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-retry-stop", "").await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        llm.replies(
            vec![Ok(answer("not json", 11, 4)), Ok(answer("{\"n\":1}", 6, 2))],
            None,
            Some(gate.clone()),
        );
        let worker = llm.spawn_run(
            "llm-retry-stop",
            "await llm('a', schema={'type':'object','required':['n']})",
        );
        until("the first attempt", || llm.requests().len() == 1).await;
        gate.add_permits(1);
        until("the repair retry", || {
            llm.concurrency().0 == 1 && llm.requests().len() == 2
        })
        .await;
        llm.stop("llm-retry-stop");
        let python = worker.await.unwrap();
        assert_eq!(python["status"], "cancelled", "{python}");
        let call = &python["calls"][0];
        assert_eq!(call["status"], "cancelled", "{python}");
        assert_eq!(
            (call["tokens_in"].clone(), call["tokens_out"].clone()),
            (json!(17), json!(4)),
            "{python}"
        );
        assert_eq!(python["llm"]["input_tokens"], 17, "{python}");
        assert_eq!(python["llm"]["output_tokens"], 4, "{python}");
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn max_tokens_defaults_to_8192_and_is_capped_at_the_model_limit() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-max-tokens", "").await;
        {
            let mut state = llm.app.gcx.caps_state.write().await;
            let mut caps = (**state.caps.as_ref().unwrap()).clone();
            let mut limited = (*model("cheap/light")).clone();
            limited.max_output_tokens = Some(4000);
            caps.chat_models.insert("cheap/light".into(), Arc::new(limited));
            state.caps = Some(Arc::new(caps));
        }
        llm.reply((0..3).map(|_| answer("x", 1, 1)).collect(), None);
        let python = llm.run(
            "llm-max-tokens",
            "[(await llm('a', model='chat'))['ok'], (await llm('b', model='chat', max_tokens=300))['ok'], (await llm('c', max_tokens=100000))['ok']]",
        )
        .await;
        assert_eq!(result(&python), json!([true, true, true]), "{python}");
        let sent: Vec<_> = llm.requests().iter().map(|r| (r.model_id.clone(), r.params.max_tokens)).collect();
        assert_eq!(
            sent,
            [("model".to_string(), 8192), ("model".to_string(), 300), ("cheap/light".to_string(), 4000)]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn helper_is_absent_when_the_call_budget_is_zero() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-off", "python_max_llm_calls: 0\n").await;
        let python = llm.run("llm-off", "await llm('a')").await;
        assert_eq!(python["error"]["type"], "NameError", "{python}");
        let tools: Vec<_> = llm
            .app
            .tool_registry
            .acquire_tool_catalog("agent", Some("model"), None)
            .await
            .index
            .tools
            .iter()
            .filter(|t| t.name == "python")
            .map(|t| t.description.clone())
            .collect();
        assert!(tools.iter().all(|d| !d.contains("async def llm(")));
    })
    .await
    .unwrap();
}

fn all_files(root: &std::path::Path) -> std::collections::BTreeSet<std::path::PathBuf> {
    let mut out = std::collections::BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path);
            }
        }
    }
    out
}

/// The file set once two listings 200 ms apart agree (writers have settled).
async fn settled_files(root: &std::path::Path) -> std::collections::BTreeSet<std::path::PathBuf> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut last = all_files(root);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let now = all_files(root);
            if now == last {
                return now;
            }
            last = now;
        }
    })
    .await
    .expect("files kept changing for 15 s")
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn script_model_calls_write_no_internal_trace() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-trace", "").await;
        llm.reply(vec![answer("one", 1, 1), answer("two", 1, 1)], None);
        llm.run("llm-trace-warm", "1").await;
        let before = settled_files(llm.dir.path()).await;
        let python = llm
            .run(
                "llm-trace",
                "[(await llm('a'))['text'], (await llm('b'))['text']]",
            )
            .await;
        assert_eq!(result(&python), json!(["one", "two"]), "{python}");
        let workspace = std::fs::canonicalize(python["workspace_dir"].as_str().unwrap()).unwrap();
        let added: Vec<_> = settled_files(llm.dir.path())
            .await
            .difference(&before)
            .filter(|p| !std::fs::canonicalize(p).unwrap().starts_with(&workspace))
            .cloned()
            .collect();
        let index = std::fs::read_to_string(
            llm.dir
                .path()
                .join("project/.refact/trajectories/index.json"),
        )
        .unwrap_or_default();
        assert!(
            added.is_empty(),
            "internal traces written: {added:?} {index}"
        );
    })
    .await
    .unwrap();
}

async fn zone_a_policy(dir: &tempfile::TempDir, app: &AppState) -> std::path::PathBuf {
    let file = dir.path().join("project/zone_a.txt");
    std::fs::write(&file, "guarded").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec!["zone_a.txt".into()],
        send_to: vec!["model".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    file
}

/// Chat history whose tool result came from a direct `cat` of a guarded file.
fn guarded_history(
    app: &AppState,
    file: &std::path::Path,
) -> Vec<crate::call_validation::ChatMessage> {
    let mut read = crate::call_validation::ChatMessage::new("tool".into(), "guarded".into());
    read.tool_call_id = "cat-1".into();
    let record = crate::privacy::records::declared_file_record(&app.gcx, file).unwrap();
    crate::privacy::records::merge_records(&mut read, [record]);
    vec![
        crate::call_validation::ChatMessage::new("user".into(), "read it".into()),
        serde_json::from_value(json!({"role":"assistant","content":"","tool_calls":[{"id":"cat-1","type":"function","function":{"name":"cat","arguments":"{}"}}]})).unwrap(),
        read,
    ]
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn chat_history_records_gate_model_calls() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-history", "").await;
        let file = zone_a_policy(&llm.dir, &llm.app).await;
        let history = guarded_history(&llm.app, &file);
        llm.reply(vec![], None);
        let python = llm.run_with_history(
            &history,
            "llm-history-refused",
            "msg = None\ntry:\n    await llm('summarise: guarded', model='light')\nexcept PermissionError as e:\n    msg = str(e)\nmsg",
        )
        .await;
        let message = result(&python);
        let message = message.as_str().unwrap_or_else(|| panic!("{python}"));
        assert!(message.contains("\"cheap\"") && message.contains("zone_a"), "{message}");
        assert!(llm.requests().is_empty());
        llm.reply(vec![answer("ok", 3, 1)], None);
        let python = llm.run_with_history(
            &history,
            "llm-history-fallback",
            "(await llm('summarise: guarded'))['model']",
        )
        .await;
        assert_eq!(result(&python), "model", "{python}");
        assert_eq!(llm.requests().len(), 1);
        assert_eq!(llm.requests()[0].model_id, "model");
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn engine_written_run_files_inherit_the_run_zone_across_restart() {
    tokio::time::timeout(Duration::from_secs(90), async {
        let llm = llm_fixture("pyllm-zone", "").await;
        zone_a_policy(&llm.dir, &llm.app).await;
        llm.reply(vec![answer("ok", 3, 1)], None);
        let first = llm.run(
            "zone-writer",
            "x = open('zone_a.txt').read()\nshow([{'v': x}])\nprint(x)\nawait llm('echo ' + x, model='chat')\nx",
        )
        .await;
        assert_eq!(first["status"], "completed", "{first}");
        let native_run_dir = std::path::Path::new(first["run_dir"].as_str().unwrap());
        let run_dir = refact_python_kernel::hostfs::virtual_path(
            native_run_dir,
            if cfg!(windows) { refact_python_kernel::hostfs::Platform::Windows } else { refact_python_kernel::hostfs::Platform::Unix },
        );
        let restart = serde_json::from_value::<crate::call_validation::ChatToolCall>(json!({"id":"zone-reader","type":"function","function":{"name":"python","arguments":json!({"code":format!(
            "out = []\nfor name in ['displays/1.json', 'calls/1_llm.json', 'stdout.txt', 'result.json']:\n    out.append(len(open({run_dir:?} + '/' + name).read()))\nout"
        ),"restart":true}).to_string()}})).unwrap();
        let messages = llm.execute(&[], restart).await;
        let second = &messages[0];
        assert_eq!(second.extra["python"]["status"], "completed", "{:?}", second.extra);
        let files = second.extra["privacy"]["files"].as_array().cloned().unwrap_or_default();
        for name in ["displays/1.json", "calls/1_llm.json", "stdout.txt", "result.json"] {
            assert!(
                files.iter().any(|f| f["path"].as_str().is_some_and(|p| dunce::canonicalize(p).ok() == Some(dunce::canonicalize(native_run_dir.join(name)).unwrap())) && f["zone"] != "normal"),
                "{name} read without the run zone: {files:?}"
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn schema_retry_counts_against_and_respects_the_token_budget() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-retry-budget", "python_max_llm_tokens: 1000\n").await;
        llm.reply(
            vec![answer("not json", 900, 200), answer("{\"n\":1}", 1, 1)],
            None,
        );
        let python = llm
            .run(
                "llm-retry-budget",
                "await llm('a', schema={'type':'object','required':['n']})",
            )
            .await;
        let value = result(&python);
        assert_eq!(value["ok"], false, "{python}");
        assert!(
            value["error"].as_str().unwrap().contains("no retry"),
            "{value}"
        );
        assert_eq!(llm.requests().len(), 1);
        assert_eq!(python["llm"]["input_tokens"], 900);
        let llm = llm_fixture("pyllm-retry-counted", "").await;
        llm.reply(
            vec![answer("not json", 7, 1), answer("{\"n\":1}", 5, 2)],
            None,
        );
        let python = llm
            .run(
                "llm-retry-counted",
                "await llm('a', schema={'type':'object','required':['n']})",
            )
            .await;
        assert_eq!(result(&python)["ok"], true, "{python}");
        assert_eq!(python["llm"]["input_tokens"], 12);
        assert_eq!(python["llm"]["output_tokens"], 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn indented_display_files_are_read_by_the_workspace_view() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let llm = llm_fixture("pyllm-show-file", "").await;
        let python = llm
            .run("show-file", "show([{'a': 1}, {'a': 2}], title='T')")
            .await;
        assert_eq!(python["status"], "completed", "{python}");
        let run_dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
        let text = std::fs::read_to_string(run_dir.join("displays/1.json")).unwrap();
        assert!(text.contains("\n    [1],\n    [2]\n"), "{text}");
        let view = crate::python_tool::workspace_view::workspace_view(&llm.app, &llm.chat)
            .await
            .unwrap();
        let shown = &view.displays[0].display;
        assert_eq!(
            (shown.title.as_deref(), shown.rows.as_ref().map(Vec::len)),
            (Some("T"), Some(2))
        );
    })
    .await
    .unwrap();
}
