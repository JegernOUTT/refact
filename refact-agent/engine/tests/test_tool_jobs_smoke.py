#!/usr/bin/env python3
"""Opt-in real HTTP tool-job smoke using an isolated local OpenAI stub.

The fixture provider is deterministic test infrastructure, not an LLM evaluation.
Run only with an explicitly supplied freshly compiled binary; never reuses an
existing engine, provider, workspace, cache, or credentials. The native command
runs once, blocks on a local file barrier, and is released only after HTTP proves
that the owning chat accepted another user/model turn while the job is pending.
"""
import argparse
import asyncio
import json
import hashlib
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid
import unittest
from unittest.mock import patch

from aiohttp import web
import aiohttp

ENGINE_PORT = 8017
PROVIDER_PORT = 8018
MODEL = "localstub/foreground-smoke"
MARKER = "TOOL_JOB_SMOKE_NATIVE_RESULT"
CMDLINE_NAME = "cmdline_smoke"
SECOND_PROMPT = "Second human turn while native job is pending."
PHASE_TIMEOUT_SECONDS = 20
RECEIPT_TIMEOUT_SECONDS = 75
BARRIER_TIMEOUT_SECONDS = 120
COMMAND_TIMEOUT_SECONDS = 150


class StubProvider:
    """OpenAI-compatible deterministic tool-calling provider, local only."""
    def __init__(self, barrier: Path, counter: Path):
        self.barrier, self.counter = barrier, counter
        self.calls = []
        self.emitted = 0
        self.second_seen_pending = False
        self.app = web.Application()
        self.app.router.add_get("/v1/models", self.models)
        self.app.router.add_post("/v1/chat/completions", self.completion)
        self.runner = None

    async def models(self, _request):
        return web.json_response({"object": "list", "data": [{"id": MODEL, "object": "model"}]})

    async def completion(self, request):
        payload = await request.json()
        self.calls.append(payload)
        messages = payload.get("messages", [])
        tool_specs = {item.get("function", {}).get("name"): item.get("function", {})
                      for item in payload.get("tools", [])}
        if CMDLINE_NAME not in tool_specs:
            raise web.HTTPBadRequest(text=f"expected configured {CMDLINE_NAME} tool")
        # First turn: emit exactly one invocation, then block the actual tool
        # process while the server's foreground generation returns its receipt.
        if self.emitted == 0:
            self.emitted += 1
            # The cmdline integration's schema consists of the declared command
            # arguments directly; this fixture declares none.
            args = {}
            call = {"id": "call-smoke-once", "type": "function", "function": {
                "name": CMDLINE_NAME, "arguments": json.dumps(args)}}
            response = {"id": "stub-first", "object": "chat.completion", "created": 1,
                "model": MODEL, "choices": [{"index": 0, "finish_reason": "tool_calls",
                    "message": {"role": "assistant", "content": None, "tool_calls": [call]}}]}
        else:
            # User's second turn is served immediately while the first job is
            # blocked; after release the tool result prompts the owning chat again.
            saw_native = any(item.get("role") == "tool" and MARKER in str(item.get("content"))
                             for item in messages)
            saw_second = any(item.get("role") == "user" and SECOND_PROMPT in str(item.get("content", ""))
                             for item in messages)
            if saw_second and not self.barrier.exists():
                self.second_seen_pending = True
            text = ("post-tool-model-continuation" if saw_native else
                    "second-turn-accepted" if saw_second else "first-turn-receipt-acknowledged")
            response = {"id": "stub-followup", "object": "chat.completion", "created": 2,
                "model": MODEL, "choices": [{"index": 0, "finish_reason": "stop",
                    "message": {"role": "assistant", "content": text}}]}
        if payload.get("stream"):
            choice = response["choices"][0]
            delta = dict(choice["message"])
            for index, call in enumerate(delta.get("tool_calls", [])):
                call["index"] = index
            chunk = {**response, "object": "chat.completion.chunk", "choices": [
                {"index": 0, "delta": delta, "finish_reason": None}]}
            end = {**response, "object": "chat.completion.chunk", "choices": [
                {"index": 0, "delta": {}, "finish_reason": choice["finish_reason"]}]}
            return web.Response(content_type="text/event-stream", text=
                f"data: {json.dumps(chunk)}\n\ndata: {json.dumps(end)}\n\ndata: [DONE]\n\n")
        return web.json_response(response)

    async def start(self):
        self.runner = web.AppRunner(self.app)
        await self.runner.setup()
        site = web.TCPSite(self.runner, "127.0.0.1", PROVIDER_PORT)
        await site.start()

    async def stop(self):
        if self.runner:
            await self.runner.cleanup()


class Engine:
    def __init__(self, binary: Path, workspace: Path, cache: Path, config: Path):
        self.binary, self.workspace, self.cache, self.config = binary, workspace, cache, config
        self.process = None

    def start(self):
        with socket.socket() as sock:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind(("127.0.0.1", ENGINE_PORT))
        env = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL", "TMPDIR")
               if key in os.environ}
        # Isolate all user config/cache/environment state; configuration written
        # below points only to this process's loopback provider.
        env.update({"HOME": str(self.config), "XDG_CONFIG_HOME": str(self.config),
                    "XDG_CACHE_HOME": str(self.cache), "REFACT_CACHE_DIR": str(self.cache),
                    "REFACT_CONFIG_DIR": str(self.config),
                    "HTTP_PROXY": "http://127.0.0.1:9", "HTTPS_PROXY": "http://127.0.0.1:9",
                    "ALL_PROXY": "http://127.0.0.1:9", "NO_PROXY": "127.0.0.1,localhost"})
        self.log_path = self.workspace.parent / "engine.log"
        self.log = self.log_path.open("wb")
        self.process = subprocess.Popen([str(self.binary), "--http-port", str(ENGINE_PORT),
            "--workspace-folder", str(self.workspace), "--no-scheduler", "--logs-stderr"],
            cwd=self.workspace, env=env, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise AssertionError("owned engine exited before readiness")
            try:
                with socket.create_connection(("127.0.0.1", ENGINE_PORT), timeout=.25):
                    return
            except OSError:
                time.sleep(.1)
        raise AssertionError("owned engine readiness timeout")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


class HTTP:
    def __init__(self, session):
        self.session = session
        self.base = f"http://127.0.0.1:{ENGINE_PORT}/v1"

    async def request(self, method, path, body=None):
        async with self.session.request(method, self.base + path, json=body,
                timeout=aiohttp.ClientTimeout(total=20)) as response:
            text = await response.text()
            data = text if path == "/ping" else json.loads(text)
            assert response.status < 300, f"{method} {path}: HTTP {response.status}: {data}"
            return data

    async def command(self, chat, kind, **kwargs):
        return await self.request("POST", f"/chats/{chat}/commands", {
            "client_request_id": "tool-jobs-smoke-" + uuid.uuid4().hex,
            "type": kind, **kwargs})

    async def snapshot(self, chat):
        async with self.session.get(self.base + "/chats/subscribe", params={"chat_id": chat},
                timeout=aiohttp.ClientTimeout(total=15)) as response:
            assert response.status == 200
            while True:
                line = await response.content.readline()
                assert line, "SSE ended without snapshot"
                if line.startswith(b"data: "):
                    event = json.loads(line[6:])
                    if event.get("type") == "snapshot":
                        return event


async def until(label, callback, seconds=PHASE_TIMEOUT_SECONDS):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = await callback()
        if result:
            return result
        await asyncio.sleep(.1)
    raise AssertionError(label + ": finite timeout")


def write_fixture(workspace: Path, config: Path, cache: Path, provider_port: int):
    workspace.mkdir(parents=True)
    (workspace / ".refact" / "integrations.d").mkdir(parents=True)
    (workspace / ".refact" / "integrations.d" / "cmdline_smoke.yaml").write_text(
        "command: python3 -u barrier_tool.py\ncommand_workdir: "
        + str(workspace) + "\ndescription: Run the deterministic foreground smoke barrier tool\n"
        f"parameters: []\nparameters_required: []\ntimeout: '{COMMAND_TIMEOUT_SECONDS}'\n"
        "available: {on_your_laptop: true, when_isolated: true}\n"
        "confirmation: {ask_user: [], deny: []}\n")
    (workspace / "barrier_tool.py").write_text(
        "import json, pathlib, sys, time\n"
        f"base = pathlib.Path({str(workspace)!r})\n"
        "(base / 'invoked').open('a').write('1\\n')\n"
        "(base / 'started').write_text('ready')\n"
        f"deadline = time.monotonic() + {BARRIER_TIMEOUT_SECONDS}\n"
        "while not (base / 'release').exists() and time.monotonic() < deadline: time.sleep(.025)\n"
        "if not (base / 'release').exists(): raise SystemExit('barrier timed out')\n"
        f"print({MARKER!r}, flush=True)\n")
    provider_dir = config / ".config" / "refact" / "providers.d"
    provider_dir.mkdir(parents=True)
    # Real Rust tokenizers loader, synthetic vocabulary: this tests orchestration,
    # not LLM quality. Never consult or mutate the user's tokenizer cache/network.
    tokenizer = config / "smoke-tokenizer.json"
    tokenizer.write_text(json.dumps({
        "version": "1.0", "truncation": None, "padding": None,
        "added_tokens": [], "normalizer": None,
        "pre_tokenizer": {"type": "Whitespace"}, "post_processor": None,
        "decoder": None,
        "model": {"type": "WordLevel", "vocab": {"[UNK]": 0}, "unk_token": "[UNK]"},
    }))
    (provider_dir / "localstub.yaml").write_text(
        "base_provider: custom\ndisplay_name: Local stub smoke\nenabled: true\napi_key: local-only\n"
        f"chat_endpoint: http://127.0.0.1:{provider_port}/v1/chat/completions\n"
        "wire_format: openai_chat_completions\nenabled_models: [foreground-smoke]\n"
        "custom_models:\n  foreground-smoke:\n    n_ctx: 32768\n    supports_tools: true\n"
        f"    tokenizer: {tokenizer.as_uri()}\n    max_output_tokens: 2048\n")
    (config / ".config" / "refact" / "privacy.yaml").write_text(
        "privacy_rules:\n  zones:\n    - name: local-smoke-only\n      patterns: ['*']\n"
        "      send_to: [localstub]\n      on_shell_read: withhold\n")
    # Project defaults keep the test model explicit and away from any provider
    # discovery/fallback behavior.
    config.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True)


async def run(args):
    with tempfile.TemporaryDirectory(prefix="tool-jobs-http-smoke-") as temporary:
        root = Path(temporary)
        workspace, config, cache = root / "workspace", root / "config", root / "cache"
        write_fixture(workspace, config, cache, PROVIDER_PORT)
        provider = StubProvider(workspace / "release", workspace / "provider-invocations")
        await provider.start()
        engine = Engine(args.binary.resolve(strict=True), workspace, cache, config)
        chat = "tool-job-smoke-" + uuid.uuid4().hex
        try:
            engine.start()
            async with aiohttp.ClientSession(read_bufsize=2 ** 20) as session:
                http = HTTP(session)
                await http.request("GET", "/ping")
                await http.request("POST", "/lsp-initialize", {"project_roots": [workspace.as_uri()]})
                await http.snapshot(chat)
                params = await http.command(chat, "set_params", patch={"model": MODEL, "mode": "agent", "auto_approve_dangerous_commands": True})
                assert params.get("status") == "applied", params
                await http.command(chat, "user_message", content="Use cmdline_smoke exactly once and stop.")
                await until("native command barrier entered", lambda: asyncio.to_thread((workspace / "started").exists))
                before = await wait_for_receipt(http, chat)
                messages = before.get("messages", [])
                receipt = next((m for m in messages if m.get("role") == "tool" and
                    m.get("tool_job")), None)
                assert receipt, "missing accepted tool-job receipt"
                meta = receipt["tool_job"]
                job_id = meta["job_id"]
                assert meta.get("status") == "accepted", "receipt must not imply execution success"
                assert receipt.get("tool_failed") is None
                pending = await http.request("GET", f"/chats/{chat}/tool-jobs/{job_id}")
                assert pending.get("result") is None, "barrier job unexpectedly completed"
                assert pending.get("receipt", {}).get("job_id", job_id) == job_id
                # Genuine new human command during unfinished job demonstrates the
                # owning session remains responsive, without background-control tools.
                accepted = await http.command(chat, "user_message", content=SECOND_PROMPT)
                assert accepted.get("status") == "accepted", accepted
                second = await until("second owning-chat model turn", lambda: _second_turn(http, chat))
                assert second and provider.second_seen_pending, "new human turn not served before release"
                assert not (workspace / "release").exists()
                assert (await http.request("GET", f"/chats/{chat}/tool-jobs/{job_id}"))["result"] is None
                invocations = (workspace / "invoked").read_text().splitlines()
                assert invocations == ["1"], f"expected one native invocation before release: {invocations}"
                (workspace / "release").write_text("release\n")
                terminal = await until("job terminal status", lambda: _terminal(http, chat, job_id))
                assert terminal["status"] == "succeeded", terminal
                result = await http.request("GET", f"/chats/{chat}/tool-jobs/{job_id}")
                assert result["status"] == "succeeded" and result.get("result")
                native = result["result"].get("messages", [])
                assert len(native) == 1 and MARKER in text(native[0]), native
                assert native[0].get("tool_call_id") == "call-smoke-once"
                assert native[0].get("tool_failed") is False
                assert native[0].get("exec", {}).get("exit_code") == 0
                assert (await http.request("GET", f"/chats/{chat}/tool-jobs/{job_id}"))["result"] == result["result"]
                after = await until("one completion notice", lambda: _completion_snapshot(http, chat, job_id))
                notices = completion_notices(after, job_id)
                assert len(notices) == 1, f"expected one completion notice, got {len(notices)}"
                notice = notices[0]
                assert notice.get("role") == "event", notice
                assert notice["tool_job_result"]["actor"] == "tool_completion"
                assert notice["tool_job_result"]["tool_call_id"] == "call-smoke-once"
                assert notice["event"]["source"] == "tool_job.completed"
                assert text(notice) == text(native[0]), "actor completion changed native output"
                stable_id = f"tool_job:{job_id}:{terminal['revision']}:0"
                assert notice.get("message_id") == stable_id, notice
                # Re-read across bridge reconciliation, not just its first append.
                await asyncio.sleep(2.2)
                repeated = completion_notices(await http.snapshot(chat), job_id)
                assert len(repeated) == 1 and repeated[0].get("message_id") == stable_id
                assert provider.emitted == 1
                assert (workspace / "invoked").read_text().splitlines() == ["1"]
                print(json.dumps({"pass": True, "chat_id": chat, "job_id": job_id,
                    "provider_kind": "deterministic local stub; not actual LLM",
                    "pending_receipt": "accepted, not success", "second_turn_before_release": True,
                    "native_invocation_count": 1, "terminal_status": terminal["status"],
                    "completion_notice_count": len(notices), "completion_message_id": stable_id,
                    "exact_native_result_retrieved": True}, indent=2))
        except Exception:
            print(json.dumps({"fixture_provider_calls": provider.calls}, default=str), flush=True)
            try:
                async with aiohttp.ClientSession(read_bufsize=2 ** 20) as diagnostic_session:
                    print(json.dumps({"failure_snapshot": await HTTP(diagnostic_session).snapshot(chat)}), flush=True)
            except Exception as diagnostic_error:
                print(f"failure snapshot unavailable: {diagnostic_error}", flush=True)
            if getattr(engine, "log_path", None):
                print(engine.log_path.read_text(errors="replace")[-16000:], flush=True)
            raise
        finally:
            # Release/terminate only resources created by this harness. Never
            # treat a cancel request as proof that native effects are quiescent.
            (workspace / "release").touch()
            engine.stop()
            await provider.stop()


def text(message):
    content = message.get("content", "")
    return content if isinstance(content, str) else json.dumps(content)


async def wait_for_receipt(http, chat):
    return await until("accepted tool-job receipt", lambda: _receipt_snapshot(http, chat),
                       seconds=RECEIPT_TIMEOUT_SECONDS)


async def _receipt_snapshot(http, chat):
    snap = await http.snapshot(chat)
    return snap if any(m.get("role") == "tool" and
        m.get("tool_job") for m in snap.get("messages", [])) else None


async def _completion_snapshot(http, chat, job_id):
    snap = await http.snapshot(chat)
    notices = completion_notices(snap, job_id)
    assert len(notices) <= 1, "duplicate completion notices"
    return snap if notices else None


def completion_notices(snapshot, job_id):
    return [m for m in snapshot.get("messages", []) if
            m.get("tool_job_result", {}).get("job_id") == job_id]


async def _second_turn(http, chat):
    snap = await http.snapshot(chat)
    messages = snap.get("messages", [])
    positions = [i for i, m in enumerate(messages) if m.get("role") == "user" and text(m) == SECOND_PROMPT]
    if not positions:
        return None
    return snap if any(m.get("role") == "assistant" and text(m) == "second-turn-accepted"
                       for m in messages[positions[-1] + 1:]) else None


async def _terminal(http, chat, job_id):
    snapshot = await http.request("GET", f"/chats/{chat}/tool-jobs/{job_id}")
    return snapshot if snapshot.get("result") is not None else None


class FixtureDeadlineTests(unittest.IsolatedAsyncioTestCase):
    async def test_receipt_after_inline_budget_and_finite_timeout(self):
        now = 0.0

        async def advance(seconds):
            nonlocal now
            now += seconds

        expected = {"messages": [{"role": "tool", "tool_job": {"status": "accepted"}}]}

        class ReceiptHTTP:
            async def snapshot(self, _chat):
                return expected if now >= 65 else {"messages": []}

        async def missing():
            return None

        with patch.object(time, "monotonic", side_effect=lambda: now), \
                patch.object(asyncio, "sleep", side_effect=advance):
            self.assertEqual(await wait_for_receipt(ReceiptHTTP(), "fixture-chat"), expected)
            with self.assertRaisesRegex(AssertionError, "missing: finite timeout"):
                await until("missing", missing, seconds=RECEIPT_TIMEOUT_SECONDS)

        self.assertGreater(BARRIER_TIMEOUT_SECONDS,
                           RECEIPT_TIMEOUT_SECONDS + PHASE_TIMEOUT_SECONDS + 20)
        self.assertGreater(COMMAND_TIMEOUT_SECONDS, BARRIER_TIMEOUT_SECONDS)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workspace = root / "workspace"
            write_fixture(workspace, root / "config", root / "cache", PROVIDER_PORT)
            command = (workspace / ".refact/integrations.d/cmdline_smoke.yaml").read_text()
            self.assertIn(f"timeout: '{COMMAND_TIMEOUT_SECONDS}'", command)
            barrier = (workspace / "barrier_tool.py").read_text()
            self.assertIn(f"deadline = time.monotonic() + {BARRIER_TIMEOUT_SECONDS}", barrier)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True,
                        help="explicit freshly compiled refact-lsp binary (not an existing server)")
    parser.add_argument("--binary-sha256", required=True, help="parent-authorized fresh binary SHA256")
    args = parser.parse_args()
    digest = hashlib.sha256(args.binary.resolve(strict=True).read_bytes()).hexdigest()
    assert digest == args.binary_sha256, "binary differs from explicitly authorized build"
    print(json.dumps({"binary_sha256": digest}), flush=True)
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
