import { useEffect, useState } from "react";
import { AlertTriangle, X } from "lucide-react";

import {
  Badge,
  Button,
  ButtonGroup,
  Dialog,
  FieldStack,
  FieldText,
  FieldTextarea,
  Icon,
  IconButton,
  Surface,
} from "../../components/ui";
import type { KnowledgeMemoRecord } from "../../services/refact/types";
import {
  useDeleteMemoryMutation,
  useUpdateMemoryMutation,
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
      .catch((_error: unknown) => undefined);
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
      .catch((_error: unknown) => undefined);
  };

  const handleDiscardChanges = () => {
    setShowDiscardDialog(false);
    setIsDirty(false);
  };

  if (!memory) {
    return (
      <Surface className={styles.container} radius="none">
        <p className={styles.emptyState}>No memory selected</p>
      </Surface>
    );
  }

  const canSave = Boolean(
    memory.file_path && isDirty && draft.title && draft.content,
  );
  const canDelete = Boolean(memory.file_path);

  return (
    <Surface className={styles.container} radius="none">
      <div className={styles.scrollArea}>
        <FieldStack
          label={
            <>
              TITLE{" "}
              {isDirty ? (
                <span className={styles.dirtyIndicator}>●</span>
              ) : null}
            </>
          }
        >
          <FieldText
            value={draft.title}
            onChange={(value) => handleFieldChange("title", value)}
            placeholder="Untitled"
            className={styles.input}
          />
        </FieldStack>

        <FieldStack label="KIND">
          <Surface
            className={styles.readOnlyValue}
            radius="control"
            variant="surface-1"
          >
            {draft.kind}
          </Surface>
        </FieldStack>

        <FieldStack label="CREATED">
          <Surface
            className={styles.readOnlyValue}
            radius="control"
            variant="surface-1"
          >
            {memory.created ?? "—"}
          </Surface>
        </FieldStack>

        <FieldStack label="TAGS">
          {draft.tags.length > 0 ? (
            <div className={styles.tagsContainer}>
              {draft.tags.map((tag) => (
                <Badge key={tag} tone="accent" className={styles.tag}>
                  {tag}
                  <IconButton
                    className={styles.tagRemove}
                    icon={X}
                    size="sm"
                    variant="plain"
                    onClick={() => handleRemoveTag(tag)}
                    aria-label={`Remove ${tag}`}
                  />
                </Badge>
              ))}
            </div>
          ) : null}
          <FieldText
            value={tagsInput}
            onChange={setTagsInput}
            onBlur={handleTagsBlur}
            placeholder="comma, separated, tags"
            className={styles.input}
          />
        </FieldStack>

        <FieldStack label="FILE PATH">
          <Surface
            className={styles.readOnlyValue}
            radius="control"
            variant="surface-1"
          >
            {memory.file_path ?? (
              <span className={styles.warning}>
                <Icon icon={AlertTriangle} size="sm" tone="warning" /> This
                memory has no file path and cannot be edited
              </span>
            )}
          </Surface>
        </FieldStack>

        <FieldStack label="CONTENT">
          <FieldTextarea
            value={draft.content}
            onChange={(value) => handleFieldChange("content", value)}
            className={styles.textarea}
            placeholder="Memory content..."
          />
        </FieldStack>
      </div>

      <div className={styles.actions}>
        <Button
          className={styles.actionButton}
          onClick={handleSave}
          disabled={!canSave || isSaving}
          loading={isSaving}
          variant="primary"
        >
          Save
        </Button>
        <Button
          className={styles.actionButton}
          variant="danger"
          onClick={() => setIsDeleteOpen(true)}
          disabled={!canDelete}
        >
          Delete
        </Button>
      </div>

      {isDeleteOpen ? (
        <Dialog open={isDeleteOpen} onOpenChange={setIsDeleteOpen}>
          <Dialog.Content maxWidth="420px">
            <Dialog.Title>Delete Memory</Dialog.Title>
            <div className={styles.dialogBody}>
              <p>What would you like to do?</p>
              <ButtonGroup className={styles.dialogActions}>
                <Button variant="ghost" onClick={() => setIsDeleteOpen(false)}>
                  Cancel
                </Button>
                <Button variant="soft" onClick={() => handleDelete(true)}>
                  Archive
                </Button>
                <Button variant="danger" onClick={() => handleDelete(false)}>
                  Permanently Delete
                </Button>
              </ButtonGroup>
            </div>
          </Dialog.Content>
        </Dialog>
      ) : null}

      {showDiscardDialog ? (
        <Dialog open={showDiscardDialog} onOpenChange={setShowDiscardDialog}>
          <Dialog.Content maxWidth="420px">
            <Dialog.Title>Unsaved Changes</Dialog.Title>
            <div className={styles.dialogBody}>
              <p>You have unsaved changes. Discard them?</p>
              <ButtonGroup className={styles.dialogActions}>
                <Button
                  variant="ghost"
                  onClick={() => setShowDiscardDialog(false)}
                >
                  Cancel
                </Button>
                <Button variant="danger" onClick={handleDiscardChanges}>
                  Discard
                </Button>
              </ButtonGroup>
            </div>
          </Dialog.Content>
        </Dialog>
      ) : null}
    </Surface>
  );
}
