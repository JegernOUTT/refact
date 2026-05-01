import { useState, useEffect } from "react";
import { Button, Dialog, Flex, TextField } from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import type { KnowledgeMemoRecord } from "../../services/refact/types";
import {
  useUpdateMemoryMutation,
  useDeleteMemoryMutation,
} from "../../services/refact/knowledgeGraphApi";
import styles from "./MemoryDetailsEditor.module.css";

interface MemoryDetailsEditorProps {
  memory: KnowledgeMemoRecord | null;
  onMemoryUpdated?: () => void;
  onMemoryDeleted?: () => void;
}

interface DraftMemory {
  title: string;
  content: string;
  tags: string[];
  kind: string;
}

export function MemoryDetailsEditor({
  memory,
  onMemoryUpdated,
  onMemoryDeleted,
}: MemoryDetailsEditorProps) {
  const [draft, setDraft] = useState<DraftMemory>({
    title: "",
    content: "",
    tags: [],
    kind: "code",
  });
  const [isDirty, setIsDirty] = useState(false);
  const [isDeleteOpen, setIsDeleteOpen] = useState(false);
  const [showDiscardDialog, setShowDiscardDialog] = useState(false);
  const [tagsInput, setTagsInput] = useState("");

  const [updateMemory, { isLoading: isSaving }] = useUpdateMemoryMutation();
  const [deleteMemory] = useDeleteMemoryMutation();

  useEffect(() => {
    if (!memory) {
      setDraft({ title: "", content: "", tags: [], kind: "code" });
      setIsDirty(false);
      setTagsInput("");
    } else {
      setDraft({
        title: memory.title ?? "",
        content: memory.content,
        tags: memory.tags,
        kind: memory.kind ?? "code",
      });
      setIsDirty(false);
      setTagsInput(memory.tags.join(", "));
    }
  }, [memory]);

  const handleFieldChange = (
    field: keyof DraftMemory,
    value: string | string[],
  ) => {
    setDraft((prev) => ({ ...prev, [field]: value }));
    setIsDirty(true);
  };

  const parseTags = (input: string): string[] => {
    return input
      .split(/[,\n]/)
      .map((tag) => tag.trim())
      .filter((tag) => tag.length > 0)
      .filter((tag, index, self) => self.indexOf(tag) === index);
  };

  const handleTagsBlur = () => {
    const parsed = parseTags(tagsInput);
    handleFieldChange("tags", parsed);
  };

  const handleRemoveTag = (tagToRemove: string) => {
    const newTags = draft.tags.filter((tag) => tag !== tagToRemove);
    handleFieldChange("tags", newTags);
    setTagsInput(newTags.join(", "));
  };

  const handleSave = () => {
    if (!memory?.file_path || !draft.title || !draft.content) return;

    void updateMemory({
      file_path: memory.file_path,
      title: draft.title,
      content: draft.content,
      tags: draft.tags,
      kind: draft.kind,
      filenames: [memory.file_path],
    })
      .unwrap()
      .then(() => {
        setIsDirty(false);
        onMemoryUpdated?.();
      })
      .catch((_error: unknown) => {
        // Error is handled by RTK Query
      });
  };

  const handleDelete = (archive: boolean) => {
    if (!memory?.file_path) return;

    void deleteMemory({
      file_path: memory.file_path,
      archive,
    })
      .unwrap()
      .then(() => {
        setIsDeleteOpen(false);
        onMemoryDeleted?.();
      })
      .catch((_error: unknown) => {
        // Error is handled by RTK Query
      });
  };

  const handleDiscardChanges = () => {
    setShowDiscardDialog(false);
    setIsDirty(false);
  };

  if (!memory) {
    return (
      <div className={styles.container}>
        <p className={styles.emptyState}>No memory selected</p>
      </div>
    );
  }

  const canSave = memory.file_path && isDirty && draft.title && draft.content;
  const canDelete = memory.file_path;

  return (
    <div className={styles.container}>
      <div className={styles.scrollArea}>
        <div className={styles.field}>
          <label className={styles.label}>
            TITLE {isDirty && <span className={styles.dirtyIndicator}>●</span>}
          </label>
          <TextField.Root
            value={draft.title}
            onChange={(e) => handleFieldChange("title", e.target.value)}
            placeholder="Untitled"
            className={styles.input}
          />
        </div>

        <div className={styles.field}>
          <label className={styles.label}>KIND</label>
          <div className={styles.readOnlyValue}>{draft.kind}</div>
        </div>

        <div className={styles.field}>
          <label className={styles.label}>CREATED</label>
          <div className={styles.readOnlyValue}>{memory.created ?? "—"}</div>
        </div>

        <div className={styles.field}>
          <label className={styles.label}>TAGS</label>
          {draft.tags.length > 0 && (
            <div className={styles.tagsContainer}>
              {draft.tags.map((tag) => (
                <span key={tag} className={styles.tag}>
                  {tag}
                  <button
                    className={styles.tagRemove}
                    onClick={() => handleRemoveTag(tag)}
                    aria-label={`Remove ${tag}`}
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
          <TextField.Root
            value={tagsInput}
            onChange={(e) => setTagsInput(e.target.value)}
            onBlur={handleTagsBlur}
            placeholder="comma, separated, tags"
            className={styles.input}
          />
        </div>

        <div className={styles.field}>
          <label className={styles.label}>FILE PATH</label>
          <div className={styles.readOnlyValue}>
            {memory.file_path ?? (
              <span className={styles.warning}>
                <ExclamationTriangleIcon /> This memory has no file path and
                cannot be edited
              </span>
            )}
          </div>
        </div>

        <div className={styles.field}>
          <label className={styles.label}>CONTENT</label>
          <textarea
            value={draft.content}
            onChange={(e) => handleFieldChange("content", e.target.value)}
            className={styles.textarea}
            placeholder="Memory content..."
          />
        </div>
      </div>

      <div className={styles.actions}>
        <Button
          onClick={handleSave}
          disabled={!canSave || isSaving}
          style={{ flex: 1 }}
        >
          {isSaving ? "Saving..." : "Save"}
        </Button>
        <Button
          color="red"
          variant="outline"
          onClick={() => setIsDeleteOpen(true)}
          disabled={!canDelete}
          style={{ flex: 1 }}
        >
          Delete
        </Button>
      </div>

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
                <Button color="yellow" onClick={() => handleDelete(true)}>
                  Archive
                </Button>
                <Button color="red" onClick={() => handleDelete(false)}>
                  Permanently Delete
                </Button>
              </Flex>
            </Flex>
          </Dialog.Content>
        </Dialog.Root>
      )}

      {showDiscardDialog && (
        <Dialog.Root
          open={showDiscardDialog}
          onOpenChange={setShowDiscardDialog}
        >
          <Dialog.Content>
            <Dialog.Title>Unsaved Changes</Dialog.Title>
            <Flex direction="column" gap="3">
              <p>You have unsaved changes. Discard them?</p>
              <Flex gap="2" justify="end">
                <Button
                  variant="outline"
                  onClick={() => setShowDiscardDialog(false)}
                >
                  Cancel
                </Button>
                <Button color="red" onClick={handleDiscardChanges}>
                  Discard
                </Button>
              </Flex>
            </Flex>
          </Dialog.Content>
        </Dialog.Root>
      )}
    </div>
  );
}
