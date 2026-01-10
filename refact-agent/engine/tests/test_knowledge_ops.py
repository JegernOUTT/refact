#!/usr/bin/env python3
"""
Tests for knowledge operations (update/delete memory) endpoints.

Run with:
  python tests/test_knowledge_ops.py

Requires:
  - refact-lsp running on port 8001
  - pip install requests
"""

import sys
import tempfile
import requests
from pathlib import Path

LSP_URL = "http://127.0.0.1:8001"


def test_update_memory():
    """Test updating a memory file"""
    print("\n=== Test: Update memory ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        test_file = knowledge_dir / "test_memory.md"
        initial_content = """---
id: "test-123"
title: "Original Title"
tags: ["test", "original"]
kind: code
created: 2024-01-01
updated: 2024-01-01
filenames: []
links: []
status: active
---

Original content here."""
        
        test_file.write_text(initial_content)
        
        update_payload = {
            "file_path": str(test_file),
            "title": "Updated Title",
            "content": "Updated content here.",
            "tags": ["test", "updated"],
            "kind": "decision",
            "filenames": ["src/main.rs"]
        }
        
        try:
            response = requests.post(
                f"{LSP_URL}/v1/knowledge/update-memory",
                json=update_payload,
                timeout=10
            )
            
            if response.status_code != 200:
                print(f"✗ Expected 200, got {response.status_code}: {response.text}")
                return False
            
            result = response.json()
            if not result.get("success"):
                print(f"✗ Expected success=true, got {result}")
                return False
            
            updated_content = test_file.read_text()
            checks = [
                ("Updated Title" in updated_content, "Title updated"),
                ("Updated content here." in updated_content, "Content updated"),
                ("updated" in updated_content, "Tags updated"),
                ("decision" in updated_content, "Kind updated"),
                ("src/main.rs" in updated_content, "Filenames updated"),
            ]
            
            all_passed = True
            for passed, desc in checks:
                status = "✓" if passed else "✗"
                print(f"  {status} {desc}")
                all_passed = all_passed and passed
            
            return all_passed
            
        except Exception as e:
            print(f"✗ Error: {e}")
            return False


def test_update_memory_not_found():
    """Test updating a non-existent memory file"""
    print("\n=== Test: Update non-existent memory ===")
    
    update_payload = {
        "file_path": "/nonexistent/path/memory.md",
        "content": "Some content",
        "tags": []
    }
    
    try:
        response = requests.post(
            f"{LSP_URL}/v1/knowledge/update-memory",
            json=update_payload,
            timeout=10
        )
        
        if response.status_code == 404:
            print("✓ Correctly returned 404 for non-existent file")
            return True
        else:
            print(f"✗ Expected 404, got {response.status_code}")
            return False
            
    except Exception as e:
        print(f"✗ Error: {e}")
        return False


def test_delete_memory_permanent():
    """Test permanently deleting a memory file"""
    print("\n=== Test: Delete memory permanently ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        test_file = knowledge_dir / "test_delete.md"
        test_file.write_text("""---
id: "test-delete"
title: "To Delete"
---

Content to delete.""")
        
        if not test_file.exists():
            print("✗ Test file was not created")
            return False
        
        delete_payload = {
            "file_path": str(test_file),
            "archive": False
        }
        
        try:
            response = requests.post(
                f"{LSP_URL}/v1/knowledge/delete-memory",
                json=delete_payload,
                timeout=10
            )
            
            if response.status_code != 200:
                print(f"✗ Expected 200, got {response.status_code}: {response.text}")
                return False
            
            result = response.json()
            if not result.get("success"):
                print(f"✗ Expected success=true, got {result}")
                return False
            
            if test_file.exists():
                print("✗ File still exists after deletion")
                return False
            
            print("✓ File deleted successfully")
            return True
            
        except Exception as e:
            print(f"✗ Error: {e}")
            return False


def test_delete_memory_archive():
    """Test archiving a memory file"""
    print("\n=== Test: Archive memory ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        test_file = knowledge_dir / "test_archive.md"
        test_file.write_text("""---
id: "test-archive"
title: "To Archive"
---

Content to archive.""")
        
        if not test_file.exists():
            print("✗ Test file was not created")
            return False
        
        delete_payload = {
            "file_path": str(test_file),
            "archive": True
        }
        
        try:
            response = requests.post(
                f"{LSP_URL}/v1/knowledge/delete-memory",
                json=delete_payload,
                timeout=10
            )
            
            if response.status_code != 200:
                print(f"✗ Expected 200, got {response.status_code}: {response.text}")
                return False
            
            result = response.json()
            if not result.get("success"):
                print(f"✗ Expected success=true, got {result}")
                return False
            
            if test_file.exists():
                print("✗ Original file still exists")
                return False
            
            archive_dir = knowledge_dir / "archive"
            archived_file = archive_dir / "test_archive.md"
            if not archived_file.exists():
                print(f"✗ Archived file not found at {archived_file}")
                return False
            
            print("✓ File archived successfully")
            return True
            
        except Exception as e:
            print(f"✗ Error: {e}")
            return False


def test_delete_memory_not_found():
    """Test deleting a non-existent memory file"""
    print("\n=== Test: Delete non-existent memory ===")
    
    delete_payload = {
        "file_path": "/nonexistent/path/memory.md",
        "archive": False
    }
    
    try:
        response = requests.delete(
            f"{LSP_URL}/v1/knowledge/delete-memory",
            json=delete_payload,
            timeout=10
        )
        
        if response.status_code == 404:
            print("✓ Correctly returned 404 for non-existent file")
            return True
        else:
            print(f"✗ Expected 404, got {response.status_code}")
            return False
            
    except Exception as e:
        print(f"✗ Error: {e}")
        return False


def test_path_traversal_attack():
    """Test that path traversal attacks are blocked"""
    print("\n=== Test: Path traversal attack (../../../etc/passwd) ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        attack_paths = [
            "../../../etc/passwd",
            "../../.ssh/id_rsa",
            knowledge_dir / ".." / ".." / "sensitive.md",
        ]
        
        all_blocked = True
        for attack_path in attack_paths:
            update_payload = {
                "file_path": str(attack_path),
                "content": "Malicious content",
                "tags": []
            }
            
            try:
                response = requests.post(
                    f"{LSP_URL}/v1/knowledge/update-memory",
                    json=update_payload,
                    timeout=10
                )
                
                if response.status_code in [403, 404]:
                    print(f"  ✓ Blocked: {attack_path}")
                else:
                    print(f"  ✗ Not blocked (got {response.status_code}): {attack_path}")
                    all_blocked = False
                    
            except Exception as e:
                print(f"  ✗ Error testing {attack_path}: {e}")
                all_blocked = False
        
        return all_blocked


def test_symlink_escape_attack():
    """Test that symlink escapes are blocked"""
    print("\n=== Test: Symlink escape attack ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        outside_dir = Path(tmpdir) / "outside"
        outside_dir.mkdir(exist_ok=True)
        
        target_file = outside_dir / "secret.md"
        target_file.write_text("Secret content")
        
        symlink_path = knowledge_dir / "escape.md"
        try:
            symlink_path.symlink_to(target_file)
        except OSError:
            print("  ⚠ Cannot create symlinks (Windows?), skipping test")
            return True
        
        update_payload = {
            "file_path": str(symlink_path),
            "content": "Trying to overwrite via symlink",
            "tags": []
        }
        
        try:
            response = requests.post(
                f"{LSP_URL}/v1/knowledge/update-memory",
                json=update_payload,
                timeout=10
            )
            
            if response.status_code == 403:
                print("✓ Symlink escape blocked with 403 Forbidden")
                return True
            else:
                print(f"✗ Expected 403, got {response.status_code}")
                return False
                
        except Exception as e:
            print(f"✗ Error: {e}")
            return False


def test_non_markdown_file_rejected():
    """Test that non-.md files are rejected"""
    print("\n=== Test: Non-.md file rejection ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        test_files = [
            knowledge_dir / "test.txt",
            knowledge_dir / "test.py",
            knowledge_dir / "test.sh",
            knowledge_dir / "test",
        ]
        
        all_rejected = True
        for test_file in test_files:
            test_file.write_text("Some content")
            
            update_payload = {
                "file_path": str(test_file),
                "content": "Content",
                "tags": []
            }
            
            try:
                response = requests.post(
                    f"{LSP_URL}/v1/knowledge/update-memory",
                    json=update_payload,
                    timeout=10
                )
                
                if response.status_code == 400:
                    print(f"  ✓ Rejected: {test_file.name}")
                else:
                    print(f"  ✗ Not rejected (got {response.status_code}): {test_file.name}")
                    all_rejected = False
                    
            except Exception as e:
                print(f"  ✗ Error testing {test_file.name}: {e}")
                all_rejected = False
        
        return all_rejected


def test_path_outside_knowledge_dir():
    """Test that paths outside knowledge directory are rejected"""
    print("\n=== Test: Path outside knowledge directory ===")
    
    with tempfile.TemporaryDirectory() as tmpdir:
        knowledge_dir = Path(tmpdir) / ".refact" / "knowledge"
        knowledge_dir.mkdir(parents=True, exist_ok=True)
        
        outside_file = Path(tmpdir) / "outside.md"
        outside_file.write_text("Outside content")
        
        update_payload = {
            "file_path": str(outside_file),
            "content": "Trying to update outside file",
            "tags": []
        }
        
        try:
            response = requests.post(
                f"{LSP_URL}/v1/knowledge/update-memory",
                json=update_payload,
                timeout=10
            )
            
            if response.status_code == 403:
                print("✓ Path outside knowledge dir blocked with 403 Forbidden")
                return True
            else:
                print(f"✗ Expected 403, got {response.status_code}")
                return False
                
        except Exception as e:
            print(f"✗ Error: {e}")
            return False


def main():
    print("=" * 60)
    print("Knowledge Operations Tests")
    print("=" * 60)
    print(f"Testing against: {LSP_URL}")
    
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
    results.append(("Update memory", test_update_memory()))
    results.append(("Update non-existent memory", test_update_memory_not_found()))
    results.append(("Delete memory permanently", test_delete_memory_permanent()))
    results.append(("Archive memory", test_delete_memory_archive()))
    results.append(("Delete non-existent memory", test_delete_memory_not_found()))
    
    print("\n" + "=" * 60)
    print("Security Tests")
    print("=" * 60)
    results.append(("Path traversal attack blocked", test_path_traversal_attack()))
    results.append(("Symlink escape blocked", test_symlink_escape_attack()))
    results.append(("Non-.md files rejected", test_non_markdown_file_rejected()))
    results.append(("Paths outside knowledge dir blocked", test_path_outside_knowledge_dir()))
    
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
