# Knowledge Graph Cache Invalidation Test

## Issue
After updating or deleting a memory file via `/v1/knowledge/update-memory` or `/v1/knowledge/delete-memory`, the knowledge graph would return stale data because `build_knowledge_graph()` reads from `documents_state.memory_document_map` cache.

## Fix
Added cache invalidation in both handlers:
- `handle_v1_knowledge_update_memory`: Line 111
- `handle_v1_knowledge_delete_memory`: Line 176

Both now call:
```rust
gcx.write().await.documents_state.memory_document_map.remove(&file_path);
```

## Test Scenario

### Setup
1. Create a memory file: `.refact/knowledge/test.md`
2. Load it into cache via any tool that reads it
3. Verify knowledge graph shows original content

### Update Test
1. POST to `/v1/knowledge/update-memory` with new content
2. Cache entry should be invalidated
3. GET `/v1/knowledge-graph` should show updated content (not cached)

### Delete Test
1. DELETE to `/v1/knowledge/delete-memory`
2. Cache entry should be invalidated
3. GET `/v1/knowledge-graph` should not show the deleted file

## Verification
The fix follows the same pattern used in:
- `tool_rm.rs:299-303` - File deletion
- `tool_mv.rs:224` - File move
- `files_in_workspace.rs:931` - Document removal

## Expected Behavior
- ✅ Update memory → UI immediately sees new content
- ✅ Delete memory → UI immediately sees removal
- ✅ No stale data served from cache
- ✅ Performance not impacted (invalidation is O(1))
