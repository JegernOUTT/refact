use super::*;

#[test]
fn arguments_are_checked_like_python() {
    assert_eq!(
        parse_args(&json!({"cmd": "cargo test"})),
        Ok(RunArgs {
            cmd: "cargo test".into(),
            timeout: None,
            cwd: None,
            tail: DEFAULT_TAIL_LINES
        })
    );
    assert_eq!(
        parse_args(&json!({"cmd": "make", "timeout": 3600, "cwd": "sub", "tail": 0})),
        Ok(RunArgs {
            cmd: "make".into(),
            timeout: Some(3600),
            cwd: Some("sub".into()),
            tail: 0
        })
    );
    assert_eq!(
        parse_args(&json!({"cmd": "x", "timeout": null, "cwd": null, "tail": 2000}))
            .unwrap()
            .tail,
        2000
    );
    for (kwargs, text) in [
        (json!({}), "run() missing required argument: 'cmd'"),
        (json!({"cmd": ""}), "run() cmd must be a non-empty string"),
        (json!({"cmd": "  "}), "run() cmd must be a non-empty string"),
        (json!({"cmd": 1}), "run() cmd must be a non-empty string"),
        (
            json!({"cmd": "x", "timeout": 0}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd": "x", "timeout": 3601}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd": "x", "timeout": 1.5}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd": "x", "timeout": "5"}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd": "x", "timeout": true}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd": "x", "tail": -1}),
            "run() tail must be an integer between 0 and 2000",
        ),
        (
            json!({"cmd": "x", "tail": "5"}),
            "run() tail must be an integer between 0 and 2000",
        ),
        (
            json!({"cmd": "x", "tail": 2001}),
            "run() tail must be an integer between 0 and 2000",
        ),
        (
            json!({"cmd": "x", "cwd": 3}),
            "run() cwd must be a non-empty str or None",
        ),
        (
            json!({"cmd": "x", "output_limit": "all"}),
            "run() got an unexpected keyword argument 'output_limit'",
        ),
        (
            json!({"cmd": "x", "run_in_background": true}),
            "run() got an unexpected keyword argument 'run_in_background'",
        ),
    ] {
        assert_eq!(parse_args(&kwargs).unwrap_err(), text, "{kwargs}");
    }
}

#[test]
fn the_shell_call_has_no_output_or_background_options() {
    let args =
        parse_args(&json!({"cmd": "cargo   test\n-p x", "timeout": 30, "cwd": "crates"})).unwrap();
    assert_eq!(
        shell_kwargs(&args),
        json!({"command": "cargo   test\n-p x", "description": "run(): cargo test -p x", "timeout": 30, "workdir": "crates"})
    );
    let long = parse_args(&json!({"cmd": "x".repeat(100)})).unwrap();
    let description = shell_kwargs(&long)["description"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(description, format!("run(): {}…", "x".repeat(80)));
    assert_eq!(
        shell_kwargs(&parse_args(&json!({"cmd": "ls"})).unwrap()),
        json!({"command": "ls", "description": "run(): ls"})
    );
}

#[test]
fn problem_lines_cover_common_tools_and_skip_warnings_and_summaries() {
    let output = [
        "   Compiling foo v0.1.0",
        "warning: unused variable: `x`",
        "warning: build failed, waiting for other jobs to finish...",
        "warning: a build script panicked at startup and was retried",
        "Error: 0 errors, 2 warnings",
        "error[E0425]: cannot find value `y` in this scope",
        "src/error.rs:10:5: note: in this expansion",
        "src/a.c:3:1: error: expected ';'",
        "src/app.ts(4,1): error TS2304: Cannot find name 'z'.",
        "test tests::adds ... FAILED",
        "test tests::subs ... ok",
        "failures:",
        "thread 'main' panicked at src/main.rs:2:5:",
        "FAILED t.py::test_x - AssertionError: boom",
        "E   assert 1 == 2",
        "Traceback (most recent call last):",
        "ValueError: x",
        "--- FAIL: TestX (0.00s)",
        "  ✕ adds (3 ms)",
        "FAIL src/sum.test.js",
        "npm ERR! code ELIFECYCLE",
        "fatal: not a git repository",
        "test result: ok. 5 passed; 0 failed; 0 ignored",
        "test result: FAILED. 1 passed; 1 failed",
        "error: could not compile `foo` (lib) due to 1 previous error",
        "error[E0425]: cannot find value `y` in this scope",
    ]
    .join("\n");
    assert_eq!(
        problems(&output),
        vec![
            "error[E0425]: cannot find value `y` in this scope",
            "src/a.c:3:1: error: expected ';'",
            "src/app.ts(4,1): error TS2304: Cannot find name 'z'.",
            "test tests::adds ... FAILED",
            "failures:",
            "thread 'main' panicked at src/main.rs:2:5:",
            "FAILED t.py::test_x - AssertionError: boom",
            "E   assert 1 == 2",
            "Traceback (most recent call last):",
            "ValueError: x",
            "--- FAIL: TestX (0.00s)",
            "  ✕ adds (3 ms)",
            "FAIL src/sum.test.js",
            "npm ERR! code ELIFECYCLE",
            "fatal: not a git repository",
            "error: could not compile `foo` (lib) due to 1 previous error",
        ]
    );
}

#[test]
fn problems_are_capped_cut_and_read_as_a_terminal_shows_them() {
    let many = (0..30)
        .map(|i| format!("error: problem {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let found = problems(&many);
    assert_eq!(found.len(), MAX_PROBLEMS);
    assert_eq!(found[19], "error: problem 19");
    let long = format!("error: {}", "é".repeat(400));
    assert_eq!(problems(&long)[0].chars().count(), PROBLEM_CHARS);
    assert_eq!(
        problems("\x1b[1m\x1b[31merror\x1b[0m\x1b[1m: boom\x1b[0m\r\n"),
        vec!["error: boom"]
    );
    assert_eq!(
        problems("Compiling 10%\rCompiling 50%\rerror: late\n"),
        vec!["error: late"]
    );
    assert!(problems("all good\n").is_empty());
}

#[test]
fn tail_keeps_the_last_lines_within_line_and_byte_bounds() {
    let output = (1..=100)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(tail(&output, 0), "");
    assert_eq!(tail(&output, 3), "line 98\nline 99\nline 100");
    assert_eq!(tail(&output, 1000).lines().count(), 100);
    assert_eq!(tail("", 5), "");
    assert_eq!(tail("one\r\ntwo\r\n", 5), "one\ntwo");
    assert_eq!(tail("50%\r100%\ndone", 5), "100%\ndone");
    let wide = format!("{}\nlast", "w".repeat(5000));
    let cut = tail(&wide, 2);
    assert_eq!(cut.lines().next().unwrap().chars().count(), TAIL_LINE_CHARS);
    let big = (0..2000)
        .map(|_| "z".repeat(900))
        .collect::<Vec<_>>()
        .join("\n");
    let bounded = tail(&big, MAX_TAIL_LINES);
    assert!(bounded.len() <= TAIL_MAX_BYTES, "{}", bounded.len());
    assert!(bounded.len() > TAIL_MAX_BYTES - 1000);
    let unterminated = "x".repeat(10 * 1024 * 1024);
    assert_eq!(tail(&unterminated, 60).len(), TAIL_LINE_CHARS);
}

#[test]
fn results_report_status_problems_and_log() {
    let end = RunEnd {
        status: "exited",
        exit_code: Some(3),
        duration_ms: 12_345,
        output: "a\nerror: boom\n",
        log: Some("/run/logs/1_run.log".into()),
        truncated: false,
        failure: None,
    };
    let (value, summary) = result(&end, 60);
    assert_eq!(
        value,
        json!({"ok": false, "exit_code": 3, "status": "exited", "duration_s": 12.3, "tail": "a\nerror: boom", "problems": ["error: boom"], "log": "/run/logs/1_run.log", "truncated": false, "error": "the command exited with code 3"})
    );
    assert_eq!(summary, "exit 3 · 12.3 s · 1 problem");

    let (value, summary) = result(
        &RunEnd {
            exit_code: Some(0),
            output: "done\n",
            truncated: true,
            ..end
        },
        60,
    );
    assert_eq!(value["ok"], true);
    assert_eq!(value["error"], Value::Null);
    assert_eq!(value["truncated"], true);
    assert_eq!(summary, "exit 0 · 12.3 s");

    for (status, error, head) in [
        (
            "timed_out",
            "the command timed out and was killed",
            "timed out",
        ),
        ("killed", "the command was stopped", "killed"),
        (
            "sandbox_launcher_failed",
            "the command failed: launcher",
            "failed",
        ),
    ] {
        let (value, summary) = result(
            &RunEnd {
                status,
                exit_code: None,
                duration_ms: 1000,
                output: "",
                log: None,
                truncated: false,
                failure: Some("launcher"),
            },
            60,
        );
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"], error);
        assert_eq!(summary, format!("{head} · 1.0 s"));
    }

    let (value, summary) = withheld(Some(0), 2500, "output withheld by the privacy policy");
    assert_eq!(
        value,
        json!({"ok": false, "exit_code": 0, "status": "withheld", "duration_s": 2.5, "tail": "", "problems": [], "log": null, "truncated": false, "error": "output withheld by the privacy policy"})
    );
    assert_eq!(summary, "withheld · 2.5 s");
}

#[test]
fn a_result_is_fitted_by_trimming_the_tail_first_then_the_problems() {
    let output = (0..100)
        .map(|i| format!("error: line {i:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (full, _) = result(
        &RunEnd {
            status: "exited",
            exit_code: Some(1),
            duration_ms: 1000,
            output: &output,
            log: Some("/run/logs/1_run.log".into()),
            truncated: false,
            failure: None,
        },
        100,
    );
    let size = |value: &Value| value.to_string().len();
    let kept_fields = |value: &Value| {
        for key in [
            "ok",
            "exit_code",
            "status",
            "duration_s",
            "log",
            "truncated",
            "error",
        ] {
            assert_eq!(value[key], full[key], "{key}");
        }
    };

    let mut same = full.clone();
    assert!(fit_result(&mut same, |v| size(v) <= size(&full)));
    assert_eq!(same, full);

    let budget = size(&full) - 900;
    let mut cut = full.clone();
    assert!(fit_result(&mut cut, |v| size(v) <= budget));
    assert!(size(&cut) <= budget);
    let tail = cut["tail"].as_str().unwrap();
    assert!(tail.ends_with("error: line 099"), "{tail}");
    let first = tail.lines().next().unwrap();
    let dropped = full["tail"].as_str().unwrap().lines().collect::<Vec<_>>();
    let previous = dropped[dropped.iter().position(|l| *l == first).unwrap() - 1];
    let mut one_more = cut.clone();
    one_more["tail"] = json!(format!("{previous}\n{tail}"));
    assert!(size(&one_more) > budget, "the longest fitting tail is kept");
    assert_eq!(cut["problems"], full["problems"]);
    kept_fields(&cut);

    let mut without_tail = full.clone();
    without_tail["tail"] = json!("");
    let budget = size(&without_tail) - 100;
    let mut cut = full.clone();
    assert!(fit_result(&mut cut, |v| size(v) <= budget));
    assert!(size(&cut) <= budget);
    assert_eq!(cut["tail"], "");
    let problems = cut["problems"].as_array().unwrap();
    let all = full["problems"].as_array().unwrap();
    assert!(!problems.is_empty() && problems.len() < all.len());
    assert_eq!(problems[..], all[..problems.len()]);
    kept_fields(&cut);

    let mut nothing = full.clone();
    assert!(!fit_result(&mut nothing, |_| false));
    assert_eq!(nothing["tail"], "");
    assert_eq!(nothing["problems"], json!([]));
    kept_fields(&nothing);
}
