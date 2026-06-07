import React, { useCallback, useState, type ReactNode } from "react";
import {
  Copy,
  ExternalLink,
  FileText,
  GitMerge,
  LogOut,
  Plus,
  Trash2,
} from "lucide-react";
import { Button, Dialog, Popover } from "../../components/ui";
import {
  useDeleteWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import { selectApiKey, selectConfig } from "../Config/configSlice";
import { selectChatId, setThreadWorktree } from "../Chat/Thread";
import { WorktreeStatusBadge } from "./WorktreeStatusBadge";
import { WorktreeDiffPanel } from "./WorktreeDiffPanel";
import { MergeWorktreeModal } from "./MergeWorktreeModal";
import { buildWorktreeConflictPrompt } from "./worktreeConflict";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type WorktreeMenuProps = {
  currentWorktree: WorktreeMeta | null;
  currentRecord?: WorktreeRecordView | null;
  records: WorktreeRecordView[];
  isLoading: boolean;
  feedback?: string | null;
  canCopyPath: boolean;
  onCreate: () => void;
  onSelect: (record: WorktreeRecordView) => void;
  onDetach: () => void;
  onOpenInNewWindow: () => void;
  onCopyPath: () => void;
};

type ActionButtonProps = {
  label: string;
  title: string;
  icon: ReactNode;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  primary?: boolean;
};

function ActionButton({
  label,
  title,
  icon,
  onClick,
  disabled = false,
  danger = false,
  primary = false,
}: ActionButtonProps) {
  const className = [
    styles.actionButton,
    primary ? styles.actionPrimary : "",
    danger && !disabled ? styles.actionDanger : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      type="button"
      className={className}
      onClick={onClick}
      disabled={disabled}
      aria-label={title}
      title={title}
    >
      <span className={styles.actionIcon} aria-hidden="true">
        {icon}
      </span>
      <span className={styles.actionLabel}>{label}</span>
    </button>
  );
}

function compactPath(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return normalized || path;
  return parts.slice(-2).join("/");
}

function displayName(worktree: WorktreeMeta): string {
  const branch = worktree.branch?.trim();
  return branch !== undefined && branch.length > 0
    ? branch
    : compactPath(worktree.root);
}

function referencesLabel(record: WorktreeRecordView): string {
  if (record.reference_count === 0) return "unused";
  if (record.reference_count === 1) return "1 ref";
  return `${record.reference_count} refs`;
}

function referenceCount(
  worktree: WorktreeMeta | null,
  record?: WorktreeRecordView | null,
): number {
  return record?.reference_count ?? worktree?.reference_count ?? 0;
}

export const WorktreeMenu: React.FC<WorktreeMenuProps> = ({
  currentWorktree,
  currentRecord,
  records,
  isLoading,
  feedback,
  canCopyPath,
  onCreate,
  onSelect,
  onDetach,
  onOpenInNewWindow,
  onCopyPath,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const [diffOpen, setDiffOpen] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [localFeedback, setLocalFeedback] = useState<string | null>(null);
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const sharedCount = referenceCount(currentWorktree, currentRecord);
  const worktreeAvailable = Boolean(currentWorktree);
  const hasFeedback =
    (feedback?.length ?? 0) > 0 || (localFeedback?.length ?? 0) > 0;
  const detachLabel = currentWorktree ? "Detach" : "Main";
  const detachTitle = currentWorktree
    ? "Detach worktree and use main workspace"
    : "Already using main workspace";

  const handleAskRefact = useCallback(
    async (files: string[], response: MergeWorktreeResponse) => {
      if (!currentWorktree || !chatId) {
        throw new Error("No active worktree chat is available.");
      }
      const prompt = buildWorktreeConflictPrompt({
        worktree: currentWorktree,
        record: currentRecord,
        response,
        files,
      });
      await sendUserMessage(chatId, prompt, config, apiKey, true);
      setLocalFeedback("Conflict resolution request sent to Refact.");
    },
    [apiKey, chatId, config, currentRecord, currentWorktree],
  );

  const handleDelete = useCallback(async () => {
    if (!currentWorktree) return;
    setLocalFeedback(null);
    try {
      await deleteWorktree({
        id: currentWorktree.id,
        source_workspace_root: currentWorktree.source_workspace_root,
        delete_branch: deleteBranch,
        force_referenced: true,
      }).unwrap();
      setDeleteOpen(false);
      setLocalFeedback("Worktree deleted.");
      if (chatId && currentWorktree.id) {
        dispatch(setThreadWorktree({ chatId, worktree: null }));
        onDetach();
      }
    } catch (error) {
      setLocalFeedback(`Delete failed: ${worktreeErrorText(error)}`);
    }
  }, [
    chatId,
    currentWorktree,
    deleteBranch,
    deleteWorktree,
    dispatch,
    onDetach,
  ]);

  const handleMerged = useCallback(
    (response: MergeWorktreeResponse) => {
      const cleanup = response.cleanup;
      if (
        (cleanup?.worktree_deleted ?? false) ||
        (cleanup?.registry_deleted ?? false)
      ) {
        if (chatId) {
          dispatch(setThreadWorktree({ chatId, worktree: null }));
        }
        onDetach();
        setMergeOpen(false);
        setLocalFeedback("Worktree merged and removed.");
      } else if (response.merged === true) {
        setLocalFeedback("Worktree merged.");
      }
    },
    [chatId, dispatch, onDetach],
  );

  return (
    <>
      <Popover.Content
        className={styles.content}
        side="top"
        align="start"
        sideOffset={8}
        scrollable={false}
      >
        <div className={styles.menu}>
          <div className={styles.menuHeader}>
            <span className={styles.titleText}>Worktrees</span>
            {currentWorktree && currentRecord && (
              <WorktreeStatusBadge
                worktree={currentWorktree}
                record={currentRecord}
              />
            )}
          </div>
          <p className={styles.menuHint}>
            Paths warn/remap; shell uses scoped cwd; shared refs affect all
            chats.
          </p>

          {hasFeedback && (
            <div className={styles.feedbackStack}>
              {feedback && <p className={styles.feedbackText}>{feedback}</p>}
              {localFeedback && (
                <p className={styles.feedbackText}>{localFeedback}</p>
              )}
            </div>
          )}

          <div className={styles.actionGrid}>
            <ActionButton
              label="Create"
              title="Create worktree"
              icon={<Plus />}
              onClick={onCreate}
              primary
            />
            <ActionButton
              label={detachLabel}
              title={detachTitle}
              icon={<LogOut />}
              onClick={onDetach}
              disabled={!currentWorktree}
            />
            <ActionButton
              label="Open"
              title="Open worktree in new window"
              icon={<ExternalLink />}
              onClick={onOpenInNewWindow}
              disabled={!currentWorktree}
            />
            <ActionButton
              label="Copy"
              title="Copy workspace path"
              icon={<Copy />}
              onClick={onCopyPath}
              disabled={!canCopyPath}
            />
          </div>

          <div className={styles.separator} />

          <div className={styles.section}>
            <span className={styles.sectionHeader}>Existing</span>
            <div className={styles.list}>
              {isLoading && <span className={styles.emptyText}>Loading...</span>}
              {!isLoading && records.length === 0 && (
                <span className={styles.emptyText}>None yet</span>
              )}
              {records.map((record) => {
                const selected = currentWorktree?.id === record.meta.id;
                const title = displayName(record.meta);
                const usedBy = record.referencing_chat_ids?.length
                  ? record.referencing_chat_ids.join(", ")
                  : record.references
                      .map((reference) => reference.chat_id)
                      .filter((value): value is string => Boolean(value))
                      .join(", ");
                return (
                  <button
                    key={record.meta.id}
                    type="button"
                    className={`${styles.item} ${
                      selected ? styles.itemSelected : ""
                    }`}
                    onClick={() => onSelect(record)}
                    aria-label={`Select worktree ${title}`}
                    aria-current={selected ? "true" : undefined}
                    title={`Use ${title}`}
                  >
                    <span className={styles.itemTitle}>
                      <span className={styles.itemHeader}>
                        <span className={styles.itemName}>{title}</span>
                        <WorktreeStatusBadge
                          worktree={record.meta}
                          record={record}
                        />
                      </span>
                      <span className={styles.path}>{record.meta.root}</span>
                      <span className={styles.metaText}>
                        {referencesLabel(record)}
                        {usedBy ? ` · used by ${usedBy}` : ""}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className={styles.separator} />

          <div className={styles.reviewActions}>
            <ActionButton
              label="Diff"
              title="View worktree diff"
              icon={<FileText />}
              onClick={() => setDiffOpen(true)}
              disabled={!worktreeAvailable}
            />
            <ActionButton
              label="Merge"
              title="Merge worktree"
              icon={<GitMerge />}
              onClick={() => setMergeOpen(true)}
              disabled={!worktreeAvailable}
            />
            <ActionButton
              label="Delete"
              title="Delete or discard worktree"
              icon={<Trash2 />}
              onClick={() => setDeleteOpen(true)}
              disabled={!worktreeAvailable}
              danger={worktreeAvailable}
            />
          </div>

          {sharedCount > 1 ? (
            <p className={styles.feedbackText}>
              Shared by {sharedCount} references. Delete and discard actions can
              affect other chats.
            </p>
          ) : null}
        </div>
      </Popover.Content>

      <WorktreeDiffPanel
        open={diffOpen}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setDiffOpen}
        closeOnNonInteractiveContentClick
      />

      <MergeWorktreeModal
        open={mergeOpen}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setMergeOpen}
        onMerged={handleMerged}
        onAskRefact={handleAskRefact}
        onOpenWorktree={onOpenInNewWindow}
        closeOnNonInteractiveContentClick
      />

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <Dialog.Content maxWidth="420px">
          <div {...dialogNonInteractiveCloseHandlers(() => setDeleteOpen(false))}>
            <Dialog.Title>Delete worktree</Dialog.Title>
            <Dialog.Description>
              Delete or discard the selected worktree from disk.
            </Dialog.Description>

            <div className={styles.modalFields}>
              <div className={styles.dialogOverlayText}>
                <span className={styles.labelText}>
                  {currentWorktree ? displayName(currentWorktree) : "No worktree"}
                </span>
                {currentWorktree && (
                  <span className={styles.path}>{currentWorktree.root}</span>
                )}
              </div>

              {sharedCount > 1 && (
                <p className={styles.warningBox}>
                  This worktree is shared by {sharedCount} references. Deleting it
                  may affect other chats that use the same worktree.
                </p>
              )}

              <label className={styles.checkboxRow}>
                <input
                  type="checkbox"
                  checked={deleteBranch}
                  onChange={(event) => setDeleteBranch(event.currentTarget.checked)}
                  disabled={deleteState.isLoading}
                />
                Delete git branch too
              </label>

              {localFeedback && localFeedback.startsWith("Delete failed") && (
                <p className={styles.errorBox}>{localFeedback}</p>
              )}
            </div>

            <div className={styles.modalActions}>
              <Dialog.Close asChild>
                <Button variant="soft" disabled={deleteState.isLoading}>
                  Cancel
                </Button>
              </Dialog.Close>
              <Button
                variant="danger"
                onClick={() => void handleDelete()}
                disabled={!currentWorktree || deleteState.isLoading}
                loading={deleteState.isLoading}
              >
                {deleteState.isLoading ? "Deleting..." : "Delete worktree"}
              </Button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog>
    </>
  );
};

WorktreeMenu.displayName = "WorktreeMenu";
