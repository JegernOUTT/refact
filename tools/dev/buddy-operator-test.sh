#!/bin/sh
# Run already-built artifacts. No cargo, downloads, production settings or activation.
set -eu
refuse() { printf 'B02 REFUSED: %s\n' "$*" >&2; exit 78; }
[ "$#" = 2 ] || [ "$#" = 4 ] || refuse 'usage: sh tools/dev/buddy-operator-test.sh TEST_EXE REFACT_LSP [GUI_ROOT PLAYWRIGHT_BROWSER_ROOT]'
gui=${3:-}
browsers=${4:-}
if [ -n "$gui" ]; then
  gui=$(realpath "$gui")
  browsers=$(realpath "$browsers")
  [ -f "$gui/node_modules/@playwright/test/cli.js" ] || refuse 'local Playwright dependency missing'
  [ -f "$gui/node_modules/vite/bin/vite.js" ] || refuse 'local Vite dependency missing'
  [ -d "$browsers" ] || refuse 'local browser artifact root missing'
  found_browser=0
  for executable in "$browsers"/chromium-*/chrome-linux*/chrome "$browsers"/chromium_headless_shell-*/chrome-linux*/headless_shell; do
    if [ -x "$executable" ]; then found_browser=1; fi
  done
  [ "$found_browser" = 1 ] || refuse 'local Chromium artifact missing; never download or use a host browser profile'
  [ -x /usr/bin/node ] || refuse '/usr/bin/node missing'
fi
[ "$(uname -s)" = Linux ] || refuse 'Linux bubblewrap network namespace required'
command -v bwrap >/dev/null || refuse 'bubblewrap missing'
[ -x "$1" ] && [ -x "$2" ] || refuse 'both compiled artifacts must be executable'
# Reqwest's production TLS client requires roots even for loopback HTTP.
# Expose only this public trust bundle, never host configuration or private keys.
ca_bundle=/etc/ssl/certs/ca-certificates.crt
[ -r "$ca_bundle" ] || refuse "public CA bundle missing: $ca_bundle"
# Capability failure is an error, never a skipped/passed acceptance scenario.
bwrap --unshare-net --ro-bind / / -- /bin/true || refuse 'network namespace enforcement unavailable'
# Opt-in reports stay under this checkout's ignored .refact directory. The
# destination is never mounted into the worker; only scratch test evidence leaves.
report=
if [ "${B02_RETAIN_REPORT:-0}" = 1 ]; then
  checkout=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
  [ ! -L "$checkout/.refact" ] || refuse 'report root must not be a symlink'
  mkdir -p "$checkout/.refact"
  report=$(mktemp -d "$checkout/.refact/b02-report.XXXXXX")
  printf 'B02 report: %s\n' "$report"
fi
scratch=$(mktemp -d)
cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  if [ -n "$report" ]; then
    for evidence in results browser-evidence.json browser-requests.json browser-diagnostics.json restarted-1 restarted-2; do
      if [ -e "$scratch/$evidence" ]; then cp -R "$scratch/$evidence" "$report/"; fi
    done
    printf '%s\n' "$status" > "$report/exit-status"
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM
mkdir -p "$scratch/bin" "$scratch/project" "$scratch/home" "$scratch/cache" "$scratch/config"
cp "$1" "$scratch/bin/operator-test"
cp "$2" "$scratch/bin/refact-lsp"
host_net=$(readlink /proc/self/ns/net)
# A new mount root deliberately excludes /home, /run, host /tmp and host /etc.
# Only system executables/libraries are read-only; no host Unix sockets or credentials.
set -- --unshare-all --die-with-parent --new-session --cap-drop ALL --clearenv \
  --ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/sbin /sbin \
  --proc /proc --dev /dev --tmpfs /tmp --dir /etc \
  --ro-bind "$ca_bundle" /etc/ssl/certs/ca-certificates.crt \
  --setenv SSL_CERT_FILE /etc/ssl/certs/ca-certificates.crt \
  --bind "$scratch" /scratch --chdir /scratch/project \
  --setenv HOME /scratch/home --setenv XDG_CONFIG_HOME /scratch/config \
  --setenv XDG_CACHE_HOME /scratch/cache --setenv PATH /usr/bin:/bin \
  --setenv B02_HOST_NET "$host_net" --setenv B02_MODE controller \
  --setenv REFACT_DAEMON_NO_UPGRADE 1
for lib in /lib /lib64; do
  if [ -d "$lib" ]; then set -- "$@" --ro-bind "$lib" "$lib"; fi
done
if [ -n "$gui" ]; then
  mkdir -p "$scratch/gui" "$scratch/results"
  # Copy only test/source inputs; never expose the checkout, user roots or sockets.
  cp -R "$gui/src" "$gui/tests" "$scratch/gui/"
  cp "$gui/package.json" "$gui/vite.config.ts" "$gui/playwright.operator.config.ts" "$scratch/gui/"
  set -- "$@" --ro-bind "$gui/node_modules" /scratch/gui/node_modules \
    --ro-bind "$browsers" /opt/b02-browsers \
    --setenv PLAYWRIGHT_BROWSERS_PATH /opt/b02-browsers \
    --setenv B02_BROWSER 1 --setenv REFACT_LSP_URL http://127.0.0.1:39191
fi
if bwrap "$@" -- /scratch/bin/operator-test --exact buddy_operator::whole_worker_entry --nocapture --test-threads=1 >"$scratch/operator-test.log" 2>&1; then
  test_status=0
else
  test_status=$?
fi
cat "$scratch/operator-test.log"
[ "$test_status" = 0 ] || exit "$test_status"
if grep -Eq 'running 0 tests|(^|[[:space:]])0 passed([[:space:]]|$)' "$scratch/operator-test.log"; then
  printf 'B02 REFUSED: operator acceptance filter ran zero tests\n' >&2
  exit 1
fi
