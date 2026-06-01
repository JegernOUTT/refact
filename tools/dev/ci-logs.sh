#!/usr/bin/env bash
# ci-logs.sh — dump the tail of each failed job's log from a GitHub Actions run.
#
# No error-guessing: failures are almost always at the end of the log, so we
# just show the last N lines (default 300) of each failed job. Uses `gh api`
# (works on any gh version; `gh run view --log-failed` is unreliable for
# reusable-workflow jobs).
#
# Usage:
#   tools/dev/ci-logs.sh <run-url | job-url | run-id> [tail-lines]
#   tools/dev/ci-logs.sh 26750108081           # last 300 lines per failed job
#   tools/dev/ci-logs.sh 26750108081 600        # last 600 lines
#
# Env:
#   GH_REPO      owner/repo (default JegernOUTT/refact)
#   TAIL_LINES   default tail length (default 300; overridden by arg 2)
set -uo pipefail

REPO="${GH_REPO:-JegernOUTT/refact}"
ARG="${1:-}"
TAIL_LINES="${2:-${TAIL_LINES:-300}}"
# An omitted optional integration param arrives as literal "%tail_lines%"; any
# non-numeric value (incl. that placeholder) falls back to the default.
case "$TAIL_LINES" in ''|*[!0-9]*) TAIL_LINES=300 ;; esac
if [ -z "$ARG" ] || [ "$ARG" = "%ref%" ]; then
  echo "usage: ci-logs.sh <run-url | job-url | run-id> [tail-lines]" >&2
  exit 2
fi

run_id="$(printf '%s' "$ARG" | grep -oE 'runs/[0-9]+' | grep -oE '[0-9]+' | head -1)"
[ -z "$run_id" ] && run_id="$(printf '%s' "$ARG" | grep -oE '^[0-9]+$' || true)"
if [ -z "$run_id" ]; then
  echo "could not parse a run id from: $ARG" >&2
  exit 2
fi

# Failed jobs: "id<TAB>name"
mapfile -t failed < <(gh api "repos/$REPO/actions/runs/$run_id/jobs" --paginate \
  --jq '.jobs[] | select(.conclusion=="failure") | "\(.id)\t\(.name)"' 2>/dev/null)

if [ "${#failed[@]}" -eq 0 ]; then
  echo "No failed jobs in run $run_id (passing, cancelled, or in progress)."
  echo "Status: tools/dev/ci-status.sh $run_id"
  exit 0
fi

echo "Run $run_id — ${#failed[@]} failed job(s); last $TAIL_LINES lines each:"
echo

for entry in "${failed[@]}"; do
  job_id="${entry%%	*}"
  job_name="${entry#*	}"
  echo "════════════════════════════════════════════════════════════"
  echo "❌ $job_name  (job=$job_id)"
  # Failed step name(s) for orientation.
  gh api "repos/$REPO/actions/jobs/$job_id" \
    --jq '.steps[]? | select(.conclusion=="failure") | "   ✗ step #\(.number): \(.name)"' 2>/dev/null || true
  echo "──────────────────── last $TAIL_LINES lines ────────────────────"
  # Raw log, timestamps stripped for readability, last N lines.
  gh api "repos/$REPO/actions/jobs/$job_id/logs" 2>/dev/null \
    | sed -E 's/^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z[[:space:]]?//' \
    | tail -n "$TAIL_LINES" \
    || echo "   (could not fetch log for job $job_id)"
  echo
done

echo "════════════════════════════════════════════════════════════"
echo "Full log:  gh api repos/$REPO/actions/jobs/<job_id>/logs"
echo "More tail: tools/dev/ci-logs.sh $run_id 600"
