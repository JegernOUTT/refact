import React, { useCallback, useEffect, useState } from "react";
import {
  Box,
  Button,
  Callout,
  Checkbox,
  Dialog,
  Flex,
  Select,
  Spinner,
  Text,
  TextArea,
  TextField,
} from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import {
  type CreateTaskDocumentRequest,
  type TaskDocumentKind,
  useCreateTaskDocumentMutation,
  useGetTaskDocumentQuery,
  usePinTaskDocumentMutation,
  useUpdateTaskDocumentMutation,
} from "../../../services/refact/taskDocumentsApi";

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
  const [pinDocument, { isLoading: isPinning }] = usePinTaskDocumentMutation();

  const isSaving = isCreating || isUpdating || isPinning;

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

  const handleSlugChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const value = event.target.value;
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
    },
    [],
  );

  const handleNameChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const value = event.target.value;
      setName(value);
      setNameError(value.trim().length === 0 ? "Name is required" : null);
    },
    [],
  );

  const handleContentChange = useCallback(
    (event: React.ChangeEvent<HTMLTextAreaElement>) => {
      const value = event.target.value;
      setContent(value);
      setContentError(value.trim().length === 0 ? "Content is required" : null);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    setMutationError(null);
    try {
      if (isEditMode) {
        if (!slug || existingDoc?.slug !== slug) {
          setMutationError("Document is still loading. Please wait.");
          return;
        }
        await updateDocument({ taskId, slug, content }).unwrap();
        if (pinned !== existingDoc.pinned) {
          await pinDocument({ taskId, slug, pinned }).unwrap();
        }
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
    pinDocument,
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
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="600px">
        <Dialog.Title>
          {isEditMode ? "Edit document" : "New document"}
        </Dialog.Title>
        {isEditMode && !isEditDocumentReady ? (
          <Flex justify="center" p="6">
            <Spinner aria-label="Loading document" />
          </Flex>
        ) : (
          <Flex direction="column" gap="3" mt="2">
            <Box>
              <Text size="2" weight="medium" as="div" mb="1">
                Slug
              </Text>
              <TextField.Root
                value={formSlug}
                onChange={handleSlugChange}
                readOnly={isEditMode}
                placeholder="my-doc"
                aria-label="Slug"
              />
              {slugError && (
                <Text size="1" color="red" as="div" mt="1">
                  {slugError}
                </Text>
              )}
            </Box>
            <Box>
              <Text size="2" weight="medium" as="div" mb="1">
                Name
              </Text>
              <TextField.Root
                value={name}
                onChange={handleNameChange}
                placeholder="Document name"
                aria-label="Name"
                readOnly={isEditMode}
              />
              {!isEditMode && nameError && (
                <Text size="1" color="red" as="div" mt="1">
                  {nameError}
                </Text>
              )}
            </Box>
            <Box>
              <Text size="2" weight="medium" as="div" mb="1">
                Kind
              </Text>
              <Select.Root
                value={kind}
                onValueChange={(v) => setKind(v as TaskDocumentKind)}
                disabled={isEditMode}
              >
                <Select.Trigger aria-label="Kind" />
                <Select.Content>
                  {DOCUMENT_KINDS.map((k) => (
                    <Select.Item key={k} value={k}>
                      {k}
                    </Select.Item>
                  ))}
                </Select.Content>
              </Select.Root>
            </Box>
            <Text as="label" size="2">
              <Flex align="center" gap="2">
                <Checkbox
                  checked={pinned}
                  onCheckedChange={(checked) => setPinned(checked === true)}
                />
                Pinned
              </Flex>
            </Text>
            <Box>
              <Text size="2" weight="medium" as="div" mb="1">
                Content
              </Text>
              <TextArea
                value={content}
                onChange={handleContentChange}
                placeholder="Write markdown content here..."
                aria-label="Content"
                rows={12}
              />
              {contentError && (
                <Text size="1" color="red" as="div" mt="1">
                  {contentError}
                </Text>
              )}
            </Box>
            {mutationError && (
              <Callout.Root color="red" size="1">
                <Callout.Icon>
                  <ExclamationTriangleIcon />
                </Callout.Icon>
                <Callout.Text>{mutationError}</Callout.Text>
              </Callout.Root>
            )}
            <Flex justify="end" gap="2">
              <Dialog.Close>
                <Button variant="soft" color="gray" disabled={isSaving}>
                  Cancel
                </Button>
              </Dialog.Close>
              <Button
                onClick={() => void handleSave()}
                disabled={isSaving || !isEditDocumentReady || !canSave}
              >
                {isSaving ? "Saving..." : "Save"}
              </Button>
            </Flex>
          </Flex>
        )}
      </Dialog.Content>
    </Dialog.Root>
  );
};

export default DocumentEditor;
