import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryListView } from './MemoryListView';
import type { KnowledgeMemoRecord } from '../../services/refact/types';

const mockMemories: KnowledgeMemoRecord[] = [
  {
    memid: 'mem-1',
    tags: ['tag1', 'tag2'],
    content: 'Test content 1',
    title: 'Test Memory 1',
    kind: 'code',
  },
  {
    memid: 'mem-2',
    tags: ['tag3', 'tag4', 'tag5', 'tag6'],
    content: 'Test content 2',
    title: 'Test Memory 2',
    kind: 'decision',
  },
  {
    memid: 'mem-3',
    tags: [],
    content: 'Test content 3',
    title: 'Very Long Title That Should Be Truncated After Two Lines Of Text',
    kind: 'preference',
  },
];

describe('MemoryListView', () => {
  it('renders empty state when no memories', () => {
    render(
      <MemoryListView
        memories={[]}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('No memories to display')).toBeInTheDocument();
  });

  it('renders card grid with memories', () => {
    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('Test Memory 1')).toBeInTheDocument();
    expect(screen.getByText('Test Memory 2')).toBeInTheDocument();
    expect(screen.getByText(/Very Long Title/)).toBeInTheDocument();
  });

  it('calls onSelectId when card is clicked', async () => {
    const user = userEvent.setup();
    const onSelectId = vi.fn();

    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={onSelectId}
        linkedIds={new Set()}
      />
    );

    const card = screen.getByText('Test Memory 1').closest('button');
    expect(card).toBeInTheDocument();

    if (card) {
      await user.click(card);
      expect(onSelectId).toHaveBeenCalledWith('mem-1');
    }
  });

  it('highlights selected card', () => {
    render(
      <MemoryListView
        memories={mockMemories}
        selectedId="mem-2"
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    const selectedCard = screen.getByText('Test Memory 2').closest('button');
    expect(selectedCard?.className).toContain('selected');
  });

  it('shows link badge for linked memories', () => {
    const linkedIds = new Set(['mem-1', 'mem-3']);

    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={linkedIds}
      />
    );

    const linkBadges = screen.getAllByLabelText('Linked in graph');
    expect(linkBadges).toHaveLength(2);
  });

  it('displays kind badges with correct icons', () => {
    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByLabelText('Kind: code')).toBeInTheDocument();
    expect(screen.getByLabelText('Kind: decision')).toBeInTheDocument();
    expect(screen.getByLabelText('Kind: preference')).toBeInTheDocument();
  });

  it('shows tag dots and +N indicator', () => {
    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('+1')).toBeInTheDocument();
  });

  it('capitalizes kind in metadata', () => {
    render(
      <MemoryListView
        memories={mockMemories}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('Code')).toBeInTheDocument();
    expect(screen.getByText('Decision')).toBeInTheDocument();
    expect(screen.getByText('Preference')).toBeInTheDocument();
  });

  it('handles memory without title', () => {
    const memoryWithoutTitle: KnowledgeMemoRecord = {
      memid: 'mem-4',
      tags: [],
      content: 'Content',
      kind: 'code',
    };

    render(
      <MemoryListView
        memories={[memoryWithoutTitle]}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('Untitled')).toBeInTheDocument();
  });

  it('handles memory without kind', () => {
    const memoryWithoutKind: KnowledgeMemoRecord = {
      memid: 'mem-5',
      tags: [],
      content: 'Content',
      title: 'Test',
    };

    render(
      <MemoryListView
        memories={[memoryWithoutKind]}
        selectedId={null}
        onSelectId={vi.fn()}
        linkedIds={new Set()}
      />
    );

    expect(screen.getByText('Code')).toBeInTheDocument();
  });
});
