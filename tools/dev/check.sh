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

# run <component> <name> <dir> <command...>
# The subshell wraps ONLY `cd && command`; the `if` reads its exit code in the
# PARENT shell, so results/overall persist (a subshell around the whole record
# would lose them and silently swallow failures).
run() {
  local comp="$1" name="$2" dir="$3"; shift 3
  echo "── [$comp] $name ──"
  if ( cd "$dir" && "$@" ); then
    results="${results}\n  ✅ PASS  $comp: $name"
  else
    results="${results}\n  ❌ FAIL  $comp: $name"
    overall=1
  fi
  echo
}

for comp in $components; do
  case "$comp" in
    engine)
      run engine "cargo fmt --check" refact-agent/engine cargo fmt --check
      run engine "cargo check"       refact-agent/engine cargo check
      run engine "cargo test --lib"  refact-agent/engine cargo test --lib
      run engine "cargo test --doc"  refact-agent/engine cargo test --doc
      ;;
    gui)
      run gui "format:check" refact-agent/gui npm run format:check
      run gui "types"        refact-agent/gui npm run types
      run gui "lint"         refact-agent/gui npm run lint
      run gui "test"         refact-agent/gui npm run test
      ;;
    vscode)
      run vscode "compile" plugins/vscode npm run compile
      run vscode "lint"    plugins/vscode npm run lint
      ;;
    intellij)
      # Lightweight Kotlin compile check (full packaging = build-jb-plugin-local.sh)
      run intellij "compileKotlin" plugins/intellij ./gradlew --offline compileKotlin
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
