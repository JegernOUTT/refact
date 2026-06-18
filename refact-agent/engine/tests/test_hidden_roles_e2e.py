import asyncio
import json
import os
import uuid
from pathlib import Path

import pytest

httpx = pytest.importorskip("httpx")

def _base_url() -> str:
    port = os.environ.get("REFACT_LSP_PORT")
    if not port:
        pytest.skip("REFACT_LSP_PORT is unset; live refact-lsp integration tests skipped")
    return f"http://127.0.0.1:{port}"


def _workspace_root() -> Path:
    return Path(__file__).resolve().parents[2]


async def _post_command(client: httpx.AsyncClient, base_url: str, chat_id: str, payload: dict) -> dict:
    response = await client.post(
        f"{base_url}/v1/chats/{chat_id}/commands",
        json={"client_request_id": str(uuid.uuid4()), **payload},
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def _init_workspace(client: httpx.AsyncClient, base_url: str) -> None:
    response = await client.post(
        f"{base_url}/v1/lsp-initialize",
        json={"project_roots": [_workspace_root().as_uri()]},
    )
    assert response.status_code == 200, response.text


async def _read_sse_event(lines) -> dict:
    async for line in lines:
        if line.startswith("data: "):
            return json.loads(line[6:])
    raise AssertionError("SSE stream ended before an event arrived")


async def _collect_until(base_url: str, chat_id: str, predicate, timeout: float = 30.0) -> list[dict]:
    events: list[dict] = []
    async with httpx.AsyncClient(timeout=None) as client:
        async with client.stream(
            "GET",
            f"{base_url}/v1/chats/subscribe",
            params={"chat_id": chat_id},
        ) as response:
            assert response.status_code == 200, response.text
            lines = response.aiter_lines()
            deadline = asyncio.get_running_loop().time() + timeout
            while True:
                remaining = deadline - asyncio.get_running_loop().time()
                if remaining <= 0:
                    raise AssertionError(f"timed out waiting for events; saw {events!r}")
                event = await asyncio.wait_for(_read_sse_event(lines), timeout=remaining)
                events.append(event)
                if predicate(events):
                    return events


def _message(event: dict) -> dict:
    return event.get("message") or {}


def _event_meta(message: dict) -> dict:
    return message.get("extra", {}).get("event") or {}


def _plan_meta(message: dict) -> dict:
    return message.get("extra", {}).get("plan") or {}


def _messages(events: list[dict]) -> list[dict]:
    result: list[dict] = []
    for event in events:
        if event.get("type") == "snapshot":
            result.extend(event.get("messages") or [])
        if event.get("type") == "message_added":
            result.append(_message(event))
    return result


def _event_message(events: list[dict], subkind: str) -> dict | None:
    for message in _messages(events):
        if message.get("role") == "event" and _event_meta(message).get("subkind") == subkind:
            return message
    return None


def _plan_message(events: list[dict], version: int) -> dict | None:
    for message in _messages(events):
        if message.get("role") == "plan" and _plan_meta(message).get("version") == version:
            return message
    return None


def _goal_meta(message: dict) -> dict:
    return message.get("extra", {}).get("goal") or {}


def _goal_message(events: list[dict], version: int | None = None) -> dict | None:
    for message in _messages(events):
        if message.get("role") == "goal" and (version is None or _goal_meta(message).get("version") == version):
            return message
    return None


def _runtime_goal_event(events: list[dict]) -> dict | None:
    for event in reversed(events):
        if event.get("type") == "runtime_updated" and "goal_status" in event:
            return event
    return None


async def _snapshot(base_url: str, chat_id: str) -> dict:
    events = await _collect_until(base_url, chat_id, lambda seen: any(e.get("type") == "snapshot" for e in seen))
    return next(event for event in events if event.get("type") == "snapshot")


async def _drive_commands_and_collect(
    base_url: str,
    chat_id: str,
    commands: list[dict],
    predicate,
    timeout: float = 30.0,
) -> list[dict]:
    waiter = asyncio.create_task(_collect_until(base_url, chat_id, predicate, timeout=timeout))
    await asyncio.sleep(0.1)
    async with httpx.AsyncClient(timeout=10.0) as client:
        for command in commands:
            await _post_command(client, base_url, chat_id, command)
    return await waiter


def _goal_budget(**overrides) -> dict:
    budget = {
        "max_turns": 10,
        "max_minutes": 15,
        "max_tokens": 200_000,
        "cooldown_ms": 1_500,
        "no_progress_token_threshold": 50,
        "no_progress_turns": 2,
    }
    budget.update(overrides)
    return budget


def _goal_progress(**overrides) -> dict:
    progress = {
        "turns_used": 0,
        "tokens_used": 0,
        "started_at_ms": 1,
        "no_progress_turns": 0,
        "last_nudge_at_ms": 0,
    }
    progress.update(overrides)
    return progress


def _goal_hidden_message(content: str, **overrides) -> dict:
    meta = {
        "mode": "agent",
        "version": 1,
        "created_at_ms": 1,
        "supersedes": None,
        "active": True,
        "status": "active",
        "budget": _goal_budget(),
        "progress": _goal_progress(),
        "attempts": [],
        "events": [],
    }
    meta.update(overrides)
    return {
        "message_id": f"goal-{uuid.uuid4().hex[:8]}",
        "role": "goal",
        "content": content,
        "extra": {"goal": meta},
    }


def _goal_pursuit_message(content: str, payload: dict) -> dict:
    return {
        "message_id": f"goal-pursuit-{uuid.uuid4().hex[:8]}",
        "role": "event",
        "content": content,
        "extra": {
            "event": {
                "subkind": "goal_pursuit",
                "source": "chat.goal_verifier",
                "payload": payload,
            }
        },
    }


def _tool_call(name: str, arguments: dict, call_id: str | None = None) -> dict:
    return {
        "id": call_id or f"call-{uuid.uuid4().hex[:8]}",
        "type": "function",
        "function": {
            "name": name,
            "arguments": json.dumps(arguments),
        },
    }


async def _execute_tool(
    client: httpx.AsyncClient,
    base_url: str,
    chat_id: str,
    name: str,
    arguments: dict,
    *,
    model_name: str = "gpt-4o-mini",
) -> list[dict]:
    assistant = {
        "role": "assistant",
        "content": "",
        "tool_calls": [_tool_call(name, arguments)],
    }
    response = await client.post(
        f"{base_url}/v1/tools-execute",
        json={
            "messages": [{"role": "user", "content": f"Call {name}"}, assistant],
            "n_ctx": 4096,
            "maxgen": 256,
            "subchat_tool_parameters": {},
            "postprocess_parameters": {
                "use_ast_based_pp": True,
                "useful_background": 5.0,
                "useful_symbol_default": 10.0,
                "downgrade_parent_coef": 0.6,
                "downgrade_body_coef": 0.8,
                "comments_propagate_up_coef": 0.99,
                "close_small_gaps": True,
                "take_floor": 0.0,
                "max_files_n": 0,
            },
            "model_name": model_name,
            "chat_id": chat_id,
            "style": None,
        },
    )
    assert response.status_code == 200, response.text
    data = response.json()
    assert data["tools_ran"] is True
    return data["messages"]


def _tool_json(message: dict) -> dict:
    content = message.get("content")
    if isinstance(content, str):
        return json.loads(content)
    return content


@pytest.mark.asyncio
async def test_mode_switch_installs_plan_and_emits_event():
    base_url = _base_url()
    chat_id = f"test-hidden-mode-{uuid.uuid4().hex[:8]}"

    async def drive() -> None:
        async with httpx.AsyncClient(timeout=10.0) as client:
            await _post_command(
                client,
                base_url,
                chat_id,
                {
                    "type": "set_params",
                    "patch": {"mode": "task_agent", "reason": "Install an integration-test plan"},
                },
            )

    waiter = asyncio.create_task(
        _collect_until(
            base_url,
            chat_id,
            lambda seen: _event_message(seen, "mode_switch") is not None
            and _plan_message(seen, 1) is not None,
        )
    )
    await asyncio.sleep(0.1)
    await drive()
    events = await waiter

    mode_event = _event_message(events, "mode_switch")
    assert mode_event is not None
    assert _event_meta(mode_event)["source"] == "chat.session"
    plan = _plan_message(events, 1)
    assert plan is not None
    assert _plan_meta(plan)["mode"] == "task_agent"


@pytest.mark.asyncio
async def test_set_plan_tool_creates_v2():
    base_url = _base_url()
    chat_id = f"test-hidden-set-plan-{uuid.uuid4().hex[:8]}"

    async with httpx.AsyncClient(timeout=10.0) as client:
        waiter_v1 = asyncio.create_task(
            _collect_until(base_url, chat_id, lambda seen: _plan_message(seen, 1) is not None)
        )
        await asyncio.sleep(0.1)
        await _post_command(client, base_url, chat_id, {"type": "set_params", "patch": {"mode": "task_agent"}})
        await waiter_v1

        messages = await _execute_tool(
            client,
            base_url,
            chat_id,
            "set_plan",
            {"content": "## Updated plan\n- Verify hidden role v2", "summary": "E2E v2"},
        )

    result = _tool_json(messages[-1])
    assert result["version"] == 2

    events = await _collect_until(base_url, chat_id, lambda seen: _plan_message(seen, 2) is not None)
    plan = _plan_message(events, 2)
    assert plan is not None
    assert _plan_meta(plan)["version"] == 2
    assert "Verify hidden role v2" in plan.get("content", "")


@pytest.mark.asyncio
async def test_goal_role_command_happy_path_projection():
    base_url = _base_url()
    chat_id = f"test-goal-met-{uuid.uuid4().hex[:8]}"

    events = await _drive_commands_and_collect(
        base_url,
        chat_id,
        [
            {"type": "set_params", "patch": {"mode": "agent"}},
            {"type": "set_goal", "content": "Ship the hidden goal docs"},
            {"type": "update_goal", "note": "Acceptance: E2E covers MET"},
        ],
        lambda seen: _goal_message(seen, 1) is not None
        and _event_message(seen, "goal_delta") is not None
        and _runtime_goal_event(seen) is not None,
    )

    goal_message = _goal_message(events, 1)
    assert goal_message is not None
    assert goal_message.get("role") == "goal"
    assert _goal_meta(goal_message)["active"] is True
    assert _goal_meta(goal_message)["version"] == 1

    goal_delta = _event_message(events, "goal_delta")
    assert goal_delta is not None
    assert _event_meta(goal_delta)["source"] == "chat.command.update_goal"

    snapshot = await _snapshot(base_url, chat_id)
    goal = snapshot.get("goal")
    assert goal is not None
    assert goal["status"] == "active"
    assert goal["active"] is True
    assert goal["version"] == 1
    assert "Acceptance: E2E covers MET" in goal["content"]

    runtime = _runtime_goal_event(events)
    assert runtime is not None
    assert runtime["goal_active"] is True
    assert runtime["goal_status"] == "active"


@pytest.mark.asyncio
async def test_goal_met_pursuit_restore_projection_completed():
    base_url = _base_url()
    chat_id = f"test-goal-met-{uuid.uuid4().hex[:8]}"
    goal = _goal_hidden_message(
        "Ship the pond",
        status="completed",
        attempts=[
            {
                "at_ms": 10,
                "trigger": "task_done",
                "verdict": "met",
                "gaps": [],
                "verifier_reply": "GOAL: MET",
            }
        ],
        progress=_goal_progress(turns_used=1, tokens_used=77),
    )
    pursuit = _goal_pursuit_message(
        "Goal verification passed.",
        {"kind": "verified", "at_ms": 10, "gaps": []},
    )

    events = await _drive_commands_and_collect(
        base_url,
        chat_id,
        [{"type": "restore_messages", "messages": [goal, pursuit]}],
        lambda seen: _goal_message(seen, 1) is not None
        and _event_message(seen, "goal_pursuit") is not None,
    )

    pursuit_event = _event_message(events, "goal_pursuit")
    assert pursuit_event is not None
    assert _event_meta(pursuit_event)["payload"]["kind"] == "verified"

    snapshot = await _snapshot(base_url, chat_id)
    projected = snapshot.get("goal")
    assert projected is not None
    assert projected["active"] is True
    assert projected["status"] == "completed"
    assert projected["attempts"][0]["verdict"] == "met"
    assert projected["attempts"][0]["verifier_reply"] == "GOAL: MET"
    assert projected["events"][0]["kind"] == "goal_pursuit"
    assert projected["progress"]["turns_used"] == 1
    assert projected["progress"]["tokens_used"] == 77


@pytest.mark.asyncio
async def test_goal_unmet_rearm_restore_projection():
    base_url = _base_url()
    chat_id = f"test-goal-unmet-{uuid.uuid4().hex[:8]}"
    goal = _goal_hidden_message(
        "Ship the pond",
        attempts=[
            {
                "at_ms": 10,
                "trigger": "task_done",
                "verdict": "unmet",
                "gaps": ["missing tests"],
                "verifier_reply": "GOAL: UNMET\n- missing tests",
            }
        ],
    )
    pursuit = _goal_pursuit_message(
        "Goal verification found gaps:\nmissing tests",
        {"kind": "verification_gaps", "at_ms": 10, "gaps": ["missing tests"]},
    )

    events = await _drive_commands_and_collect(
        base_url,
        chat_id,
        [{"type": "restore_messages", "messages": [goal, pursuit]}],
        lambda seen: _goal_message(seen, 1) is not None
        and _event_message(seen, "goal_pursuit") is not None,
    )

    restored_goal = _goal_message(events, 1)
    assert restored_goal is not None
    assert _goal_meta(restored_goal)["attempts"][0]["verdict"] == "unmet"

    pursuit_event = _event_message(events, "goal_pursuit")
    assert pursuit_event is not None
    assert _event_meta(pursuit_event)["payload"]["kind"] == "verification_gaps"

    snapshot = await _snapshot(base_url, chat_id)
    projected = snapshot.get("goal")
    assert projected is not None
    assert projected["status"] == "active"
    assert projected["active"] is True
    assert projected["attempts"][0]["verdict"] == "unmet"
    assert projected["attempts"][0]["gaps"] == ["missing tests"]
    assert projected["events"][0]["kind"] == "goal_pursuit"


@pytest.mark.asyncio
async def test_goal_budget_exhaustion_restore_projection_stops_pursuit():
    base_url = _base_url()
    chat_id = f"test-goal-budget-{uuid.uuid4().hex[:8]}"
    goal = _goal_hidden_message(
        "Stop when budget is exhausted",
        status="budget_exhausted",
        budget=_goal_budget(max_turns=1),
        progress=_goal_progress(turns_used=1, tokens_used=120),
    )
    pursuit = _goal_pursuit_message(
        "Goal budget exhausted.",
        {"kind": "budget_exhausted", "at_ms": 20},
    )

    await _drive_commands_and_collect(
        base_url,
        chat_id,
        [{"type": "restore_messages", "messages": [goal, pursuit]}],
        lambda seen: _goal_message(seen, 1) is not None,
    )

    snapshot = await _snapshot(base_url, chat_id)
    projected = snapshot.get("goal")
    assert projected is not None
    assert projected["active"] is True
    assert projected["status"] == "budget_exhausted"
    assert projected["progress"]["turns_used"] == 1
    assert projected["progress"]["tokens_used"] == 120


@pytest.mark.asyncio
async def test_goal_ownership_transfer_restore_projection():
    base_url = _base_url()
    source_chat_id = f"test-goal-transfer-source-{uuid.uuid4().hex[:8]}"
    target_chat_id = f"test-goal-transfer-target-{uuid.uuid4().hex[:8]}"

    source_goal = _goal_hidden_message(
        "Transfer ownership to a new chat",
        active=False,
        status="transferred",
        transferred_to=target_chat_id,
    )
    target_goal = _goal_hidden_message(
        "Transfer ownership to a new chat",
        transferred_from=source_chat_id,
        progress=_goal_progress(started_at_ms=30),
    )

    await _drive_commands_and_collect(
        base_url,
        source_chat_id,
        [{"type": "restore_messages", "messages": [source_goal]}],
        lambda seen: _goal_message(seen, 1) is not None,
    )
    await _drive_commands_and_collect(
        base_url,
        target_chat_id,
        [{"type": "restore_messages", "messages": [target_goal]}],
        lambda seen: _goal_message(seen, 1) is not None,
    )

    source_snapshot = await _snapshot(base_url, source_chat_id)
    source_projected = source_snapshot.get("goal")
    assert source_projected is not None
    assert source_projected["active"] is False
    assert source_projected["status"] == "transferred"
    assert source_projected["transferred_to"] == target_chat_id

    target_snapshot = await _snapshot(base_url, target_chat_id)
    target_projected = target_snapshot.get("goal")
    assert target_projected is not None
    assert target_projected["active"] is True
    assert target_projected["status"] == "active"
    assert target_projected["transferred_from"] == source_chat_id
    assert target_projected["progress"]["turns_used"] == 0
    assert target_projected["progress"]["tokens_used"] == 0


@pytest.mark.asyncio
async def test_goal_restart_restore_same_owner_counters_survive():
    base_url = _base_url()
    chat_id = f"test-goal-restart-{uuid.uuid4().hex[:8]}"
    goal = _goal_hidden_message(
        "Restore the same owner after restart",
        progress=_goal_progress(turns_used=4, tokens_used=2048, no_progress_turns=1, last_nudge_at_ms=90),
        attempts=[
            {
                "at_ms": 80,
                "trigger": "finish",
                "verdict": "met",
                "gaps": [],
                "verifier_reply": "GOAL: MET",
            }
        ],
    )

    await _drive_commands_and_collect(
        base_url,
        chat_id,
        [{"type": "restore_messages", "messages": [goal]}],
        lambda seen: _goal_message(seen, 1) is not None,
    )

    snapshot = await _snapshot(base_url, chat_id)
    projected = snapshot.get("goal")
    assert projected is not None
    assert projected["active"] is True
    assert projected["status"] == "active"
    assert projected.get("transferred_from") is None
    assert projected.get("transferred_to") is None
    assert projected["progress"]["turns_used"] == 4
    assert projected["progress"]["tokens_used"] == 2048
    assert projected["progress"]["no_progress_turns"] == 1
    assert projected["progress"]["last_nudge_at_ms"] == 90
    assert projected["attempts"][0]["verdict"] == "met"
