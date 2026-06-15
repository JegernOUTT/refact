import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { GitBranch } from "lucide-react";
import { Icon, Popover } from "../../components/ui";
import {
  DEFAULT_MODE,
  selectChatId,
  selectThreadWorktree,
  setThreadWorktree,
} from "../Chat/Thread";
import { selectApiKey, selectConfig, selectHost } from "../Config/configSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import {
  updateChatParams,
  useCreateWorktreeMutation,
  useDeleteWorktreeMutation,
  useGetWorktreeDiffQuery,
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

type AttachWorktreeOptions = {
  optimistic?: boolean;
};

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

export const WorktreeControl: React.FC<WorktreeControlProps> = ({
  disabled,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const currentWorktree = useAppSelector(selectThreadWorktree);
  const host = useAppSelector(selectHost);
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const [menuOpen, setMenuOpen] = useState(false);
  const handleMenuOpenChange = useCallback(
    (open: boolean) => {
      setMenuOpen(open);
      onOpenChange?.(open);
    },
    [onOpenChange],
  );
  const [createOpen, setCreateOpen] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const pendingCreatedWorktreeIdRef = useRef<string | null>(null);
  const [pendingCreatedWorktreeId, setPendingCreatedWorktreeIdState] = useState<
    string | null
  >(null);
  const setPendingCreatedWorktreeId = useCallback((id: string | null) => {
    pendingCreatedWorktreeIdRef.current = id;
    setPendingCreatedWorktreeIdState(id);
  }, []);
  const copyToClipboard = useCopyToClipboard();
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const { data, isLoading } = useListWorktreesQuery(undefined, {
    pollingInterval: 5000,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const [createWorktree, createState] = useCreateWorktreeMutation();
  const [deleteWorktree] = useDeleteWorktreeMutation();
  const [openWorktree] = useOpenWorktreeMutation();

  const records = data?.worktrees ?? EMPTY_WORKTREE_RECORDS;
  const currentRecord = useMemo(
    () => records.find((record) => record.meta.id === currentWorktree?.id),
    [currentWorktree?.id, records],
  );
  const isPendingCreatedWorktree =
    currentWorktree?.id !== undefined &&
    pendingCreatedWorktreeIdRef.current === currentWorktree.id;
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
  const triggerLabel = fullLabel;
  const hostCanOpenFolder =
    host === "vscode" || host === "jetbrains" || host === "ide";
  const branchSuggestion = useMemo(
    () => defaultBranchName(chatId || DEFAULT_MODE),
    [chatId],
  );
  const baseBranch = sourceBranch ?? "";
  const baseBranchOptions = useMemo(() => {
    const sourceBranches = data?.source_branches ?? [];
    const branches = [
      sourceBranch,
      ...sourceBranches,
      ...records.flatMap((record) => [
        record.meta.base_branch,
        record.meta.branch,
      ]),
    ];
    return branches.filter((branch): branch is string => Boolean(branch));
  }, [data?.source_branches, records, sourceBranch]);

  const attachWorktree = useCallback(
    async (
      worktree: WorktreeMeta | null,
      { optimistic = true }: AttachWorktreeOptions = {},
    ): Promise<boolean> => {
      if (!chatId) return false;
      const previousWorktree = currentWorktree;
      if (optimistic) {
        dispatch(setThreadWorktree({ chatId, worktree }));
        setFeedback(worktree ? "Worktree attached." : "Using main workspace.");
      }
      try {
        await updateChatParams(
          chatId,
          worktree ? { worktree_id: worktree.id } : { worktree: null },
          config,
          apiKey,
        );
        if (!optimistic) {
          dispatch(setThreadWorktree({ chatId, worktree }));
          setFeedback(
            worktree ? "Worktree attached." : "Using main workspace.",
          );
        }
        return true;
      } catch (error) {
        if (optimistic) {
          dispatch(setThreadWorktree({ chatId, worktree: previousWorktree }));
        }
        setFeedback(`Worktree update failed: ${worktreeErrorText(error)}`);
        return false;
      }
    },
    [apiKey, chatId, config, currentWorktree, dispatch],
  );

  useEffect(() => {
    if (
      !currentWorktree ||
      !data ||
      currentRecord !== undefined ||
      isPendingCreatedWorktree
    ) {
      return;
    }
    void attachWorktree(null);
  }, [
    attachWorktree,
    currentRecord,
    currentWorktree,
    data,
    isPendingCreatedWorktree,
  ]);

  useEffect(() => {
    if (currentRecord?.meta.id === pendingCreatedWorktreeId) {
      setPendingCreatedWorktreeId(null);
    }
  }, [
    currentRecord?.meta.id,
    pendingCreatedWorktreeId,
    setPendingCreatedWorktreeId,
  ]);

  const handleSelect = useCallback(
    (record: WorktreeRecordView) => {
      void attachWorktree(record.meta).then((attached) => {
        if (attached) setMenuOpen(false);
      });
    },
    [attachWorktree],
  );

  const handleDetach = useCallback(() => {
    void attachWorktree(null).then((detached) => {
      if (detached) setMenuOpen(false);
    });
  }, [attachWorktree]);

  const handleCreate = useCallback(
    async ({ branch, baseBranch }: CreateWorktreeValues) => {
      if (!chatId) return;
      setCreateError(null);
      const request = {
        branch,
        kind: "chat" as const,
        ...(baseBranch ? { base_branch: baseBranch } : {}),
      };
      try {
        const response = await createWorktree(request).unwrap();
        setPendingCreatedWorktreeId(response.worktree.meta.id);
        const attached = await attachWorktree(response.worktree.meta, {
          optimistic: false,
        });
        if (attached) {
          setCreateOpen(false);
          setMenuOpen(false);
        } else {
          setPendingCreatedWorktreeId(null);
          await deleteWorktree({
            id: response.worktree.meta.id,
            delete_branch: true,
          }).unwrap();
          setCreateError(
            "Worktree attach failed; created worktree was deleted.",
          );
        }
      } catch (error) {
        setPendingCreatedWorktreeId(null);
        setCreateError(worktreeErrorText(error));
      }
    },
    [
      attachWorktree,
      chatId,
      createWorktree,
      deleteWorktree,
      setPendingCreatedWorktreeId,
    ],
  );

  const handleCopyPath = useCallback(() => {
    if (!copyPath) {
      setFeedback("No path is available to copy.");
      return;
    }
    copyToClipboard(copyPath);
    setFeedback("Path copied to clipboard.");
  }, [copyPath, copyToClipboard]);

  const handleOpenInNewWindow = useCallback(async () => {
    if (!currentWorktree) {
      handleCopyPath();
      return;
    }
    try {
      const response = await openWorktree({ id: currentWorktree.id }).unwrap();
      if (response.can_open_folder && hostCanOpenFolder) {
        openFolderInNewWindow(response.path);
        setFeedback("Opening worktree in a new window.");
      } else {
        copyToClipboard(response.path);
        setFeedback("Path copied to clipboard.");
      }
    } catch (error) {
      setFeedback(`Open failed: ${worktreeErrorText(error)}`);
    }
  }, [
    copyToClipboard,
    currentWorktree,
    handleCopyPath,
    hostCanOpenFolder,
    openFolderInNewWindow,
    openWorktree,
  ]);

  return (
    <>
      <Popover open={menuOpen} onOpenChange={handleMenuOpenChange}>
        <Popover.Trigger asChild>
          <button
            type="button"
            data-testid="worktree-control-trigger"
            className={`${styles.trigger} ${
              currentWorktree ? styles.triggerActive : ""
            }`}
            title={triggerLabel}
            aria-label={`Worktree scope: ${triggerLabel}`}
            disabled={disabled}
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
              <span className={styles.triggerText}>{label}</span>
              {currentWorktree && (
                <WorktreeStatusBadge
                  worktree={currentWorktree}
                  record={currentRecord}
                  additions={currentDiff?.stats.additions}
                  deletions={currentDiff?.stats.deletions}
                />
              )}
            </span>
          </button>
        </Popover.Trigger>
        <WorktreeMenu
          currentWorktree={currentWorktree}
          currentRecord={currentRecord}
          records={records}
          isLoading={isLoading}
          feedback={feedback}
          canCopyPath={Boolean(copyPath)}
          onCreate={() => {
            setCreateError(null);
            setCreateOpen(true);
          }}
          onSelect={handleSelect}
          onDetach={handleDetach}
          onOpenInNewWindow={() => void handleOpenInNewWindow()}
          onCopyPath={handleCopyPath}
        />
      </Popover>

      <CreateWorktreeModal
        open={createOpen}
        defaultBranch={branchSuggestion}
        defaultBaseBranch={baseBranch}
        baseBranchOptions={baseBranchOptions}
        isCreating={createState.isLoading}
        error={createError}
        onOpenChange={setCreateOpen}
        onCreate={handleCreate}
      />
    </>
  );
};

WorktreeControl.displayName = "WorktreeControl";
