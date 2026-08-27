#!/usr/bin/env bash
# perf.sh — run the engine performance benchmarks.
#
# These live behind the `perf-tests` feature so they stay out of the correctness
# gate: they execute real workloads and measure wall-clock, CPU and IO, so they
# are slow and only meaningful when run single-threaded on an idle machine.
#
# Usage:
#   tools/dev/perf.sh              # run every perf_harness benchmark
#   tools/dev/perf.sh fanout       # run only benchmarks matching "fanout"
#
# Note: the CPU and IO counters are Linux-only, so some assertions are skipped
# or unavailable on macOS and Windows.
set -euo pipefail

FILTER="${1:-chat::perf_harness}"

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root/refact-agent/engine"

if [ -n "$(pgrep -f 'cargo (test|build|check)' || true)" ]; then
  echo "⚠ Another cargo process is running; benchmark numbers will be unreliable." >&2
fi

echo "▶ Running engine performance benchmarks (filter: $FILTER)"
exec cargo test -p refact-lsp --lib --features perf-tests -- \
  "$FILTER" --test-threads=1 --nocapture
