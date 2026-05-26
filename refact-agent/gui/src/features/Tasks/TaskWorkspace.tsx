import React, { useCallback, useState, useEffect, useMemo } from "react";
import {
  Flex,
  Box,
  Text,
  Button,
  Heading,
  Badge,
  Card,
  Dialog,
  Checkbox,
  Tooltip,
  Tabs,
} from "@radix-ui/themes";
import {
  PlusIcon,
  Cross2Icon,
  ChevronDownIcon,
  FileTextIcon,
} from "@radix-ui/react-icons";
import { AgentStatusDot } from "./AgentStatusDot";
import { ScrollArea } from "../../components/ScrollArea";
import { ChatLoading } from "../../components/ChatContent/ChatLoading";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop } from "../Pages/pagesSlice";
import { KanbanBoard } from "./KanbanBoard";
import {
  useGetTaskQuery,
  useGetBoardQuery,
  useListTaskTrajectoriesQuery,
  useUpdateTaskMetaMutation,
  useCreatePlannerChatMutation,
  useDeletePlannerChatMutation,
  BoardCard,
  tasksApi,
} from "../../services/refact/tasks";
import { ModelPickerPopover } from "../../components/ChatForm/ModelPickerPopover";
import { Markdown } from "../../components/Markdown";
import { CollapsePanel } from "../../components/shared/CollapsePanel";
import { ResizeDivider } from "../Dashboard/components/ResizeDivider/ResizeDivider";
import styles from "./Tasks.module.css";
import { Chat } from "../Chat";
import { selectConfig } from "../Config/configSlice";
import {
  createChatWithId,
  setThreadWorktree,
  switchToThread,
} from "../Chat/Thread";
import {
  openTask,
  addPlannerChat,
  removePlannerChat,
  restorePlannerChat,
  selectOpenTasksFromRoot,
  setTaskActiveChat,
  selectTaskActiveChat,
  PlannerInfo,
} from "./tasksSlice";
import { selectThreadById, selectRuntimeById } from "../Chat/Thread";
import { InternalLinkProvider } from "../../contexts/InternalLinkContext";
import { parseRefactLink } from "../../contexts/internalLinkUtils";
import {
  useDeleteWorktreeMutation,
  useListWorktreesQuery,
  useOpenWorktreeMutation,
  type MergeWorktreeResponse,
  type WorktreeMeta,
  type WorktreeRecordView,
} from "../../services/refact";
import {
  sendUserMessage,
  updateChatParams,
} from "../../services/refact/chatCommands";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import {
  BranchIcon,
  WorktreeDiffPanel,
  MergeWorktreeModal,
  WorktreeStatusBadge,
  buildWorktreeConflictPrompt,
  worktreeErrorText,
} from "../Worktrees";
import {
  loadTaskWorkspaceLayout,
  saveTaskWorkspaceLayout,
} from "../../utils/chatUiPersistence";
import { MemoryInboxPanel } from "./TaskMemories/MemoryInboxPanel";
import { DocumentsPanel } from "./TaskDocuments/DocumentsPanel";
import { CardCommentsSection } from "./CardComments";

type ActiveChat =
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string }
  | null;

export type CardWorktreeTarget = {
  id: string;
  label: string;
  record?: WorktreeRecordView;
  meta?: WorktreeMeta | null;
  legacy: boolean;
  stale: boolean;
  referenceCount?: number;
};

const LEGACY_WORKTREE_TOOLTIP =
  "This worktree was created before the registry; recreate it via `restart_agent(mode=fresh)` to enable actions.";

function compactPath(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return normalized || path;
  return parts.slice(-2).join("/");
}

function worktreeLabel(
  card: BoardCard,
  record?: WorktreeRecordView,
  meta?: WorktreeMeta | null,
): string | null {
  return (
    card.agent_worktree_name ??
    card.agent_branch ??
    record?.meta.branch ??
    meta?.branch ??
    record?.meta.root ??
    meta?.root ??
    card.agent_worktree ??
    null
  );
}

function formatWorktreeTargetLabel(label: string): string {
  return label.includes("/") || label.includes("\\")
    ? compactPath(label)
    : label;
}

function makeLegacyTarget(
  card: BoardCard,
  threadWorktree?: WorktreeMeta | null,
): CardWorktreeTarget | null {
  const label = worktreeLabel(card, undefined, threadWorktree);
  if (!label) return null;
  return {
    id: "",
    label: formatWorktreeTargetLabel(label),
    meta: threadWorktree ?? null,
    legacy: true,
    stale: threadWorktree?.deleted === true || threadWorktree?.stale === true,
    referenceCount: threadWorktree?.reference_count,
  };
}

function isActionableWorktree(worktree: CardWorktreeTarget): boolean {
  return !worktree.legacy && !worktree.stale && worktree.id.trim().length > 0;
}

export function resolveCardWorktree(
  taskId: string,
  card: BoardCard,
  records: WorktreeRecordView[],
  threadWorktree?: WorktreeMeta | null,
): CardWorktreeTarget | null {
  const byName = card.agent_worktree_name
    ? records.find((record) => record.meta.id === card.agent_worktree_name)
    : undefined;
  const byThread = threadWorktree
    ? records.find((record) => record.meta.id === threadWorktree.id)
    : undefined;
  const byCard = records.find(
    (record) =>
      record.meta.task_id === taskId && record.meta.card_id === card.id,
  );
  const byBranch = card.agent_branch
    ? records.find(
        (record) =>
          record.meta.branch === card.agent_branch &&
          (!record.meta.task_id || record.meta.task_id === taskId),
      )
    : undefined;
  const record = byName ?? byThread ?? byCard ?? byBranch;
  const meta = record?.meta ?? threadWorktree ?? null;
  const id = record?.meta.id ?? threadWorktree?.id ?? card.agent_worktree_name;
  if (!id) {
    if (card.agent_worktree || card.agent_branch) {
      return makeLegacyTarget(card, threadWorktree);
    }
    return null;
  }
  const label = worktreeLabel(card, record, meta);
  if (!label) return null;
  return {
    id,
    label: formatWorktreeTargetLabel(label),
    record,
    meta,
    legacy: false,
    stale:
      record?.status.path_exists === false ||
      record?.meta.lifecycle_state === "deleted" ||
      meta?.deleted === true ||
      meta?.stale === true,
    referenceCount: record?.reference_count ?? meta?.reference_count,
  };
}

interface PlannerPanelProps {
  plannerChats: PlannerInfo[];
  activeChat: ActiveChat;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
}

interface PlannerItemProps {
  planner: PlannerInfo;
  isSelected: boolean;
  onSelect: () => void;
  onRemove: () => void;
}

function formatPlannerDate(dateStr: string): string {
  if (!dateStr) return "";
  try {
    const date = new Date(dateStr);
    return date.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

function formatAgentChatTitle(
  cardId: string | undefined,
  cardTitle: string,
): string {
  return cardId ? `Agent: ${cardId} ${cardTitle}` : `Agent: ${cardTitle}`;
}

type AgentRef = { chat_id: string };

function parsePlannerDeleteError(err: unknown): string {
  if (typeof err === "object" && err && "status" in err) {
    const e = err as {
      status: number;
      data?: { error?: string; agent_refs?: AgentRef[] };
    };
    if (e.status === 409 && e.data?.agent_refs) {
      const ids = e.data.agent_refs
        .map((r) => r.chat_id)
        .slice(0, 3)
        .join(", ");
      const extra =
        e.data.agent_refs.length > 3
          ? ` (+${e.data.agent_refs.length - 3} more)`
          : "";
      return `${e.data.error ?? "Conflict"}: ${ids}${extra}`;
    }
    if (e.data?.error) return e.data.error;
  }
  return "Unknown error";
}

const DEFAULT_BOARD_HEIGHT_PX = 180;
const MIN_BOARD_HEIGHT_PX = 80;
const MAX_BOARD_HEIGHT_RATIO = 0.6;

function clampBoardHeight(value: number, containerHeight?: number): number {
  const maxHeight =
    containerHeight && Number.isFinite(containerHeight) && containerHeight > 0
      ? Math.max(MIN_BOARD_HEIGHT_PX, containerHeight * MAX_BOARD_HEIGHT_RATIO)
      : 480;
  return Math.max(MIN_BOARD_HEIGHT_PX, Math.min(maxHeight, value));
}

function defaultTaskWorkspaceLayout() {
  return {
    chatExpanded: false,
    panelsExpanded: false,
    boardHeightPx: DEFAULT_BOARD_HEIGHT_PX,
  };
}

export const PlannerItem: React.FC<PlannerItemProps> = ({
  planner,
  isSelected,
  onSelect,
  onRemove,
}) => {
  const thread = useAppSelector((state) => selectThreadById(state, planner.id));
  const runtime = useAppSelector((state) =>
    selectRuntimeById(state, planner.id),
  );
  const title = thread?.title ?? planner.title;
  const hasGeneratedTitle =
    title && title !== "New Chat" && title.trim() !== "";
  const displayTitle = hasGeneratedTitle
    ? title
    : formatPlannerDate(planner.createdAt);

  const sessionState = runtime?.session_state;
  const isWaiting = sessionState === "waiting_user_input";
  const waitingCards = planner.waitingForCardIds ?? [];
  const showWaitingChips = isWaiting && waitingCards.length > 0;
  const visibleCards = waitingCards.slice(0, 5);
  const hiddenCount = Math.max(0, waitingCards.length - 5);

  return (
    <Box
      className={`${styles.panelItem} ${
        isSelected ? styles.panelItemSelected : ""
      }`}
      onClick={onSelect}
    >
      <Flex align="center" gap="1" className={styles.panelItemLead}>
        <Badge size="1" color="violet">
          <FileTextIcon />
        </Badge>
      </Flex>
      <Box className={styles.panelItemContent}>
        <Text size="1" className={styles.panelItemTitle}>
          {displayTitle}
        </Text>
      </Box>
      {showWaitingChips && (
        <Flex
          gap="1"
          wrap="nowrap"
          align="center"
          className={styles.plannerWaitingChips}
          data-testid={`planner-waiting-chips-${planner.id}`}
        >
          {visibleCards.map((cardId) => (
            <Badge
              key={cardId}
              size="1"
              color="amber"
              variant="soft"
              title={`Waiting for ${cardId}`}
            >
              {cardId}
            </Badge>
          ))}
          {hiddenCount > 0 && (
            <Text size="1" color="gray" className={styles.plannerWaitingMore}>
              … and {hiddenCount} more
            </Text>
          )}
        </Flex>
      )}
      <Tooltip content="Delete planner chat">
        <Button
          size="1"
          variant="ghost"
          color="gray"
          aria-label="Delete planner chat"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <Cross2Icon />
        </Button>
      </Tooltip>
    </Box>
  );
};

const PlannerPanel: React.FC<PlannerPanelProps> = ({
  plannerChats,
  activeChat,
  onSelectPlanner,
  onRemovePlanner,
}) => {
  return (
    <Box className={styles.panelList}>
      <Box className={styles.panelContent}>
        {plannerChats.length === 0 ? (
          <Flex align="center" justify="center" style={{ flex: 1 }}>
            <Text size="1" color="gray">
              No planner chats yet
            </Text>
          </Flex>
        ) : (
          <ScrollArea scrollbars="vertical">
            <Flex direction="column" gap="1">
              {plannerChats.map((planner) => (
                <PlannerItem
                  key={planner.id}
                  planner={planner}
                  isSelected={
                    activeChat?.type === "planner" &&
                    activeChat.chatId === planner.id
                  }
                  onSelect={() => onSelectPlanner(planner.id)}
                  onRemove={() => onRemovePlanner(planner.id)}
                />
              ))}
            </Flex>
          </ScrollArea>
        )}
      </Box>
    </Box>
  );
};

interface AgentsPanelProps {
  cards: BoardCard[];
  activeChat: ActiveChat;
  onSelectAgent: (cardId: string, chatId: string) => void;
  defaultAgentModel?: string;
  onModelChange?: (model: string) => void;
}

const AgentsPanel: React.FC<AgentsPanelProps> = ({
  cards,
  activeChat,
  onSelectAgent,
  defaultAgentModel,
  onModelChange,
}) => {
  const activeAgents = cards.filter(
    (c) => c.column === "doing" && c.agent_chat_id,
  );
  const completedAgents = cards.filter(
    (c) => c.column === "done" && c.agent_chat_id,
  );
  const failedAgents = cards.filter(
    (c) => c.column === "failed" && c.agent_chat_id,
  );

  const renderAgentItem = (
    card: BoardCard,
    status: "doing" | "done" | "failed",
  ) => {
    const isActive =
      activeChat?.type === "agent" && activeChat.cardId === card.id;
    return (
      <Box
        key={card.id}
        className={`${styles.panelItem} ${
          isActive ? styles.panelItemSelected : ""
        }`}
        onClick={() =>
          card.agent_chat_id && onSelectAgent(card.id, card.agent_chat_id)
        }
      >
        <div className={styles.panelItemLead}>
          <AgentStatusDot status={status} size="medium" />
        </div>
        <Flex align="center" gap="1" className={styles.panelItemContent}>
          <Badge size="1" color="gray" variant="soft">
            {card.id}
          </Badge>
          <Text size="1" className={styles.panelItemTitle}>
            {card.title}
          </Text>
        </Flex>
      </Box>
    );
  };

  return (
    <Box className={styles.panelList}>
      <Box className={styles.panelContent}>
        {activeAgents.length === 0 &&
        completedAgents.length === 0 &&
        failedAgents.length === 0 ? (
          <Flex align="center" justify="center" style={{ flex: 1 }}>
            <Text size="1" color="gray">
              No agents yet
            </Text>
          </Flex>
        ) : (
          <ScrollArea scrollbars="vertical">
            <Flex direction="column" gap="1">
              {activeAgents.map((card) => renderAgentItem(card, "doing"))}
              {completedAgents.map((card) => renderAgentItem(card, "done"))}
              {failedAgents.map((card) => renderAgentItem(card, "failed"))}
            </Flex>
          </ScrollArea>
        )}
      </Box>
      {onModelChange && (
        <Flex className={styles.modelPickerRow}>
          <ModelPickerPopover
            value={defaultAgentModel ?? ""}
            onValueChange={onModelChange}
          />
        </Flex>
      )}
    </Box>
  );
};

interface CardDetailProps {
  taskId: string;
  card: BoardCard;
  worktree: CardWorktreeTarget | null;
  worktreeLabel: string | null;
  isWorktreeLoading: boolean;
  onClose: () => void;
  onInternalLink?: (url: string) => boolean;
  onViewDiff: (worktree: CardWorktreeTarget) => void;
  onMerge: (worktree: CardWorktreeTarget) => void;
  onOpenWorktree: (worktree: CardWorktreeTarget) => void;
  onDeleteWorktree: (worktree: CardWorktreeTarget) => void;
}

const CardDetail: React.FC<CardDetailProps> = ({
  taskId,
  card,
  worktree,
  worktreeLabel,
  isWorktreeLoading,
  onClose,
  onInternalLink,
  onViewDiff,
  onMerge,
  onOpenWorktree,
  onDeleteWorktree,
}) => {
  const worktreeActionsDisabled = !worktree || !isActionableWorktree(worktree);
  const worktreeActionsTooltip = worktree?.legacy
    ? LEGACY_WORKTREE_TOOLTIP
    : worktree?.stale
      ? "This worktree appears stale, missing, or deleted."
      : undefined;
  const invokeWorktreeAction = (
    action: (target: CardWorktreeTarget) => void,
  ) => {
    if (!worktree || worktreeActionsDisabled) return;
    action(worktree);
  };
  const wrapWorktreeAction = (button: React.ReactNode) =>
    worktreeActionsTooltip ? (
      <Tooltip content={worktreeActionsTooltip}>
        <span>{button}</span>
      </Tooltip>
    ) : (
      button
    );

  return (
    <Box className={styles.cardDetailOverlay} onClick={onClose}>
      <Card className={styles.cardDetail} onClick={(e) => e.stopPropagation()}>
        <Flex direction="column" gap="3">
          <Flex justify="between" align="center">
            <Heading size="3" className={styles.cardDetailTitle}>
              <Badge size="1" color="gray" variant="soft" mr="2">
                {card.id}
              </Badge>
              {card.title}
            </Heading>
            <Badge
              color={
                card.column === "done"
                  ? "green"
                  : card.column === "failed"
                    ? "red"
                    : "blue"
              }
            >
              {card.column}
            </Badge>
          </Flex>

          {card.depends_on.length > 0 && (
            <Box>
              <Text size="2" weight="medium" color="gray">
                Dependencies
              </Text>
              <Flex gap="1" mt="1">
                {card.depends_on.map((dep) => (
                  <Badge key={dep} size="1" variant="soft">
                    {dep}
                  </Badge>
                ))}
              </Flex>
            </Box>
          )}

          {worktreeLabel && (
            <Box>
              <Text size="2" weight="medium" color="gray">
                Worktree
              </Text>
              <Flex direction="column" gap="2" mt="1">
                <Flex gap="2" align="center" wrap="wrap">
                  <Badge size="1" color="green" variant="soft">
                    <BranchIcon /> {worktreeLabel}
                  </Badge>
                  {worktree?.record ?? worktree?.meta ? (
                    <WorktreeStatusBadge
                      worktree={worktree.meta ?? worktree.record?.meta}
                      record={worktree.record}
                    />
                  ) : null}
                  {worktree?.referenceCount && worktree.referenceCount > 1 ? (
                    <Badge size="1" color="amber" variant="soft">
                      shared by {worktree.referenceCount}
                    </Badge>
                  ) : null}
                </Flex>
                {isWorktreeLoading && (
                  <Text size="1" color="gray">
                    Loading worktree metadata...
                  </Text>
                )}
                {!isWorktreeLoading && !worktree && (
                  <Text size="1" color="gray">
                    Worktree metadata is unavailable or stale.
                  </Text>
                )}
                {worktree?.stale && (
                  <Text size="1" color="amber">
                    This worktree appears stale, missing, or deleted.
                  </Text>
                )}
                {worktree?.legacy && (
                  <Text size="1" color="amber">
                    Legacy / unregistered worktree
                  </Text>
                )}
                <Flex gap="2" wrap="wrap">
                  {wrapWorktreeAction(
                    <Button
                      type="button"
                      size="1"
                      variant="soft"
                      disabled={worktreeActionsDisabled}
                      title={worktreeActionsTooltip}
                      onClick={() => invokeWorktreeAction(onViewDiff)}
                    >
                      View Diff
                    </Button>,
                  )}
                  {wrapWorktreeAction(
                    <Button
                      type="button"
                      size="1"
                      variant="soft"
                      disabled={worktreeActionsDisabled}
                      title={worktreeActionsTooltip}
                      onClick={() => invokeWorktreeAction(onMerge)}
                    >
                      Merge
                    </Button>,
                  )}
                  {wrapWorktreeAction(
                    <Button
                      type="button"
                      size="1"
                      variant="soft"
                      color="gray"
                      disabled={worktreeActionsDisabled}
                      title={worktreeActionsTooltip}
                      onClick={() => invokeWorktreeAction(onOpenWorktree)}
                    >
                      Open
                    </Button>,
                  )}
                  {wrapWorktreeAction(
                    <Button
                      type="button"
                      size="1"
                      variant="soft"
                      color="red"
                      disabled={worktreeActionsDisabled}
                      title={worktreeActionsTooltip}
                      onClick={() => invokeWorktreeAction(onDeleteWorktree)}
                    >
                      Discard/Delete
                    </Button>,
                  )}
                </Flex>
              </Flex>
            </Box>
          )}

          {card.instructions && (
            <Box>
              <Text size="2" weight="medium" color="gray">
                Instructions
              </Text>
              <Box className={styles.cardDetailSection}>
                {onInternalLink ? (
                  <InternalLinkProvider
                    onInternalLink={(url) => {
                      onClose();
                      return onInternalLink(url);
                    }}
                  >
                    <Markdown canHaveInteractiveElements={false}>
                      {card.instructions}
                    </Markdown>
                  </InternalLinkProvider>
                ) : (
                  <Markdown canHaveInteractiveElements={false}>
                    {card.instructions}
                  </Markdown>
                )}
              </Box>
            </Box>
          )}

          {card.final_report && (
            <Box>
              <Text size="2" weight="medium" color="gray">
                Final Report
              </Text>
              <Box
                className={styles.cardDetailSection}
                style={{ background: "var(--green-2)" }}
              >
                <Markdown canHaveInteractiveElements={false}>
                  {card.final_report}
                </Markdown>
              </Box>
            </Box>
          )}

          {card.status_updates.length > 0 && (
            <Box>
              <Text size="2" weight="medium" color="gray">
                Updates
              </Text>
              <Flex direction="column" gap="1" mt="1">
                {card.status_updates.map((update, i) => (
                  <Text key={i} size="1" color="gray">
                    {new Date(update.timestamp).toLocaleString()}:{" "}
                    {update.message}
                  </Text>
                ))}
              </Flex>
            </Box>
          )}

          <CardCommentsSection
            taskId={taskId}
            cardId={card.id}
            comments={card.comments ?? []}
          />

          <Flex justify="end">
            <Button variant="soft" onClick={onClose}>
              Close
            </Button>
          </Flex>
        </Flex>
      </Card>
    </Box>
  );
};

interface TaskWorkspaceProps {
  taskId: string;
}

export const TaskWorkspace: React.FC<TaskWorkspaceProps> = ({ taskId }) => {
  const dispatch = useAppDispatch();
  const taskWorkspaceRef = React.useRef<HTMLDivElement>(null);
  const config = useAppSelector(selectConfig);
  const { data: task, isLoading: taskLoading } = useGetTaskQuery(taskId, {
    pollingInterval: 0,
  });
  const { data: board, isLoading: boardLoading } = useGetBoardQuery(taskId, {
    pollingInterval: 0,
  });
  const { data: worktreesData, isLoading: worktreesLoading } =
    useListWorktreesQuery(undefined);
  const [openWorktree] = useOpenWorktreeMutation();
  const [deleteWorktree, deleteWorktreeState] = useDeleteWorktreeMutation();
  const copyToClipboard = useCopyToClipboard();
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const { data: savedPlanners } = useListTaskTrajectoriesQuery({
    taskId,
    role: "planner",
  });
  const [updateTaskMeta] = useUpdateTaskMetaMutation();
  const [createPlannerChat, { isLoading: isCreatingPlanner }] =
    useCreatePlannerChatMutation();
  const [deletePlannerChat] = useDeletePlannerChatMutation();
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const currentTaskUI = openTasks.find((t) => t.id === taskId);
  const plannerChats = useMemo(
    () =>
      [...(currentTaskUI?.plannerChats ?? [])].sort((a, b) =>
        b.updatedAt.localeCompare(a.updatedAt),
      ),
    [currentTaskUI?.plannerChats],
  );
  const activeChat = useAppSelector((state) =>
    selectTaskActiveChat(state, taskId),
  );
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const selectedCard = useMemo(
    () =>
      selectedCardId
        ? board?.cards.find((c) => c.id === selectedCardId) ?? null
        : null,
    [board, selectedCardId],
  );
  const [diffTarget, setDiffTarget] = useState<CardWorktreeTarget | null>(null);
  const [mergeTargetId, setMergeTargetId] = useState<string | null>(null);
  const [mergeTargetWorktree, setMergeTargetWorktree] =
    useState<CardWorktreeTarget | null>(null);
  const mergeTarget = useMemo(() => {
    if (!mergeTargetId || !mergeTargetWorktree) return null;
    const card = board?.cards.find((c) => c.id === mergeTargetId) ?? null;
    if (!card) return null;
    return { card, worktree: mergeTargetWorktree };
  }, [board, mergeTargetId, mergeTargetWorktree]);
  const [deleteTargetId, setDeleteTargetId] = useState<string | null>(null);
  const [deleteTargetWorktree, setDeleteTargetWorktree] =
    useState<CardWorktreeTarget | null>(null);
  const deleteTarget = useMemo(() => {
    if (!deleteTargetId || !deleteTargetWorktree) return null;
    const card = board?.cards.find((c) => c.id === deleteTargetId) ?? null;
    if (!card) return null;
    return { card, worktree: deleteTargetWorktree };
  }, [board, deleteTargetId, deleteTargetWorktree]);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [notification, setNotification] = useState<string | null>(null);
  const [layout, setLayout] = useState(() =>
    loadTaskWorkspaceLayout(taskId, defaultTaskWorkspaceLayout()),
  );
  const [workspaceTab, setWorkspaceTab] = useState("chat");
  const prevTaskStatusRef = React.useRef<string | undefined>(undefined);
  const chatExpanded = layout.chatExpanded;
  const panelsExpanded = layout.panelsExpanded;
  const boardHeightPx = layout.boardHeightPx;
  const worktreeRecords = useMemo(
    () => worktreesData?.worktrees ?? [],
    [worktreesData?.worktrees],
  );
  const selectedCardThread = useAppSelector((state) =>
    selectedCard?.agent_chat_id
      ? selectThreadById(state, selectedCard.agent_chat_id)
      : null,
  );
  const selectedCardWorktree = useMemo(
    () =>
      selectedCard
        ? resolveCardWorktree(
            taskId,
            selectedCard,
            worktreeRecords,
            selectedCardThread?.worktree,
          )
        : null,
    [selectedCard, selectedCardThread?.worktree, taskId, worktreeRecords],
  );
  const selectedCardWorktreeLabel = selectedCard
    ? selectedCardWorktree?.label ??
      worktreeLabel(selectedCard, undefined, selectedCardThread?.worktree)
    : null;

  useEffect(() => {
    if (task) {
      dispatch(openTask({ id: taskId, name: task.name }));
    }
  }, [dispatch, taskId, task]);

  // Restore saved planner trajectories into Redux once the OpenTask entry
  // exists. This effect is intentionally idempotent: the per-planner dedup
  // check below guards against duplicate dispatches, so it can safely re-run
  // when `savedPlanners` or `currentTaskUI` updates. We must wait for
  // `currentTaskUI` (created by `openTask` after `task` loads) — without it,
  // `addPlannerChat`/`setTaskActiveChat` reducers silently no-op and the
  // restore is permanently lost (race condition: savedPlanners can arrive
  // before task).
  useEffect(() => {
    if (!savedPlanners || !currentTaskUI) return;

    for (const traj of savedPlanners) {
      dispatch(
        createChatWithId({
          id: traj.id,
          title: traj.title,
          isTaskChat: true,
          mode: "TASK_PLANNER",
          taskMeta: {
            task_id: taskId,
            role: "planner",
            planner_chat_id: traj.id,
          },
        }),
      );

      if (currentTaskUI.plannerChats.some((p) => p.id === traj.id)) continue;

      dispatch(
        addPlannerChat({
          taskId,
          planner: {
            id: traj.id,
            title: traj.title,
            createdAt: traj.created_at,
            updatedAt: traj.updated_at,
            sessionState: traj.session_state,
            waitingForCardIds: traj.waiting_for_card_ids,
          },
        }),
      );
    }

    if (savedPlanners.length > 0 && !activeChat) {
      const mostRecent = savedPlanners.reduce((latest, p) =>
        p.updated_at > latest.updated_at ? p : latest,
      );
      dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: { type: "planner", chatId: mostRecent.id },
        }),
      );
    }
  }, [dispatch, taskId, savedPlanners, currentTaskUI, activeChat]);

  useEffect(() => {
    const fallbackPlannerId = plannerChats[0]?.id;
    if (!activeChat && fallbackPlannerId) {
      dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: { type: "planner", chatId: fallbackPlannerId },
        }),
      );
      return;
    }

    if (
      activeChat?.type === "planner" &&
      !plannerChats.some((p) => p.id === activeChat.chatId)
    ) {
      dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: fallbackPlannerId
            ? { type: "planner", chatId: fallbackPlannerId }
            : null,
        }),
      );
    }
  }, [activeChat, plannerChats, dispatch, taskId]);

  useEffect(() => {
    if (activeChat?.type === "agent" && board) {
      const card = board.cards.find((c) => c.id === activeChat.cardId);
      if (!card || card.agent_chat_id !== activeChat.chatId) {
        const fallbackPlannerId = plannerChats[0]?.id;
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: fallbackPlannerId
              ? { type: "planner", chatId: fallbackPlannerId }
              : null,
          }),
        );
      }
    }
  }, [activeChat, board, dispatch, taskId, plannerChats]);

  useEffect(() => {
    if (!task) return;

    const prevStatus = prevTaskStatusRef.current;
    const currentStatus = task.status;

    prevTaskStatusRef.current = currentStatus;

    if (prevStatus === "planning" && currentStatus === "active") {
      setNotification("Planning complete! You can now spawn agents.");
      setTimeout(() => setNotification(null), 3000);
    }
  }, [task]);

  // Switch chat when activeChat changes
  useEffect(() => {
    if (!activeChat) return;
    const chatId = activeChat.chatId;
    dispatch(switchToThread({ id: chatId, openTab: false }));
  }, [dispatch, activeChat]);

  const handleBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleCardClick = useCallback((card: BoardCard) => {
    setSelectedCardId(card.id);
  }, []);

  const showNotification = useCallback((message: string) => {
    setNotification(message);
    window.setTimeout(() => setNotification(null), 3000);
  }, []);

  const handleNewPlanner = useCallback(() => {
    if (isCreatingPlanner) return;
    createPlannerChat(taskId)
      .unwrap()
      .then((result) => {
        const newChatId = result.chat_id;
        const now = new Date().toISOString();
        dispatch(
          createChatWithId({
            id: newChatId,
            title: "",
            isTaskChat: true,
            mode: "TASK_PLANNER",
            taskMeta: {
              task_id: taskId,
              role: "planner",
              planner_chat_id: newChatId,
            },
          }),
        );
        dispatch(
          addPlannerChat({
            taskId,
            planner: {
              id: newChatId,
              title: "",
              createdAt: now,
              updatedAt: now,
            },
          }),
        );
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "planner", chatId: newChatId },
          }),
        );
      })
      .catch((err: unknown) => {
        showNotification(`Create failed: ${parsePlannerDeleteError(err)}`);
      });
  }, [
    dispatch,
    taskId,
    createPlannerChat,
    isCreatingPlanner,
    showNotification,
  ]);

  const handleRemovePlanner = useCallback(
    (chatId: string) => {
      const previous = plannerChats.find((p) => p.id === chatId);
      dispatch(removePlannerChat({ taskId, chatId }));
      if (activeChat?.type === "planner" && activeChat.chatId === chatId) {
        const remaining = plannerChats.filter((p) => p.id !== chatId);
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: remaining[0]
              ? { type: "planner", chatId: remaining[0].id }
              : null,
          }),
        );
      }
      void deletePlannerChat({ taskId, chatId })
        .unwrap()
        .then(() => {
          dispatch(
            tasksApi.util.invalidateTags([
              { type: "TaskTrajectories", id: taskId },
            ]),
          );
          showNotification("Planner chat deleted.");
        })
        .catch((err: unknown) => {
          if (previous)
            dispatch(restorePlannerChat({ taskId, planner: previous }));
          showNotification(`Delete failed: ${parsePlannerDeleteError(err)}`);
        });
    },
    [
      dispatch,
      taskId,
      activeChat,
      plannerChats,
      deletePlannerChat,
      showNotification,
    ],
  );

  const handleSelectPlanner = useCallback(
    (chatId: string) => {
      dispatch(
        setTaskActiveChat({ taskId, activeChat: { type: "planner", chatId } }),
      );
    },
    [dispatch, taskId],
  );

  const handleSelectAgent = useCallback(
    (cardId: string, chatId: string) => {
      const card = board?.cards.find((c) => c.id === cardId);
      const cardTitle = card?.title ?? `Card ${cardId}`;

      dispatch(
        createChatWithId({
          id: chatId,
          title: formatAgentChatTitle(cardId, cardTitle),
          isTaskChat: true,
          mode: "TASK_AGENT",
          taskMeta: {
            task_id: taskId,
            role: "agents",
            card_id: cardId,
          },
          model: task?.default_agent_model,
        }),
      );

      dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: { type: "agent", cardId, chatId },
        }),
      );
    },
    [board, taskId, dispatch, task?.default_agent_model],
  );

  const handleCardAgentClick = useCallback(
    (card: BoardCard) => {
      if (!card.agent_chat_id) return;
      handleSelectAgent(card.id, card.agent_chat_id);
      setSelectedCardId(null);
    },
    [handleSelectAgent],
  );

  const handleInternalLink = useCallback(
    (url: string): boolean => {
      const parsed = parseRefactLink(url);
      if (!parsed) return false;

      if (parsed.type === "chat") {
        const chatId = parsed.id;
        const card = board?.cards.find((c) => c.agent_chat_id === chatId);

        let cardId = card?.id ?? "";
        if (!cardId && chatId.startsWith("agent-")) {
          // Format: agent-{card_id}-{uuid8}
          // Parse from end to handle hyphenated card IDs like "T-1"
          const withoutPrefix = chatId.slice("agent-".length);
          const lastDashIdx = withoutPrefix.lastIndexOf("-");
          if (lastDashIdx > 0) {
            cardId = withoutPrefix.slice(0, lastDashIdx);
          }
        }

        const cardTitle = card?.title ?? `Card ${cardId}`;

        dispatch(
          createChatWithId({
            id: chatId,
            title: formatAgentChatTitle(cardId, cardTitle),
            isTaskChat: true,
            mode: "TASK_AGENT",
            taskMeta: {
              task_id: taskId,
              role: "agents",
              card_id: cardId,
            },
            model: task?.default_agent_model,
          }),
        );

        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "agent", cardId, chatId },
          }),
        );
        return true;
      }

      return false;
    },
    [board, taskId, dispatch, task?.default_agent_model],
  );

  const handleToggleChatExpanded = useCallback(() => {
    setLayout((prev) => {
      const next = { ...prev, chatExpanded: !prev.chatExpanded };
      saveTaskWorkspaceLayout(taskId, next);
      return next;
    });
  }, [taskId]);

  const handleTogglePanelsExpanded = useCallback(() => {
    setLayout((prev) => {
      const next = { ...prev, panelsExpanded: !prev.panelsExpanded };
      saveTaskWorkspaceLayout(taskId, next);
      return next;
    });
  }, [taskId]);

  const handleBoardResizeDrag = useCallback(
    (clientY: number) => {
      const container = taskWorkspaceRef.current;
      const rect = container?.getBoundingClientRect();
      const nextHeight = clampBoardHeight(
        rect ? clientY - rect.top : clientY,
        rect?.height,
      );
      setLayout((prev) => {
        const next = { ...prev, boardHeightPx: nextHeight };
        saveTaskWorkspaceLayout(taskId, next);
        return next;
      });
    },
    [taskId],
  );

  const handleBoardResizeReset = useCallback(() => {
    setLayout((prev) => {
      const next = { ...prev, boardHeightPx: DEFAULT_BOARD_HEIGHT_PX };
      saveTaskWorkspaceLayout(taskId, next);
      return next;
    });
  }, [taskId]);

  const handleModelChange = useCallback(
    (model: string) => {
      void updateTaskMeta({ taskId, defaultAgentModel: model });
    },
    [taskId, updateTaskMeta],
  );

  useEffect(() => {
    if (!board || !selectedCardId) return;
    if (!board.cards.some((c) => c.id === selectedCardId)) {
      setSelectedCardId(null);
      showNotification("Card was deleted by another planner.");
    }
  }, [board, selectedCardId, showNotification]);

  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible") {
        dispatch(tasksApi.util.invalidateTags([{ type: "Board", id: taskId }]));
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [dispatch, taskId]);

  const invalidateTaskQueries = useCallback(() => {
    dispatch(
      tasksApi.util.invalidateTags([
        { type: "Tasks", id: taskId },
        { type: "Board", id: taskId },
        "Tasks",
      ]),
    );
  }, [dispatch, taskId]);

  const handleViewCardDiff = useCallback((worktree: CardWorktreeTarget) => {
    if (!isActionableWorktree(worktree)) return;
    setDiffTarget(worktree);
  }, []);

  const handleMergeCardWorktree = useCallback(
    (worktree: CardWorktreeTarget) => {
      if (!selectedCard || !isActionableWorktree(worktree)) return;
      setMergeTargetId(selectedCard.id);
      setMergeTargetWorktree(worktree);
    },
    [selectedCard],
  );

  const handleOpenCardWorktree = useCallback(
    async (worktree: CardWorktreeTarget) => {
      if (!isActionableWorktree(worktree)) return;
      try {
        const response = await openWorktree({
          id: worktree.id,
          source_workspace_root:
            worktree.record?.meta.source_workspace_root ??
            worktree.meta?.source_workspace_root,
        }).unwrap();
        const hostCanOpenFolder =
          config.host === "vscode" ||
          config.host === "jetbrains" ||
          config.host === "ide";
        if (response.can_open_folder && hostCanOpenFolder) {
          openFolderInNewWindow(response.path);
          showNotification("Opening worktree in a new window.");
        } else {
          copyToClipboard(response.path);
          showNotification("Worktree path copied to clipboard.");
        }
      } catch (error) {
        showNotification(`Open failed: ${worktreeErrorText(error)}`);
      }
    },
    [
      config.host,
      copyToClipboard,
      openFolderInNewWindow,
      openWorktree,
      showNotification,
    ],
  );

  const handleDeleteCardWorktree = useCallback(
    (worktree: CardWorktreeTarget) => {
      if (!selectedCard || !isActionableWorktree(worktree)) return;
      setDeleteBranch(false);
      setDeleteTargetId(selectedCard.id);
      setDeleteTargetWorktree(worktree);
    },
    [selectedCard],
  );

  const handleConfirmDeleteCardWorktree = useCallback(async () => {
    if (!deleteTarget || !isActionableWorktree(deleteTarget.worktree)) return;
    try {
      await deleteWorktree({
        id: deleteTarget.worktree.id,
        source_workspace_root:
          deleteTarget.worktree.record?.meta.source_workspace_root ??
          deleteTarget.worktree.meta?.source_workspace_root,
        delete_branch: deleteBranch,
      }).unwrap();
      if (deleteTarget.card.agent_chat_id) {
        dispatch(
          setThreadWorktree({
            chatId: deleteTarget.card.agent_chat_id,
            worktree: null,
          }),
        );
      }
      setDeleteTargetId(null);
      setDeleteTargetWorktree(null);
      invalidateTaskQueries();
      showNotification("Worktree deleted.");
    } catch (error) {
      showNotification(`Delete failed: ${worktreeErrorText(error)}`);
    }
  }, [
    deleteBranch,
    deleteTarget,
    deleteWorktree,
    dispatch,
    invalidateTaskQueries,
    showNotification,
  ]);

  const handleCardMergeCompleted = useCallback(
    (response: MergeWorktreeResponse) => {
      if (
        response.cleanup?.worktree_deleted &&
        mergeTarget?.card.agent_chat_id
      ) {
        dispatch(
          setThreadWorktree({
            chatId: mergeTarget.card.agent_chat_id,
            worktree: null,
          }),
        );
      }
      invalidateTaskQueries();
      showNotification("Worktree merge completed.");
    },
    [dispatch, invalidateTaskQueries, mergeTarget, showNotification],
  );

  const handleAskRefactForMerge = useCallback(
    async (files: string[], response: MergeWorktreeResponse) => {
      if (!mergeTarget) throw new Error("No task worktree is selected.");
      const fallbackPlannerId =
        activeChat?.type === "planner"
          ? activeChat.chatId
          : plannerChats[0]?.id;
      const chatId = mergeTarget.card.agent_chat_id ?? fallbackPlannerId;
      if (!chatId) throw new Error("No agent or planner chat is available.");
      if (!config.lspPort) throw new Error("LSP port is unavailable.");
      const apiKey = config.apiKey ?? undefined;
      const prompt = buildWorktreeConflictPrompt({
        worktree: mergeTarget.worktree.meta,
        record: mergeTarget.worktree.record,
        response,
        files,
        taskId,
        cardId: mergeTarget.card.id,
      });
      if (mergeTarget.card.agent_chat_id) {
        dispatch(
          createChatWithId({
            id: chatId,
            title: formatAgentChatTitle(
              mergeTarget.card.id,
              mergeTarget.card.title,
            ),
            isTaskChat: true,
            mode: "TASK_AGENT",
            taskMeta: {
              task_id: taskId,
              role: "agents",
              card_id: mergeTarget.card.id,
            },
            model: task?.default_agent_model,
            worktree: mergeTarget.worktree.meta ?? null,
          }),
        );
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: {
              type: "agent",
              cardId: mergeTarget.card.id,
              chatId,
            },
          }),
        );
      } else {
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "planner", chatId },
          }),
        );
      }
      dispatch(switchToThread({ id: chatId, openTab: false }));
      if (mergeTarget.worktree.meta) {
        dispatch(
          setThreadWorktree({ chatId, worktree: mergeTarget.worktree.meta }),
        );
      }
      await updateChatParams(
        chatId,
        { worktree_id: mergeTarget.worktree.id },
        config.lspPort,
        apiKey,
      );
      await sendUserMessage(chatId, prompt, config.lspPort, apiKey, true);
      showNotification("Conflict resolution request sent to Refact.");
    },
    [
      activeChat,
      plannerChats,
      config.apiKey,
      config.lspPort,
      dispatch,
      mergeTarget,
      showNotification,
      task?.default_agent_model,
      taskId,
    ],
  );

  if (taskLoading || boardLoading || !task || !board) {
    return <ChatLoading />;
  }

  const chatLabel = !activeChat
    ? "No chat selected"
    : activeChat.type === "planner"
      ? `Planner`
      : formatAgentChatTitle(
          activeChat.cardId,
          board.cards.find((c) => c.id === activeChat.cardId)?.title ?? "",
        );
  const agentChats = board.cards.filter((card) => card.agent_chat_id);
  const doneAgentChats = agentChats.filter((card) => card.column === "done");
  const chatToggleLabel = chatExpanded ? "Collapse chat" : "Expand chat";
  const panelsToggleLabel = panelsExpanded
    ? "Collapse planners and agents"
    : "Expand planners and agents";
  const boardSectionStyle: React.CSSProperties = {
    flex: `0 0 ${boardHeightPx}px`,
  };

  return (
    <Box ref={taskWorkspaceRef} className={styles.taskWorkspace}>
      <CollapsePanel
        collapsed={chatExpanded}
        className={styles.workspaceChromeCollapse}
      >
        <Box className={styles.boardSection} style={boardSectionStyle}>
          <KanbanBoard
            board={board}
            onCardClick={handleCardClick}
            onAgentClick={handleCardAgentClick}
          />
        </Box>

        <ResizeDivider
          onDrag={handleBoardResizeDrag}
          onReset={handleBoardResizeReset}
        />

        <Box className={styles.panelsWrapper}>
          <div className={styles.panelsHeader}>
            <button
              type="button"
              onClick={handleTogglePanelsExpanded}
              aria-expanded={panelsExpanded}
              aria-label={panelsToggleLabel}
              title={panelsToggleLabel}
              className={styles.sectionHeaderToggle}
            >
              <ChevronDownIcon
                className={`${styles.chevron} ${
                  panelsExpanded ? styles.chevronExpanded : ""
                }`}
              />
              <Text
                size="1"
                weight="bold"
                color="gray"
                className={styles.sectionHeaderLabel}
              >
                Planners / Agents
              </Text>
            </button>
            <Flex align="center" gap="2" className={styles.sectionHeaderMeta}>
              <Badge size="1" color="gray" variant="soft">
                {plannerChats.length} planner
                {plannerChats.length === 1 ? "" : "s"}
              </Badge>
              {agentChats.length > 0 && (
                <Badge size="1" color="gray" variant="soft">
                  {doneAgentChats.length}/{agentChats.length} agents
                </Badge>
              )}
              <button
                type="button"
                className={styles.sectionHeaderActionButton}
                onClick={handleNewPlanner}
                aria-label="New planner"
                title="New planner"
              >
                <PlusIcon />
              </button>
            </Flex>
          </div>

          <CollapsePanel
            collapsed={!panelsExpanded}
            className={styles.panelsCollapse}
          >
            <Flex className={styles.panelsSection}>
              <PlannerPanel
                plannerChats={plannerChats}
                activeChat={activeChat}
                onSelectPlanner={handleSelectPlanner}
                onRemovePlanner={handleRemovePlanner}
              />
              <AgentsPanel
                cards={board.cards}
                activeChat={activeChat}
                onSelectAgent={handleSelectAgent}
                defaultAgentModel={task.default_agent_model}
                onModelChange={handleModelChange}
              />
            </Flex>
          </CollapsePanel>
        </Box>
      </CollapsePanel>

      <Box className={styles.chatSection}>
        <Tabs.Root
          value={workspaceTab}
          onValueChange={setWorkspaceTab}
          className={styles.workspaceTabs}
        >
          <div className={styles.chatHeader}>
            <button
              type="button"
              onClick={handleToggleChatExpanded}
              aria-expanded={chatExpanded}
              aria-label={chatToggleLabel}
              title={chatToggleLabel}
              className={`${styles.sectionHeaderToggle} ${styles.chatHeaderToggle}`}
            >
              <ChevronDownIcon
                className={`${styles.chevron} ${
                  chatExpanded ? styles.chevronExpanded : ""
                }`}
              />
              <Text
                size="1"
                weight="bold"
                color="gray"
                className={styles.sectionHeaderLabel}
              >
                Task
              </Text>
              {workspaceTab === "chat" && (
                <Text size="1" color="gray" className={styles.chatHeaderLabel}>
                  {chatLabel}
                </Text>
              )}
            </button>
            <Tabs.List size="1">
              <Tabs.Trigger value="chat">Chat</Tabs.Trigger>
              <Tabs.Trigger value="memories">Memories</Tabs.Trigger>
              <Tabs.Trigger value="documents">Documents</Tabs.Trigger>
            </Tabs.List>
          </div>
          <Box className={styles.chatContent}>
            {workspaceTab === "chat" ? (
              <Box className={styles.workspaceTabContent}>
                {activeChat ? (
                  <InternalLinkProvider onInternalLink={handleInternalLink}>
                    <Chat
                      host={config.host}
                      tabbed={false}
                      backFromChat={handleBack}
                    />
                  </InternalLinkProvider>
                ) : (
                  <Flex
                    align="center"
                    justify="center"
                    style={{ height: "100%" }}
                  >
                    <Text color="gray">
                      Create a planner chat to get started
                    </Text>
                  </Flex>
                )}
              </Box>
            ) : workspaceTab === "memories" ? (
              <Box className={styles.workspaceTabContent}>
                <MemoryInboxPanel taskId={taskId} />
              </Box>
            ) : (
              <Box className={styles.workspaceTabContent}>
                <DocumentsPanel taskId={taskId} />
              </Box>
            )}
          </Box>
        </Tabs.Root>
      </Box>

      {selectedCard && (
        <CardDetail
          taskId={taskId}
          card={selectedCard}
          worktree={selectedCardWorktree}
          worktreeLabel={selectedCardWorktreeLabel}
          isWorktreeLoading={worktreesLoading}
          onClose={() => setSelectedCardId(null)}
          onInternalLink={handleInternalLink}
          onViewDiff={handleViewCardDiff}
          onMerge={handleMergeCardWorktree}
          onOpenWorktree={(worktree) => void handleOpenCardWorktree(worktree)}
          onDeleteWorktree={handleDeleteCardWorktree}
        />
      )}

      <WorktreeDiffPanel
        open={Boolean(diffTarget)}
        worktreeId={diffTarget?.id}
        worktree={diffTarget?.meta}
        record={diffTarget?.record}
        onOpenChange={(open) => {
          if (!open) setDiffTarget(null);
        }}
      />

      <MergeWorktreeModal
        open={Boolean(mergeTarget)}
        worktreeId={mergeTarget?.worktree.id}
        worktree={mergeTarget?.worktree.meta}
        record={mergeTarget?.worktree.record}
        taskId={taskId}
        defaultTargetBranch={task.base_branch}
        onOpenChange={(open) => {
          if (!open) {
            setMergeTargetId(null);
            setMergeTargetWorktree(null);
          }
        }}
        onMerged={handleCardMergeCompleted}
        onAskRefact={handleAskRefactForMerge}
        onOpenWorktree={() =>
          mergeTarget ? handleOpenCardWorktree(mergeTarget.worktree) : undefined
        }
      />

      <Dialog.Root
        open={Boolean(deleteTarget)}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTargetId(null);
            setDeleteTargetWorktree(null);
          }
        }}
      >
        <Dialog.Content maxWidth="420px">
          <Dialog.Title>Delete worktree</Dialog.Title>
          <Dialog.Description size="2" color="gray">
            Delete or discard this task agent worktree from disk.
          </Dialog.Description>
          <Flex direction="column" gap="3" mt="3">
            <Text size="2" weight="medium">
              {deleteTarget?.worktree.label ?? "Worktree"}
            </Text>
            {deleteTarget?.worktree.referenceCount !== undefined &&
              deleteTarget.worktree.referenceCount > 1 && (
                <Text size="2" color="amber">
                  This worktree is shared by{" "}
                  {deleteTarget.worktree.referenceCount} references.
                </Text>
              )}
            <Text as="label" size="2">
              <Flex align="center" gap="2">
                <Checkbox
                  checked={deleteBranch}
                  onCheckedChange={(checked) =>
                    setDeleteBranch(checked === true)
                  }
                  disabled={deleteWorktreeState.isLoading}
                />
                Delete git branch too
              </Flex>
            </Text>
          </Flex>
          <Flex justify="end" gap="2" mt="4">
            <Dialog.Close>
              <Button
                type="button"
                variant="soft"
                color="gray"
                disabled={deleteWorktreeState.isLoading}
              >
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              type="button"
              color="red"
              disabled={!deleteTarget || deleteWorktreeState.isLoading}
              onClick={() => void handleConfirmDeleteCardWorktree()}
            >
              {deleteWorktreeState.isLoading
                ? "Deleting..."
                : "Delete worktree"}
            </Button>
          </Flex>
        </Dialog.Content>
      </Dialog.Root>

      {notification && (
        <Box
          style={{
            position: "fixed",
            bottom: "var(--space-4)",
            left: "50%",
            transform: "translateX(-50%)",
            background: "var(--accent-9)",
            color: "white",
            padding: "var(--space-3) var(--space-4)",
            borderRadius: "var(--radius-3)",
            zIndex: 50,
            boxShadow: "0 4px 12px rgba(0, 0, 0, 0.15)",
          }}
        >
          <Text size="2">{notification}</Text>
        </Box>
      )}
    </Box>
  );
};
