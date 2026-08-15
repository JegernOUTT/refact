#!/usr/bin/env bash
set -euo pipefail

if ! command -v node >/dev/null 2>&1; then
  echo "build_injected.sh: node is required" >&2
  exit 1
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "build_injected.sh: npm is required" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INJECTED="$ROOT/refact-agent/engine/crates/refact-browser/injected"
OUTPUT="$ROOT/refact-agent/engine/crates/refact-browser/src/generated/injected_bundle.js"
TEMP="$(mktemp)"
trap 'rm -f "$TEMP"' EXIT

cd "$INJECTED"
npm ci --no-audit --no-fund
SOURCE_HASH="$(node <<'NODE'
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const files = [];
const visit = directory => {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory())
      visit(target);
    else if (entry.isFile())
      files.push(target);
  }
};
visit('src');
const hash = crypto.createHash('sha256');
for (const file of files.sort()) {
  hash.update(file.split(path.sep).join('/'));
  hash.update('\0');
  hash.update(fs.readFileSync(file));
  hash.update('\0');
}
process.stdout.write(hash.digest('hex'));
NODE
)"
npm run --silent build -- --outfile="$TEMP"
node - "$TEMP" <<'NODE'
const fs = require('fs');
const output = process.argv[2];
let content = fs.readFileSync(output, 'utf8');
let sourceStart = content.indexOf('var __toCommonJS');
if (sourceStart !== -1)
  sourceStart = content.indexOf('\n', sourceStart);
if (sourceStart === -1)
  throw new Error(`build_injected.sh: did not find the esbuild CommonJS preamble in ${output}`);
const preamble = `
var __export = (target, all) => { for (var name in all) target[name] = all[name]; };
var __toCommonJS = mod => ({ ...mod, __esModule: true });
`;
content = preamble + content.slice(sourceStart + 1);
fs.writeFileSync(output, content);
NODE
{
  printf '// @refact-injected-hash %s\n' "$SOURCE_HASH"
  cat "$TEMP"
} > "$TEMP.complete"
mv "$TEMP.complete" "$TEMP"
mkdir -p "$(dirname "$OUTPUT")"
if ! cmp -s "$TEMP" "$OUTPUT"; then
  cp "$TEMP" "$OUTPUT"
fi
