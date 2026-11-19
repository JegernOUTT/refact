use refact_python_kernel::{client::*, hostfs::HostFs, kernel::FakeOs, protocol::*};
use serde_json::json;
use std::time::Duration;

async fn next(c: &mut KernelClient) -> KernelEvent {
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
async fn client(os: Box<dyn refact_python_kernel::kernel::OsBackend>) -> KernelClient {
    let mut c = KernelClient::spawn_in_thread(os, ClientOptions::default()).unwrap();
    next(&mut c).await;
    c
}
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("shim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
async fn value(c: &mut KernelClient, code: &str) -> Result<serde_json::Value, String> {
    c.send(KernelCommand::Feed {
        run_id: "t".into(),
        code: code.into(),
        globals: vec![],
        limits: RunLimits::default(),
    })
    .await
    .unwrap();
    match next(c).await {
        KernelEvent::Completed {
            value, ..
        } => Ok(value),
        KernelEvent::Raised {
            exc_type,
            message,
            ..
        } => Err(format!("{exc_type}: {message}")),
        other => panic!("{code}: {other:?}"),
    }
}

#[tokio::test]
async fn io_is_served_by_the_kernel_without_project_files() {
    let mut c = client(Box::new(FakeOs::default())).await;
    assert_eq!(
        value(&mut c, "import io\nio.__file__").await,
        Ok(json!("/refact-shims/io.py"))
    );
}

#[tokio::test]
async fn io_string_and_bytes_streams_round_trip() {
    let mut c = client(Box::new(FakeOs::default())).await;
    let code = "import io, csv
s = io.StringIO()
json.dump({'a': [1, 2]}, s)
s.seek(0)
loaded = json.load(s)
out = io.StringIO()
w = csv.writer(out)
w.writerow(['x', 'y'])
w.writerow([1, 2])
rows = list(csv.DictReader(io.StringIO(out.getvalue())))
o = io.StringIO('hello')
o.write('J')
o.seek(0, 2)
o.write('!')
with io.StringIO('x') as closed:
    pass
try:
    closed.read()
    after_close = 'read'
except ValueError:
    after_close = 'ValueError'
b = io.BytesIO(b'abc')
[loaded, rows, o.getvalue(), after_close, b.read(2) == b'ab']";
    assert_eq!(
        value(&mut c, code).await,
        Ok(json!([{"a": [1, 2]}, [{"x": "1", "y": "2"}], "Jello!", "ValueError", true]))
    );
}

#[tokio::test]
async fn the_io_shim_wins_over_a_project_file() {
    let dir = scratch("precedence");
    std::fs::write(dir.join("io.py"), "X = 1\n").unwrap();
    let mut c = client(Box::new(HostFs::default())).await;
    c.send(KernelCommand::Configure {
        project_root: dir.clone(),
        env: Default::default(),
        capture_dir: None,
        deny_write: Default::default(),
        deny_read: Default::default(),
    })
    .await
    .unwrap();
    assert_eq!(
        value(&mut c, "import io\n[io.__file__, hasattr(io, 'X')]").await,
        Ok(json!(["/refact-shims/io.py", false]))
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn an_imported_shim_survives_a_snapshot() {
    let mut c = client(Box::new(FakeOs::default())).await;
    assert!(value(&mut c, "import io\nbuf = io.StringIO('kept')")
        .await
        .is_ok());
    let dir = scratch("snapshot");
    let path = dir.join("state.monty").to_string_lossy().into_owned();
    c.send(KernelCommand::Snapshot {
        path: path.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        next(&mut c).await,
        KernelEvent::SnapshotDone { .. }
    ));
    let mut c = client(Box::new(FakeOs::default())).await;
    c.send(KernelCommand::LoadSnapshot {
        max_state_bytes: 1 << 30,
        expected_sha256: {
            use sha2::Digest;
            sha2::Sha256::digest(std::fs::read(&path).unwrap())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        },
        path,
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut c).await, KernelEvent::Loaded));
    assert_eq!(
        value(&mut c, "[buf.getvalue(), io.StringIO('new').read()]").await,
        Ok(json!(["kept", "new"]))
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn io_streams_iterate_by_line_and_refuse_negative_seeks_and_closed_use() {
    let mut c = client(Box::new(FakeOs::default())).await;
    let code = "import io
s = io.StringIO('a\\nb\\nc')
first = next(s)
for line in s:
    second = line
    break
rest = s.read()
pos = s.tell()
lines = io.StringIO('x\\ny').readlines()
one = io.StringIO('p\\nq').readline()
def error(f):
    try:
        f()
        return None
    except Exception as e:
        return type(e).__name__
b = io.BytesIO(b'xy')
b.close()
s.close()
[first, second, rest, pos, lines, one,
 error(lambda: io.StringIO('x').seek(-1)),
 error(lambda: io.BytesIO(b'x').seek(-1)),
 error(lambda: b.read()),
 error(lambda: b.getvalue()),
 error(lambda: s.flush()),
 error(lambda: next(s))]";
    assert_eq!(
        value(&mut c, code).await,
        Ok(json!([
            "a\n",
            "b\n",
            "c",
            5,
            ["x\n", "y"],
            "p\n",
            "ValueError",
            "ValueError",
            "ValueError",
            "ValueError",
            "ValueError",
            "ValueError"
        ]))
    );
}
