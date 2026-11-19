use refact_python_kernel::{client::*, protocol::*, kernel::FakeOs};
use serde_json::json;
use std::time::Duration;
async fn event(c: &mut KernelClient) -> ClientEvent {
    tokio::time::timeout(Duration::from_secs(10), c.next_event())
        .await
        .unwrap()
}
#[tokio::test]
async fn last_expression_value_is_returned_as_json() {
    let mut c =
        KernelClient::spawn_in_thread(Box::new(FakeOs::default()), ClientOptions::default())
            .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Ready { .. })
    ));
    c.send(KernelCommand::Feed {
        run_id: "1".into(),
        code: "6 * 7".into(),
        globals: vec![],
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..}) if value==json!(42))
    );
    c.send(KernelCommand::Shutdown).await.unwrap();
    assert!(matches!(event(&mut c).await, ClientEvent::Exited { .. }));
}
async fn client() -> KernelClient {
    let mut c =
        KernelClient::spawn_in_thread(Box::new(FakeOs::default()), ClientOptions::default())
            .unwrap();
    event(&mut c).await;
    c
}
async fn feed(c: &KernelClient, code: &str, globals: Vec<GlobalBinding>) {
    c.send(KernelCommand::Feed {
        run_id: "test".into(),
        code: code.into(),
        globals,
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
}
fn host(name: &str) -> GlobalBinding {
    GlobalBinding {
        name: name.into(),
        kind: GlobalKind::AsyncFn,
    }
}
#[tokio::test]
async fn print_output_is_collected() {
    let mut c = client().await;
    feed(&c, "print('hello')", vec![]).await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Stdout{text,..})if text=="hello\n")
    );
}
#[tokio::test]
async fn async_host_function_round_trip() {
    let mut c = client().await;
    feed(&c, "await f(x=1)", vec![host("f")]).await;
    let ClientEvent::Kernel(KernelEvent::CallStarted {
        call_id,
        kwargs,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert_eq!(kwargs, json!({"x":1}));
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::NeedResults { .. })
    ));
    c.send(KernelCommand::Results {
        run_id: "test".into(),
        results: vec![(call_id, HostResult::Ok(json!(42)))],
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(42))
    );
}
#[tokio::test]
async fn gather_starts_both_calls_before_need_results_and_preserves_order() {
    let mut c = client().await;
    feed(
        &c,
        "import asyncio\nawait asyncio.gather(f(), g())",
        vec![host("f"), host("g")],
    )
    .await;
    let mut ids = vec![];
    for _ in 0..2 {
        let ClientEvent::Kernel(KernelEvent::CallStarted {
            call_id, ..
        }) = event(&mut c).await
        else {
            panic!()
        };
        ids.push(call_id);
    }
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::NeedResults { .. })
    ));
    c.send(KernelCommand::Results {
        run_id: "test".into(),
        results: vec![
            (ids[1], HostResult::Ok(json!(2))),
            (ids[0], HostResult::Ok(json!(1))),
        ],
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!([1,2]))
    );
}
#[tokio::test]
async fn positional_arguments_raise_type_error() {
    let mut c = client().await;
    feed(&c, "await f(1)", vec![host("f")]).await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Raised{exc_type,..})if exc_type=="TypeError")
    );
}
fn positional_host(name: &str, params: &[&str]) -> GlobalBinding {
    GlobalBinding {
        name: name.into(),
        kind: GlobalKind::AsyncFnPositional(params.iter().map(|p| p.to_string()).collect()),
    }
}
async fn first_call_or_error(c: &mut KernelClient) -> Result<serde_json::Value, (String, String)> {
    match event(c).await {
        ClientEvent::Kernel(KernelEvent::CallStarted {
            kwargs, ..
        }) => Ok(kwargs),
        ClientEvent::Kernel(KernelEvent::Raised {
            exc_type,
            message,
            ..
        }) => Err((exc_type, message)),
        other => panic!("unexpected {other:?}"),
    }
}
#[tokio::test]
async fn declared_positional_parameters_map_to_keywords() {
    let mut c = client().await;
    feed(
        &c,
        "await llm('hi', system='s')",
        vec![positional_host("llm", &["prompt"])],
    )
    .await;
    assert_eq!(
        first_call_or_error(&mut c).await.unwrap(),
        json!({"prompt":"hi","system":"s"})
    );
    let mut c = client().await;
    feed(
        &c,
        "await llm(prompt='kw')",
        vec![positional_host("llm", &["prompt"])],
    )
    .await;
    assert_eq!(
        first_call_or_error(&mut c).await.unwrap(),
        json!({"prompt":"kw"})
    );
}
#[tokio::test]
async fn extra_or_duplicate_positional_arguments_raise_type_error() {
    for code in ["await llm('a', 'b')", "await llm('a', prompt='b')"] {
        let mut c = client().await;
        feed(&c, code, vec![positional_host("llm", &["prompt"])]).await;
        let (exc_type, message) = first_call_or_error(&mut c).await.unwrap_err();
        assert_eq!(exc_type, "TypeError", "{code}");
        assert!(message.contains("llm()"), "{message}");
    }
    let mut c = client().await;
    feed(
        &c,
        "await f(1)",
        vec![host("f"), positional_host("llm", &["prompt"])],
    )
    .await;
    let (exc_type, message) = first_call_or_error(&mut c).await.unwrap_err();
    assert_eq!(exc_type, "TypeError");
    assert!(message.contains("keyword arguments only"));
}
#[tokio::test]
async fn python_exception_preserves_repl_state() {
    let mut c = client().await;
    feed(&c, "x=42\n1/0", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Raised { .. })
    ));
    feed(&c, "x", vec![]).await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(42))
    );
}
#[tokio::test]
async fn host_error_result_raises_named_exception() {
    let mut c = client().await;
    feed(
        &c,
        "try:\n    await f()\nexcept PermissionError:\n    answer = 42\nanswer",
        vec![host("f")],
    )
    .await;
    let ClientEvent::Kernel(KernelEvent::CallStarted {
        call_id, ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    event(&mut c).await;
    c.send(KernelCommand::Results {
        run_id: "test".into(),
        results: vec![(
            call_id,
            HostResult::Err {
                exc_type: "PermissionError".into(),
                message: "no".into(),
            },
        )],
    })
    .await
    .unwrap();
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(42))
    );
}
#[tokio::test]
async fn large_operation_reports_memory_limit() {
    let mut c = client().await;
    feed(&c, "'x' * 1000000000", vec![]).await;
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::LimitExceeded {
            kind: LimitKind::Memory,
            ..
        })
    ));
}
#[tokio::test]
async fn compute_limit_stops_infinite_loop() {
    let mut c = client().await;
    c.send(KernelCommand::Feed {
        run_id: "test".into(),
        code: "while True: pass".into(),
        globals: vec![],
        limits: RunLimits {
            max_compute_ms: 200,
            ..RunLimits::default()
        },
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::LimitExceeded {
            kind: LimitKind::Compute,
            ..
        })
    ));
}
#[tokio::test]
async fn abort_at_suspension_cancels_and_keeps_repl() {
    let mut c = client().await;
    feed(&c, "x=42\nawait f()", vec![host("f")]).await;
    event(&mut c).await;
    event(&mut c).await;
    c.send(KernelCommand::Abort {
        run_id: "test".into(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Cancelled { .. })
    ));
    feed(&c, "x", vec![]).await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(42))
    );
}
#[tokio::test]
async fn snapshot_round_trip_restores_variables() {
    let mut c = client().await;
    feed(&c, "x=42", vec![]).await;
    event(&mut c).await;
    let path = std::env::temp_dir()
        .join(format!("kernel-test-{}.snapshot", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    c.send(KernelCommand::Snapshot {
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::SnapshotDone { .. })
    ));
    c.send(KernelCommand::Reset).await.unwrap();
    event(&mut c).await;
    c.send(KernelCommand::LoadSnapshot {
        max_state_bytes: 1024 * 1024 * 1024,
        expected_sha256: {
            use sha2::Digest;
            sha2::Sha256::digest(std::fs::read(&path).unwrap())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        },
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Loaded)
    ));
    feed(&c, "x", vec![]).await;
    assert!(
        matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(42))
    );
    std::fs::remove_file(path).unwrap();
}
#[test]
fn tampered_snapshot_fails_hmac_verification() {
    use refact_python_kernel::snapshot::*;
    let hash = "ab".repeat(32);
    let sig = sign(b"key", &hash, "chat");
    assert!(verify(b"key", &hash, "chat", &sig));
    assert!(!verify(b"key", &"ac".repeat(32), "chat", &sig));
    assert!(!verify(b"key", &hash, "other", &sig));
}
#[tokio::test]
async fn variable_summaries_list_user_globals() {
    let mut c = client().await;
    feed(&c, "answer=42\n_hidden=1", vec![host("f")]).await;
    event(&mut c).await;
    c.send(KernelCommand::Inspect).await.unwrap();
    let ClientEvent::Kernel(KernelEvent::Variables {
        items, ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "answer");
}
#[tokio::test]
async fn globals_are_rebound_on_every_feed() {
    let mut c = client().await;
    for i in 1..3 {
        feed(
            &c,
            "x",
            vec![GlobalBinding {
                name: "x".into(),
                kind: GlobalKind::Json(json!(i)),
            }],
        )
        .await;
        assert!(
            matches!(event(&mut c).await,ClientEvent::Kernel(KernelEvent::Completed{value,..})if value==json!(i))
        );
    }
}
#[tokio::test]
async fn kernel_thread_exits_on_shutdown() {
    let mut c = client().await;
    c.send(KernelCommand::Shutdown).await.unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Exited {
            code: Some(0),
            ..
        }
    ));
}

#[tokio::test]
async fn hostfs_python_file_environment_and_listing_apis() {
    use refact_python_kernel::hostfs::{HostFs, HostFsConfig};
    let root = std::env::temp_dir().join(format!("hostfs-kernel-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut c = KernelClient::spawn_in_thread(
        Box::new(HostFs::new(HostFsConfig {
            project_root: root.clone(),
            env: std::collections::BTreeMap::from([("TEST".into(), "yes".into())]),
            ..Default::default()
        })),
        ClientOptions::default(),
    )
    .unwrap();
    event(&mut c).await;
    feed(&c,"from pathlib import Path\nimport os\nwith open('a', 'w') as f:\n    f.write('abcdef')\nwith open('a') as f:\n    first=f.read(2)\n    f.seek(1)\n    rest=f.read()\n    pos=f.tell()\nclosed=f.closed\n[first,rest,pos,closed,Path('a').read_text(),os.getenv('TEST'),os.listdir('.') ]",vec![]).await;
    loop {
        match event(&mut c).await {
            ClientEvent::Kernel(
                KernelEvent::Fs {
                    ..
                }
                | KernelEvent::WriteCapture {
                    ..
                },
            ) => {}
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => {
                assert_eq!(
                    value,
                    json!(["ab", "bcdef", 6, true, "abcdef", "yes", ["a"]])
                );
                break;
            }
            e => panic!("unexpected {e:?}"),
        }
    }
    c.send(KernelCommand::Shutdown).await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn hostfs_import_glob_append_and_reconfiguration() {
    use refact_python_kernel::hostfs::HostFs;
    let root = std::env::temp_dir().join(format!("hostfs-import-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("local_module.py"), "answer = 42\n").unwrap();
    let mut c =
        KernelClient::spawn_in_thread(Box::new(HostFs::default()), ClientOptions::default())
            .unwrap();
    event(&mut c).await;
    for iteration in 0..2 {
        c.send(KernelCommand::Configure {
            project_root: root.clone(),
            env: Default::default(),
            capture_dir: Some(root.join("captures")),
            deny_write: Default::default(),
            deny_read: Default::default(),
        })
        .await
        .unwrap();
        feed(&c,"from pathlib import Path\nimport local_module\nwith open('log','a') as f:\n    f.write('line\\n')\n[local_module.answer, sorted(p.name for p in Path('.').glob('*.py'))]",vec![]).await;
        let mut captured = false;
        loop {
            match event(&mut c).await {
                ClientEvent::Kernel(KernelEvent::WriteCapture {
                    original, ..
                }) => {
                    captured = true;
                    if iteration == 0 {
                        assert!(matches!(original, WriteOriginal::Missing));
                    } else {
                        assert!(matches!(original, WriteOriginal::File { .. }));
                    }
                }
                ClientEvent::Kernel(KernelEvent::Fs {
                    ..
                }) => {}
                ClientEvent::Kernel(KernelEvent::Completed {
                    value, ..
                }) => {
                    assert_eq!(value, json!([42, ["local_module.py"]]));
                    break;
                }
                e => panic!("unexpected {e:?}"),
            }
        }
        assert!(captured);
    }
    assert_eq!(
        std::fs::read_to_string(root.join("log")).unwrap(),
        "line\nline\n"
    );
    c.send(KernelCommand::Shutdown).await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn snapshot_never_replaces_an_existing_file() {
    let mut c = client().await;
    let path = std::env::temp_dir().join(format!("kept-snapshot-{}", std::process::id()));
    std::fs::write(&path, "kept").unwrap();
    c.send(KernelCommand::Snapshot {
        path: path.to_string_lossy().into_owned(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::SnapshotFailed { .. })
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
    std::fs::remove_file(path).unwrap();
}

async fn completed(c: &mut KernelClient) -> serde_json::Value {
    loop {
        match event(c).await {
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => return value,
            ClientEvent::Kernel(KernelEvent::Stdout {
                ..
            }) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}
async fn variable_names(c: &mut KernelClient) -> Vec<String> {
    c.send(KernelCommand::Inspect).await.unwrap();
    let ClientEvent::Kernel(KernelEvent::Variables {
        items, ..
    }) = event(c).await
    else {
        panic!()
    };
    items.into_iter().map(|item| item.name).collect()
}
#[tokio::test]
async fn a_session_value_under_a_default_import_name_is_replaced_by_the_next_run() {
    let mut c = client().await;
    feed(
        &c,
        "exec('asyncio = 5')\nexec('json = 6')\n[asyncio, json]",
        vec![],
    )
    .await;
    assert_eq!(completed(&mut c).await, json!([5, 6]));
    let names = variable_names(&mut c).await;
    assert!(
        names.contains(&"asyncio".to_string()) && names.contains(&"json".to_string()),
        "{names:?}"
    );
    feed(&c, "await asyncio.sleep(0)\njson.dumps([1])", vec![]).await;
    assert_eq!(completed(&mut c).await, json!("[1]"));
    let names = variable_names(&mut c).await;
    assert!(
        !names.contains(&"asyncio".to_string()) && !names.contains(&"json".to_string()),
        "{names:?}"
    );
}
#[tokio::test]
async fn a_saved_session_with_values_under_every_default_name_gets_the_defaults_back() {
    let mut c = client().await;
    feed(
        &c,
        "for name in ['asyncio', 'os', 're', 'json', 'Path']:\n    exec(name + ' = 0')\nkept = 6\n[asyncio, os, re, json, Path, kept]",
        vec![],
    )
    .await;
    assert_eq!(completed(&mut c).await, json!([0, 0, 0, 0, 0, 6]));
    let path = std::env::temp_dir().join(format!(
        "legacy-defaults-{}-{}.snapshot",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    c.send(KernelCommand::Snapshot {
        path: path.to_string_lossy().into(),
    })
    .await
    .unwrap();
    let ClientEvent::Kernel(KernelEvent::SnapshotDone {
        sha256, ..
    }) = event(&mut c).await
    else {
        panic!("the snapshot was not written")
    };
    let mut c = client().await;
    c.send(KernelCommand::LoadSnapshot {
        max_state_bytes: 1024 * 1024 * 1024,
        expected_sha256: sha256,
        path: path.to_string_lossy().into(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Loaded)
    ));
    std::fs::remove_file(&path).unwrap();
    feed(
        &c,
        "await asyncio.sleep(0)\n[os.sep, re.escape('.'), json.dumps([1]), Path('a/b').name, kept]",
        vec![],
    )
    .await;
    assert_eq!(completed(&mut c).await, json!(["/", "\\.", "[1]", "b", 6]));
    let names = variable_names(&mut c).await;
    for name in ["asyncio", "os", "re", "json", "Path"] {
        assert!(!names.contains(&name.to_string()), "{name}: {names:?}");
    }
    assert!(names.contains(&"kept".to_string()), "{names:?}");
}
#[tokio::test]
async fn default_imports_are_bound_for_every_run_and_hidden() {
    let mut c = client().await;
    let code =
        "await asyncio.sleep(0)\n[os.sep, re.escape('.'), json.dumps([1]), Path('a/b').name]";
    let expected = json!(["/", "\\.", "[1]", "b"]);
    feed(&c, code, vec![]).await;
    assert_eq!(completed(&mut c).await, expected);
    let names = variable_names(&mut c).await;
    for name in ["asyncio", "os", "re", "json", "Path"] {
        assert!(!names.contains(&name.to_string()), "{name}: {names:?}");
    }
    c.send(KernelCommand::Reset).await.unwrap();
    event(&mut c).await;
    feed(&c, code, vec![]).await;
    assert_eq!(completed(&mut c).await, expected);
    let path = std::env::temp_dir()
        .join(format!("kernel-defaults-{}.snapshot", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    c.send(KernelCommand::Snapshot {
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::SnapshotDone { .. })
    ));
    let mut c = client().await;
    c.send(KernelCommand::LoadSnapshot {
        max_state_bytes: 1024 * 1024 * 1024,
        expected_sha256: {
            use sha2::Digest;
            sha2::Sha256::digest(std::fs::read(&path).unwrap())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        },
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        event(&mut c).await,
        ClientEvent::Kernel(KernelEvent::Loaded)
    ));
    feed(&c, code, vec![]).await;
    assert_eq!(completed(&mut c).await, expected);
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn injected_asyncio_module_is_usable_and_hidden() {
    let mut c = client().await;
    feed(&c, "answer = 1\nawait asyncio.sleep(0)\nanswer", vec![]).await;
    assert_eq!(completed(&mut c).await, json!(1));
    let names = variable_names(&mut c).await;
    assert!(!names.contains(&"asyncio".to_string()), "{names:?}");
    assert!(names.contains(&"answer".to_string()));
}

#[tokio::test]
async fn variable_list_cap_is_reported_with_total() {
    let mut c = client().await;
    let code: String = (0..230).map(|i| format!("v{i} = {i}\n")).collect();
    feed(&c, &code, vec![]).await;
    completed(&mut c).await;
    c.send(KernelCommand::Inspect).await.unwrap();
    let ClientEvent::Kernel(KernelEvent::Variables {
        items,
        truncated,
        total,
        ..
    }) = event(&mut c).await
    else {
        panic!()
    };
    assert_eq!(items.len(), 200);
    assert!(truncated);
    assert_eq!(total, Some(230));
}
async fn call_sites(
    c: &mut KernelClient,
    code: &str,
) -> (Vec<(Option<u32>, Option<u32>)>, KernelEvent) {
    feed(c, code, vec![host("f")]).await;
    let mut sites = vec![];
    let mut ids = vec![];
    loop {
        let ClientEvent::Kernel(e) = event(c).await else {
            panic!("kernel exited")
        };
        match e {
            KernelEvent::CallStarted {
                call_id,
                line,
                col,
                ..
            } => {
                sites.push((line, col));
                ids.push(call_id);
            }
            KernelEvent::NeedResults {
                ..
            } => {
                c.send(KernelCommand::Results {
                    run_id: "test".into(),
                    results: ids
                        .drain(..)
                        .map(|id| (id, HostResult::Ok(json!(1))))
                        .collect(),
                })
                .await
                .unwrap();
            }
            KernelEvent::Stdout {
                ..
            } => {}
            other => return (sites, other),
        }
    }
}
fn text_at(code: &str, line: Option<u32>, col: Option<u32>) -> String {
    code.lines()
        .nth(line.unwrap() as usize - 1)
        .unwrap()
        .chars()
        .skip(col.unwrap() as usize)
        .collect()
}
#[tokio::test]
async fn call_started_reports_call_site_in_the_fed_code() {
    let mut c = client().await;
    let code = "a = await f(x=1)\n\nif a:\n    b = await f(x=2)\na + b";
    let (sites, done) = call_sites(&mut c, code).await;
    assert!(matches!(done, KernelEvent::Completed { .. }), "{done:?}");
    assert_eq!(sites, vec![(Some(1), Some(10)), (Some(4), Some(14))]);
    for (line, col) in sites {
        assert!(text_at(code, line, col).starts_with("f("));
    }
    let code = "await asyncio.gather(*[f(x=i) for i in range(3)])";
    let (sites, _) = call_sites(&mut c, code).await;
    assert_eq!(sites.len(), 3);
    assert!(sites.iter().all(|s| *s == sites[0]));
    assert!(text_at(code, sites[0].0, sites[0].1).starts_with("f(x=i)"));
    let code = "s = \"héllo\"; r = await f(x=1)";
    let (sites, _) = call_sites(&mut c, code).await;
    assert_eq!(sites, vec![(Some(1), Some(23))]);
    assert!(text_at(code, Some(1), Some(23)).starts_with("f("));
    let code = "async def g():\n    return await f(x=1)\nawait g()";
    let (sites, _) = call_sites(&mut c, code).await;
    assert_eq!(sites, vec![(Some(2), Some(17))]);
    assert!(text_at(code, Some(2), Some(17)).starts_with("f("));
    let (sites, _) = call_sites(&mut c, "async def h():\n    return await f(x=1)").await;
    assert!(sites.is_empty());
    let (sites, done) = call_sites(&mut c, "await h()").await;
    assert!(matches!(done, KernelEvent::Completed { .. }), "{done:?}");
    assert_eq!(sites, vec![(None, None)]);
}
#[tokio::test]
async fn raised_reports_failing_line_of_the_fed_code() {
    let mut c = client().await;
    let (_, done) = call_sites(&mut c, "x = 1\ny = x / 0\nz = 2").await;
    assert!(
        matches!(
            done,
            KernelEvent::Raised {
                line: Some(2),
                ..
            }
        ),
        "{done:?}"
    );
    call_sites(&mut c, "def boom():\n    return 1 / 0").await;
    let (_, done) = call_sites(&mut c, "boom()").await;
    assert!(
        matches!(
            done,
            KernelEvent::Raised {
                line: Some(1),
                ..
            }
        ),
        "{done:?}"
    );
}
#[test]
fn older_call_started_and_raised_frames_still_parse() {
    let e: KernelEvent = serde_json::from_value(json!({"CallStarted":{"truncated":false,"run_id":"r","call_id":1,"function":"f","kwargs":{}}})).unwrap();
    assert!(matches!(
        e,
        KernelEvent::CallStarted {
            line: None,
            col: None,
            ..
        }
    ));
    let e: KernelEvent = serde_json::from_value(
        json!({"Raised":{"run_id":"r","exc_type":"E","message":"m","traceback":"t"}}),
    )
    .unwrap();
    assert!(matches!(
        e,
        KernelEvent::Raised {
            line: None,
            ..
        }
    ));
    let v = serde_json::to_value(KernelEvent::CallStarted {
        truncated: false,
        run_id: "r".into(),
        call_id: 1,
        function: "f".into(),
        kwargs: json!({}),
        line: None,
        col: None,
        exceeded: None,
    })
    .unwrap();
    assert!(v["CallStarted"].get("line").is_none());
}

async fn outcome(c: &mut KernelClient, code: &str) -> Result<serde_json::Value, String> {
    feed(c, code, vec![]).await;
    loop {
        match event(c).await {
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => break Ok(value),
            ClientEvent::Kernel(KernelEvent::Raised {
                exc_type,
                message,
                ..
            }) => break Err(format!("{exc_type}: {message}")),
            _ => {}
        }
    }
}
#[tokio::test]
async fn path_objects_are_not_orderable_but_sort_with_key_str() {
    let mut c = client().await;
    for code in [
        "from pathlib import Path\nsorted([Path('b'), Path('a')])",
        "from pathlib import Path\nmin([Path('b'), Path('a')])",
        "from pathlib import Path\nmax([Path('b'), Path('a')])",
        "from pathlib import Path\nPath('a') < Path('b')",
    ] {
        let error = outcome(&mut c, code).await.unwrap_err();
        assert!(
            error.starts_with("TypeError: '")
                && error.contains("' not supported between instances of"),
            "{code}: {error}"
        );
    }
    assert_eq!(
        outcome(
            &mut c,
            "from pathlib import Path\n[str(p) for p in sorted([Path('b'), Path('a')], key=str)]"
        )
        .await,
        Ok(json!(["a", "b"]))
    );
    assert_eq!(
        outcome(&mut c, "from pathlib import Path\n[str(min([Path('b'), Path('a')], key=str)), str(max([Path('b'), Path('a')], key=str))]").await,
        Ok(json!(["a", "b"]))
    );
}

async fn variables(c: &mut KernelClient) -> KernelEvent {
    c.send(KernelCommand::Inspect).await.unwrap();
    match event(c).await {
        ClientEvent::Kernel(
            e @ KernelEvent::Variables {
                ..
            },
        ) => e,
        other => panic!("expected variables, got {other:?}"),
    }
}
#[tokio::test]
async fn inspect_lists_a_large_session_and_estimates_its_largest_variables() {
    let mut c = client().await;
    feed(
        &c,
        "lines = ['0123456789'] * 400000\ntext = 'y' * 1000000\nsmall = 1\nconfig = {'a': [1, 2]}",
        vec![],
    )
    .await;
    completed(&mut c).await;
    for _ in 0..2 {
        let KernelEvent::Variables {
            items,
            total,
            unknown,
            largest,
            previews_truncated,
            ..
        } = variables(&mut c).await
        else {
            unreachable!()
        };
        assert!(!unknown);
        assert_eq!(total, Some(4));
        let shown: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|v| (v.name.as_str(), v.type_name.as_str(), v.preview.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("lines", "list", "list[400000]"),
                ("text", "str", &*format!("'{}", "y".repeat(119))),
                ("small", "int", "1"),
                ("config", "dict", "{'a': [1, 2]}"),
            ]
        );
        assert!(previews_truncated);
        let sizes: Vec<(&str, Option<u64>, Option<u64>)> = largest
            .iter()
            .map(|s| (s.name.as_str(), s.len, s.approx_bytes))
            .collect();
        assert_eq!(
            sizes[..2],
            [
                ("lines", Some(400_000), Some(64 + 400_000 * 60)),
                ("text", Some(1_000_000), Some(1_000_000)),
            ]
        );
    }
}
#[tokio::test]
async fn unmeasured_variables_list_instances_first_and_are_counted_without_modules_classes_or_functions(
) {
    let mut c = client().await;
    feed(
        &c,
        "import math, csv, random, itertools, collections\nfrom pathlib import Path\nclass Index:\n    pass\nclass Cache:\n    pass\ndef f():\n    return 1\np1 = Path('a')\np2 = Path('b')\np3 = Path('c')\np4 = Path('d')\np5 = Path('e')\nr = range(3)\ncache = Cache()\nindex = Index()\ndata = [1, 2]",
        vec![],
    )
    .await;
    completed(&mut c).await;
    let listed = serde_json::to_value(variables(&mut c).await).unwrap();
    let listed = &listed["Variables"];
    let unmeasured: Vec<&str> = listed["largest"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|size| size["approx_bytes"].is_null())
        .map(|size| size["name"].as_str().unwrap())
        .collect();
    assert_eq!(unmeasured, ["cache", "index", "p1", "p2", "p3"]);
    assert_eq!(listed["unmeasured"], json!(8));
    assert!(listed["largest"]
        .as_array()
        .unwrap()
        .iter()
        .any(|size| size["name"] == "data"));
}

#[tokio::test]
async fn the_listing_cuts_names_and_type_names_at_200_characters() {
    let mut c = client().await;
    let name = "v".repeat(300);
    let class = "C".repeat(300);
    feed(
        &c,
        &format!("class {class}:\n    pass\n{name} = {class}()"),
        vec![],
    )
    .await;
    completed(&mut c).await;
    let KernelEvent::Variables {
        items,
        largest,
        ..
    } = variables(&mut c).await
    else {
        unreachable!()
    };
    let listed: Vec<(String, String)> = items
        .iter()
        .map(|v| (v.name.clone(), v.type_name.clone()))
        .collect();
    assert_eq!(
        listed,
        [
            ("C".repeat(200), "type".to_string()),
            ("v".repeat(200), "C".repeat(200)),
        ]
    );
    assert!(largest
        .iter()
        .all(|s| s.name.chars().count() <= 200 && s.type_name.chars().count() <= 200));
}

#[tokio::test]
async fn inspect_never_calls_a_name_or_a_method_the_script_defined() {
    let mut c = client().await;
    feed(
        &c,
        "calls = []\nclass Obj:\n    def __repr__(self):\n        calls.append('repr')\n        return 'Obj'\n    def __len__(self):\n        calls.append('__len__')\n        return 1\ndef len(x):\n    calls.append('len')\n    return 0\ndef type(x):\n    calls.append('type')\n    return int\no = Obj()\ndata = [1, 2, 3]",
        vec![],
    )
    .await;
    completed(&mut c).await;
    let KernelEvent::Variables {
        items,
        unknown,
        largest,
        ..
    } = variables(&mut c).await
    else {
        unreachable!()
    };
    assert!(!unknown);
    let preview = |name: &str| {
        items
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.preview.clone())
    };
    assert_eq!(preview("o").as_deref(), Some("<Obj>"));
    assert_eq!(preview("data").as_deref(), Some("[1, 2, 3]"));
    assert!(largest
        .iter()
        .any(|s| s.name == "o" && s.approx_bytes.is_none()));
    feed(&c, "calls", vec![]).await;
    assert_eq!(completed(&mut c).await, json!([]));
    let mut c = client().await;
    feed(
        &c,
        "calls = []\ndef locals():\n    calls.append('locals')\n    return {}",
        vec![],
    )
    .await;
    completed(&mut c).await;
    let KernelEvent::Variables {
        items,
        unknown,
        total,
        ..
    } = variables(&mut c).await
    else {
        unreachable!()
    };
    assert!(unknown && items.is_empty() && total.is_none());
    feed(&c, "calls", vec![]).await;
    assert_eq!(completed(&mut c).await, json!([]));
}
