#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 0 ]]; then
    echo "Usage: $0" >&2
    exit 2
fi

if [[ ! -f Cargo.toml || ! -d src || ! -d crates/refact-core/src ]]; then
    echo "Run this script from refact-agent/engine" >&2
    exit 1
fi

declare -a SCENARIOS=()
declare -a SECONDS_RESULTS=()
declare -a STATUS_RESULTS=()

run_timed() {
    local scenario="$1"
    shift
    local log_file
    local time_file

    log_file=$(mktemp)
    time_file=$(mktemp)

    echo
    echo "== ${scenario} =="
    echo "$*"

    local status=0
    { /usr/bin/time -p "$@" > >(tee "${log_file}") 2> >(tee "${time_file}" >&2); } || status=$?

    local elapsed
    elapsed=$(awk '/^real / { value = $2 } END { if (value == "") value = "n/a"; print value }' "${time_file}")

    SCENARIOS+=("${scenario}")
    SECONDS_RESULTS+=("${elapsed}")
    STATUS_RESULTS+=("${status}")

    rm -f "${log_file}" "${time_file}"

    if [[ "${status}" -ne 0 ]]; then
        echo "Scenario failed with exit code ${status}" >&2
        exit "${status}"
    fi
}

print_report() {
    echo
    echo "Compile benchmark report"
    printf '%-56s %12s %8s\n' "Scenario" "Seconds" "Status"
    printf '%-56s %12s %8s\n' "--------" "-------" "------"

    local i
    for i in "${!SCENARIOS[@]}"; do
        printf '%-56s %12s %8s\n' "${SCENARIOS[$i]}" "${SECONDS_RESULTS[$i]}" "${STATUS_RESULTS[$i]}"
    done
}

main() {
    echo "Refact engine compile benchmark"
    echo "Working directory: $(pwd)"
    echo "This script only cleans the refact-lsp package, not the whole target directory."

    echo
    echo "== Preparing cold refact-lsp check =="
    cargo clean -p refact-lsp
    run_timed "Cold refact-lsp check" cargo check -p refact-lsp

    touch src/lib.rs
    run_timed "Incremental check after touching src/lib.rs" cargo check -p refact-lsp

    touch crates/refact-core/src/lib.rs
    run_timed "Incremental workspace check after touching refact-core" cargo check --workspace

    print_report
}

main
