#!/bin/sh
# Explicit offline scratch-only runner. Timing is reported, never asserted by tests.
set -eu
if [ "${BUDDY_SOAK_TIMING:-0}" != 1 ]; then
    echo 'Opt in with BUDDY_SOAK_TIMING=1 sh tools/dev/buddy-operator-soak.sh' >&2
    exit 2
fi
cd "$(dirname "$0")/../../refact-agent/engine"
# Cargo offline prevents dependency downloads; fixtures invoke no network adapters.
cargo test --offline --no-default-features --test it --no-run
# Build time is excluded from this informational wall-clock measurement.
time cargo test --offline --no-default-features --test it -- buddy_operator_soak:: --test-threads=1
