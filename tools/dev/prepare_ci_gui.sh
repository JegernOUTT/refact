#!/usr/bin/env bash
set -euo pipefail

[[ $# -eq 1 && -f "$1/package-lock.json" ]] || {
  echo "usage: prepare_ci_gui.sh GUI_DIRECTORY" >&2
  exit 1
}

case "$(uname -m)" in
  x86_64)
    arch=x64
    digest=1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce
    ;;
  aarch64)
    arch=arm64
    digest=6287aa46b6c375d816e63ae66d024fbbfd70404825aabdfdf898a970871d2f4b
    ;;
  *)
    echo "unsupported CI GUI producer architecture" >&2
    exit 1
    ;;
esac

archive="$(mktemp)"
trap 'rm -f "$archive"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL \
  "https://nodejs.org/dist/v22.23.4/node-v22.23.4-linux-$arch.tar.xz" -o "$archive"
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
mkdir -p /opt/refact-ci-node
tar -xJf "$archive" -C /opt/refact-ci-node --strip-components=1
export PATH="/opt/refact-ci-node/bin:$PATH"
cd "$1"
npm ci --include=dev
