#!/usr/bin/env bash
# ci-status.sh — show GitHub Actions run status: jobs + failed steps.
#
# Accepts a run URL, a job URL, or a bare run id. Prints each job's
# status/conclusion and pinpoints failed steps. Version-independent
# (uses `gh api`, works on old gh too).
#
# Usage:
#   tools/dev/ci-status.sh <run-url | job-url | run-id>
#   tools/dev/ci-status.sh https://github.com/JegernOUTT/refact/actions/runs/26750108081
#   tools/dev/ci-status.sh 26750108081
#
# Env:
#   GH_REPO   owner/repo (default JegernOUTT/refact)
set -uo pipefail

REPO="${GH_REPO:-JegernOUTT/refact}"
ARG="${1:-}"
if [ -z "$ARG" ]; then
  echo "usage: ci-status.sh <run-url | job-url | run-id>" >&2
  exit 2
fi

# Extract the run id from a URL or accept a bare numeric id.
run_id="$(printf '%s' "$ARG" | grep -oE 'runs/[0-9]+' | grep -oE '[0-9]+' | head -1)"
[ -z "$run_id" ] && run_id="$(printf '%s' "$ARG" | grep -oE '^[0-9]+$' || true)"
if [ -z "$run_id" ]; then
  echo "could not parse a run id from: $ARG" >&2
  exit 2
fi

echo "Run $run_id  ($REPO)"

# Run-level summary.
gh api "repos/$REPO/actions/runs/$run_id" \
  --jq '"  workflow: \(.name)\n  branch:   \(.head_branch)\n  sha:      \(.head_sha[0:10])\n  status:   \(.status) / \(.conclusion // "—")\n  url:      \(.html_url)"' 2>&1
echo
echo "Jobs:"
gh api "repos/$REPO/actions/runs/$run_id/jobs" --paginate \
  --jq '.jobs[] | "  \(if .conclusion=="success" then "✅" elif .conclusion=="failure" then "❌" elif .conclusion=="cancelled" then "🚫" elif .conclusion=="skipped" then "⏭️" else "⏳" end) \(.name) — \(.status)/\(.conclusion // "running")"' 2>&1

echo
echo "Failed steps:"
fail_steps="$(gh api "repos/$REPO/actions/runs/$run_id/jobs" --paginate \
  --jq '.jobs[] | select(.conclusion=="failure") | "  ✗ [\(.name)] step \(.steps[] | select(.conclusion=="failure") | "#\(.number) \(.name)") (job=\(.id))"' 2>&1)"
if [ -n "$fail_steps" ]; then
  printf '%s\n' "$fail_steps"
  echo
  echo "→ Fetch logs:  tools/dev/ci-logs.sh $run_id"
else
  echo "  (none — run not failed, or still in progress)"
fi
