import pytest

from owned_utility_tests import ALPHA_FILE, BETA_FILE, until, utility_engine


async def at_completion_post(client, query: str) -> dict:
    payload = {"query": query.replace("|", ""), "cursor": query.find("|"), "top_n": 50}
    response = await client.post("/at-command-completion", json=payload)
    assert response.status_code == 200, f"Response code: {response.status_code}: {response.text}"
    decoded = response.json()
    assert set(decoded) >= {"completions", "replace", "is_cmd_executable"}, decoded
    return decoded


@pytest.mark.asyncio
async def test_bare_at_lists_registered_commands(utility_engine):
    async with utility_engine.client() as client:
        resp = await at_completion_post(client, "\n@|\n    ")
    assert resp["replace"] == [1, 2]
    assert resp["completions"], resp
    assert all(x.endswith(" ") and x.startswith("@") for x in resp["completions"]), resp
    assert {"@file ", "@tree "} <= set(resp["completions"]), resp
    assert resp["is_cmd_executable"] is False


@pytest.mark.asyncio
async def test_at_after_text_replaces_only_command_token(utility_engine):
    async with utility_engine.client() as client:
        resp = await at_completion_post(client, "\nsome text over here @|\n    ")
    assert resp["replace"] == [21, 22]
    assert "@file " in resp["completions"], resp


@pytest.mark.asyncio
async def test_partial_command_completes_to_file(utility_engine):
    async with utility_engine.client() as client:
        resp = await at_completion_post(client, "\n@file abc and @fi|\n    ")
    assert resp["replace"] == [15, 18]
    assert resp["completions"] == ["@file "]


@pytest.mark.asyncio
async def test_file_argument_completes_seeded_workspace_file(utility_engine):
    async with utility_engine.client() as client:
        async def seeded():
            resp = await at_completion_post(client, "\n@file abc and @file utility_alpha|\n    ")
            hits = [x for x in resp["completions"] if x.strip().endswith(ALPHA_FILE.split("/")[-1])]
            return (resp, hits) if hits else None
        resp, hits = await until("seeded workspace file completion", seeded)
    assert resp["replace"] == [21, 34], resp
    assert hits and all(x.endswith(" ") for x in hits), resp
    assert resp["completions"][0] == ALPHA_FILE + " ", resp
    assert set(resp["completions"]) <= {ALPHA_FILE + " ", BETA_FILE + " "}, resp
    async with utility_engine.client() as client:
        empty = await at_completion_post(client, "\n@file abc and @file |\n    ")
    assert empty["replace"] == [21, 21], empty
    assert len(empty["completions"]) != 0, empty
    assert all(x.endswith(" ") for x in empty["completions"]), empty


@pytest.mark.asyncio
async def test_unknown_command_and_plain_text_have_no_completion(utility_engine):
    async with utility_engine.client() as client:
        unknown = await at_completion_post(client, "\n@zzznotacommand|\n")
        plain = await at_completion_post(client, "\nplain text only|\n")
    assert unknown["completions"] == [] and plain["completions"] == [], (unknown, plain)
    assert plain["replace"] == [-1, -1]
