import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { KnowledgeWorkspace } from './KnowledgeWorkspace';
import type { KnowledgeGraphResponse } from '../../services/refact/types';

const mockGraphData: KnowledgeGraphResponse = {
  nodes: [
    { 
      id: 'doc1', 
      node_type: 'doc_code', 
      label: 'Code Memory 1',
      title: 'Code Memory 1',
      content: 'This is code memory content',
      tags: ['rust', 'backend'],
      created: '2024-01-10T10:00:00Z',
      file_path: '/path/to/memory1.md',
      kind: 'code'
    },
    { 
      id: 'doc2', 
      node_type: 'doc_decision', 
      label: 'Decision Memory 2',
      title: 'Decision Memory 2',
      content: 'This is decision memory content',
      tags: ['architecture'],
      created: '2024-01-09T10:00:00Z',
      file_path: '/path/to/memory2.md',
      kind: 'decision'
    },
    { 
      id: 'doc3', 
      node_type: 'doc_preference', 
      label: 'Preference Memory 3',
      title: 'Preference Memory 3',
      content: 'This is preference memory content',
      tags: ['style'],
      created: '2024-01-08T10:00:00Z',
      file_path: '/path/to/memory3.md',
      kind: 'preference'
    },
    { id: 'doc4', node_type: 'doc_deprecated', label: 'Deprecated Memory' },
    { id: 'doc5', node_type: 'doc_trajectory', label: 'Trajectory Memory' },
    { id: 'tag1', node_type: 'tag', label: 'Tag Node' },
  ],
  edges: [
    { source: 'doc1', target: 'doc2', edge_type: 'relates_to' },
    { source: 'doc2', target: 'doc3', edge_type: 'relates_to' },
    { source: 'doc1', target: 'tag1', edge_type: 'tagged_with' },
  ],
  stats: {
    doc_count: 5,
    tag_count: 1,
    file_count: 0,
    entity_count: 0,
    edge_count: 3,
    active_docs: 3,
    deprecated_docs: 1,
    trajectory_count: 1,
  },
};

let mockGraphResponse: KnowledgeGraphResponse | null = mockGraphData;
let mockIsLoading = false;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let mockError: any = null;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
vi.mock('../../services/refact/knowledgeGraphApi', () => ({
  useGetKnowledgeGraphQuery: () => ({
    data: mockGraphResponse,
    isLoading: mockIsLoading,
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    error: mockError,
  }),
  useUpdateMemoryMutation: () => [vi.fn(), { isLoading: false }],
  useDeleteMemoryMutation: () => [vi.fn()],
}));

vi.mock('./MemoryListView', () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  MemoryListView: ({ memories, selectedId, onSelectId, linkedIds }: any) => (
    <div data-testid="memory-list">
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
      <div>Memories: {memories.length}</div>
      <div>Selected: {selectedId || 'none'}</div>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
      <div>Linked: {linkedIds.size}</div>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-explicit-any */}
      {memories.map((m: any) => (
        // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-return, @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-assignment
        <button key={m.memid} onClick={() => onSelectId(m.memid)}>
          {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
          {m.title}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('./KnowledgeGraphView', () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  KnowledgeGraphView: ({ nodes, edges, onSelectId, isLoading }: any) => (
    <div data-testid="graph-view">
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
      <div>Nodes: {nodes.length}</div>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
      <div>Edges: {edges.length}</div>
      <div>Loading: {isLoading ? 'yes' : 'no'}</div>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-explicit-any */}
      {nodes.map((n: any) => (
        // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-return, @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-assignment
        <button key={n.id} onClick={() => onSelectId(n.id)}>
          {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access */}
          {n.label}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('./MemoryDetailsEditor', () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  MemoryDetailsEditor: ({ memory, onMemoryDeleted }: any) => (
    <div data-testid="details-editor">
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-assignment */}
      <div>Memory: {memory ? memory.title : 'none'}</div>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment */}
      <button onClick={onMemoryDeleted}>Delete</button>
    </div>
  ),
}));

// eslint-disable-next-line @typescript-eslint/no-explicit-any
describe('KnowledgeWorkspace', () => {
  beforeEach(() => {
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    mockGraphResponse = mockGraphData;
    mockIsLoading = false;
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    mockError = null;
  });

  it('renders all three panels', () => {
    render(<KnowledgeWorkspace />);

    expect(screen.getByTestId('memory-list')).toBeInTheDocument();
    expect(screen.getByTestId('graph-view')).toBeInTheDocument();
    expect(screen.getByTestId('details-editor')).toBeInTheDocument();
  });

  it('filters out deprecated and trajectory nodes', () => {
    render(<KnowledgeWorkspace />);

    expect(screen.getByText('Memories: 3')).toBeInTheDocument();
    expect(screen.queryByText('Deprecated Memory')).not.toBeInTheDocument();
    expect(screen.queryByText('Trajectory Memory')).not.toBeInTheDocument();
  });

  it('computes linked IDs correctly', () => {
    render(<KnowledgeWorkspace />);

    expect(screen.getByText('Linked: 3')).toBeInTheDocument();
  });

  it('shows only linked nodes in graph', () => {
    render(<KnowledgeWorkspace />);

    const graphView = screen.getByTestId('graph-view');
    expect(graphView).toHaveTextContent('Nodes: 3');
    expect(graphView).toHaveTextContent('Edges: 2');
  });

  it('syncs selection between list and graph', async () => {
    const user = userEvent.setup();
    render(<KnowledgeWorkspace />);

    const listButton = screen.getAllByRole('button', { name: /Code Memory 1/i })[0];
    await user.click(listButton);

    expect(screen.getByText('Selected: doc1')).toBeInTheDocument();
    expect(screen.getByText('Memory: Code Memory 1')).toBeInTheDocument();
  });

  it('updates editor when selection changes', async () => {
    const user = userEvent.setup();
    render(<KnowledgeWorkspace />);

    const button1 = screen.getAllByRole('button', { name: /Code Memory 1/i })[0];
    await user.click(button1);
    expect(screen.getByText('Memory: Code Memory 1')).toBeInTheDocument();

    const button2 = screen.getAllByRole('button', { name: /Decision Memory 2/i })[0];
    await user.click(button2);
    expect(screen.getByText('Memory: Decision Memory 2')).toBeInTheDocument();
  });

  it('clears selection when memory is deleted', async () => {
    const user = userEvent.setup();
    render(<KnowledgeWorkspace />);

    const selectButton = screen.getAllByRole('button', { name: /Code Memory 1/i })[0];
    await user.click(selectButton);
    expect(screen.getByText('Memory: Code Memory 1')).toBeInTheDocument();

    const deleteButton = screen.getByRole('button', { name: /Delete/i });
    await user.click(deleteButton);

    expect(screen.getByText('Memory: none')).toBeInTheDocument();
    expect(screen.getByText('Selected: none')).toBeInTheDocument();
  });

  it('shows error state when graph fails to load', () => {
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    mockError = { message: 'Failed to fetch' };
    render(<KnowledgeWorkspace />);

    expect(screen.getByText('Failed to load knowledge graph')).toBeInTheDocument();
  });

  it('handles empty graph data', () => {
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    mockGraphResponse = {
      nodes: [],
      edges: [],
      stats: {
        doc_count: 0,
        tag_count: 0,
        file_count: 0,
        entity_count: 0,
        edge_count: 0,
        active_docs: 0,
        deprecated_docs: 0,
        trajectory_count: 0,
      },
    };
    render(<KnowledgeWorkspace />);

    expect(screen.getByText('Memories: 0')).toBeInTheDocument();
    expect(screen.getByText('Nodes: 0')).toBeInTheDocument();
    expect(screen.getByText('Edges: 0')).toBeInTheDocument();
  });

  it('converts graph nodes to memory records', () => {
    render(<KnowledgeWorkspace />);

    expect(screen.getAllByText('Code Memory 1').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Decision Memory 2').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Preference Memory 3').length).toBeGreaterThan(0);
  });

  it('populates memory records with full data from graph nodes', async () => {
    const user = userEvent.setup();
    render(<KnowledgeWorkspace />);

    const button = screen.getAllByRole('button', { name: /Code Memory 1/i })[0];
    await user.click(button);

    expect(screen.getByText('Memory: Code Memory 1')).toBeInTheDocument();
  });

  it('includes plain "doc" node type (without underscore)', () => {
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    mockGraphResponse = {
      nodes: [
        { 
          id: 'doc1', 
          node_type: 'doc', 
          label: 'Plain Doc Memory',
          title: 'Plain Doc Memory',
          content: 'This is a plain doc memory',
          tags: ['test'],
          created: '2024-01-10T10:00:00Z',
          file_path: '/path/to/plain.md',
          kind: 'code'
        },
        { 
          id: 'doc2', 
          node_type: 'doc_code', 
          label: 'Code Memory',
          title: 'Code Memory',
          content: 'This is code memory',
          tags: ['test'],
          created: '2024-01-10T10:00:00Z',
          file_path: '/path/to/code.md',
          kind: 'code'
        },
      ],
      edges: [
        { source: 'doc1', target: 'doc2', edge_type: 'relates_to' },
      ],
      stats: {
        doc_count: 2,
        tag_count: 0,
        file_count: 0,
        entity_count: 0,
        edge_count: 1,
        active_docs: 2,
        deprecated_docs: 0,
        trajectory_count: 0,
      },
    };
    
    render(<KnowledgeWorkspace />);

    // Both memories should be visible
    expect(screen.getByText('Memories: 2')).toBeInTheDocument();
    expect(screen.getAllByText('Plain Doc Memory').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Code Memory').length).toBeGreaterThan(0);
  });
});
