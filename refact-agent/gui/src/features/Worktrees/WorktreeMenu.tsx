import React, { useCallback, useState } from "react";
import {
  CircleHelp,
  Copy,
  ExternalLink,
  GitBranch,
  LogOut,
  Plus,
} from "lucide-react";
import { Button, Dialog, Popover } from "../../components/ui";
import {
  KitButton,
  KitDivider,
  KitHeader,
  KitNote,
  KitPanel,
  KitRow,
  KitSection,
  KitStaticRow,
  KitTooltip,
  kit,
} from "../../components/ComposerKit";
import {
  useDeleteWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { useAppSelector } from "../../hooks";
import { selectApiKey, selectConfig } from "../Config/configSlice";
import { WorktreeStatusBadge } from "./WorktreeStatusBadge";
import { WorktreeDiffPanel } from "./WorktreeDiffPanel";
import {
  MergeWorktreeModal,
  type WorktreeConflictHandoff,
} from "./MergeWorktreeModal";
import { buildWorktreeConflictPrompt } from "./worktreeConflict";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type WorktreeMenuProps = {
  chatId: string;
  currentWorktree: WorktreeMeta | null;
  currentRecord?: WorktreeRecordView | null;
  records: WorktreeRecordView[];
  isLoading: boolean;
  feedback?: string | null;
  inventoryNotice?: string | null;
  isRefreshing?: boolean;
  onRefresh?: () => void;
  scopeChangeBlockedReason?: string | null;
  scopeChangePending?: boolean;
  canCopyPath: boolean;
  onCreate: () => void;
  onSelect: (record: WorktreeRecordView) => void;
  onDetach: () => void;
  onOpenInNewWindow: () => void;
  onCopyPath: () => void;
};

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
  chatId,
  currentWorktree,
  currentRecord,
  records,
  isLoading,
  feedback,
  inventoryNotice,
  isRefreshing,
  onRefresh,
  scopeChangeBlockedReason,
  scopeChangePending,
  canCopyPath,
  onCreate,
  onSelect,
  onDetach,
  onOpenInNewWindow,
  onCopyPath,
}) => {
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const [diffOpen, setDiffOpen] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [localFeedback, setLocalFeedback] = useState<string | null>(null);
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const sharedCount = referenceCount(currentWorktree, currentRecord);
  const scopeChangeDisabled =
    Boolean(scopeChangeBlockedReason) || scopeChangePending === true;
  const hasFeedback =
    (feedback?.length ?? 0) > 0 || (localFeedback?.length ?? 0) > 0;
  const detachLabel = currentWorktree ? "Back to main" : "Using main workspace";
  const detachTitle = currentWorktree
    ? "Detach worktree and use main workspace"
    : "Already using main workspace";

  const handleAskRefact = useCallback(
    async (
      files: string[],
      response: MergeWorktreeResponse,
      handoff: WorktreeConflictHandoff,
    ) => {
      if (!currentWorktree || !chatId) {
        throw new Error("No active worktree chat is available.");
      }
      const prompt = buildWorktreeConflictPrompt({
        worktree: currentWorktree,
        record: currentRecord,
        response,
        sync: handoff.sync,
        toolAvailable: handoff.toolAvailable,
        files,
        selector: { worktree_id: handoff.sync.id },
      });
      await sendUserMessage(chatId, prompt, config, apiKey, true);
      setLocalFeedback("Conflict resolution request sent to Refact.");
    },
    [apiKey, chatId, config, currentRecord, currentWorktree],
  );

  const handleDelete = useCallback(async () => {
    if (!currentWorktree || scopeChangeDisabled) return;
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
    onDetach,
    scopeChangeDisabled,
  ]);

  const handleMerged = useCallback(
    (response: MergeWorktreeResponse) => {
      const cleanup = response.cleanup;
      const summary =
        response.merged === true ? "Worktree merged." : "Nothing to merge.";
      const hasWarnings =
        (response.warnings?.length ?? 0) > 0 ||
        (cleanup?.warnings.length ?? 0) > 0;
      setLocalFeedback(
        hasWarnings
          ? `${summary} Review the merge result for cleanup details and warnings.`
          : summary,
      );
      if (
        (cleanup?.worktree_deleted ?? false) ||
        (cleanup?.registry_deleted ?? false)
      ) {
        onDetach();
      }
    },
    [onDetach],
  );

  const recordActions = (
    <span className={styles.recordActions}>
      <KitButton
        className={styles.recordAction}
        aria-label="View worktree diff"
        onClick={() => setDiffOpen(true)}
      >
        Diff
      </KitButton>
      <KitButton
        className={styles.recordAction}
        aria-label="Merge worktree"
        disabled={scopeChangeDisabled}
        onClick={() => setMergeOpen(true)}
      >
        Merge
      </KitButton>
      <KitButton
        className={styles.recordAction}
        variant="danger"
        aria-label="Delete or discard worktree"
        disabled={scopeChangeDisabled}
        onClick={() => setDeleteOpen(true)}
      >
        Delete
      </KitButton>
    </span>
  );

  return (
    <>
      <Popover.Content
        className={kit.popover}
        side="top"
        align="start"
        sideOffset={8}
        maxWidth="min(340px, calc(100vw - 2 * var(--rf-space-3)))"
        scrollable={false}
        onOpenAutoFocus={(event) => {
          const content = event.currentTarget;
          const row =
            content instanceof HTMLElement
              ? content.querySelector<HTMLElement>(`.${kit.row}:not(:disabled)`)
              : null;
          if (row) {
            event.preventDefault();
            row.focus();
          }
        }}
      >
        <KitPanel className={styles.menuPanel}>
          <KitHeader
            title={
              <>
                <GitBranch aria-hidden="true" size={13} />
                {currentWorktree
                  ? displayName(currentWorktree)
                  : "Main workspace"}
              </>
            }
          >
            <KitTooltip label="Paths warn and remap; shell uses the scoped cwd; shared refs affect all chats.">
              <button
                type="button"
                className={styles.helpButton}
                aria-label="About worktrees"
              >
                <CircleHelp aria-hidden="true" size={13} />
              </button>
            </KitTooltip>
          </KitHeader>

          {scopeChangeBlockedReason ? (
            <div role="status">
              <KitStaticRow tone="muted" label={scopeChangeBlockedReason} />
            </div>
          ) : null}
          {inventoryNotice ? (
            <>
              <div role="status">
                <KitStaticRow tone="muted" label={inventoryNotice} />
              </div>
              {onRefresh ? (
                <KitRow
                  label="Refresh worktree list"
                  disabled={isRefreshing}
                  onClick={onRefresh}
                />
              ) : null}
            </>
          ) : null}
          {hasFeedback ? (
            <div role="status">
              {feedback ? <KitNote>{feedback}</KitNote> : null}
              {localFeedback ? <KitNote>{localFeedback}</KitNote> : null}
            </div>
          ) : null}

          <KitRow
            icon={<Plus aria-hidden="true" />}
            label="New worktree…"
            aria-label="Create worktree"
            onClick={onCreate}
            disabled={scopeChangeDisabled}
          />
          <KitRow
            icon={<LogOut aria-hidden="true" />}
            label={detachLabel}
            aria-label={detachTitle}
            onClick={onDetach}
            disabled={!currentWorktree || scopeChangeDisabled}
          />
          <KitRow
            icon={<ExternalLink aria-hidden="true" />}
            label="Open folder"
            aria-label="Open worktree in new window"
            onClick={onOpenInNewWindow}
            disabled={!currentWorktree}
          />
          <KitRow
            icon={<Copy aria-hidden="true" />}
            label="Copy path"
            aria-label="Copy workspace path"
            onClick={onCopyPath}
            disabled={!canCopyPath}
          />

          <KitDivider />
          <KitSection>Existing</KitSection>
          <div className={styles.recordList}>
            {currentWorktree &&
            !records.some((record) => record.meta.id === currentWorktree.id) ? (
              <div className={styles.recordRow}>
                <KitStaticRow
                  icon={<GitBranch aria-hidden="true" />}
                  label={displayName(currentWorktree)}
                  strong
                  sub="Attached · not in this list"
                />
                {recordActions}
              </div>
            ) : null}
            {isLoading ? <KitStaticRow tone="muted" label="Loading…" /> : null}
            {!isLoading && records.length === 0 ? (
              <KitStaticRow tone="muted" label="None yet" />
            ) : null}
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
                <div key={record.meta.id} className={styles.recordRow}>
                  <KitRow
                    icon={<GitBranch aria-hidden="true" />}
                    label={title}
                    strong={selected}
                    sub={`${referencesLabel(record)}${
                      usedBy ? ` · used by ${usedBy}` : ""
                    }`}
                    badges={
                      <WorktreeStatusBadge
                        worktree={record.meta}
                        record={record}
                      />
                    }
                    onClick={() => onSelect(record)}
                    aria-label={`Select worktree ${title}`}
                    aria-current={selected ? "true" : undefined}
                    title={
                      record.meta.kind === "task_agent"
                        ? "Managed by task lifecycle"
                        : record.meta.root
                    }
                    disabled={
                      scopeChangeDisabled || record.meta.kind === "task_agent"
                    }
                  />
                  {selected ? recordActions : null}
                </div>
              );
            })}
          </div>

          {sharedCount > 1 ? (
            <KitNote>
              Shared by {sharedCount} references. Delete and discard actions can
              affect other chats.
            </KitNote>
          ) : null}
        </KitPanel>
      </Popover.Content>

      <WorktreeDiffPanel
        open={diffOpen}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setDiffOpen}
      />

      <MergeWorktreeModal
        open={mergeOpen && !scopeChangeDisabled}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setMergeOpen}
        onMerged={handleMerged}
        onAskRefact={handleAskRefact}
        onOpenWorktree={onOpenInNewWindow}
      />

      <Dialog
        open={deleteOpen && !scopeChangeDisabled}
        onOpenChange={setDeleteOpen}
      >
        <Dialog.Content maxWidth="420px">
          <div>
            <Dialog.Title>Delete worktree</Dialog.Title>
            <Dialog.Description>
              Delete or discard the selected worktree from disk.
            </Dialog.Description>

            <div className={styles.modalFields}>
              <div className={styles.dialogOverlayText}>
                <span className={styles.labelText}>
                  {currentWorktree
                    ? displayName(currentWorktree)
                    : "No worktree"}
                </span>
                {currentWorktree && (
                  <span className={styles.path}>{currentWorktree.root}</span>
                )}
              </div>

              {sharedCount > 1 && (
                <p className={styles.warningBox}>
                  This worktree is shared by {sharedCount} references. Deleting
                  it may affect other chats that use the same worktree.
                </p>
              )}

              <label className={styles.checkboxRow}>
                <input
                  type="checkbox"
                  checked={deleteBranch}
                  onChange={(event) =>
                    setDeleteBranch(event.currentTarget.checked)
                  }
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
                disabled={
                  !currentWorktree ||
                  deleteState.isLoading ||
                  scopeChangeDisabled
                }
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
