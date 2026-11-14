import asyncio
import json
import os
import uuid
from pathlib import Path

import pytest

import httpx
from owned_engine import owned_engine, MODEL, native_messages


async def _post_command(client: httpx.AsyncClient, base_url: str, chat_id: str, payload: dict) -> dict:
    if payload.get("type") == "set_params":
        payload = {**payload, "patch": {"model": MODEL, **payload["patch"]}}
    response = await client.post(
        f"{base_url}/v1/chats/{chat_id}/commands",
        json={"client_request_id": str(uuid.uuid4()), **payload},
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def _init_workspace(client: httpx.AsyncClient, base_url: str) -> None:
    response = await client.post(
        f"{base_url}/v1/lsp-initialize",
        json={"project_roots": []},
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
                try:
                    event = await asyncio.wait_for(_read_sse_event(lines), timeout=remaining)
                except TimeoutError:
                    raise AssertionError(f"SSE timed out; saw {events!r}") from None
                events.append(event)
                if predicate(events):
                    return events


def _message(event: dict) -> dict:
    return event.get("message") or {}


def _event_meta(message: dict) -> dict:
    return message.get("event") or {}


def _plan_meta(message: dict) -> dict:
    return message.get("plan") or {}


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
    model_name: str = MODEL,
) -> list[dict]:
    await _post_command(client, base_url, chat_id, {"type": "set_params", "patch": {"model": MODEL}})
    assistant = {
        "role": "assistant",
        "content": "",
        "tool_calls": [_tool_call(name, arguments)],
    }
    response = await client.post(
        f"{base_url}/v1/tools-execute",
        json={
            "messages": [{"role": "user", "content": f"Call {name}"}, assistant],
            "n_ctx": 32768,
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
    return await native_messages(client, base_url, chat_id, data)


def _tool_json(message: dict) -> dict:
    content = message.get("content")
    if isinstance(content, str):
        return json.loads(content)
    return content


async def _cron_create(client: httpx.AsyncClient, base_url: str, chat_id: str, args: dict) -> dict:
    messages = await _execute_tool(client, base_url, chat_id, "schedule", {"action": "create", **args})
    return _tool_json(messages[-1])


async def _cron_delete(client: httpx.AsyncClient, base_url: str, chat_id: str, task_id: str) -> dict:
    messages = await _execute_tool(client, base_url, chat_id, "schedule", {"action": "delete", "id": task_id})
    return _tool_json(messages[-1])


async def _cron_list(client: httpx.AsyncClient, base_url: str, chat_id: str) -> list[dict]:
    messages = await _execute_tool(client, base_url, chat_id, "schedule", {"action": "list", "scope": "all"})
    return _tool_json(messages[-1])


@pytest.mark.asyncio
async def test_cron_create_list_delete_roundtrip(owned_engine):
    base_url = owned_engine.base_url
    chat_id = f"test-cron-roundtrip-{uuid.uuid4().hex[:8]}"
    task_id = None

    async with httpx.AsyncClient(timeout=10.0) as client:
        await _init_workspace(client, base_url)
        try:
            created = await _cron_create(
                client,
                base_url,
                chat_id,
                {
                    "cron": "*/5 * * * *",
                    "prompt": "roundtrip prompt",
                    "recurring": True,
                    "durable": False,
                    "description": f"roundtrip {uuid.uuid4().hex[:8]}",
                },
            )
            task_id = created["id"]
            listed = await _cron_list(client, base_url, chat_id)
            assert any(task["id"] == task_id for task in listed)

            deleted = await _cron_delete(client, base_url, chat_id, task_id)
            task_id = None
            assert deleted == {"removed": True}

            listed_again = await _cron_list(client, base_url, chat_id)
            assert all(task["id"] != created["id"] for task in listed_again)
        finally:
            if task_id is not None:
                await _cron_delete(client, base_url, chat_id, task_id)


@pytest.mark.asyncio
async def test_cron_fires_and_injects_event(owned_engine):
    base_url = owned_engine.base_url
    chat_id = f"test-cron-fire-{uuid.uuid4().hex[:8]}"
    prompt = f"cron fired prompt {uuid.uuid4().hex}"
    task_id = None

    async with httpx.AsyncClient(timeout=10.0) as client:
        await _init_workspace(client, base_url)
        try:
            created = await _cron_create(
                client,
                base_url,
                chat_id,
                {
                    "cron": "* * * * *",
                    "prompt": prompt,
                    "recurring": True,
                    "durable": False,
                    "description": "fire soon",
                },
            )
            task_id = created["id"]

            events = await _collect_until(
                base_url,
                chat_id,
                lambda seen: _event_message(seen, "cron_fire") is not None
                and any(message.get("role") == "user" and prompt in message.get("content", "") for message in _messages(seen)),
                timeout=75.0,
            )

            cron_event = _event_message(events, "cron_fire")
            assert cron_event is not None
            payload = _event_meta(cron_event).get("payload", {})
            assert payload.get("task_id") == task_id
            assert any(message.get("role") == "user" and prompt in message.get("content", "") for message in _messages(events))
        finally:
            if task_id is not None:
                await _cron_delete(client, base_url, chat_id, task_id)
