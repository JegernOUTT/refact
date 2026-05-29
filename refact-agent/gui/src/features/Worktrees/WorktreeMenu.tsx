import React, { useCallback, useState, type ReactNode } from "react";
import {
  Button,
  Checkbox,
  Dialog,
  Flex,
  Popover,
  Separator,
  Text,
} from "@radix-ui/themes";
import {
  CopyIcon,
  DoubleArrowRightIcon,
  ExitIcon,
  FileTextIcon,
  OpenInNewWindowIcon,
  PlusIcon,
  TrashIcon,
} from "@radix-ui/react-icons";
import {
  useDeleteWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectApiKey, selectLspPort } from "../Config/configSlice";
import { selectChatId, setThreadWorktree } from "../Chat/Thread";
import { WorktreeStatusBadge } from "./WorktreeStatusBadge";
import { WorktreeDiffPanel } from "./WorktreeDiffPanel";
import { MergeWorktreeModal } from "./MergeWorktreeModal";
import { buildWorktreeConflictPrompt } from "./worktreeConflict";
import { worktreeErrorText } from "./worktreeError";
import styles from "./Worktrees.module.css";

type WorktreeMenuProps = {
  currentWorktree: WorktreeMeta | null;
  currentRecord?: WorktreeRecordView | null;
  records: WorktreeRecordView[];
  isLoading: boolean;
  feedback?: string | null;
  canCopyPath: boolean;
  onCreate: () => void;
  onSelect: (record: WorktreeRecordView) => void;
  onDetach: () => void;
  onOpenInNewWindow: () => void;
  onCopyPath: () => void;
};

type ActionButtonProps = {
  label: string;
  title: string;
  icon: ReactNode;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  primary?: boolean;
};

function ActionButton({
  label,
  title,
  icon,
  onClick,
  disabled = false,
  danger = false,
  primary = false,
}: ActionButtonProps) {
  const className = [
    styles.actionButton,
    primary ? styles.actionPrimary : "",
    danger && !disabled ? styles.actionDanger : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      type="button"
      className={className}
      onClick={onClick}
      disabled={disabled}
      aria-label={title}
      title={title}
    >
      <span className={styles.actionIcon} aria-hidden="true">
        {icon}
      </span>
      <Text
        size="1"
        weight={primary ? "medium" : "regular"}
        className={styles.actionLabel}
      >
        {label}
      </Text>
    </button>
  );
}

function compactPath(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return normalized || path;
  return parts.slice(-2).join("/");
}

function displayName(worktree: WorktreeMeta): string {
  const branch = worktree.branch?.trim();
  return branch !== undefined && branch.length > 0
    ? branch
    : compactPath(worktree.root);
}

function referencesLabel(record: WorktreeRecordView): string {
  if (record.reference_count === 0) return "unused";
  if (record.reference_count === 1) return "1 ref";
  return `${record.reference_count} refs`;
}

function referenceCount(
  worktree: WorktreeMeta | null,
  record?: WorktreeRecordView | null,
): number {
  return record?.reference_count ?? worktree?.reference_count ?? 0;
}

export const WorktreeMenu: React.FC<WorktreeMenuProps> = ({
  currentWorktree,
  currentRecord,
  records,
  isLoading,
  feedback,
  canCopyPath,
  onCreate,
  onSelect,
  onDetach,
  onOpenInNewWindow,
  onCopyPath,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const lspPort = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey) ?? undefined;
  const [diffOpen, setDiffOpen] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [localFeedback, setLocalFeedback] = useState<string | null>(null);
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const sharedCount = referenceCount(currentWorktree, currentRecord);
  const worktreeAvailable = Boolean(currentWorktree);
  const hasFeedback =
    (feedback?.length ?? 0) > 0 || (localFeedback?.length ?? 0) > 0;
  const detachLabel = currentWorktree ? "Detach" : "Main";
  const detachTitle = currentWorktree
    ? "Detach worktree and use main workspace"
    : "Already using main workspace";

  const handleAskRefact = useCallback(
    async (files: string[], response: MergeWorktreeResponse) => {
      if (!currentWorktree || !chatId || !lspPort) {
        throw new Error("No active worktree chat is available.");
      }
      const prompt = buildWorktreeConflictPrompt({
        worktree: currentWorktree,
        record: currentRecord,
        response,
        files,
      });
      await sendUserMessage(chatId, prompt, lspPort, apiKey, true);
      setLocalFeedback("Conflict resolution request sent to Refact.");
    },
    [apiKey, chatId, currentRecord, currentWorktree, lspPort],
  );

  const handleDelete = useCallback(async () => {
    if (!currentWorktree) return;
    setLocalFeedback(null);
    try {
      await deleteWorktree({
        id: currentWorktree.id,
        source_workspace_root: currentWorktree.source_workspace_root,
        delete_branch: deleteBranch,
        force_referenced: true,
      }).unwrap();
      setDeleteOpen(false);
      setLocalFeedback("Worktree deleted.");
      if (chatId && currentWorktree.id) {
        dispatch(setThreadWorktree({ chatId, worktree: null }));
        onDetach();
      }
    } catch (error) {
      setLocalFeedback(`Delete failed: ${worktreeErrorText(error)}`);
    }
  }, [
    chatId,
    currentWorktree,
    deleteBranch,
    deleteWorktree,
    dispatch,
    onDetach,
  ]);

  const handleMerged = useCallback(
    (response: MergeWorktreeResponse) => {
      const cleanup = response.cleanup;
      if (
        (cleanup?.worktree_deleted ?? false) ||
        (cleanup?.registry_deleted ?? false)
      ) {
        if (chatId) {
          dispatch(setThreadWorktree({ chatId, worktree: null }));
        }
        onDetach();
        setMergeOpen(false);
        setLocalFeedback("Worktree merged and removed.");
      } else if (response.merged === true) {
        setLocalFeedback("Worktree merged.");
      }
    },
    [chatId, dispatch, onDetach],
  );

  return (
    <>
      <Popover.Content
        className={styles.content}
        side="top"
        align="start"
        sideOffset={8}
      >
        <div className={styles.menu}>
          <Flex
            align="center"
            justify="between"
            className={styles.sectionHeader}
          >
            <Text size="2" weight="bold">
              Worktrees
            </Text>
            {currentWorktree && currentRecord && (
              <WorktreeStatusBadge
                worktree={currentWorktree}
                record={currentRecord}
              />
            )}
          </Flex>
          <Text size="1" color="gray" className={styles.menuHint}>
            Paths warn/remap; shell uses scoped cwd; shared refs affect all
            chats.
          </Text>

          {hasFeedback && (
            <Flex direction="column" gap="1" className={styles.feedback}>
              {feedback && (
                <Text size="1" color="gray">
                  {feedback}
                </Text>
              )}
              {localFeedback && (
                <Text size="1" color="gray">
                  {localFeedback}
                </Text>
              )}
            </Flex>
          )}

          <div className={styles.actionGrid}>
            <ActionButton
              label="Create"
              title="Create worktree"
              icon={<PlusIcon />}
              onClick={onCreate}
              primary
            />
            <ActionButton
              label={detachLabel}
              title={detachTitle}
              icon={<ExitIcon />}
              onClick={onDetach}
              disabled={!currentWorktree}
            />
            <ActionButton
              label="Open"
              title="Open worktree in new window"
              icon={<OpenInNewWindowIcon />}
              onClick={onOpenInNewWindow}
              disabled={!currentWorktree}
            />
            <ActionButton
              label="Copy"
              title="Copy workspace path"
              icon={<CopyIcon />}
              onClick={onCopyPath}
              disabled={!canCopyPath}
            />
          </div>

          <Separator size="4" />

          <div className={styles.section}>
            <Text size="1" color="gray" className={styles.sectionHeader}>
              Existing
            </Text>
            <div className={styles.list}>
              {isLoading && (
                <Text size="1" color="gray" className={styles.sectionHeader}>
                  Loading...
                </Text>
              )}
              {!isLoading && records.length === 0 && (
                <Text size="1" color="gray" className={styles.sectionHeader}>
                  None yet
                </Text>
              )}
              {records.map((record) => {
                const selected = currentWorktree?.id === record.meta.id;
                const title = displayName(record.meta);
                const usedBy = record.referencing_chat_ids?.length
                  ? record.referencing_chat_ids.join(", ")
                  : record.references
                      .map((reference) => reference.chat_id)
                      .filter((value): value is string => Boolean(value))
                      .join(", ");
                return (
                  <button
                    key={record.meta.id}
                    type="button"
                    className={`${styles.item} ${
                      selected ? styles.itemSelected : ""
                    }`}
                    onClick={() => onSelect(record)}
                    aria-label={`Select worktree ${title}`}
                    aria-current={selected ? "true" : undefined}
                    title={`Use ${title}`}
                  >
                    <Flex
                      direction="column"
                      gap="1"
                      className={styles.itemTitle}
                    >
                      <Flex align="center" gap="2" wrap="wrap">
                        <Text size="1" weight="medium">
                          {title}
                        </Text>
                        <WorktreeStatusBadge
                          worktree={record.meta}
                          record={record}
                        />
                      </Flex>
                      <Text size="1" color="gray" className={styles.path}>
                        {record.meta.root}
                      </Text>
                      <Text size="1" color="gray">
                        {referencesLabel(record)}
                        {usedBy ? ` · used by ${usedBy}` : ""}
                      </Text>
                    </Flex>
                  </button>
                );
              })}
            </div>
          </div>

          <Separator size="4" />

          <div className={styles.reviewActions}>
            <ActionButton
              label="Diff"
              title="View worktree diff"
              icon={<FileTextIcon />}
              onClick={() => setDiffOpen(true)}
              disabled={!worktreeAvailable}
            />
            <ActionButton
              label="Merge"
              title="Merge worktree"
              icon={<DoubleArrowRightIcon />}
              onClick={() => setMergeOpen(true)}
              disabled={!worktreeAvailable}
            />
            <ActionButton
              label="Delete"
              title="Delete or discard worktree"
              icon={<TrashIcon />}
              onClick={() => setDeleteOpen(true)}
              disabled={!worktreeAvailable}
              danger={worktreeAvailable}
            />
          </div>

          {sharedCount > 1 ? (
            <Text size="1" color="gray" className={styles.feedback}>
              Shared by {sharedCount} references. Delete and discard actions can
              affect other chats.
            </Text>
          ) : null}
        </div>
      </Popover.Content>

      <WorktreeDiffPanel
        open={diffOpen}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setDiffOpen}
      />

      <MergeWorktreeModal
        open={mergeOpen}
        worktreeId={currentWorktree?.id}
        worktree={currentWorktree}
        record={currentRecord}
        onOpenChange={setMergeOpen}
        onMerged={handleMerged}
        onAskRefact={handleAskRefact}
        onOpenWorktree={onOpenInNewWindow}
      />

      <Dialog.Root open={deleteOpen} onOpenChange={setDeleteOpen}>
        <Dialog.Content maxWidth="420px">
          <Dialog.Title>Delete worktree</Dialog.Title>
          <Dialog.Description size="2" color="gray">
            Delete or discard the selected worktree from disk.
          </Dialog.Description>

          <Flex direction="column" gap="3" mt="3">
            <div className={styles.dialogOverlayText}>
              <Text size="2" weight="medium">
                {currentWorktree ? displayName(currentWorktree) : "No worktree"}
              </Text>
              {currentWorktree && (
                <Text size="1" color="gray" className={styles.path}>
                  {currentWorktree.root}
                </Text>
              )}
            </div>

            {sharedCount > 1 && (
              <Text size="2" color="amber" className={styles.warningBox}>
                This worktree is shared by {sharedCount} references. Deleting it
                may affect other chats that use the same worktree.
              </Text>
            )}

            <Text as="label" size="2">
              <Flex align="center" gap="2">
                <Checkbox
                  checked={deleteBranch}
                  onCheckedChange={(checked) =>
                    setDeleteBranch(checked === true)
                  }
                  disabled={deleteState.isLoading}
                />
                Delete git branch too
              </Flex>
            </Text>

            {localFeedback && localFeedback.startsWith("Delete failed") && (
              <Text size="2" color="red" className={styles.warningBox}>
                {localFeedback}
              </Text>
            )}
          </Flex>

          <Flex className={styles.modalActions}>
            <Dialog.Close>
              <Button
                type="button"
                variant="soft"
                color="gray"
                disabled={deleteState.isLoading}
              >
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              type="button"
              color="red"
              onClick={() => void handleDelete()}
              disabled={!currentWorktree || deleteState.isLoading}
            >
              {deleteState.isLoading ? "Deleting..." : "Delete worktree"}
            </Button>
          </Flex>
        </Dialog.Content>
      </Dialog.Root>
    </>
  );
};

WorktreeMenu.displayName = "WorktreeMenu";
