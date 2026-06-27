#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
REFACT_AGENT_DIR="$REPO_ROOT/refact-agent"
GUI_DIR="$REFACT_AGENT_DIR/gui"
REFACT_BINARY="$REFACT_AGENT_DIR/engine/target/debug/refact"

WEBVIEW_DIST_DIR="$SCRIPT_DIR/src/main/resources/webview/dist"
BIN_DIR="$SCRIPT_DIR/src/main/resources/bin"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "This script currently supports only Linux x86_64 local development."
  exit 1
fi

echo "=== Building debug LSP binary with embedded engine-served GUI assets ==="
(cd "$REFACT_AGENT_DIR/engine" && cargo build)

echo "=== Building GUI for IntelliJ webview resources ==="
cd "$GUI_DIR"
npm install
npm run build

echo "=== Copying GUI dist ==="
rm -rf "$WEBVIEW_DIST_DIR"
mkdir -p "$WEBVIEW_DIST_DIR"
cp -r "$GUI_DIR/dist/"* "$WEBVIEW_DIST_DIR/"

echo "=== Copying local-only Linux x86_64 refact binary for runIde bundled-probe ==="
echo "=== CI plugin packages do not bundle this binary; the local resolver probe uses it for runIde ==="
mkdir -p "$BIN_DIR/dist-x86_64-unknown-linux-gnu"
cp "$REFACT_BINARY" "$BIN_DIR/dist-x86_64-unknown-linux-gnu/refact"
chmod +x "$BIN_DIR/dist-x86_64-unknown-linux-gnu/refact"

echo "=== Done ==="
