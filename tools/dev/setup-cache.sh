#!/usr/bin/env bash
# setup-cache.sh — enable sccache so Rust builds are shared across worktrees.
#
# Each git worktree has its own target/ dir, so cold builds recompile ~85 crates
# (10-20 min). sccache caches compiled artifacts globally, so a fresh worktree
# build becomes cache hits instead of cold. It is parallel-safe (unlike a shared
# CARGO_TARGET_DIR, which locks and blocks simultaneous worktree builds).
#
# Idempotent. Safe to re-run.
#
# Usage:
#   tools/dev/setup-cache.sh           # configure + verify
#   tools/dev/setup-cache.sh --status  # just show current cache state
set -euo pipefail

CARGO_CONFIG="${CARGO_HOME:-$HOME/.cargo}/config.toml"
BASHRC="$HOME/.bashrc"
CACHE_SIZE="${SCCACHE_CACHE_SIZE:-50G}"

show_status() {
  echo "=== Build cache status ==="
  if command -v sccache >/dev/null 2>&1; then
    echo "sccache: $(sccache --version)"
    sccache --show-stats 2>/dev/null | grep -iE "cache location|max cache size|cache hits rate|compile requests$" || true
  else
    echo "sccache: NOT installed"
  fi
  echo
  echo "Worktree target/ disk usage:"
  du -sh "$HOME/.cache/refact/worktrees"/*/*/refact-agent/engine/target 2>/dev/null | sort -rh | head -8 || echo "  (none)"
}

if [ "${1:-}" = "--status" ]; then
  show_status
  exit 0
fi

# 1. sccache must be installed.
if ! command -v sccache >/dev/null 2>&1; then
  echo "sccache not found. Install it first:"
  echo "  cargo install sccache    # or: apt/brew install sccache"
  exit 1
fi

# 2. Point cargo at sccache as the rustc wrapper.
mkdir -p "$(dirname "$CARGO_CONFIG")"
touch "$CARGO_CONFIG"
if grep -q 'rustc-wrapper' "$CARGO_CONFIG" 2>/dev/null; then
  echo "✓ rustc-wrapper already set in $CARGO_CONFIG"
else
  cp "$CARGO_CONFIG" "${CARGO_CONFIG}.bak.$(date +%s)" 2>/dev/null || true
  printf '\n[build]\nrustc-wrapper = "sccache"\n' >> "$CARGO_CONFIG"
  echo "✓ Added rustc-wrapper = sccache to $CARGO_CONFIG"
fi

# 3. Env vars: incremental MUST be off for sccache to cache; bump cache size.
if ! grep -q 'SCCACHE_CACHE_SIZE' "$BASHRC" 2>/dev/null; then
  {
    printf '\n# sccache: shared Rust compilation cache across worktrees\n'
    printf 'export SCCACHE_CACHE_SIZE="%s"\n' "$CACHE_SIZE"
    printf 'export CARGO_INCREMENTAL=0\n'
  } >> "$BASHRC"
  echo "✓ Added SCCACHE_CACHE_SIZE=$CACHE_SIZE and CARGO_INCREMENTAL=0 to $BASHRC"
else
  echo "✓ sccache env vars already in $BASHRC"
fi

# 4. Start the server and verify.
export SCCACHE_CACHE_SIZE="$CACHE_SIZE"
sccache --start-server 2>/dev/null || true
echo
show_status
echo
echo "✅ sccache enabled. Open a new shell (or 'source ~/.bashrc') so CARGO_INCREMENTAL=0 applies."
echo "   Watch hit rate climb across builds with: tools/dev/setup-cache.sh --status"
