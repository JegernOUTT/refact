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
