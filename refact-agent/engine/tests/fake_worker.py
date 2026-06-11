#!/usr/bin/env python3
import argparse
import json
import os
import signal
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


class WorkerHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/ping":
            self.send_response(200)
            self.send_header("content-type", "text/plain")
            self.end_headers()
            self.wfile.write((self.server.ping_message + "\n").encode())
            return
        if self.path.startswith("/v1/echo"):
            self.send_echo(b"")
            return
        if self.path == "/v1/sse":
            self.send_sse()
            return
        if self.path.startswith("/v1/chats/subscribe"):
            self.send_chat_sse()
            return
        if self.path == "/v1/slow":
            time.sleep(2)
            self.send_json({"ok": True})
            return
        if self.path == "/build_info":
            self.send_json({"version": "fake-worker"})
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        if self.path == "/v1/graceful-shutdown":
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"success":true}')
            threading.Thread(target=self.server.shutdown, daemon=True).start()
            return
        if self.path.startswith("/v1/echo"):
            length = int(self.headers.get("content-length", "0") or "0")
            self.send_echo(self.rfile.read(length))
            return
        if self.path.startswith("/v1/chats/") and self.path.endswith("/commands"):
            self.handle_chat_command()
            return
        self.send_response(404)
        self.end_headers()

    def send_echo(self, body):
        headers = {key.lower(): value for key, value in self.headers.items()}
        payload = {
            "method": self.command,
            "path": self.path,
            "headers": headers,
            "body_len": len(body),
            "body_text": body.decode("utf-8", "replace") if len(body) <= 8192 else None,
        }
        self.send_json(payload)

    def send_sse(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()
        for chunk in [b"data: chunk-a\n\n", b"data: chunk-b\n\n", b"data: chunk-c\n\n"]:
            self.wfile.write(chunk)
            self.wfile.flush()
            time.sleep(0.5)

    def handle_chat_command(self):
        length = int(self.headers.get("content-length", "0") or "0")
        body = self.rfile.read(length)
        try:
            command = json.loads(body.decode("utf-8")) if body else {}
        except json.JSONDecodeError:
            self.send_response(400)
            self.end_headers()
            return
        with self.server.chat_cond:
            self.server.commands.append(command)
            self.server.chat_cond.notify_all()
        self.send_json({"status": "accepted"})

    def send_chat_sse(self):
        parsed = urlparse(self.path)
        chat_id = parse_qs(parsed.query).get("chat_id", ["chat"])[0]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()

        def emit(seq, payload):
            event = {"chat_id": chat_id, "seq": str(seq)}
            event.update(payload)
            self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
            self.wfile.flush()

        emit(0, {
            "type": "snapshot",
            "thread": {"id": chat_id, "title": "New Chat", "model": "", "mode": "agent", "tool_use": "agent"},
            "runtime": {"state": "idle", "paused": False, "error": None, "queue_size": 0, "pause_reasons": [], "queued_items": []},
            "messages": [],
            "background_agents": [],
        })
        if not self.wait_for_command("user_message", 10):
            return
        script = os.environ.get("FAKE_WORKER_CHAT_SCRIPT", "ok")
        if script == "pause":
            emit(1, {"type": "pause_required", "reasons": [self.pause_reason()]})
            emit(2, {"type": "runtime_updated", "state": "paused", "error": None, "is_compressing": False})
            decision = self.wait_for_decision(10)
            if decision:
                accepted = all(d.get("accepted") for d in decision.get("decisions", []))
                if accepted:
                    emit(3, {"type": "pause_cleared"})
                    self.emit_chat_answer(emit, 4, "approved path")
                else:
                    emit(3, {"type": "pause_cleared"})
                    emit(4, {"type": "runtime_updated", "state": "idle", "error": None, "is_compressing": False})
            return
        self.emit_chat_answer(emit, 1, "hello world")

    def emit_chat_answer(self, emit, seq, text):
        emit(seq, {"type": "stream_started", "message_id": "assistant-1"})
        emit(seq + 1, {"type": "stream_delta", "message_id": "assistant-1", "ops": [
            {"op": "set_tool_calls", "tool_calls": [{"function": {"name": "fake_tool", "arguments": "{\"x\":1}"}}]},
            {"op": "append_content", "text": text[:5]},
        ]})
        emit(seq + 2, {"type": "stream_delta", "message_id": "assistant-1", "ops": [
            {"op": "append_content", "text": text[5:]},
            {"op": "set_usage", "usage": {"prompt_tokens": 1, "completion_tokens": 2}},
        ]})
        emit(seq + 3, {"type": "stream_finished", "message_id": "assistant-1", "finish_reason": None})
        emit(seq + 4, {"type": "runtime_updated", "state": "idle", "error": None, "is_compressing": False})

    def wait_for_command(self, command_type, timeout_secs):
        deadline = time.time() + timeout_secs
        with self.server.chat_cond:
            while time.time() < deadline:
                if any(command.get("type") == command_type for command in self.server.commands):
                    return True
                self.server.chat_cond.wait(0.1)
        return False

    def wait_for_decision(self, timeout_secs):
        deadline = time.time() + timeout_secs
        with self.server.chat_cond:
            while time.time() < deadline:
                for command in self.server.commands:
                    if command.get("type") == "tool_decisions":
                        return command
                self.server.chat_cond.wait(0.1)
        return None

    def pause_reason(self):
        return {
            "type": "confirmation",
            "tool_name": "fake_tool",
            "command": "fake_tool({\"x\":1})",
            "rule": "fake confirmation",
            "tool_call_id": "fake-call-1",
            "integr_config_path": None,
        }

    def send_json(self, payload):
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.send_header("x-hop-test", "visible")
        self.send_header("connection", "x-hidden")
        self.send_header("x-hidden", "hidden")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        return


class WorkerServer(ThreadingHTTPServer):
    allow_reuse_address = True

    def __init__(self, server_address, handler, ping_message):
        super().__init__(server_address, handler)
        self.ping_message = ping_message
        self.commands = []
        self.chat_cond = threading.Condition()


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

    if os.environ.get("FAKE_WORKER_CRASH") == "1":
        sys.exit(1)

    server = WorkerServer(("127.0.0.1", args.http_port), WorkerHandler, args.ping_message)

    def stop(_signum, _frame):
        server.shutdown()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    server.serve_forever()
    server.server_close()


if __name__ == "__main__":
    main()
