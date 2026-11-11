#!/usr/bin/env python3
"""Opt-in real-model Review runner: python tests/test_review_tool_smoke.py --help.

Drives one user turn of a Review-mode chat over the HTTP endpoints the GUI uses and
reports what the model did with the review_* tools and what the workspace holds.
Point it at a dedicated server whose workspace is a disposable repository and whose
model is already configured; never at a server with a user's chats. It posts nothing
to a pull request. Not a default CI test: it needs a model and takes minutes.
"""
import argparse
import json
import sys
import time
import urllib.error
import urllib.request
import uuid

ACTIVE = ("queued", "running", "cancel_requested")
QUIET = ("context_file", "cd_instruction", "event")


def text(message):
    content = message.get("content")
    if isinstance(content, list):
        return " ".join(str(part.get("text", "")) for part in content if isinstance(part, dict))
    return str(content or "")


class Server:
    def __init__(self, url, chat):
        self.url, self.chat = url.rstrip("/"), chat

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            self.url + path, data=data, method=method, headers={"Content-Type": "application/json"}
        )
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read() or b"null")

    def optional(self, path):
        try:
            return self.call("GET", path)
        except urllib.error.HTTPError:
            return None

    def command(self, kind, **kwargs):
        body = {"client_request_id": "smoke-" + uuid.uuid4().hex, "type": kind, **kwargs}
        return self.call("POST", f"/v1/chats/{self.chat}/commands", body)

    def snapshot(self):
        with urllib.request.urlopen(f"{self.url}/v1/chats/subscribe?chat_id={self.chat}", timeout=120) as stream:
            for line in stream:
                if line.startswith(b"data: "):
                    return json.loads(line[6:])
        return {}

    def review(self):
        return self.optional(f"/v1/reviews/by-chat/{self.chat}") or {}

    def stack(self):
        return self.optional(f"/v1/review-stacks/by-chat/{self.chat}")


def run(server, options):
    boundary = len(server.snapshot().get("messages") or [])
    if boundary == 0:
        server.command("set_params", patch={"model": options.model, "mode": "review"})
    server.command("user_message", content=options.prompt)
    started, settled, outcome, current = time.time(), 0, "timeout", {}
    while time.time() - started < options.limit:
        time.sleep(6)
        current = server.snapshot()
        runtime = current.get("runtime") or {}
        spoken = [m for m in (current.get("messages") or [])[boundary:] if m.get("role") not in QUIET]
        last = spoken[-1] if spoken else {}
        if runtime.get("state") == "error":
            outcome = "error"
            break
        busy = any(r.get("status") in ACTIVE for r in server.review().get("runs") or [])
        done = (
            runtime.get("state") == "idle"
            and not runtime.get("queue_size")
            and last.get("role") == "assistant"
            and not last.get("tool_calls")
            and text(last).strip()
            and not busy
        )
        settled = settled + 1 if done else 0
        if settled >= 3:
            outcome = "idle"
            break
    return outcome, round(time.time() - started, 1), (current.get("messages") or [])[boundary:]


def report(server, outcome, seconds, messages):
    functions, order = {}, []
    for message in messages:
        for tool_call in message.get("tool_calls") or []:
            functions[tool_call["id"]] = tool_call["function"]
            order.append(tool_call["function"].get("name"))
    results = [
        {
            "name": functions.get(m.get("tool_call_id"), {}).get("name"),
            "failed": bool(m.get("tool_failed")),
            "bytes": len(json.dumps(m)),
            "error": text(m)[:400] if m.get("tool_failed") else None,
            "args": (functions.get(m.get("tool_call_id"), {}).get("arguments") or "")[:300],
        }
        for m in messages
        if m.get("role") == "tool"
    ]
    review, stack = server.review(), server.stack()
    pages = [
        {
            "id": step["id"],
            "title": step["current"]["title"],
            "version": step["current"]["version"],
            "html_chars": len(step["current"].get("html") or ""),
            "inserts": len(step["current"].get("blocks") or []),
        }
        for step in review.get("tour") or []
    ]
    return {
        "outcome": outcome,
        "seconds": seconds,
        "tool_calls": len(results),
        "failed_calls": [r for r in results if r["failed"]],
        "first_page_at_call": order.index("review_tour") + 1 if "review_tour" in order else None,
        "call_order": order,
        "trajectory_bytes": len(json.dumps(messages)),
        "largest_tool_result_bytes": max([r["bytes"] for r in results] or [0]),
        "review": {
            "id": review.get("id"),
            "pages": pages,
            "findings": len(review.get("annotations") or []),
            "threads": len(review.get("threads") or []),
            "sources": len(review.get("evidence") or []),
            "workers": [r.get("status") for r in review.get("runs") or []],
            "remote_links": len(review.get("remote_links") or []),
        },
        "stack": None if not stack else [m.get("branch_ref") for m in stack.get("members") or []],
    }


def problems(result):
    found = []
    if result["outcome"] != "idle":
        found.append(f"the turn ended as {result['outcome']}")
    if not result["review"]["id"]:
        found.append("no review was opened")
    if not any(page["html_chars"] for page in result["review"]["pages"]):
        found.append("no tour page was published")
    for call in result["failed_calls"]:
        if (call["name"] or "").startswith("review_"):
            found.append(f"{call['name']} failed: {call['error']}")
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", required=True, help="dedicated server, for example http://127.0.0.1:8006")
    parser.add_argument("--model", required=True, help="configured chat model")
    parser.add_argument("--prompt", required=True, help="what to review: a commit range, a branch or a PR link")
    parser.add_argument("--chat", default="review-smoke-" + uuid.uuid4().hex[:8])
    parser.add_argument("--limit", type=int, default=1800, help="seconds to wait for the turn")
    parser.add_argument("--out", help="write the full JSON report here")
    options = parser.parse_args()
    server = Server(options.url, options.chat)
    result = report(server, *run(server, options))
    if options.out:
        with open(options.out, "w") as file:
            json.dump(result, file, indent=1)
    print(json.dumps({k: v for k, v in result.items() if k != "call_order"}, indent=1))
    found = problems(result)
    for problem in found:
        print("PROBLEM:", problem, file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
