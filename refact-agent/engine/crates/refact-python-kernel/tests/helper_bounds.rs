use refact_python_kernel::{
    hostfs::*,
    kernel::{HelperCall, HelperError, OsBackend},
    protocol::*,
};
use serde_json::{json, Value};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    io::Read,
    path::PathBuf,
};

struct Counting;
thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}
fn count(bytes: isize, call: bool) {
    if ON.try_with(Cell::get).unwrap_or(false) {
        let live = LIVE.with(|l| {
            l.set(l.get() + bytes);
            l.get()
        });
        PEAK.with(|p| p.set(p.get().max(live)));
        if call {
            CALLS.with(|c| c.set(c.get() + 1));
        }
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            count(layout.size() as isize, true);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        count(-(layout.size() as isize), false);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            count(size as isize - layout.size() as isize, true);
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What `f` returns, the most bytes it held at once and how many allocations it made.
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
    LIVE.with(|l| l.set(0));
    PEAK.with(|p| p.set(0));
    CALLS.with(|c| c.set(0));
    ON.with(|on| on.set(true));
    let out = f();
    ON.with(|on| on.set(false));
    (out, PEAK.with(Cell::get) as usize, CALLS.with(Cell::get))
}

const MIB: usize = 1024 * 1024;

struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("helper-bounds-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(
    f: &mut HostFs,
    name: &str,
    args: Value,
    kwargs: Value,
    max_result_bytes: u64,
    list_files: &mut dyn FnMut(&FileListRequest) -> Result<FileList, HelperError>,
) -> Result<Value, (String, String)> {
    match f.helper(HelperCall {
        name,
        args: args.as_array().cloned().unwrap_or_default(),
        kwargs: kwargs.as_object().cloned().unwrap_or_default(),
        marks: vec![],
        max_result_bytes,
        list_files,
    }) {
        Ok(value) => Ok(value),
        Err(HelperError::Raise(kind, message)) => Err((kind, message)),
        Err(_) => panic!("{name}() was interrupted"),
    }
}
fn no_listing(_: &FileListRequest) -> Result<FileList, HelperError> {
    panic!("unexpected file listing")
}

#[test]
fn read_holds_the_result_and_one_bounded_line_whatever_it_skips() {
    let t = Temp::new("read");
    let huge = "x".repeat(8 * MIB);
    std::fs::write(t.0.join("first.txt"), format!("{huge}\nshort\n")).unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        ..Default::default()
    });
    let limit = 64 * 1024;
    for (args, expected) in [
        (json!(["first.txt", 2, 2]), Ok(json!("   2 | short\n"))),
        (json!(["first.txt", -1]), Ok(json!("   2 | short\n"))),
        (json!(["first.txt", 1, 1]), Err("RuntimeError")),
        (json!(["first.txt", -2]), Err("RuntimeError")),
    ] {
        let (result, peak, _) = measure(|| {
            call(
                &mut f,
                "read",
                args.clone(),
                json!({}),
                limit,
                &mut no_listing,
            )
        });
        match expected {
            Ok(value) => assert_eq!(result, Ok(value), "{args}"),
            Err(kind) => assert_eq!(result.map_err(|e| e.0), Err(kind.to_string()), "{args}"),
        }
        assert!(
            peak < 3 * limit as usize + MIB / 4,
            "{args}: held {peak} bytes"
        );
    }
}

#[test]
fn grep_text_stops_building_rows_at_its_output_cut() {
    let t = Temp::new("grep");
    let text: String = (0..410_000)
        .map(|i| if i % 41 == 20 { "needle\n" } else { "hay\n" })
        .collect();
    std::fs::write(t.0.join("a.txt"), text).unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        ..Default::default()
    });
    let mut lists_itself = |request: &FileListRequest| {
        Ok(FileList {
            files: vec![request.path.clone()],
            ..Default::default()
        })
    };
    let mut peaks = vec![];
    for limit in [1, 10_000] {
        let (text, peak, _) = measure(|| {
            call(
                &mut f,
                "grep",
                json!(["needle", "a.txt"]),
                json!({"context": 20, "limit": limit}),
                16 * MIB as u64,
                &mut lists_itself,
            )
            .unwrap()
        });
        let text = text.as_str().unwrap().to_string();
        assert!(text.chars().count() < 210_000, "{limit}");
        peaks.push(peak);
    }
    assert!(
        peaks[1] < peaks[0] + 2 * MIB,
        "a limit of 10,000 held {} bytes, a limit of 1 held {}",
        peaks[1],
        peaks[0]
    );
}

#[test]
fn replace_renders_its_diff_only_up_to_the_cut() {
    let t = Temp::new("replace");
    let line = format!("x{}\n", "y".repeat(8 * 1024));
    std::fs::write(t.0.join("a.txt"), line.repeat(1000)).unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        ..Default::default()
    });
    let (_, kept, _) = measure(|| {
        let mut bytes = vec![];
        std::fs::File::open(t.0.join("a.txt"))
            .unwrap()
            .take(32 * MIB as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        let updated = std::str::from_utf8(&bytes).unwrap().replace("x", "z");
        (bytes.len(), updated.len())
    });
    let (diff, peak, _) = measure(|| {
        call(
            &mut f,
            "replace",
            json!(["a.txt", "x", "z"]),
            json!({"count": 0}),
            16 * MIB as u64,
            &mut no_listing,
        )
        .unwrap()
    });
    assert!(diff.as_str().unwrap().ends_with(" more diff lines\n"));
    assert!(
        peak < kept + MIB,
        "replace held {peak} bytes; the text before and after takes {kept}"
    );
}

#[test]
fn grep_resolves_its_rules_once_per_call() {
    let t = Temp::new("rules");
    std::fs::create_dir(t.0.join("d")).unwrap();
    let files: Vec<PathBuf> = (0..200)
        .map(|i| {
            let p = t.0.join(format!("d/f{i}.txt"));
            std::fs::write(&p, "needle\n").unwrap();
            p
        })
        .collect();
    let mut calls = vec![];
    for rules in [0, 20] {
        let mut f = HostFs::new(HostFsConfig {
            project_root: t.0.clone(),
            deny_read: PathDenyRules {
                prefixes: (0..rules)
                    .map(|i| t.0.join(format!("protected{i}/deep")))
                    .collect(),
                files: (0..rules)
                    .map(|i| t.0.join(format!("secret{i}.yaml")))
                    .collect(),
                allow_prefixes: vec![t.0.join("protected0/deep/allowed")],
                ..Default::default()
            },
            ..Default::default()
        });
        let mut listing = |_: &FileListRequest| {
            Ok(FileList {
                files: files.clone(),
                ..Default::default()
            })
        };
        let (_, _, made) = measure(|| {
            call(
                &mut f,
                "grep",
                json!(["needle", "d"]),
                json!({"data": true}),
                16 * MIB as u64,
                &mut listing,
            )
            .unwrap()
        });
        calls.push(made);
    }
    assert!(
        calls[1] < calls[0] + 20 * 200,
        "20 rules made {} allocations, none {}",
        calls[1],
        calls[0]
    );
}
