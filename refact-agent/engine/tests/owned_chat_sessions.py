"""Required owned chat protocol lane; no external worker or provider."""
import argparse
import asyncio
from contextlib import asynccontextmanager
import hashlib
import json
import os
from pathlib import Path
import tempfile
import uuid

import aiohttp
import pytest
import pytest_asyncio
from aiohttp import web
from owned_engine import MODEL, StubProvider, owned_engine, verified_copy


@pytest.fixture
def held_provider(monkeypatch):
    release = asyncio.Event()
    entered = asyncio.Event()
    entered.release = release
    original = StubProvider.completion

    async def completion(provider, request):
        payload = await request.json()
        if not payload.get("stream"):
            return await original(provider, request)
        provider.calls.append(payload)
        response = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
        await response.prepare(request)
        chunk = {"id": "owned-abort", "object": "chat.completion.chunk", "created": 1,
                 "model": MODEL, "choices": [{"index": 0, "delta": {"role": "assistant", "content": "partial-owned-output"}, "finish_reason": None}]}
        await response.write(("data: " + json.dumps(chunk) + "\n\n").encode())
        entered.set()
        await asyncio.wait_for(release.wait(), 20)
        return response

    monkeypatch.setattr(StubProvider, "completion", completion)
    try:
        yield entered
    finally:
        release.set()

TINY_PNG = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="


class Chat:
    def __init__(self, engine, session, chat_id=None):
        self.engine, self.session = engine, session
        self.id = chat_id or "owned-chat-" + uuid.uuid4().hex
        self.events = []

    async def command(self, kind, expected=202, request_id=None, **kwargs):
        request_id = request_id or uuid.uuid4().hex
        async with self.session.post(f"{self.engine.base_url}/v1/chats/{self.id}/commands", json={
            "client_request_id": request_id, "type": kind, **kwargs}) as response:
            body = await response.json()
            assert response.status == expected, (kind, response.status, body)
            return body, request_id

    async def next(self, kind, predicate=lambda event: True):
        async with asyncio.timeout(20):
            while True:
                line = await self.response.content.readline()
                assert line, ("SSE closed", self.events)
                if not line.startswith(b"data: "):
                    continue
                event = json.loads(line[6:])
                self.events.append(event)
                if event["type"] == kind and predicate(event):
                    return event

    async def configure(self, model=MODEL):
        await self.command("set_params", expected=200, patch={"model": model, "mode": "NO_TOOLS"})
        await self.next("thread_updated", lambda e: e.get("model") == model)

    async def turn(self, content="Hello", **kwargs):
        await self.command("user_message", content=content, **kwargs)
        user = (await self.next("message_added", lambda e: e["message"]["role"] == "user"))["message"]
        assert user["message_id"]
        start = await self.next("stream_started")
        assert start["message_id"]
        finish = await self.next("stream_finished", lambda e: e["message_id"] == start["message_id"])
        assert finish["finish_reason"] == "stop", self.events
        await self.next("runtime_updated", lambda e: e["state"] == "idle")
        assert any(e["type"] == "stream_delta" and e.get("message_id") == start["message_id"] for e in self.events), self.events
        return user

    @asynccontextmanager
    async def subscribe(self, path=None):
        url = path or f"/v1/chats/subscribe?chat_id={self.id}"
        async with self.session.get(self.engine.base_url + url) as response:
            assert response.status == 200
            assert response.content_type == "text/event-stream"
            self.response = response
            yield self

    async def snapshot(self):
        other = Chat(self.engine, self.session, self.id)
        async with other.subscribe():
            return await other.next("snapshot")


@pytest_asyncio.fixture
async def chat(owned_engine, request):
    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=30), read_bufsize=4 * 1024 * 1024) as session:
        chat = Chat(owned_engine, session)
        async with chat.subscribe():
            snapshot = await chat.next("snapshot")
            assert snapshot["chat_id"] == chat.id
            assert snapshot["runtime"]["state"] == "idle"
            await chat.configure()
            try:
                yield chat
            finally:
                logs = Path(__file__).resolve().parents[3] / ".refact/verification/owned-chat-sessions"
                logs.mkdir(parents=True, exist_ok=True)
                (logs / (request.node.name + ".events.json")).write_text(json.dumps(chat.events, indent=2))
                if "held_provider" in request.fixturenames:
                    request.getfixturevalue("held_provider").release.set()


CASES = {
    "basic": ["subscribe_returns_snapshot", "send_command_accepted", "duplicate_command_detected", "full_message_flow", "abort_command"],
    "abort": ["abort_while_streaming", "abort_idempotency", "abort_before_stream_starts"],
    "attachments": ["string_content_with_attachments", "multimodal_content_with_attachments", "attachments_exceed_image_limit", "attachment_missing_url", "attachment_invalid_data_url"],
    "editing": ["update_message", "remove_message", "retry_from_index", "snapshot_after_edit"],
    "errors": ["invalid_model_error", "ack_correlation_invalid_content", "ack_correlation_duplicate", "uncapped_command_burst_drains"],
    "reliability": ["invalid_multimodal_content_rejected", "missing_type_field_rejected", "empty_content_array_accepted", "valid_text_content_accepted", "too_many_images_rejected", "trajectory_subscribe_receives_events"],
}
EXPECTED = {f"test_chat_session_{suite}.py::test_{name}" for suite, names in CASES.items() for name in names}


class Inventory:
    def __init__(self):
        self.collected, self.passed, self.skipped = [], [], []

    def pytest_collection_modifyitems(self, items):
        self.collected = [item.nodeid.split("/")[-1] for item in items]
        if set(self.collected) != EXPECTED or len(self.collected) != len(EXPECTED):
            raise pytest.UsageError(f"chat inventory mismatch: {self.collected}; expected {sorted(EXPECTED)}")

    def pytest_runtest_logreport(self, report):
        if report.skipped:
            self.skipped.append(report.nodeid)
        if report.when == "call" and report.passed:
            self.passed.append(report.nodeid)

    def pytest_collectreport(self, report):
        if report.skipped:
            self.skipped.append(report.nodeid)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    args = parser.parse_args()
    tests = Path(__file__).resolve().parent
    logs = tests.parents[2] / ".refact/verification/owned-chat-sessions"
    logs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="owned-chat-run-") as directory:
        binary = verified_copy(args.binary.resolve(strict=True), args.binary_sha256, Path(directory) / "refact-lsp")
        os.environ.update(REFACT_TEST_BINARY=str(binary), REFACT_TEST_BINARY_SHA256=args.binary_sha256)
        inventory = Inventory()
        code = pytest.main(["-q", "--tb=short", "--junitxml=" + str(logs / "results.xml"), *[
            str(tests / f"test_chat_session_{suite}.py") for suite in CASES]], plugins=[inventory])
        summary = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "expected": len(EXPECTED),
                   "collected": inventory.collected, "passed": inventory.passed, "skipped": inventory.skipped, "pytest_exit": int(code)}
        (logs / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary), flush=True)
        raise SystemExit(int(code) or int(bool(inventory.skipped)) or int(len(inventory.passed) != len(EXPECTED)))


if __name__ == "__main__":
    main()
