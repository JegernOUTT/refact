use super::hint;
use refact_python_kernel::{client::*, hostfs::HostFs, protocol::*};
use std::time::Duration;

/// The exception `code` raises in this interpreter, as (type, message).
async fn raised(code: &str, globals: &[&str]) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
    let mut client =
        KernelClient::spawn_in_thread(Box::new(HostFs::default()), ClientOptions::default())
            .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        client.next_event().await;
        client
            .send(KernelCommand::Configure {
                project_root: dir.path().into(),
                env: Default::default(),
                capture_dir: None,
                deny_write: Default::default(),
                deny_read: Default::default(),
            })
            .await
            .unwrap();
        client
            .send(KernelCommand::Feed {
                run_id: "hint".into(),
                code: code.into(),
                globals: globals
                    .iter()
                    .map(|name| GlobalBinding {
                        name: (*name).into(),
                        kind: GlobalKind::AsyncFn,
                    })
                    .collect(),
                limits: RunLimits::default(),
            })
            .await
            .unwrap();
        loop {
            match client.next_event().await {
                ClientEvent::Kernel(KernelEvent::Raised {
                    exc_type,
                    message,
                    ..
                }) => break (exc_type, message),
                ClientEvent::Kernel(KernelEvent::Completed {
                    ..
                }) => panic!("{code}: did not fail"),
                ClientEvent::Kernel(KernelEvent::LimitExceeded {
                    message, ..
                }) => panic!("{code}: {message}"),
                ClientEvent::Kernel(KernelEvent::SyncCall {
                    call_id,
                    function,
                    ..
                }) => client
                    .send(KernelCommand::SyncResult {
                        call_id,
                        result: HostResult::Err {
                            exc_type: "NameError".into(),
                            message: format!("name '{function}' is not defined"),
                        },
                    })
                    .await
                    .unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn every_hint_matches_an_error_this_interpreter_raises() {
    let deep = format!("x = {}1{}", "[".repeat(300), "]".repeat(300));
    for (code, globals, start) in [
        ("x = 1\ndel x", &[][..], "del is not supported"),
        ("next(f for f in ['a'])", &[], "Generator expressions"),
        ("re.subn('a', 'b', 'aaa')", &[], "Use new = re.sub"),
        ("re.compile('a').subn('b', 'aaa')", &[], "Use new = re.sub"),
        (
            "re.sub('a+', lambda m: m.group(0).upper(), 'xaay')",
            &[],
            "Build the result with re.finditer",
        ),
        ("Path('/a/b').parents", &[], "Use [Path(*p.parts"),
        ("Path('/a/b').relative_to('/a')", &[], "Use os.path.relpath"),
        (
            "Path('/a/b').is_relative_to('/a')",
            &[],
            "Use os.path.relpath",
        ),
        ("Path('~').expanduser()", &[], "Use os.path.expanduser"),
        ("Path.home()", &[], "Use os.path.expanduser"),
        ("Path('x').home()", &[], "Use os.path.expanduser"),
        ("Path('x').touch()", &[], "Use if not p.exists()"),
        ("Path('a.py').match('*.py')", &[], "Use fnmatch.fnmatch"),
        ("os.readlink('f.txt')", &[], "Symlink calls are unavailable"),
        ("os.lstat('f.txt')", &[], "Symlink calls are unavailable"),
        (
            "os.symlink('f.txt', 'g.txt')",
            &[],
            "Symlink calls are unavailable",
        ),
        (
            "Path('g.txt').symlink_to('f.txt')",
            &[],
            "Symlink calls are unavailable",
        ),
        (
            "Path('f.txt').samefile('f.txt')",
            &[],
            "Symlink calls are unavailable",
        ),
        (
            "sorted([Path('f.txt')], key=lambda p: p.stat().st_size)",
            &[],
            "File, tool and helper calls cannot run inside callbacks",
        ),
        ("list(open('f.txt'))", &[], "Use for line in f:"),
        (deep.as_str(), &[], "Split long"),
        (
            "class E(Exception):\n    pass",
            &[],
            "Subclassing is not supported",
        ),
        ("def g():\n    yield 1", &[], "Generators are not supported"),
        (
            "def g():\n    yield from [1]",
            &[],
            "Generators are not supported",
        ),
        (
            "class A:\n    @property\n    def x(self):\n        return 1",
            &[],
            "Use plain methods",
        ),
        (
            "async def f():\n    async with x:\n        pass",
            &[],
            "Use await on plain coroutines",
        ),
        (
            "async def f():\n    async for v in x:\n        pass",
            &[],
            "Use await on plain coroutines",
        ),
        (
            "raise KeyError(5)",
            &[],
            "Exceptions take at most one str argument",
        ),
        (
            "Path('f.txt').read_text(encoding='utf-8')",
            &[],
            "Files are always UTF-8 here",
        ),
        (
            "Path('f.txt').write_text('a', encoding='utf-8')",
            &[],
            "Files are always UTF-8 here",
        ),
        (
            "open('f.txt', errors='ignore')",
            &[],
            "Read bytes and decode",
        ),
        ("open('f.txt', 'r+')", &[], "Open the file once"),
        ("open('g.txt', 'x')", &[], "Check os.path.exists"),
        ("print('a', file=open('g.txt', 'w'))", &[], "Build the text"),
        (
            "open('g.txt', 'w').writelines(['a'])",
            &[],
            "Use f.write(''.join(lines))",
        ),
        (
            "import sys\nsys.stdout.write('a')",
            &[],
            "sys.stdout.write and sys.stderr.write are missing",
        ),
        ("import sys\nsys.exit(1)", &[], "End the script normally"),
        (
            "asyncio.create_task",
            &[],
            "Only asyncio.gather, run and sleep exist",
        ),
        (
            "await asyncio.gather(asyncio.sleep(0), return_exceptions=True)",
            &[],
            "Catch errors per call",
        ),
        ("str.maketrans('a', 'b')", &[], "Translate with a dict"),
        (
            "import functools\nfunctools.wraps",
            &[],
            "Drop @functools.wraps",
        ),
        (
            "from dataclasses import asdict",
            &[],
            "Build the dict explicitly",
        ),
        (
            "import collections\ncollections.OrderedDict",
            &[],
            "Plain dict keeps insertion order",
        ),
        (
            "sorted([Path('b'), Path('a')])",
            &[],
            "Paths are not orderable",
        ),
        ("glob.glob('*')", &["glob"], "glob is the file-search tool"),
        (
            "d = {'a': 1}\nmax(d, key=d.get)",
            &[],
            "Methods cannot be used as values",
        ),
        (
            "max(['a'], key=str.lower)",
            &[],
            "Methods cannot be used as values",
        ),
        ("import subprocess", &[], "Run commands with the shell tool"),
        ("import requests", &[], "Fetch URLs with the web tool"),
        ("import urllib.parse", &[], "Fetch URLs with the web tool"),
        ("import shutil", &[], "Copy with Path(dst)"),
        ("import threading", &[], "Use await asyncio.gather"),
        (
            "import collections.abc",
            &[],
            "Import these names from typing",
        ),
        ("import glob", &[], "There is no glob module"),
        (
            "import textwrap",
            &[],
            "This runtime has no 'textwrap' module",
        ),
        ("callable(len)", &[], "callable() does not exist"),
    ] {
        let (exc_type, message) = raised(code, globals).await;
        let found = hint(&exc_type, &message);
        assert!(
            found.as_deref().is_some_and(|h| h.starts_with(start)),
            "{code}: {exc_type}: {message} -> {found:?}"
        );
    }
}

#[test]
fn unrelated_errors_get_no_hint() {
    for (exc_type, message) in [
        ("ValueError", "invalid literal for int() with base 10: 'x'"),
        ("AttributeError", "'dict' object has no attribute 'foo'"),
        ("AttributeError", "'dict' object has no attribute 'home'"),
        ("AttributeError", "'N' object has no attribute 'parents'"),
        (
            "AttributeError",
            "'N' object has no attribute 'relative_to'",
        ),
        (
            "AttributeError",
            "'list' object has no attribute 'writelines'",
        ),
        (
            "AttributeError",
            "'str' object has no attribute 'format_map'",
        ),
        ("NameError", "name 'foo' is not defined"),
        ("ZeroDivisionError", "division by zero"),
        (
            "TypeError",
            "unsupported operand type(s) for +: 'int' and 'str'",
        ),
    ] {
        assert_eq!(hint(exc_type, message), None, "{exc_type}: {message}");
    }
}
