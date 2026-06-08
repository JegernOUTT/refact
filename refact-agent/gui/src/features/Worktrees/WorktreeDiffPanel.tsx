import React from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { Button, Dialog, Badge } from "../../components/ui";
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

function statusTone(
  status?: WorktreeStatus | null,
): React.ComponentProps<typeof Badge>["tone"] {
  if (!status) return "muted";
  if ((status.deleted ?? false) || !status.path_exists) return "danger";
  if ((status.conflicted ?? false) || status.dirty) return "warning";
  return "success";
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.diffDialog} maxWidth="900px">
        <div
          {...(closeOnNonInteractiveContentClick
            ? dialogNonInteractiveCloseHandlers(() => onOpenChange(false))
            : {})}
        >
          <Dialog.Title>Worktree diff</Dialog.Title>
          <Dialog.Description>Review changes for {label}</Dialog.Description>

          <div className={styles.modalFields}>
            <div className={styles.badgeRow}>
              <Badge tone={statusTone(status)}>{statusLabel(status)}</Badge>
              {data?.branch && <Badge tone="muted">{data.branch}</Badge>}
              {data?.base_branch && (
                <Badge tone="muted">target {data.base_branch}</Badge>
              )}
            </div>

            {isFetching && (
              <div className={styles.loadingRow}>
                <span className={styles.spinner} aria-hidden="true" />
                <span className={styles.helpText}>
                  Loading worktree diff...
                </span>
              </div>
            )}

            {error && (
              <div className={styles.errorBox}>
                <span className={styles.errorTitle}>
                  Could not load worktree diff.
                </span>
                <span className={styles.metaText}>
                  {worktreeErrorText(error)}
                </span>
                <Button size="sm" variant="soft" onClick={() => void refetch()}>
                  Retry
                </Button>
              </div>
            )}

            {data && (
              <>
                <p className={styles.metaText}>{statsText(data.stats)}</p>

                {data.patch_truncated && (
                  <p className={styles.warningBox}>
                    Patch preview was truncated by the backend.
                  </p>
                )}

                <div className={styles.diffFileList}>
                  {data.files.length === 0 ? (
                    <span className={styles.emptyText}>
                      No changed files reported.
                    </span>
                  ) : (
                    data.files.map((file) => (
                      <div
                        key={`${file.source}-${file.path}`}
                        className={styles.diffFileItem}
                      >
                        <span className={styles.itemTitle}>
                          <span className={styles.itemName}>{file.path}</span>
                          <span className={styles.metaText}>
                            {file.source} · {file.status}
                          </span>
                        </span>
                        {fileDelta(file.additions, file.deletions) && (
                          <span className={styles.metaText}>
                            {fileDelta(file.additions, file.deletions)}
                          </span>
                        )}
                      </div>
                    ))
                  )}
                </div>

                <div className={`${styles.patchScroller} scrollX`}>
                  <RichPatchPreview patch={data.patch} />
                </div>
              </>
            )}
          </div>

          <div className={styles.modalActions}>
            <Dialog.Close asChild>
              <Button variant="soft">Close</Button>
            </Dialog.Close>
          </div>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

WorktreeDiffPanel.displayName = "WorktreeDiffPanel";
