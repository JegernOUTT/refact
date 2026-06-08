import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Dialog,
  FieldSelect,
  FieldText,
} from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import {
  useMergeWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMergeStrategy,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import { tasksApi } from "../../services/refact/tasks";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import { mergeConflictFiles } from "./worktreeConflict";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type MergeWorktreeModalProps = {
  open: boolean;
  worktreeId?: string | null;
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  taskId?: string;
  defaultTargetBranch?: string | null;
  onOpenChange: (open: boolean) => void;
  onMerged?: (response: MergeWorktreeResponse) => void;
  onAskRefact?: (
    files: string[],
    response: MergeWorktreeResponse,
  ) => void | Promise<void>;
  onOpenWorktree?: () => void | Promise<void>;
  closeOnNonInteractiveContentClick?: boolean;
};

function displayWorktreeLabel(
  worktree?: WorktreeMeta | null,
  record?: WorktreeRecordView | null,
): string {
  const branch = record?.meta.branch ?? worktree?.branch;
  if (branch && branch.trim().length > 0) return branch;
  return record?.meta.root ?? worktree?.root ?? "worktree";
}

function initialTargetBranch(
  record?: WorktreeRecordView | null,
  worktree?: WorktreeMeta | null,
  defaultTargetBranch?: string | null,
): string {
  return (
    defaultTargetBranch ??
    record?.meta.base_branch ??
    worktree?.base_branch ??
    "main"
  );
}

function hasMergeConflict(response: MergeWorktreeResponse): boolean {
  return (
    Boolean(response.conflict) ||
    response.has_conflicts === true ||
    response.conflicted === true ||
    response.status === "conflict"
  );
}

function isMerged(response: MergeWorktreeResponse): boolean {
  return response.merged === true && !hasMergeConflict(response);
}

function didCleanupNoop(response: MergeWorktreeResponse): boolean {
  const cleanup = response.cleanup;
  return (
    response.status === "nothing_to_merge" &&
    cleanup != null &&
    (cleanup.worktree_deleted ||
      cleanup.branch_deleted ||
      cleanup.registry_deleted)
  );
}

function responseSummary(response: MergeWorktreeResponse): string {
  if (hasMergeConflict(response)) return "Merge conflicts detected.";
  if (response.merged === true) return "Merge completed.";
  if (response.status === "nothing_to_merge") return "Nothing to merge.";
  return response.message ?? response.status ?? "Merge finished.";
}

function responseWarnings(response: MergeWorktreeResponse): string[] {
  return [...(response.warnings ?? []), ...(response.cleanup?.warnings ?? [])];
}

function resultTone(
  merged: boolean,
  conflicted: boolean,
): React.ComponentProps<typeof Badge>["tone"] {
  if (merged) return "success";
  if (conflicted) return "warning";
  return "muted";
}

export const MergeWorktreeModal: React.FC<MergeWorktreeModalProps> = ({
  open,
  worktreeId,
  worktree,
  record,
  taskId,
  defaultTargetBranch,
  onOpenChange,
  onMerged,
  onAskRefact,
  onOpenWorktree,
  closeOnNonInteractiveContentClick = false,
}) => {
  const dispatch = useAppDispatch();
  const [strategy, setStrategy] = useState<WorktreeMergeStrategy>("squash");
  const [deleteAfterMerge, setDeleteAfterMerge] = useState(true);
  const [includeUncommitted, setIncludeUncommitted] = useState(true);
  const [targetBranch, setTargetBranch] = useState(
    initialTargetBranch(record, worktree, defaultTargetBranch),
  );
  const [result, setResult] = useState<MergeWorktreeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionFeedback, setActionFeedback] = useState<string | null>(null);
  const [mergeWorktree, mergeState] = useMergeWorktreeMutation();
  const queryId = worktreeId ?? record?.meta.id ?? worktree?.id ?? "";
  const sourceWorkspaceRoot =
    record?.meta.source_workspace_root ?? worktree?.source_workspace_root;
  const label = displayWorktreeLabel(worktree, record);
  const conflictFiles = useMemo(
    () => (result ? mergeConflictFiles(result) : []),
    [result],
  );
  const warnings = useMemo(
    () => (result ? responseWarnings(result) : []),
    [result],
  );
  const resolvedTaskId = taskId ?? record?.meta.task_id ?? worktree?.task_id;

  useEffect(() => {
    if (!open) return;
    setStrategy("squash");
    setDeleteAfterMerge(true);
    setIncludeUncommitted(true);
    setTargetBranch(initialTargetBranch(record, worktree, defaultTargetBranch));
    setResult(null);
    setError(null);
    setActionFeedback(null);
  }, [defaultTargetBranch, open, record, worktree]);

  const invalidateTask = useCallback(() => {
    if (!resolvedTaskId) return;
    dispatch(
      tasksApi.util.invalidateTags([
        { type: "Tasks", id: resolvedTaskId },
        { type: "Board", id: resolvedTaskId },
        "Tasks",
      ]),
    );
  }, [dispatch, resolvedTaskId]);

  const handleMerge = useCallback(async () => {
    if (!queryId) {
      setError("No worktree selected.");
      return;
    }
    setResult(null);
    setError(null);
    setActionFeedback(null);
    try {
      const trimmedTargetBranch = targetBranch.trim();
      const response = await mergeWorktree({
        id: queryId,
        source_workspace_root: sourceWorkspaceRoot,
        strategy,
        target_branch:
          trimmedTargetBranch.length > 0 ? trimmedTargetBranch : undefined,
        delete_after_merge: deleteAfterMerge,
        include_uncommitted: includeUncommitted,
        generate_commit_message: true,
      }).unwrap();
      setResult(response);
      invalidateTask();
      if (isMerged(response) || didCleanupNoop(response)) {
        onMerged?.(response);
      }
    } catch (mergeError) {
      setError(worktreeErrorText(mergeError));
    }
  }, [
    deleteAfterMerge,
    includeUncommitted,
    invalidateTask,
    mergeWorktree,
    onMerged,
    queryId,
    sourceWorkspaceRoot,
    strategy,
    targetBranch,
  ]);

  const handleAskRefact = useCallback(async () => {
    if (!result || !onAskRefact) return;
    setActionFeedback(null);
    try {
      await onAskRefact(conflictFiles, result);
      setActionFeedback("Conflict resolution request sent to Refact.");
    } catch (askError) {
      setActionFeedback(`Could not ask Refact: ${worktreeErrorText(askError)}`);
    }
  }, [conflictFiles, onAskRefact, result]);

  const handleOpenWorktree = useCallback(async () => {
    if (!onOpenWorktree) return;
    setActionFeedback(null);
    try {
      await onOpenWorktree();
    } catch (openError) {
      setActionFeedback(
        `Could not open worktree: ${worktreeErrorText(openError)}`,
      );
    }
  }, [onOpenWorktree]);

  const conflicted = result ? hasMergeConflict(result) : false;
  const merged = result ? isMerged(result) : false;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.mergeDialog} maxWidth="900px">
        <div
          {...(closeOnNonInteractiveContentClick
            ? dialogNonInteractiveCloseHandlers(() => onOpenChange(false))
            : {})}
        >
          <Dialog.Title>Merge worktree</Dialog.Title>
          <Dialog.Description>
            Merge {label} into a target branch.
          </Dialog.Description>

          <div className={styles.modalFields}>
            <div className={styles.field}>
              <span className={styles.labelText}>Strategy</span>
              <FieldSelect
                value={strategy}
                options={[
                  { value: "squash", label: "Squash merge" },
                  { value: "merge", label: "Regular merge" },
                ]}
                onChange={(value) =>
                  setStrategy(value as WorktreeMergeStrategy)
                }
                disabled={mergeState.isLoading}
                aria-label="Merge strategy"
              />
            </div>

            <label className={styles.field} htmlFor="worktree-target-branch">
              <span className={styles.labelText}>Target branch</span>
              <FieldText
                id="worktree-target-branch"
                value={targetBranch}
                onChange={setTargetBranch}
                disabled={mergeState.isLoading}
              />
            </label>

            <div className={styles.checkboxStack}>
              <label className={styles.checkboxRow}>
                <input
                  type="checkbox"
                  checked={deleteAfterMerge}
                  onChange={(event) =>
                    setDeleteAfterMerge(event.currentTarget.checked)
                  }
                  disabled={mergeState.isLoading}
                />
                Delete worktree after merge or if there is nothing to merge
              </label>
              <label className={styles.checkboxRow}>
                <input
                  type="checkbox"
                  checked={includeUncommitted}
                  onChange={(event) =>
                    setIncludeUncommitted(event.currentTarget.checked)
                  }
                  disabled={mergeState.isLoading}
                />
                Include uncommitted changes by auto-committing first
              </label>
            </div>

            {error && <p className={styles.errorBox}>{error}</p>}

            {result && (
              <div className={styles.resultBox}>
                <div className={styles.badgeRow}>
                  <Badge tone={resultTone(merged, conflicted)}>
                    {result.status ?? (merged ? "merged" : "finished")}
                  </Badge>
                  {result.strategy && (
                    <Badge tone="muted">{result.strategy}</Badge>
                  )}
                </div>
                <p
                  className={`${styles.resultSummary} ${
                    conflicted
                      ? styles.resultWarning
                      : merged
                        ? styles.resultSuccess
                        : ""
                  }`}
                >
                  {responseSummary(result)}
                </p>
                {result.source_branch && result.target_branch && (
                  <p className={styles.metaText}>
                    {result.source_branch} → {result.target_branch}
                  </p>
                )}
                {result.merge_commit && (
                  <p className={styles.metaText}>
                    Merge commit: {result.merge_commit}
                  </p>
                )}
                {result.cleanup && (
                  <p className={styles.metaText}>
                    Cleanup: worktree{" "}
                    {result.cleanup.worktree_deleted ? "deleted" : "kept"},
                    branch {result.cleanup.branch_deleted ? "deleted" : "kept"}
                  </p>
                )}
                {conflicted && (
                  <div className={styles.conflictBlock}>
                    <span className={styles.labelText}>Conflicted files</span>
                    <ul className={styles.conflictList}>
                      {conflictFiles.length === 0 ? (
                        <li>No conflicted files were reported.</li>
                      ) : (
                        conflictFiles.map((file) => <li key={file}>{file}</li>)
                      )}
                    </ul>
                    <div className={styles.inlineActions}>
                      <Button
                        size="sm"
                        variant="soft"
                        onClick={() => void handleAskRefact()}
                        disabled={!onAskRefact}
                      >
                        Ask Refact to resolve conflicts
                      </Button>
                      <Button
                        size="sm"
                        variant="soft"
                        onClick={() => void handleOpenWorktree()}
                        disabled={!onOpenWorktree}
                      >
                        Open worktree
                      </Button>
                    </div>
                  </div>
                )}
                {actionFeedback && (
                  <p className={styles.metaText}>{actionFeedback}</p>
                )}
                {warnings.length > 0 && (
                  <div className={styles.warningStack}>
                    {warnings.map((warning, index) => (
                      <p
                        key={`${index}-${warning}`}
                        className={styles.warningText}
                      >
                        {warning}
                      </p>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>

          <div className={styles.modalActions}>
            <Dialog.Close asChild>
              <Button variant="soft" disabled={mergeState.isLoading}>
                Close
              </Button>
            </Dialog.Close>
            <Button
              variant="primary"
              onClick={() => void handleMerge()}
              disabled={mergeState.isLoading || queryId.length === 0}
              loading={mergeState.isLoading}
            >
              Merge
            </Button>
          </div>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

MergeWorktreeModal.displayName = "MergeWorktreeModal";
