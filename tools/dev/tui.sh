#!/usr/bin/env bash
# tui.sh — drive the real `refact tui` binary inside a detached tmux pane.
#
# The harness runs the TUI against an isolated daemon (temp cache/config dirs and
# a free port) backed by tests/tui_fake_worker.py, so it never touches the user's
# real ~/.cache/refact daemon or tmux server (dedicated socket `-L refact-tui`).
# When the caller is seccomp-filtered (e.g. launched from an agent exec tool), the
# tmux server is spawned through `systemd-run --user` so the TUI does not inherit
# a RET_TRACE filter that turns file writes into ENOSYS.
#
# Usage:
#   tools/dev/tui.sh start [--cols N] [--rows N] [--project DIR] [--alt-screen] [--name NAME]
#   tools/dev/tui.sh keys Enter C-q Escape PageUp S-Enter
#   tools/dev/tui.sh type "hello world"
#   tools/dev/tui.sh paste "pasted text"
#   tools/dev/tui.sh wheel up|down [N]
#   tools/dev/tui.sh screen [--history] [--ansi]
#   tools/dev/tui.sh resize COLS ROWS
#   tools/dev/tui.sh log
#   tools/dev/tui.sh status
#   tools/dev/tui.sh stop
#
# Native tmux scrolling can be exercised with:
#   tmux -L refact-tui copy-mode -t refact-tui-dev
#   tmux -L refact-tui send-keys -t refact-tui-dev -X scroll-up
#   tmux -L refact-tui send-keys -t refact-tui-dev -X cancel
set -euo pipefail

TMUX_SOCKET="refact-tui"
SESSION="${REFACT_TUI_SESSION:-refact-tui-dev}"
ROOT_DIR="/tmp/refact-tui-harness"

repo_root="$(git rev-parse --show-toplevel)"
engine_dir="$repo_root/refact-agent/engine"
binary="$engine_dir/target/debug/refact"
worker_py="$engine_dir/tests/tui_fake_worker.py"

tm() {
  tmux -L "$TMUX_SOCKET" "$@"
}

state_dir() {
  echo "$ROOT_DIR/$SESSION"
}

require_session() {
  if ! tm has-session -t "$SESSION" 2>/dev/null; then
    echo "no tmux session '$SESSION' on socket $TMUX_SOCKET; run: tools/dev/tui.sh start" >&2
    exit 1
  fi
}

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

ensure_binary() {
  if [ ! -x "$binary" ]; then
    echo "▶ building refact binary (missing at $binary)"
    (cd "$engine_dir" && REFACT_SKIP_GUI_BUILD=1 cargo build --bin refact)
  fi
}

make_project() {
  local dir="$1"
  mkdir -p "$dir/src"
  cat >"$dir/README.md" <<'EOF'
# tui-harness-project

Scratch project used by tools/dev/tui.sh.
EOF
  cat >"$dir/src/main.rs" <<'EOF'
fn main() {
    println!("refact tui harness project");
}
EOF
  cat >"$dir/Cargo.toml" <<'EOF'
[package]
name = "tui-harness-project"
version = "0.1.0"
edition = "2021"
EOF
  git -C "$dir" init -q
  git -C "$dir" add -A
  git -C "$dir" -c user.email=harness@example.com -c user.name=harness commit -qm "initial" >/dev/null 2>&1 || true
}

cmd_start() {
  local cols=120 rows=40 project="" alt=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --cols) cols="$2"; shift 2 ;;
      --rows) rows="$2"; shift 2 ;;
      --project) project="$(cd "$2" && pwd)"; shift 2 ;;
      --alt-screen) alt=1; shift ;;
      --name) SESSION="$2"; shift 2 ;;
      *) echo "unknown start option: $1" >&2; exit 2 ;;
    esac
  done

  ensure_binary
  if tm has-session -t "$SESSION" 2>/dev/null; then
    echo "session '$SESSION' already running; run: tools/dev/tui.sh stop" >&2
    exit 1
  fi

  local dir
  dir="$(state_dir)"
  rm -rf "$dir"
  mkdir -p "$dir/cache" "$dir/config" "$dir/xdg-cache"
  if [ -z "$project" ]; then
    project="$dir/project"
    make_project "$project"
  fi
  local port
  port="$(free_port)"

  cat >"$dir/env.sh" <<EOF
export REFACT_DAEMON_CACHE_DIR="$dir/cache"
export REFACT_DAEMON_CONFIG_DIR="$dir/config"
export REFACT_DAEMON_DIR="$dir/cache/daemon"
export REFACT_DAEMON_PORT="$port"
export REFACT_DAEMON_WORKER_CMD="python3 $worker_py"
export REFACT_SKIP_GUI_BUILD=1
export XDG_CACHE_HOME="$dir/xdg-cache"
export TERM=xterm-256color
export COLORTERM=truecolor
EOF
  if [ "$alt" = 1 ]; then
    echo 'export REFACT_TUI_ALT_SCREEN=1' >>"$dir/env.sh"
  fi
  echo "$port" >"$dir/port"
  echo "$project" >"$dir/project.path"

  mkdir -p "$dir/cache/daemon"
  cat >"$dir/cache/daemon/daemon.yaml" <<EOF
port: $port
bind: 127.0.0.1
idle_timeout_secs: 600
EOF

  if ! tm list-sessions >/dev/null 2>&1; then
    tm kill-server 2>/dev/null || true
  fi
  local launcher=()
  if grep -q '^Seccomp:[[:space:]]*[12]' /proc/self/status 2>/dev/null && command -v systemd-run >/dev/null; then
    launcher=(systemd-run --user --quiet --collect -p KillMode=process --)
  fi
  "${launcher[@]}" tmux -L "$TMUX_SOCKET" new-session -d -s "$SESSION" -x "$cols" -y "$rows" \
    "bash --noprofile --norc -c 'set -a; . \"$dir/env.sh\"; set +a; cd \"$project\"; \"$binary\" tui --project \"$project\" 2>\"$dir/tui.stderr\"; echo TUI_EXITED; sleep 300'"
  local deadline=$((SECONDS + 10))
  while [ $SECONDS -lt $deadline ] && ! tm has-session -t "$SESSION" 2>/dev/null; do
    sleep 0.2
  done

  local deadline=$((SECONDS + 40))
  local ready=0
  while [ $SECONDS -lt $deadline ]; do
    if tm capture-pane -p -t "$SESSION" 2>/dev/null | grep -q "Ask Refact"; then
      ready=1
      break
    fi
    sleep 0.5
  done

  echo "session:   $SESSION (tmux -L $TMUX_SOCKET)"
  echo "state dir: $dir"
  echo "project:   $project"
  echo "port:      $port"
  echo "size:      ${cols}x${rows}"
  if [ "$ready" = 1 ]; then
    echo "status:    tui visible"
  else
    echo "status:    TIMEOUT waiting for tui text; see: tools/dev/tui.sh screen / log" >&2
  fi
}

cmd_keys() {
  require_session
  [ $# -gt 0 ] || { echo "usage: tui.sh keys <key names...>" >&2; exit 2; }
  tm send-keys -t "$SESSION" "$@"
}

cmd_type() {
  require_session
  [ $# -gt 0 ] || { echo "usage: tui.sh type <text>" >&2; exit 2; }
  tm send-keys -t "$SESSION" -l -- "$*"
}

cmd_paste() {
  require_session
  [ $# -gt 0 ] || { echo "usage: tui.sh paste <text>" >&2; exit 2; }
  tm send-keys -t "$SESSION" -l -- "$(printf '\033[200~%s\033[201~' "$*")"
}

cmd_wheel() {
  require_session
  local dir_arg="${1:-up}" count="${2:-1}"
  local code
  case "$dir_arg" in
    up) code=64 ;;
    down) code=65 ;;
    *) echo "usage: tui.sh wheel up|down [N]" >&2; exit 2 ;;
  esac
  local cols rows col row
  cols="$(tm display-message -p -t "$SESSION" '#{pane_width}')"
  rows="$(tm display-message -p -t "$SESSION" '#{pane_height}')"
  col=$((cols / 2))
  row=$((rows / 2))
  local i
  for ((i = 0; i < count; i++)); do
    tm send-keys -t "$SESSION" -l -- "$(printf '\033[<%d;%d;%dM' "$code" "$col" "$row")"
    sleep 0.05
  done
}

cmd_screen() {
  require_session
  local args=(-p -t "$SESSION")
  while [ $# -gt 0 ]; do
    case "$1" in
      --history) args+=(-S -) ; shift ;;
      --ansi) args+=(-e) ; shift ;;
      *) echo "unknown screen option: $1" >&2; exit 2 ;;
    esac
  done
  local cols rows
  cols="$(tm display-message -p -t "$SESSION" '#{pane_width}')"
  rows="$(tm display-message -p -t "$SESSION" '#{pane_height}')"
  echo "--- pane $SESSION (rows=$rows cols=$cols) ---"
  tm capture-pane "${args[@]}" | awk '{printf "%02d|%s\n", NR, $0}'
  echo "--- end (rows=$rows cols=$cols) ---"
}

cmd_resize() {
  require_session
  [ $# -eq 2 ] || { echo "usage: tui.sh resize COLS ROWS" >&2; exit 2; }
  tm resize-window -t "$SESSION" -x "$1" -y "$2"
  tm resize-pane -t "$SESSION" -x "$1" -y "$2" 2>/dev/null || true
}

cmd_log() {
  local dir
  dir="$(state_dir)"
  local logs="$dir/cache/daemon/logs"
  if [ -f "$logs/daemon.log" ]; then
    echo "--- daemon.log (tail 60) ---"
    tail -n 60 "$logs/daemon.log"
  else
    echo "--- no daemon.log at $logs ---"
  fi
  local worker
  for worker in "$logs"/worker-*.log; do
    [ -f "$worker" ] || continue
    echo "--- $(basename "$worker") (tail 40) ---"
    tail -n 40 "$worker"
  done
  if [ -s "$dir/tui.stderr" ]; then
    echo "--- tui.stderr (tail 40) ---"
    tail -n 40 "$dir/tui.stderr"
  fi
}

cmd_status() {
  local dir
  dir="$(state_dir)"
  if tm has-session -t "$SESSION" 2>/dev/null; then
    echo "session:   $SESSION alive"
    echo "size:      $(tm display-message -p -t "$SESSION" '#{pane_width}x#{pane_height}')"
  else
    echo "session:   $SESSION not running"
  fi
  echo "state dir: $dir"
  if [ -f "$dir/port" ]; then
    echo "port:      $(cat "$dir/port")"
  fi
  if [ -f "$dir/cache/daemon/daemon.json" ]; then
    echo "daemon.json: $(cat "$dir/cache/daemon/daemon.json")"
  fi
}

cmd_stop() {
  local dir
  dir="$(state_dir)"
  if tm has-session -t "$SESSION" 2>/dev/null; then
    tm send-keys -t "$SESSION" C-q 2>/dev/null || true
    sleep 1
    tm kill-session -t "$SESSION" 2>/dev/null || true
  fi

  if [ -f "$dir/cache/daemon/daemon.json" ]; then
    local pid
    pid="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("pid",""))' "$dir/cache/daemon/daemon.json" 2>/dev/null || true)"
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
      sleep 0.5
      kill -9 "$pid" 2>/dev/null || true
    fi
  fi

  pkill -f "tui_fake_worker.py" 2>/dev/null || true

  if tm list-sessions 2>/dev/null | grep -q .; then
    :
  else
    tm kill-server 2>/dev/null || true
  fi

  rm -rf "$dir"
  echo "stopped $SESSION and removed $dir"
}

usage() {
  sed -n '2,22p' "$0"
}

command="${1:-}"
[ $# -gt 0 ] && shift || true
case "$command" in
  start) cmd_start "$@" ;;
  keys) cmd_keys "$@" ;;
  type) cmd_type "$@" ;;
  paste) cmd_paste "$@" ;;
  wheel) cmd_wheel "$@" ;;
  screen) cmd_screen "$@" ;;
  resize) cmd_resize "$@" ;;
  log) cmd_log "$@" ;;
  status) cmd_status "$@" ;;
  stop) cmd_stop "$@" ;;
  ""|-h|--help|help) usage ;;
  *) echo "unknown subcommand: $command" >&2; usage >&2; exit 2 ;;
esac
