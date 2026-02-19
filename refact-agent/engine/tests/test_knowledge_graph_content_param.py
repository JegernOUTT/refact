#!/usr/bin/env python3
"""
Test knowledge graph content parameter.

Run with:
  python tests/test_knowledge_graph_content_param.py

Requires:
  - refact-lsp running on port 8001
  - pip install requests
"""
"""
NOTE: If tests fail with "content should not be None", the server needs to be
restarted to pick up the new code changes:
  1. Stop the running refact-lsp server
  2. Rebuild: cargo build --release
  3. Restart the server
  4. Run tests again
"""

import requests
import sys

LSP_URL = "http://127.0.0.1:8001"


def test_knowledge_graph_without_content():
    """Test that default behavior excludes content"""
    print("\n=== Test: Default behavior excludes content ===")
    response = requests.get(
        f"{LSP_URL}/v1/knowledge-graph",
        timeout=10
    )
    assert response.status_code == 200, f"Expected 200, got {response.status_code}"
    
    data = response.json()
    assert "nodes" in data
    assert "edges" in data
    assert "stats" in data
    
    doc_nodes = [n for n in data["nodes"] if n["node_type"].startswith("doc")]
    print(f"  Found {len(doc_nodes)} doc nodes")
    
    if doc_nodes:
        for node in doc_nodes:
            assert node.get("content") is None, f"Node {node['id']} should not have content by default"
        print(f"  ✓ All doc nodes have content=None by default")
    else:
        print(f"  ⚠ No doc nodes found (empty knowledge base)")
    
    return True


def test_knowledge_graph_with_content_param_0():
    """Test that include_content=0 excludes content"""
    print("\n=== Test: include_content=0 excludes content ===")
    response = requests.get(
        f"{LSP_URL}/v1/knowledge-graph?include_content=0",
        timeout=10
    )
    assert response.status_code == 200, f"Expected 200, got {response.status_code}"
    
    data = response.json()
    doc_nodes = [n for n in data["nodes"] if n["node_type"].startswith("doc")]
    
    if doc_nodes:
        for node in doc_nodes:
            assert node.get("content") is None, f"Node {node['id']} should not have content with include_content=0"
        print(f"  ✓ All {len(doc_nodes)} doc nodes have content=None")
    else:
        print(f"  ⚠ No doc nodes found")
    
    return True


def test_knowledge_graph_with_content_param_1():
    """Test that include_content=1 includes content"""
    print("\n=== Test: include_content=1 includes content ===")
    response = requests.get(
        f"{LSP_URL}/v1/knowledge-graph?include_content=1",
        timeout=10
    )
    assert response.status_code == 200, f"Expected 200, got {response.status_code}"
    
    data = response.json()
    doc_nodes = [n for n in data["nodes"] if n["node_type"].startswith("doc")]
    
    if doc_nodes:
        for node in doc_nodes:
            assert "content" in node, f"Node {node['id']} should have content field with include_content=1"
            if node["content"] is not None:
                print(f"  ✓ Node {node['id']}: content length = {len(node['content'])} chars")
        print(f"  ✓ All {len(doc_nodes)} doc nodes have content field")
    else:
        print(f"  ⚠ No doc nodes found")
    
    return True


def test_knowledge_graph_response_size_difference():
    """Test that response without content is significantly smaller"""
    print("\n=== Test: Response size difference ===")
    response_without = requests.get(
        f"{LSP_URL}/v1/knowledge-graph?include_content=0",
        timeout=10
    )
    response_with = requests.get(
        f"{LSP_URL}/v1/knowledge-graph?include_content=1",
        timeout=10
    )
    
    assert response_without.status_code == 200
    assert response_with.status_code == 200
    
    size_without = len(response_without.content)
    size_with = len(response_with.content)
    
    print(f"  Response without content: {size_without:,} bytes")
    print(f"  Response with content: {size_with:,} bytes")
    
    data_with = response_with.json()
    doc_nodes = [n for n in data_with["nodes"] if n["node_type"].startswith("doc")]
    
    if doc_nodes:
        assert size_without < size_with, "Response without content should be smaller"
        reduction_percent = ((size_with - size_without) / size_with) * 100
        print(f"  ✓ Size reduction: {reduction_percent:.1f}% ({size_with:,} → {size_without:,} bytes)")
    else:
        print(f"  ⚠ No doc nodes found, cannot measure size difference")
    
    return True


def main():
    print("=" * 60)
    print("Knowledge Graph Content Parameter Tests")
    print("=" * 60)
    print(f"Testing against: {LSP_URL}")

    # Check if server is running
    try:
        response = requests.get(f"{LSP_URL}/v1/ping", timeout=2)
        if response.status_code != 200:
            print(f"\n✗ Server not responding correctly at {LSP_URL}")
            sys.exit(1)
    except Exception as e:
        print(f"\n✗ Cannot connect to server at {LSP_URL}: {e}")
        print("  Make sure refact-lsp is running with: cargo run")
        sys.exit(1)

    print("✓ Server is running\n")

    results = []

    # Run tests
    try:
        results.append(("Default excludes content", test_knowledge_graph_without_content()))
    except Exception as e:
        print(f"✗ Error: {e}")
        results.append(("Default excludes content", False))

    try:
        results.append(("include_content=0 excludes", test_knowledge_graph_with_content_param_0()))
    except Exception as e:
        print(f"✗ Error: {e}")
        results.append(("include_content=0 excludes", False))

    try:
        results.append(("include_content=1 includes", test_knowledge_graph_with_content_param_1()))
    except Exception as e:
        print(f"✗ Error: {e}")
        results.append(("include_content=1 includes", False))

    try:
        results.append(("Response size difference", test_knowledge_graph_response_size_difference()))
    except Exception as e:
        print(f"✗ Error: {e}")
        results.append(("Response size difference", False))

    # Summary
    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)

    passed = sum(1 for _, r in results if r)
    total = len(results)

    for name, result in results:
        status = "✓ PASS" if result else "✗ FAIL"
        print(f"  {status}: {name}")

    print(f"\nTotal: {passed}/{total} passed")

    sys.exit(0 if passed == total else 1)


if __name__ == "__main__":
    main()
