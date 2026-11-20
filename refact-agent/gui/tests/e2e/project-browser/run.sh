#!/bin/sh
set -eu

if [ "$#" -lt 2 ]; then
  echo 'usage: sh tests/e2e/project-browser/run.sh /absolute/path/to/refact /absolute/path/to/chrome [playwright args...]' >&2
  exit 2
fi
gui=$(cd "$(dirname "$0")/../../.." && pwd)
engine=$(realpath "$1")
chrome=$(realpath "$2")
shift 2
node=$(command -v node)
test -x "$engine" && test -x "$chrome" && test -f "$gui/node_modules/@playwright/test/cli.js"
mkdir -p "$gui/test-results"
scratch=$(mktemp -d "$gui/test-results/.project-browser.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/config" "$scratch/cache" "$scratch/tmp" "$scratch/data" "$scratch/runtime"
chmod 700 "$scratch/runtime"
host_net=$(readlink /proc/self/ns/net)
cd "$gui"
env -i PATH=/usr/bin:/bin:/usr/sbin HOME="$scratch/home" \
  XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache" \
  XDG_DATA_HOME="$scratch/data" XDG_RUNTIME_DIR="$scratch/runtime" TMPDIR="$scratch/tmp" \
  REFACT_PROJECT_BROWSER_SCRATCH="$scratch" REFACT_PROJECT_BROWSER_HOST_NET="$host_net" \
  REFACT_PROJECT_BROWSER_ENGINE="$engine" REFACT_PROJECT_BROWSER_CHROME="$chrome" \
  REFACT_PROJECT_BROWSER_BASELINE="${REFACT_PROJECT_BROWSER_BASELINE:-0}" \
  unshare --user --map-root-user --net --ipc --fork --pid --mount-proc --kill-child sh -ec '
    ip link set lo up
    ip link add fixture0 type dummy
    ip link set fixture0 addrgenmode none
    ip addr add 192.0.2.1/32 dev fixture0
    ip link set fixture0 up
    mount --bind "$TMPDIR" /tmp
    export TMPDIR=/tmp
    node=$1
    shift
    exec setpriv --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs \
      "$node" node_modules/@playwright/test/cli.js test --config tests/e2e/project-browser/playwright.config.ts "$@"
  ' sh "$node" "$@"
