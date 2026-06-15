import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import { Clock, Pencil, Pin, Plus, Trash2 } from "lucide-react";
import { Markdown } from "../../../components/Markdown";
import { Checkbox } from "../../../components/Checkbox";
import {
  Badge,
  Button,
  Dialog,
  ErrorState,
  Flex,
  IconButton,
  Popover,
  Select,
  Spinner,
  Surface,
  Text,
  Tooltip,
} from "../../../components/ui";
import type { BadgeTone } from "../../../components/ui";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../../components/shared/useDelayedUnmount";
import { DocumentEditor } from "./DocumentEditor";
import {
  type TaskDocumentSummary,
  useDeleteTaskDocumentMutation,
  useGetTaskDocumentHistoryQuery,
  useGetTaskDocumentQuery,
  useListTaskDocumentsQuery,
  usePinTaskDocumentMutation,
} from "../../../services/refact/taskDocumentsApi";
import {
  DOCUMENT_KINDS,
  documentKindColor,
} from "../../../services/refact/taskKinds";
import styles from "./TaskDocuments.module.css";

const ALL_VALUE = "all";

function formatUpdatedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function badgeTone(color: ReturnType<typeof documentKindColor>): BadgeTone {
  if (color === "red") return "danger";
  if (color === "amber") return "warning";
  if (color === "gray") return "muted";
  if (color === "green" || color === "teal") return "success";
  return "accent";
}

type DocumentRowProps = {
  document: TaskDocumentSummary;
  isExpanded: boolean;
  expandedContent?: string;
  isExpandedLoading: boolean;
  onToggleExpand: () => void;
  onPin: (slug: string, pinned: boolean) => void | Promise<void>;
  onEdit: (slug: string) => void;
  onHistory: (slug: string) => void;
  onDelete: (slug: string) => void | Promise<void>;
};

const DocumentRow: React.FC<DocumentRowProps> = ({
  document,
  isExpanded,
  expandedContent,
  isExpandedLoading,
  onToggleExpand,
  onPin,
  onEdit,
  onHistory,
  onDelete,
}) => {
  const pinned = document.pinned;
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isExpanded,
    COLLAPSE_ANIMATION_MS,
  );
  const lastExpandedContent = React.useRef<string | undefined>(expandedContent);
  if (expandedContent !== undefined) {
    lastExpandedContent.current = expandedContent;
  }
  const renderedExpandedContent =
    expandedContent ?? (!isExpanded ? lastExpandedContent.current : undefined);

  return (
    <Surface
      animated="rise"
      className={classNames(styles.row, pinned && styles.rowPinned)}
      data-testid={`document-row-${document.slug}`}
      onClick={onToggleExpand}
      radius="card"
      variant="plain"
    >
      <Flex justify="between" align="start" gap="2" className={styles.rowTop}>
        <Flex align="center" gap="2" wrap="wrap" className={styles.rowHeader}>
          <Tooltip content={pinned ? "Unpin" : "Pin"}>
            <IconButton
              size="sm"
              variant="plain"
              aria-label={pinned ? "Unpin" : "Pin"}
              icon={Pin}
              className={classNames(
                styles.rowIconButton,
                pinned && styles.iconButtonPinned,
              )}
              onClick={(e) => {
                e.stopPropagation();
                void onPin(document.slug, !pinned);
              }}
            />
          </Tooltip>
          <Badge
            tone={badgeTone(documentKindColor(document.kind))}
            data-testid={`kind-badge-${document.slug}`}
          >
            {document.kind}
          </Badge>
          <Text weight="bold" size="2" className={styles.rowTitle}>
            {document.name}
          </Text>
          <Text size="1" className={styles.mutedText}>
            v{document.version}
          </Text>
          <Text size="1" className={styles.mutedText}>
            {formatUpdatedAt(document.updated_at)}
          </Text>
        </Flex>

        <Flex gap="1" align="center" className={styles.rowControls}>
          <Tooltip content="Edit">
            <IconButton
              size="sm"
              variant="plain"
              aria-label="Edit"
              icon={Pencil}
              className={styles.rowIconButton}
              onClick={(e) => {
                e.stopPropagation();
                onEdit(document.slug);
              }}
            />
          </Tooltip>
          <Tooltip content="History">
            <IconButton
              size="sm"
              variant="plain"
              aria-label="History"
              icon={Clock}
              className={styles.rowIconButton}
              onClick={(e) => {
                e.stopPropagation();
                onHistory(document.slug);
              }}
            />
          </Tooltip>
          <Popover>
            <Tooltip content="Delete">
              <Popover.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="plain"
                  aria-label="Delete"
                  icon={Trash2}
                  className={classNames(
                    styles.rowIconButton,
                    styles.dangerIcon,
                  )}
                  onClick={(e) => e.stopPropagation()}
                />
              </Popover.Trigger>
            </Tooltip>
            <Popover.Content className={styles.confirmPopover}>
              <Flex direction="column" gap="3">
                <Text size="2">Delete this document?</Text>
                <Flex gap="2" wrap="wrap">
                  <Popover.Close asChild>
                    <Button
                      size="sm"
                      variant="danger"
                      onClick={() => {
                        void onDelete(document.slug);
                      }}
                    >
                      Confirm delete
                    </Button>
                  </Popover.Close>
                  <Popover.Close asChild>
                    <Button size="sm" variant="plain">
                      Cancel
                    </Button>
                  </Popover.Close>
                </Flex>
              </Flex>
            </Popover.Content>
          </Popover>
        </Flex>
      </Flex>

      {shouldRender && (
        <div
          className="rf-expand-grid"
          data-open={isAnimatingOpen}
          data-state={isAnimatingOpen ? "open" : "closed"}
        >
          <div className={styles.content}>
            {isExpandedLoading ? (
              <div className={styles.inlineLoading}>
                <Spinner size="sm" />
              </div>
            ) : renderedExpandedContent !== undefined ? (
              <Markdown canHaveInteractiveElements={false}>
                {renderedExpandedContent}
              </Markdown>
            ) : (
              <Text size="2" className={styles.mutedText}>
                Document content is unavailable.
              </Text>
            )}
          </div>
        </div>
      )}
    </Surface>
  );
};

type DocumentsPanelProps = {
  taskId: string;
};

export const DocumentsPanel: React.FC<DocumentsPanelProps> = ({ taskId }) => {
  const [kindFilter, setKindFilter] = useState<string>(ALL_VALUE);
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [expandedSlug, setExpandedSlug] = useState<string | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [editorSlug, setEditorSlug] = useState<string | undefined>(undefined);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historySlug, setHistorySlug] = useState<string | null>(null);
  const [selectedHistoryVersion, setSelectedHistoryVersion] = useState<
    number | null
  >(null);

  const { data, isFetching, error } = useListTaskDocumentsQuery({ taskId });

  const {
    currentData: requestedExpandedDoc,
    isFetching: isExpandedFetching,
    isError: isExpandedError,
  } = useGetTaskDocumentQuery(
    { taskId, slug: expandedSlug ?? "" },
    { skip: !expandedSlug },
  );
  const expandedDoc =
    requestedExpandedDoc?.slug === expandedSlug
      ? requestedExpandedDoc
      : undefined;

  const { currentData: historyData, isFetching: isHistoryFetching } =
    useGetTaskDocumentHistoryQuery(
      { taskId, slug: historySlug ?? "" },
      { skip: !historySlug || !historyOpen },
    );

  const {
    currentData: selectedHistoryDoc,
    isFetching: isHistoryDocFetching,
    isError: isHistoryDocError,
  } = useGetTaskDocumentQuery(
    {
      taskId,
      slug: historySlug ?? "",
      version: selectedHistoryVersion ?? undefined,
    },
    {
      skip: !historyOpen || !historySlug || selectedHistoryVersion === null,
    },
  );
  const currentHistoryDoc =
    selectedHistoryDoc?.slug === historySlug &&
    selectedHistoryDoc.version === selectedHistoryVersion
      ? selectedHistoryDoc
      : undefined;

  const historyRows =
    historyData?.slug === historySlug ? historyData.history : undefined;
  const isHistoryContentLoading =
    selectedHistoryVersion !== null &&
    (isHistoryDocFetching || (!isHistoryDocError && !currentHistoryDoc));

  const closeHistory = useCallback(() => {
    setHistoryOpen(false);
    setSelectedHistoryVersion(null);
  }, []);

  const handleHistoryOpenChange = useCallback(
    (open: boolean) => {
      if (!open) {
        closeHistory();
      } else {
        setHistoryOpen(true);
      }
    },
    [closeHistory],
  );

  const [pinDocument] = usePinTaskDocumentMutation();
  const [deleteDocument] = useDeleteTaskDocumentMutation();

  const sorted = useMemo(() => {
    return [...(data?.documents ?? [])].sort((a, b) => {
      if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
      return b.updated_at.localeCompare(a.updated_at);
    });
  }, [data?.documents]);

  const visible = useMemo(() => {
    return sorted.filter((doc) => {
      if (kindFilter !== ALL_VALUE && doc.kind !== kindFilter) return false;
      if (pinnedOnly && !doc.pinned) return false;
      return true;
    });
  }, [sorted, kindFilter, pinnedOnly]);

  const handleToggleExpand = useCallback((slug: string) => {
    setExpandedSlug((prev) => (prev === slug ? null : slug));
  }, []);

  const handlePin = useCallback(
    async (slug: string, pinned: boolean) => {
      await pinDocument({ taskId, slug, pinned })
        .unwrap()
        .catch(() => undefined);
    },
    [pinDocument, taskId],
  );

  const handleEdit = useCallback((slug: string) => {
    setEditorSlug(slug);
    setEditorMode("edit");
    setEditorOpen(true);
  }, []);

  const handleHistory = useCallback((slug: string) => {
    setHistorySlug(slug);
    setSelectedHistoryVersion(null);
    setHistoryOpen(true);
  }, []);

  const handleDelete = useCallback(
    async (slug: string) => {
      await deleteDocument({ taskId, slug })
        .unwrap()
        .catch(() => undefined);
    },
    [deleteDocument, taskId],
  );

  const handleNewDocument = useCallback(() => {
    setEditorSlug(undefined);
    setEditorMode("create");
    setEditorOpen(true);
  }, []);

  return (
    <div className={`${styles.root} rf-enter`}>
      <Flex justify="between" align="center" gap="2" className={styles.header}>
        <Text weight="bold" size="3">
          {data?.documents.length ?? 0} documents
        </Text>
        <Button
          size="sm"
          variant="soft"
          leftIcon={Plus}
          onClick={handleNewDocument}
        >
          New
        </Button>
      </Flex>

      <Surface
        animated="rise"
        className={styles.filters}
        radius="card"
        variant="glass"
      >
        <Flex gap="2" align="center" className={styles.filterRow} wrap="wrap">
          <Select value={kindFilter} onValueChange={setKindFilter}>
            <Select.Trigger
              aria-label="Kind filter"
              className={styles.filterControl}
            />
            <Select.Content>
              <Select.Item value={ALL_VALUE}>All kinds</Select.Item>
              {DOCUMENT_KINDS.map((k) => (
                <Select.Item key={k} value={k}>
                  {k}
                </Select.Item>
              ))}
            </Select.Content>
          </Select>
          <Checkbox
            checked={pinnedOnly}
            onCheckedChange={(v) => setPinnedOnly(v === true)}
          >
            Pinned only
          </Checkbox>
          {isFetching && <Spinner size="sm" />}
        </Flex>
      </Surface>

      {error && (
        <ErrorState
          title="Failed to load documents."
          variant="compact"
          className={styles.errorState}
        />
      )}

      <Flex direction="column" gap="2" className={`${styles.list} rf-stagger`}>
        {isFetching && !data ? (
          <div className={styles.loadingState}>
            <Spinner />
          </div>
        ) : visible.length > 0 ? (
          visible.map((doc) => (
            <DocumentRow
              key={doc.slug}
              document={doc}
              isExpanded={expandedSlug === doc.slug}
              expandedContent={
                expandedSlug === doc.slug && expandedDoc?.slug === doc.slug
                  ? expandedDoc.content
                  : undefined
              }
              isExpandedLoading={
                expandedSlug === doc.slug &&
                (isExpandedFetching ||
                  (!isExpandedError && expandedDoc?.slug !== doc.slug))
              }
              onToggleExpand={() => handleToggleExpand(doc.slug)}
              onPin={handlePin}
              onEdit={handleEdit}
              onHistory={handleHistory}
              onDelete={handleDelete}
            />
          ))
        ) : (
          <Text as="div" className={styles.emptyState}>
            No documents yet. Click + New to create a plan or design doc.
          </Text>
        )}
      </Flex>

      <DocumentEditor
        taskId={taskId}
        mode={editorMode}
        slug={editorSlug}
        open={editorOpen}
        onOpenChange={setEditorOpen}
      />

      <Dialog open={historyOpen} onOpenChange={handleHistoryOpenChange}>
        <Dialog.Content
          className={styles.historyDialog}
          maxHeight="calc(100dvh - var(--rf-space-5))"
          maxWidth="760px"
        >
          <Dialog.Title>History: {historySlug}</Dialog.Title>
          <Flex gap="3" className={styles.historyBody}>
            <Flex direction="column" gap="2" className={styles.historyList}>
              {isHistoryFetching && historyRows === undefined ? (
                <div className={styles.inlineLoading}>
                  <Spinner size="sm" />
                </div>
              ) : historyRows?.length ? (
                historyRows.map((entry) => (
                  <button
                    key={entry.version}
                    type="button"
                    className={classNames(
                      styles.historyVersionButton,
                      "rf-pressable",
                      selectedHistoryVersion === entry.version &&
                        styles.historyVersionButtonActive,
                    )}
                    onClick={() => setSelectedHistoryVersion(entry.version)}
                  >
                    <Text size="2" weight="medium" as="span">
                      v{entry.version}
                    </Text>
                    <Text size="1" as="span" className={styles.mutedText}>
                      {formatUpdatedAt(entry.updated_at)}
                    </Text>
                  </button>
                ))
              ) : (
                <Text size="2" className={styles.mutedText}>
                  No history available.
                </Text>
              )}
            </Flex>
            <Surface
              className={styles.historyContent}
              radius="card"
              variant="glass"
            >
              {selectedHistoryVersion === null ? (
                <Text size="2" className={styles.mutedText}>
                  Select a version to view its content.
                </Text>
              ) : isHistoryContentLoading ? (
                <div className={styles.inlineLoading}>
                  <Spinner size="sm" />
                </div>
              ) : currentHistoryDoc ? (
                <Markdown canHaveInteractiveElements={false}>
                  {currentHistoryDoc.content}
                </Markdown>
              ) : (
                <Text size="2" className={styles.mutedText}>
                  Historical content is unavailable.
                </Text>
              )}
            </Surface>
          </Flex>
          <Flex justify="end">
            <Dialog.Close asChild>
              <Button size="sm" variant="soft">
                Close
              </Button>
            </Dialog.Close>
          </Flex>
        </Dialog.Content>
      </Dialog>
    </div>
  );
};

export default DocumentsPanel;
