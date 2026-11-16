use refact_python_kernel::{client::*, protocol::*};
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};

async fn event(c: &mut KernelClient) -> ClientEvent {
    tokio::time::timeout(Duration::from_secs(30), c.next_event())
        .await
        .expect("worker must answer within 30 seconds")
}
async fn client() -> KernelClient {
    let mut c = KernelClient::spawn(
        WorkerCommand {
            program: env!("CARGO_BIN_EXE_refact-python-worker").into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
        },
        ClientOptions {
            memory_limit_bytes: 0,
            ..Default::default()
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), c.next_event())
        .await
        .unwrap();
    c
}
async fn feed(c: &KernelClient, code: &str, limits: Value, globals: Value) {
    let mut all = json!([{"name": "f", "kind": "AsyncFn"}, {"name": "g", "kind": "SyncFn"}]);
    all.as_array_mut()
        .unwrap()
        .extend(globals.as_array().unwrap().iter().cloned());
    c.send(
        serde_json::from_value(json!({"Feed": {
            "run_id": "large", "code": code, "globals": all, "limits": limits
        }}))
        .unwrap(),
    )
    .await
    .unwrap();
}
fn limits(extra: Value) -> Value {
    let mut limits = json!({"max_memory_bytes": 1073741824u64, "max_compute_ms": 10000});
    limits
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    limits
}
async fn answer_async(c: &mut KernelClient, result: Value) {
    let ClientEvent::Kernel(KernelEvent::CallStarted {
        call_id, ..
    }) = event(c).await
    else {
        panic!("expected CallStarted")
    };
    assert!(matches!(
        event(c).await,
        ClientEvent::Kernel(KernelEvent::NeedResults { .. })
    ));
    c.send(KernelCommand::Results {
        run_id: "large".into(),
        results: vec![(call_id, HostResult::Ok(result))],
    })
    .await
    .unwrap();
}
async fn completed(c: &mut KernelClient) -> Value {
    match event(c).await {
        ClientEvent::Kernel(KernelEvent::Completed {
            value,
            truncated,
            ..
        }) => {
            assert!(!truncated, "{value}");
            value
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn result_of_100000_nodes_round_trips_intact() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut c = client().await;
        let items: Vec<Value> = (0..50_000).map(|i| json!({"i": i})).collect();
        feed(
            &c,
            "r = await f()\n[len(r['items']), sum(x['i'] for x in r['items']), r['items'][-1]['i']]",
            limits(json!({})),
            json!([]),
        )
        .await;
        answer_async(&mut c, json!({"items": items})).await;
        assert_eq!(
            completed(&mut c).await,
            json!([50_000, 49_999i64 * 50_000 / 2, 49_999])
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn result_of_5_mib_round_trips_intact() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut c = client().await;
        let text = "é".repeat(5 * 1024 * 1024 / 2) + "end";
        feed(
            &c,
            "r = await f()\n[len(r['text']), r['text'][-3:]]",
            limits(json!({})),
            json!([]),
        )
        .await;
        answer_async(&mut c, json!({"ok": true, "text": text})).await;
        assert_eq!(
            completed(&mut c).await,
            json!([5 * 1024 * 1024 / 2 + 3, "end"])
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn over_limit_results_raise_in_the_script() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut c = client().await;
        let catch = "try:\n    r = {}\nexcept RuntimeError as e:\n    r = 'raised: ' + str(e)\nr";
        feed(
            &c,
            &catch.replace("{}", "await f()"),
            limits(json!({"max_result_bytes": 100_000, "max_result_nodes": 1_000_000})),
            json!([]),
        )
        .await;
        answer_async(&mut c, json!({"text": "x".repeat(200_000)})).await;
        let message = completed(&mut c).await;
        let message = message.as_str().unwrap_or_else(|| panic!("{message}"));
        assert!(
            message.starts_with("raised: f() returned a result of 200011 bytes"),
            "{message}"
        );
        assert!(message.contains("100000 bytes"), "{message}");
        assert!(message.contains("narrow the call"), "{message}");

        feed(
            &c,
            &catch.replace("{}", "await f()"),
            limits(json!({"max_result_nodes": 1000})),
            json!([]),
        )
        .await;
        answer_async(&mut c, json!((0..2000).collect::<Vec<_>>())).await;
        let message = completed(&mut c).await;
        assert!(
            message.as_str().unwrap().contains("2001 values")
                && message.as_str().unwrap().contains("1000 values"),
            "{message}"
        );

        let mut deep = json!(0);
        for _ in 0..40 {
            deep = json!([deep]);
        }
        feed(
            &c,
            &catch.replace("{}", "await f()"),
            limits(json!({})),
            json!([]),
        )
        .await;
        answer_async(&mut c, deep).await;
        let message = completed(&mut c).await;
        assert!(
            message.as_str().unwrap().contains("levels deep"),
            "{message}"
        );

        feed(
            &c,
            &catch.replace("{}", "g()"),
            limits(json!({"max_result_bytes": 100_000})),
            json!([]),
        )
        .await;
        let ClientEvent::Kernel(KernelEvent::SyncCall {
            call_id, ..
        }) = event(&mut c).await
        else {
            panic!("expected SyncCall")
        };
        c.send(KernelCommand::SyncResult {
            call_id,
            result: HostResult::Ok(json!("y".repeat(300_000))),
        })
        .await
        .unwrap();
        let message = completed(&mut c).await;
        assert!(
            message
                .as_str()
                .unwrap()
                .starts_with("raised: g() returned a result of 300002 bytes"),
            "{message}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn oversized_json_global_fails_the_run_instead_of_binding_a_placeholder() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut c = client().await;
        feed(
            &c,
            "len(BIG)",
            limits(json!({"max_result_bytes": 100_000})),
            json!([{"name": "BIG", "kind": {"Json": "z".repeat(200_000)}}]),
        )
        .await;
        match event(&mut c).await {
            ClientEvent::Kernel(KernelEvent::Raised {
                exc_type,
                message,
                ..
            }) => {
                assert_eq!(exc_type, "RuntimeError");
                assert!(message.contains("BIG"), "{message}");
                assert!(message.contains("200002 bytes"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        feed(
            &c,
            "len(SMALL)",
            limits(json!({})),
            json!([{"name": "SMALL", "kind": {"Json": "z".repeat(300_000)}}]),
        )
        .await;
        assert_eq!(completed(&mut c).await, json!(300_000));
    })
    .await
    .unwrap();
}

#[test]
fn older_run_limits_frames_get_result_budget_defaults() {
    let limits: RunLimits =
        serde_json::from_value(json!({"max_memory_bytes": 42, "max_compute_ms": 7})).unwrap();
    assert_eq!(limits.max_result_bytes, 16 * 1024 * 1024);
    assert_eq!(limits.max_result_nodes, 8 * 1024 * 1024 + 1);
    let frame: KernelCommand = serde_json::from_value(json!({"Feed": {
        "run_id": "old", "code": "1", "globals": [],
        "limits": {"max_memory_bytes": 1, "max_compute_ms": 1, "max_value_nodes": 5}
    }}))
    .unwrap();
    let KernelCommand::Feed {
        limits, ..
    } = frame
    else {
        panic!()
    };
    assert_eq!(
        (limits.max_value_nodes, limits.max_result_bytes),
        (5, 16 * 1024 * 1024)
    );
}

#[test]
fn every_value_within_the_byte_budget_fits_the_node_budget() {
    use refact_python_kernel::convert::{fits_result, json_size, result_nodes_for};
    for n in [0usize, 1, 2, 3, 1000] {
        let ints = json!(vec![0; n]);
        let size = json_size(&ints);
        let bytes = size.bytes as u64;
        assert!(fits_result(&size, result_nodes_for(bytes), bytes), "{n}");
        assert!(
            !fits_result(&size, result_nodes_for(bytes), bytes - 1),
            "{n}"
        );
    }
    let dicts = json!((0..1000).map(|_| json!({})).collect::<Vec<_>>());
    let size = json_size(&dicts);
    assert!(fits_result(
        &size,
        result_nodes_for(size.bytes as u64),
        size.bytes as u64
    ));
}

#[test]
fn json_size_matches_serialized_bytes() {
    use refact_python_kernel::convert::json_size;
    let value = json!({"a\"b": [1, -2, 3.5, 1e300, u64::MAX, null, true, false, "\u{1}\n\té"], "": {}, "e": []});
    let size = json_size(&value);
    assert_eq!(size.bytes, serde_json::to_vec(&value).unwrap().len());
    assert_eq!(size.nodes, 13);
    assert_eq!(size.depth, 2);
}

#[tokio::test]
async fn parallel_results_over_the_frame_limit_raise_instead_of_stalling() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut c = client().await;
        feed(
            &c,
            "import asyncio\ntry:\n    out = [len(r['text']) for r in await asyncio.gather(f(), f(), f())]\nexcept RuntimeError as e:\n    out = 'raised: ' + str(e)\nout",
            limits(json!({"max_result_bytes": 32 * 1024 * 1024})),
            json!([]),
        )
        .await;
        let mut ids = vec![];
        loop {
            match event(&mut c).await {
                ClientEvent::Kernel(KernelEvent::CallStarted {
                    call_id, ..
                }) => ids.push(call_id),
                ClientEvent::Kernel(KernelEvent::NeedResults { .. }) => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(ids.len(), 3);
        c.send(KernelCommand::Results {
            run_id: "large".into(),
            results: ids
                .iter()
                .map(|id| (*id, HostResult::Ok(json!({"text": "x".repeat(25 * 1024 * 1024)}))))
                .collect(),
        })
        .await
        .unwrap();
        let value = completed(&mut c).await;
        let message = value.as_str().unwrap_or_else(|| panic!("{value:.200}"));
        assert!(message.starts_with("raised: a result of 26214411 bytes"), "{message}");
        assert!(message.contains("together") && message.contains("narrow"), "{message}");
        feed(&c, "r = await f()\nlen(r)", limits(json!({})), json!([])).await;
        answer_async(&mut c, json!([1, 2])).await;
        assert_eq!(completed(&mut c).await, json!(2));
    })
    .await
    .unwrap();
}
