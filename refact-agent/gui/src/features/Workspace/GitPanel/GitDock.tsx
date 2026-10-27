import { ContextMenu } from "@radix-ui/themes";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";

import {
  Badge,
  FieldSelect,
  LoadingState,
  Tooltip,
  type BadgeTone,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import {
  normalizeGitStatus,
  useRollbackGitPathsMutation,
  useStageGitPathsMutation,
  type GitRepoState,
  type GitSkippedPath,
  type NormalizedGitStatus,
} from "../../../services/refact/gitRead";
import { selectFocusedWorkspaceChatId } from "../workspaceSlice";
import { BranchChip } from "./BranchChip";
import { BranchPopup } from "./branches/BranchPopup";
import { GitFeedback } from "./branches/GitFeedback";
import {
  useGitActions,
  type GitActionsController,
} from "./branches/useGitActions";
import { ChangesToolbar } from "./ChangesToolbar";
import { ChangesTree } from "./ChangesTree";
import {
  applyInclusion,
  buildChangesTree,
  checkableCount,
  collectEntries,
  commitPaths,
  containerIds,
  flattenChangesTree,
  includedEntries,
  isCheckable,
  nodeCheckState,
  pruneInclusion,
  type ChangesTreeEntry,
  type ChangesTreeNode,
  type InclusionState,
} from "./changesTreeModel";
import { CommitPanel } from "./CommitPanel";
import {
  DeleteUnversionedDialog,
  RollbackDialog,
  SkippedPaths,
} from "./GitActionDialogs";
import { gitErrorText } from "./gitErrors";
import {
  openGitDiff,
  selectActiveGitRoot,
  selectGitRootView,
  setActiveGitRoot,
  updateGitRootView,
  type GitRootViewState,
} from "./gitPanelSlice";
import { REPO_STATE_LABELS } from "./repoState";
import { useGitStatus } from "./useGitStatus";
import styles from "./GitDock.module.css";

type PendingDialog =
  | { kind: "rollback"; entries: ChangesTreeEntry[] }
  | { kind: "delete"; entries: ChangesTreeEntry[] };

type Feedback = {
  tone: "success" | "error";
  text: string;
  skipped: GitSkippedPath[];
};

function rootLabel(root: string): string {
  const normalized = root.replace(/[/\\]+$/, "");
  return normalized.split(/[/\\]/).pop() ?? root;
}

function rollbackPaths(entries: ChangesTreeEntry[]): string[] {
  return commitPaths(entries);
}

function AheadBehind({ status }: { status: NormalizedGitStatus }) {
  const ahead = status.ahead ?? 0;
  const behind = status.behind ?? 0;
  if (!status.upstream && ahead === 0 && behind === 0) return null;
  const label = status.upstream
    ? `${ahead} ahead, ${behind} behind ${status.upstream}`
    : `${ahead} ahead, ${behind} behind upstream`;
  return (
    <Tooltip content={label}>
      <span aria-label={label} className={styles.aheadBehind} role="note">
        ↑{ahead} ↓{behind}
      </span>
    </Tooltip>
  );
}

function RepoStateBadge({ state }: { state: GitRepoState }) {
  const label = REPO_STATE_LABELS[state];
  if (!label) return null;
  const tone: BadgeTone = state === "bisect" ? "accent" : "warning";
  return (
    <Badge size="xs" tone={tone}>
      {label}
    </Badge>
  );
}

function useRootView(chatId: string | null, root: string) {
  const dispatch = useAppDispatch();
  const view = useAppSelector((state) =>
    selectGitRootView(state, chatId, root),
  );
  const update = useCallback(
    (patch: Partial<GitRootViewState>) =>
      dispatch(updateGitRootView({ chatId, root, patch })),
    [chatId, dispatch, root],
  );
  return [view, update] as const;
}

type CommitToolWindowProps = {
  chatId: string | null;
  status: NormalizedGitStatus;
  refreshing: boolean;
  onRefresh: () => void;
  actions: GitActionsController;
  messageRef: RefObject<HTMLTextAreaElement>;
};

function CommitToolWindow({
  chatId,
  status,
  refreshing,
  onRefresh,
  actions,
  messageRef,
}: CommitToolWindowProps) {
  const dispatch = useAppDispatch();
  const root = status.root;
  const [view, updateView] = useRootView(chatId, root);
  const { canOpen, openFile } = useOpenFileInApp();
  const copy = useCopyToClipboard();
  const [stage, stageState] = useStageGitPathsMutation();
  const [rollback, rollbackState] = useRollbackGitPathsMutation();
  const [dialog, setDialog] = useState<PendingDialog | null>(null);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);

  const inclusion = useMemo<InclusionState>(
    () => ({
      excludedChanges: new Set(view.excludedChanges),
      includedUnversioned: new Set(view.includedUnversioned),
    }),
    [view.excludedChanges, view.includedUnversioned],
  );
  const collapsed = useMemo(() => new Set(view.collapsed), [view.collapsed]);
  const groups = useMemo(
    () => buildChangesTree(status, view.groupByDirectory),
    [status, view.groupByDirectory],
  );
  const rows = useMemo(
    () => flattenChangesTree(groups, collapsed),
    [collapsed, groups],
  );
  const included = useMemo(
    () => includedEntries(status, inclusion),
    [inclusion, status],
  );
  const paths = useMemo(() => commitPaths(included), [included]);
  const selectedNode =
    rows.find((row) => row.node.id === view.selectedId)?.node ?? null;
  const selectedFile =
    selectedNode?.type === "file" ? selectedNode.entry : null;

  useEffect(() => {
    const pruned = pruneInclusion(inclusion, status);
    if (
      pruned.excludedChanges.length !== view.excludedChanges.length ||
      pruned.includedUnversioned.length !== view.includedUnversioned.length
    ) {
      updateView(pruned);
    }
  }, [
    inclusion,
    status,
    updateView,
    view.excludedChanges.length,
    view.includedUnversioned.length,
  ]);

  const setInclusion = useCallback(
    (entries: ChangesTreeEntry[], include: boolean) =>
      updateView(applyInclusion(inclusion, entries, include)),
    [inclusion, updateView],
  );

  const toggleInclude = useCallback(
    (node: ChangesTreeNode) => {
      const entries = collectEntries(node).filter(isCheckable);
      setInclusion(entries, nodeCheckState(node, inclusion) !== true);
    },
    [inclusion, setInclusion],
  );

  const setExpanded = useCallback(
    (id: string, expanded: boolean) => {
      const next = new Set(collapsed);
      if (expanded) next.delete(id);
      else next.add(id);
      updateView({ collapsed: [...next] });
    },
    [collapsed, updateView],
  );

  const showDiff = useCallback(
    (entry: ChangesTreeEntry) =>
      dispatch(
        openGitDiff({
          root,
          path: entry.path,
          old_path: entry.oldPath,
          kind: entry.kind,
          status: entry.status,
        }),
      ),
    [dispatch, root],
  );

  const jumpToSource = (entry: ChangesTreeEntry) =>
    openFile({ path: entry.absolutePath, resolved: true });

  const runStage = async (entries: ChangesTreeEntry[], verb: string) => {
    setFeedback(null);
    try {
      await stage({ root, paths: entries.map((entry) => entry.path) }).unwrap();
      setFeedback({
        tone: "success",
        text: `${verb} ${entries.length} ${
          entries.length === 1 ? "file" : "files"
        }`,
        skipped: [],
      });
    } catch (error) {
      setFeedback({
        tone: "error",
        text: gitErrorText(error).message,
        skipped: [],
      });
    }
  };

  const confirmDialog = async (deleteAddedFiles: boolean) => {
    if (!dialog) return;
    setDialogError(null);
    try {
      const response = await rollback(
        dialog.kind === "rollback"
          ? {
              root,
              paths: rollbackPaths(dialog.entries),
              ...(deleteAddedFiles ? { delete_added_files: true } : {}),
            }
          : {
              root,
              paths: dialog.entries.map((entry) => entry.path),
              delete_untracked: true,
            },
      ).unwrap();
      setDialog(null);
      setFeedback({
        tone: "success",
        text:
          dialog.kind === "rollback"
            ? `Rolled back ${response.restored + response.unstaged} ${
                response.restored + response.unstaged === 1 ? "file" : "files"
              }`
            : `Deleted ${response.deleted} ${
                response.deleted === 1 ? "file" : "files"
              }`,
        skipped: response.skipped,
      });
    } catch (error) {
      setDialogError(gitErrorText(error).message);
    }
  };

  const openDialog = (
    kind: PendingDialog["kind"],
    entries: ChangesTreeEntry[],
  ) => {
    if (entries.length === 0) return;
    setDialogError(null);
    setDialog({ kind, entries });
  };

  const changeEntriesOf = (node: ChangesTreeNode | null) =>
    node ? collectEntries(node).filter((entry) => entry.kind === "change") : [];
  const toolbarRollbackEntries = (() => {
    const fromSelection = changeEntriesOf(selectedNode);
    return fromSelection.length > 0
      ? fromSelection
      : included.filter((entry) => entry.kind === "change");
  })();

  const renderContextMenu = (node: ChangesTreeNode) => {
    if (node.type !== "file") {
      const entries = collectEntries(node);
      const checkable = entries.filter(isCheckable);
      const changes = entries.filter((entry) => entry.kind === "change");
      const untracked = entries.filter((entry) => entry.kind === "untracked");
      const expanded = !collapsed.has(node.id);
      return (
        <>
          {checkable.length > 0 ? (
            <>
              <ContextMenu.Item onSelect={() => setInclusion(checkable, true)}>
                Include All
              </ContextMenu.Item>
              <ContextMenu.Item onSelect={() => setInclusion(checkable, false)}>
                Exclude All
              </ContextMenu.Item>
              <ContextMenu.Separator />
            </>
          ) : null}
          {changes.length > 0 ? (
            <ContextMenu.Item onSelect={() => openDialog("rollback", changes)}>
              Rollback…
            </ContextMenu.Item>
          ) : null}
          {untracked.length > 0 ? (
            <ContextMenu.Item
              color="red"
              onSelect={() => openDialog("delete", untracked)}
            >
              Delete…
            </ContextMenu.Item>
          ) : null}
          <ContextMenu.Item onSelect={() => setExpanded(node.id, !expanded)}>
            {expanded ? "Collapse" : "Expand"}
          </ContextMenu.Item>
        </>
      );
    }
    const entry = node.entry;
    const canJump = canOpen && entry.status !== "DELETED";
    return (
      <>
        {entry.kind !== "conflict" ? (
          <ContextMenu.Item onSelect={() => showDiff(entry)}>
            Show Diff
          </ContextMenu.Item>
        ) : null}
        <ContextMenu.Item
          disabled={!canJump}
          onSelect={() => jumpToSource(entry)}
        >
          Jump to Source
        </ContextMenu.Item>
        <ContextMenu.Separator />
        {entry.kind === "change" ? (
          <ContextMenu.Item onSelect={() => openDialog("rollback", [entry])}>
            Rollback…
          </ContextMenu.Item>
        ) : null}
        {entry.kind === "untracked" ? (
          <>
            <ContextMenu.Item
              disabled={stageState.isLoading}
              onSelect={() => void runStage([entry], "Added")}
            >
              Add to VCS
            </ContextMenu.Item>
            <ContextMenu.Item
              color="red"
              onSelect={() => openDialog("delete", [entry])}
            >
              Delete…
            </ContextMenu.Item>
          </>
        ) : null}
        {entry.kind === "conflict" ? (
          <ContextMenu.Item
            disabled={stageState.isLoading}
            onSelect={() => void runStage([entry], "Resolved")}
          >
            Mark as Resolved
          </ContextMenu.Item>
        ) : null}
        <ContextMenu.Separator />
        <ContextMenu.Item onSelect={() => copy(entry.path)}>
          Copy Path
        </ContextMenu.Item>
      </>
    );
  };

  const isEmpty =
    status.changes.length === 0 &&
    status.untracked.length === 0 &&
    status.conflicted.length === 0;

  return (
    <>
      <ChangesToolbar
        canRollback={toolbarRollbackEntries.length > 0}
        canShowDiff={selectedFile !== null && selectedFile.kind !== "conflict"}
        groupByDirectory={view.groupByDirectory}
        onCollapseAll={() => updateView({ collapsed: containerIds(groups) })}
        onExpandAll={() => updateView({ collapsed: [] })}
        onRefresh={onRefresh}
        onRollback={() => openDialog("rollback", toolbarRollbackEntries)}
        onShowDiff={() => {
          if (selectedFile) showDiff(selectedFile);
        }}
        onToggleGroupByDirectory={() =>
          updateView({ groupByDirectory: !view.groupByDirectory })
        }
        refreshing={refreshing}
        remote={{
          fetching: actions.fetching,
          pulling: actions.pulling,
          pushing: actions.pushing,
          canPush: actions.canPush,
          updateBlockedReason: actions.blockedReason,
          onFetch: () => void actions.fetch(),
          onPull: () => void actions.pull(),
          onPush: () => actions.openPush(),
        }}
      />
      <GitFeedback
        feedback={actions.feedback}
        onDismiss={actions.clearFeedback}
      />
      {feedback ? (
        <div
          className={
            feedback.tone === "success" ? styles.successText : styles.errorText
          }
          role={feedback.tone === "success" ? "status" : "alert"}
        >
          <span>{feedback.text}</span>
          <SkippedPaths skipped={feedback.skipped} />
        </div>
      ) : null}
      {status.truncated ? (
        <p className={styles.notice} role="note">
          The change list is truncated.
        </p>
      ) : null}
      {isEmpty ? (
        <p className={styles.emptyState} role="status">
          No changes — working tree clean
        </p>
      ) : (
        <ChangesTree
          collapsed={collapsed}
          groupByDirectory={view.groupByDirectory}
          inclusion={inclusion}
          onSelect={(id) => updateView({ selectedId: id })}
          onSetExpanded={setExpanded}
          onShowDiff={showDiff}
          onToggleInclude={toggleInclude}
          renderContextMenu={renderContextMenu}
          rows={rows}
          selectedId={view.selectedId}
        />
      )}
      <CommitPanel
        amendAvailable={!status.extended || status.headOid !== null}
        canPush={actions.canPush}
        includedCount={included.length}
        key={root}
        messageRef={messageRef}
        onCommitted={() => setFeedback(null)}
        onPushRequested={() => actions.openPush()}
        paths={paths}
        root={root}
        totalCount={checkableCount(status)}
      />
      {dialog?.kind === "rollback" ? (
        <RollbackDialog
          busy={rollbackState.isLoading}
          entries={dialog.entries}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onConfirm={(deleteAdded) => void confirmDialog(deleteAdded)}
        />
      ) : null}
      {dialog?.kind === "delete" ? (
        <DeleteUnversionedDialog
          busy={rollbackState.isLoading}
          entries={dialog.entries}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onConfirm={() => void confirmDialog(false)}
        />
      ) : null}
    </>
  );
}

export function GitDock() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectFocusedWorkspaceChatId);
  const activeRoot = useAppSelector((state) =>
    selectActiveGitRoot(state, chatId),
  );
  const statusQuery = useGitStatus();
  const roots = useMemo(
    () => (statusQuery.data?.roots ?? []).map(normalizeGitStatus),
    [statusQuery.data?.roots],
  );
  const rootPaths = useMemo(() => roots.map((root) => root.root), [roots]);

  useEffect(() => {
    if (rootPaths.length === 0 || rootPaths.includes(activeRoot)) return;
    dispatch(setActiveGitRoot({ chatId, root: rootPaths[0] ?? "" }));
  }, [activeRoot, chatId, dispatch, rootPaths]);

  const activeStatus =
    roots.find((root) => root.root === activeRoot) ?? roots.at(0);
  const noRepositories = statusQuery.data !== undefined && roots.length === 0;
  const messageRef = useRef<HTMLTextAreaElement>(null);
  const actions = useGitActions({
    root: activeStatus?.root ?? "",
    status: activeStatus ?? null,
  });
  const focusCommitMessage = useCallback(() => {
    // The closing popover restores focus to its trigger on a timer; queue behind it.
    window.setTimeout(() =>
      window.setTimeout(() => messageRef.current?.focus()),
    );
  }, []);

  return (
    <div className={styles.dockPanel} data-testid="git-dock-panel">
      <header className={styles.header}>
        <h2 className={styles.title}>Commit</h2>
        {activeStatus ? (
          <div className={styles.headerMeta}>
            <BranchPopup
              controller={actions}
              onCommit={focusCommitMessage}
              trigger={
                <BranchChip
                  branch={activeStatus.branch}
                  headDetached={activeStatus.headDetached}
                  headOid={activeStatus.headOid}
                />
              }
            />
            <AheadBehind status={activeStatus} />
            <RepoStateBadge state={activeStatus.repoState} />
          </div>
        ) : null}
      </header>

      {roots.length > 1 && activeStatus ? (
        <div className={styles.rootPicker}>
          <FieldSelect
            aria-label="Git root"
            onChange={(root) => dispatch(setActiveGitRoot({ chatId, root }))}
            options={roots.map((root) => ({
              value: root.root,
              label: rootLabel(root.root),
            }))}
            value={activeStatus.root}
          />
        </div>
      ) : null}

      {noRepositories ? (
        <p className={styles.emptyState} role="status">
          No git repository found in this workspace.
        </p>
      ) : activeStatus ? (
        <CommitToolWindow
          actions={actions}
          chatId={chatId}
          messageRef={messageRef}
          key={activeStatus.root}
          onRefresh={() => void statusQuery.refetch()}
          refreshing={statusQuery.isFetching}
          status={activeStatus}
        />
      ) : statusQuery.error ? (
        <p className={styles.errorText} role="alert">
          {gitErrorText(statusQuery.error).message}
        </p>
      ) : (
        <LoadingState label="Loading Git status" />
      )}
      {actions.dialogs}
    </div>
  );
}
