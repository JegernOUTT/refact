use refact_python_kernel::{client::*, protocol::*};
use serde_json::json;
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
    tokio::time::timeout(Duration::from_secs(10), c.next_event())
        .await
        .unwrap();
    c
}
async fn feed(c: &KernelClient, code: &str, globals: Vec<GlobalBinding>) {
    c.send(KernelCommand::Feed {
        run_id: "regression".into(),
        code: code.into(),
        globals,
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
}
async fn alive(c: &mut KernelClient) {
    feed(c, "42", vec![]).await;
    assert!(
        matches!(event(c).await, ClientEvent::Kernel(KernelEvent::Completed { value, .. }) if value == json!(42))
    );
}
async fn feed_limits(c: &KernelClient, code: &str, limits: serde_json::Value) {
    c.send(
        serde_json::from_value(json!({"Feed": {
            "run_id": "regression", "code": code, "globals": [
                {"name": "f", "kind": "AsyncFn"},
                {"name": "g", "kind": "SyncFn"}
            ], "limits": limits
        }}))
        .unwrap(),
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn completed_list_of_5000_strings_is_not_truncated_with_default_limits() {
    let mut c = client().await;
    feed(&c, "['file' + str(i) for i in range(5000)]", vec![]).await;
    let ClientEvent::Kernel(KernelEvent::Completed {
        value,
        truncated,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(value.as_array().unwrap().len(), 5000);
    alive(&mut c).await;
}
#[tokio::test]
async fn call_kwargs_of_400_kib_are_not_truncated_with_default_limits() {
    let mut c = client().await;
    feed_limits(
        &c,
        "await f(body='x' * 409600)",
        json!({"max_memory_bytes": 134217728, "max_compute_ms": 1000}),
    )
    .await;
    let ClientEvent::Kernel(KernelEvent::CallStarted {
        kwargs,
        truncated,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(kwargs["body"].as_str().unwrap().len(), 409600);
}
#[tokio::test]
async fn larger_configured_value_budget_is_applied() {
    let mut c = client().await;
    feed_limits(
        &c,
        "'x' * 3000000",
        json!({"max_memory_bytes": 134217728, "max_compute_ms": 1000, "max_value_bytes": 4000000}),
    )
    .await;
    let ClientEvent::Kernel(KernelEvent::Completed {
        value,
        truncated,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(value.as_str().unwrap().len(), 3000000);
    feed(&c, "'x' * 3000000", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Completed {
            truncated: true,
            ..
        })
    ));
    alive(&mut c).await;
}
#[tokio::test]
async fn sync_args_use_default_argument_budget() {
    let mut c = client().await;
    feed_limits(
        &c,
        "g('x' * 3000000)",
        json!({"max_memory_bytes": 134217728, "max_compute_ms": 1000}),
    )
    .await;
    let ClientEvent::Kernel(KernelEvent::SyncCall {
        args,
        truncated,
        call_id,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(args[0].as_str().unwrap().len(), 3000000);
    c.send(KernelCommand::SyncResult {
        call_id,
        result: HostResult::Ok(json!(null)),
    })
    .await
    .unwrap();
    event(&mut c).await;
    alive(&mut c).await;
}
#[test]
fn legacy_run_limits_have_conversion_defaults() {
    let limits: RunLimits =
        serde_json::from_value(json!({"max_memory_bytes": 42, "max_compute_ms": 7})).unwrap();
    assert_eq!(limits.max_memory_bytes, 42);
    assert_eq!(limits.max_compute_ms, 7);
    assert_eq!(limits.max_value_nodes, 200_000);
    assert_eq!(limits.max_value_bytes, 2 * 1024 * 1024);
    assert_eq!(limits.max_arg_bytes, 16 * 1024 * 1024);
}
#[tokio::test]
async fn small_configured_budget_truncates() {
    let mut c = client().await;
    for (code, overrides) in [
        ("'x' * 1000", json!({"max_value_bytes": 128})),
        ("list(range(100))", json!({"max_value_nodes": 10})),
        ("await f(body='x' * 1000)", json!({"max_arg_bytes": 128})),
        ("g('x' * 1000)", json!({"max_arg_bytes": 128})),
    ] {
        let mut limits = json!({"max_memory_bytes": 134217728, "max_compute_ms": 1000});
        limits
            .as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        feed_limits(&c, code, limits).await;
        let ClientEvent::Kernel(e) = event(&mut c).await else {
            panic!()
        };
        assert!(match e {
            KernelEvent::Completed {
                truncated, ..
            } => truncated,
            KernelEvent::CallStarted {
                truncated, ..
            } => {
                event(&mut c).await;
                c.send(KernelCommand::Abort {
                    run_id: "regression".into(),
                })
                .await
                .unwrap();
                event(&mut c).await;
                truncated
            }
            KernelEvent::SyncCall {
                truncated,
                call_id,
                ..
            } => {
                c.send(KernelCommand::SyncResult {
                    call_id,
                    result: HostResult::Ok(json!(null)),
                })
                .await
                .unwrap();
                event(&mut c).await;
                truncated
            }
            other => panic!("{other:?}"),
        });
        alive(&mut c).await;
    }
}
#[tokio::test]
async fn budgets_are_clamped_below_frame_limit() {
    let mut c = client().await;
    for code in [
        "'x' * 35000000",
        "await f(body='x' * 35000000)",
        "g('x' * 35000000)",
        "'\\x00' * 12000000",
    ] {
        feed_limits(&c, code, json!({"max_memory_bytes": 134217728, "max_compute_ms": 1000, "max_value_bytes": u64::MAX, "max_arg_bytes": u64::MAX})).await;
        let ClientEvent::Kernel(e) = event(&mut c).await else {
            panic!()
        };
        assert!(matches!(
            e,
            KernelEvent::Completed {
                truncated: true,
                ..
            } | KernelEvent::CallStarted {
                truncated: true,
                ..
            } | KernelEvent::SyncCall {
                truncated: true,
                ..
            }
        ));
        let mut frame = vec![];
        write_frame(&mut frame, &e).unwrap();
        if matches!(e, KernelEvent::CallStarted { .. }) {
            event(&mut c).await;
            c.send(KernelCommand::Abort {
                run_id: "regression".into(),
            })
            .await
            .unwrap();
            event(&mut c).await;
        } else if let KernelEvent::SyncCall {
            call_id, ..
        } = e
        {
            c.send(KernelCommand::SyncResult {
                call_id,
                result: HostResult::Ok(json!(null)),
            })
            .await
            .unwrap();
            event(&mut c).await;
        }
        alive(&mut c).await;
    }
}
#[tokio::test]
async fn completed_dag_is_bounded() {
    let mut c = client().await;
    feed(&c, "a=[0]\nfor i in range(30): a=[a,a]\na", vec![]).await;
    let ClientEvent::Kernel(e) = event(&mut c).await else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(e).unwrap()["Completed"]["truncated"],
        true
    );
    alive(&mut c).await;
}
#[tokio::test]
async fn kwargs_dag_is_bounded() {
    let mut c = client().await;
    feed(
        &c,
        "a=[0]\nfor i in range(24): a=[a,a]\nawait f(x=a)",
        vec![GlobalBinding {
            name: "f".into(),
            kind: GlobalKind::AsyncFn,
        }],
    )
    .await;
    let ClientEvent::Kernel(e) = event(&mut c).await else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(e).unwrap()["CallStarted"]["truncated"],
        true
    );
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::NeedResults { .. })
    ));
    c.send(KernelCommand::Abort {
        run_id: "regression".into(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Cancelled { .. })
    ));
    alive(&mut c).await;
}
#[tokio::test]
async fn inspect_dag_is_bounded() {
    let mut c = client().await;
    feed(&c, "a=[0]\nfor i in range(26): a=[a,a]\nNone", vec![]).await;
    event(&mut c).await;
    c.send(KernelCommand::Inspect).await.unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Variables {
            truncated: false,
            previews_truncated: true,
            ..
        })
    ));
    alive(&mut c).await;
}
#[tokio::test]
async fn huge_completed_preserves_session() {
    let mut c = client().await;
    feed(&c, "'x'*40000000", vec![]).await;
    let ClientEvent::Kernel(e) = event(&mut c).await else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(e).unwrap()["Completed"]["truncated"],
        true
    );
    alive(&mut c).await;
}
#[tokio::test]
async fn deep_completed_preserves_session() {
    let mut c = client().await;
    feed(&c, "a=0\nfor i in range(150): a=[a]\na", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Completed { .. })
    ));
    alive(&mut c).await;
}
#[tokio::test]
async fn user_exception_is_not_compute_limit() {
    let mut c = client().await;
    feed(&c, "raise ValueError('time limit exceeded')", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Raised { .. })
    ));
}
#[tokio::test]
async fn output_has_distinct_limit() {
    let mut c = client().await;
    feed(&c, "print('x'*1100000)", vec![]).await;
    let mut saved = 0usize;
    loop {
        match event(&mut c).await {
            ClientEvent::Kernel(KernelEvent::Stdout {
                text, ..
            }) => saved += text.len(),
            ClientEvent::Kernel(KernelEvent::LimitExceeded {
                kind: LimitKind::Output,
                ..
            }) => break,
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(saved, MAX_STDOUT_BYTES);
}
#[tokio::test]
async fn json_u64_is_python_int() {
    let mut c = client().await;
    feed(
        &c,
        "[type(x).__name__, x == 18446744073709551615]",
        vec![GlobalBinding {
            name: "x".into(),
            kind: GlobalKind::Json(json!(u64::MAX)),
        }],
    )
    .await;
    assert!(
        matches!(event(&mut c).await, ClientEvent::Kernel(KernelEvent::Completed { value, .. }) if value == json!(["int", true]))
    );
}
#[tokio::test]
async fn deep_json_global_is_refused_and_the_worker_survives() {
    let mut v = serde_json::Value::Null;
    for _ in 0..150 {
        v = json!([v]);
    }
    let mut c = client().await;
    let error = c
        .send(KernelCommand::Feed {
            run_id: "regression".into(),
            code: "42".into(),
            globals: vec![GlobalBinding {
                name: "x".into(),
                kind: GlobalKind::Json(v),
            }],
            limits: RunLimits::default(),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("global x") && error.to_string().contains("150 levels"),
        "{error}"
    );
    alive(&mut c).await;
}
#[tokio::test]
async fn stale_abort_is_ignored() {
    let mut c = client().await;
    feed(&c, "1", vec![]).await;
    event(&mut c).await;
    let cmd: KernelCommand = serde_json::from_value(json!({"Abort":{"run_id":"old"}})).unwrap();
    c.send(cmd).await.unwrap();
    alive(&mut c).await;
}
#[tokio::test]
async fn snapshot_hash_checked_before_loading() {
    let mut c = client().await;
    let path = std::env::temp_dir().join(format!("hash-regression-{}", std::process::id()));
    c.send(KernelCommand::Snapshot {
        path: path.to_string_lossy().into_owned(),
    })
    .await
    .unwrap();
    event(&mut c).await;
    let cmd =
        serde_json::from_value(json!({"LoadSnapshot":{"path":path,"expected_sha256":"wrong"}}))
            .unwrap();
    c.send(cmd).await.unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::LoadFailed { .. })
    ));
    std::fs::remove_file(path).unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn malformed_worker_output_kills_worker() {
    let mut c = KernelClient::spawn(WorkerCommand { program: "python3".into(), args: vec!["-c".into(), "import sys,time;sys.stdout.buffer.write(b'\\x01\\x00\\x00\\x00!');sys.stdout.flush();time.sleep(60)".into()], env: HashMap::new(), cwd: None }, ClientOptions::default()).unwrap();
    assert!(format!("{:?}", event(&mut c).await).starts_with("ProtocolError"));
    assert!(matches!(event(&mut c).await, ClientEvent::Exited { .. }));
}
#[cfg(target_os = "linux")]
#[test]
fn runtime_shutdown_reaps_worker() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let c = rt.block_on(async {
        KernelClient::spawn(
            WorkerCommand {
                program: "sleep".into(),
                args: vec!["60".into()],
                env: HashMap::new(),
                cwd: None,
            },
            ClientOptions::default(),
        )
        .unwrap()
    });
    let pid = c.pid().unwrap();
    drop(rt);
    drop(c);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::path::Path::new(&format!("/proc/{pid}")).exists()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn malformed_command_boundary_is_recoverable() {
    use refact_python_kernel::{worker::run_worker, kernel::FakeOs};
    let mut input = vec![1, 0, 0, 0, b'!'];
    write_frame(&mut input, &KernelCommand::Inspect).unwrap();
    write_frame(&mut input, &KernelCommand::Shutdown).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = vec![];
        let result = run_worker(&input[..], &mut output, Box::new(FakeOs::default()));
        tx.send((result, output)).unwrap();
    });
    let (result, output) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    result.unwrap();
    let mut output = &output[..];
    read_frame::<_, KernelEvent>(&mut output).unwrap();
    assert!(matches!(
        read_frame::<_, KernelEvent>(&mut output).unwrap(),
        KernelEvent::ProtocolError { .. }
    ));
    assert!(matches!(
        read_frame::<_, KernelEvent>(&mut output).unwrap(),
        KernelEvent::Variables { .. }
    ));
}
#[tokio::test]
async fn deep_results_raise_and_stale_results_are_ignored() {
    let mut c = client().await;
    feed(
        &c,
        "await f()",
        vec![GlobalBinding {
            name: "f".into(),
            kind: GlobalKind::AsyncFn,
        }],
    )
    .await;
    let ClientEvent::Kernel(KernelEvent::CallStarted {
        call_id, ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    event(&mut c).await;
    let mut v = json!(0);
    for _ in 0..150 {
        v = json!([v]);
    }
    c.send(KernelCommand::Results {
        run_id: "old".into(),
        results: vec![(call_id, HostResult::Ok(json!("stale")))],
    })
    .await
    .unwrap();
    c.send(KernelCommand::Results {
        run_id: "regression".into(),
        results: vec![(call_id, HostResult::Ok(v))],
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Raised{run_id,exc_type,message,..}) if run_id=="regression" && exc_type=="RuntimeError" && message.contains("150 levels"))
    );
    alive(&mut c).await;
}

#[tokio::test]
async fn asyncio_is_available_without_import() {
    let mut c = client().await;
    feed(&c, "await asyncio.gather()", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Completed { .. })
    ));
    c.send(KernelCommand::Reset).await.unwrap();
    event(&mut c).await;
    feed(&c, "await asyncio.gather()", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Completed { .. })
    ));
}

#[tokio::test]
async fn snapshot_load_enforces_passed_state_limit() {
    let mut c = client().await;
    feed(&c, "x = 42", vec![]).await;
    event(&mut c).await;
    let path = std::env::temp_dir().join(format!("pycm-state-limit-{}", std::process::id()));
    c.send(KernelCommand::Snapshot {
        path: path.to_string_lossy().into(),
    })
    .await
    .unwrap();
    let ClientEvent::Kernel(KernelEvent::SnapshotDone {
        bytes,
        sha256,
    }) = event(&mut c).await
    else {
        panic!()
    };
    c.send(KernelCommand::LoadSnapshot {
        path: path.to_string_lossy().into(),
        expected_sha256: sha256.clone(),
        max_state_bytes: bytes - 1,
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await, ClientEvent::Kernel(KernelEvent::LoadFailed {message}) if message.contains("configured state limit"))
    );
    c.send(KernelCommand::LoadSnapshot {
        path: path.to_string_lossy().into(),
        expected_sha256: sha256,
        max_state_bytes: bytes,
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Loaded)
    ));
    feed(&c, "[x, await asyncio.gather()]", vec![]).await;
    assert!(
        matches!(event(&mut c).await, ClientEvent::Kernel(KernelEvent::Completed {value,..}) if value == json!([42, []]))
    );
}

#[tokio::test]
async fn variable_previews_are_120_characters_not_bytes() {
    let mut c = client().await;
    feed(&c, "label = 'é' * 150", vec![]).await;
    event(&mut c).await;
    c.send(KernelCommand::Inspect).await.unwrap();
    let ClientEvent::Kernel(KernelEvent::Variables {
        items,
        truncated,
        total,
        previews_truncated,
    }) = event(&mut c).await
    else {
        panic!()
    };
    let label = items.iter().find(|v| v.name == "label").unwrap();
    assert_eq!(label.preview.chars().count(), 120);
    assert!(previews_truncated);
    assert!(!truncated);
    assert_eq!(total, Some(1));
}
#[tokio::test]
async fn show_receives_keywords_and_a_large_table_within_its_budget() {
    let mut c = client().await;
    c.send(KernelCommand::Feed {
        run_id: "regression".into(),
        code: "rows = [{('c' + str(j)): i * j for j in range(30)} for i in range(10000)]\nshow(rows, title='T', kind='table')\nshow(value='x' * 3, y=['a'])".into(),
        globals: vec![GlobalBinding {
            name: DISPLAY_FUNCTION.into(),
            kind: GlobalKind::SyncFn,
        }],
        limits: RunLimits {
            max_memory_bytes: 1 << 30,
            max_compute_ms: 60_000,
            ..Default::default()
        },
    })
    .await
    .unwrap();
    let ClientEvent::Kernel(KernelEvent::SyncCall {
        args,
        kwargs,
        truncated,
        call_id,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(args[0].as_array().unwrap().len(), 10000);
    assert_eq!(args[0][9999]["c29"], json!(9999 * 29));
    assert_eq!(kwargs, json!({"title": "T", "kind": "table"}));
    c.send(KernelCommand::SyncResult {
        call_id,
        result: HostResult::Ok(json!(null)),
    })
    .await
    .unwrap();
    let ClientEvent::Kernel(KernelEvent::SyncCall {
        args,
        kwargs,
        call_id,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert_eq!(args, json!([]));
    assert_eq!(kwargs, json!({"value": "xxx", "y": ["a"]}));
    c.send(KernelCommand::SyncResult {
        call_id,
        result: HostResult::Ok(json!(null)),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Completed { .. })
    ));
}
#[test]
fn legacy_sync_call_frames_parse_without_kwargs() {
    let event: KernelEvent = serde_json::from_value(json!({"SyncCall": {"truncated": false, "run_id": "r", "call_id": 1, "function": "help", "args": ["x"]}})).unwrap();
    let KernelEvent::SyncCall {
        kwargs, ..
    } = &event
    else {
        panic!()
    };
    assert!(kwargs.is_null());
    assert!(serde_json::to_value(&event).unwrap()["SyncCall"]
        .get("kwargs")
        .is_none());
}
