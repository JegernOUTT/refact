#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
ORACLE="${REPO_ROOT}/tools/dev/review_bench_oracle.json"
BENCH_DIR="${REVIEW_BENCH_DIR:-${HOME}/.cache/refact/review-bench}"

usage() {
  cat <<'USAGE'
review_bench.sh - score the review tool against a pinned oracle commit

  review_bench.sh prepare        Create a worktree at the oracle commit and print the review call
  review_bench.sh score <file>   Score a saved ReviewReport json against the oracle
  review_bench.sh clean          Remove the bench worktree

The review itself runs inside a chat, not from this script: open a chat in the prepared
worktree and issue the review call that `prepare` prints, then save the tool result's
metering `review_report` object to a file and pass it to `score`.
USAGE
}

oracle_field() {
  python3 -c "import json,sys;print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$ORACLE" "$1"
}

cmd_prepare() {
  local commit worktree
  commit="$(oracle_field commit)"
  worktree="${BENCH_DIR}/${commit}"
  mkdir -p "$BENCH_DIR"
  if [ ! -d "$worktree" ]; then
    git -C "$REPO_ROOT" worktree add --detach "$worktree" "$commit"
  fi
  echo "worktree: $worktree"
  python3 - "$ORACLE" "$worktree" <<'PY'
import json, sys
oracle = json.load(open(sys.argv[1]))
worktree = sys.argv[2]
files = [f"{worktree}/{path}" for path in oracle["scope"]]
call = {
    "what_to_check": "browser tool contract, lifecycle, downloads and error reporting",
    "files": files,
    "base": f"{oracle['commit']}~1",
    "depth": "deep",
    "scope_mode": "strict",
    "parallel_depth": 4,
}
print("\nrun this in a chat opened on the prepared worktree:\n")
print(f"review({json.dumps(call, indent=2)})")
print("\nthen save the tool result metering `review_report` to a file and run:")
print("  tools/dev/review_bench.sh score <file>")
PY
}

cmd_score() {
  local report="${1:-}"
  if [ -z "$report" ] || [ ! -f "$report" ]; then
    echo "usage: review_bench.sh score <review_report.json>" >&2
    exit 2
  fi
  python3 - "$ORACLE" "$report" <<'PY'
import json, sys

oracle = json.load(open(sys.argv[1]))
report = json.load(open(sys.argv[2]))
if "review_report" in report:
    report = report["review_report"]

findings = report.get("findings", [])

def is_hypothesis(finding):
    if finding.get("disputed"):
        return True
    return not finding.get("reproduction") and not finding.get("evidence_present")

def text(finding):
    return " ".join(
        str(finding.get(key, ""))
        for key in ("title", "claim", "evidence", "fix", "file")
    ).lower()

def matches(entry, finding):
    body = text(finding)
    hits = sum(1 for keyword in entry["keywords"] if keyword.lower() in body)
    return hits >= max(2, len(entry["keywords"]) - 1)

supported = [f for f in findings if not is_hypothesis(f)]
hypotheses = [f for f in findings if is_hypothesis(f)]
reproduced = [f for f in supported if f.get("reproduction")]

found, missed = [], []
for entry in oracle["true_defects"]:
    hit = next((f for f in findings if matches(entry, f)), None)
    (found if hit else missed).append(entry["id"])

false_positives = []
for entry in oracle["known_false_positives"]:
    hit = next((f for f in supported if matches(entry, f)), None)
    if hit:
        false_positives.append((entry["id"], hit.get("id", "?")))

total = len(oracle["true_defects"])
recall = len(found) / total if total else 0.0
repro_share = len(reproduced) / len(supported) if supported else 0.0
duration_min = report.get("duration_ms", 0) / 60000

print(f"findings          : {len(findings)} ({len(supported)} supported, {len(hypotheses)} hypotheses)")
print(f"reproduced        : {len(reproduced)} ({repro_share:.0%} of supported; target {oracle['targets']['reproduced_share']:.0%})")
print(f"recall            : {len(found)}/{total} = {recall:.0%} (target {oracle['targets']['recall']:.0%})")
print(f"known FPs shown   : {len(false_positives)} (target 0) {false_positives if false_positives else ''}")
print(f"duplicates merged : {report.get('duplicates_merged', 0)}")
print(f"wall clock        : {duration_min:.1f} min (target {oracle['targets']['wall_clock_minutes']} min)")
print("\nstages:")
for stage in report.get("stages", []):
    reason = f" — {stage['reason']}" if stage.get("reason") else ""
    coverage = stage.get("coverage", {})
    print(
        f"  {stage.get('name','?'):<14} {stage.get('status','?'):<10}"
        f" {stage.get('duration_ms',0)//1000:>4}s"
        f" files={len(coverage.get('files_read', []))}"
        f" cmds={len(coverage.get('commands_run', []))}"
        f"{reason}"
    )
if missed:
    print("\nmissed defects:")
    for entry in missed:
        print(f"  {entry}")

failures = []
if recall < oracle["targets"]["recall"]:
    failures.append("recall below target")
if false_positives:
    failures.append("known false positive presented as supported")
if supported and repro_share < oracle["targets"]["reproduced_share"]:
    failures.append("too few reproduced findings")
if not report.get("stages"):
    failures.append("no stage rows")
print("\nRESULT:", "FAIL — " + "; ".join(failures) if failures else "PASS")
sys.exit(1 if failures else 0)
PY
}

cmd_clean() {
  local commit worktree
  commit="$(oracle_field commit)"
  worktree="${BENCH_DIR}/${commit}"
  if [ -d "$worktree" ]; then
    git -C "$REPO_ROOT" worktree remove --force "$worktree"
  fi
  echo "removed $worktree"
}

case "${1:-}" in
  prepare) cmd_prepare ;;
  score) shift; cmd_score "$@" ;;
  clean) cmd_clean ;;
  *) usage; exit 2 ;;
esac
