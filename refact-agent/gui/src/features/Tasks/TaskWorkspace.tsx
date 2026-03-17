import React, { useCallback, useState, useEffect, useMemo } from "react";
import {
  Flex,
  Box,
  Text,
  Button,
  Heading,
  Badge,
  Card,
} from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  PlusIcon,
  PersonIcon,
  Cross2Icon,
  ChevronDownIcon,
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
  BoardCard,
} from "../../services/refact/tasks";
import { ModelPickerPopover } from "../../components/ChatForm/ModelPickerPopover";
import { Markdown } from "../../components/Markdown";
import styles from "./Tasks.module.css";
import { Chat } from "../Chat";
import { selectConfig } from "../Config/configSlice";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import {
  openTask,
  addPlannerChat,
  removePlannerChat,
  selectOpenTasksFromRoot,
  setTaskActiveChat,
  selectTaskActiveChat,
  PlannerInfo,
} from "./tasksSlice";
import { selectThreadById } from "../Chat/Thread";
import { InternalLinkProvider } from "../../contexts/InternalLinkContext";
import { parseRefactLink } from "../../contexts/internalLinkUtils";

type ActiveChat =
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string }
  | null;

interface PlannerPanelProps {
  plannerChats: PlannerInfo[];
  activeChat: ActiveChat;
  activePlannerId: string | null;
  onNewPlanner: () => void;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
}

interface PlannerItemProps {
  planner: PlannerInfo;
  isSelected: boolean;
  isActive: boolean;
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

const PlannerItem: React.FC<PlannerItemProps> = ({
  planner,
  isSelected,
  isActive,
  onSelect,
  onRemove,
}) => {
  const thread = useAppSelector((state) => selectThreadById(state, planner.id));
  const title = thread?.title ?? planner.title;
  const hasGeneratedTitle =
    title && title !== "New Chat" && title.trim() !== "";
  const displayTitle = hasGeneratedTitle
    ? title
    : formatPlannerDate(planner.createdAt);

  return (
    <Box
      className={`${styles.panelItem} ${
        isSelected ? styles.panelItemSelected : ""
      }`}
      onClick={onSelect}
    >
      <Flex align="center" gap="1" className={styles.panelItemLead}>
        {isActive && (
          <Badge size="1" color="green" radius="full">
            ●
          </Badge>
        )}
        <Badge size="1" color="violet">
          📋
        </Badge>
      </Flex>
      <Box className={styles.panelItemContent}>
        <Text size="1" className={styles.panelItemTitle}>
          {displayTitle}
        </Text>
      </Box>
      <Button
        size="1"
        variant="ghost"
        color="gray"
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
      >
        <Cross2Icon />
      </Button>
    </Box>
  );
};

const PlannerPanel: React.FC<PlannerPanelProps> = ({
  plannerChats,
  activeChat,
  activePlannerId,
  onNewPlanner,
  onSelectPlanner,
  onRemovePlanner,
}) => {
  return (
    <Box className={styles.panel}>
      <Flex className={styles.panelHeader}>
        <Text size="2" weight="medium">
          Planners
        </Text>
        <Button size="1" variant="ghost" onClick={onNewPlanner}>
          <PlusIcon />
        </Button>
      </Flex>
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
                  isActive={planner.id === activePlannerId}
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

  const total =
    completedAgents.length + failedAgents.length + activeAgents.length;
  const done = completedAgents.length;

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
    <Box className={styles.panel}>
      <Flex className={styles.panelHeader}>
        <Text size="2" weight="medium">
          Agents
        </Text>
        {total > 0 && (
          <Badge size="1" color="gray">
            {done}/{total} done
          </Badge>
        )}
      </Flex>
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
        <Flex p="2" style={{ borderTop: "1px solid var(--gray-4)" }}>
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
  card: BoardCard;
  onClose: () => void;
  onInternalLink?: (url: string) => boolean;
}

const CardDetail: React.FC<CardDetailProps> = ({
  card,
  onClose,
  onInternalLink,
}) => {
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
  const config = useAppSelector(selectConfig);
  const { data: task, isLoading: taskLoading } = useGetTaskQuery(taskId, {
    pollingInterval: 0,
  });
  const { data: board, isLoading: boardLoading } = useGetBoardQuery(taskId, {
    pollingInterval: 0,
  });
  const { data: savedPlanners } = useListTaskTrajectoriesQuery({
    taskId,
    role: "planner",
  });
  const [updateTaskMeta] = useUpdateTaskMetaMutation();
  const [createPlannerChat, { isLoading: isCreatingPlanner }] =
    useCreatePlannerChatMutation();
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const currentTaskUI = openTasks.find((t) => t.id === taskId);
  const plannerChats = useMemo(
    () => currentTaskUI?.plannerChats ?? [],
    [currentTaskUI?.plannerChats],
  );
  const activePlannerId = useMemo(() => {
    if (plannerChats.length === 0) return null;
    return plannerChats.reduce((latest, p) =>
      p.updatedAt > latest.updatedAt ? p : latest,
    ).id;
  }, [plannerChats]);
  const activeChat = useAppSelector((state) =>
    selectTaskActiveChat(state, taskId),
  );
  const [selectedCard, setSelectedCard] = useState<BoardCard | null>(null);
  const [notification, setNotification] = useState<string | null>(null);
  const [chatExpanded, setChatExpanded] = useState(false);
  const plannersRestoredRef = React.useRef(false);
  const prevTaskStatusRef = React.useRef<string | undefined>(undefined);

  useEffect(() => {
    if (task) {
      dispatch(openTask({ id: taskId, name: task.name }));
    }
  }, [dispatch, taskId, task]);

  useEffect(() => {
    if (!savedPlanners || plannersRestoredRef.current) return;
    plannersRestoredRef.current = true;

    for (const traj of savedPlanners) {
      if (plannerChats.some((p) => p.id === traj.id)) continue;

      dispatch(
        createChatWithId({
          id: traj.id,
          title: traj.title,
          isTaskChat: true,
          mode: "TASK_PLANNER",
          taskMeta: { task_id: taskId, role: "planner" },
        }),
      );
      dispatch(
        addPlannerChat({
          taskId,
          planner: {
            id: traj.id,
            title: traj.title,
            createdAt: traj.created_at,
            updatedAt: traj.updated_at,
            sessionState: traj.session_state,
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
  }, [dispatch, taskId, savedPlanners, plannerChats, activeChat]);

  useEffect(() => {
    if (
      activeChat?.type === "planner" &&
      !plannerChats.some((p) => p.id === activeChat.chatId)
    ) {
      if (activePlannerId) {
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "planner", chatId: activePlannerId },
          }),
        );
      } else {
        dispatch(setTaskActiveChat({ taskId, activeChat: null }));
      }
    }
  }, [activeChat, plannerChats, activePlannerId, dispatch, taskId]);

  useEffect(() => {
    if (activeChat?.type === "agent" && board) {
      const cardExists = board.cards.some((c) => c.id === activeChat.cardId);
      if (!cardExists) {
        if (activePlannerId) {
          dispatch(
            setTaskActiveChat({
              taskId,
              activeChat: { type: "planner", chatId: activePlannerId },
            }),
          );
        } else {
          dispatch(setTaskActiveChat({ taskId, activeChat: null }));
        }
      }
    }
  }, [activeChat, board, dispatch, taskId, activePlannerId]);
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
    setSelectedCard(card);
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
            taskMeta: { task_id: taskId, role: "planner" },
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
      .catch(() => undefined);
  }, [dispatch, taskId, createPlannerChat, isCreatingPlanner]);

  const handleRemovePlanner = useCallback(
    (chatId: string) => {
      dispatch(removePlannerChat({ taskId, chatId }));
      if (activeChat?.type === "planner" && activeChat.chatId === chatId) {
        const remaining = plannerChats.filter((p) => p.id !== chatId);
        if (remaining.length > 0) {
          const mostRecent = remaining.reduce((latest, p) =>
            p.updatedAt > latest.updatedAt ? p : latest,
          );
          dispatch(
            setTaskActiveChat({
              taskId,
              activeChat: { type: "planner", chatId: mostRecent.id },
            }),
          );
        } else {
          dispatch(setTaskActiveChat({ taskId, activeChat: null }));
        }
      }
    },
    [dispatch, taskId, activeChat, plannerChats],
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
          taskMeta: { task_id: taskId, role: "agents", card_id: cardId },
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
            taskMeta: { task_id: taskId, role: "agents", card_id: cardId },
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
    setChatExpanded((prev) => !prev);
  }, []);

  const handleModelChange = useCallback(
    (model: string) => {
      void updateTaskMeta({ taskId, defaultAgentModel: model });
    },
    [taskId, updateTaskMeta],
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

  const branchDisplay =
    activeChat?.type === "agent"
      ? board.cards.find((c) => c.id === activeChat.cardId)?.agent_branch ??
        task.base_branch ??
        "(unknown)"
      : task.base_branch ?? "(unknown)";

  return (
    <Box
      className={`${styles.taskWorkspace} ${
        chatExpanded ? styles.expanded : ""
      }`}
    >
      <Flex className={styles.taskHeader} justify="between" align="center">
        <Flex align="center" gap="3">
          <Button variant="ghost" size="1" onClick={handleBack}>
            <ArrowLeftIcon />
          </Button>
          <Heading size="4">{task.name}</Heading>
          <Badge
            color={
              task.status === "active"
                ? "blue"
                : task.status === "completed"
                  ? "green"
                  : "gray"
            }
          >
            {task.status}
          </Badge>
          <Badge color="gray">🌿 {branchDisplay}</Badge>
        </Flex>
        <Text size="1" color="gray">
          {task.cards_done}/{task.cards_total} done
          {task.cards_failed > 0 && ` • ${task.cards_failed} failed`}
        </Text>
      </Flex>

      {!chatExpanded && (
        <>
          <Box className={styles.boardSection}>
            <KanbanBoard board={board} onCardClick={handleCardClick} />
          </Box>

          <Flex className={styles.panelsSection}>
            <PlannerPanel
              plannerChats={plannerChats}
              activeChat={activeChat}
              activePlannerId={activePlannerId}
              onNewPlanner={handleNewPlanner}
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
        </>
      )}

      <Box className={styles.chatSection}>
        <Flex
          className={styles.chatHeader}
          align="center"
          gap="2"
          px="3"
          py="2"
          onClick={handleToggleChatExpanded}
          style={{ cursor: "pointer" }}
        >
          <ChevronDownIcon
            className={`${styles.chevron} ${
              chatExpanded ? styles.chevronExpanded : ""
            }`}
          />
          <PersonIcon />
          <Text size="2" weight="medium">
            {chatLabel}
          </Text>
        </Flex>
        <Box className={styles.chatContent}>
          {activeChat ? (
            <InternalLinkProvider onInternalLink={handleInternalLink}>
              <Chat
                host={config.host}
                tabbed={false}
                backFromChat={handleBack}
              />
            </InternalLinkProvider>
          ) : (
            <Flex align="center" justify="center" style={{ height: "100%" }}>
              <Text color="gray">Create a planner chat to get started</Text>
            </Flex>
          )}
        </Box>
      </Box>

      {selectedCard && (
        <CardDetail
          card={selectedCard}
          onClose={() => setSelectedCard(null)}
          onInternalLink={handleInternalLink}
        />
      )}

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
