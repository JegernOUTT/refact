# KnowledgeGraphView Component

## Overview

A simplified, pure graph renderer for displaying memory nodes and their relationships. This component focuses exclusively on document-to-document connections without filters, modes, or UI controls.

## Features

- **Doc-only rendering**: Only displays `doc_code`, `doc_decision`, `doc_preference`, `doc_pattern`, and `doc_lesson` nodes
- **Edge filtering**: Automatically filters edges to only show connections between doc nodes
- **Node coloring by kind**:
  - `doc_code` → Blue (#3B82F6)
  - `doc_decision` → Purple (#8B5CF6)
  - `doc_preference` → Green (#10B981)
  - `doc_pattern` → Amber (#F59E0B)
  - `doc_lesson` → Cyan (#06B6D4)
- **Node sizing by degree**: More connected nodes appear larger
- **Interactive selection**: Click nodes to select, click background to deselect
- **Force-directed layout**: Uses fcose algorithm for optimal link visualization
- **Zoom-based labels**: Node labels appear on hover or when zoomed in
- **Empty state handling**: Shows "No linked memories" when no nodes available

## Props

```typescript
interface KnowledgeGraphViewProps {
  nodes: KnowledgeGraphNode[];      // All nodes from API
  edges: KnowledgeGraphEdge[];      // All edges from API
  selectedId: string | null;        // Currently selected node ID
  onSelectId: (id: string | null) => void; // Selection callback
  isLoading?: boolean;              // Show loading state
}
```

## Usage

```typescript
import { KnowledgeGraphView } from './features/Knowledge';

function MyComponent() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const { data: graph, isLoading } = useGetKnowledgeGraphQuery();

  return (
    <KnowledgeGraphView
      nodes={graph?.nodes ?? []}
      edges={graph?.edges ?? []}
      selectedId={selectedId}
      onSelectId={setSelectedId}
      isLoading={isLoading}
    />
  );
}
```

## Filtering Behavior

### Nodes
- **Included**: `doc_code`, `doc_decision`, `doc_preference`, `doc_pattern`, `doc_lesson`
- **Excluded**: `doc_deprecated`, `doc_trajectory`, `tag`, `file`, `entity`, and any other types

### Edges
- Only edges where both source AND target are included doc nodes
- All other edges are filtered out

## Layout Configuration

Uses fcose (force-directed) layout with these parameters:
- `idealEdgeLength`: 120px
- `nodeRepulsion`: 5000
- `edgeElasticity`: 0.5
- `animationDuration`: 500ms

## Styling

- Node size: Maps degree (1-20) to size (30-60px)
- Selected node: 3px purple border + slightly larger
- Edges: Gray (#9CA3AF) with arrow, 40% opacity
- Labels: Hidden by default, shown on hover or zoom > 1.2

## Differences from KnowledgeGraph.tsx

| Feature | KnowledgeGraph | KnowledgeGraphView |
|---------|----------------|-------------------|
| Filter UI | ✅ Checkboxes for kinds/statuses | ❌ None |
| Sidebar | ✅ Stats + node details | ❌ None |
| Focus mode | ✅ 1-hop/2-hop traversal | ❌ None |
| Overview mode | ✅ Concentric layout | ❌ None |
| Node groups | ✅ Tags/files/entities toggles | ❌ None |
| Deprecated nodes | ✅ Optional display | ❌ Always hidden |
| Trajectory nodes | ✅ Optional display | ❌ Always hidden |
| Layout | Concentric or fcose | fcose only |

## Testing

Run tests:
```bash
npm test -- --run src/features/Knowledge/KnowledgeGraphView.test.tsx
```

Test coverage:
- ✅ Empty state rendering
- ✅ Node and edge rendering
- ✅ Non-doc node filtering
- ✅ Edge filtering (doc-doc only)
- ✅ Deprecated/trajectory exclusion
- ✅ Empty edges handling
- ✅ Loading state
- ✅ All doc node types
- ✅ Selection callback

## Implementation Notes

- Uses `react-cytoscapejs` for graph rendering
- Cytoscape event handlers have ESLint suppressions due to library's poor TypeScript support
- Layout runs on every element change (nodes/edges update)
- Selected node auto-centers in viewport
- No console errors on unmount (layout properly stopped)
