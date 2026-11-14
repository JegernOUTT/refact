import pytest
from owned_chat_sessions import chat, owned_engine, TINY_PNG

pytestmark = pytest.mark.asyncio
IMAGE = {"type": "image_url", "image_url": {"url": TINY_PNG}}


async def assert_images(chat, content, attachments, count, text):
    await chat.command("user_message", content=content, attachments=attachments)
    message = (await chat.next("message_added", lambda e: e["message"]["role"] == "user"))["message"]
    parts = message["content"]
    assert isinstance(parts, list), message
    assert [p["m_content"] for p in parts if p["m_type"] == "text"] == [text]
    images = [p for p in parts if p["m_type"] == "image/png"]
    assert len(images) == count, message
    assert all(p["m_content"] == TINY_PNG.split(",", 1)[1] for p in images), message
    persisted = next(m for m in (await chat.snapshot())["messages"] if m["message_id"] == message["message_id"])
    assert persisted["content"] == parts


async def test_string_content_with_attachments(chat):
    await assert_images(chat, "Image attachment", [IMAGE], 1, "Image attachment")


async def test_multimodal_content_with_attachments(chat):
    await assert_images(chat, [{"type": "text", "text": "Three images"}, IMAGE], [IMAGE, IMAGE], 3, "Three images")


async def test_attachments_exceed_image_limit(chat):
    body, _ = await chat.command("user_message", expected=400,
                                 content=[IMAGE] * 25, attachments=[IMAGE] * 26)
    assert "image" in body["error"].lower()


async def test_attachment_missing_url(chat):
    body, _ = await chat.command("user_message", expected=400, content="Missing", attachments=[{"image_url": {}}])
    assert "image_url.url" in body["error"]


async def test_attachment_invalid_data_url(chat):
    body, _ = await chat.command("user_message", expected=400, content="Invalid", attachments=[{"image_url": {"url": "not-a-valid-data-url"}}])
    assert "image" in body["error"].lower()
