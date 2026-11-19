use refact_python_kernel::{client::*, kernel::FakeOs, protocol::*};
use serde_json::json;
use std::time::Duration;

async fn event(c: &mut KernelClient) -> KernelEvent {
    match tokio::time::timeout(Duration::from_secs(10), c.next_event())
        .await
        .unwrap()
    {
        ClientEvent::Kernel(e) => e,
        other => panic!("unexpected {other:?}"),
    }
}
async fn client() -> KernelClient {
    let mut c =
        KernelClient::spawn_in_thread(Box::new(FakeOs::default()), ClientOptions::default())
            .unwrap();
    event(&mut c).await;
    c
}
fn session_globals() -> Vec<GlobalBinding> {
    let mut globals: Vec<_> = ["rm", "cat", "tree", "web", "glob"]
        .into_iter()
        .map(|name| GlobalBinding {
            name: name.into(),
            kind: GlobalKind::AsyncFn,
        })
        .collect();
    globals.push(GlobalBinding {
        name: "show".into(),
        kind: GlobalKind::SyncFn,
    });
    globals.push(GlobalBinding {
        name: "PROJECT_ROOT".into(),
        kind: GlobalKind::Json(json!("/p")),
    });
    globals
}
/// Feeds `print('ran')` and then `code`; a refusal must come before anything printed.
async fn run(
    c: &mut KernelClient,
    code: &str,
    globals: Vec<GlobalBinding>,
) -> Result<(), KernelEvent> {
    c.send(KernelCommand::Feed {
        run_id: "t".into(),
        code: format!("print('ran')\n{code}"),
        globals,
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
    let mut ran = false;
    loop {
        match event(c).await {
            KernelEvent::Stdout {
                ..
            } => ran = true,
            KernelEvent::Completed {
                ..
            } if ran => return Ok(()),
            e @ KernelEvent::Raised {
                ..
            } if !ran => return Err(e),
            other => panic!("{code}: unexpected {other:?} (ran: {ran})"),
        }
    }
}
async fn refused(c: &mut KernelClient, code: &str) -> (String, Option<u32>) {
    match run(c, code, session_globals()).await {
        Err(KernelEvent::Raised {
            exc_type,
            message,
            line,
            ..
        }) if exc_type == "SyntaxError" && message.ends_with("Nothing in this script ran.") => {
            (message, line)
        }
        other => panic!("{code}: expected a refusal before anything ran, got {other:?}"),
    }
}

const REBOUND: &str = "Tool functions, helpers, constants and the pre-imported modules are bound again before every run, so a value you keep in";

#[tokio::test]
async fn module_scope_bindings_of_bound_names_are_refused_before_anything_runs() {
    let mut c = client().await;
    assert_eq!(
        refused(&mut c, "rm = [1]").await,
        (
            format!("line 2 binds 'rm', an async function bound by this session (a tool or helper). {REBOUND} 'rm' would be replaced by the next run. Rename it (for example 'rm_'). Nothing in this script ran."),
            Some(2)
        )
    );
    assert_eq!(
        refused(&mut c, "import glob").await,
        ("line 2 imports glob, but glob is the file-search tool (await glob(pattern=...)); for local patterns use Path(dir).glob(...) or .rglob(...). Remove the import. Nothing in this script ran.".into(), Some(2))
    );
    assert_eq!(
        refused(&mut c, "rm = 1\nshow = 2\nPROJECT_ROOT = 3").await,
        ("this script binds names the session binds again before every run: line 2 'rm' (async function), line 3 'show' (helper), line 4 'PROJECT_ROOT' (constant). Rename them (for example 'rm_', 'show_', 'PROJECT_ROOT_'). Nothing in this script ran.".into(), Some(2))
    );
    for (code, line, start) in [
        (
            "def show(a, b):\n    pass",
            2,
            "line 2 defines 'show', a helper bound by this session.",
        ),
        (
            "class rm:\n    pass",
            2,
            "line 2 defines 'rm', an async function",
        ),
        (
            "from os import path as cat",
            2,
            "line 2 binds 'cat' with an import, an async function",
        ),
        ("for tree in [1]:\n    pass", 2, "line 2 binds 'tree',"),
        ("with open('x') as web:\n    pass", 2, "line 2 binds 'web',"),
        (
            "try:\n    pass\nexcept Exception as rm:\n    pass",
            4,
            "line 4 binds 'rm',",
        ),
        ("rm += [1]", 2, "line 2 binds 'rm',"),
        ("rm: list = []", 2, "line 2 binds 'rm',"),
        ("(rm := 1)", 2, "line 2 binds 'rm',"),
        ("[(rm := x) for x in [1]]", 2, "line 2 binds 'rm',"),
        (
            "match 1:\n    case rm:\n        pass",
            3,
            "line 3 binds 'rm',",
        ),
        (
            "match {'k': 1}:\n    case {'k': 1, **rm}:\n        pass",
            3,
            "line 3 binds 'rm',",
        ),
        (
            "match [1, 2]:\n    case [1, *rm]:\n        pass",
            3,
            "line 3 binds 'rm',",
        ),
        (
            "def f():\n    global rm\n    rm = 1",
            3,
            "line 3 binds 'rm',",
        ),
        (
            "class A:\n    def m(self):\n        global cat\n        cat = 1",
            4,
            "line 4 binds 'cat',",
        ),
        (
            "if True:\n    PROJECT_ROOT = 'x'",
            3,
            "line 3 binds 'PROJECT_ROOT', a constant set before every run.",
        ),
        (
            "def f(x=(cat := 1)):\n    return x",
            2,
            "line 2 binds 'cat',",
        ),
        ("f = lambda x=(cat := 1): x", 2, "line 2 binds 'cat',"),
        ("a, (b, *cat) = 1, (2, 3)", 2, "line 2 binds 'cat',"),
        (
            "import os, json as cat",
            2,
            "line 2 binds 'cat' with an import,",
        ),
        (
            "json = 1",
            2,
            "line 2 binds 'json', a module imported before every run.",
        ),
        (
            "import os.path as re",
            2,
            "line 2 binds 're' with an import, a module imported before every run.",
        ),
        (
            "Path = 1",
            2,
            "line 2 binds 'Path', the class pathlib.Path, imported before every run.",
        ),
        (
            "def ident(f):\n    return f\n@(cat := ident)\ndef g():\n    pass",
            4,
            "line 4 binds 'cat',",
        ),
        (
            "match [1]:\n    case [cat] | (cat,):\n        pass",
            3,
            "line 3 binds 'cat',",
        ),
        (
            "class P:\n    x = 1\nmatch P():\n    case P(x=cat):\n        pass",
            5,
            "line 5 binds 'cat',",
        ),
        (
            "with open('x') as (a, web):\n    pass",
            2,
            "line 2 binds 'web',",
        ),
        ("import rm.sub", 2, "line 2 binds 'rm' with an import,"),
        ("rm = await cat(paths=['x'])", 2, "line 2 binds 'rm',"),
    ] {
        let (message, at) = refused(&mut c, code).await;
        assert!(message.starts_with(start), "{code}: {message}");
        assert_eq!(at, Some(line), "{code}");
    }
    let (message, _) = refused(&mut c, "from os import path as cat").await;
    assert!(
        message.contains("(for example 'from os import path as cat_')"),
        "{message}"
    );
}

#[tokio::test]
async fn names_outside_module_scope_and_canonical_imports_stay_legal() {
    let mut c = client().await;
    for code in [
        "def f():\n    rm = 1\n    return rm\nf()",
        "class A:\n    rm = 1\nA.rm",
        "[cat for cat in [1]]",
        "(lambda cat: cat)(1)",
        "def f(cat):\n    return cat\nf(1)",
        "x = 'rm = 1'  # rm = 2",
        "class A:\n    pass\na = A()\na.rm = 1\nd = {}\nd['rm'] = 1",
        "dict(rm=1)",
        "y = PROJECT_ROOT",
        "import os\nimport os.path\nimport re\nimport json\nimport asyncio\nfrom pathlib import Path\nimport json as json",
        "rm: list",
        "from pathlib import Path as Path",
        "x = f'{rm}'",
    ] {
        assert_eq!(run(&mut c, code, session_globals()).await.map_err(|e| format!("{e:?}")), Ok(()), "{code}");
    }
    assert!(run(&mut c, "cat = 1", vec![]).await.is_ok());
}

#[tokio::test]
async fn interpreter_errors_win_over_the_binding_check() {
    let mut c = client().await;
    for code in [
        "rm = (",
        "rm = 1\ndel x",
        "try:\n    pass\nexcept* ValueError as rm:\n    pass",
        &format!("rm = {}1{}", "[".repeat(300), "]".repeat(300)),
    ] {
        match run(&mut c, code, session_globals()).await {
            Err(KernelEvent::Raised {
                message, ..
            }) => {
                assert!(
                    !message.ends_with("Nothing in this script ran."),
                    "{code}: {message}"
                );
                if code.contains("del x") {
                    assert!(message.contains("the 'del' statement"), "{code}: {message}");
                }
            }
            other => panic!("{code}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn module_level_comprehension_variables_named_like_builtins_are_refused_only_when_read() {
    let mut c = client().await;
    assert_eq!(
        refused(&mut c, "[id for id in ['a']]").await,
        ("line 2: the comprehension variable 'id' has the name of a builtin, and at module level this interpreter reads the builtin 'id' instead of the variable. Rename the variable (for example 'id_'). Nothing in this script ran.".into(), Some(2))
    );
    for code in [
        "{type: 1 for type in ['a']}",
        "sum(max for max in [1, 2])",
        "[[str for _ in [1]] for str in ['a']]",
        "[x for id in ['a'] for x in [id]]",
        "[(lambda q=sorted: q)() for sorted in ['a']]",
    ] {
        let (message, at) = refused(&mut c, code).await;
        assert!(
            message.starts_with("line 2: the comprehension variable '"),
            "{code}: {message}"
        );
        assert_eq!(at, Some(2), "{code}");
    }
    for code in [
        "[1 for id in ['a']]",
        "id = 5\n[id for id in ['a']]",
        "[(lambda: id)() for id in ['a']]",
        "def f():\n    return [id for id in ['a']]\nf()",
        "[input for input in ['a']]",
    ] {
        assert_eq!(
            run(&mut c, code, vec![])
                .await
                .map_err(|e| format!("{e:?}")),
            Ok(()),
            "{code}"
        );
    }
    let mut c = client().await;
    assert!(run(&mut c, "type = 5", vec![]).await.is_ok());
    assert!(run(&mut c, "[type for type in ['a']]", vec![])
        .await
        .is_ok());
}

#[tokio::test]
async fn the_comprehension_guard_asks_the_interpreter_and_calls_no_session_name() {
    let mut c = client().await;
    assert!(run(&mut c, "if False:\n    id = 0", vec![]).await.is_ok());
    assert_eq!(
        run(&mut c, "[id for id in ['a']]", vec![])
            .await
            .map_err(|e| format!("{e:?}")),
        Ok(())
    );
    let mut c = client().await;
    assert!(run(
        &mut c,
        "list = [1, 2]\ndef locals():\n    raise ValueError('a session name was called')",
        vec![]
    )
    .await
    .is_ok());
    let (message, at) = refused(&mut c, "[ord for ord in ['a']]").await;
    assert!(
        message.starts_with("line 2: the comprehension variable 'ord'"),
        "{message}"
    );
    assert_eq!(at, Some(2));
}

#[tokio::test]
async fn an_assignment_binds_its_target_only_after_its_value() {
    let mut c = client().await;
    for code in [
        "ord = [ord for ord in ['a']]",
        "reversed: list = [reversed for reversed in ['a']]",
        "(pow := [pow for pow in ['a']])",
    ] {
        let (message, at) = refused(&mut c, code).await;
        assert!(
            message.starts_with("line 2: the comprehension variable '"),
            "{code}: {message}"
        );
        assert_eq!(at, Some(2), "{code}");
    }
    assert_eq!(
        run(
            &mut c,
            "for divmod in [divmod for divmod in ['a']]:\n    pass",
            vec![]
        )
        .await
        .map_err(|e| format!("{e:?}")),
        Ok(())
    );
}

#[tokio::test]
async fn the_name_the_variable_listing_uses_is_reserved() {
    let mut c = client().await;
    for (code, line) in [
        ("__refact__ = 7", 2),
        ("import os as __refact__", 2),
        ("def f():\n    global __refact__\n    __refact__ = 1", 3),
    ] {
        assert_eq!(
            refused(&mut c, code).await,
            (format!("line {line} binds '__refact__', a name this session reserves for listing variables; a value kept there would be replaced. Rename it (for example 'refact_'). Nothing in this script ran."), Some(line)),
            "{code}"
        );
    }
    let (message, _) = refused(&mut c, "__refact__ = 1\nrm = 2").await;
    assert_eq!(message, "this script binds names the session binds again before every run: line 2 '__refact__' (reserved name), line 3 'rm' (async function). Rename them (for example 'refact_', 'rm_'). Nothing in this script ran.");
    assert_eq!(
        run(
            &mut c,
            "def f():\n    __refact__ = 1\n    return __refact__\nf()",
            session_globals()
        )
        .await
        .map_err(|e| format!("{e:?}")),
        Ok(())
    );
}

#[tokio::test]
async fn an_import_of_glob_never_gets_a_rename_that_still_fails() {
    let mut c = client().await;
    for code in ["from glob import glob", "import glob.x"] {
        let (message, at) = refused(&mut c, code).await;
        assert!(
            message.starts_with("line 2 imports glob, but glob is the file-search tool"),
            "{code}: {message}"
        );
        assert_eq!(at, Some(2), "{code}");
    }
    let (message, _) = refused(&mut c, "import glob\nrm = 1").await;
    assert_eq!(message, "this script binds names the session binds again before every run: line 2 'glob' (the glob tool; there is no glob module), line 3 'rm' (async function). Remove the glob import and rename the others (for example 'rm_'). Nothing in this script ran.");
}

#[tokio::test]
async fn an_aliased_glob_import_is_removed_not_renamed() {
    let mut c = client().await;
    for code in ["from glob import glob as rm", "import glob as cat"] {
        assert_eq!(
            refused(&mut c, code).await,
            ("line 2 imports glob, but glob is the file-search tool (await glob(pattern=...)); for local patterns use Path(dir).glob(...) or .rglob(...). Remove the import. Nothing in this script ran.".into(), Some(2)),
            "{code}"
        );
    }
    for (code, expected) in [
        (
            "from glob import glob as cat\nrm = 1",
            "line 2 'cat' (imported from glob; there is no glob module), line 3 'rm' (async function). Remove the glob import and rename the others (for example 'rm_').",
        ),
        (
            "import glob\nfrom glob import x as rm",
            "line 2 'glob' (the glob tool; there is no glob module), line 3 'rm' (imported from glob; there is no glob module). Remove the glob imports.",
        ),
    ] {
        let (message, _) = refused(&mut c, code).await;
        assert_eq!(
            message,
            format!("this script binds names the session binds again before every run: {expected} Nothing in this script ran."),
            "{code}"
        );
    }
}
