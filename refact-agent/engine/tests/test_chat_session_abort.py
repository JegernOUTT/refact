import asyncio
import pytest
from owned_chat_sessions import chat, held_provider, owned_engine

pytestmark = [pytest.mark.asyncio, pytest.mark.usefixtures("held_provider")]


async def abort_active(chat, held_provider):
    await chat.command("user_message", content="Hold this local stream")
    start = await chat.next("stream_started")
    assert start["message_id"]
    delta = await chat.next("stream_delta", lambda e: e["message_id"] == start["message_id"])
    assert "partial-owned-output" in str(delta)
    await asyncio.wait_for(held_provider.wait(), 5)
    assert not any(e["type"] == "stream_finished" for e in chat.events)
    await chat.command("abort", expected=200)
    finish = await chat.next("stream_finished", lambda e: e["message_id"] == start["message_id"])
    assert finish["finish_reason"] == "abort"
    await chat.next("message_removed", lambda e: e["message_id"] == start["message_id"])
    await chat.next("runtime_updated", lambda e: e["state"] == "idle")
    snapshot = await chat.snapshot()
    assert snapshot["runtime"]["state"] == "idle"
    assert all(m["message_id"] != start["message_id"] for m in snapshot["messages"])


async def test_abort_while_streaming(chat, held_provider):
    await abort_active(chat, held_provider)


async def test_abort_idempotency(chat, held_provider):
    await abort_active(chat, held_provider)
    body, _ = await chat.command("abort", expected=200)
    assert body["status"] == "aborted"
    assert (await chat.snapshot())["runtime"]["state"] == "idle"


async def test_abort_before_stream_starts(chat, held_provider):
    assert not any(e["type"] == "stream_started" for e in chat.events)
    assert not held_provider.is_set()
    body, _ = await chat.command("abort", expected=200)
    assert body["status"] == "aborted"
    assert (await chat.snapshot())["runtime"]["state"] == "idle"
    await abort_active(chat, held_provider)
