import { useEffect, useMemo, useState } from "react";
import { GitBranch, RefreshCw } from "lucide-react";

import { Badge, Button, Icon } from "../../../components/ui";
import { useAppSelector } from "../../../hooks";
import {
  useGetGitStatusQuery,
  useStageGitPathsMutation,
  useUnstageGitPathsMutation,
  type GitFileChange,
  type GitStatusRoot,
} from "../../../services/refact/gitRead";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import { BranchesLog } from "./BranchesLog";
import { CommitBox } from "./CommitBox";
import { DiffView } from "./DiffView";
import { StatusList, type SelectedGitFile } from "./StatusList";
import { WorktreesSection } from "./WorktreesSection";
import styles from "./GitPanel.module.css";

function rootLabel(root: string): string {
  const normalized = root.replace(/[/\\]+$/, "");
  return normalized.split(/[/\\]/).pop() ?? root;
}

function branchLabel(status?: GitStatusRoot): string {
  if (!status) return "";
  if (status.head_detached) return "Detached HEAD";
  return status.branch ?? "No branch";
}

function first<T>(values: T[]): T | undefined {
  return values.length > 0 ? values[0] : undefined;
}

function firstRoot(values: string[]): string | undefined {
  return values.length > 0 ? values[0] : undefined;
}

export function GitPanel() {
  const configuredRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? [],
  );
  const statusQuery = useGetGitStatusQuery(configuredRoots);
  const [stage] = useStageGitPathsMutation();
  const [unstage] = useUnstageGitPathsMutation();
  const roots = useMemo(
    () => statusQuery.data?.roots ?? [],
    [statusQuery.data?.roots],
  );
  const rootPaths = useMemo(() => roots.map((root) => root.root), [roots]);
  const [activeRoot, setActiveRoot] = useState("");
  const [selected, setSelected] = useState<
    (SelectedGitFile & { root: string }) | null
  >(null);
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);

  useEffect(() => {
    if (rootPaths.length === 0) return;
    if (!rootPaths.includes(activeRoot)) {
      setActiveRoot(firstRoot(rootPaths) ?? "");
      setSelected(null);
    }
  }, [activeRoot, rootPaths]);

  const activeStatus =
    roots.find((root) => root.root === activeRoot) ?? first(roots);
  const resolvedRoot = activeStatus?.root ?? "";
  const noRepositories = statusQuery.data !== undefined && roots.length === 0;
  const selectedForRoot = selected?.root === resolvedRoot ? selected : null;
  const statusError = statusQuery.error
    ? worktreeErrorText(statusQuery.error)
    : mutationError;

  const mutatePath = async (change: GitFileChange, staged: boolean) => {
    if (!resolvedRoot) return;
    setPendingPath(change.relative_path);
    setMutationError(null);
    setFeedback(null);
    try {
      const mutate = staged ? unstage : stage;
      await mutate({
        root: resolvedRoot,
        paths: [change.relative_path],
      }).unwrap();
      setSelected({
        root: resolvedRoot,
        path: change.relative_path,
        staged: !staged,
      });
    } catch (error) {
      setMutationError(worktreeErrorText(error));
    } finally {
      setPendingPath(null);
    }
  };

  return (
    <div className={styles.panel}>
      <header className={styles.panelHeader}>
        <div className={styles.panelTitle}>
          <Icon icon={GitBranch} size="md" />
          <div>
            <h1>Git</h1>
            <p>Stage, review, commit, and inspect repository history.</p>
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          leftIcon={RefreshCw}
          loading={statusQuery.isFetching}
          disabled={statusQuery.isFetching}
          onClick={() => void statusQuery.refetch()}
        >
          Refresh
        </Button>
      </header>

      {roots.length > 1 ? (
        <div className={styles.rootTabs} role="tablist" aria-label="Git roots">
          {roots.map((root) => (
            <button
              type="button"
              role="tab"
              key={root.root}
              aria-selected={resolvedRoot === root.root}
              className={styles.rootTab}
              onClick={() => {
                setActiveRoot(root.root);
                setSelected(null);
                setMutationError(null);
                setFeedback(null);
              }}
            >
              {rootLabel(root.root)}
            </button>
          ))}
        </div>
      ) : null}

      {feedback ? (
        <p className={styles.successBanner} role="status">
          {feedback}
        </p>
      ) : null}

      {noRepositories ? (
        <section className={styles.section}>
          <p className={styles.emptyText} role="status">
            No git repository found in this workspace.
          </p>
        </section>
      ) : (
        <div className={styles.contentGrid}>
          <aside className={styles.sidebar}>
            <section
              className={styles.section}
              aria-labelledby="git-changes-heading"
            >
              <header className={styles.sectionHeader}>
                <div>
                  <h2 id="git-changes-heading">Changes</h2>
                  <p>{branchLabel(activeStatus)}</p>
                </div>
                <Badge tone="muted">
                  {(activeStatus?.staged.length ?? 0) +
                    (activeStatus?.unstaged.length ?? 0)}
                </Badge>
              </header>
              <StatusList
                status={activeStatus}
                selected={selectedForRoot}
                pendingPath={pendingPath}
                isLoading={statusQuery.isLoading}
                error={statusError ?? null}
                onSelect={(file) =>
                  setSelected({ ...file, root: resolvedRoot })
                }
                onStage={(change) => void mutatePath(change, false)}
                onUnstage={(change) => void mutatePath(change, true)}
                onRefresh={() => void statusQuery.refetch()}
              />
            </section>
            {resolvedRoot ? (
              <CommitBox
                key={resolvedRoot}
                root={resolvedRoot}
                stagedChanges={activeStatus?.staged ?? []}
                onCommitted={(shortOid) => {
                  setFeedback(`Committed ${shortOid}`);
                  setSelected(null);
                  void statusQuery.refetch();
                }}
              />
            ) : null}
          </aside>
          <main className={styles.mainColumn}>
            {resolvedRoot ? (
              <>
                <DiffView root={resolvedRoot} selected={selectedForRoot} />
                <BranchesLog
                  key={`history-${resolvedRoot}`}
                  root={resolvedRoot}
                />
                <WorktreesSection
                  key={`worktrees-${resolvedRoot}`}
                  root={resolvedRoot}
                />
              </>
            ) : null}
          </main>
        </div>
      )}
    </div>
  );
}
