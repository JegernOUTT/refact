import pytest
from owned_chat_sessions import chat, owned_engine

pytestmark = pytest.mark.asyncio


async def test_update_message(chat):
    user = await chat.turn("Original message")
    await chat.command("update_message", message_id=user["message_id"], content="Updated message content")
    updated = await chat.next("message_updated", lambda e: e["message_id"] == user["message_id"])
    assert updated["message"]["message_id"] == user["message_id"]
    assert updated["message"]["content"] == "Updated message content"


async def test_remove_message(chat):
    user = await chat.turn("Message to remove")
    await chat.command("remove_message", message_id=user["message_id"])
    removed = await chat.next("message_removed", lambda e: e["message_id"] == user["message_id"])
    assert removed["message_id"] == user["message_id"]
    assert all(m["message_id"] != user["message_id"] for m in (await chat.snapshot())["messages"])


async def test_retry_from_index(chat):
    first = await chat.turn("First message")
    second = await chat.turn("Second message")
    before = (await chat.snapshot())["messages"]
    index = next(i for i, m in enumerate(before) if m["message_id"] == second["message_id"])
    await chat.command("retry_from_index", index=index, content="Replacement second message")
    truncated = await chat.next("messages_truncated")
    assert truncated["from_index"] == index
    replacement = await chat.next("message_added", lambda e: e["message"]["role"] == "user")
    assert replacement["message"]["content"] == "Replacement second message"
    after = (await chat.snapshot())["messages"]
    assert any(m["message_id"] == first["message_id"] for m in after)
    assert all(m["message_id"] != second["message_id"] for m in after)


async def test_snapshot_after_edit(chat):
    user = await chat.turn("Original content")
    await chat.command("update_message", message_id=user["message_id"], content="Updated content")
    await chat.next("message_updated", lambda e: e["message_id"] == user["message_id"])
    snapshot = await chat.snapshot()
    matching = [m for m in snapshot["messages"] if m["message_id"] == user["message_id"]]
    assert len(matching) == 1
    assert matching[0]["content"] == "Updated content"
