import React, { useCallback, useEffect, useRef, useState } from "react";
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
  Text,
} from "../../../components/ui";
import { Checkbox } from "../../../components/Checkbox";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  type CreateTaskDocumentRequest,
  type TaskDocumentDetail,
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

type DocumentDraft = {
  formSlug: string;
  name: string;
  kind: TaskDocumentKind;
  pinned: boolean;
  content: string;
  baseline?: TaskDocumentDetail;
  conflictVersion?: number;
  mutationError?: string;
  nameError?: string | null;
  contentError?: string | null;
};

const EMPTY_DRAFT: DocumentDraft = {
  formSlug: "",
  name: "",
  kind: "plan",
  pinned: false,
  content: "",
};

function draftFromDocument(doc: TaskDocumentDetail): DocumentDraft {
  return {
    formSlug: doc.slug,
    name: doc.name,
    kind: doc.kind,
    pinned: doc.pinned,
    content: doc.content,
    baseline: doc,
    conflictVersion: undefined,
    mutationError: undefined,
    nameError: null,
    contentError: null,
  };
}

function isDraftDirty(draft: DocumentDraft): boolean {
  return draft.baseline
    ? draft.content !== draft.baseline.content ||
        draft.pinned !== draft.baseline.pinned
    : Boolean(
        draft.formSlug ||
          draft.name ||
          draft.content ||
          draft.pinned ||
          draft.kind !== "plan",
      );
}

function sameBaseline(a: TaskDocumentDetail, b: TaskDocumentDetail): boolean {
  return (
    a.version === b.version &&
    a.updated_at === b.updated_at &&
    a.content === b.content &&
    a.pinned === b.pinned &&
    a.name === b.name &&
    a.kind === b.kind
  );
}

function conflictVersionFromError(
  error: unknown,
  expectedVersion: number | undefined,
): number | undefined {
  if (typeof error !== "object" || error === null) return undefined;
  if (!("status" in error) || error.status !== 409 || !("data" in error))
    return undefined;
  const data: unknown = error.data;
  if (typeof data !== "object" || data === null) return undefined;
  if (
    !("code" in data) ||
    data.code !== "task_document_version_conflict" ||
    !("expected_version" in data) ||
    data.expected_version !== expectedVersion ||
    !("actual_version" in data) ||
    typeof data.actual_version !== "number" ||
    !Number.isSafeInteger(data.actual_version) ||
    data.actual_version < 0
  )
    return undefined;
  return data.actual_version;
}

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
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  const identity = JSON.stringify([endpoint, taskId, mode, slug]);
  const activeEditor = useRef({ identity, open });
  if (
    activeEditor.current.identity !== identity ||
    activeEditor.current.open !== open
  ) {
    activeEditor.current = { identity, open };
  }
  useEffect(() => {
    activeEditor.current = { identity, open };
    return () => {
      activeEditor.current = { ...activeEditor.current, open: false };
    };
  }, [identity, open]);

  const {
    currentData: requestedDoc,
    isError: isFetchError,
    isFetching,
    refetch,
  } = useGetTaskDocumentQuery(
    { taskId, slug: slug ?? "" },
    {
      skip: !isEditMode || !slug || !open,
      refetchOnMountOrArgChange: true,
    },
  );
  const existingDoc = requestedDoc?.slug === slug ? requestedDoc : undefined;
  const [drafts, setDrafts] = useState<Partial<Record<string, DocumentDraft>>>(
    {},
  );
  const draft = drafts[identity] ?? EMPTY_DRAFT;
  const {
    formSlug,
    name,
    kind,
    pinned,
    content,
    baseline,
    conflictVersion,
    mutationError,
    nameError,
    contentError,
  } = draft;
  const isDirty = isDraftDirty(draft);
  const hasConflict = Boolean(
    conflictVersion !== undefined ||
      (baseline &&
        existingDoc &&
        existingDoc.version >= baseline.version &&
        !sameBaseline(baseline, existingDoc)),
  );
  const canResolveConflict = Boolean(
    existingDoc &&
      (conflictVersion === undefined ||
        existingDoc.version >= conflictVersion) &&
      !isFetching &&
      !isFetchError,
  );
  const isEditDocumentReady = !isEditMode || Boolean(baseline);
  const [confirmation, setConfirmation] = useState<{
    identity: string;
    action: "close" | "reload";
  } | null>(null);
  const currentConfirmation =
    confirmation?.identity === identity ? confirmation.action : null;

  const [createDocument, { isLoading: isCreating }] =
    useCreateTaskDocumentMutation();
  const [updateDocument, { isLoading: isUpdating }] =
    useUpdateTaskDocumentMutation();
  const savePending = useRef(false);
  const isSaving = isCreating || isUpdating;

  useEffect(() => {
    if (!open || !isEditMode || !existingDoc) return;
    setDrafts((previous) => {
      const current = previous[identity];
      if (
        current &&
        (isDraftDirty(current) ||
          current.conflictVersion !== undefined ||
          (current.baseline &&
            (current.baseline.version > existingDoc.version ||
              sameBaseline(current.baseline, existingDoc))))
      ) {
        return previous;
      }
      return {
        ...previous,
        [identity]: {
          ...draftFromDocument(existingDoc),
          mutationError: current?.mutationError,
        },
      };
    });
  }, [open, isEditMode, existingDoc, identity]);

  const updateDraft = useCallback(
    (changes: Partial<DocumentDraft>) => {
      setDrafts((previous) => ({
        ...previous,
        [identity]: { ...(previous[identity] ?? EMPTY_DRAFT), ...changes },
      }));
    },
    [identity],
  );

  const forgetDraft = useCallback(() => {
    setDrafts((previous) => {
      return Object.fromEntries(
        Object.entries(previous).filter(([key]) => key !== identity),
      );
    });
    setConfirmation(null);
  }, [identity]);

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (savePending.current) return;
      if (!nextOpen && isDirty) {
        setConfirmation({ identity, action: "close" });
        return;
      }
      setConfirmation(null);
      onOpenChange(nextOpen);
    },
    [identity, isDirty, onOpenChange],
  );

  const reloadServer = () => {
    if (!existingDoc || !canResolveConflict || savePending.current) return;
    updateDraft(draftFromDocument(existingDoc));
    setConfirmation(null);
  };

  const isSlugValid =
    SLUG_PATTERN.test(formSlug) && formSlug.length >= SLUG_MIN_LENGTH;
  const isNameValid = name.trim().length > 0;
  const isContentValid = content.trim().length > 0;
  const canSave =
    !isSaving &&
    !hasConflict &&
    isEditDocumentReady &&
    (!isEditMode || (Boolean(existingDoc) && !isFetching && !isFetchError)) &&
    isContentValid &&
    (isEditMode || (isSlugValid && isNameValid));
  const slugError =
    !formSlug || isSlugValid
      ? null
      : !SLUG_PATTERN.test(formSlug)
        ? "Slug must start with a-z or 0-9 and contain only a-z, 0-9, _, -"
        : "Slug must be at least 3 characters";

  const handleSave = async () => {
    if (savePending.current || !canSave || !open) return;
    const session = activeEditor.current;
    savePending.current = true;
    updateDraft({ mutationError: undefined });
    try {
      if (isEditMode) {
        if (!slug || !baseline || existingDoc?.slug !== slug) return;
        const saved = await updateDocument({
          taskId,
          slug,
          content,
          pinned,
          expectedVersion: baseline.version,
        }).unwrap();
        if (activeEditor.current !== session || !session.open) return;
        updateDraft(draftFromDocument(saved));
      } else {
        const req: CreateTaskDocumentRequest = {
          taskId,
          slug: formSlug,
          name,
          kind,
          content,
          pinned,
        };
        await createDocument(req).unwrap();
        if (activeEditor.current !== session || !session.open) return;
        forgetDraft();
      }
      setConfirmation(null);
      onOpenChange(false);
    } catch (error: unknown) {
      if (activeEditor.current !== session || !session.open) return;
      const serverVersion = isEditMode
        ? conflictVersionFromError(error, baseline?.version)
        : undefined;
      updateDraft({
        conflictVersion: serverVersion,
        mutationError:
          serverVersion === undefined
            ? `Failed to save document ${
                slug ?? formSlug
              } in task ${taskId}. The save outcome could not be confirmed. Your draft is still here. Check the server copy before retrying.`
            : undefined,
      });
    } finally {
      savePending.current = false;
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content
        className={styles.editorDialog}
        maxHeight="calc(100dvh - var(--rf-space-5))"
        maxWidth="600px"
      >
        <Dialog.Title>
          {isEditMode ? "Edit document" : "New document"}
        </Dialog.Title>
        <Dialog.Description>
          Reference document for task {taskId}
          {isEditMode ? ` / ${slug ?? ""}` : ""}. This does not change the task
          goal or its attached plan.
        </Dialog.Description>
        {isEditMode && isFetchError && (
          <ErrorState
            title={`Failed to load document ${slug ?? ""} in task ${taskId}.`}
            description="Any local draft has been kept. Retry before saving."
            retry={
              <Button
                variant="soft"
                disabled={isFetching}
                onClick={() => void refetch()}
              >
                Retry document
              </Button>
            }
          />
        )}
        {isEditMode && !isEditDocumentReady ? (
          !isFetchError && (
            <div className={styles.loadingState}>
              <Spinner label="Loading document" />
            </div>
          )
        ) : (
          <Flex direction="column" gap="3" className={styles.editorForm}>
            {baseline && (
              <Text size="1" className={styles.mutedText}>
                Draft based on v{baseline.version}
                {isDirty ? " · Unsaved changes" : ""}
              </Text>
            )}
            {hasConflict && (
              <Flex
                direction="column"
                gap="2"
                role="alert"
                className={styles.conflictNotice}
              >
                <Text size="2" weight="medium">
                  This document changed on the server. Your draft has been kept.
                </Text>
                <Text size="2">
                  Server v
                  {Math.max(existingDoc?.version ?? 0, conflictVersion ?? 0)};
                  draft based on v{baseline?.version}. Keep your draft for the
                  next save, or reload the server copy. You can copy your text
                  below first. Keeping your draft uses the loaded server version
                  for your next save. Saving replaces server content only if
                  that version still matches.
                </Text>
                <Flex gap="2" wrap="wrap">
                  {!canResolveConflict && (
                    <Button
                      variant="soft"
                      disabled={isSaving || isFetching}
                      onClick={() => void refetch()}
                    >
                      Refresh server copy
                    </Button>
                  )}
                  <Button
                    variant="soft"
                    disabled={isSaving || !canResolveConflict}
                    onClick={() => {
                      if (
                        !existingDoc ||
                        !canResolveConflict ||
                        savePending.current
                      )
                        return;
                      updateDraft({
                        baseline: existingDoc,
                        name: existingDoc.name,
                        kind: existingDoc.kind,
                        conflictVersion: undefined,
                        mutationError: undefined,
                      });
                      setConfirmation(null);
                    }}
                  >
                    Keep draft for next save
                  </Button>
                  <Button
                    variant="plain"
                    disabled={isSaving || !canResolveConflict}
                    onClick={() => {
                      if (isDirty) {
                        setConfirmation({ identity, action: "reload" });
                      } else {
                        reloadServer();
                      }
                    }}
                  >
                    Reload server copy
                  </Button>
                </Flex>
              </Flex>
            )}
            <Field label="Slug" error={slugError}>
              <FieldText
                value={formSlug}
                onChange={(value) => updateDraft({ formSlug: value })}
                readOnly={isEditMode || isSaving}
                placeholder="my-doc"
                aria-label="Slug"
              />
            </Field>
            <Field label="Name" error={!isEditMode ? nameError : null}>
              <FieldText
                value={name}
                onChange={(value) =>
                  updateDraft({
                    name: value,
                    nameError: value.trim() ? null : "Name is required",
                  })
                }
                placeholder="Document name"
                aria-label="Name"
                readOnly={isEditMode || isSaving}
              />
            </Field>
            <Field label="Kind">
              <FieldSelect
                value={kind}
                options={DOCUMENT_KINDS.map((documentKind) => ({
                  value: documentKind,
                  label: documentKind,
                }))}
                onChange={(value) =>
                  updateDraft({ kind: value as TaskDocumentKind })
                }
                disabled={isEditMode || isSaving}
                aria-label="Kind"
              />
            </Field>
            <Checkbox
              checked={pinned}
              disabled={isSaving}
              onCheckedChange={(checked) =>
                updateDraft({ pinned: checked === true })
              }
            >
              Pinned
            </Checkbox>
            <Field label="Content" error={contentError}>
              <FieldTextarea
                value={content}
                onChange={(value) =>
                  updateDraft({
                    content: value,
                    contentError: value.trim() ? null : "Content is required",
                  })
                }
                readOnly={isSaving}
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
          </Flex>
        )}
        {currentConfirmation && (
          <Flex direction="column" gap="2" role="alert">
            <Text size="2">
              {currentConfirmation === "close"
                ? "Discard unsaved changes?"
                : "Discard your draft and reload the server copy?"}
            </Text>
            <Flex gap="2" wrap="wrap">
              <Button variant="plain" onClick={() => setConfirmation(null)}>
                Keep editing
              </Button>
              <Button
                variant="danger"
                disabled={
                  isSaving ||
                  (currentConfirmation === "reload" && !canResolveConflict)
                }
                onClick={() => {
                  if (savePending.current) return;
                  if (currentConfirmation === "reload") {
                    reloadServer();
                  } else {
                    forgetDraft();
                    onOpenChange(false);
                  }
                }}
              >
                {currentConfirmation === "close"
                  ? "Discard changes"
                  : "Discard draft and reload"}
              </Button>
            </Flex>
          </Flex>
        )}
        <Dialog.Footer>
          <Button
            variant="plain"
            disabled={isSaving}
            onClick={() => handleOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            onClick={() => void handleSave()}
            disabled={!canSave || Boolean(currentConfirmation)}
            loading={isSaving}
          >
            Save
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
};

export default DocumentEditor;
