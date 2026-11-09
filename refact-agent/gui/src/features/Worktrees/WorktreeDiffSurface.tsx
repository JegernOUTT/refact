import { useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { useAppDispatch } from "../../hooks";
import { gitReadApi } from "../../services/refact/gitRead";
import { Button, Spinner } from "../../components/ui";
import {
  useGetWorktreeQuery,
  useGetWorktreeDiffQuery,
} from "../../services/refact/worktrees";
import GitDiffViewer from "../Workspace/GitPanel/diff/GitDiffViewer";
import type { WorktreeDiffDescriptor } from "../Workspace/worktreeDiffDescriptor";
import {
  worktreeDiffEntryKey,
  worktreeFileDescriptor,
} from "../Workspace/worktreeDiffModel";
import { worktreeErrorText } from "./worktreeError";
import styles from "../Workspace/WorktreeDiffSurface.module.css";

export function WorktreeDiffSurface({
  descriptor,
  surfaceKey,
}: {
  descriptor: WorktreeDiffDescriptor;
  surfaceKey?: string;
}) {
  const dispatch = useAppDispatch();
  const args = {
    id: descriptor.worktreeId,
    source_workspace_root: descriptor.sourceWorkspaceRoot,
  };
  const record = useGetWorktreeQuery(descriptor.root ? skipToken : args);
  const query = useGetWorktreeDiffQuery(
    { ...args, max_patch_bytes: 120000 },
    { refetchOnMountOrArgChange: true },
  );
  const [selected, setSelected] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);
  const data = query.currentData;
  const root = descriptor.root ?? record.currentData?.meta.root;
  const file =
    data?.files.find((entry) => worktreeDiffEntryKey(entry) === selected) ??
    data?.files[0];
  const diff =
    file && data && root ? worktreeFileDescriptor(root, file, data) : null;
  const error = query.error ?? record.error;
  const refresh = () => {
    if (root)
      dispatch(gitReadApi.util.invalidateTags([{ type: "GitFile", id: root }]));
    void query.refetch();
    if (!descriptor.root) void record.refetch();
    setGeneration((value) => value + 1);
  };
  return (
    <section className={styles.surface} aria-label="Worktree diff">
      <header className={styles.header}>
        <div className={styles.heading}>
          <strong>
            Worktree diff ·{" "}
            {descriptor.label ?? data?.branch ?? descriptor.worktreeId}
          </strong>
          <span className={styles.muted}>
            {root ?? descriptor.sourceWorkspaceRoot} · Session-only review
          </span>
        </div>
        <Button
          size="sm"
          variant="soft"
          onClick={refresh}
          disabled={query.isFetching}
        >
          Refresh
        </Button>
      </header>
      {query.isFetching && (
        <div className={styles.notice} role="status">
          <Spinner label="Loading worktree diff" />
        </div>
      )}
      {error && (
        <div className={styles.notice} role="alert">
          Could not load worktree diff. {worktreeErrorText(error)}{" "}
          <Button size="sm" onClick={refresh}>
            Retry
          </Button>
        </div>
      )}
      {data && (
        <>
          <div className={styles.notice}>
            {data.stats.files_changed} file layers ·{" "}
            {data.stats.committed_files} committed · {data.stats.staged_files}{" "}
            staged · {data.stats.unstaged_files} unstaged ·{" "}
            {data.stats.untracked_files} untracked
          </div>
          {data.patch_truncated && (
            <div className={styles.mutedNotice}>
              Summary patch truncated; detailed file contents load separately
              (viewer limits are shown per file).
            </div>
          )}
          {data.files.length === 0 ? (
            <div className={styles.notice}>No changed files reported.</div>
          ) : (
            <div className={styles.body}>
              <nav
                className={styles.files}
                aria-label="All changed files and layers"
              >
                {data.files.map((entry) => {
                  const key = worktreeDiffEntryKey(entry);
                  return (
                    <button
                      type="button"
                      key={key}
                      className={styles.file}
                      aria-current={
                        file && key === worktreeDiffEntryKey(file)
                          ? "true"
                          : undefined
                      }
                      onClick={() => setSelected(key)}
                      title={entry.path}
                    >
                      <span className={styles.path}>
                        {entry.old_path
                          ? `${entry.old_path} → ${entry.path}`
                          : entry.path}
                      </span>
                      <span className={styles.muted}>
                        {entry.source} · {entry.status}{" "}
                        {entry.additions != null ? `+${entry.additions}` : ""}{" "}
                        {entry.deletions != null ? `−${entry.deletions}` : ""}
                      </span>
                    </button>
                  );
                })}
              </nav>
              <div className={styles.viewer}>
                {diff ? (
                  <GitDiffViewer
                    key={`${
                      file ? worktreeDiffEntryKey(file) : ""
                    }:${generation}:${JSON.stringify(diff)}`}
                    descriptor={diff}
                    surfaceKey={surfaceKey}
                  />
                ) : (
                  <div className={styles.notice}>
                    {root
                      ? "Exact comparison unavailable: the server did not provide a resolved base, rename path, or supported layer. Refresh or update the engine."
                      : "Resolving worktree root…"}
                  </div>
                )}
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}
