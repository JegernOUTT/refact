import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { KnowledgeWorkspace } from './KnowledgeWorkspace';
import type { KnowledgeGraphResponse } from '../../services/refact/types';

const mockGraphData: KnowledgeGraphResponse = {
  nodes: [
    { id: 'doc1', node_type: 'doc_code', label: 'Code Memory 1' },
    { id: 'doc2', node_type: 'doc_decision', label: 'Decision Memory 2' },
    { id: 'doc3', node_type: 'doc_preference', label: 'Preference Memory 3' },
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
let mockError: any = null;

vi.mock('../../services/refact/knowledgeGraphApi', () => ({
  useGetKnowledgeGraphQuery: () => ({
    data: mockGraphResponse,
    isLoading: mockIsLoading,
    error: mockError,
  }),
  useUpdateMemoryMutation: () => [vi.fn(), { isLoading: false }],
  useDeleteMemoryMutation: () => [vi.fn()],
}));

vi.mock('./MemoryListView', () => ({
  MemoryListView: ({ memories, selectedId, onSelectId, linkedIds }: any) => (
    <div data-testid="memory-list">
      <div>Memories: {memories.length}</div>
      <div>Selected: {selectedId || 'none'}</div>
      <div>Linked: {linkedIds.size}</div>
      {memories.map((m: any) => (
        <button key={m.memid} onClick={() => onSelectId(m.memid)}>
          {m.title}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('./KnowledgeGraphView', () => ({
  KnowledgeGraphView: ({ nodes, edges, onSelectId, isLoading }: any) => (
    <div data-testid="graph-view">
      <div>Nodes: {nodes.length}</div>
      <div>Edges: {edges.length}</div>
      <div>Loading: {isLoading ? 'yes' : 'no'}</div>
      {nodes.map((n: any) => (
        <button key={n.id} onClick={() => onSelectId(n.id)}>
          {n.label}
        </button>
      ))}
    </div>
  ),
}));

vi.mock('./MemoryDetailsEditor', () => ({
  MemoryDetailsEditor: ({ memory, onMemoryDeleted }: any) => (
    <div data-testid="details-editor">
      <div>Memory: {memory ? memory.title : 'none'}</div>
      <button onClick={onMemoryDeleted}>Delete</button>
    </div>
  ),
}));

describe('KnowledgeWorkspace', () => {
  beforeEach(() => {
    mockGraphResponse = mockGraphData;
    mockIsLoading = false;
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
    mockError = { message: 'Failed to fetch' };
    render(<KnowledgeWorkspace />);

    expect(screen.getByText('Failed to load knowledge graph')).toBeInTheDocument();
  });

  it('handles empty graph data', () => {
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
});
