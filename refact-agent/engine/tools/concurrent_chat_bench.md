# Concurrent chat benchmark

Run the deterministic baseline fixture:

```bash
cd refact-agent/engine && cargo run -p refact-lsp --bin concurrent_chat_bench -- --quick
```

The command writes one JSON report to stdout using `refact.concurrent_chat_benchmark.v1`. It exercises the fixed 1/4/8/16/32-chat workload matrix, logical 1/10/50 MiB histories, 10/50/200 descriptors, write/index/watch/VecDB simulations, active/background GUI flushes, and sequence-gap snapshot recovery without providers or network access.

`--soak` materializes the full history fixtures and is an explicit local/release benchmark; CI runs the quick fixture and invariant tests only.
