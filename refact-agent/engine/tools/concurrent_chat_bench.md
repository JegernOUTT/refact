# Concurrent chat benchmark

Run the provider-free quick baseline:

```bash
cd refact-agent/engine && cargo run -p refact-lsp --bin concurrent_chat_bench -- --quick
```

The command writes one `refact.concurrent_chat_benchmark.v1` JSON report to stdout. It runs the fixed 1/4/8/16/32-chat, logical 1/10/50 MiB, 10/50/200-descriptor matrix in an isolated Tokio workspace. Quick mode caps materialized history at 8 KiB and reports both logical and materialized history bytes.

Every workload saves real `ChatSession` snapshots through `save_trajectory_snapshot`, performs same-directory concurrent trajectory saves and rapid same-chat checkpoints, reloads a saved trajectory to verify integrity, and runs real index upsert/rebuild paths. Files and byte amplification are derived from filesystem deltas. The report aggregates diagnostics for snapshot, serialization, atomic write, commit, index lock/read/write/rebuild, and actual `AppToolRegistry` catalog construction. The catalog is deterministic local built-ins only: it performs no provider, MCP, or network operation.

Only the `legacy` variant is emitted until an optimized engine configuration exists. The report intentionally contains no simulated GUI timing, watcher, VecDB, or future optimized-counter claims.

Run the full-history soak benchmark explicitly in a release build:

```bash
cd refact-agent/engine && cargo run --release -p refact-lsp --bin concurrent_chat_bench -- --soak
```

CI runs the bounded quick fixture and invariant tests; soak is for deliberate local before/after measurements.
