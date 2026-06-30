import React, { useCallback, useState, useEffect, useMemo } from "react";
import { Flex, Box, Text } from "@radix-ui/themes";
import {
  Badge,
  Button,
  Dialog,
  Icon,
  IconButton,
  Popover,
  StatusDot,
  Tabs,
  Tooltip,
} from "../../components/ui";
import { Checkbox } from "../../components/Checkbox";
import { PlusIcon, ChevronDownIcon } from "@radix-ui/react-icons";
import { FileText, GitBranch, ListChecks, Target, X } from "lucide-react";
import { AgentStatusDot } from "./AgentStatusDot";
import { ScrollArea } from "../../components/ScrollArea";
import { ChatLoading } from "../../components/ChatContent/ChatLoading";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, popBackTo, push } from "../Pages/pagesSlice";
import { KanbanBoard } from "./KanbanBoard";
import {
  useGetTaskQuery,
  useGetBoardQuery,
  useListTaskTrajectoriesQuery,
  useCreatePlannerChatMutation,
  useDeletePlannerChatMutation,
  BoardCard,
  tasksApi,
} from "../../services/refact/tasks";
import { Markdown } from "../../components/Markdown";
import { ModeMenuItem } from "../../components/ChatForm/ModeSelect";
import modeSelectStyles from "../../components/ChatForm/ModeSelect.module.css";
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
  updatePlannerChat,
  PlannerInfo,
} from "./tasksSlice";
import {
  selectBackgroundAgentsByThread,
  selectCurrentThreadId,
  selectRuntimeById,
  selectThreadById,
} from "../Chat/Thread";
import { getStatusFromSessionState } from "../../utils/sessionStatus";
import { useGetChatModesQuery } from "../../services/refact/chatModes";
import { InternalLinkProvider } from "../../contexts/InternalLinkContext";
import { parseRefactLink } from "../../contexts/internalLinkUtils";
import { resolveChatLink } from "./internalLinkResolver";
import {
  useDeleteWorktreeMutation,
  useListWorktreesQuery,
  useOpenWorktreeMutation,
  type MergeWorktreeResponse,
  type BackgroundAgentSummary,
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
  loadTaskWorkspaceTab,
  saveTaskWorkspaceTab,
  type TaskWorkspaceTab,
} from "../../utils/chatUiPersistence";
import { MemoryInboxPanel } from "./TaskMemories/MemoryInboxPanel";
import { DocumentsPanel } from "./TaskDocuments/DocumentsPanel";
import { CardCommentsSection } from "./CardComments";
import {
  isActionableWorktree,
  resolveCardWorktree,
  worktreeLabel,
  type CardWorktreeTarget,
} from "./TaskWorkspaceWorktree";

type ActiveChat =
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string }
  | null;

const LEGACY_WORKTREE_TOOLTIP =
  "This worktree was created before the registry; recreate it via `restart_agent(mode=fresh)` to enable actions.";
const EMPTY_BACKGROUND_AGENTS: Record<string, BackgroundAgentSummary> =
  Object.freeze({});
const EMPTY_LINKED_CARDS: string[] = [];

interface PlannerPanelProps {
  plannerChats: PlannerInfo[];
  activeChat: ActiveChat;
  linkedCardsByPlanner: Map<string, string[]>;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
}

interface PlannerItemProps {
  planner: PlannerInfo;
  isSelected: boolean;
  linkedCardIds?: string[];
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

function sameWaitingCards(a?: string[], b?: string[]): boolean {
  if (a === b) return true;
  const left = a ?? [];
  const right = b ?? [];
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i += 1) {
    if (left[i] !== right[i]) return false;
  }
  return true;
}

const cardStatusTone = (
  column: string,
): React.ComponentProps<typeof Badge>["tone"] => {
  if (column === "done") return "success";
  if (column === "failed") return "danger";
  if (column === "doing") return "accent";
  return "muted";
};

const workspaceTabIndex = (tab: TaskWorkspaceTab): number => {
  if (tab === "chat") return 1;
  if (tab === "memories") return 2;
  if (tab === "documents") return 3;
  return 0;
};

const isTaskWorkspaceTab = (value: string): value is TaskWorkspaceTab =>
  value === "board" ||
  value === "chat" ||
  value === "memories" ||
  value === "documents";

export const PlannerItem: React.FC<PlannerItemProps> = ({
  planner,
  isSelected,
  linkedCardIds = EMPTY_LINKED_CARDS,
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

  const sessionState = runtime?.session_state ?? planner.sessionState;
  const statusDot = getStatusFromSessionState(sessionState);
  const mode = planner.mode ?? thread?.mode;
  const showModeBadge = Boolean(mode) && mode !== "task_planner";
  const isWaiting = sessionState === "waiting_user_input";
  const waitingCards = planner.waitingForCardIds ?? [];
  const showWaitingChips = isWaiting && waitingCards.length > 0;
  const visibleCards = waitingCards.slice(0, 5);
  const hiddenCount = Math.max(0, waitingCards.length - 5);
  const visibleLinkedCards = linkedCardIds.slice(0, 4);
  const hiddenLinkedCount = Math.max(0, linkedCardIds.length - 4);

  return (
    <Box
      className={`${styles.panelItem} rf-pressable ${
        isSelected ? styles.panelItemSelected : ""
      }`}
      role="button"
      tabIndex={0}
      aria-label={`Open chat ${displayTitle}`}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className={styles.panelItemLead}>
        <StatusDot
          status={statusDot}
          size="medium"
          pulse={statusDot === "in_progress"}
        />
      </div>
      <Box className={styles.panelItemContent}>
        <div className={styles.panelItemTitleRow}>
          {showModeBadge && (
            <Badge tone="muted" className={styles.agentItemBadge}>
              {mode}
            </Badge>
          )}
          <Text size="1" className={styles.panelItemTitle}>
            {displayTitle}
          </Text>
        </div>
        {linkedCardIds.length > 0 && (
          <Flex
            gap="1"
            wrap="wrap"
            align="center"
            className={styles.plannerLinkedCards}
            data-testid={`planner-linked-cards-${planner.id}`}
          >
            {visibleLinkedCards.map((cardId) => (
              <Badge
                key={cardId}
                tone="muted"
                className={styles.agentItemBadge}
                title={`Spawned agent for ${cardId}`}
              >
                {cardId}
              </Badge>
            ))}
            {hiddenLinkedCount > 0 && (
              <Text size="1" color="gray" className={styles.plannerWaitingMore}>
                +{hiddenLinkedCount}
              </Text>
            )}
          </Flex>
        )}
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
                tone="warning"
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
      </Box>
      <Tooltip content="Delete chat">
        <span>
          <IconButton
            size="sm"
            variant="ghost"
            aria-label="Delete chat"
            icon={X}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
          />
        </span>
      </Tooltip>
    </Box>
  );
};

const PlannerPanel: React.FC<PlannerPanelProps> = ({
  plannerChats,
  activeChat,
  linkedCardsByPlanner,
  onSelectPlanner,
  onRemovePlanner,
}) => {
  return (
    <Box className={styles.panelList}>
      <Box className={styles.panelContent}>
        {plannerChats.length === 0 ? (
          <Flex align="center" justify="center" className={styles.emptyState}>
            <Text size="1" color="gray">
              No chats yet
            </Text>
          </Flex>
        ) : (
          <ScrollArea
            className={styles.panelScrollArea}
            data-testid="planner-panel-scroll-owner"
            scrollbars="vertical"
          >
            <Flex direction="column" gap="1" className="rf-stagger">
              {plannerChats.map((planner) => (
                <PlannerItem
                  key={planner.id}
                  planner={planner}
                  isSelected={
                    activeChat?.type === "planner" &&
                    activeChat.chatId === planner.id
                  }
                  linkedCardIds={
                    linkedCardsByPlanner.get(planner.id) ?? EMPTY_LINKED_CARDS
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

type AgentChatStatus = "doing" | "done" | "failed";

interface AgentChatEntry {
  card: BoardCard;
  status: AgentChatStatus;
}

function agentChatEntries(cards: BoardCard[]): AgentChatEntry[] {
  const byColumn = (column: AgentChatStatus): AgentChatEntry[] =>
    cards
      .filter((card) => card.column === column && card.agent_chat_id)
      .map((card) => ({ card, status: column }));
  return [...byColumn("doing"), ...byColumn("done"), ...byColumn("failed")];
}

interface AgentItemProps {
  card: BoardCard;
  status: AgentChatStatus;
  isSelected: boolean;
  onSelect: () => void;
}

const AgentItem: React.FC<AgentItemProps> = ({
  card,
  status,
  isSelected,
  onSelect,
}) => {
  return (
    <Box
      className={`${styles.panelItem} rf-pressable ${
        isSelected ? styles.panelItemSelected : ""
      }`}
      role="button"
      tabIndex={0}
      aria-label={`Open agent chat ${card.id} ${card.title}`}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className={styles.panelItemLead}>
        <AgentStatusDot status={status} size="medium" />
      </div>
      <Flex align="center" gap="1" className={styles.panelItemContent}>
        <Badge tone="muted" className={styles.agentItemBadge}>
          {card.id}
        </Badge>
        <Text size="1" className={styles.panelItemTitle}>
          {card.title}
        </Text>
      </Flex>
    </Box>
  );
};

interface AgentsPanelProps {
  cards: BoardCard[];
  activeChat: ActiveChat;
  onSelectAgent: (cardId: string, chatId: string) => void;
}

const AgentsPanel: React.FC<AgentsPanelProps> = ({
  cards,
  activeChat,
  onSelectAgent,
}) => {
  const agents = agentChatEntries(cards);

  return (
    <Box className={styles.panelList}>
      <Box className={styles.panelContent}>
        {agents.length === 0 ? (
          <Flex align="center" justify="center" className={styles.emptyState}>
            <Text size="1" color="gray">
              No task agents yet
            </Text>
          </Flex>
        ) : (
          <ScrollArea
            className={styles.panelScrollArea}
            data-testid="agents-panel-scroll-owner"
            scrollbars="vertical"
          >
            <Flex direction="column" gap="1" className="rf-stagger">
              {agents.map(({ card, status }) => (
                <AgentItem
                  key={card.id}
                  card={card}
                  status={status}
                  isSelected={
                    activeChat?.type === "agent" &&
                    activeChat.cardId === card.id
                  }
                  onSelect={() =>
                    card.agent_chat_id &&
                    onSelectAgent(card.id, card.agent_chat_id)
                  }
                />
              ))}
            </Flex>
          </ScrollArea>
        )}
      </Box>
    </Box>
  );
};

interface BoardRailProps {
  plannerChats: PlannerInfo[];
  cards: BoardCard[];
  activeChat: ActiveChat;
  linkedCardsByPlanner: Map<string, string[]>;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
  onSelectAgent: (cardId: string, chatId: string) => void;
}

const BoardRail: React.FC<BoardRailProps> = ({
  plannerChats,
  cards,
  activeChat,
  linkedCardsByPlanner,
  onSelectPlanner,
  onRemovePlanner,
  onSelectAgent,
}) => {
  const agentChats = cards.filter((card) => card.agent_chat_id);
  const doneAgentChats = agentChats.filter((card) => card.column === "done");

  return (
    <aside className={styles.boardRail} aria-label="Chats and task agents">
      <div className={styles.railGroupHeader}>
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionHeaderLabel}
        >
          Chats
        </Text>
        <Flex align="center" gap="2" className={styles.sectionHeaderMeta}>
          <Badge tone="muted">{plannerChats.length}</Badge>
        </Flex>
      </div>
      <PlannerPanel
        plannerChats={plannerChats}
        activeChat={activeChat}
        linkedCardsByPlanner={linkedCardsByPlanner}
        onSelectPlanner={onSelectPlanner}
        onRemovePlanner={onRemovePlanner}
      />
      <div className={styles.railGroupHeader}>
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionHeaderLabel}
        >
          Task Agents
        </Text>
        <Flex align="center" gap="2" className={styles.sectionHeaderMeta}>
          <Badge tone="muted">
            {doneAgentChats.length}/{agentChats.length}
          </Badge>
        </Flex>
      </div>
      <AgentsPanel
        cards={cards}
        activeChat={activeChat}
        onSelectAgent={onSelectAgent}
      />
    </aside>
  );
};

interface ChatSwitcherProps {
  label: string;
  plannerChats: PlannerInfo[];
  cards: BoardCard[];
  activeChat: ActiveChat;
  linkedCardsByPlanner: Map<string, string[]>;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
  onSelectAgent: (cardId: string, chatId: string) => void;
}

const ChatSwitcher: React.FC<ChatSwitcherProps> = ({
  label,
  plannerChats,
  cards,
  activeChat,
  linkedCardsByPlanner,
  onSelectPlanner,
  onRemovePlanner,
  onSelectAgent,
}) => {
  const [open, setOpen] = useState(false);
  const agents = agentChatEntries(cards);
  const activeAgent =
    activeChat?.type === "agent"
      ? agents.find(({ card }) => card.id === activeChat.cardId)
      : undefined;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className={styles.chatSwitcherTrigger}
          aria-label="Switch chat"
          title="Switch chat"
        >
          {activeAgent && (
            <AgentStatusDot status={activeAgent.status} size="small" />
          )}
          <Text size="1" className={styles.chatSwitcherLabel}>
            {label}
          </Text>
          <ChevronDownIcon className={styles.chatSwitcherChevron} />
        </button>
      </Popover.Trigger>
      <Popover.Content
        align="end"
        sideOffset={6}
        maxWidth="340px"
        className={styles.chatSwitcherContent}
      >
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionHeaderLabel}
        >
          Chats
        </Text>
        {plannerChats.length === 0 ? (
          <Text size="1" color="gray">
            No chats yet
          </Text>
        ) : (
          <Flex direction="column" gap="1">
            {plannerChats.map((planner) => (
              <PlannerItem
                key={planner.id}
                planner={planner}
                isSelected={
                  activeChat?.type === "planner" &&
                  activeChat.chatId === planner.id
                }
                linkedCardIds={
                  linkedCardsByPlanner.get(planner.id) ?? EMPTY_LINKED_CARDS
                }
                onSelect={() => {
                  setOpen(false);
                  onSelectPlanner(planner.id);
                }}
                onRemove={() => onRemovePlanner(planner.id)}
              />
            ))}
          </Flex>
        )}
        {agents.length > 0 && (
          <>
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionHeaderLabel}
            >
              Task Agents
            </Text>
            <Flex direction="column" gap="1">
              {agents.map(({ card, status }) => (
                <AgentItem
                  key={card.id}
                  card={card}
                  status={status}
                  isSelected={
                    activeChat?.type === "agent" &&
                    activeChat.cardId === card.id
                  }
                  onSelect={() => {
                    if (!card.agent_chat_id) return;
                    setOpen(false);
                    onSelectAgent(card.id, card.agent_chat_id);
                  }}
                />
              ))}
            </Flex>
          </>
        )}
      </Popover.Content>
    </Popover>
  );
};

interface NewChatModeButtonProps {
  disabled?: boolean;
  onCreate: (mode: string) => void;
}

const EXCLUDED_NEW_CHAT_MODES = new Set(["task_planner", "task_agent"]);

const NewChatModeButton: React.FC<NewChatModeButtonProps> = ({
  disabled,
  onCreate,
}) => {
  const [open, setOpen] = useState(false);
  const { data } = useGetChatModesQuery(undefined);
  const modes = useMemo(
    () =>
      (data?.modes ?? []).filter(
        (mode) => !EXCLUDED_NEW_CHAT_MODES.has(mode.id),
      ),
    [data],
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className={styles.headerActionButton}
          disabled={disabled}
          aria-label="New chat"
          title="New chat"
        >
          <PlusIcon />
          <Text size="1">Chat</Text>
          <ChevronDownIcon className={styles.chatSwitcherChevron} />
        </button>
      </Popover.Trigger>
      <Popover.Content
        align="end"
        sideOffset={6}
        maxWidth="320px"
        className={styles.chatSwitcherContent}
      >
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionHeaderLabel}
        >
          New chat
        </Text>
        {modes.length === 0 ? (
          <Text size="1" color="gray">
            No modes available
          </Text>
        ) : (
          <div className={modeSelectStyles.modeList}>
            {modes.map((mode, index) => (
              <React.Fragment key={mode.id}>
                {index > 0 && <div className={modeSelectStyles.separator} />}
                <ModeMenuItem
                  mode={mode}
                  isSelected={false}
                  onSelect={() => {
                    setOpen(false);
                    onCreate(mode.id);
                  }}
                />
              </React.Fragment>
            ))}
          </div>
        )}
      </Popover.Content>
    </Popover>
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
    <Dialog.Content
      className={styles.cardDetailDialog}
      maxHeight="min(760px, calc(100dvh - var(--rf-space-5)))"
      maxWidth="720px"
    >
      <div className={styles.cardDetailRoot}>
        <div className={styles.cardDetailHeader}>
          <div className={styles.cardDetailTitleGroup}>
            <Badge tone="muted">{card.id}</Badge>
            <Dialog.Title className={styles.cardDetailTitle}>
              {card.title}
            </Dialog.Title>
          </div>
          <Badge tone={cardStatusTone(card.column)}>
            {card.column === "doing" ||
            card.column === "done" ||
            card.column === "failed" ? (
              <AgentStatusDot status={card.column} size="small" />
            ) : null}
            {card.column}
          </Badge>
        </div>

        <section className={styles.cardDetailMetaGrid}>
          <div className={styles.cardDetailMetaItem}>
            <span className={styles.cardDetailMetaLabel}>Priority</span>
            <Badge
              tone={
                card.priority === "P0"
                  ? "danger"
                  : card.priority === "P1"
                    ? "warning"
                    : "muted"
              }
            >
              {card.priority}
            </Badge>
          </div>
          {card.depends_on.length > 0 && (
            <div className={styles.cardDetailMetaItem}>
              <span className={styles.cardDetailMetaLabel}>Dependencies</span>
              <div className={styles.cardDetailChipRow}>
                {card.depends_on.map((dep) => (
                  <Badge key={dep} tone="muted">
                    {dep}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          {worktreeLabel && (
            <div className={styles.cardDetailMetaItem}>
              <span className={styles.cardDetailMetaLabel}>Worktree</span>
              <div className={styles.cardDetailChipRow}>
                <Badge tone="success" title={`Worktree: ${worktreeLabel}`}>
                  <BranchIcon /> {worktreeLabel}
                </Badge>
                {worktree?.record ?? worktree?.meta ? (
                  <WorktreeStatusBadge
                    worktree={worktree.meta ?? worktree.record?.meta}
                    record={worktree.record}
                  />
                ) : null}
                {worktree?.referenceCount && worktree.referenceCount > 1 ? (
                  <Badge tone="warning">
                    shared by {worktree.referenceCount}
                  </Badge>
                ) : null}
              </div>
            </div>
          )}
        </section>

        {worktreeLabel && (
          <section className={styles.cardDetailSectionBlock}>
            <div className={styles.cardDetailSectionHeader}>
              <Icon icon={GitBranch} size="sm" tone="muted" />
              <Text size="2" weight="medium">
                Worktree actions
              </Text>
            </div>
            <div className={styles.cardDetailWorktreeBody}>
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
              <div className={styles.cardDetailActions}>
                {wrapWorktreeAction(
                  <Button
                    type="button"
                    size="sm"
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
                    size="sm"
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
                    size="sm"
                    variant="soft"
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
                    size="sm"
                    variant="danger"
                    disabled={worktreeActionsDisabled}
                    title={worktreeActionsTooltip}
                    onClick={() => invokeWorktreeAction(onDeleteWorktree)}
                  >
                    Discard/Delete
                  </Button>,
                )}
              </div>
            </div>
          </section>
        )}

        <section className={styles.cardDetailSectionBlock}>
          <div className={styles.cardDetailSectionHeader}>
            <Icon icon={Target} size="sm" tone="muted" />
            <Text size="2" weight="medium">
              Goal
            </Text>
          </div>
          <Box className={styles.cardDetailSection}>{card.title}</Box>
        </section>

        {card.target_files.length > 0 && (
          <section className={styles.cardDetailSectionBlock}>
            <div className={styles.cardDetailSectionHeader}>
              <Icon icon={FileText} size="sm" tone="muted" />
              <Text size="2" weight="medium">
                Files
              </Text>
            </div>
            <Box className={styles.cardDetailSection}>
              <div className={styles.cardDetailFileList}>
                {card.target_files.map((file) => (
                  <Badge key={file} tone="muted">
                    {file}
                  </Badge>
                ))}
              </div>
            </Box>
          </section>
        )}

        {card.instructions && (
          <section className={styles.cardDetailSectionBlock}>
            <div className={styles.cardDetailSectionHeader}>
              <Icon icon={ListChecks} size="sm" tone="muted" />
              <Text size="2" weight="medium">
                Instructions
              </Text>
            </div>
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
          </section>
        )}

        {card.final_report && (
          <section className={styles.cardDetailSectionBlock}>
            <div className={styles.cardDetailSectionHeader}>
              <Icon icon={FileText} size="sm" tone="success" />
              <Text size="2" weight="medium">
                Final Report
              </Text>
            </div>
            <Box
              className={`${styles.cardDetailSection} ${styles.finalReportSection}`}
            >
              <Markdown canHaveInteractiveElements={false}>
                {card.final_report}
              </Markdown>
            </Box>
          </section>
        )}

        {card.status_updates.length > 0 && (
          <section className={styles.cardDetailSectionBlock}>
            <div className={styles.cardDetailSectionHeader}>
              <Icon icon={ListChecks} size="sm" tone="muted" />
              <Text size="2" weight="medium">
                Updates
              </Text>
            </div>
            <div className={`${styles.cardDetailUpdates} rf-stagger`}>
              {card.status_updates.map((update, i) => (
                <div
                  key={i}
                  className={`${styles.cardDetailUpdate} rf-enter-rise`}
                >
                  <Text size="1" color="gray">
                    {new Date(update.timestamp).toLocaleString()}
                  </Text>
                  <Text size="2">{update.message}</Text>
                </div>
              ))}
            </div>
          </section>
        )}

        <CardCommentsSection
          taskId={taskId}
          cardId={card.id}
          comments={card.comments ?? []}
        />

        <div className={styles.cardDetailFooter}>
          <Dialog.Close asChild>
            <Button variant="soft">Close</Button>
          </Dialog.Close>
        </div>
      </div>
    </Dialog.Content>
  );
};

interface TaskWorkspaceProps {
  taskId: string;
}

export const TaskWorkspace: React.FC<TaskWorkspaceProps> = ({ taskId }) => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const {
    data: task,
    isLoading: taskLoading,
    isError: taskError,
  } = useGetTaskQuery(taskId, {
    pollingInterval: 0,
  });
  const {
    data: board,
    isLoading: boardLoading,
    isError: boardError,
  } = useGetBoardQuery(taskId, {
    pollingInterval: 0,
  });
  const { data: worktreesData, isLoading: worktreesLoading } =
    useListWorktreesQuery(undefined);
  const [openWorktree] = useOpenWorktreeMutation();
  const [deleteWorktree, deleteWorktreeState] = useDeleteWorktreeMutation();
  const copyToClipboard = useCopyToClipboard();
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const { data: savedPlanners, isLoading: savedPlannersLoading } =
    useListTaskTrajectoriesQuery({
      taskId,
      role: "planner",
    });
  const { data: savedAgents } = useListTaskTrajectoriesQuery({
    taskId,
    role: "agents",
  });
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
  const linkedCardsByPlanner = useMemo(() => {
    const agentToPlanner = new Map<string, string>();
    for (const traj of savedAgents ?? []) {
      if (traj.parent_id) agentToPlanner.set(traj.id, traj.parent_id);
    }
    const result = new Map<string, string[]>();
    for (const card of board?.cards ?? []) {
      if (!card.agent_chat_id) continue;
      const plannerId = agentToPlanner.get(card.agent_chat_id);
      if (!plannerId) continue;
      const existing = result.get(plannerId);
      if (existing) {
        existing.push(card.id);
      } else {
        result.set(plannerId, [card.id]);
      }
    }
    return result;
  }, [savedAgents, board?.cards]);
  const activeChat = useAppSelector((state) =>
    selectTaskActiveChat(state, taskId),
  );
  const hasActiveChatRuntime = useAppSelector((state) =>
    activeChat ? Boolean(selectRuntimeById(state, activeChat.chatId)) : false,
  );
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const activeChatBackgroundAgents = useAppSelector((state) =>
    activeChat
      ? selectBackgroundAgentsByThread(state, activeChat.chatId)
      : EMPTY_BACKGROUND_AGENTS,
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
  const notificationTimerRef = React.useRef<ReturnType<
    typeof setTimeout
  > | null>(null);
  useEffect(() => {
    return () => {
      if (notificationTimerRef.current)
        clearTimeout(notificationTimerRef.current);
    };
  }, []);
  const [explicitTab, setExplicitTab] = useState<TaskWorkspaceTab | null>(() =>
    loadTaskWorkspaceTab(taskId),
  );
  const smartDefaultTab: TaskWorkspaceTab =
    (savedPlanners?.length ?? 0) > 0 ? "chat" : "board";
  const workspaceTab: TaskWorkspaceTab = explicitTab ?? smartDefaultTab;
  const prevTaskStatusRef = React.useRef<string | undefined>(undefined);
  // Just-created chats are protected from reconciliation until the saved
  // trajectory list refetch includes them (prevents bouncing to old planner).
  const pendingCreatedPlannerIdsRef = React.useRef<Set<string>>(new Set());
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

    const savedPlannerIds = new Set(savedPlanners.map((planner) => planner.id));

    const pendingCreatedIds = pendingCreatedPlannerIdsRef.current;
    for (const id of Array.from(pendingCreatedIds)) {
      if (savedPlannerIds.has(id)) pendingCreatedIds.delete(id);
    }

    for (const planner of currentTaskUI.plannerChats) {
      if (
        !savedPlannerIds.has(planner.id) &&
        !pendingCreatedIds.has(planner.id)
      ) {
        dispatch(removePlannerChat({ taskId, chatId: planner.id }));
      }
    }

    for (const traj of savedPlanners) {
      dispatch(
        createChatWithId({
          id: traj.id,
          title: traj.title,
          isTaskChat: true,
          openTab: false,
          mode: traj.mode ?? "TASK_PLANNER",
          taskMeta: {
            task_id: taskId,
            role: "planner",
            planner_chat_id: traj.id,
          },
        }),
      );

      const existing = currentTaskUI.plannerChats.find((p) => p.id === traj.id);
      if (existing) {
        if (
          existing.title !== traj.title ||
          existing.updatedAt !== traj.updated_at ||
          existing.sessionState !== traj.session_state ||
          existing.mode !== traj.mode ||
          !sameWaitingCards(
            existing.waitingForCardIds,
            traj.waiting_for_card_ids,
          )
        ) {
          dispatch(
            updatePlannerChat({
              taskId,
              planner: {
                id: traj.id,
                title: traj.title,
                updatedAt: traj.updated_at,
                sessionState: traj.session_state,
                mode: traj.mode,
                waitingForCardIds: traj.waiting_for_card_ids,
              },
            }),
          );
        }
        continue;
      }

      dispatch(
        addPlannerChat({
          taskId,
          planner: {
            id: traj.id,
            title: traj.title,
            createdAt: traj.created_at,
            updatedAt: traj.updated_at,
            sessionState: traj.session_state,
            mode: traj.mode,
            waitingForCardIds: traj.waiting_for_card_ids,
          },
        }),
      );
    }

    const mostRecentPlanner =
      savedPlanners.length > 0
        ? savedPlanners.reduce((latest, planner) =>
            planner.updated_at > latest.updated_at ? planner : latest,
          )
        : null;
    const fallbackActiveChat = mostRecentPlanner
      ? { type: "planner" as const, chatId: mostRecentPlanner.id }
      : null;

    if (!activeChat) {
      if (fallbackActiveChat) {
        dispatch(setTaskActiveChat({ taskId, activeChat: fallbackActiveChat }));
      }
      return;
    }

    if (
      activeChat.type === "planner" &&
      !savedPlannerIds.has(activeChat.chatId) &&
      !pendingCreatedIds.has(activeChat.chatId)
    ) {
      dispatch(setTaskActiveChat({ taskId, activeChat: fallbackActiveChat }));
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
    if (activeChat?.type !== "agent" || !board || hasActiveChatRuntime) return;
    const card = board.cards.find(
      (candidate) =>
        candidate.id === activeChat.cardId &&
        candidate.agent_chat_id === activeChat.chatId,
    );
    if (!card) return;

    dispatch(
      createChatWithId({
        id: activeChat.chatId,
        title: formatAgentChatTitle(card.id, card.title),
        isTaskChat: true,
        openTab: false,
        mode: "TASK_AGENT",
        taskMeta: {
          task_id: taskId,
          role: "agents",
          card_id: card.id,
        },
      }),
    );
  }, [activeChat, board, dispatch, hasActiveChatRuntime, taskId]);

  useEffect(() => {
    if (!task) return;

    const prevStatus = prevTaskStatusRef.current;
    const currentStatus = task.status;

    prevTaskStatusRef.current = currentStatus;

    if (prevStatus === "planning" && currentStatus === "active") {
      setNotification("Planning complete! You can now spawn agents.");
      if (notificationTimerRef.current)
        clearTimeout(notificationTimerRef.current);
      notificationTimerRef.current = setTimeout(
        () => setNotification(null),
        3000,
      );
    }
  }, [task]);

  useEffect(() => {
    if (!activeChat || !hasActiveChatRuntime) return;
    if (currentThreadId === activeChat.chatId) return;
    dispatch(switchToThread({ id: activeChat.chatId, openTab: false }));
  }, [dispatch, activeChat, hasActiveChatRuntime, currentThreadId]);

  const handleBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleCardClick = useCallback((card: BoardCard) => {
    setSelectedCardId(card.id);
  }, []);

  const showNotification = useCallback((message: string) => {
    setNotification(message);
    if (notificationTimerRef.current)
      clearTimeout(notificationTimerRef.current);
    notificationTimerRef.current = setTimeout(
      () => setNotification(null),
      3000,
    );
  }, []);

  const handleWorkspaceTabChange = useCallback(
    (value: string) => {
      if (!isTaskWorkspaceTab(value)) return;
      setExplicitTab(value);
      saveTaskWorkspaceTab(taskId, value);
    },
    [taskId],
  );

  const openChatTab = useCallback(() => {
    handleWorkspaceTabChange("chat");
  }, [handleWorkspaceTabChange]);

  const createTaskChat = useCallback(
    (mode: string) => {
      if (isCreatingPlanner) return;
      createPlannerChat({ taskId, mode })
        .unwrap()
        .then((result) => {
          const newChatId = result.chat_id;
          const resolvedMode = result.mode ?? mode;
          const now = new Date().toISOString();
          pendingCreatedPlannerIdsRef.current.add(newChatId);
          dispatch(
            createChatWithId({
              id: newChatId,
              title: "",
              isTaskChat: true,
              openTab: false,
              mode: resolvedMode,
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
                mode: resolvedMode,
              },
            }),
          );
          dispatch(
            setTaskActiveChat({
              taskId,
              activeChat: { type: "planner", chatId: newChatId },
            }),
          );
          openChatTab();
        })
        .catch((err: unknown) => {
          showNotification(`Create failed: ${parsePlannerDeleteError(err)}`);
        });
    },
    [
      dispatch,
      taskId,
      createPlannerChat,
      isCreatingPlanner,
      openChatTab,
      showNotification,
    ],
  );

  const handleNewPlanner = useCallback(() => {
    createTaskChat("task_planner");
  }, [createTaskChat]);

  const handleRemovePlanner = useCallback(
    (chatId: string) => {
      const previous = plannerChats.find((p) => p.id === chatId);
      pendingCreatedPlannerIdsRef.current.delete(chatId);
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
      openChatTab();
    },
    [dispatch, taskId, openChatTab],
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
          openTab: false,
          mode: "TASK_AGENT",
          taskMeta: {
            task_id: taskId,
            role: "agents",
            card_id: cardId,
          },
        }),
      );

      dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: { type: "agent", cardId, chatId },
        }),
      );
      openChatTab();
    },
    [board, taskId, dispatch, openChatTab],
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

      if (parsed.type !== "chat" || !parsed.id) return false;

      const action = resolveChatLink(parsed.id, plannerChats, board);
      switch (action.kind) {
        case "planner":
          handleSelectPlanner(action.chatId);
          return true;
        case "agent":
          handleSelectAgent(action.cardId, action.chatId);
          return true;
        case "unknown": {
          const agent = Object.values(activeChatBackgroundAgents).find(
            (candidate) => candidate.child_chat_id === action.chatId,
          );
          if (!agent) showNotification(`Chat not found: ${action.chatId}`);
          dispatch(
            createChatWithId({
              id: action.chatId,
              parentId: activeChat?.chatId,
              linkType:
                agent?.kind ??
                (activeChat?.type === "agent" ? "delegate" : "subagent"),
            }),
          );
          dispatch(switchToThread({ id: action.chatId }));
          dispatch(popBackTo({ name: "history" }));
          dispatch(push({ name: "chat" }));
          return true;
        }
      }
    },
    [
      activeChat,
      activeChatBackgroundAgents,
      board,
      dispatch,
      plannerChats,
      handleSelectPlanner,
      handleSelectAgent,
      showNotification,
    ],
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
        dispatch(
          tasksApi.util.invalidateTags([
            { type: "Board", id: taskId },
            { type: "TaskTrajectories", id: `${taskId}/agents` },
          ]),
        );
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
        force_referenced: true,
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
            openTab: false,
            mode: "TASK_AGENT",
            taskMeta: {
              task_id: taskId,
              role: "agents",
              card_id: mergeTarget.card.id,
            },
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
      openChatTab();
      dispatch(switchToThread({ id: chatId, openTab: false }));
      if (mergeTarget.worktree.meta) {
        dispatch(
          setThreadWorktree({ chatId, worktree: mergeTarget.worktree.meta }),
        );
      }
      await updateChatParams(
        chatId,
        { worktree_id: mergeTarget.worktree.id },
        config,
        apiKey,
      );
      await sendUserMessage(chatId, prompt, config, apiKey, true);
      showNotification("Conflict resolution request sent to Refact.");
    },
    [
      activeChat,
      plannerChats,
      config,
      dispatch,
      mergeTarget,
      openChatTab,
      showNotification,
      taskId,
    ],
  );

  if (taskError || boardError) {
    return (
      <Flex
        align="center"
        justify="center"
        className={styles.fullHeightEmptyState}
      >
        <Text color="gray">Task is no longer available.</Text>
      </Flex>
    );
  }

  if (taskLoading || boardLoading || savedPlannersLoading || !task || !board) {
    return <ChatLoading />;
  }

  const chatLabel = !activeChat
    ? "No chat selected"
    : activeChat.type === "planner"
      ? "Planner"
      : formatAgentChatTitle(
          activeChat.cardId,
          board.cards.find((c) => c.id === activeChat.cardId)?.title ?? "",
        );
  const runningAgentCount = board.cards.filter(
    (card) => card.column === "doing" && card.agent_chat_id,
  ).length;
  const waitingPlannerCount = plannerChats.filter(
    (planner) => planner.sessionState === "waiting_user_input",
  ).length;

  return (
    <Box className={styles.taskWorkspace}>
      <Tabs
        value={workspaceTab}
        onValueChange={handleWorkspaceTabChange}
        className={styles.workspaceTabs}
      >
        <div className={styles.workspaceHeader}>
          <Tabs.List
            activeIndex={workspaceTabIndex(workspaceTab)}
            className={styles.workspaceTabList}
            itemCount={4}
          >
            <Tabs.Trigger value="board">
              <span className={styles.tabTriggerContent}>
                Board
                {runningAgentCount > 0 && (
                  <Badge
                    tone="accent"
                    title={`${runningAgentCount} running agent${
                      runningAgentCount === 1 ? "" : "s"
                    }`}
                  >
                    {runningAgentCount}
                  </Badge>
                )}
                {waitingPlannerCount > 0 && (
                  <Badge
                    tone="warning"
                    title={`${waitingPlannerCount} planner${
                      waitingPlannerCount === 1 ? "" : "s"
                    } waiting for input`}
                  >
                    {waitingPlannerCount}
                  </Badge>
                )}
              </span>
            </Tabs.Trigger>
            <Tabs.Trigger value="chat">Chat</Tabs.Trigger>
            <Tabs.Trigger value="memories">Memories</Tabs.Trigger>
            <Tabs.Trigger value="documents">Documents</Tabs.Trigger>
          </Tabs.List>
          <div className={styles.headerActionsPanel}>
            <button
              type="button"
              className={styles.headerActionButton}
              onClick={handleNewPlanner}
              disabled={isCreatingPlanner}
              aria-label="New task planner"
              title="New task planner"
            >
              <PlusIcon />
              <Text size="1">Planner</Text>
            </button>
            <NewChatModeButton
              disabled={isCreatingPlanner}
              onCreate={createTaskChat}
            />
            <ChatSwitcher
              label={chatLabel}
              plannerChats={plannerChats}
              cards={board.cards}
              activeChat={activeChat}
              linkedCardsByPlanner={linkedCardsByPlanner}
              onSelectPlanner={handleSelectPlanner}
              onRemovePlanner={handleRemovePlanner}
              onSelectAgent={handleSelectAgent}
            />
          </div>
        </div>
        <Box className={styles.chatContent}>
          {workspaceTab === "board" ? (
            <Box className={styles.workspaceTabContent}>
              <div className={styles.boardTabLayout}>
                <BoardRail
                  plannerChats={plannerChats}
                  cards={board.cards}
                  activeChat={activeChat}
                  linkedCardsByPlanner={linkedCardsByPlanner}
                  onSelectPlanner={handleSelectPlanner}
                  onRemovePlanner={handleRemovePlanner}
                  onSelectAgent={handleSelectAgent}
                />
                <Box className={styles.boardArea}>
                  <KanbanBoard
                    board={board}
                    onCardClick={handleCardClick}
                    onAgentClick={handleCardAgentClick}
                  />
                </Box>
              </div>
            </Box>
          ) : workspaceTab === "chat" ? (
            <Box className={styles.workspaceTabContent}>
              {activeChat ? (
                hasActiveChatRuntime ? (
                  <InternalLinkProvider onInternalLink={handleInternalLink}>
                    <Chat
                      host={config.host}
                      tabbed={false}
                      backFromChat={handleBack}
                      chatId={activeChat.chatId}
                    />
                  </InternalLinkProvider>
                ) : (
                  <Flex
                    align="center"
                    justify="center"
                    className={styles.fullHeightEmptyState}
                  >
                    <Text color="gray">Loading chat…</Text>
                  </Flex>
                )
              ) : (
                <Flex
                  align="center"
                  justify="center"
                  className={styles.fullHeightEmptyState}
                >
                  <Text color="gray">Create a planner chat to get started</Text>
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
      </Tabs>

      <Dialog
        open={Boolean(selectedCard)}
        onOpenChange={(open) => {
          if (!open) setSelectedCardId(null);
        }}
      >
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
      </Dialog>

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

      <Dialog
        open={Boolean(deleteTarget)}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTargetId(null);
            setDeleteTargetWorktree(null);
          }
        }}
      >
        <Dialog.Content
          className={styles.deleteWorktreeDialog}
          maxWidth="420px"
        >
          <div className={styles.deleteWorktreeRoot}>
            <Dialog.Title>Delete worktree</Dialog.Title>
            <Dialog.Description>
              Delete or discard this task agent worktree from disk.
            </Dialog.Description>
            <div className={styles.deleteWorktreeBody}>
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
              <Checkbox
                checked={deleteBranch}
                onCheckedChange={(checked) => setDeleteBranch(checked === true)}
                disabled={deleteWorktreeState.isLoading}
              >
                Delete git branch too
              </Checkbox>
            </div>
            <div className={styles.deleteWorktreeActions}>
              <Dialog.Close asChild>
                <Button
                  type="button"
                  variant="soft"
                  disabled={deleteWorktreeState.isLoading}
                >
                  Cancel
                </Button>
              </Dialog.Close>
              <Button
                type="button"
                variant="danger"
                disabled={!deleteTarget || deleteWorktreeState.isLoading}
                loading={deleteWorktreeState.isLoading}
                onClick={() => void handleConfirmDeleteCardWorktree()}
              >
                {deleteWorktreeState.isLoading
                  ? "Deleting..."
                  : "Delete worktree"}
              </Button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog>

      {notification && (
        <Box
          role="status"
          aria-live="polite"
          className={styles.notificationToast}
        >
          <Text size="2">{notification}</Text>
        </Box>
      )}
    </Box>
  );
};
