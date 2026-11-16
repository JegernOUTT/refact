# Python kernel: Monty 1.6.0 gate

This crate implements the Monty worker used by the engine's `python` tool. The gate notes below describe the original interpreter API probes; the worker sections describe the implemented protocol and backend. `tests/monty_smoke.rs` exercises the published interpreter on the repository's pinned `nightly-2026-11-04` (rustc 1.101.0-nightly). The interpreter requires Rust 1.98. Type checking is optional and off by default.

## (a) Persistent expressions — verified

`MontyRepl::new(script_name, ResourceTracker, CompileOptions)` creates the session. `MontyRepl::feed_run(code, NamedValues, PrintWriter)` returns `MontyObject`; two feeds retain `x`. `feed_start` also exists, consuming the session and returning `Result<ReplProgress, Box<ReplStartError>>`.

## (b) Async host function — verified

Supply `MontyObject::async_function(name, None)` in `NamedValues`. `ReplProgress::FunctionCall(ReplFunctionCall)` exposes `function_name`, `is_async`, and `call_id`. `ReplFunctionCall::resume_pending` returns an external future to Python. `ReplProgress::ResolveFutures(ReplResolveFutures)` suspends the await; `ReplResolveFutures::resume(Vec<(u32, ExtFunctionResult)>, print)` supplies results. `ReplFunctionCall::resume` can supply an immediate value instead.

## (c) Concurrent gather — verified

`await asyncio.gather(f(), g())` produces both `FunctionCall`s, each answered with `resume_pending`, before `ResolveFutures`. Resolving IDs in reverse order still returns the Python result list in input order. No host result is needed to start the second call.

## (d) File/OS dispatch — verified with a different API

`ReplProgress::OsCall(ReplOsCall)` and `ReplOsCall::resume_with(print, closure)` are the host handler API; **there is no `OsHandler` trait** in `monty` or `monty-types`. The closure receives `monty_types::OsFunctionCall` by value. Tested variants are `Open(OpenCallArgs)`, `ReadText(MontyPath)`, and `Getenv(GetenvArgs)`. `open` expects a `MontyObject::file_handle(MontyFileHandle { path, mode, position })` answer. Other variants cover filesystem checks/mutations, clocks, sleep, entropy, imports and type hints. `ImportArgs::candidates()` exists and yields ordered virtual module paths; module reading remains the host's responsibility.

## (e) Idle dump/load — verified

The assumed API exists: `monty::dump(script_name, None, &[], SessionRef::Idle(&repl))` returns bytes; `Dump::load(&bytes)?.state` contains `Session::Idle(Box<MontyRepl>)`. Variables survive. Only load authenticated, trusted snapshots: the upstream contract says decoding is not validation and malicious bytes can panic, abort or hang the process.

## (f) Memory/time limits — verified with an important limitation

`ResourceTracker` is a **concrete struct**, not an implementable trait. `ResourceLimits::max_feed_duration` and `max_turn_duration` limit execution excluding host suspension; the default clock is elapsed execution wall time. `ResourceTracker::with_execution_clock(ExecutionClock::Cpu)` requests Linux thread CPU accounting, but requires `monty-types` feature `cpu-clock`; it is not enabled by this scaffold's requested `tzdb` feature. Unsupported CPU clocks fall back to elapsed time. The test ends `while True: pass` after a 5ms execution budget. It is not a resumable time slice.

`ResourceLimits.max_memory` plus `check_allocation`/`check_large_result` rejects oversized individual operations even without an allocator (tested with a billion-character string). Resource errors escape `except BaseException`; memory errors carry `ExcType::MemoryError` and `memory limit exceeded`, time errors carry `time limit exceeded`, unlike an ordinary `ZeroDivisionError`. Error strings are not a dedicated host-side resource error discriminator; ordinary code can deliberately raise similarly named exceptions. The interpreter internally uses `ResourceError::{Memory, Time}`.

### Enforcement options

* **(a) `monty-alloc` installed:** the MIT-licensed `monty-alloc` 1.6.0 crate supplies `LimitedAllocator`, a `#[global_allocator]` forwarding to `std::alloc::System`. It counts all process allocations, not only Python, through atomic live-byte increments/decrements and a hard-limit load per allocation; no measured overhead is claimed. `set_hard_limit(memory_limit_with_headroom(...))` establishes/lowers the process baseline and arms a ceiling relative to that baseline. Default interpreter headroom is 4MiB (32MiB for type checking). Soft checkpoint violations raise `MemoryError`; hard overflow **aborts the whole process**, or exits with status 65 under `exit-code`. Direct `mmap`, thread stacks and mapped code are not counted. This is designed for isolated workers, not concurrent in-process sessions.
* **(b) Up-front checks only:** our strongest option without changing the engine allocator. Individual excessive allocations are rejected, but many small allocations can accumulate beyond the nominal limit. A bounded regression test explicitly demonstrates this gap. This is **not a sandbox memory ceiling**.
* **(c) Custom tracker:** unavailable: no trait, allocation callback, cancellation callback, or pluggable tracker parameter. There is no safe per-session allocator counter extension in this API.

The engine already installs `#[global_allocator] static GLOBAL_ALLOC: mimalloc::MiMalloc` in `src/lib.rs`. Rust permits only one global allocator; `LimitedAllocator` is not generic over an allocator and does not compose with MiMalloc as shipped. Replacing it would affect every engine allocation and a hard overflow would kill the engine. This task does not install or add a dependency on `monty-alloc`.

## (g) Cancellation — verified with a different API

There is no custom `ResourceTracker` implementation or `Arc<AtomicBool>` hook for interrupting executing bytecode from another thread. Every suspended progress type supports recovery through `into_repl`; `ReplProgress::into_repl` abandons the current execution while preserving globals. `ReplFunctionCall`, `ReplNameLookup`, `ReplOsCall`, and `ReplResolveFutures` also expose `abort(exception, print)` for uncatchable host aborts. The test abandons a host-call suspension and verifies earlier variables survive.

Use a short `max_turn_duration` to bound uninterrupted execution, then inspect host cancellation state at host round trips. If a script never suspends, the budget terminates the snippet; there is **no re-arm-and-resume instruction continuation** after a timeout. A user cancellation can be labelled separately by host state at suspension, but an actual timeout remains a compute-budget failure, not proof of cross-thread cancellation.

## (h) Dedicated worker thread — verified

Compile-time `Send` assertions cover `MontyRepl` and `ReplProgress`. A test moves an idle session to `std::thread::spawn` and evaluates there. This is not a claim of `Sync` or safe simultaneous access.

## (i) Global summary — verified with a different API

No direct global getter exists; `function_names` only lists callable definitions. Feed `locals()` at module scope with `MontyRepl::feed_run` to obtain a `MontyObject` dictionary of global names and values. The test compares the returned dictionary on the host with `MontyObject::dict`. Summary collection executes another snippet and must use budgets/output bounds; globals may include objects that are expensive to export.

## (j) Print capture — verified

`PrintWriter::CollectString(&mut String, Some(max_bytes))` captures stdout in emission order (tested). `CollectStreams` retains stream labels; `Callback` supports a host callback. `Disabled` discards print output. Use an explicit cap; `None` allows unbounded collection.

## Assumed progress/error names

All five assumed variants exist: `ReplProgress::{FunctionCall, NameLookup, OsCall, ResolveFutures, Complete}`. `Complete` is a struct variant `{ repl, value }`; `into_complete()` extracts both. `NameLookup` resumes with `NameLookupResult` (smoke tested). `ReplStartError` exists with public `repl` and `error` fields; failures preserve the session. Source comments suggesting syntax failures consume the session conflict with the actual `feed_start` implementation, which returns a `ReplStartError` too.

## Python feature surface

Smoke tested: user classes and methods, f-strings, list comprehensions, try/except, `json`, `re`, `datetime`, `pathlib`, `os` imports, and top-level await. This does not imply full CPython or arbitrary extension-module support. File and environment operations require host answers; imports outside the built-in module set require host module handling.

## Repository registration

Both crates are workspace members, internal version 0.1.0. `tests/monty_smoke.rs` is Cargo-auto-discovered. `tools/bump_release_version.py` automatically exempts internal 0.1.0 crates, so no registration is needed. `tools/dev/ci_test_inventory.json` and `tools/dev/ci_tests.py` do not exist in this checkout; no deny configuration was found. No files outside the assigned targets were modified.

## Optional static type checking: lock compatibility

`typecheck = ["dep:monty-type-checking"]` is retained and builds with Monty 1.6.0. Fresh resolution selected `ruff_python_ast` 0.0.14 and Salsa 0.28.5, failing with E0407 (`hash` is not a member of `salsa::HashEqLike`) at `name.rs:176,219,232`. Ruff declares Salsa 0.28.2. Pinning only `salsa`/`salsa-macros` left `salsa-macro-rules` 0.28.5 and failed in `ruff_db` with E0046/E0061. The lockfile pins **all three Salsa packages to 0.28.2**, after which feature compilation passes. These packages are only in the optional typechecking dependency graph; default Monty remains 1.6.0. Avoid blindly updating these pins until upstream compatibility is fixed.

Available crates.io versions observed: Monty, monty-types and monty-type-checking 1.6.0, 1.5.0, 1.4.0, 1.3.0, 1.2.0, 1.1.0, 1.0.0 (all unyanked); Salsa 0.28.5/0.28.4/0.28.2/0.28.1/0.28.0 (0.28.3 yanked). No interpreter version substitution was made.

## Machine/build facts

Observed 32 logical CPUs; 125GiB RAM (60GiB available at measurement), 127GiB swap; worktree and `~/.cache` share `/dev/nvme1n1p1`, 1.8TiB total, 56GiB free (97% used). `tools/dev/setup-cache.sh --status`: effective rustc-wrapper unset; incremental unset; sccache executable exists but is not enabled. No cache-hit/daemon claim is made.

The initial cold-with-respect-to-Monty `cargo check -p refact-tool-api -p refact-python-catalog -p refact-python-kernel` took approximately 31.7s including registry resolution/downloads; existing project artifacts/downloads were not deleted. This is the first check that built the kernel, not a pristine whole-workspace benchmark.

# Worker implementation

`Kernel` owns an idle `Option<MontyRepl>` and consumes/reclaims it across progress states.
`KernelIo` supplies blocking command reception and event emission. `OsBackend: Send`
handles one `OsFunctionCall`, returning `MontyObject` or `(exception_type, message)`;
`take_events` drains filesystem audit events. `DenyOs` rejects every operation with
`OSError`; the production worker supplies HostFs (open native access with configured deny rules). `FakeOs`
provides a map of text files for tests.

`worker::run_worker(reader, writer, os)` sends `Ready` first and uses stdout only for
protocol traffic. The standalone `refact-python-worker` binary calls
`run_worker_stdio`. Production engine integration self-execs its executable
with `--python-worker`; `src/main.rs` detects that argument before initializing
engine services and dispatches to `run_worker_stdio` with HostFs.

Every frame is a little-endian u32 JSON byte length followed by exactly that many
UTF-8 JSON bytes, capped at 64 MiB before allocation. Serde's externally tagged enum
representation is used. `protocol.rs` is the wire schema: commands are Configure, Feed,
Results, SyncResult, Abort, Snapshot, LoadSnapshot, Reset, Inspect, Shutdown; events
include host-call requests, stdout, filesystem audits, completion/error/limit,
snapshot/load acknowledgements and variable summaries. `Feed.run_id` is an opaque
String carried by every run-scoped event; one client processes one feed at a time. Async
functions bound as `GlobalKind::AsyncFn` accept keyword arguments only (positional arguments
raise TypeError). `GlobalKind::AsyncFnPositional(names)` declares positional parameter names:
leading positional arguments map to those keyword names in order before `CallStarted`, while
extra positional arguments or a name given both ways raise TypeError. SyncCall.args is a JSON
positional array; when there are no positional arguments, the `name`/`p` keywords are folded into it
as before. Other keyword arguments travel in `kwargs` (a JSON object, omitted when empty and null in
older frames). `show` (`protocol::DISPLAY_FUNCTION`) converts its arguments with the separate
`RunLimits.max_display_nodes` / `max_display_bytes` budget (defaults 1,000,000 / 32 MiB).
Abort is only a suspension reply, not a cross-thread interrupt.

Changed protocol shapes (Rust field notation; wire enums remain externally tagged):

```text
Results { run_id: String, results: Vec<(u32, HostResult)> }
Abort { run_id: String }
LoadSnapshot { path: String, expected_sha256: String }
CallStarted { run_id: String, call_id: u32, function: String, kwargs: Value, truncated: bool, line: Option<u32>, col: Option<u32> }
SyncCall { run_id: String, call_id: u32, function: String, args: Value, kwargs: Value, truncated: bool }
NeedResults { run_id: String, pending: Vec<u32> }
Stdout { run_id: String, text: String }
Fs { run_id: String, op: String, path: String, bytes: u64 }
WriteCapture { run_id: String, path: String, original: WriteOriginal }
Completed { run_id: String, value: Value, repr: String, truncated: bool }
Raised { run_id: String, exc_type: String, message: String, traceback: String, line: Option<u32> }
LimitExceeded { run_id: String, kind: LimitKind, message: String }
Cancelled { run_id: String }
ProtocolError { message: String }
LimitKind = "memory" | "compute" | "output"
```

CallStarted `line`/`col` locate the call expression (Monty `ReplFunctionCall.position`) in the
fed code: `line` is 1-based, `col` counts Unicode scalar values from the line start (0-based).
Raised `line` is the 1-based line of the innermost traceback frame in the fed code. Both are
reported only when the position belongs to this feed's snippet (`MontyRepl::next_script_name()`
taken right before the user code is fed, after the `asyncio` guard feed); a call or raise inside
a function defined by an earlier feed points into that earlier snippet and reports none (a raise
then reports this feed's calling frame). The fields are omitted when absent and default to none,
so older frames still parse.

Results and Abort whose run_id differs from the active feed are silently ignored,
including after completion. Invalid command JSON at an intact frame boundary emits
KernelEvent::ProtocolError and continues reading, including during suspension.
Oversized outgoing events emit a small ProtocolError rather than ending the worker.

`KernelClient::spawn(WorkerCommand, ClientOptions)` is synchronous and requires an
active Tokio runtime. `send`, `next_event`, and `kill` are async. `pid` is optional
(thread clients have no pid). `WorkerCommand` contains `program: PathBuf`,
`args: Vec<String>`, `env: HashMap<String,String>`, `cwd: Option<PathBuf>`.
`ClientOptions` contains `memory_limit_bytes: u64` and `memory_poll_ms: u64`;
defaults are 128 MiB and 25 ms; zero memory disables the RSS watchdog. The engine
uses its configured memory limit and a 250 ms poll. Process launch clears the inherited
environment before applying `WorkerCommand.env`; the engine separately configures
Python getenv values and the intended working directory. `ClientEvent` is
`Kernel(KernelEvent)`, `ProtocolError { message: String }`,
`MemoryExceeded { rss_bytes: u64 }`, or
`Exited { code: Option<i32>, stderr_tail: String }` (last 16 KiB, lossy UTF-8).
Shutdown drains final events before Exited. Drop requests kill and the supervisor
reaps the child. On Unix a std-thread waitpid fallback kills and reaps the direct child if runtime
shutdown drops the supervisor. A malformed/oversized/partially read worker frame
emits ClientEvent::ProtocolError, kills the worker, then delivers Exited. Clean
EOF at a frame boundary is normal process exit.

## Resource and state semantics

RunLimits has `max_memory_bytes: u64` (128 MiB), `max_compute_ms: u64` (1000 ms),
`max_value_nodes: u32` (200,000), `max_value_bytes: u64` (2 MiB), and
`max_arg_bytes: u64` (16 MiB), `max_result_nodes: u32` (8,388,609) and
`max_result_bytes: u64` (16 MiB). The conversion limits have serde defaults
for older JSON commands. A fresh resource tracker is installed each feed. Monty guards excessive
individual operations and execution time excluding suspension. RSS is a separate,
process-wide sampled ceiling; it includes Rust/code/runtime overhead, and may
overshoot between polls. The watchdog kills and waits for the child before emitting
MemoryExceeded, then Exited. Explicit kill/Drop loses all unsnapshotted state;
restart requires a new client. External stop is process kill, not Python Abort.

Python exceptions, syntax errors, Monty memory/compute errors, and Abort at a host
suspension recover Monty's returned REPL, preserving globals already committed
(including assignments preceding an exception). There is no instruction continuation
following a limit. Limit classification uses Monty exception types (TimeoutError /
MemoryError), not exception message substrings. Output collection has an independent
flag and reports LimitKind::Output.

`spawn_in_thread(os, options)` exists only for dependant tests. It runs the same
Kernel on a dedicated 64 MiB-stack thread with bounded command channels; no RSS
watchdog or safe forced thread termination is possible. Dropping/kill disconnects
commands and terminates at the next suspension or execution budget. Shutdown emits
Exited after dropping Kernel. It is not production isolation.

`run_worker_stdio` runs the worker loop on a thread with the same 64 MiB stack
(`worker::INTERPRETER_STACK_SIZE`), so the interpreter does not depend on the
platform's main-thread stack (1 MiB on Windows).

Print capture is bounded to 1 MiB cumulatively per feed. Inspect runs
`locals()` under a 200 ms budget, excludes underscore names and bindings from the
last feed, caps at 200 items and 120 UTF-8 bytes per preview. Monty
must materialize the locals result before truncation; inspect is not a streaming
global-memory API. Previews are formatted incrementally and stop immediately at
their node, depth or byte budget. Non-JSON values export as `{"__repr__": "..."}`; string-keyed
dicts and tuples/lists export recursively. A user dict of exactly the shape
`{"__repr__": "..."}` is indistinguishable from the repr fallback.

Each Feed's Completed conversion uses `max_value_nodes` / `max_value_bytes`;
CallStarted kwargs and SyncCall args share `max_value_nodes` / `max_arg_bytes`
across that call's arguments. Both byte limits are clamped to 32 MiB, with JSON
escaping charged to the byte budget to stay below the 64 MiB frame limit.
Depth remains capped at 32. Inspect retains its fixed 4096-node / depth-32 traversal and 120-byte previews.
Completed repr and fallback repr remain capped at 4096 UTF-8 bytes; cutting only
the Completed repr does not mark a fully exported JSON value truncated.
Over-budget output values become bounded
`{"__repr__": "..."}` fallbacks, with `truncated: true` on Completed, CallStarted,
SyncCall, or Variables. JSON u64 values use Python big integers, never
floating-point conversion.

JSON going into a script (async call results, SyncResult values and `GlobalKind::Json`
globals) is never cut. The worker measures each value (`convert::json_size`: exact
serialized bytes, node count, depth) against `max_result_bytes` (clamped to 32 MiB),
`max_result_nodes` and depth 32. A value that fits converts whole. A call result that does not
fit resumes the awaiting call with RuntimeError `f() returned a result of N bytes and M values,
over the limit a script can receive at once (L bytes, K values); narrow the call (...)`,
naming the function (and the depth when that is the cause); the session stays usable. A
global that does not fit fails the feed with Raised RuntimeError naming the global. The node
rule is `convert::result_nodes_for(bytes) = min(bytes, 32 MiB) / 2 + 1`: the smallest value in a
container (`0,`) takes two bytes, so any value within the byte limit also fits the node limit
(a 16 MiB result of small dicts passes); the node limit only matters when set lower explicitly.
`KernelClient::send` refuses what a frame cannot carry instead of cutting it: a Json global
over 32 MiB or depth 32 is an `InvalidData` send error, an Ok result over that becomes a
RuntimeError result, and results answered together in one Results frame (gather) whose sum
would pass the 64 MiB frame limit have their largest members replaced by RuntimeError
`a result of N bytes did not fit: results awaited together must stay under ... in total`.
Measured in a debug build: a 2 MiB result of 100,000 dicts (300,002 values) takes 8 ms to
measure and about 115 ms to convert; a 16 MiB text result 95 ms and 100 ms; send to Completed
(frame encode, pipe, decode, convert, run) is about 0.45 s for either with a process worker.
Conversion runs on the worker thread or process between protocol reads, never on the engine's
run loop, so Stop is not delayed beyond that time.

Snapshots use idle dumps, create-new temporary files, sync and rename. SHA-256
reports the bytes written. **Before LoadSnapshot, the engine must authenticate the
actual file bytes**: hash, then verify HMAC with the secret key and chat id.
`snapshot::sign(key: &[u8], sha256_hex: &str, chat_id: &str) -> String` and
`verify(key, sha256_hex, chat_id, signature: &str) -> bool` use HMAC-SHA256 over the
32 decoded digest bytes followed by chat-id UTF-8. The engine uses the taint-aware variants described below, appending the canonical taint digest; legacy signatures are not accepted for engine state. Invalid hex is rejected (sign
returns an empty string). The worker intentionally has no signing secret and does
not authenticate LoadSnapshot itself. The authenticated digest must be passed as
expected_sha256; the worker hashes the bytes it actually read and emits LoadFailed
on mismatch before Dump::load. Monty dump decoding is unsafe for untrusted bytes;
path capability validation remains an engine responsibility. Snapshots must live in a trusted,
engine-owned directory. Failed load keeps the previous idle session.

## HostFs backend (production worker default)

This is **not confinement**: the worker follows symlinks and accepts absolute paths
outside the project, subject to process credentials and configured protected-path rules.
`HostFsConfig` holds `project_root: PathBuf`, `env: BTreeMap<String,String>`,
`deny_write: PathDenyRules`, `deny_read: PathDenyRules`,
`capture_dir: Option<PathBuf>`, `max_capture_file_bytes: u64` (2 MiB), and
`max_capture_run_bytes: u64` (32 MiB). Unconfigured workers use their current
working directory and an empty Python environment, not inherited launch credentials.
Relative paths resolve against the project root; Monty's initial cwd is set to
that root. Python `os.chdir` changes its interpreter cwd for later calls/feeds.
`Configure` explicitly resets that cwd and per-run capture/accounting state.
The engine must send Configure before each Feed. Capture size limits are backend
configuration, not fields in the wire command.

Pure `host_path(path, Platform)` and `virtual_path(path, Platform)` preserve Unix
spelling and map Windows `C:\dir\file` to/from `/C:/dir/file`; both modes are
unit tested on Unix. `virtual_path` drops the verbatim prefix of drive and UNC paths
(`\\?\C:\x` becomes `/C:/x`), so `Path.resolve()` lines up with `PROJECT_ROOT`.
Paths in audit events are absolute host spellings, retaining
symlink components (the filesystem follows them, without canonical-path confinement).

### Complete Monty 1.6.0 dispatch table

| OsFunctionCall variant | HostFs service |
| --- | --- |
| Exists | `Path::try_exists`, following links |
| IsFile | `Path::is_file`, following links |
| IsDir | `Path::is_dir`, following links |
| IsSymlink | `Path::is_symlink`, no final-link following |
| ReadText | whole-file UTF-8 `read_to_string` |
| ReadBytes | whole-file `read` |
| Stat | metadata; Unix native stat fields, portable metadata fallback elsewhere |
| Iterdir | `read_dir`, absolute virtual Path entries |
| Scan | upstream `collect_scan` over native listings/lookups, serving scandir/walk/glob/rglob |
| Resolve | `canonicalize`, requires existing target |
| Absolute | absolute host spelling without following symlinks |
| WriteText | capture then create/truncate/write UTF-8; character count |
| AppendText | capture then create/append UTF-8; character count |
| WriteBytes | capture then create/truncate/write; byte count |
| AppendBytes | capture then create/append; byte count |
| Open | native open-time read/check/create/truncate, return MontyFileHandle |
| CreateExclusive | validate args, atomic create_new, Unix permissions subject to umask |
| Mkdir | create_dir/create_dir_all; parents/exist_ok checked separately |
| Unlink | capture then remove_file |
| Rmdir | remove_dir (empty directories only) |
| Rename | capture source and destination, native rename, return destination Path |
| Getenv | configured env lookup or supplied default |
| GetEnviron | configured env dictionary only |
| DateToday | host local calendar date |
| DateTimeNow | host clock, local naive or requested fixed offset |
| Urandom | OS entropy, capped at 32 MiB per request |
| Time | epoch seconds, or monotonic elapsed seconds for monotonic/perf_counter callers |
| Sleep | blocking thread sleep |
| SystemSleep | blocking thread sleep |
| AsyncSleep | blocking immediate answer; no concurrent host timer scheduling |
| AsyncSystemSleep | blocking immediate answer; no concurrent host timer scheduling |
| Import | ordered `ImportArgs::candidates()` reads; `handle_result` returns ModuleFile |
| ImportStubs | empty stub dictionary (no host stub catalog) |
| GetTypeHints | empty hints dictionary (no registered host type metadata) |

There are no separate file-handle OS variants: Monty manages buffering, position,
seek/tell, close/flush and context-manager state; reads/writes become ReadText,
ReadBytes, WriteText/Bytes and AppendText/Bytes. The default `OsBackend::handle_result`
wraps `handle`; HostFs overrides it for the non-object ModuleFile import response.
Calling its legacy `handle(Import(...))` directly cannot represent ModuleFile and
returns OSError; kernel dispatch uses `handle_result`.

### Wire additions and provenance

Serde externally tagged commands/events retain these exact Rust shapes:

```rust
KernelCommand::Configure {
    project_root: PathBuf,
    env: BTreeMap<String, String>,
    capture_dir: Option<PathBuf>,
    deny_write: PathDenyRules,
    deny_read: PathDenyRules,
}
KernelEvent::Fs { run_id: String, op: String, path: String, bytes: u64 }
KernelEvent::WriteCapture { run_id: String, path: String, original: WriteOriginal }
```

`WriteOriginal` is internally tagged by `kind` with snake_case variants:
`{"kind":"missing"}`, `{"kind":"file","capture_path":"/absolute/captures/0"}`,
or `{"kind":"too_large"}`. Existing files with capture disabled also report
`too_large` (no original content retained). First mutations emit one capture per
host path per configuration, before writing/appending/truncating/deleting or
renaming; directories are not content-captured. Captures use create-new numbered
files, skip occupied numbers, and never overwrite earlier captures. Copy failures
fail the operation instead of silently destroying an uncaptured original.
Fs events use Monty operation names, report content byte counts (zero for metadata
and structural mutations), and rename records both endpoints. Scan additionally
records every directory listed and metadata path consulted. Fs represents successful
operations, not a promise that a failed syscall could not have partial effects.
Native errors map to FileNotFoundError, PermissionError, IsADirectoryError,
NotADirectoryError, FileExistsError, otherwise OSError, preserving the OS message.
Reading or opening a directory and listing a non-directory are detected before the
I/O, so they raise the same exception on every OS.

### Python file API accuracy

Verified end to end: file-module imports, glob, append with capture reset,
`pathlib.Path.write_text/read_text`, `open('r'/'w'/'a')`, context
managers, `read(size)`, `seek`, `tell`, `closed`, `os.getenv`, and `os.listdir`.
Binary and append modes use the same tested native byte/text backends. Monty's
read handles cache a whole-file snapshot; text positions are character indices,
not CPython opaque byte cookies. Handles do not keep native descriptors alive.
`flush`/`close` are interpreter bookkeeping after immediate writes.

Upstream rejects `open` exclusive `'x'` and update `'+'` modes, nondefault encodings
(other than UTF-8) and error handlers. Text `seek` differs from CPython cookies;
iterating an unread file through synchronous consuming builtins such as `list(f)`
requires first loading its buffer (`read`/`seek`), unlike `for`/`next`. HostFs does
not add unavailable CPython modules or native extensions. Resolve currently requires
an existing target. Scan work is bounded to 100,000 visits and 32 MiB of selector
state; exceeding these bounds raises OSError, including pathological symlink cycles.
Async sleep blocks the worker, so it does not interleave sibling Python coroutines
while sleeping. These are API limitations, not filesystem security boundaries.

## Engine protected-state contract

Direct script writes remain allowed in read-only modes. The engine configures deny-write
prefixes from the extension loader (global `.claude`, config, plugins and all workspace
`.claude`/`.refact` directories), effective-root configuration and the chat workspace.
`PathDenyRules.allow_prefixes` has a serde default and wins over write denials only
after resolving symlinks; the engine allows only the chat's `artifacts/`. Ancestor
checks apply to unlink/rmdir/rename, not mkdir or ordinary file creation. Captures
and worker snapshots are engine writes, not script calls through HostFs.
Reads deny the whole config directory, custom secrets file, protected integrations.d
locations and home `.claude/.credentials.json`; RUNS_DIR and trajectories remain open.
PermissionError directs changes to engine configuration/state through file-edit tools.
Python's configured environment matches shell policy with variables.yaml replacements
only, excluding secrets.yaml. Protected import candidates propagate PermissionError.

The engine signs `sha256(state.monty) || chat_id || sha256(canonical JSON taint)`
using `snapshot::{sign_with_taint,verify_with_taint}`. Forks carry signed taint and
remap copied artifact ledger paths. Quota discovery is bounded by depth/entries,
never follows symlinks and recognizes parseable run.json structure across legacy,
child, task, Buddy and global layouts; only old terminal inactive runs are pruned.
Outer jobs hold normal chat/workspace/worktree scopes plus PythonRun. Nested jobs
hold the parent's PythonRun scope only, avoiding scope-change drain deadlocks.
`protocol::MAX_STDOUT_BYTES` is the shared 1 MiB output ceiling; stdout.txt stores
all accepted bytes including the prefix of an oversized print, and stdout_bytes
matches that saved file. Excess output terminates with LimitKind::Output.
