import pytest

from owned_utility_tests import ALPHA_BODY, ALPHA_FILE, BETA_BODY, BETA_FILE, utility_engine

EXPECTED_DOCS = {"kg-alpha": "doc_code", "kg-beta": "doc_decision"}


async def graph(client, query=""):
    response = await client.get("/knowledge-graph" + query)
    assert response.status_code == 200, f"Expected 200, got {response.status_code}: {response.text}"
    data = response.json()
    assert {"nodes", "edges", "stats"} <= set(data), data
    return response, data


def doc_nodes(data):
    nodes = {n["id"]: n for n in data["nodes"] if n["node_type"].startswith("doc")}
    assert {k: v["node_type"] for k, v in nodes.items()} == EXPECTED_DOCS, data["nodes"]
    return nodes


@pytest.mark.asyncio
async def test_seeded_graph_structure(utility_engine):
    async with utility_engine.client() as client:
        _, data = await graph(client)
    stats = data["stats"]
    assert stats["doc_count"] == 2 and stats["active_docs"] == 2, stats
    nodes = doc_nodes(data)
    assert nodes["kg-alpha"]["title"] == "Utility Alpha Knowledge"
    assert nodes["kg-alpha"]["file_path"] == str(utility_engine.knowledge / "alpha.md")
    edges = {(e["source"], e["target"], e["edge_type"]) for e in data["edges"]}
    assert ("kg-alpha", "kg-beta", "links_to") in edges, edges
    assert ("kg-alpha", "tag:component:utility-shared", "tagged_with") in edges, edges
    assert ("kg-beta", "tag:component:utility-shared", "tagged_with") in edges, edges
    assert ("kg-alpha", "entity:UtilityAlphaEntity", "mentions") in edges, edges
    assert ("kg-beta", "entity:UtilityBetaEntity", "mentions") in edges, edges
    assert ("kg-alpha", f"file:{ALPHA_FILE}", "references_file") in edges, edges
    assert ("kg-beta", f"file:{BETA_FILE}", "references_file") in edges, edges
    assert stats["edge_count"] == len(data["edges"]), stats


@pytest.mark.asyncio
async def test_knowledge_graph_without_content(utility_engine):
    async with utility_engine.client() as client:
        _, data = await graph(client)
    for node in doc_nodes(data).values():
        assert "content" not in node, f"Node {node['id']} should not have content by default"


@pytest.mark.asyncio
async def test_knowledge_graph_with_content_param_0(utility_engine):
    async with utility_engine.client() as client:
        _, data = await graph(client, "?include_content=0")
    for node in doc_nodes(data).values():
        assert "content" not in node, f"Node {node['id']} should not have content with include_content=0"


@pytest.mark.asyncio
async def test_knowledge_graph_with_content_param_1(utility_engine):
    async with utility_engine.client() as client:
        _, data = await graph(client, "?include_content=1")
    nodes = doc_nodes(data)
    assert nodes["kg-alpha"]["content"].strip() == ALPHA_BODY
    assert nodes["kg-beta"]["content"].strip() == BETA_BODY
    assert all("content" not in n for n in data["nodes"] if not n["node_type"].startswith("doc"))


@pytest.mark.asyncio
async def test_invalid_include_content_defaults_to_excluded(utility_engine):
    async with utility_engine.client() as client:
        _, data = await graph(client, "?include_content=yes")
    for node in doc_nodes(data).values():
        assert "content" not in node


@pytest.mark.asyncio
async def test_knowledge_graph_response_size_difference(utility_engine):
    async with utility_engine.client() as client:
        without, data_without = await graph(client, "?include_content=0")
        with_content, data_with = await graph(client, "?include_content=1")
    assert data_without["stats"] == data_with["stats"]
    expected_growth = sum(len(n["content"]) for n in doc_nodes(data_with).values())
    assert expected_growth > 0
    assert len(without.content) < len(with_content.content)
    assert len(with_content.content) - len(without.content) >= expected_growth
    assert b"KG_ALPHA_BODY_MARKER" in with_content.content
    assert b"KG_ALPHA_BODY_MARKER" not in without.content
