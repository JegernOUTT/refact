import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Checkbox,
  Dialog,
  Flex,
  Select,
  Spinner,
  Text,
  TextField,
} from "@radix-ui/themes";
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
    (cleanup.worktree_deleted || cleanup.branch_deleted || cleanup.registry_deleted)
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
  const warnings = useMemo(() => (result ? responseWarnings(result) : []), [result]);
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
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content
        className={styles.mergeDialog}
        {...(closeOnNonInteractiveContentClick
          ? dialogNonInteractiveCloseHandlers(() => onOpenChange(false))
          : {})}
      >
        <Dialog.Title>Merge worktree</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Merge {label} into a target branch.
        </Dialog.Description>

        <Flex direction="column" gap="3" mt="3">
          <div className={styles.field}>
            <Text size="2" weight="medium">
              Strategy
            </Text>
            <Select.Root
              value={strategy}
              onValueChange={(value) =>
                setStrategy(value as WorktreeMergeStrategy)
              }
              disabled={mergeState.isLoading}
            >
              <Select.Trigger aria-label="Merge strategy" />
              <Select.Content>
                <Select.Item value="squash">Squash merge</Select.Item>
                <Select.Item value="merge">Regular merge</Select.Item>
              </Select.Content>
            </Select.Root>
          </div>

          <label className={styles.field} htmlFor="worktree-target-branch">
            <Text size="2" weight="medium">
              Target branch
            </Text>
            <TextField.Root
              id="worktree-target-branch"
              value={targetBranch}
              onChange={(event) => setTargetBranch(event.target.value)}
              disabled={mergeState.isLoading}
            />
          </label>

          <Flex direction="column" gap="2">
            <Text as="label" size="2">
              <Flex align="center" gap="2">
                <Checkbox
                  checked={deleteAfterMerge}
                  onCheckedChange={(checked) =>
                    setDeleteAfterMerge(checked === true)
                  }
                  disabled={mergeState.isLoading}
                />
                Delete worktree after merge or if there is nothing to merge
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex align="center" gap="2">
                <Checkbox
                  checked={includeUncommitted}
                  onCheckedChange={(checked) =>
                    setIncludeUncommitted(checked === true)
                  }
                  disabled={mergeState.isLoading}
                />
                Include uncommitted changes by auto-committing first
              </Flex>
            </Text>
          </Flex>

          {error && (
            <Text size="2" color="red" className={styles.warningBox}>
              {error}
            </Text>
          )}

          {result && (
            <Flex direction="column" gap="2" className={styles.resultBox}>
              <Flex gap="2" align="center" wrap="wrap">
                <Badge color={merged ? "green" : conflicted ? "amber" : "gray"}>
                  {result.status ?? (merged ? "merged" : "finished")}
                </Badge>
                {result.strategy && (
                  <Badge color="gray" variant="soft">
                    {result.strategy}
                  </Badge>
                )}
              </Flex>
              <Text
                size="2"
                color={conflicted ? "amber" : merged ? "green" : "gray"}
              >
                {responseSummary(result)}
              </Text>
              {result.source_branch && result.target_branch && (
                <Text size="1" color="gray">
                  {result.source_branch} → {result.target_branch}
                </Text>
              )}
              {result.merge_commit && (
                <Text size="1" color="gray">
                  Merge commit: {result.merge_commit}
                </Text>
              )}
              {result.cleanup && (
                <Text size="1" color="gray">
                  Cleanup: worktree{" "}
                  {result.cleanup.worktree_deleted ? "deleted" : "kept"}, branch{" "}
                  {result.cleanup.branch_deleted ? "deleted" : "kept"}
                </Text>
              )}
              {conflicted && (
                <Flex direction="column" gap="2">
                  <Text size="2" weight="medium">
                    Conflicted files
                  </Text>
                  <ul className={styles.conflictList}>
                    {conflictFiles.length === 0 ? (
                      <li>No conflicted files were reported.</li>
                    ) : (
                      conflictFiles.map((file) => <li key={file}>{file}</li>)
                    )}
                  </ul>
                  <Flex gap="2" wrap="wrap">
                    <Button
                      type="button"
                      size="2"
                      variant="soft"
                      onClick={() => void handleAskRefact()}
                      disabled={!onAskRefact}
                    >
                      Ask Refact to resolve conflicts
                    </Button>
                    <Button
                      type="button"
                      size="2"
                      variant="soft"
                      color="gray"
                      onClick={() => void handleOpenWorktree()}
                      disabled={!onOpenWorktree}
                    >
                      Open worktree
                    </Button>
                  </Flex>
                </Flex>
              )}
              {actionFeedback && (
                <Text size="1" color="gray">
                  {actionFeedback}
                </Text>
              )}
              {warnings.length > 0 && (
                <Flex direction="column" gap="1">
                  {warnings.map((warning, index) => (
                    <Text key={`${index}-${warning}`} size="1" color="amber">
                      {warning}
                    </Text>
                  ))}
                </Flex>
              )}
            </Flex>
          )}
        </Flex>

        <Flex className={styles.modalActions}>
          <Dialog.Close>
            <Button
              type="button"
              variant="soft"
              color="gray"
              disabled={mergeState.isLoading}
            >
              Close
            </Button>
          </Dialog.Close>
          <Button
            type="button"
            onClick={() => void handleMerge()}
            disabled={mergeState.isLoading || queryId.length === 0}
          >
            {mergeState.isLoading ? <Spinner size="1" /> : "Merge"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

MergeWorktreeModal.displayName = "MergeWorktreeModal";
