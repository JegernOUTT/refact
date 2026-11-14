import json

import pytest

from owned_utility_tests import GITHUB_ASK_RULE, GITHUB_DENY_RULE, GITHUB_TOOL, until, utility_engine


def github_call(call_id, arguments):
    return {"id": call_id, "type": "function", "function": {
        "name": GITHUB_TOOL, "arguments": arguments if isinstance(arguments, str) else json.dumps(arguments)}}


async def check(client, tool_calls):
    response = await client.post("/tools-check-if-confirmation-needed", json={"tool_calls": tool_calls})
    return response


async def fixture_tool_loaded(client):
    async def loaded():
        response = await client.get("/tools")
        assert response.status_code == 200, response.text
        names = {tool["spec"]["name"] for group in response.json() for tool in group["tools"]}
        return GITHUB_TOOL in names
    await until("configured github confirmation fixture tool", loaded)


@pytest.mark.asyncio
async def test_paused_by_confirmation_and_denial_rules(utility_engine):
    async with utility_engine.client() as client:
        await fixture_tool_loaded(client)
        response = await check(client, [
            github_call("1", {"command": "repo delete", "project_dir": str(utility_engine.workspace)}),
            github_call("2", {"command": "auth token", "project_dir": str(utility_engine.workspace)}),
            github_call("3", {"command": "repo list", "project_dir": str(utility_engine.workspace)}),
        ])
    assert response.status_code == 200, response.text
    data = response.json()
    assert data["pause"] is True
    assert len(data["pause_reasons"]) == 2, data
    reasons = {reason["tool_call_id"]: reason for reason in data["pause_reasons"]}
    assert set(reasons) == {"1", "2"}, data
    assert reasons["1"]["type"] == "confirmation" and reasons["1"]["rule"] == GITHUB_ASK_RULE, data
    assert reasons["1"]["command"] == "gh 'repo delete'", data
    assert reasons["2"]["type"] == "denial" and reasons["2"]["rule"] == GITHUB_DENY_RULE, data
    assert reasons["2"]["command"] == "gh 'auth token'", data
    for reason in reasons.values():
        assert reason["integr_config_path"] == str(utility_engine.github_config), data


@pytest.mark.asyncio
async def test_allowed_command_does_not_pause(utility_engine):
    async with utility_engine.client() as client:
        await fixture_tool_loaded(client)
        response = await check(client, [github_call("3", {"command": "repo list"})])
    assert response.status_code == 200, response.text
    assert response.json() == {"pause": False, "pause_reasons": []}


@pytest.mark.asyncio
async def test_unparseable_arguments_are_denied(utility_engine):
    async with utility_engine.client() as client:
        await fixture_tool_loaded(client)
        response = await check(client, [github_call("4", "{not json")])
    assert response.status_code == 200, response.text
    data = response.json()
    assert data["pause"] is False
    assert len(data["pause_reasons"]) == 1, data
    reason = data["pause_reasons"][0]
    assert reason["type"] == "denial" and reason["tool_call_id"] == "4", data
    assert reason["rule"].startswith("tool parsing problem"), data


@pytest.mark.asyncio
async def test_missing_required_argument_is_not_authorized_by_rules(utility_engine):
    async with utility_engine.client() as client:
        await fixture_tool_loaded(client)
        response = await check(client, [github_call("5", {"project_dir": str(utility_engine.workspace)})])
    assert response.status_code == 200, response.text
    assert response.json() == {"pause": False, "pause_reasons": []}


@pytest.mark.asyncio
async def test_malformed_request_body_is_rejected(utility_engine):
    async with utility_engine.client() as client:
        response = await client.post("/tools-check-if-confirmation-needed",
                                     json={"tool_calls": [{"id": "6", "function": {"name": GITHUB_TOOL}}]})
    assert response.status_code == 422, response.text
    assert "JSON problem" in response.text
