use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef};
use monty_types::{
    CompileOptions, ExcType, MontyFileHandle, MontyObject, NamedValues, OsFunctionCall,
    PrintWriter, ResourceLimits, ResourceTracker,
};
use std::time::Duration;

fn repl() -> MontyRepl {
    MontyRepl::new(
        "smoke.py",
        ResourceTracker::default(),
        CompileOptions::default(),
    )
}

fn feed(repl: &mut MontyRepl, code: &str) -> MontyObject {
    repl.feed_run(code, NamedValues::new(), PrintWriter::Disabled)
        .unwrap()
}

#[test]
fn persistent_expression_and_globals() {
    let mut repl = repl();
    assert_eq!(feed(&mut repl, "x = 40\nx + 1"), MontyObject::int(41));
    assert_eq!(feed(&mut repl, "x + 2"), MontyObject::int(42));
    assert_eq!(
        feed(&mut repl, "locals()"),
        MontyObject::dict([(MontyObject::string("x"), MontyObject::int(40))])
    );
}

#[test]
fn async_host_function_and_gather_order() {
    for (code, names) in [
        ("await f()", vec!["f"]),
        (
            "import asyncio\nawait asyncio.gather(f(), g())",
            vec!["f", "g"],
        ),
    ] {
        let inputs: Vec<_> = names
            .iter()
            .map(|n| (n.to_string(), MontyObject::async_function(*n, None)))
            .collect();
        let mut progress = repl()
            .feed_start(code, inputs, PrintWriter::Disabled)
            .unwrap();
        let mut calls = Vec::new();
        for name in &names {
            let call = progress
                .into_function_call()
                .expect("host call before resolution");
            assert_eq!(&call.function_name, name);
            assert!(call.is_async);
            calls.push(call.call_id);
            progress = call.resume_pending(PrintWriter::Disabled).unwrap();
        }
        let pending = progress.into_resolve_futures().unwrap();
        assert_eq!(pending.pending_call_ids().len(), calls.len());
        let results = calls
            .iter()
            .enumerate()
            .rev()
            .map(|(i, id)| (*id, MontyObject::int(10 + i as i64).into()))
            .collect();
        let (_, value) = pending
            .resume(results, PrintWriter::Disabled)
            .unwrap()
            .into_complete()
            .unwrap();
        let expected = if calls.len() == 1 {
            MontyObject::int(10)
        } else {
            MontyObject::list(vec![MontyObject::int(10), MontyObject::int(11)])
        };
        assert_eq!(value, expected);
    }
}

#[test]
fn os_calls_are_host_dispatched() {
    let mut progress = repl().feed_start("import pathlib\nimport os\nf = open('/a')\na = pathlib.Path('/b').read_text()\nb = os.getenv('SMOKE')\n(a, b)", NamedValues::new(), PrintWriter::Disabled).unwrap();
    let mut seen = Vec::new();
    loop {
        match progress {
            ReplProgress::OsCall(call) => {
                progress = call
                    .resume_with(PrintWriter::Disabled, |op| {
                        seen.push(op.name());
                        match op {
                            OsFunctionCall::Open(args) => {
                                MontyObject::file_handle(MontyFileHandle {
                                    path: args.path.into_string(),
                                    mode: args.mode,
                                    position: 0,
                                })
                                .into()
                            }
                            OsFunctionCall::ReadText(_) => MontyObject::string("contents").into(),
                            OsFunctionCall::Getenv(args) => {
                                assert_eq!(args.key, "SMOKE");
                                MontyObject::string("host-value").into()
                            }
                            other => panic!("unexpected OS call: {other:?}"),
                        }
                    })
                    .unwrap();
            }
            ReplProgress::Complete {
                value, ..
            } => {
                assert_eq!(seen, ["open", "Path.read_text", "os.getenv"]);
                assert_eq!(
                    value,
                    MontyObject::tuple(vec![
                        MontyObject::string("contents"),
                        MontyObject::string("host-value")
                    ])
                );
                break;
            }
            other => panic!("unexpected progress {other:?}"),
        }
    }
}

#[test]
fn idle_dump_and_load() {
    let mut repl = repl();
    feed(&mut repl, "x = 42");
    let bytes = monty::dump("smoke.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut restored) = Dump::load(&bytes).unwrap().state else {
        panic!("not idle")
    };
    assert_eq!(feed(&mut restored, "x"), MontyObject::int(42));
}

#[test]
fn limits_are_uncatchable_and_distinct_from_python_errors() {
    let limits = ResourceLimits {
        max_memory: Some(1_000_000),
        ..ResourceLimits::default()
    };
    let mut memory = MontyRepl::new(
        "memory.py",
        ResourceTracker::new(limits),
        CompileOptions::default(),
    );
    let err = memory
        .feed_run(
            "try:\n    x = 'x' * 1000000000\nexcept BaseException:\n    pass",
            NamedValues::new(),
            PrintWriter::Disabled,
        )
        .unwrap_err();
    assert_eq!(err.exc_type(), ExcType::MemoryError);
    assert!(err.to_string().contains("memory limit exceeded"));
    let limits = ResourceLimits::default().max_feed_duration(Duration::from_millis(5));
    let mut timed = MontyRepl::new(
        "time.py",
        ResourceTracker::new(limits),
        CompileOptions::default(),
    );
    let err = timed
        .feed_run(
            "try:\n    while True:\n        pass\nexcept BaseException:\n    pass",
            NamedValues::new(),
            PrintWriter::Disabled,
        )
        .unwrap_err();
    assert!(err.to_string().contains("time limit exceeded"));
    let err = repl()
        .feed_run("1 / 0", NamedValues::new(), PrintWriter::Disabled)
        .unwrap_err();
    assert_eq!(err.exc_type(), ExcType::ZeroDivisionError);
}

#[test]
fn abort_at_suspension_preserves_session() {
    let progress = repl()
        .feed_start(
            "x = 42\nf()",
            vec![("f".to_owned(), MontyObject::async_function("f", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
    let call = progress.into_function_call().unwrap();
    let mut recovered = call.into_repl();
    assert_eq!(feed(&mut recovered, "x"), MontyObject::int(42));
}

#[test]
fn session_and_progress_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<MontyRepl>();
    assert_send::<ReplProgress>();
    let session = repl();
    assert_eq!(
        std::thread::spawn(move || {
            let mut session = session;
            feed(&mut session, "6 * 7")
        })
        .join()
        .unwrap(),
        MontyObject::int(42)
    );
}

#[test]
fn print_capture() {
    let mut output = String::new();
    repl()
        .feed_run(
            "print('hello', 42)",
            NamedValues::new(),
            PrintWriter::CollectString(&mut output, Some(1024)),
        )
        .unwrap();
    assert_eq!(output, "hello 42\n");
}

#[test]
fn language_features_and_imports() {
    let mut session = repl();
    assert_eq!(feed(&mut session, "import json\nimport re\nimport datetime\nimport pathlib\nimport os\nclass A:\n    def value(self):\n        return 42\ntry:\n    1 / 0\nexcept ZeroDivisionError:\n    answer = A().value()\njson.loads(json.dumps([f'{i}' for i in range(2)])) == ['0', '1'] and answer == 42 and re.match('a', 'abc') is not None"), MontyObject::bool(true));
}

#[test]
fn unresolved_name_can_be_supplied_by_host() {
    let progress = repl()
        .feed_start("host_value + 1", NamedValues::new(), PrintWriter::Disabled)
        .unwrap();
    let lookup = progress.into_name_lookup().unwrap();
    assert_eq!(lookup.name, "host_value");
    let (_, value) = lookup
        .resume(MontyObject::int(41).into(), PrintWriter::Disabled)
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(value, MontyObject::int(42));
}

#[test]
fn upfront_memory_guard_does_not_account_for_accumulated_small_allocations() {
    let limits = ResourceLimits {
        max_memory: Some(4096),
        ..ResourceLimits::default()
    };
    let mut session = MontyRepl::new(
        "memory-caveat.py",
        ResourceTracker::new(limits),
        CompileOptions::default(),
    );
    assert_eq!(
        feed(
            &mut session,
            "items = [['x' * 100 for j in range(100)] for i in range(100)]\nlen(items)"
        ),
        MontyObject::int(100)
    );
}
