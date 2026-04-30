import React, { useCallback, useMemo, useState } from "react";
import { Flex, Popover, Text } from "@radix-ui/themes";
import {
  DEFAULT_MODE,
  selectChatId,
  selectThreadWorktree,
  setThreadWorktree,
} from "../Chat/Thread";
import { selectApiKey, selectHost, selectLspPort } from "../Config/configSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import {
  updateChatParams,
  useCreateWorktreeMutation,
  useDeleteWorktreeMutation,
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

export const WorktreeControl: React.FC = () => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const currentWorktree = useAppSelector(selectThreadWorktree);
  const host = useAppSelector(selectHost);
  const lspPort = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const [menuOpen, setMenuOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const copyToClipboard = useCopyToClipboard();
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const { data, isLoading } = useListWorktreesQuery(undefined);
  const [createWorktree, createState] = useCreateWorktreeMutation();
  const [deleteWorktree] = useDeleteWorktreeMutation();
  const [openWorktree] = useOpenWorktreeMutation();

  const records = data?.worktrees ?? EMPTY_WORKTREE_RECORDS;
  const currentRecord = useMemo(
    () => records.find((record) => record.meta.id === currentWorktree?.id),
    [currentWorktree?.id, records],
  );
  const mainWorkspacePath = data?.source_workspace_root;
  const copyPath = currentWorktree?.root ?? mainWorkspacePath ?? null;
  const label = worktreeLabel(currentWorktree);
  const branchLabel = currentWorktree?.branch?.trim();
  const fullLabel =
    branchLabel !== undefined && branchLabel.length > 0 ? branchLabel : label;
  const hostCanOpenFolder =
    host === "vscode" || host === "jetbrains" || host === "ide";
  const branchSuggestion = useMemo(
    () => defaultBranchName(chatId || DEFAULT_MODE),
    [chatId],
  );
  const baseBranch =
    currentWorktree?.base_branch ?? currentWorktree?.branch ?? "main";
  const baseBranchOptions = useMemo(() => {
    const branches = records.flatMap((record) => [
      record.meta.base_branch,
      record.meta.branch,
    ]);
    return branches.filter((branch): branch is string => Boolean(branch));
  }, [records]);

  const attachWorktree = useCallback(
    async (worktree: WorktreeMeta | null): Promise<boolean> => {
      if (!chatId || !lspPort) return false;
      const previousWorktree = currentWorktree;
      dispatch(setThreadWorktree({ chatId, worktree }));
      setFeedback(worktree ? "Worktree attached." : "Using main workspace.");
      try {
        await updateChatParams(
          chatId,
          worktree ? { worktree_id: worktree.id } : { worktree: null },
          lspPort,
          apiKey,
        );
        return true;
      } catch (error) {
        dispatch(setThreadWorktree({ chatId, worktree: previousWorktree }));
        setFeedback(`Worktree update failed: ${worktreeErrorText(error)}`);
        return false;
      }
    },
    [apiKey, chatId, currentWorktree, dispatch, lspPort],
  );

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
      try {
        const response = await createWorktree({
          branch,
          base_branch: baseBranch,
          kind: "chat",
        }).unwrap();
        const attached = await attachWorktree(response.worktree.meta);
        if (attached) {
          setCreateOpen(false);
          setMenuOpen(false);
        } else {
          await deleteWorktree({
            id: response.worktree.meta.id,
            delete_branch: true,
          }).unwrap();
          setCreateError(
            "Worktree attach failed; created worktree was deleted.",
          );
        }
      } catch (error) {
        setCreateError(worktreeErrorText(error));
      }
    },
    [attachWorktree, chatId, createWorktree, deleteWorktree],
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
      <Popover.Root open={menuOpen} onOpenChange={setMenuOpen}>
        <Popover.Trigger>
          <button
            type="button"
            data-testid="worktree-control-trigger"
            className={`${styles.trigger} ${
              currentWorktree ? styles.triggerActive : ""
            }`}
            title={currentWorktree ? fullLabel : "Main workspace"}
            aria-label={`Worktree scope: ${
              currentWorktree ? fullLabel : "Main workspace"
            }`}
          >
            <Flex align="center" gap="1" className={styles.triggerInner}>
              <Text size="1" className={styles.triggerText}>
                {label}
              </Text>
              {currentWorktree && (
                <WorktreeStatusBadge
                  worktree={currentWorktree}
                  record={currentRecord}
                />
              )}
            </Flex>
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
      </Popover.Root>

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
