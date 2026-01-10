import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { Provider } from 'react-redux';
import { configureStore } from '@reduxjs/toolkit';
import { MemoryDetailsEditor } from './MemoryDetailsEditor';
import { knowledgeGraphApi } from '../../services/refact/knowledgeGraphApi';
import type { KnowledgeMemoRecord } from '../../services/refact/types';

const mockMemory: KnowledgeMemoRecord = {
  memid: 'test-123',
  title: 'Test Memory',
  content: 'Test content',
  tags: ['tag1', 'tag2'],
  kind: 'code',
  file_path: '/path/to/memory.md',
  created: '2024-01-01',
};

const createMockStore = () => {
  return configureStore({
    reducer: {
      [knowledgeGraphApi.reducerPath]: knowledgeGraphApi.reducer,
      config: (state = { lspPort: 8001, apiKey: '' }) => state,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(knowledgeGraphApi.middleware),
  });
};

describe('MemoryDetailsEditor', () => {
  let store: ReturnType<typeof createMockStore>;

  beforeEach(() => {
    store = createMockStore();
  });

  it('renders "No memory selected" when memory is null', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={null} />
      </Provider>
    );

    expect(screen.getByText('No memory selected')).toBeInTheDocument();
  });

  it('displays all memory fields when memory is provided', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    expect(screen.getByDisplayValue('Test Memory')).toBeInTheDocument();
    expect(screen.getByDisplayValue('Test content')).toBeInTheDocument();
    expect(screen.getByText('tag1')).toBeInTheDocument();
    expect(screen.getByText('tag2')).toBeInTheDocument();
    expect(screen.getByText('code')).toBeInTheDocument();
    expect(screen.getByText('2024-01-01')).toBeInTheDocument();
    expect(screen.getByText('/path/to/memory.md')).toBeInTheDocument();
  });

  it('sets isDirty to true when title is edited', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const titleInput = screen.getByDisplayValue('Test Memory');
    fireEvent.change(titleInput, { target: { value: 'Updated Title' } });

    expect(screen.getByText('●')).toBeInTheDocument();
  });

  it('disables save button when not dirty', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const saveButton = screen.getByRole('button', { name: /save/i });
    expect(saveButton).toBeDisabled();
  });

  it('enables save button when dirty', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const titleInput = screen.getByDisplayValue('Test Memory');
    fireEvent.change(titleInput, { target: { value: 'Updated Title' } });

    const saveButton = screen.getByRole('button', { name: /save/i });
    expect(saveButton).not.toBeDisabled();
  });

  it('parses tags correctly on blur', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const tagsInput = screen.getByPlaceholderText('comma, separated, tags');
    fireEvent.change(tagsInput, { target: { value: 'new1, new2, new3' } });
    fireEvent.blur(tagsInput);

    expect(screen.getByText('new1')).toBeInTheDocument();
    expect(screen.getByText('new2')).toBeInTheDocument();
    expect(screen.getByText('new3')).toBeInTheDocument();
  });

  it('removes tag when X is clicked', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const removeButton = screen.getAllByLabelText(/remove/i)[0];
    fireEvent.click(removeButton);

    expect(screen.queryByText('tag1')).not.toBeInTheDocument();
    expect(screen.getByText('tag2')).toBeInTheDocument();
  });

  it('shows delete confirmation dialog when delete is clicked', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const deleteButton = screen.getByRole('button', { name: /delete/i });
    fireEvent.click(deleteButton);

    expect(screen.getByText('Delete Memory')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /archive/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /permanently delete/i })).toBeInTheDocument();
  });

  it('disables save and delete when file_path is missing', () => {
    const memoryWithoutPath = { ...mockMemory, file_path: undefined };

    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={memoryWithoutPath} />
      </Provider>
    );

    expect(screen.getByText(/no file path/i)).toBeInTheDocument();

    const titleInput = screen.getByDisplayValue('Test Memory');
    fireEvent.change(titleInput, { target: { value: 'Updated' } });

    const saveButton = screen.getByRole('button', { name: /save/i });
    const deleteButton = screen.getByRole('button', { name: /delete/i });

    expect(saveButton).toBeDisabled();
    expect(deleteButton).toBeDisabled();
  });

  it('resets draft when memory changes', () => {
    const { rerender } = render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const titleInput = screen.getByDisplayValue('Test Memory');
    fireEvent.change(titleInput, { target: { value: 'Updated Title' } });

    expect(screen.getByText('●')).toBeInTheDocument();

    const newMemory = { ...mockMemory, memid: 'new-id', file_path: '/new/path.md' };
    rerender(
      <Provider store={store}>
        <MemoryDetailsEditor memory={newMemory} />
      </Provider>
    );

    expect(screen.queryByText('●')).not.toBeInTheDocument();
  });

  it('deduplicates tags', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const tagsInput = screen.getByPlaceholderText('comma, separated, tags');
    fireEvent.change(tagsInput, { target: { value: 'tag1, tag1, tag2' } });
    fireEvent.blur(tagsInput);

    const tag1Elements = screen.getAllByText('tag1');
    expect(tag1Elements).toHaveLength(1);
  });

  it('trims and filters empty tags', () => {
    render(
      <Provider store={store}>
        <MemoryDetailsEditor memory={mockMemory} />
      </Provider>
    );

    const tagsInput = screen.getByPlaceholderText('comma, separated, tags');
    fireEvent.change(tagsInput, { target: { value: '  tag1  ,  , tag2  ' } });
    fireEvent.blur(tagsInput);

    expect(screen.getByText('tag1')).toBeInTheDocument();
    expect(screen.getByText('tag2')).toBeInTheDocument();
  });
});
