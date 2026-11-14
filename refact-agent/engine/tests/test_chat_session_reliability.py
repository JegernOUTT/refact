import pytest
from owned_chat_sessions import chat, owned_engine, Chat, TINY_PNG

pytestmark = pytest.mark.asyncio


async def test_invalid_multimodal_content_rejected(chat):
    body, _ = await chat.command("user_message", expected=400, content=[{"type": "unknown_type", "data": "some data"}])
    assert "Unknown content type" in body["error"]


async def test_missing_type_field_rejected(chat):
    body, _ = await chat.command("user_message", expected=400, content=[{"text": "hello"}])
    assert "type" in body["error"]


async def test_empty_content_array_accepted(chat):
    await chat.command("user_message", content=[])
    user = (await chat.next("message_added", lambda e: e["message"]["role"] == "user"))["message"]
    assert user["content"] == ""
    persisted = next(m for m in (await chat.snapshot())["messages"] if m["message_id"] == user["message_id"])
    assert persisted["content"] == ""


async def test_valid_text_content_accepted(chat):
    await chat.command("user_message", content=[{"type": "text", "text": "Hello world"}])
    message = (await chat.next("message_added", lambda e: e["message"]["role"] == "user"))["message"]
    assert message["content"] == [{"m_type": "text", "m_content": "Hello world"}]


async def test_too_many_images_rejected(chat):
    body, _ = await chat.command("user_message", expected=400,
                                 content=[{"type": "image_url", "image_url": {"url": TINY_PNG}}] * 51)
    assert "image" in body["error"].lower()


async def test_trajectory_subscribe_receives_events(chat):
    trajectories = Chat(chat.engine, chat.session)
    async with trajectories.subscribe("/v1/trajectories/subscribe"):
        await chat.turn("Trigger owned trajectory persistence")
        event = await trajectories.next("updated", lambda e: e.get("id") == chat.id)
        assert event["id"] == chat.id
