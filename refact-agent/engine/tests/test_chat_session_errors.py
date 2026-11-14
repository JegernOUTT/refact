import pytest
from owned_chat_sessions import chat, owned_engine, MODEL

pytestmark = pytest.mark.asyncio


async def test_invalid_model_error(chat):
    await chat.configure("localstub/nonexistent-model-xyz")
    await chat.command("user_message", content="Hello")
    error = await chat.next("runtime_updated", lambda e: e["state"] == "error")
    assert error.get("error"), error
    snapshot = await chat.snapshot()
    assert snapshot["runtime"]["state"] == "error"
    assert not any(m["role"] == "assistant" for m in snapshot["messages"])


async def test_ack_correlation_invalid_content(chat):
    body, request_id = await chat.command("user_message", expected=400, content=[{"type": "invalid_type", "data": "test"}])
    assert body["status"] == "invalid_content"
    ack = await chat.next("ack", lambda e: e["client_request_id"] == request_id)
    assert ack["accepted"] is False
    assert ack["result"]["error"] == body["error"]


async def test_ack_correlation_duplicate(chat):
    _, request_id = await chat.command("set_params", expected=200, patch={"model": MODEL})
    first = await chat.next("ack", lambda e: e["client_request_id"] == request_id)
    assert first["accepted"] is True
    body, _ = await chat.command("set_params", expected=200, request_id=request_id, patch={"model": MODEL})
    assert body["status"] == "duplicate"
    duplicate = await chat.next("ack", lambda e: e["client_request_id"] == request_id)
    assert duplicate["accepted"] is True
    assert duplicate["result"]["duplicate"] is True


async def test_uncapped_command_burst_drains(chat):
    user = await chat.turn("Burst target")
    requests = []
    for i in range(150):
        _, request_id = await chat.command("update_message", message_id=user["message_id"], content=f"Burst {i}")
        requests.append(request_id)
    observed = []
    while len(observed) < len(requests):
        ack = await chat.next("ack", lambda e: e["client_request_id"] in requests)
        assert ack["accepted"] is True
        assert ack["result"]["queued"] is True
        observed.append(ack["client_request_id"])
    assert observed == requests
    updates = lambda: [e for e in chat.events if e["type"] == "message_updated" and e["message_id"] == user["message_id"]]
    while len(updates()) < 150:
        await chat.next("message_updated", lambda e: e["message_id"] == user["message_id"])
    assert [e["message"]["content"] for e in updates()] == [f"Burst {i}" for i in range(150)]
    persisted = next(m for m in (await chat.snapshot())["messages"] if m["message_id"] == user["message_id"])
    assert persisted["content"] == "Burst 149"
