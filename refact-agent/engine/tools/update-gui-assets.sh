#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$ENGINE_DIR/../.." && pwd)"
GUI_DIR="$REPO_ROOT/refact-agent/gui"
ASSET_DIST_DIR="$ENGINE_DIR/assets/chat/dist"

cd "$GUI_DIR"

if [[ ! -d node_modules ]]; then
  npm ci
fi

npm run build

rm -rf "$ASSET_DIST_DIR/chat"
mkdir -p "$ASSET_DIST_DIR"
cp -R "$GUI_DIR/dist/chat" "$ASSET_DIST_DIR/chat"

echo "Copied GUI chat assets to $ASSET_DIST_DIR/chat"
find "$ASSET_DIST_DIR/chat" -maxdepth 2 -type f | sort
