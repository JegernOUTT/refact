#!/usr/bin/env bash
# check.sh — run pre-push checks only for components that changed.
#
# Auto-detects changed components (via changed.sh) and runs each component's
# checks in dependency order: engine -> gui -> vscode -> intellij -> docs.
# Runs in the main checkout or any worktree (checks the cwd's tree).
#
# Usage:
#   tools/dev/check.sh [components...]   # default: auto-detect via changed.sh
#   tools/dev/check.sh engine            # force-check only the engine
#   tools/dev/check.sh engine gui        # force-check engine + gui
#
# Env:
#   BASE_REF   base ref for change detection (default origin/main)
#
# Exit code: 0 if all checks pass, 1 if any fail. Prints a summary table.
set -uo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Drop any unsubstituted %placeholder% args. When this script is invoked via a
# cmdline integration, an omitted optional param arrives literally as "%components%"
# (replace_args only substitutes params the caller actually provided).
args=()
for a in "$@"; do case "$a" in %*%) ;; *) [ -n "$a" ] && args+=("$a") ;; esac; done
set -- "${args[@]+"${args[@]}"}"

# Components: from args, else auto-detect.
if [ "$#" -gt 0 ]; then
  components="$*"
else
  components="$(BASE_REF="${BASE_REF:-origin/main}" bash "$here/changed.sh" | tr '\n' ' ')"
fi

if [ -z "${components// }" ]; then
  echo "✓ No relevant components changed — nothing to check."
  exit 0
fi

echo "▶ Checking components: $components"
echo

results=""
overall=0

# record <component> <name> <exit-code>
record() {
  local status="✅ PASS"
  [ "$3" -ne 0 ] && { status="❌ FAIL"; overall=1; }
  results="${results}\n  ${status}  $1: $2"
}

# run <component> <name> <command...>
run() {
  local comp="$1" name="$2"; shift 2
  echo "── [$comp] $name ──"
  if "$@"; then record "$comp" "$name" 0; else record "$comp" "$name" $?; fi
  echo
}

for comp in $components; do
  case "$comp" in
    engine)
      ( cd refact-agent/engine && run engine "cargo fmt --check" cargo fmt --check )
      ( cd refact-agent/engine && run engine "cargo check"       cargo check )
      ( cd refact-agent/engine && run engine "cargo test --lib"  cargo test --lib )
      ( cd refact-agent/engine && run engine "cargo test --doc"  cargo test --doc )
      ;;
    gui)
      ( cd refact-agent/gui && run gui "format:check" npm run format:check )
      ( cd refact-agent/gui && run gui "types"        npm run types )
      ( cd refact-agent/gui && run gui "lint"         npm run lint )
      ( cd refact-agent/gui && run gui "test"         npm run test )
      ;;
    vscode)
      ( cd plugins/vscode && run vscode "compile" npm run compile )
      ( cd plugins/vscode && run vscode "lint"    npm run lint )
      ;;
    intellij)
      # Lightweight Kotlin compile check (full packaging = build-jb-plugin-local.sh)
      ( cd plugins/intellij && run intellij "compileKotlin" ./gradlew --offline compileKotlin )
      ;;
    docs|infra)
      echo "── [$comp] no automated checks (skipped) ──"; echo
      ;;
    *)
      echo "⚠ unknown component: $comp (skipped)"; echo
      ;;
  esac
done

echo "════════════════════════════════════════"
echo "Check summary:"
printf '%b\n' "$results"
echo "════════════════════════════════════════"
[ "$overall" -eq 0 ] && echo "✅ All checks passed." || echo "❌ Some checks failed."
exit "$overall"
