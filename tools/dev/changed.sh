#!/usr/bin/env bash
# changed.sh — detect which components changed vs a base ref.
#
# Prints one component per line from: engine gui vscode intellij docs infra
# Works in the main checkout or any worktree. Considers committed changes since
# the merge-base with the base branch, plus staged, unstaged, and untracked files.
#
# Usage:
#   tools/dev/changed.sh [base-ref]      # default base-ref: origin/main
#
# Examples:
#   tools/dev/changed.sh                 # components changed vs origin/main
#   tools/dev/changed.sh main            # compare vs local main
set -euo pipefail

BASE_REF="${1:-origin/main}"

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

# Resolve a merge-base so we only see this branch's own changes, not everything
# that landed on the base branch since we forked. Fall back gracefully.
base_commit=""
for ref in "$BASE_REF" main origin/main HEAD; do
  if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
    base_commit="$(git merge-base "$ref" HEAD 2>/dev/null || true)"
    [ -n "$base_commit" ] && break
  fi
done
[ -n "$base_commit" ] || base_commit="HEAD"

{
  # Committed on this branch since base
  git diff --name-only "$base_commit"...HEAD 2>/dev/null || true
  # Staged + unstaged
  git diff --name-only --cached 2>/dev/null || true
  git diff --name-only 2>/dev/null || true
  # Untracked (new files not yet added)
  git ls-files --others --exclude-standard 2>/dev/null || true
} | sort -u > /tmp/.changed_files.$$

# Map changed paths to component names.
components=""
add() { case " $components " in *" $1 "*) ;; *) components="$components $1" ;; esac; }

while IFS= read -r f; do
  [ -z "$f" ] && continue
  case "$f" in
    refact-agent/engine/*)        add engine ;;
    refact-agent/gui/*)           add gui ;;
    plugins/vscode/*)             add vscode ;;
    plugins/intellij/*)           add intellij ;;
    docs/*)                       add docs ;;
    .github/*|tools/*|*.sh)       add infra ;;
  esac
done < /tmp/.changed_files.$$
rm -f /tmp/.changed_files.$$

# Emit, dependency order: engine -> gui -> plugins -> docs/infra
for c in engine gui vscode intellij docs infra; do
  case " $components " in *" $c "*) echo "$c" ;; esac
done
