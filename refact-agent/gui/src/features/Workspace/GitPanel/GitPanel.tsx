import { useMemo } from "react";
import { GitBranch } from "lucide-react";

import { Icon } from "../../../components/ui";
import { useAppSelector } from "../../../hooks";
import { useGetGitStatusQuery } from "../../../services/refact/gitRead";
import { BranchesLog } from "./BranchesLog";
import { DiffView } from "./DiffView";
import { selectActiveGitRoot, selectSelectedGitFile } from "./gitPanelSlice";
import { workspaceRootForGitRoot } from "./gitRoots";
import { WorktreesSection } from "./WorktreesSection";
import {
  selectFocusedChatWorkspaceRoots,
  selectFocusedChatWorktreeSourceRoot,
  selectFocusedWorkspaceChatId,
} from "../workspaceSlice";
import styles from "./GitPanel.module.css";

const EMPTY_ROOTS: string[] = [];

function first<T>(values: T[]): T | undefined {
  return values.length > 0 ? values[0] : undefined;
}

export function GitPanel() {
  const chatId = useAppSelector(selectFocusedWorkspaceChatId);
  const configuredRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const contextRoots = useAppSelector(selectFocusedChatWorkspaceRoots);
  const worktreeSourceRoot = useAppSelector(
    selectFocusedChatWorktreeSourceRoot,
  );
  const activeRoot = useAppSelector((state) =>
    selectActiveGitRoot(state, chatId),
  );
  const selected = useAppSelector((state) =>
    selectSelectedGitFile(state, chatId),
  );
  const statusQuery = useGetGitStatusQuery(contextRoots);
  const roots = useMemo(
    () => statusQuery.data?.roots ?? [],
    [statusQuery.data?.roots],
  );
  const activeStatus =
    roots.find((root) => root.root === activeRoot) ?? first(roots);
  const resolvedRoot = activeStatus
    ? activeStatus.root
    : selected
      ? selected.root
      : "";
  const selectedForRoot = selected?.root === resolvedRoot ? selected : null;
  const worktreesRoot = workspaceRootForGitRoot(
    configuredRoots,
    resolvedRoot,
    worktreeSourceRoot,
  );

  return (
    <div className={styles.panel} data-testid="git-main-panel">
      <header className={styles.panelHeader}>
        <div className={styles.panelTitle}>
          <Icon icon={GitBranch} size="md" />
          <div>
            <h1>Git</h1>
            <p>Review changes, repository history, and worktrees.</p>
          </div>
        </div>
      </header>

      <main className={styles.mainColumn}>
        {resolvedRoot ? (
          <>
            <DiffView root={resolvedRoot} selected={selectedForRoot} />
            <BranchesLog key={`history-${resolvedRoot}`} root={resolvedRoot} />
            <WorktreesSection
              key={`worktrees-${worktreesRoot}`}
              workspaceRoot={worktreesRoot}
            />
          </>
        ) : (
          <section className={styles.section}>
            <p className={styles.emptyText} role="status">
              Select a changed file from the Git dock to inspect its diff.
            </p>
          </section>
        )}
      </main>
    </div>
  );
}
