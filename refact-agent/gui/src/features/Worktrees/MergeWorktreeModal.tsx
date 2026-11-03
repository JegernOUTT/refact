import { TriangleAlert } from "lucide-react";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { FieldText } from "../../components/ui";
import {
  KitBadge,
  KitButton,
  KitCheckRow,
  KitDialog,
  KitDialogCancel,
  KitDivider,
  KitRow,
  KitSection,
  KitStaticRow,
  kit,
  type KitTone,
} from "../../components/ComposerKit";
import { useAppDispatch } from "../../hooks";
import {
  useMergeWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMergeStrategy,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import { tasksApi } from "../../services/refact/tasks";
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

function isCompletedNoop(
  response: MergeWorktreeResponse,
  deleteRequested: boolean,
): boolean {
  const cleanup = response.cleanup;
  return (
    response.status === "nothing_to_merge" &&
    (!deleteRequested ||
      cleanup == null ||
      cleanup.worktree_deleted ||
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
  hasWarnings: boolean,
): KitTone {
  if (conflicted || hasWarnings) return "warning";
  if (merged) return "success";
  return "muted";
}

const MERGE_STRATEGIES: {
  value: WorktreeMergeStrategy;
  label: string;
  hint: string;
}[] = [
  {
    value: "squash",
    label: "Squash merge",
    hint: "Combine all commits into one",
  },
  {
    value: "merge",
    label: "Regular merge",
    hint: "Keep the commit history",
  },
];

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
}) => {
  const dispatch = useAppDispatch();
  const [strategy, setStrategy] = useState<WorktreeMergeStrategy>("squash");
  const [deleteAfterMerge, setDeleteAfterMerge] = useState(true);
  const [includeUncommitted, setIncludeUncommitted] = useState(true);
  const [targetBranch, setTargetBranch] = useState(
    initialTargetBranch(record, worktree, defaultTargetBranch),
  );
  const [completedResult, setCompletedResult] = useState<{
    response: MergeWorktreeResponse;
    id: string;
    label: string;
  } | null>(null);
  const result = completedResult?.response ?? null;
  const [error, setError] = useState<string | null>(null);
  const [actionFeedback, setActionFeedback] = useState<string | null>(null);
  const initializedWorktreeId = useRef<string | null>(null);
  const [mergeWorktree, mergeState] = useMergeWorktreeMutation();
  const queryId = worktreeId ?? record?.meta.id ?? worktree?.id ?? "";
  const sourceWorkspaceRoot =
    record?.meta.source_workspace_root ?? worktree?.source_workspace_root;
  const label =
    completedResult?.label ?? displayWorktreeLabel(worktree, record);
  const canActOnWorktree =
    queryId.length > 0 &&
    (!completedResult || completedResult.id === queryId) &&
    !result?.cleanup?.worktree_deleted &&
    !result?.cleanup?.registry_deleted &&
    !result?.deleted &&
    !record?.meta.deleted &&
    !record?.status.deleted &&
    !worktree?.status?.deleted &&
    !worktree?.deleted;
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
    if (!open) {
      initializedWorktreeId.current = null;
      return;
    }
    if (
      initializedWorktreeId.current === queryId ||
      (!queryId && initializedWorktreeId.current !== null)
    ) {
      return;
    }
    initializedWorktreeId.current = queryId;
    setStrategy("squash");
    setDeleteAfterMerge(true);
    setIncludeUncommitted(true);
    setTargetBranch(initialTargetBranch(record, worktree, defaultTargetBranch));
    setCompletedResult(null);
    setError(null);
    setActionFeedback(null);
  }, [defaultTargetBranch, open, queryId, record, worktree]);

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
    if (!canActOnWorktree || mergeState.isLoading) return;
    setCompletedResult(null);
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
      setCompletedResult({ response, id: queryId, label });
      invalidateTask();
      const completed =
        !hasMergeConflict(response) &&
        response.success !== false &&
        (isMerged(response) || isCompletedNoop(response, deleteAfterMerge));
      if (completed) {
        onMerged?.(response);
      }
      if (completed && responseWarnings(response).length === 0) {
        onOpenChange(false);
      }
    } catch (mergeError) {
      setError(worktreeErrorText(mergeError));
    }
  }, [
    canActOnWorktree,
    deleteAfterMerge,
    includeUncommitted,
    invalidateTask,
    label,
    mergeState.isLoading,
    mergeWorktree,
    onMerged,
    onOpenChange,
    queryId,
    sourceWorkspaceRoot,
    strategy,
    targetBranch,
  ]);

  const handleAskRefact = useCallback(async () => {
    if (!result || !onAskRefact || !canActOnWorktree) return;
    setActionFeedback(null);
    try {
      await onAskRefact(conflictFiles, result);
      setActionFeedback("Conflict resolution request sent to Refact.");
    } catch (askError) {
      setActionFeedback(`Could not ask Refact: ${worktreeErrorText(askError)}`);
    }
  }, [canActOnWorktree, conflictFiles, onAskRefact, result]);

  const handleOpenWorktree = useCallback(async () => {
    if (!onOpenWorktree || !canActOnWorktree) return;
    setActionFeedback(null);
    try {
      await onOpenWorktree();
    } catch (openError) {
      setActionFeedback(
        `Could not open worktree: ${worktreeErrorText(openError)}`,
      );
    }
  }, [canActOnWorktree, onOpenWorktree]);

  const conflicted = result ? hasMergeConflict(result) : false;
  const merged = result ? isMerged(result) : false;

  const isBusy = mergeState.isLoading;
  const controlsDisabled = isBusy || !canActOnWorktree;

  return (
    <KitDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Merge worktree"
      description={`Merge ${label} into a target branch.`}
      width="560px"
      closeLabel="Close dialog"
      footer={
        <>
          <KitDialogCancel disabled={isBusy}>Close</KitDialogCancel>
          <KitButton
            variant="primary"
            onClick={() => void handleMerge()}
            disabled={controlsDisabled}
            aria-busy={isBusy || undefined}
          >
            Merge
          </KitButton>
        </>
      }
    >
      <KitSection>Strategy</KitSection>
      <div
        role="radiogroup"
        aria-label="Merge strategy"
        onKeyDown={(event) => {
          const step =
            event.key === "ArrowDown" || event.key === "ArrowRight"
              ? 1
              : event.key === "ArrowUp" || event.key === "ArrowLeft"
                ? -1
                : 0;
          if (step === 0 || controlsDisabled) return;
          event.preventDefault();
          const index = MERGE_STRATEGIES.findIndex(
            (option) => option.value === strategy,
          );
          const next =
            MERGE_STRATEGIES[
              (index + step + MERGE_STRATEGIES.length) % MERGE_STRATEGIES.length
            ];
          setStrategy(next.value);
          event.currentTarget
            .querySelector<HTMLElement>(`[data-strategy="${next.value}"]`)
            ?.focus();
        }}
      >
        {MERGE_STRATEGIES.map((option) => (
          <KitRow
            key={option.value}
            role="radio"
            data-strategy={option.value}
            aria-checked={strategy === option.value}
            tabIndex={strategy === option.value ? 0 : -1}
            label={option.label}
            hint={option.hint}
            selected={strategy === option.value}
            disabled={controlsDisabled}
            onClick={() => setStrategy(option.value)}
          />
        ))}
      </div>

      <label className={styles.field}>
        <span className={kit.section}>Target branch</span>
        <span className={styles.fieldControl}>
          <FieldText
            value={targetBranch}
            onChange={setTargetBranch}
            disabled={controlsDisabled}
          />
        </span>
      </label>

      <KitDivider />
      <KitCheckRow
        label="Delete worktree after merge"
        hint="Also applies when there is nothing to merge"
        ariaLabel="Delete worktree after merge or if there is nothing to merge"
        checked={deleteAfterMerge}
        onCheckedChange={setDeleteAfterMerge}
        disabled={controlsDisabled}
      />
      <KitCheckRow
        label="Include uncommitted changes"
        hint="Auto-commit them first"
        ariaLabel="Include uncommitted changes by auto-committing first"
        checked={includeUncommitted}
        onCheckedChange={setIncludeUncommitted}
        disabled={controlsDisabled}
      />

      {error ? (
        <p className={styles.formError} role="alert">
          <TriangleAlert aria-hidden="true" />
          {error}
        </p>
      ) : null}

      {result ? (
        <>
          <KitDivider />
          <KitSection>Result</KitSection>
          <KitStaticRow
            label={responseSummary(result)}
            badges={
              <>
                <KitBadge
                  tone={resultTone(merged, conflicted, warnings.length > 0)}
                >
                  {result.status ?? (merged ? "merged" : "finished")}
                </KitBadge>
                {result.strategy ? (
                  <KitBadge>{result.strategy}</KitBadge>
                ) : null}
              </>
            }
          />
          {result.source_branch && result.target_branch ? (
            <p className={`${styles.metaText} ${styles.resultLine}`}>
              {result.source_branch} → {result.target_branch}
            </p>
          ) : null}
          {result.merge_commit ? (
            <p className={`${styles.metaText} ${styles.resultLine}`}>
              Merge commit: {result.merge_commit}
            </p>
          ) : null}
          {result.cleanup ? (
            <p className={`${styles.metaText} ${styles.resultLine}`}>
              Cleanup: worktree{" "}
              {result.cleanup.worktree_deleted ? "deleted" : "kept"}, branch{" "}
              {result.cleanup.branch_deleted ? "deleted" : "kept"}, registry{" "}
              {result.cleanup.registry_deleted ? "deleted" : "kept"}
            </p>
          ) : null}
          {conflicted ? (
            <>
              <KitSection>Conflicted files</KitSection>
              {conflictFiles.length === 0 ? (
                <KitStaticRow
                  tone="muted"
                  label="No conflicted files were reported."
                />
              ) : (
                conflictFiles.map((file) => (
                  <KitStaticRow key={file} label={file} />
                ))
              )}
              <div className={styles.inlineActions}>
                <KitButton
                  onClick={() => void handleAskRefact()}
                  disabled={!onAskRefact || !canActOnWorktree}
                >
                  Ask Refact to resolve conflicts
                </KitButton>
                <KitButton
                  onClick={() => void handleOpenWorktree()}
                  disabled={!onOpenWorktree || !canActOnWorktree}
                >
                  Open worktree
                </KitButton>
              </div>
            </>
          ) : null}
          {actionFeedback ? (
            <p className={`${styles.metaText} ${styles.resultLine}`}>
              {actionFeedback}
            </p>
          ) : null}
          {warnings.map((warning, index) => (
            <p
              key={`${index}-${warning}`}
              className={`${styles.warningText} ${styles.resultLine}`}
            >
              {warning}
            </p>
          ))}
        </>
      ) : null}
    </KitDialog>
  );
};

MergeWorktreeModal.displayName = "MergeWorktreeModal";
