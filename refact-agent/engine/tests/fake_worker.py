#!/usr/bin/env python3
import argparse
import os
import signal
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class WorkerHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/ping":
            self.send_response(200)
            self.send_header("content-type", "text/plain")
            self.end_headers()
            self.wfile.write((self.server.ping_message + "\n").encode())
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
        self.send_response(404)
        self.end_headers()

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
