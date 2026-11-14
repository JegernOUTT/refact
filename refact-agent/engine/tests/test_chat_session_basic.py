import pytest
from owned_chat_sessions import chat, owned_engine, MODEL

pytestmark = pytest.mark.asyncio


async def test_subscribe_returns_snapshot(chat):
    snapshot = await chat.snapshot()
    assert snapshot["chat_id"] == chat.id
    assert snapshot["runtime"]["state"] == "idle"
    assert snapshot["thread"]["model"] == MODEL


async def test_send_command_accepted(chat):
    body, request_id = await chat.command("user_message", content="Hello, world!")
    assert body["status"] == "accepted"
    ack = await chat.next("ack", lambda e: e["client_request_id"] == request_id)
    assert ack["accepted"] is True
    added = await chat.next("message_added", lambda e: e["message"]["role"] == "user")
    assert added["message"]["content"] == "Hello, world!"


async def test_duplicate_command_detected(chat):
    _, request_id = await chat.command("set_params", expected=200, patch={"model": MODEL})
    body, _ = await chat.command("set_params", expected=200, request_id=request_id, patch={"model": MODEL})
    assert body["status"] == "duplicate"
    await chat.next("ack", lambda e: e["client_request_id"] == request_id and e.get("result", {}).get("duplicate") is True)


async def test_full_message_flow(chat):
    user = await chat.turn("Local deterministic transport")
    snapshot = await chat.snapshot()
    assert any(m["message_id"] == user["message_id"] for m in snapshot["messages"])
    assert any(m["role"] == "assistant" and m["content"] == "first-turn-receipt-acknowledged" for m in snapshot["messages"])


async def test_abort_command(chat):
    body, _ = await chat.command("abort", expected=200)
    assert body["status"] == "aborted"
    assert (await chat.snapshot())["runtime"]["state"] == "idle"
