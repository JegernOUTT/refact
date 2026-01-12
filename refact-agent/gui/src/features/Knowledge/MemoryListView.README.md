# MemoryListView Component

A card-based grid view for displaying knowledge memories with responsive layout and interactive selection.

## Features

- **Responsive Grid**: 2-3 column layout that adapts to screen size
- **Kind Badges**: Color-coded icons matching KnowledgeGraph colors
- **Selection State**: Visual highlight for selected cards
- **Link Indicators**: 🔗 badge for memories that appear in graph edges
- **Tag Display**: Shows up to 3 tag dots with "+N" overflow indicator
- **Empty State**: Friendly message when no memories available
- **Accessibility**: Keyboard-focusable cards with ARIA labels

## Usage

```tsx
import { MemoryListView } from './features/Knowledge';

function MyComponent() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const linkedIds = new Set(['mem-1', 'mem-3']);
  
  const filteredMemories = memories.filter(
    m => m.kind !== 'deprecated' && m.kind !== 'trajectory'
  );

  return (
    <MemoryListView
      memories={filteredMemories}
      selectedId={selectedId}
      onSelectId={setSelectedId}
      linkedIds={linkedIds}
    />
  );
}
```

## Props

| Prop | Type | Description |
|------|------|-------------|
| `memories` | `KnowledgeMemoRecord[]` | Array of memory records to display |
| `selectedId` | `string \| null` | ID of currently selected memory |
| `onSelectId` | `(id: string) => void` | Callback when card is clicked |
| `linkedIds` | `Set<string>` | Set of memory IDs that appear in graph edges |

## Kind Colors

Matches KnowledgeGraph.tsx colors:

- 📄 **code** - Blue (#3B82F6)
- 🎯 **decision** - Purple (#8B5CF6)
- ⭐ **preference** - Green (#10B981)
- 🔄 **pattern** - Amber (#F59E0B)
- 📚 **lesson** - Cyan (#06B6D4)

## Layout

- **Mobile/Small**: 2 columns (min-width: 768px)
- **Desktop**: 3 columns (min-width: 1200px)
- **Card min-height**: 120px
- **Gap**: `var(--space-3)` (12px)

## Styling

Uses Radix design tokens exclusively:
- Colors: `--color-panel`, `--gray-a7`, `--accent-9`
- Spacing: `--space-1` through `--space-4`
- Radius: `--radius-1`, `--radius-2`
- Transitions: 150ms ease

## Accessibility

- Cards are `<button>` elements for keyboard navigation
- `aria-pressed` indicates selection state
- `aria-label` on kind badges and link indicators
- Focus ring with `outline: 2px solid var(--accent-9)`
- Title tooltips on tag dots (via `title` attribute)
