#!/usr/bin/env python3
import argparse
import json
import os
import signal
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


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
