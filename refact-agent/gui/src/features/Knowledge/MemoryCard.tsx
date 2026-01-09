import { useState } from 'react';
import { Button, Dialog, Flex } from '@radix-ui/themes';
import type { KnowledgeMemoRecord } from '../../services/refact/types';
import { useDeleteMemoryMutation } from '../../services/refact/knowledgeGraphApi';
import { MemoryEditModal } from './MemoryEditModal';
import styles from './MemoryCard.module.css';

interface MemoryCardProps {
  memory: KnowledgeMemoRecord | null;
  onClose?: () => void;
}

export function MemoryCard({ memory, onClose }: MemoryCardProps) {
  const [isEditOpen, setIsEditOpen] = useState(false);
  const [isDeleteOpen, setIsDeleteOpen] = useState(false);
  const [deleteMemory] = useDeleteMemoryMutation();

  if (!memory) {
    return (
      <div className={styles.container}>
        <p style={{ color: 'var(--gray-10)' }}>Select a memory to view details</p>
      </div>
    );
  }

  const handleDelete = async (archive: boolean) => {
    if (memory.file_path) {
      await deleteMemory({
        file_path: memory.file_path,
        archive,
      });
      setIsDeleteOpen(false);
      onClose?.();
    }
  };

  return (
    <div className={styles.container}>
      <div className={styles.header}>
        <h2 className={styles.title}>{memory.title || 'Untitled'}</h2>
      </div>

      <div className={styles.metadata}>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Kind</span>
          <span className={styles.metaValue}>{memory.kind || 'unknown'}</span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Created</span>
          <span className={styles.metaValue}>{memory.created || '—'}</span>
        </div>
      </div>

      {memory.tags.length > 0 && (
        <div>
          <span className={styles.metaLabel}>Tags</span>
          <div className={styles.tagsContainer}>
            {memory.tags.map((tag) => (
              <span key={tag} className={styles.tag}>
                {tag}
              </span>
            ))}
          </div>
        </div>
      )}

      {memory.content && (
        <div>
          <span className={styles.metaLabel}>Content Preview</span>
          <div className={styles.contentPreview}>
            {memory.content.slice(0, 500)}
            {memory.content.length > 500 && '...'}
          </div>
        </div>
      )}

      <div className={styles.actions}>
        <button
          className={styles.button}
          onClick={() => setIsEditOpen(true)}
        >
          ✏️ Edit
        </button>
        <button
          className={`${styles.button} ${styles.buttonDelete}`}
          onClick={() => setIsDeleteOpen(true)}
        >
          🗑️ Delete
        </button>
      </div>

      {isEditOpen && (
        <MemoryEditModal
          memory={memory}
          isOpen={isEditOpen}
          onClose={() => setIsEditOpen(false)}
        />
      )}

      {isDeleteOpen && (
        <Dialog.Root open={isDeleteOpen} onOpenChange={setIsDeleteOpen}>
          <Dialog.Content>
            <Dialog.Title>Delete Memory</Dialog.Title>
            <Flex direction="column" gap="3">
              <p>What would you like to do?</p>
              <Flex gap="2" justify="end">
                <Button
                  variant="outline"
                  onClick={() => setIsDeleteOpen(false)}
                >
                  Cancel
                </Button>
                <Button
                  color="yellow"
                  onClick={() => handleDelete(true)}
                >
                  Archive
                </Button>
                <Button
                  color="red"
                  onClick={() => handleDelete(false)}
                >
                  Permanently Delete
                </Button>
              </Flex>
            </Flex>
          </Dialog.Content>
        </Dialog.Root>
      )}
    </div>
  );
}
