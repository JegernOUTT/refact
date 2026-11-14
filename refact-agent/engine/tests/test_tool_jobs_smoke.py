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


from owned_engine import StubProvider, Engine, HTTP, until, write_fixture, verified_copy


async def run(args):
    with tempfile.TemporaryDirectory(prefix="tool-jobs-http-smoke-") as temporary:
        root = Path(temporary)
        workspace, config, cache = root / "workspace", root / "config", root / "cache"
        binary = verified_copy(args.binary, args.binary_sha256, root / "refact-lsp")
        provider = StubProvider(workspace / "release", workspace / "provider-invocations")
        await provider.start()
        write_fixture(workspace, config, cache, provider.port)
        engine = Engine(binary, workspace, cache, config)
        chat = "tool-job-smoke-" + uuid.uuid4().hex
        try:
            engine.start()
            async with aiohttp.ClientSession(read_bufsize=2 ** 20) as session:
                http = HTTP(session, engine.port)
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
                    print(json.dumps({"failure_snapshot": await HTTP(diagnostic_session, engine.port).snapshot(chat)}), flush=True)
            except Exception as diagnostic_error:
                print(f"failure snapshot unavailable: {diagnostic_error}", flush=True)
            if getattr(engine, "log_path", None):
                print(engine.log_path.read_text(errors="replace")[-16000:], flush=True)
            raise
        finally:
            # Release/terminate only resources created by this harness. Never
            # treat a cancel request as proof that native effects are quiescent.
            (workspace / "release").touch()
            try:
                engine.stop()
            finally:
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
