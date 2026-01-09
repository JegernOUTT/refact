import { useState } from 'react';
import { Dialog, Button, Flex, TextField } from '@radix-ui/themes';
import type { KnowledgeMemoRecord } from '../../services/refact/types';
import { useUpdateMemoryMutation } from '../../services/refact/knowledgeGraphApi';

interface MemoryEditModalProps {
  memory: KnowledgeMemoRecord;
  isOpen: boolean;
  onClose: () => void;
}

export function MemoryEditModal({
  memory,
  isOpen,
  onClose,
}: MemoryEditModalProps) {
  const [formData, setFormData] = useState({
    title: memory.title || '',
    content: memory.content || '',
    tags: memory.tags.join(', '),
    kind: memory.kind || 'code',
  });
  const [updateMemory, { isLoading }] = useUpdateMemoryMutation();

  const handleSubmit = async () => {
    if (memory.file_path) {
      await updateMemory({
        file_path: memory.file_path,
        title: formData.title,
        content: formData.content,
        tags: formData.tags.split(',').map(t => t.trim()).filter(Boolean),
        kind: formData.kind,
        filenames: memory.file_path ? [memory.file_path] : [],
      });
      onClose();
    }
  };

  return (
    <Dialog.Root open={isOpen} onOpenChange={onClose}>
      <Dialog.Content>
        <Dialog.Title>Edit Memory</Dialog.Title>
        <Flex direction="column" gap="3">
          <div>
            <label>Title</label>
            <TextField.Root
              value={formData.title}
              onChange={(e) =>
                setFormData({ ...formData, title: e.target.value })
              }
              placeholder="Memory title"
            />
          </div>

          <div>
            <label>Kind</label>
            <select
              value={formData.kind}
              onChange={(e) =>
                setFormData({ ...formData, kind: e.target.value })
              }
            >
              <option>code</option>
              <option>decision</option>
              <option>trajectory</option>
              <option>preference</option>
            </select>
          </div>

          <div>
            <label>Tags (comma-separated)</label>
            <TextField.Root
              value={formData.tags}
              onChange={(e) =>
                setFormData({ ...formData, tags: e.target.value })
              }
              placeholder="python, testing, bug"
            />
          </div>

          <div>
            <label>Content</label>
            <textarea
              value={formData.content}
              onChange={(e) =>
                setFormData({ ...formData, content: e.target.value })
              }
              rows={8}
              style={{
                width: '100%',
                padding: 'var(--space-2)',
                borderRadius: 'var(--radius-2)',
                border: '1px solid var(--gray-a7)',
                fontFamily: 'var(--font-mono)',
              }}
            />
          </div>

          <Flex gap="2" justify="end">
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={handleSubmit} disabled={isLoading}>
              {isLoading ? 'Saving...' : 'Save'}
            </Button>
          </Flex>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
}
