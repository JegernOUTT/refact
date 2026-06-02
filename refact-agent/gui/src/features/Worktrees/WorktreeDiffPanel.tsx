import React from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { Badge, Button, Dialog, Flex, Spinner, Text } from "@radix-ui/themes";
import {
  useGetWorktreeDiffQuery,
  type WorktreeMeta,
  type WorktreeRecordView,
  type WorktreeStatus,
} from "../../services/refact";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type WorktreeDiffPanelProps = {
  open: boolean;
  worktreeId?: string | null;
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  sourceWorkspaceRoot?: string;
  onOpenChange: (open: boolean) => void;
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

function statusLabel(status?: WorktreeStatus | null): string {
  if (!status) return "unknown";
  if (status.deleted) return "deleted";
  if (!status.path_exists) return "missing";
  if (status.conflicted) return "conflicted";
  if (status.dirty) return "dirty";
  return "clean";
}

function statusColor(
  status?: WorktreeStatus | null,
): "green" | "amber" | "red" | "gray" {
  if (!status) return "gray";
  if ((status.deleted ?? false) || !status.path_exists) return "red";
  if ((status.conflicted ?? false) || status.dirty) return "amber";
  return "green";
}

function statsText(stats: {
  committed_files: number;
  staged_files: number;
  unstaged_files: number;
  untracked_files: number;
  files_changed: number;
}): string {
  return `${stats.files_changed} changed · ${stats.committed_files} committed · ${stats.staged_files} staged · ${stats.unstaged_files} unstaged · ${stats.untracked_files} untracked`;
}

function fileDelta(
  additions?: number | null,
  deletions?: number | null,
): string {
  const parts: string[] = [];
  if (typeof additions === "number") parts.push(`+${additions}`);
  if (typeof deletions === "number") parts.push(`-${deletions}`);
  return parts.join(" ");
}

function patchLineClass(line: string): string {
  if (line.startsWith("+++ ") || line.startsWith("--- ")) {
    return styles.patchLineFile;
  }
  if (line.startsWith("@@")) return styles.patchLineHunk;
  if (line.startsWith("+")) return styles.patchLineAdd;
  if (line.startsWith("-")) return styles.patchLineRemove;
  if (line.startsWith("diff --git") || line.startsWith("index ")) {
    return styles.patchLineMeta;
  }
  return styles.patchLineContext;
}

function RichPatchPreview({ patch }: { patch: string }) {
  const text = patch.length > 0 ? patch : "No patch preview available.";
  return (
    <pre className={styles.patchPreview}>
      {text.split("\n").map((line, index) => (
        <span
          key={`${index}-${line.slice(0, 12)}`}
          className={`${styles.patchLine} ${patchLineClass(line)}`}
        >
          {line || " "}
        </span>
      ))}
    </pre>
  );
}

export const WorktreeDiffPanel: React.FC<WorktreeDiffPanelProps> = ({
  open,
  worktreeId,
  worktree,
  record,
  sourceWorkspaceRoot,
  onOpenChange,
  closeOnNonInteractiveContentClick = false,
}) => {
  const queryId = worktreeId ?? record?.meta.id ?? worktree?.id ?? "";
  const resolvedSourceRoot =
    sourceWorkspaceRoot ??
    record?.meta.source_workspace_root ??
    worktree?.source_workspace_root;
  const canQueryDiff =
    open &&
    queryId.length > 0 &&
    (record !== undefined || worktreeId === undefined || worktreeId === null);
  const diffQuery = canQueryDiff
    ? {
        id: record?.meta.id ?? queryId,
        source_workspace_root: resolvedSourceRoot,
        max_patch_bytes: 120000,
      }
    : skipToken;
  const { data, isFetching, error, refetch } =
    useGetWorktreeDiffQuery(diffQuery);
  const label = displayWorktreeLabel(worktree, record);
  const status = data?.status ?? record?.status ?? worktree?.status;

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content
        className={styles.diffDialog}
        {...(closeOnNonInteractiveContentClick
          ? dialogNonInteractiveCloseHandlers(() => onOpenChange(false))
          : {})}
      >
        <Dialog.Title>Worktree diff</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Review changes for {label}
        </Dialog.Description>

        <Flex direction="column" gap="3" mt="3">
          <Flex gap="2" wrap="wrap" align="center">
            <Badge color={statusColor(status)} variant="soft">
              {statusLabel(status)}
            </Badge>
            {data?.branch && (
              <Badge color="gray" variant="soft">
                {data.branch}
              </Badge>
            )}
            {data?.base_branch && (
              <Badge color="gray" variant="soft">
                target {data.base_branch}
              </Badge>
            )}
          </Flex>

          {isFetching && (
            <Flex align="center" gap="2">
              <Spinner size="1" />
              <Text size="2" color="gray">
                Loading worktree diff...
              </Text>
            </Flex>
          )}

          {error && (
            <Flex direction="column" gap="2" className={styles.warningBox}>
              <Text size="2" color="red">
                Could not load worktree diff.
              </Text>
              <Text size="1" color="gray">
                {worktreeErrorText(error)}
              </Text>
              <Button
                type="button"
                size="1"
                variant="soft"
                onClick={() => void refetch()}
              >
                Retry
              </Button>
            </Flex>
          )}

          {data && (
            <>
              <Text size="2" color="gray">
                {statsText(data.stats)}
              </Text>

              {data.patch_truncated && (
                <Text size="2" color="amber" className={styles.warningBox}>
                  Patch preview was truncated by the backend.
                </Text>
              )}

              <div className={styles.diffFileList}>
                {data.files.length === 0 ? (
                  <Text size="2" color="gray">
                    No changed files reported.
                  </Text>
                ) : (
                  data.files.map((file) => (
                    <Flex
                      key={`${file.source}-${file.path}`}
                      justify="between"
                      align="center"
                      gap="2"
                      className={styles.diffFileItem}
                    >
                      <Flex
                        direction="column"
                        gap="1"
                        className={styles.itemTitle}
                      >
                        <Text size="2" weight="medium">
                          {file.path}
                        </Text>
                        <Text size="1" color="gray">
                          {file.source} · {file.status}
                        </Text>
                      </Flex>
                      {fileDelta(file.additions, file.deletions) && (
                        <Text size="1" color="gray">
                          {fileDelta(file.additions, file.deletions)}
                        </Text>
                      )}
                    </Flex>
                  ))
                )}
              </div>

              <div className={styles.patchScroller}>
                <RichPatchPreview patch={data.patch} />
              </div>
            </>
          )}
        </Flex>

        <Flex className={styles.modalActions}>
          <Dialog.Close>
            <Button type="button" variant="soft" color="gray">
              Close
            </Button>
          </Dialog.Close>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

WorktreeDiffPanel.displayName = "WorktreeDiffPanel";
