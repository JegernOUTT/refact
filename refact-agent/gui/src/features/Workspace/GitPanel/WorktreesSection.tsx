import { useState } from "react";

import { Badge, Button, Spinner } from "../../../components/ui";
import {
  useDeleteWorktreeMutation,
  useListWorktreesQuery,
  useOpenWorktreeMutation,
  type WorktreeRecordView,
} from "../../../services/refact/worktrees";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { WorktreeDiffPanel } from "../../Worktrees/WorktreeDiffPanel";
import { WorktreeStatusBadge } from "../../Worktrees/WorktreeStatusBadge";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import styles from "./GitPanel.module.css";

export function WorktreesSection({ workspaceRoot }: { workspaceRoot: string }) {
  const worktrees = useListWorktreesQuery({
    source_workspace_root: workspaceRoot,
  });
  const [openWorktree, openState] = useOpenWorktreeMutation();
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const copyToClipboard = useCopyToClipboard();
  const [selected, setSelected] = useState<WorktreeRecordView | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleOpen = async (record: WorktreeRecordView) => {
    setPendingId(record.meta.id);
    setFeedback(null);
    setError(null);
    try {
      const response = await openWorktree({
        id: record.meta.id,
        source_workspace_root: workspaceRoot,
      }).unwrap();
      copyToClipboard(response.path);
      setFeedback(`Copied ${response.path}`);
    } catch (openError) {
      setError(worktreeErrorText(openError));
    } finally {
      setPendingId(null);
    }
  };

  const handleCleanup = async (record: WorktreeRecordView) => {
    setPendingId(record.meta.id);
    setFeedback(null);
    setError(null);
    try {
      await deleteWorktree({
        id: record.meta.id,
        source_workspace_root: workspaceRoot,
        delete_branch: false,
      }).unwrap();
      setFeedback(`Cleaned up ${record.meta.branch ?? record.meta.id}`);
    } catch (deleteError) {
      setError(worktreeErrorText(deleteError));
    } finally {
      setPendingId(null);
    }
  };

  return (
    <section className={styles.section} aria-labelledby="git-worktrees-heading">
      <header className={styles.sectionHeader}>
        <div>
          <h2 id="git-worktrees-heading">Worktrees</h2>
          <p>Inspect, open, or clean up registered worktrees for this root.</p>
        </div>
        <Badge tone="muted">{worktrees.data?.worktrees.length ?? 0}</Badge>
      </header>
      {feedback ? (
        <p className={styles.successText} role="status">
          {feedback}
        </p>
      ) : null}
      {error ? (
        <p className={styles.errorText} role="alert">
          {error}
        </p>
      ) : null}
      {worktrees.isLoading ? (
        <Spinner label="Loading worktrees" size="sm" />
      ) : worktrees.error ? (
        <p className={styles.errorText} role="alert">
          {worktreeErrorText(worktrees.error)}
        </p>
      ) : (worktrees.data?.worktrees.length ?? 0) === 0 ? (
        <p className={styles.emptyText}>No registered worktrees.</p>
      ) : (
        <ul className={styles.worktreeList}>
          {worktrees.data?.worktrees.map((record) => {
            const pending = pendingId === record.meta.id;
            return (
              <li key={record.meta.id}>
                <div className={styles.worktreeIdentity}>
                  <span className={styles.branchName}>
                    {record.meta.branch ?? record.meta.id}
                  </span>
                  <span className={styles.mutedText}>{record.meta.root}</span>
                  <WorktreeStatusBadge record={record} />
                </div>
                <div className={styles.worktreeActions}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={pendingId !== null}
                    onClick={() => setSelected(record)}
                  >
                    Diff
                  </Button>
                  <Button
                    type="button"
                    variant="soft"
                    size="sm"
                    loading={pending && openState.isLoading}
                    disabled={pendingId !== null}
                    onClick={() => void handleOpen(record)}
                  >
                    Open
                  </Button>
                  <Button
                    type="button"
                    variant="danger"
                    size="sm"
                    loading={pending && deleteState.isLoading}
                    disabled={pendingId !== null || record.status.dirty}
                    title={
                      record.status.dirty
                        ? "Dirty worktrees cannot be cleaned up here."
                        : undefined
                    }
                    onClick={() => void handleCleanup(record)}
                  >
                    Cleanup
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
      <WorktreeDiffPanel
        open={selected !== null}
        record={selected}
        sourceWorkspaceRoot={workspaceRoot}
        onOpenChange={(open) => {
          if (!open) setSelected(null);
        }}
      />
    </section>
  );
}
