# Concurrent chat benchmark

Run the provider-free quick baseline:

```bash
cd refact-agent/engine && cargo run -p refact-lsp --bin concurrent_chat_bench -- --quick
```

The command writes one `refact.concurrent_chat_benchmark.v1` JSON report to stdout. It runs the fixed 1/4/8/16/32-chat, logical 1/10/50 MiB, 10/50/200-descriptor trajectory matrix plus a provider-free `turn-tool-pool-8-chats-50-tools` fixture in an isolated Tokio workspace. Quick mode caps materialized history at 8 KiB and reports both logical and materialized history bytes.

Every trajectory workload saves real `ChatSession` snapshots through `save_trajectory_snapshot`, performs same-directory concurrent trajectory saves and rapid same-chat checkpoints, reloads a saved trajectory to verify integrity, and runs real index upsert/rebuild paths. Files and byte amplification are derived from filesystem deltas.

The turn-pool fixture serializes `REFACT_TOOL_CATALOG_SNAPSHOTS` while comparing `legacy` (`0`) and `pooled` (`1`) behavior with the same deterministic seed. It creates 8 isolated sessions, prepares schemas and aliases, drives actual confirmation/preflight and execution paths for 50 calls per chat, and includes an 8-call same-name parallel batch. Local runtime returns an immediate result and is reported separately from `tool_start_overhead_latency`.

`tool_pool_workload.counters` reports immutable catalog builds, mutable vector builds, parallel-vector expansions, confirmation/preflight starts and checks, execution lookups, runtime calls, and errors. Its comparison reports p50/p95/p99 latency summaries, catalog/preflight operation reduction, and `remaining_stage` whenever the ≥80% reduction, <100 ms p95 tool-start, or <1 ms warm schema/alias threshold misses; T-12 should use that stage as its measured-only input. The report intentionally contains no simulated GUI timing, watcher, VecDB, or provider/MCP/network activity.

Run the full-history soak benchmark explicitly in a release build:

```bash
cd refact-agent/engine && cargo run --release -p refact-lsp --bin concurrent_chat_bench -- --soak
```

CI runs the bounded quick fixtures and structural invariant tests; soak is for deliberate local before/after measurements.

Run the provider-free full-system fixture in CI-sized form through its unit test, or collect the release soak report explicitly:

```bash
cd refact-agent/engine && cargo test -p refact-lsp --lib chat::perf_harness::tests::full_soak_ci_fixture_starts_required_subsystems -- --test-threads=1
cd refact-agent/engine && cargo run --release -p refact-lsp --bin concurrent_chat_bench -- --full-soak > /tmp/refact-full-soak.json
```

`--full-soak` uses an isolated workspace and exercises actual chat sessions, queue processors, trajectory writer/index coordinator, trajectory watcher, CodeGraph's in-memory service and background scheduler, Buddy, task/goal and background-agent monitors, scheduler, exec registry, and session-cleanup startup. It uses deterministic local generation deltas and local tools; it makes no provider or network calls. The report compares `legacy` and `optimized` rollout switches serially on the same executable and machine, and labels this accurately as a **synthetic same-version comparison**, not a historical Wave 0 baseline.

The report records p95 queue wait, first delta, checkpoint return, required flush, precise end-to-end tool-call latency, its session/history, catalog/pool, alias, confirmation, hook, execution, postprocess/privacy, session-merge/event, and checkpoint stages, SSE serialization/broadcast, trajectory/index write bytes, session queue counters, monitor/cleanup scans, CPU/RSS, process IO, errors, tool ordering, and trajectory restore results. The production VecDB initializer requires embedding credentials, so the no-network fixture substitutes a local recording backend and declares that limitation in every report; CodeGraph remains an actual in-memory service whose background task drains the isolated workspace queue.

Run the focused high-rate delta and large-history fanout fixture with:

```bash
cd refact-agent/engine && cargo run --release -p refact-lsp --bin concurrent_chat_bench -- --fanout > /tmp/refact-chat-fanout.json
```

`--fanout` streams 512 deltas through a 3-active/1-lagging subscriber setup against a 256-message, 1 MiB history. It reports deltas/sec, operations/bytes per delta, emit-lock wait, SSE serialize/broadcast and first-delta latency, large-history snapshot clone/serialization bytes and time, subscriber counts, and verified lag recovery. It fails if an active subscriber lags or recovery sequence monotonicity is broken.
