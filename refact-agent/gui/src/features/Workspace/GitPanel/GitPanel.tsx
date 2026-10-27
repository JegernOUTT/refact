import { useMemo } from "react";
import { GitBranch } from "lucide-react";

import { FieldSelect, Icon, Tabs } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useGetGitStatusQuery } from "../../../services/refact/gitRead";
import {
  selectActiveGitRoot,
  selectGitDiffTargetForChat,
  selectGitLogLayout,
  setActiveGitRoot,
  setGitLogLayout,
  type GitWindowTab,
} from "./gitPanelSlice";
import { workspaceRootForGitRoot } from "./gitRoots";
import { GitLog } from "./log/GitLog";
import { WorktreesSection } from "./WorktreesSection";
import {
  selectFocusedChatWorkspaceRoots,
  selectFocusedChatWorktreeSourceRoot,
  selectFocusedWorkspaceChatId,
} from "../workspaceSlice";
import styles from "./GitPanel.module.css";

const EMPTY_ROOTS: string[] = [];
const TABS: { value: GitWindowTab; label: string }[] = [
  { value: "log", label: "Log" },
  { value: "worktrees", label: "Worktrees" },
];

function first<T>(values: T[]): T | undefined {
  return values.length > 0 ? values[0] : undefined;
}

function rootLabel(root: string): string {
  const normalized = root.replace(/[/\\]+$/, "");
  return normalized.split(/[/\\]/).pop() ?? root;
}

function isGitWindowTab(value: string): value is GitWindowTab {
  return TABS.some((tab) => tab.value === value);
}

export function GitPanel() {
  const dispatch = useAppDispatch();
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
    selectGitDiffTargetForChat(state, chatId),
  );
  const { tab } = useAppSelector(selectGitLogLayout);
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
  const worktreesRoot = workspaceRootForGitRoot(
    configuredRoots,
    resolvedRoot,
    worktreeSourceRoot,
  );
  const activeIndex = TABS.findIndex((item) => item.value === tab);

  return (
    <div className={styles.panel} data-testid="git-main-panel">
      <Tabs
        className={styles.tabs}
        onValueChange={(value) => {
          if (isGitWindowTab(value)) dispatch(setGitLogLayout({ tab: value }));
        }}
        value={tab}
      >
        <header className={styles.panelHeader}>
          <div className={styles.panelTitle}>
            <Icon icon={GitBranch} size="sm" tone="muted" />
            <h1>Git</h1>
          </div>
          <Tabs.List
            activeIndex={activeIndex}
            aria-label="Git views"
            className={styles.tabList}
            itemCount={TABS.length}
          >
            {TABS.map((item) => (
              <Tabs.Trigger key={item.value} value={item.value}>
                {item.label}
              </Tabs.Trigger>
            ))}
          </Tabs.List>
          {roots.length > 1 && activeStatus ? (
            <div className={styles.rootPicker}>
              <FieldSelect
                aria-label="Repository"
                onChange={(root) =>
                  dispatch(setActiveGitRoot({ chatId, root }))
                }
                options={roots.map((root) => ({
                  value: root.root,
                  label: rootLabel(root.root),
                }))}
                value={activeStatus.root}
              />
            </div>
          ) : null}
        </header>

        {resolvedRoot ? (
          <>
            <Tabs.Content className={styles.tabContent} value="log">
              <GitLog key={`log-${resolvedRoot}`} root={resolvedRoot} />
            </Tabs.Content>
            <Tabs.Content className={styles.tabScroll} value="worktrees">
              <WorktreesSection
                key={`worktrees-${worktreesRoot}`}
                workspaceRoot={worktreesRoot}
              />
            </Tabs.Content>
          </>
        ) : (
          <section className={styles.section}>
            <p className={styles.emptyText} role="status">
              {statusQuery.isLoading
                ? "Loading repository…"
                : "Open a folder inside a Git repository to see its log."}
            </p>
          </section>
        )}
      </Tabs>
    </div>
  );
}
