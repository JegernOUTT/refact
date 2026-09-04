#!/usr/bin/env python3
import argparse
import json
import os
import signal
import socketserver
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import request
from urllib.parse import parse_qs, urlparse

MODEL_ID = "fake/tui-model"

LONG_ANSWER = """## Overview

This is a deliberately **long** markdown answer used to exercise the scrollback,
wrapping and reflow paths of the terminal UI. It contains headings, prose,
lists, a fenced code block, a table, inline `code`, *emphasis* and a
[link](https://example.com/refact-tui).

The paragraph you are reading is long on purpose so that it wraps across several
terminal columns and produces more than one physical row for a single logical
line. Wrapping behaviour is the most common source of duplicated rows in
scrollback, so the harness needs a reliable way to reproduce it.

## Motivation

Rendering a transcript in a terminal is not the same problem as rendering it in
a browser. The terminal has a fixed grid, a scrollback buffer owned by the
emulator, and a cursor whose position is shared between the application and the
emulator. Every redraw has to agree with what the emulator already believes.

There are three failure modes we care about:

- rows that are printed twice because the application re-emits a cell that has
  already scrolled out of the viewport
- rows that are lost because the application assumed the emulator scrolled when
  it did not
- rows that are wrapped differently after a resize than they were when they were
  first emitted

## Requirements

1. Every logical line must be emitted exactly once into scrollback.
2. Re-wrapping after a resize must not duplicate previously emitted rows.
3. The viewport must always end with the composer, never with a partial cell.
4. Streaming must be incremental: partial content is rendered as it arrives.
5. Aborting a stream must leave the transcript in a consistent state.

## Implementation sketch

The renderer keeps a list of history cells. Each cell owns its logical lines and
knows how many physical rows it occupied the last time it was laid out. When the
width changes, the cell is asked to re-wrap and reports a new row count. Only
cells that are still inside the viewport may be re-drawn in place; everything
above the viewport is immutable.

```rust
pub struct HistoryCell {
    id: CellId,
    lines: Vec<String>,
    rows_at_width: Option<(u16, u16)>,
}

impl HistoryCell {
    pub fn rows(&mut self, width: u16) -> u16 {
        if let Some((cached_width, rows)) = self.rows_at_width {
            if cached_width == width {
                return rows;
            }
        }
        let rows = self
            .lines
            .iter()
            .map(|line| wrapped_rows(line, width))
            .sum::<u16>()
            .max(1);
        self.rows_at_width = Some((width, rows));
        rows
    }
}
```

The cache above is the reason a resize is cheap: only the cells whose width
changed are re-measured, and the measurement is pure so it can be repeated.

## Comparison

| Approach | Scrollback | Resize cost | Duplication risk |
|---|---|---|---|
| Full repaint | lost | high | none |
| Append-only | kept | zero | high |
| Cell cache | kept | low | low |
| Emulator-owned | kept | none | medium |

The cell cache is the compromise the TUI uses: scrollback is owned by the
emulator, and the application only ever appends whole cells to it.

## Notes on testing

Testing a terminal application is easiest when the terminal is real. A headless
harness that drives a detached tmux pane gives us exactly that: a real
emulator, a real PTY, real SIGWINCH, and `capture-pane` to read the grid back
as text. Anything less fabricates the very behaviour we are trying to verify.

- `capture-pane -p` gives the viewport
- `capture-pane -p -S -` gives the viewport plus scrollback
- `capture-pane -p -e` keeps the SGR attributes
- `resize-window` delivers a genuine SIGWINCH

## Summary

If the answer above rendered without duplicated rows, wrapped cleanly at the
current width, and left the composer at the bottom of the viewport, the
rendering path is behaving. Scroll up through the scrollback to confirm that
each heading appears exactly once.
"""

CODE_ANSWER = """Here is the requested module.

```python
import json
import time
from dataclasses import dataclass, field


@dataclass
class Turn:
    role: str
    content: str
    created_at: float = field(default_factory=time.time)

    def to_json(self):
        return json.dumps(
            {
                "role": self.role,
                "content": self.content,
                "created_at": self.created_at,
            }
        )


class Transcript:
    def __init__(self, limit=100):
        self.limit = limit
        self.turns = []

    def append(self, role, content):
        self.turns.append(Turn(role, content))
        if len(self.turns) > self.limit:
            self.turns = self.turns[-self.limit :]
        return self.turns[-1]

    def last(self, role=None):
        for turn in reversed(self.turns):
            if role is None or turn.role == role:
                return turn
        return None

    def render(self):
        return "\\n".join(f"{turn.role}: {turn.content}" for turn in self.turns)
```

That block is about forty lines, which is enough to overflow a short viewport.
"""

TABLE_ANSWER = """Here is the comparison table you asked for.

| Endpoint | Method | Auth | Streaming | Latency | Notes |
|---|---|---|---|---|---|
| /v1/caps | GET | bearer | no | 5ms | model catalogue |
| /v1/chat-modes | GET | bearer | no | 4ms | mode picker source |
| /v1/chats/subscribe | GET | bearer | yes | open | primary SSE channel |
| /v1/chats/{id}/commands | POST | bearer | no | 3ms | user input path |
| /v1/trajectories | GET | bearer | no | 12ms | paginated history |
| /v1/slash-commands | GET | bearer | no | 6ms | commands and skills |
| /v1/at-command-completion | POST | bearer | no | 9ms | @ completions |
| /v1/status | GET | bearer | no | 2ms | worker health |
| /v1/knowledge-graph | GET | bearer | no | 40ms | may be large |
| /v1/build_info | GET | none | no | 1ms | version probe |

Six columns is wide enough that a narrow pane has to wrap or truncate it.
"""

SLOW_ANSWER = """Streaming this answer slowly so mid-stream frames can be captured.

Line one of the slow answer.
Line two of the slow answer.
Line three of the slow answer.
Line four of the slow answer.
Line five of the slow answer.
Line six of the slow answer.
Line seven of the slow answer.
Line eight of the slow answer.
Line nine of the slow answer.
Line ten of the slow answer.
Line eleven of the slow answer.
Line twelve of the slow answer.
Line thirteen of the slow answer.
Line fourteen of the slow answer.
Line fifteen of the slow answer.
Line sixteen of the slow answer.
Line seventeen of the slow answer.
Line eighteen of the slow answer.
Line nineteen of the slow answer.
Line twenty of the slow answer.
Line twenty-one of the slow answer.
Line twenty-two of the slow answer.
Line twenty-three of the slow answer.
Line twenty-four of the slow answer.
Line twenty-five of the slow answer.

The stream is now finished.
"""

TOOL_INTRO = "Let me look at the working directory first."

TOOL_RESULT = """total 48
drwxr-xr-x  6 dev dev  4096 Jan 01 10:00 .
drwxr-xr-x 14 dev dev  4096 Jan 01 09:58 ..
drwxr-xr-x  8 dev dev  4096 Jan 01 10:00 .git
-rw-r--r--  1 dev dev    31 Jan 01 09:59 .gitignore
-rw-r--r--  1 dev dev   187 Jan 01 09:59 Cargo.toml
-rw-r--r--  1 dev dev  1042 Jan 01 09:59 README.md
drwxr-xr-x  2 dev dev  4096 Jan 01 10:00 src
drwxr-xr-x  2 dev dev  4096 Jan 01 10:00 tests"""

TOOL_SUMMARY = """The listing shows a small Rust project: `Cargo.toml`, a `src` directory and a
`tests` directory, tracked by git. Nothing unexpected is present."""


def default_answer(prompt):
    return (
        "You said: **{prompt}**.\n\n"
        "This is the default scripted reply. It is short enough to stay inside a "
        "single screen but long enough to wrap at least once at eighty columns, "
        "which makes it useful as a baseline for the rendering checks.\n\n"
        "Send a prompt containing `long`, `code`, `table`, `tool` or `slow` to get "
        "one of the other scripted answers instead."
    ).format(prompt=prompt.strip() or "(empty)")


def script_for(prompt):
    lowered = prompt.lower()
    if "approve" in lowered:
        return "approve", None, 0.02
    if "think" in lowered:
        return "think", None, 0.02
    if "tool" in lowered:
        return "tool", None, 0.02
    if "long" in lowered:
        return "text", LONG_ANSWER, 0.02
    if "code" in lowered:
        return "text", CODE_ANSWER, 0.02
    if "table" in lowered:
        return "text", TABLE_ANSWER, 0.02
    if "slow" in lowered:
        return "text", SLOW_ANSWER, 0.15
    return "text", default_answer(prompt), 0.02


def chunk_text(text, words_per_chunk=6):
    chunks = []
    for line in text.splitlines(keepends=True):
        words = line.split(" ")
        for start in range(0, len(words), words_per_chunk):
            chunk = " ".join(words[start : start + words_per_chunk])
            if start + words_per_chunk < len(words):
                chunk += " "
            if chunk:
                chunks.append(chunk)
    return chunks


def command_text(command):
    content = command.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for part in content:
            if isinstance(part, dict) and isinstance(part.get("text"), str):
                parts.append(part["text"])
        return "".join(parts)
    return ""


class ChatState:
    def __init__(self, chat_id):
        self.chat_id = chat_id
        self.lock = threading.Lock()
        self.cond = threading.Condition(self.lock)
        self.messages = []
        self.commands = []
        self.counter = 0
        self.abort = False
        self.thread = {
            "id": chat_id,
            "title": "TUI harness chat",
            "model": MODEL_ID,
            "mode": "agent",
            "tool_use": "agent",
        }

    def next_id(self, prefix):
        with self.lock:
            self.counter += 1
            return "{}-{}".format(prefix, self.counter)

    def append_message(self, message):
        with self.lock:
            self.messages.append(message)
            return len(self.messages) - 1

    def snapshot_messages(self):
        with self.lock:
            return [dict(message) for message in self.messages]

    def push_command(self, command):
        with self.cond:
            if command.get("type") == "abort":
                self.abort = True
            self.commands.append(command)
            self.cond.notify_all()

    def take_command(self, command_type, timeout_secs):
        deadline = time.time() + timeout_secs
        with self.cond:
            while True:
                for index, command in enumerate(self.commands):
                    if command.get("type") == command_type:
                        return self.commands.pop(index)
                remaining = deadline - time.time()
                if remaining <= 0:
                    return None
                self.cond.wait(min(remaining, 0.2))

    def take_user_message(self, timeout_secs):
        deadline = time.time() + timeout_secs
        with self.cond:
            while True:
                for index, command in enumerate(self.commands):
                    if command.get("type") == "user_message":
                        self.commands.pop(index)
                        self.abort = False
                        return command
                remaining = deadline - time.time()
                if remaining <= 0:
                    return None
                self.cond.wait(min(remaining, 0.2))

    def abort_requested(self):
        with self.lock:
            return self.abort

    def clear_abort(self):
        with self.lock:
            self.abort = False


class WorkerHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urlparse(self.path)
        route = parsed.path
        if route == "/v1/ping":
            self.send_text(self.server.ping_message + "\n")
            return
        if route == "/v1/chats/subscribe":
            self.serve_chat_sse(parse_qs(parsed.query).get("chat_id", ["chat"])[0])
            return
        if route == "/v1/caps":
            self.send_json(caps_payload())
            return
        if route == "/v1/chat-modes":
            self.send_json(chat_modes_payload())
            return
        if route == "/v1/trajectories":
            self.send_json({"items": [], "next_cursor": None, "has_more": False, "total_count": 0})
            return
        if route == "/v1/slash-commands":
            self.send_json({"commands": [], "skills": []})
            return
        if route == "/v1/status":
            self.send_json(status_payload(self.server))
            return
        if route == "/v1/build_info":
            self.send_json({"version": "tui-fake-worker", "commit": "0000000"})
            return
        self.send_error_json(404, "not found")

    def do_POST(self):
        parsed = urlparse(self.path)
        route = parsed.path
        if route == "/v1/at-command-completion":
            self.read_body()
            self.send_json({"completions": [], "replace": [0, 0], "is_cmd_executable": False})
            return
        if route.startswith("/v1/chats/") and route.endswith("/commands"):
            chat_id = route[len("/v1/chats/") : -len("/commands")]
            self.handle_chat_command(chat_id)
            return
        if route == "/v1/graceful-shutdown":
            self.send_json({"success": True})
            threading.Thread(target=self.server.shutdown, daemon=True).start()
            return
        self.send_error_json(404, "not found")

    def read_body(self):
        length = int(self.headers.get("content-length", "0") or "0")
        return self.rfile.read(length) if length else b""

    def handle_chat_command(self, chat_id):
        body = self.read_body()
        try:
            command = json.loads(body.decode("utf-8")) if body else {}
        except json.JSONDecodeError:
            command = {}
        self.server.chat_state(chat_id).push_command(command)
        self.send_json({"status": "accepted"})

    def serve_chat_sse(self, chat_id):
        state = self.server.chat_state(chat_id)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "keep-alive")
        self.end_headers()
        seq = Sequencer()

        def emit(payload):
            event = {"chat_id": chat_id, "seq": str(seq.next())}
            event.update(payload)
            self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode("utf-8"))
            self.wfile.flush()

        try:
            emit(
                {
                    "type": "snapshot",
                    "thread": dict(state.thread),
                    "runtime": {
                        "state": "idle",
                        "paused": False,
                        "error": None,
                        "queue_size": 0,
                        "pause_reasons": [],
                        "queued_items": [],
                    },
                    "messages": state.snapshot_messages(),
                    "background_agents": [],
                }
            )
            while not self.server.stopping:
                command = state.take_user_message(1.0)
                if command is None:
                    continue
                self.run_turn(state, emit, command)
        except (BrokenPipeError, ConnectionResetError, OSError):
            return

    def run_turn(self, state, emit, command):
        prompt = command_text(command)
        user_message = {
            "message_id": state.next_id("user"),
            "role": "user",
            "content": prompt,
        }
        client_message_id = command.get("client_message_id")
        if isinstance(client_message_id, str) and client_message_id:
            user_message["extra"] = {"client_message_id": client_message_id}
        index = state.append_message(user_message)
        emit({"type": "message_added", "message": user_message, "index": index})
        emit({"type": "runtime_updated", "state": "generating", "error": None, "is_compressing": False})

        kind, text, delay = script_for(prompt)
        if kind == "tool":
            self.run_tool_turn(state, emit, delay)
        elif kind == "approve":
            self.run_approval_turn(state, emit, delay)
        elif kind == "think":
            self.run_thinking_turn(state, emit, delay)
        else:
            self.stream_assistant(state, emit, text, delay)
        emit({"type": "runtime_updated", "state": "idle", "error": None, "is_compressing": False})

    def run_approval_turn(self, state, emit, delay):
        reason = {
            "type": "confirmation",
            "tool_name": "shell",
            "command": "rm -rf build/",
            "rule": "dangerous command",
            "tool_call_id": "call-approve-1",
            "integr_config_path": None,
            "args": {"command": "rm -rf build/", "cwd": "/tmp/project"},
        }
        emit({"type": "pause_required", "pause_id": "pause-1", "reasons": [reason]})
        emit({"type": "runtime_updated", "state": "paused", "error": None, "is_compressing": False})
        decision = state.take_command("tool_decisions", 120)
        emit({"type": "pause_cleared"})
        accepted = bool(decision) and all(d.get("accepted") for d in decision.get("decisions", []))
        emit({"type": "runtime_updated", "state": "generating", "error": None, "is_compressing": False})
        if accepted:
            self.stream_assistant(state, emit, "Approved, so I ran `rm -rf build/` and the directory is gone.\n", delay)
        else:
            self.stream_assistant(state, emit, "Understood, I did not run the command.\n", delay)

    def run_thinking_turn(self, state, emit, delay):
        message_id = state.next_id("assistant")
        emit({"type": "stream_started", "message_id": message_id})
        reasoning = "Let me think about the layout. The composer must stay anchored, the transcript is owned by the terminal, and every cell must render exactly once.\n"
        for chunk in chunk_text(reasoning):
            emit({"type": "stream_delta", "message_id": message_id, "ops": [{"op": "append_reasoning", "text": chunk}]})
            time.sleep(delay)
        content = "After thinking it through: the rendering model is append-only and the terminal owns the scrollback.\n"
        streamed = []
        for chunk in chunk_text(content):
            streamed.append(chunk)
            emit({"type": "stream_delta", "message_id": message_id, "ops": [{"op": "append_content", "text": chunk}]})
            time.sleep(delay)
        emit({"type": "stream_finished", "message_id": message_id, "finish_reason": None})
        assistant_message = {
            "message_id": message_id,
            "role": "assistant",
            "content": "".join(streamed),
            "reasoning": reasoning,
            "stream_finished": True,
        }
        index = state.append_message(assistant_message)
        emit({"type": "message_added", "message": assistant_message, "index": index})

    def run_tool_turn(self, state, emit, delay):
        message_id = state.next_id("assistant")
        emit({"type": "stream_started", "message_id": message_id})
        for chunk in chunk_text(TOOL_INTRO):
            emit(
                {
                    "type": "stream_delta",
                    "message_id": message_id,
                    "ops": [{"op": "append_content", "text": chunk}],
                }
            )
            time.sleep(delay)
        tool_calls = [
            {
                "id": "call-1",
                "type": "function",
                "function": {"name": "shell", "arguments": "{\"command\":\"ls -la\"}"},
            }
        ]
        emit(
            {
                "type": "stream_delta",
                "message_id": message_id,
                "ops": [{"op": "set_tool_calls", "tool_calls": tool_calls}],
            }
        )
        emit({"type": "stream_finished", "message_id": message_id, "finish_reason": "tool_calls"})
        assistant_message = {
            "message_id": message_id,
            "role": "assistant",
            "content": TOOL_INTRO,
            "tool_calls": tool_calls,
            "stream_finished": True,
        }
        index = state.append_message(assistant_message)
        emit({"type": "message_added", "message": assistant_message, "index": index})

        tool_message = {
            "message_id": state.next_id("tool"),
            "role": "tool",
            "tool_call_id": "call-1",
            "content": TOOL_RESULT,
        }
        index = state.append_message(tool_message)
        emit({"type": "message_added", "message": tool_message, "index": index})
        self.stream_assistant(state, emit, TOOL_SUMMARY, delay)

    def stream_assistant(self, state, emit, text, delay):
        message_id = state.next_id("assistant")
        emit({"type": "stream_started", "message_id": message_id})
        streamed = []
        aborted = False
        for chunk in chunk_text(text):
            if state.abort_requested() or self.server.stopping:
                aborted = True
                break
            streamed.append(chunk)
            emit(
                {
                    "type": "stream_delta",
                    "message_id": message_id,
                    "ops": [{"op": "append_content", "text": chunk}],
                }
            )
            time.sleep(delay)
        content = "".join(streamed)
        if not aborted:
            emit(
                {
                    "type": "stream_delta",
                    "message_id": message_id,
                    "ops": [
                        {
                            "op": "set_usage",
                            "usage": {
                                "prompt_tokens": 64,
                                "completion_tokens": max(len(content) // 4, 1),
                                "total_tokens": 64 + max(len(content) // 4, 1),
                            },
                        }
                    ],
                }
            )
        emit(
            {
                "type": "stream_finished",
                "message_id": message_id,
                "finish_reason": "abort" if aborted else "stop",
            }
        )
        assistant_message = {
            "message_id": message_id,
            "role": "assistant",
            "content": content,
            "stream_finished": True,
        }
        index = state.append_message(assistant_message)
        emit({"type": "message_added", "message": assistant_message, "index": index})
        state.clear_abort()

    def send_json(self, payload):
        data = json.dumps(payload).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_text(self, text):
        data = text.encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_error_json(self, status, message):
        data = json.dumps({"detail": message}).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        return


class Sequencer:
    def __init__(self):
        self.value = -1
        self.lock = threading.Lock()

    def next(self):
        with self.lock:
            self.value += 1
            return self.value


def caps_payload():
    return {
        "cloud_name": "fake-tui",
        "chat_endpoint": "/v1/chat",
        "telemetry_basic_dest": "",
        "defaults": {
            "chat_default_model": MODEL_ID,
            "chat_default_mode": "agent",
            "chat_thinking_model": MODEL_ID,
            "chat_light_model": MODEL_ID,
        },
        "chat_models": {
            MODEL_ID: {
                "name": "Fake TUI Model",
                "provider": "fake",
                "n_ctx": 128000,
                "supports_tools": True,
                "supports_multimodality": False,
                "supports_agent": True,
                "supports_temperature": True,
                "supports_parallel_tools": True,
                "reasoning_effort_options": ["low", "medium", "high"],
                "pricing": {"prompt": 1.0, "generated": 3.0},
            }
        },
        "completion_models": {},
        "embedding_model": {},
        "customization": "",
        "caps_version": 1,
    }


def chat_modes_payload():
    return {
        "modes": [
            {
                "id": "agent",
                "title": "Agent",
                "description": "Full multi-step workflow with tools",
                "is_overlay": False,
                "ui": {"tags": ["default"]},
            },
            {
                "id": "explore",
                "title": "Explore",
                "description": "Read-only exploration",
                "is_overlay": False,
                "ui": {"tags": []},
            },
        ],
        "default_mode": "agent",
    }


def status_payload(server):
    return {
        "project_id": server.project_id,
        "workspace_folder": server.workspace_folder,
        "ast": {"state": "done", "files_total": 0, "ast_index_files_total": 0},
        "vecdb": {"state": "done", "files_total": 0},
        "busy_chats": 0,
        "exec_running": 0,
        "uptime_secs": int(time.time() - server.started_at),
    }


class WorkerServer(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, server_address, handler, args):
        super().__init__(server_address, handler)
        self.ping_message = args.ping_message or ""
        self.project_id = args.project_id or ""
        self.workspace_folder = args.workspace_folder or ""
        self.started_at = time.time()
        self.stopping = False
        self.chats = {}
        self.chats_lock = threading.Lock()

    def chat_state(self, chat_id):
        with self.chats_lock:
            if chat_id not in self.chats:
                self.chats[chat_id] = ChatState(chat_id)
            return self.chats[chat_id]


class LspHandler(socketserver.BaseRequestHandler):
    def handle(self):
        return


class LspServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def start_lsp_server(port):
    if not port:
        return None
    server = LspServer(("127.0.0.1", int(port)), LspHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def start_status_pusher(args):
    endpoint = (args.daemon_endpoint or "").rstrip("/")
    project_id = args.project_id or ""
    if not endpoint or not project_id:
        return
    url = endpoint + "/daemon/v1/worker-status"
    token = os.environ.get("REFACT_DAEMON_TOKEN")

    def payload():
        return {
            "project_id": project_id,
            "pid": os.getpid(),
            "instance_token": args.ping_message or "",
            "lsp_clients": 0,
            "busy_chats": 0,
            "exec_running": 0,
            "last_activity_ts": int(time.time() * 1000),
        }

    def run():
        while True:
            headers = {"content-type": "application/json"}
            if token:
                headers["Authorization"] = "Bearer " + token
            req = request.Request(
                url, data=json.dumps(payload()).encode("utf-8"), headers=headers, method="POST"
            )
            try:
                request.urlopen(req, timeout=1.0).read()
            except Exception:
                pass
            time.sleep(0.5)

    threading.Thread(target=run, daemon=True).start()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--http-port", type=int, required=True)
    parser.add_argument("--ping-message", required=True)
    parser.add_argument("--workspace-folder")
    parser.add_argument("--http-host")
    parser.add_argument("--lsp-port")
    parser.add_argument("--project-id")
    parser.add_argument("--daemon-endpoint")
    parser.add_argument("--logs-to-file")
    parser.add_argument("--ast", action="store_true")
    parser.add_argument("--ast-max-files")
    parser.add_argument("--vecdb", action="store_true")
    parser.add_argument("--vecdb-max-files")
    args, _ = parser.parse_known_args()

    lsp_server = start_lsp_server(args.lsp_port)
    server = WorkerServer(("127.0.0.1", args.http_port), WorkerHandler, args)
    start_status_pusher(args)

    def stop(_signum, _frame):
        server.stopping = True
        if lsp_server:
            threading.Thread(target=lsp_server.shutdown, daemon=True).start()
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    server.serve_forever()
    server.server_close()


if __name__ == "__main__":
    main()
