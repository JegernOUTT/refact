use refact_python_kernel::{client::*, kernel::FakeOs, protocol::*};
use serde_json::json;
use std::time::Duration;

async fn event(c: &mut KernelClient) -> KernelEvent {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), c.next_event())
            .await
            .unwrap()
        {
            ClientEvent::Kernel(KernelEvent::Stdout {
                ..
            }) => continue,
            ClientEvent::Kernel(e) => return e,
            other => panic!("unexpected {other:?}"),
        }
    }
}
async fn client() -> KernelClient {
    let mut c =
        KernelClient::spawn_in_thread(Box::new(FakeOs::default()), ClientOptions::default())
            .unwrap();
    event(&mut c).await;
    c
}
async fn feed(c: &KernelClient, code: &str) {
    c.send(KernelCommand::Feed {
        run_id: "t".into(),
        code: code.into(),
        globals: vec![
            GlobalBinding {
                name: "f".into(),
                kind: GlobalKind::AsyncFn,
            },
            GlobalBinding {
                name: "g".into(),
                kind: GlobalKind::SyncFn,
            },
            GlobalBinding {
                name: "show".into(),
                kind: GlobalKind::SyncFn,
            },
        ],
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
}
async fn type_error(c: &mut KernelClient, code: &str) -> String {
    feed(c, code).await;
    match event(c).await {
        KernelEvent::Raised {
            exc_type,
            message,
            ..
        } if exc_type == "TypeError" => message,
        other => panic!("{code}: expected a TypeError before any call, got {other:?}"),
    }
}
async fn sync_args(c: &mut KernelClient, code: &str) -> (serde_json::Value, bool) {
    feed(c, code).await;
    let KernelEvent::SyncCall {
        call_id,
        args,
        truncated,
        ..
    } = event(c).await
    else {
        panic!("{code}: expected a sync call")
    };
    c.send(KernelCommand::SyncResult {
        call_id,
        result: HostResult::Ok(json!(null)),
    })
    .await
    .unwrap();
    assert!(matches!(event(c).await, KernelEvent::Completed { .. }));
    (args, truncated)
}

const JSON_TYPES: &str = "must be str, int, float, bool, None, list or dict, not";

#[tokio::test]
async fn non_json_call_arguments_are_type_errors_naming_the_argument() {
    let mut c = client().await;
    for (code, expected) in [
        (
            "await f(process_id=id)",
            format!("f() argument process_id {JSON_TYPES} builtin_function_or_method. At module level a comprehension variable named like a builtin (here 'id') reads the builtin in this interpreter; rename the variable."),
        ),
        (
            "await f(kind=str)",
            format!("f() argument kind {JSON_TYPES} type. At module level a comprehension variable named like a builtin (here 'str') reads the builtin in this interpreter; rename the variable."),
        ),
        (
            "from pathlib import Path\nawait f(paths=['a', Path('b')])",
            format!("f() argument paths[1] {JSON_TYPES} PosixPath. Pass str(path)."),
        ),
        (
            "await f(data={'raw': b'ab'})",
            format!("f() argument data['raw'] {JSON_TYPES} bytes. Decode it first, e.g. data.decode()."),
        ),
        (
            "await f(tags={1})",
            format!("f() argument tags {JSON_TYPES} set. Pass a list, e.g. sorted(values)."),
        ),
        (
            "await f(options={'a': {1: 2}})",
            "f() argument options['a'] has a dict key of type int; dict keys must be str.".into(),
        ),
        (
            "await f(n=2 ** 70)",
            "f() argument n is an int too large for JSON; pass it as a string.".into(),
        ),
        (
            "await f(x=float('nan'))",
            "f() argument x is a float JSON cannot represent (infinity or NaN).".into(),
        ),
    ] {
        assert_eq!(type_error(&mut c, code).await, expected, "{code}");
    }
    assert_eq!(
        type_error(&mut c, "def h():\n    pass\nawait f(x=h)").await,
        format!("f() argument x {JSON_TYPES} function.")
    );
}

#[tokio::test]
async fn helper_arguments_take_paths_and_refuse_other_non_json_values() {
    let mut c = client().await;
    assert_eq!(
        sync_args(&mut c, "from pathlib import Path\ng(Path('a/b'))").await,
        (json!(["a/b"]), false)
    );
    assert_eq!(
        type_error(&mut c, "g(b'x')").await,
        "g() argument 1 must be str, int, float, bool, None, list, dict or Path, not bytes. Decode it first, e.g. data.decode()."
    );
}

#[tokio::test]
async fn calls_to_unbound_names_are_left_to_the_host() {
    let mut c = client().await;
    feed(&c, "undefined_helper(len)").await;
    let KernelEvent::SyncCall {
        function,
        args,
        ..
    } = event(&mut c).await
    else {
        panic!("expected the host to answer for an unbound name")
    };
    assert_eq!(function, "undefined_helper");
    assert_eq!(args, json!([{"__repr__": "<built-in function len>"}]));
}

#[tokio::test]
async fn show_and_results_render_non_json_values_as_their_repr_without_truncation() {
    let mut c = client().await;
    assert_eq!(
        sync_args(
            &mut c,
            "import datetime\nshow([{'d': datetime.date(2026, 1, 1)}])"
        )
        .await,
        (
            json!([[{"d": {"__repr__": "datetime.date(2026, 1, 1)"}}]]),
            false
        )
    );
    feed(
        &c,
        "from pathlib import Path\nimport datetime\n[Path('a/b'), {1, 2}, b'abc', {1: 'a'}, datetime.date(2024, 1, 2)]",
    )
    .await;
    let KernelEvent::Completed {
        value,
        repr,
        truncated,
        ..
    } = event(&mut c).await
    else {
        panic!("expected a value")
    };
    assert!(!truncated);
    assert_eq!(
        value,
        json!([
            {"__repr__": "PosixPath('a/b')"},
            [1, 2],
            {"__repr__": "b'abc'"},
            {"__repr__": "{1: 'a'}"},
            {"__repr__": "datetime.date(2024, 1, 2)"}
        ])
    );
    assert_eq!(
        repr,
        "[PosixPath('a/b'), {1, 2}, b'abc', {1: 'a'}, datetime.date(2024, 1, 2)]"
    );
}

#[tokio::test]
async fn argument_errors_and_oversized_arguments_carry_no_value() {
    let mut c = client().await;
    let message = type_error(
        &mut c,
        "await f(paths={'k' * 100 + '\\n': b'test-credential'})",
    )
    .await;
    assert_eq!(
        message,
        format!(
            "f() argument paths['{}…'] {JSON_TYPES} bytes. Decode it first, e.g. data.decode().",
            "k".repeat(40)
        )
    );
    let message = type_error(&mut c, "await f(**{'a\\n' * 30: {1}})").await;
    assert!(
        message.starts_with(&format!(
            "f() argument {}… {JSON_TYPES} set.",
            "a\\n".repeat(20)
        )),
        "{message}"
    );
    feed(&c, "await f(x=['test-credential'] * 200001)").await;
    let KernelEvent::CallStarted {
        kwargs,
        exceeded,
        ..
    } = event(&mut c).await
    else {
        panic!("expected the oversized call to reach the host")
    };
    assert!(exceeded.is_some());
    assert_eq!(kwargs, json!({"x": {"__repr__": "<truncated>"}}));
}

async fn call_exceeded(code: &str, max_value_nodes: u32, max_arg_bytes: u64) -> Option<Exceeded> {
    call_started(code, max_value_nodes, max_arg_bytes).await.0
}

async fn call_started(
    code: &str,
    max_value_nodes: u32,
    max_arg_bytes: u64,
) -> (Option<Exceeded>, serde_json::Value) {
    let mut c = client().await;
    c.send(KernelCommand::Feed {
        run_id: "t".into(),
        code: code.into(),
        globals: vec![GlobalBinding {
            name: "f".into(),
            kind: GlobalKind::AsyncFn,
        }],
        limits: RunLimits {
            max_value_nodes,
            max_arg_bytes,
            ..RunLimits::default()
        },
    })
    .await
    .unwrap();
    match event(&mut c).await {
        KernelEvent::CallStarted {
            exceeded,
            kwargs,
            ..
        } => (exceeded, kwargs),
        other => panic!("{code}: expected a call, got {other:?}"),
    }
}

#[tokio::test]
async fn argument_budgets_count_the_json_sent_and_allow_the_limit_itself() {
    let over = |dimension, at_least, limit| {
        Some(Exceeded {
            dimension,
            at_least,
            limit,
        })
    };
    for (code, nodes, bytes, expected) in [
        ("await f(x='a' * 65528)", 200_000, 65_536, None),
        (
            "await f(x='a' * 65529)",
            200_000,
            65_536,
            over(Dimension::Bytes, 65_537, 65_536),
        ),
        ("await f(x={'x': 'a' * 65522})", 200_000, 65_536, None),
        ("await f(x='\"' * 32764)", 200_000, 65_536, None),
        (
            "await f(x='\"' * 32765)",
            200_000,
            65_536,
            over(Dimension::Bytes, 65_538, 65_536),
        ),
        ("await f(x=['a'] * 9)", 10, 65_536, None),
        (
            "await f(x=['a'] * 10)",
            10,
            65_536,
            over(Dimension::Values, 11, 10),
        ),
    ] {
        let (exceeded, kwargs) = call_started(code, nodes, bytes).await;
        assert_eq!(exceeded, expected, "{code}");
        if expected.is_none() {
            assert!(
                serde_json::to_vec(&kwargs).unwrap().len() as u64 <= bytes,
                "{code}"
            );
        }
    }
    assert_eq!(
        serde_json::to_vec(
            &call_started("await f(x='a' * 65528)", 200_000, 65_536)
                .await
                .1
        )
        .unwrap()
        .len(),
        65_536
    );
}

#[tokio::test]
async fn keyword_names_count_toward_the_argument_size_and_stop_at_256_bytes() {
    let mut c = client().await;
    for (code, function) in [
        ("await f(**{'k' * 257: 1})", "f"),
        ("g(**{'k' * 257: 1})", "g"),
    ] {
        assert_eq!(
            type_error(&mut c, code).await,
            format!("{function}() got a keyword argument name of 257 bytes; names are limited to 256 bytes"),
            "{code}"
        );
    }
    assert_eq!(
        call_exceeded("await f(**{'k' * 256: 1})", 200_000, 300).await,
        None
    );
    assert_eq!(
        call_exceeded("await f(**{'k' * 256: 1, 'j' * 256: 2})", 200_000, 300)
            .await
            .map(|e| e.dimension),
        Some(Dimension::Bytes)
    );
}

#[tokio::test]
async fn a_cut_display_text_is_described_as_text_not_as_a_byte_limit() {
    let mut c = client().await;
    feed(&c, "show([{'blob': b'x' * 5000}])").await;
    let KernelEvent::SyncCall {
        function,
        truncated,
        exceeded,
        ..
    } = event(&mut c).await
    else {
        panic!("expected show() to reach the host")
    };
    assert_eq!(function, "show");
    assert!(truncated);
    assert_eq!(
        exceeded.unwrap().describe(),
        "its text form is over 4,096 characters"
    );
}

#[tokio::test]
async fn sets_become_json_arrays_and_keep_their_python_repr() {
    let mut c = client().await;
    feed(
        &c,
        "[{1, 2}, frozenset({3}), set(), frozenset(), [{(1, 2)}]]",
    )
    .await;
    let KernelEvent::Completed {
        value,
        repr,
        truncated,
        ..
    } = event(&mut c).await
    else {
        panic!("expected a value")
    };
    assert!(!truncated);
    assert_eq!(value, json!([[1, 2], [3], [], [], [[[1, 2]]]]));
    assert_eq!(
        repr,
        "[{1, 2}, frozenset({3}), set(), frozenset(), [{[1, 2]}]]"
    );
    assert_eq!(
        sync_args(&mut c, "show({'a'})").await,
        (json!([["a"]]), false)
    );
}
