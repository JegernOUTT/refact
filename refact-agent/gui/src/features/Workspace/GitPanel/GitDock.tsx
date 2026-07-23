import { useEffect, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";

import { Badge, FieldSelect, IconButton } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  useGetGitStatusQuery,
  useStageGitPathsMutation,
  useUnstageGitPathsMutation,
  type GitFileChange,
  type GitStatusRoot,
} from "../../../services/refact/gitRead";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import { CommitBox } from "./CommitBox";
import {
  openGitFile,
  selectActiveGitRoot,
  selectGitFile,
  selectSelectedGitFile,
  setActiveGitRoot,
} from "./gitPanelSlice";
import { StatusList } from "./StatusList";
import styles from "./GitPanel.module.css";

const EMPTY_ROOTS: string[] = [];

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

export function GitDock() {
  const dispatch = useAppDispatch();
  const configuredRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const activeRoot = useAppSelector(selectActiveGitRoot);
  const selected = useAppSelector(selectSelectedGitFile);
  const statusQuery = useGetGitStatusQuery(configuredRoots);
  const [stage] = useStageGitPathsMutation();
  const [unstage] = useUnstageGitPathsMutation();
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const roots = useMemo(
    () => statusQuery.data?.roots ?? [],
    [statusQuery.data?.roots],
  );
  const rootPaths = useMemo(() => roots.map((root) => root.root), [roots]);

  useEffect(() => {
    if (rootPaths.length === 0 || rootPaths.includes(activeRoot)) return;
    dispatch(setActiveGitRoot(rootPaths[0] ?? ""));
  }, [activeRoot, dispatch, rootPaths]);

  const activeStatus =
    roots.find((root) => root.root === activeRoot) ?? first(roots);
  const resolvedRoot = activeStatus ? activeStatus.root : "";
  const selectedForRoot = selected?.root === resolvedRoot ? selected : null;
  const noRepositories = statusQuery.data !== undefined && roots.length === 0;
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
      dispatch(
        selectGitFile({
          root: resolvedRoot,
          path: change.relative_path,
          staged: !staged,
        }),
      );
    } catch (error) {
      setMutationError(worktreeErrorText(error));
    } finally {
      setPendingPath(null);
    }
  };

  return (
    <div className={styles.dockPanel} data-testid="git-dock-panel">
      <header className={styles.dockHeader}>
        <div className={styles.dockHeading}>
          <h2>Changes</h2>
          <p>{branchLabel(activeStatus)}</p>
        </div>
        <div className={styles.dockHeaderActions}>
          <Badge tone="muted">
            {activeStatus
              ? activeStatus.staged.length + activeStatus.unstaged.length
              : 0}
          </Badge>
          <IconButton
            aria-label="Refresh Git status"
            icon={RefreshCw}
            loading={statusQuery.isFetching}
            disabled={statusQuery.isFetching}
            onClick={() => void statusQuery.refetch()}
            size="sm"
            variant="plain"
          />
        </div>
      </header>

      {roots.length > 1 ? (
        <div className={styles.rootPicker}>
          <FieldSelect
            aria-label="Git root"
            onChange={(root) => {
              dispatch(setActiveGitRoot(root));
              setMutationError(null);
              setFeedback(null);
            }}
            options={roots.map((root) => ({
              value: root.root,
              label: rootLabel(root.root),
            }))}
            value={resolvedRoot}
          />
        </div>
      ) : null}

      {feedback ? (
        <p className={styles.successText} role="status">
          {feedback}
        </p>
      ) : null}

      {noRepositories ? (
        <p className={styles.dockEmptyText} role="status">
          No git repository found in this workspace.
        </p>
      ) : (
        <div className={styles.dockContent}>
          <section
            className={styles.dockSection}
            aria-labelledby="git-changes-heading"
          >
            <h3 id="git-changes-heading" className={styles.srOnly}>
              Changed files
            </h3>
            <StatusList
              status={activeStatus}
              selected={selectedForRoot}
              pendingPath={pendingPath}
              isLoading={statusQuery.isLoading}
              error={statusError ?? null}
              onSelect={(file) =>
                dispatch(openGitFile({ ...file, root: resolvedRoot }))
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
              stagedChanges={activeStatus ? activeStatus.staged : []}
              onCommitted={(shortOid) => {
                setFeedback(`Committed ${shortOid}`);
                dispatch(selectGitFile(null));
                void statusQuery.refetch();
              }}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}
