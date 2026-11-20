"""Live ownership regression; run with an explicitly verified Linux worker:

REFACT_TEST_BINARY=/absolute/path/to/refact-lsp REFACT_TEST_BINARY_SHA256=<sha256> \
    python -m pytest -q refact-agent/engine/tests/test_planner_board_ownership.py

Requires Python 3.11+, git, aiohttp, and pytest-asyncio. No running daemon or credentials needed.
"""

from contextlib import asynccontextmanager
import json
import os
from pathlib import Path
import subprocess

import aiohttp
from aiohttp import web
import pytest

from owned_chat_sessions import Chat
from owned_engine import Engine, HTTP, MODEL, StubProvider, verified_copy, write_fixture


class PlannerProvider(StubProvider):
    """Emit one requested native tool call; the worker decides whether it succeeds."""

    def __init__(self, workspace: Path):
        super().__init__(workspace / "release", workspace / "invoked", tool_call=False)
        self.turn = None

    async def completion(self, request):
        payload = await request.json()
        call_id, prompt, name, args = self.turn or (None, "", None, None)
        requested = prompt and any(
            prompt in str(message["content"]) for message in payload["messages"]
            if message["role"] == "user"
        )
        advertised = {tool["function"]["name"] for tool in payload.get("tools", [])}
        if requested and name in advertised:
            self.turn = None
            self.calls.append(call_id)
            message = {"role": "assistant", "content": None, "tool_calls": [{
                "id": call_id, "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)},
            }]}
            finish = "tool_calls"
        else:
            message = {"role": "assistant", "content": "Ownership test turn finished."}
            finish = "stop"

        response = {"id": "planner-ownership", "model": MODEL, "created": 1}
        if not payload.get("stream"):
            return web.json_response({**response, "object": "chat.completion", "choices": [
                {"index": 0, "message": message, "finish_reason": finish},
            ]})
        for index, call in enumerate(message.get("tool_calls", [])):
            call["index"] = index
        chunk = {**response, "object": "chat.completion.chunk"}
        chunks = [{**chunk, "choices": [{"index": 0, "delta": delta, "finish_reason": reason}]}
                  for delta, reason in [(message, None), ({}, finish)]]
        return web.Response(content_type="text/event-stream", text=(
            "".join(f"data: {json.dumps(item)}\n\n" for item in chunks) + "data: [DONE]\n\n"
        ))


def init_git(workspace: Path, home: Path) -> str:
    env = {"PATH": os.environ["PATH"], "HOME": str(home), "XDG_CONFIG_HOME": str(home),
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}

    def git(*args):
        return subprocess.run(["git", *args], cwd=workspace, env=env, check=True,
                              capture_output=True, text=True, timeout=10).stdout.strip()

    git("init", "--template=")
    (workspace / "README.md").write_text("Disposable planner ownership test\n")
    git("add", "README.md")
    git("-c", "user.name=Ownership Test", "-c", "user.email=test@example.invalid",
        "commit", "-m", "Initialize dummy project")
    return git("rev-parse", "HEAD")


@asynccontextmanager
async def running_engine(binary, workspace, config, cache, phase):
    engine = Engine(binary, workspace, cache, config)
    engine.base_url = f"http://127.0.0.1:{engine.port}"
    try:
        engine.start()
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=30),
                                         read_bufsize=4 * 1024 * 1024) as session:
            http = HTTP(session, engine.port)
            await http.request("GET", "/ping")
            await http.request("POST", "/lsp-initialize", {"project_roots": [workspace.as_uri()]})
            yield engine, http
    finally:
        engine.stop()
        if hasattr(engine, "log_path") and engine.log_path.exists():
            engine.log_path.rename(engine.log_path.with_name(f"engine-{phase}.log"))


async def open_bound_chat(engine, http, chat_id, task_id, mode, role):
    chat = Chat(engine, http.session, chat_id)
    snapshot = await chat.snapshot()
    assert snapshot["chat_id"] == chat_id
    thread = snapshot["thread"]
    assert thread["mode"] == mode, thread
    task_meta = thread["task_meta"]
    assert (task_meta["task_id"], task_meta["role"]) == (task_id, role), task_meta
    if role == "planner" and thread["model"] != MODEL:
        await chat.command("set_params", expected=200, patch={"model": MODEL})
    return chat


async def native_call(chat, provider, label, name, **args):
    call_id = f"call-{label}"
    prompt = f"Ownership regression: {label}"
    provider.turn = (call_id, prompt, name, args)
    async with chat.subscribe():
        await chat.next("snapshot")
        await chat.command("user_message", content=prompt)
        event = await chat.next("message_added", lambda e: (
            e["message"]["role"] == "tool" and e["message"].get("tool_call_id") == call_id
        ))
        await chat.next("runtime_updated", lambda e: e["state"] == "idle")
    assert provider.calls[-1:] == [call_id], provider.calls
    return event["message"]


def assert_owner(board, chat_id, epoch):
    assert board_owner(board) == (chat_id, epoch)


def board_owner(board):
    return board.get("owner_chat_id"), board.get("owner_epoch")


def card_title(board):
    (card,) = board["cards"]
    assert card["id"] == "T-1", card
    return card["title"]


async def update_card(chat, provider, label, title, *, accepted):
    result = await native_call(chat, provider, label, "board_update", card_id="T-1", title=title)
    if accepted:
        assert not result.get("tool_failed") and result["content"] == "Updated card T-1", result
    else:
        assert result["tool_failed"] is True, result
        assert "task board mutation requires the owning planner chat" in result["content"], result


@pytest.mark.asyncio
async def test_planner_board_ownership_survives_cold_restart(tmp_path):
    source = Path(os.environ["REFACT_TEST_BINARY"]).resolve(strict=True)
    workspace, config, cache = (tmp_path / name for name in ("workspace", "config", "cache"))
    provider = PlannerProvider(workspace)
    binary = verified_copy(source, os.environ["REFACT_TEST_BINARY_SHA256"], tmp_path / "refact-lsp")
    try:
        await provider.start()
        write_fixture(workspace, config, cache, provider.port)
        (workspace / ".refact/integrations.d/cmdline_smoke.yaml").unlink()
        (workspace / "barrier_tool.py").unlink()
        head = init_git(workspace, config)

        async with running_engine(binary, workspace, config, cache, "before") as (first, http):
            task = await http.request("POST", "/tasks", {"name": "Cold ownership regression"})
            assert task["base_commit"] == head, task
            task_id = task["id"]
            board_path = f"/tasks/{task_id}/board"
            create_chat = f"/tasks/{task_id}/planner-chats"
            a = (await http.request("POST", create_chat, {"mode": "task_planner"}))["chat_id"]
            initial = await http.request("GET", board_path)
            epoch = initial["owner_epoch"]
            assert epoch > 0
            assert_owner(initial, a, epoch)

            planner_a = await open_bound_chat(first, http, a, task_id, "task_planner", "planner")
            result = await native_call(planner_a, provider, "seed-card", "board_create",
                                       card_id="T-1", title="Original title")
            assert not result.get("tool_failed"), result
            seeded = await http.request("GET", board_path)
            assert card_title(seeded) == "Original title"

            linked = (await http.request("POST", create_chat, {"mode": "agent"}))["chat_id"]
            after_link = await http.request("GET", board_path)
            b = (await http.request("POST", create_chat, {"mode": "task_planner"}))["chat_id"]
            assert len({a, b, linked}) == 3
            before_restart = await http.request("GET", board_path)
            assert before_restart["cards"] == seeded["cards"]

        assert first.process.poll() == 0, "first engine did not stop cleanly"
        async with running_engine(binary, workspace, config, cache, "after") as (second, http):
            assert second.process.pid != first.process.pid
            restored = await http.request("GET", board_path)
            observed = {
                "linked chat created": board_owner(after_link),
                "+Planner B created": board_owner(before_restart),
                "cold restart": board_owner(restored),
            }
            assert observed == dict.fromkeys(observed, (a, epoch))
            assert restored == before_restart
            planner_a = await open_bound_chat(second, http, a, task_id, "task_planner", "planner")
            planner_b = await open_bound_chat(second, http, b, task_id, "task_planner", "planner")
            await open_bound_chat(second, http, linked, task_id, "agent", "linked")
            assert await http.request("GET", board_path) == before_restart

            await update_card(planner_a, provider, "a-after-restart", "Written by A", accepted=True)
            written_a = await http.request("GET", board_path)
            assert_owner(written_a, a, epoch)
            assert written_a["rev"] == before_restart["rev"] + 1
            assert card_title(written_a) == "Written by A"
            await update_card(planner_b, provider, "b-before-handoff", "Rejected B write", accepted=False)
            assert await http.request("GET", board_path) == written_a

            result = await native_call(planner_a, provider, "transfer-to-b", "task_board_authority",
                                       action="transfer_owner", target_chat_id=b)
            assert not result.get("tool_failed"), result
            receipt = json.loads(result["content"])
            assert receipt["action"] == "transfer_owner" and receipt["changed"] is True, receipt
            assert (receipt["task_id"], receipt["target_chat_id"], receipt["owner_chat_id"]) == (task_id, b, b)
            transferred = await http.request("GET", board_path)
            assert_owner(transferred, b, epoch + 1)
            assert transferred["rev"] == receipt["rev"] == written_a["rev"] + 1
            assert transferred["cards"] == written_a["cards"]

            await update_card(planner_a, provider, "a-after-handoff", "Rejected A write", accepted=False)
            assert await http.request("GET", board_path) == transferred
            await update_card(planner_b, provider, "b-after-handoff", "Written by B", accepted=True)
            written_b = await http.request("GET", board_path)
            assert_owner(written_b, b, epoch + 1)
            assert written_b["rev"] == transferred["rev"] + 1
            assert card_title(written_b) == "Written by B"
    finally:
        await provider.stop()
        binary.unlink()
