import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useStore } from "react-redux";
import { v4 as uuidv4 } from "uuid";
import { skipToken } from "@reduxjs/toolkit/query";
import { GitBranch } from "lucide-react";
import type { AppStore } from "../../app/store";
import { Icon, Popover } from "../../components/ui";
import { ComposerTrigger } from "../../components/ComposerKit";
import {
  DEFAULT_MODE,
  selectRuntimeById,
  selectThreadById,
  selectThreadWorktreeById,
  setThreadWorktree,
  useThreadId,
} from "../Chat/Thread";
import { selectApiKey, selectConfig, selectHost } from "../Config/configSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import {
  ChatCommandHttpError,
  sendChatCommand,
} from "../../services/refact/chatCommands";
import {
  useCreateWorktreeMutation,
  useDeleteWorktreeMutation,
  useGetWorktreeDiffQuery,
  useGetWorktreesSummaryQuery,
  useListWorktreesQuery,
  useOpenWorktreeMutation,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import {
  CreateWorktreeModal,
  type CreateWorktreeValues,
} from "./CreateWorktreeModal";
import { WorktreeMenu } from "./WorktreeMenu";
import { WorktreeStatusBadge } from "./WorktreeStatusBadge";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

const EMPTY_WORKTREE_RECORDS: WorktreeRecordView[] = [];
const WORKTREE_BRANCH_ARGS = { include_branches: true };
const useAppStore = useStore.withTypes<AppStore>();

type AttachmentOutcome =
  | "attached"
  | "rejected"
  | "unconfirmed"
  | "blocked"
  | "stale";

function compactPath(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return normalized || path;
  return parts.slice(-2).join("/");
}

function compactWorktreeLabel(label: string): string {
  const normalized = label.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 3) return normalized || label;
  return parts.slice(-3).join("/");
}

function worktreeLabel(worktree: WorktreeMeta | null): string {
  if (!worktree) return "Main";
  const branch = worktree.branch?.trim();
  return branch !== undefined && branch.length > 0
    ? compactWorktreeLabel(branch)
    : compactPath(worktree.root);
}

function sanitizeBranchComponent(value: string): string {
  const component = value
    .split("")
    .map((char) => {
      if (/^[a-zA-Z0-9_-]$/.test(char)) return char;
      return "-";
    })
    .join("")
    .replace(/^-+|-+$/g, "");
  return component.length > 0 ? component : "chat";
}

function defaultBranchName(chatId: string): string {
  const seedComponent = sanitizeBranchComponent(chatId).slice(0, 12);
  const seed = seedComponent.length > 0 ? seedComponent : "chat";
  return `refact/chat/${seed}`;
}

type WorktreeControlProps = {
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export const WorktreeControl: React.FC<WorktreeControlProps> = (props) => {
  const chatId = useThreadId();
  const config = useAppSelector(selectConfig);
  const project = useAppSelector((state) => state.current_project);
  const principal = useRef({ key: config.apiKey, epoch: 0 });
  if (principal.current.key !== config.apiKey) {
    principal.current = {
      key: config.apiKey,
      epoch: principal.current.epoch + 1,
    };
  }
  const identity = JSON.stringify([
    chatId,
    getEngineEndpointIdentity(config),
    principal.current.epoch,
    project,
  ]);
  return <WorktreeControlInstance key={identity} {...props} chatId={chatId} />;
};

const WorktreeControlInstance: React.FC<
  WorktreeControlProps & { chatId: string }
> = ({ disabled, onOpenChange, chatId }) => {
  const dispatch = useAppDispatch();
  const store = useAppStore();
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const currentWorktree = useAppSelector((state) =>
    selectThreadWorktreeById(state, chatId),
  );
  const boundTaskMeta = useAppSelector(
    (state) => selectThreadById(state, chatId)?.task_meta,
  );
  const isChatRunning = useAppSelector((state) => {
    const runtime = selectRuntimeById(state, chatId);
    return (
      runtime?.streaming === true ||
      runtime?.waiting_for_response === true ||
      runtime?.session_state === "generating" ||
      runtime?.session_state === "executing_tools"
    );
  });
  const managedScope =
    boundTaskMeta?.role === "agent" ||
    boundTaskMeta?.role === "agents" ||
    currentWorktree?.kind === "task_agent";
  const scopeDisabled = Boolean(disabled) || managedScope;
  const host = useAppSelector(selectHost);
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const project = useAppSelector((state) => state.current_project);
  const projectIdentity = JSON.stringify(project);
  const sourceIsCurrent = useCallback(() => {
    const state = store.getState();
    return (
      mounted.current &&
      getEngineEndpointIdentity(state.config) ===
        getEngineEndpointIdentity(config) &&
      state.config.apiKey === config.apiKey &&
      JSON.stringify(state.current_project) === projectIdentity
    );
  }, [config, projectIdentity, store]);
  const [menuOpen, setMenuOpen] = useState(false);
  const {
    data,
    isLoading,
    isFetching,
    isError,
    refetch: refetchList,
  } = useListWorktreesQuery(undefined, {
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const handleMenuOpenChange = useCallback(
    (open: boolean) => {
      if (open && scopeDisabled) return;
      setMenuOpen(open);
      if (open) void refetchList();
      onOpenChange?.(open);
    },
    [onOpenChange, refetchList, scopeDisabled],
  );
  const [createOpen, setCreateOpen] = useState(false);
  const createInFlightRef = useRef(false);
  const attachInFlightRef = useRef(false);
  const [isAttaching, setIsAttaching] = useState(false);
  const [isCreatingTransaction, setIsCreatingTransaction] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const copyToClipboard = useCopyToClipboard();
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const { data: branchData } = useListWorktreesQuery(
    createOpen && !scopeDisabled ? WORKTREE_BRANCH_ARGS : skipToken,
  );
  const [createWorktree] = useCreateWorktreeMutation();
  const [deleteWorktree] = useDeleteWorktreeMutation();
  const [openWorktree] = useOpenWorktreeMutation();

  const { data: inventory } = useGetWorktreesSummaryQuery(
    menuOpen
      ? { source_workspace_root: data?.source_workspace_root }
      : skipToken,
  );
  const records = data?.worktrees ?? EMPTY_WORKTREE_RECORDS;
  const currentRecord = useMemo(
    () =>
      records.find(
        (record) =>
          record.meta.id === currentWorktree?.id &&
          record.meta.source_workspace_root ===
            currentWorktree.source_workspace_root,
      ),
    [currentWorktree?.id, currentWorktree?.source_workspace_root, records],
  );
  const inventoryNotice = isError
    ? currentWorktree
      ? "Worktree list unavailable. Attached checkout unchanged."
      : "Worktree list unavailable."
    : currentWorktree && data && !currentRecord
      ? "Attached checkout is not in this list. It may be stale or incomplete; attachment unchanged."
      : null;
  const scopeChangeBlockedReason = managedScope
    ? "Checkout managed by task lifecycle. Use the task card's owner planner workflow requests for recovery or resume."
    : isChatRunning
      ? "Checkout scope cannot change while this chat is running. Retry when idle."
      : null;
  const currentDiffQuery = currentRecord
    ? {
        id: currentRecord.meta.id,
        source_workspace_root: currentRecord.meta.source_workspace_root,
        max_patch_bytes: 1,
      }
    : skipToken;
  const { data: currentDiff } = useGetWorktreeDiffQuery(currentDiffQuery, {
    pollingInterval: 5000,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const mainWorkspacePath = data?.source_workspace_root;
  const copyPath = currentWorktree?.root ?? mainWorkspacePath ?? null;
  const sourceBranch = data?.source_current_branch?.trim();
  const mainLabel = sourceBranch
    ? compactWorktreeLabel(sourceBranch)
    : "No branch";
  const label = currentWorktree ? worktreeLabel(currentWorktree) : mainLabel;
  const branchLabel = currentWorktree?.branch?.trim();
  const fullLabel = currentWorktree
    ? branchLabel !== undefined && branchLabel.length > 0
      ? branchLabel
      : currentWorktree.root
    : sourceBranch
      ? `Main workspace · ${sourceBranch}`
      : "Main workspace · no branch detected";
  const triggerLabel = managedScope
    ? `${fullLabel} · Task-managed scope`
    : fullLabel;
  const hostCanOpenFolder =
    host === "vscode" || host === "jetbrains" || host === "ide";
  const branchSuggestion = useMemo(
    () => defaultBranchName(chatId || DEFAULT_MODE),
    [chatId],
  );
  const baseBranch = sourceBranch ?? "";
  const baseBranchOptions = useMemo(() => {
    const sourceBranches = branchData?.source_branches ?? [];
    const branches = [
      sourceBranch,
      ...sourceBranches,
      ...records.flatMap((record) => [
        record.meta.base_branch,
        record.meta.branch,
      ]),
    ];
    return branches.filter((branch): branch is string => Boolean(branch));
  }, [branchData?.source_branches, records, sourceBranch]);

  const attachWorktree = useCallback(
    async (worktree: WorktreeMeta | null): Promise<AttachmentOutcome> => {
      if (!sourceIsCurrent()) return "stale";
      if (!chatId || scopeDisabled || attachInFlightRef.current)
        return "blocked";
      const before = selectRuntimeById(store.getState(), chatId);
      const taskMeta = before?.thread.task_meta;
      if (
        !before ||
        before.streaming ||
        before.waiting_for_response ||
        before.session_state === "generating" ||
        before.session_state === "executing_tools"
      )
        return "blocked";
      if (
        Boolean(scopeChangeBlockedReason) ||
        taskMeta?.role === "agent" ||
        taskMeta?.role === "agents" ||
        before.thread.worktree?.kind === "task_agent" ||
        worktree?.kind === "task_agent"
      ) {
        setFeedback(
          scopeChangeBlockedReason ??
            "This checkout is managed by task lifecycle.",
        );
        return "blocked";
      }
      attachInFlightRef.current = true;
      setIsAttaching(true);
      setFeedback("Updating checkout scope...");
      try {
        await sendChatCommand(
          chatId,
          config,
          apiKey,
          {
            type: "set_params",
            patch: worktree ? { worktree_id: worktree.id } : { worktree: null },
          },
          false,
          uuidv4(),
        );
        if (!sourceIsCurrent()) return "stale";
        const after = selectThreadById(store.getState(), chatId);
        if (
          !after ||
          JSON.stringify(after.task_meta) !== JSON.stringify(taskMeta) ||
          (after.worktree?.id !== before.thread.worktree?.id &&
            after.worktree?.id !== worktree?.id)
        ) {
          setFeedback(
            "Checkout context changed; inspect the current attachment before another action.",
          );
          return "stale";
        }
        dispatch(
          setThreadWorktree({
            chatId,
            worktree:
              worktree && after.worktree?.id === worktree.id
                ? after.worktree
                : worktree,
          }),
        );
        setFeedback(worktree ? "Worktree attached." : "Using main workspace.");
        return "attached";
      } catch (error) {
        if (!sourceIsCurrent()) return "stale";
        const rejected =
          error instanceof ChatCommandHttpError &&
          error.status >= 400 &&
          error.status < 500 &&
          error.status !== 408;
        setFeedback(
          rejected
            ? `Worktree update failed: ${worktreeErrorText(error)}`
            : `Worktree update unconfirmed: ${worktreeErrorText(
                error,
              )}. Inspect the chat attachment before retrying.`,
        );
        return rejected ? "rejected" : "unconfirmed";
      } finally {
        attachInFlightRef.current = false;
        if (sourceIsCurrent()) setIsAttaching(false);
      }
    },
    [
      apiKey,
      chatId,
      config,
      dispatch,
      scopeDisabled,
      scopeChangeBlockedReason,
      sourceIsCurrent,
      store,
    ],
  );

  const handleSelect = useCallback(
    (record: WorktreeRecordView) => {
      void attachWorktree(record.meta).then((outcome) => {
        if (outcome === "attached" && sourceIsCurrent())
          handleMenuOpenChange(false);
      });
    },
    [attachWorktree, handleMenuOpenChange, sourceIsCurrent],
  );

  const handleDetach = useCallback(() => {
    void attachWorktree(null).then((outcome) => {
      if (outcome === "attached" && sourceIsCurrent())
        handleMenuOpenChange(false);
    });
  }, [attachWorktree, handleMenuOpenChange, sourceIsCurrent]);

  const handleCreate = useCallback(
    async ({ branch, baseBranch }: CreateWorktreeValues) => {
      if (
        !chatId ||
        scopeDisabled ||
        !sourceIsCurrent() ||
        createInFlightRef.current ||
        attachInFlightRef.current ||
        scopeChangeBlockedReason
      )
        return;
      createInFlightRef.current = true;
      setIsCreatingTransaction(true);
      setCreateError(null);
      const request = {
        branch,
        kind: "chat" as const,
        ...(baseBranch ? { base_branch: baseBranch } : {}),
      };
      try {
        const response = await createWorktree(request).unwrap();
        if (!sourceIsCurrent()) return;
        const outcome = await attachWorktree(response.worktree.meta);
        if (!sourceIsCurrent() || outcome === "stale") return;
        if (outcome === "attached") {
          setCreateOpen(false);
          handleMenuOpenChange(false);
        } else if (
          outcome === "rejected" &&
          response.worktree.meta.kind === "chat" &&
          response.worktree.reference_count === 0 &&
          response.worktree.references.length === 0 &&
          selectThreadWorktreeById(store.getState(), chatId)?.id !==
            response.worktree.meta.id &&
          (response.worktree.referencing_chat_ids?.length ?? 0) === 0
        ) {
          try {
            const cleanup = await deleteWorktree({
              id: response.worktree.meta.id,
              source_workspace_root:
                response.worktree.meta.source_workspace_root,
              delete_branch: true,
            }).unwrap();
            if (!sourceIsCurrent()) return;
            if (!cleanup.deleted)
              setFeedback(
                `Inspect created checkout ${response.worktree.meta.id} at ${response.worktree.meta.root}; cleanup is not confirmed.`,
              );
            setCreateError(
              cleanup.deleted
                ? "Worktree attach failed; created worktree was deleted."
                : `Cleanup not confirmed for created worktree ${response.worktree.meta.id}. Inspect it before another action.`,
            );
          } catch (error) {
            if (sourceIsCurrent()) {
              setFeedback(
                `Inspect created checkout ${response.worktree.meta.id} at ${response.worktree.meta.root}; cleanup is not confirmed.`,
              );
              setCreateError(
                `Cleanup not confirmed for created worktree ${
                  response.worktree.meta.id
                }: ${worktreeErrorText(
                  error,
                )}. Inspect it before another action.`,
              );
            }
          }
        } else {
          setFeedback(
            `Created checkout retained: ${response.worktree.meta.id} at ${response.worktree.meta.root}. Inspect its attachment before another action.`,
          );
          setCreateError(
            `Attachment not confirmed; created worktree retained (${response.worktree.meta.id}). Inspect the chat attachment before retrying or cleanup.`,
          );
        }
      } catch (error) {
        if (sourceIsCurrent()) setCreateError(worktreeErrorText(error));
      } finally {
        createInFlightRef.current = false;
        if (sourceIsCurrent()) setIsCreatingTransaction(false);
      }
    },
    [
      attachWorktree,
      chatId,
      createWorktree,
      deleteWorktree,
      handleMenuOpenChange,
      scopeDisabled,
      scopeChangeBlockedReason,
      sourceIsCurrent,
      store,
    ],
  );

  const handleCopyPath = useCallback(
    (target?: WorktreeMeta) => {
      const path = target?.root ?? mainWorkspacePath;
      if (!path) {
        setFeedback("No path is available to copy.");
        return;
      }
      copyToClipboard(path);
      setFeedback("Path copied to clipboard.");
    },
    [mainWorkspacePath, copyToClipboard],
  );

  const handleOpenInNewWindow = useCallback(
    async (target?: WorktreeMeta) => {
      if (!target) {
        handleCopyPath();
        return;
      }
      try {
        const response = await openWorktree({
          id: target.id,
          source_workspace_root: target.source_workspace_root,
        }).unwrap();
        if (!sourceIsCurrent()) return;
        if (response.can_open_folder && hostCanOpenFolder) {
          openFolderInNewWindow(response.path);
          setFeedback("Opening worktree in a new window.");
        } else {
          copyToClipboard(response.path);
          setFeedback("Path copied to clipboard.");
        }
      } catch (error) {
        if (sourceIsCurrent())
          setFeedback(`Open failed: ${worktreeErrorText(error)}`);
      }
    },
    [
      copyToClipboard,
      handleCopyPath,
      hostCanOpenFolder,
      openFolderInNewWindow,
      openWorktree,
      sourceIsCurrent,
    ],
  );

  return (
    <>
      <Popover
        open={menuOpen && !scopeDisabled}
        onOpenChange={handleMenuOpenChange}
      >
        <Popover.Trigger asChild>
          <ComposerTrigger
            chevron
            data-testid="worktree-control-trigger"
            className={currentWorktree ? styles.triggerActive : undefined}
            title={triggerLabel}
            aria-label={`Worktree scope: ${triggerLabel}`}
            disabled={scopeDisabled}
          >
            <span className={styles.triggerInner}>
              {!currentWorktree && sourceBranch && (
                <Icon
                  icon={GitBranch}
                  size="sm"
                  tone="muted"
                  className={styles.branchIcon}
                />
              )}
              <span
                className={styles.triggerText}
                data-shrink-at={
                  Boolean(currentWorktree) || Boolean(sourceBranch)
                    ? "1"
                    : undefined
                }
              >
                {label}
              </span>
              {currentWorktree && (
                <WorktreeStatusBadge
                  worktree={currentWorktree}
                  record={currentRecord}
                  additions={currentDiff?.stats.additions}
                  deletions={currentDiff?.stats.deletions}
                />
              )}
            </span>
          </ComposerTrigger>
        </Popover.Trigger>
        {!scopeDisabled && (
          <WorktreeMenu
            inventory={inventory}
            mainPath={mainWorkspacePath}
            mainBranch={sourceBranch}
            onClose={() => handleMenuOpenChange(false)}
            sourceIsCurrent={sourceIsCurrent}
            isAttached={(meta) => {
              const attached = selectThreadWorktreeById(
                store.getState(),
                chatId,
              );
              return (
                attached?.id === meta.id &&
                attached.source_workspace_root === meta.source_workspace_root
              );
            }}
            chatId={chatId}
            currentWorktree={currentWorktree}
            currentRecord={currentRecord}
            records={records}
            isLoading={isLoading}
            feedback={feedback}
            inventoryNotice={inventoryNotice}
            isRefreshing={isFetching}
            onRefresh={() => void refetchList()}
            scopeChangeBlockedReason={scopeChangeBlockedReason}
            scopeChangePending={isAttaching || isCreatingTransaction}
            canCopyPath={Boolean(copyPath)}
            onCreate={() => {
              if (
                Boolean(scopeChangeBlockedReason) ||
                isAttaching ||
                isCreatingTransaction
              )
                return;
              setCreateError(null);
              setCreateOpen(true);
            }}
            onSelect={handleSelect}
            onDetach={handleDetach}
            onOpenInNewWindow={(target) => void handleOpenInNewWindow(target)}
            onCopyPath={handleCopyPath}
          />
        )}
      </Popover>

      <CreateWorktreeModal
        open={createOpen && !scopeDisabled}
        defaultBranch={branchSuggestion}
        defaultBaseBranch={baseBranch}
        baseBranchOptions={baseBranchOptions}
        isCreating={isCreatingTransaction}
        error={createError}
        onOpenChange={(open) => {
          if (!open && createInFlightRef.current) return;
          setCreateOpen(open);
        }}
        onCreate={handleCreate}
      />
    </>
  );
};

WorktreeControl.displayName = "WorktreeControl";
