import React, { useCallback, useEffect, useState } from "react";
import {
  Button,
  Dialog,
  ErrorState,
  Field,
  FieldSelect,
  FieldText,
  FieldTextarea,
  Flex,
  Spinner,
} from "../../../components/ui";
import { Checkbox } from "../../../components/Checkbox";
import {
  type CreateTaskDocumentRequest,
  type TaskDocumentKind,
  useCreateTaskDocumentMutation,
  useGetTaskDocumentQuery,
  useUpdateTaskDocumentMutation,
} from "../../../services/refact/taskDocumentsApi";
import styles from "./TaskDocuments.module.css";

const DOCUMENT_KINDS: TaskDocumentKind[] = [
  "plan",
  "design",
  "runbook",
  "brief",
  "postmortem",
  "spec",
];

const SLUG_PATTERN = /^[a-z0-9][a-z0-9_-]*$/;
const SLUG_MIN_LENGTH = 3;

type DocumentEditorProps = {
  taskId: string;
  mode: "create" | "edit";
  slug?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export const DocumentEditor: React.FC<DocumentEditorProps> = ({
  taskId,
  mode,
  slug,
  open,
  onOpenChange,
}) => {
  const isEditMode = mode === "edit";

  const { currentData: requestedDoc } = useGetTaskDocumentQuery(
    { taskId, slug: slug ?? "" },
    { skip: !isEditMode || !slug || !open },
  );
  const existingDoc = requestedDoc?.slug === slug ? requestedDoc : undefined;
  const isEditDocumentReady =
    !isEditMode || (Boolean(slug) && existingDoc?.slug === slug);

  const [formSlug, setFormSlug] = useState("");
  const [name, setName] = useState("");
  const [kind, setKind] = useState<TaskDocumentKind>("plan");
  const [pinned, setPinned] = useState(false);
  const [content, setContent] = useState("");
  const [slugError, setSlugError] = useState<string | null>(null);
  const [nameError, setNameError] = useState<string | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);

  const [createDocument, { isLoading: isCreating }] =
    useCreateTaskDocumentMutation();
  const [updateDocument, { isLoading: isUpdating }] =
    useUpdateTaskDocumentMutation();

  const isSaving = isCreating || isUpdating;

  useEffect(() => {
    if (!open) return;
    if (isEditMode && existingDoc) {
      setFormSlug(existingDoc.slug);
      setName(existingDoc.name);
      setKind(existingDoc.kind);
      setPinned(existingDoc.pinned);
      setContent(existingDoc.content);
      setSlugError(null);
      setNameError(null);
      setContentError(null);
      setMutationError(null);
    } else if (!isEditMode) {
      setFormSlug("");
      setName("");
      setKind("plan");
      setPinned(false);
      setContent("");
      setSlugError(null);
      setNameError(null);
      setContentError(null);
      setMutationError(null);
    }
  }, [open, isEditMode, existingDoc, slug]);

  const handleSlugChange = useCallback((value: string) => {
    setFormSlug(value);
    if (value && !SLUG_PATTERN.test(value)) {
      setSlugError(
        "Slug must start with a-z or 0-9 and contain only a-z, 0-9, _, -",
      );
    } else if (value && value.length < SLUG_MIN_LENGTH) {
      setSlugError("Slug must be at least 3 characters");
    } else {
      setSlugError(null);
    }
  }, []);

  const handleNameChange = useCallback((value: string) => {
    setName(value);
    setNameError(value.trim().length === 0 ? "Name is required" : null);
  }, []);

  const handleContentChange = useCallback((value: string) => {
    setContent(value);
    setContentError(value.trim().length === 0 ? "Content is required" : null);
  }, []);

  const handleSave = useCallback(async () => {
    setMutationError(null);
    try {
      if (isEditMode) {
        if (!slug || existingDoc?.slug !== slug) {
          setMutationError("Document is still loading. Please wait.");
          return;
        }
        await updateDocument({ taskId, slug, content, pinned }).unwrap();
      } else {
        if (
          !formSlug ||
          !SLUG_PATTERN.test(formSlug) ||
          formSlug.length < SLUG_MIN_LENGTH
        ) {
          setSlugError("Slug is required and must be valid.");
          return;
        }
        const req: CreateTaskDocumentRequest = {
          taskId,
          slug: formSlug,
          name,
          kind,
          content,
          pinned,
        };
        await createDocument(req).unwrap();
      }
      onOpenChange(false);
    } catch {
      setMutationError("Failed to save document. Please try again.");
    }
  }, [
    isEditMode,
    slug,
    updateDocument,
    taskId,
    content,
    existingDoc,
    pinned,
    formSlug,
    name,
    kind,
    createDocument,
    onOpenChange,
  ]);

  const isSlugValid =
    SLUG_PATTERN.test(formSlug) && formSlug.length >= SLUG_MIN_LENGTH;
  const isNameValid = name.trim().length > 0;
  const isContentValid = content.trim().length > 0;
  const canSave = isEditMode
    ? isContentValid
    : isSlugValid && isNameValid && isContentValid;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content
        className={styles.editorDialog}
        maxHeight="calc(100dvh - var(--rf-space-5))"
        maxWidth="600px"
      >
        <Dialog.Title>
          {isEditMode ? "Edit document" : "New document"}
        </Dialog.Title>
        {isEditMode && !isEditDocumentReady ? (
          <div className={styles.loadingState}>
            <Spinner label="Loading document" />
          </div>
        ) : (
          <Flex direction="column" gap="3" className={styles.editorForm}>
            <Field label="Slug" error={slugError}>
              <FieldText
                value={formSlug}
                onChange={handleSlugChange}
                readOnly={isEditMode}
                placeholder="my-doc"
                aria-label="Slug"
              />
            </Field>
            <Field label="Name" error={!isEditMode ? nameError : null}>
              <FieldText
                value={name}
                onChange={handleNameChange}
                placeholder="Document name"
                aria-label="Name"
                readOnly={isEditMode}
              />
            </Field>
            <Field label="Kind">
              <FieldSelect
                value={kind}
                options={DOCUMENT_KINDS.map((documentKind) => ({
                  value: documentKind,
                  label: documentKind,
                }))}
                onChange={(value) => setKind(value as TaskDocumentKind)}
                disabled={isEditMode}
                aria-label="Kind"
              />
            </Field>
            <Checkbox
              checked={pinned}
              onCheckedChange={(checked) => setPinned(checked === true)}
            >
              Pinned
            </Checkbox>
            <Field label="Content" error={contentError}>
              <FieldTextarea
                value={content}
                onChange={handleContentChange}
                placeholder="Write markdown content here..."
                aria-label="Content"
                rows={12}
                className={styles.editorTextarea}
              />
            </Field>
            {mutationError && (
              <ErrorState
                title={mutationError}
                variant="compact"
                className={styles.errorState}
              />
            )}
            <Flex justify="end" gap="2" wrap="wrap">
              <Dialog.Close asChild>
                <Button variant="plain" disabled={isSaving}>
                  Cancel
                </Button>
              </Dialog.Close>
              <Button
                onClick={() => void handleSave()}
                disabled={isSaving || !isEditDocumentReady || !canSave}
                loading={isSaving}
              >
                Save
              </Button>
            </Flex>
          </Flex>
        )}
      </Dialog.Content>
    </Dialog>
  );
};

export default DocumentEditor;
