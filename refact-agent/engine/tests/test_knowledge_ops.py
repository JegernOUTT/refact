import os

import pytest

from owned_utility_tests import memory_text, until, utility_engine


def write_memory(utility_engine, name, identifier, body="Original content here."):
    path = utility_engine.knowledge / name
    path.write_text(memory_text(identifier, "Original Title", ["test", "original"], "code", [], [], body))
    return path


async def graph_ids(client):
    response = await client.get("/knowledge-graph")
    assert response.status_code == 200, response.text
    return {n["id"] for n in response.json()["nodes"] if n["node_type"].startswith("doc")}


async def wait_indexed(client, identifier):
    await until(f"{identifier} visible in knowledge graph",
                lambda: _contains(client, identifier))


async def _contains(client, identifier):
    return identifier in await graph_ids(client)


async def update(client, payload):
    return await client.post("/knowledge/update-memory", json=payload)


async def delete(client, payload):
    return await client.post("/knowledge/delete-memory", json=payload)


@pytest.mark.asyncio
async def test_update_memory(utility_engine):
    test_file = write_memory(utility_engine, "test_memory.md", "test-123")
    async with utility_engine.client() as client:
        response = await update(client, {
            "file_path": str(test_file), "title": "Updated Title", "content": "Updated content here.",
            "tags": ["test", "updated"], "kind": "decision", "filenames": ["src/main.rs"], "auto_link": False})
        assert response.status_code == 200, response.text
        assert response.json() == {"success": True}
        graph = (await client.get("/knowledge-graph?include_content=1")).json()
    text = test_file.read_text()
    assert 'title: "Updated Title"' in text
    assert text.rstrip().endswith("Updated content here.")
    assert "Original content here." not in text
    assert 'tags: ["test", "updated"]' in text
    assert "kind: decision" in text
    assert 'filenames: ["src/main.rs"]' in text
    assert 'id: "test-123"' in text
    node = next(n for n in graph["nodes"] if n["id"] == "test-123")
    assert node["node_type"] == "doc_decision" and node["title"] == "Updated Title", node
    assert node["content"].strip() == "Updated content here."


@pytest.mark.asyncio
async def test_update_memory_not_found(utility_engine):
    missing = utility_engine.knowledge / "missing.md"
    async with utility_engine.client() as client:
        response = await update(client, {"file_path": str(missing), "content": "Some content", "tags": []})
    assert response.status_code == 404, response.text
    assert not missing.exists()


@pytest.mark.asyncio
async def test_update_rejects_invalid_status(utility_engine):
    test_file = write_memory(utility_engine, "invalid_status.md", "invalid-status")
    before = test_file.read_text()
    async with utility_engine.client() as client:
        response = await update(client, {"file_path": str(test_file), "status": "bogus"})
    assert response.status_code == 400, response.text
    assert "Invalid status" in response.text
    assert test_file.read_text() == before


@pytest.mark.asyncio
async def test_delete_memory_permanent(utility_engine):
    test_file = write_memory(utility_engine, "test_delete.md", "test-delete")
    async with utility_engine.client() as client:
        await wait_indexed(client, "test-delete")
        response = await delete(client, {"file_path": str(test_file), "archive": False})
        assert response.status_code == 200, response.text
        assert response.json() == {"success": True}
        ids = await graph_ids(client)
    assert not test_file.exists()
    assert "test-delete" not in ids and {"kg-alpha", "kg-beta"} <= ids


@pytest.mark.asyncio
async def test_delete_memory_archive(utility_engine):
    test_file = write_memory(utility_engine, "test_archive.md", "test-archive", "Content to archive.")
    async with utility_engine.client() as client:
        await wait_indexed(client, "test-archive")
        response = await delete(client, {"file_path": str(test_file), "archive": True})
        assert response.status_code == 200, response.text
        assert response.json() == {"success": True}
        ids = await graph_ids(client)
    assert test_file.exists()
    text = test_file.read_text()
    assert "status: archived" in text and "deprecated_at:" in text
    assert text.rstrip().endswith("Content to archive.")
    assert "test-archive" not in ids


@pytest.mark.asyncio
async def test_delete_memory_not_found(utility_engine):
    missing = utility_engine.knowledge / "missing.md"
    async with utility_engine.client() as client:
        response = await delete(client, {"file_path": str(missing), "archive": False})
        wrong_method = await client.request("DELETE", "/knowledge/delete-memory",
                                            json={"file_path": str(missing), "archive": False})
    assert response.status_code == 404, response.text
    assert wrong_method.status_code == 405, wrong_method.text


@pytest.mark.asyncio
async def test_path_traversal_attack(utility_engine):
    outside = utility_engine.workspace / "outside.md"
    before = outside.read_text()
    attacks = [
        "../../../etc/passwd",
        "../../.ssh/id_rsa",
        str(utility_engine.knowledge / ".." / ".." / "outside.md"),
        str(utility_engine.knowledge / ".." / ".." / ".." / "config" / ".config" / "refact" / "privacy.yaml"),
    ]
    async with utility_engine.client() as client:
        for attack in attacks:
            response = await update(client, {"file_path": attack, "content": "Malicious content", "tags": []})
            assert response.status_code in (403, 404), (attack, response.status_code, response.text)
        direct = await update(client, {"file_path": attacks[2], "content": "Malicious content"})
    assert direct.status_code == 403 and "outside knowledge" in direct.text, direct.text
    assert outside.read_text() == before


@pytest.mark.asyncio
async def test_symlink_escape_attack(utility_engine):
    outside_dir = utility_engine.workspace.parent / "outside"
    outside_dir.mkdir()
    target_file = outside_dir / "secret.md"
    target_file.write_text("Secret content")
    symlink_path = utility_engine.knowledge / "escape.md"
    symlink_path.symlink_to(target_file)
    async with utility_engine.client() as client:
        updated = await update(client, {"file_path": str(symlink_path), "content": "Overwrite via symlink"})
        deleted = await delete(client, {"file_path": str(symlink_path), "archive": False})
    assert updated.status_code == 403 and "symlink" in updated.text, updated.text
    assert deleted.status_code == 403 and "symlink" in deleted.text, deleted.text
    assert target_file.read_text() == "Secret content"
    assert symlink_path.is_symlink()


@pytest.mark.asyncio
async def test_non_markdown_file_rejected(utility_engine):
    async with utility_engine.client() as client:
        for name in ("test.txt", "test.py", "test.sh", "test"):
            test_file = utility_engine.knowledge / name
            test_file.write_text("Some content")
            response = await update(client, {"file_path": str(test_file), "content": "Content", "tags": []})
            assert response.status_code == 400, (name, response.text)
            assert test_file.read_text() == "Some content"
        directory = utility_engine.knowledge / "folder.md"
        directory.mkdir()
        response = await update(client, {"file_path": str(directory), "content": "Content"})
    assert response.status_code == 400, response.text


@pytest.mark.asyncio
async def test_path_outside_knowledge_dir(utility_engine):
    outside_file = utility_engine.workspace / "outside.md"
    before = outside_file.read_text()
    async with utility_engine.client() as client:
        updated = await update(client, {"file_path": str(outside_file), "content": "Outside update"})
        deleted = await delete(client, {"file_path": str(outside_file), "archive": False})
    assert updated.status_code == 403, updated.text
    assert deleted.status_code == 403, deleted.text
    assert outside_file.read_text() == before


@pytest.mark.asyncio
async def test_readonly_knowledge_dir_update_fails_without_mutation(utility_engine):
    if os.geteuid() == 0:
        raise AssertionError("readonly lane requires a non-root test user")
    test_file = write_memory(utility_engine, "readonly.md", "readonly-doc")
    before = test_file.read_text()
    mode = utility_engine.knowledge.stat().st_mode
    utility_engine.knowledge.chmod(0o555)
    try:
        async with utility_engine.client() as client:
            response = await update(client, {"file_path": str(test_file), "content": "Should not land"})
            deleted = await delete(client, {"file_path": str(test_file), "archive": False})
    finally:
        utility_engine.knowledge.chmod(mode)
    assert response.status_code == 500, response.text
    assert deleted.status_code == 500, deleted.text
    assert test_file.read_text() == before
    assert not [p for p in utility_engine.knowledge.iterdir() if p.name.startswith(".readonly.md.tmp")]
