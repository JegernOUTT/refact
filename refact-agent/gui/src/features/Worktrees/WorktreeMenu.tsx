import React, { useCallback, useRef, useEffect, useState } from "react";
import { CircleHelp, GitBranch, Plus } from "lucide-react";
import { Button, Dialog, Popover } from "../../components/ui";
import {
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
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectApiKey, selectConfig, selectHost } from "../Config/configSlice";
import { WorktreeCard } from "./WorktreeCard";
import {
  sameWorktree,
  selectedFirst,
  worktreeInspection,
} from "./WorktreeCard.helpers";
import { openWorktreeDiff } from "./worktreeDiffSlice";
import type { WorktreeInventory } from "../../services/refact/worktrees";
import {
  MergeWorktreeModal,
  type WorktreeConflictHandoff,
} from "./MergeWorktreeModal";
import { buildWorktreeConflictPrompt } from "./worktreeConflict";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type WorktreeMenuProps = {
  chatId: string;
  inventory?: WorktreeInventory;
  mainPath?: string;
  mainBranch?: string | null;
  onClose?: () => void;
  sourceIsCurrent?: () => boolean;
  isAttached?: (meta: WorktreeMeta) => boolean;
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
  onOpenInNewWindow: (target?: WorktreeMeta) => void;
  onCopyPath: (target?: WorktreeMeta) => void;
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

export const WorktreeMenu: React.FC<WorktreeMenuProps> = ({
  chatId,
  inventory,
  mainPath,
  mainBranch,
  onClose,
  sourceIsCurrent,
  isAttached,
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
  const host = useAppSelector(selectHost);
  const openCopiesPath = !["vscode", "jetbrains", "ide"].includes(host);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const dispatch = useAppDispatch();
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const valid = useCallback(
    () => alive.current && (sourceIsCurrent?.() ?? true),
    [sourceIsCurrent],
  );
  const [target, setTarget] = useState<{
    meta: WorktreeMeta;
    record?: WorktreeRecordView | null;
  } | null>(null);
  const targetMeta = target?.meta;
  const targetRecord = target?.record;
  const targetAttached =
    targetMeta &&
    (isAttached?.(targetMeta) ?? sameWorktree(targetMeta, currentWorktree));
  const [mergeOpen, setMergeOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [localFeedback, setLocalFeedback] = useState<string | null>(null);
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const sharedCount =
    targetRecord?.reference_count ?? targetMeta?.reference_count ?? 0;
  const scopeChangeDisabled =
    Boolean(scopeChangeBlockedReason) || scopeChangePending === true;
  const hasFeedback =
    (feedback?.length ?? 0) > 0 || (localFeedback?.length ?? 0) > 0;
  const handleAskRefact = useCallback(
    async (
      files: string[],
      response: MergeWorktreeResponse,
      handoff: WorktreeConflictHandoff,
    ) => {
      if (!targetMeta || !targetAttached || !chatId || !valid()) {
        throw new Error("No active worktree chat is available.");
      }
      const prompt = buildWorktreeConflictPrompt({
        worktree: targetMeta,
        record: targetRecord,
        response,
        sync: handoff.sync,
        files,
      });
      await sendUserMessage(chatId, prompt, config, apiKey, true);
      setLocalFeedback("Conflict resolution request sent to Refact.");
    },
    [apiKey, chatId, config, targetRecord, targetMeta, targetAttached, valid],
  );

  const handleDelete = useCallback(async () => {
    if (
      !targetMeta ||
      scopeChangeDisabled ||
      targetMeta.kind === "task_agent" ||
      !valid()
    )
      return;
    setLocalFeedback(null);
    try {
      const response = await deleteWorktree({
        id: targetMeta.id,
        source_workspace_root: targetMeta.source_workspace_root,
        delete_branch: deleteBranch,
        force_referenced: true,
      }).unwrap();
      if (!valid()) return;
      if (!response.deleted) {
        setLocalFeedback(
          "Delete not confirmed; worktree retained. Inspect it before retrying.",
        );
        return;
      }
      setDeleteOpen(false);
      setLocalFeedback(
        response.warnings.length
          ? `Worktree deleted. ${response.warnings.join(" ")}`
          : "Worktree deleted.",
      );
      if (
        isAttached?.(targetMeta) ??
        sameWorktree(targetMeta, currentWorktree)
      ) {
        onDetach();
      }
    } catch (error) {
      if (!valid()) return;
      setLocalFeedback(`Delete failed: ${worktreeErrorText(error)}`);
    }
  }, [
    currentWorktree,
    targetMeta,
    isAttached,
    deleteBranch,
    deleteWorktree,
    onDetach,
    scopeChangeDisabled,
    valid,
  ]);

  const handleMerged = useCallback(
    (response: MergeWorktreeResponse) => {
      if (!valid()) return;
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
        if (
          targetMeta &&
          (isAttached?.(targetMeta) ??
            sameWorktree(targetMeta, currentWorktree))
        )
          onDetach();
      }
    },
    [onDetach, targetMeta, currentWorktree, isAttached, valid],
  );

  return (
    <>
      <Popover.Content
        className={kit.popover}
        side="top"
        align="start"
        sideOffset={8}
        maxWidth="min(400px, calc(100vw - 2 * var(--rf-space-3)))"
        scrollable={false}
        onOpenAutoFocus={(event) => {
          const content = event.currentTarget;
          const row =
            content instanceof HTMLElement
              ? content.querySelector<HTMLElement>(
                  "[data-worktree-selected]:not(:disabled)",
                )
              : null;
          if (row) {
            event.preventDefault();
            row.focus();
            row.scrollIntoView({ block: "nearest" });
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
          <KitDivider />
          <KitSection>Checkouts</KitSection>
          <div className={styles.recordList}>
            {!currentWorktree && (
              <WorktreeCard
                selected
                openCopiesPath
                mainPath={mainPath}
                mainBranch={mainBranch}
                onSelect={onDetach}
                selectionDisabled={scopeChangeDisabled}
                onCopy={canCopyPath ? () => onCopyPath() : undefined}
                onOpen={canCopyPath ? () => onOpenInNewWindow() : undefined}
              />
            )}
            {[
              ...(currentWorktree &&
              !records.some((r) => sameWorktree(r.meta, currentWorktree))
                ? [
                    {
                      meta: currentWorktree,
                      record: currentRecord,
                      unavailable: true,
                    },
                  ]
                : []),
              ...selectedFirst(records, currentWorktree).map((record) => ({
                meta: record.meta,
                record,
                unavailable: false,
              })),
            ].map(({ meta, record, unavailable }) => (
              <WorktreeCard
                openCopiesPath={openCopiesPath}
                key={`${meta.source_workspace_root}:${meta.id}`}
                worktree={meta}
                record={record}
                inspection={worktreeInspection(inventory, meta)}
                selected={sameWorktree(meta, currentWorktree)}
                unavailable={unavailable}
                selectionDisabled={scopeChangeDisabled}
                managementDisabled={scopeChangeDisabled}
                onSelect={record ? () => onSelect(record) : () => undefined}
                onDiff={() => {
                  dispatch(
                    openWorktreeDiff({
                      worktreeId: meta.id,
                      sourceWorkspaceRoot: meta.source_workspace_root,
                      root: meta.root,
                      label: displayName(meta),
                    }),
                  );
                  onClose?.();
                }}
                onMerge={() => {
                  setTarget({ meta, record });
                  setMergeOpen(true);
                }}
                onDelete={() => {
                  setTarget({ meta, record });
                  setDeleteBranch(false);
                  setLocalFeedback(null);
                  setDeleteOpen(true);
                }}
                onCopy={() => onCopyPath(meta)}
                onOpen={() => onOpenInNewWindow(meta)}
              />
            ))}
            {currentWorktree && (
              <WorktreeCard
                selected={false}
                openCopiesPath
                mainPath={mainPath}
                mainBranch={mainBranch}
                onSelect={onDetach}
                selectionDisabled={scopeChangeDisabled}
                onCopy={canCopyPath ? () => onCopyPath() : undefined}
                onOpen={canCopyPath ? () => onOpenInNewWindow() : undefined}
              />
            )}
            {isLoading ? <KitStaticRow tone="muted" label="Loading…" /> : null}
          </div>

          {sharedCount > 1 ? (
            <KitNote>
              Shared by {sharedCount} references. Delete and discard actions can
              affect other chats.
            </KitNote>
          ) : null}
        </KitPanel>
      </Popover.Content>

      <MergeWorktreeModal
        open={mergeOpen && !scopeChangeDisabled}
        worktreeId={targetMeta?.id}
        worktree={targetMeta}
        record={targetRecord}
        onOpenChange={setMergeOpen}
        onMerged={handleMerged}
        onAskRefact={targetAttached ? handleAskRefact : undefined}
        onOpenWorktree={() => onOpenInNewWindow(targetMeta)}
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
                  {targetMeta ? displayName(targetMeta) : "No worktree"}
                </span>
                {targetMeta && (
                  <span className={styles.path}>{targetMeta.root}</span>
                )}
              </div>

              {sharedCount > 0 && (
                <p className={styles.warningBox}>
                  {sharedCount === 1
                    ? "This worktree has 1 reference."
                    : `This worktree is shared by ${sharedCount} references.`}{" "}
                  Deleting it may affect chats that use this worktree.
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

              {localFeedback &&
                (localFeedback.startsWith("Delete failed") ||
                  localFeedback.startsWith("Delete not confirmed")) && (
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
                  !targetMeta || deleteState.isLoading || scopeChangeDisabled
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
