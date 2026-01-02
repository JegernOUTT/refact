import React, { useCallback, useState, useEffect } from "react";
import { Flex, Box, Text, Button, Heading, Badge, Card } from "@radix-ui/themes";
import { ArrowLeftIcon, PlusIcon, PersonIcon, Cross2Icon } from "@radix-ui/react-icons";
import { ScrollArea } from "../../components/ScrollArea";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop } from "../Pages/pagesSlice";
import { KanbanBoard } from "./KanbanBoard";
import {
  useGetTaskQuery,
  useGetBoardQuery,
  useListTaskTrajectoriesQuery,
  BoardCard,
} from "../../services/refact/tasks";
import styles from "./Tasks.module.css";
import { Chat } from "../Chat";
import { selectConfig } from "../Config/configSlice";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import { openTask, addPlannerChat, removePlannerChat, selectOpenTasksFromRoot } from "./tasksSlice";
import { selectThreadById } from "../Chat/Thread";
import { updateChatParams } from "../../services/refact/chatCommands";
import { InternalLinkProvider, parseRefactLink } from "../../contexts/InternalLinkContext";

type ActiveChat =
  | { type: "orchestrator" }
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string };

interface PlannerPanelProps {
  taskId: string;
  plannerChats: string[];
  activeChat: ActiveChat;
  onNewPlanner: () => void;
  onSelectPlanner: (chatId: string) => void;
  onRemovePlanner: (chatId: string) => void;
}

interface PlannerItemProps {
  chatId: string;
  index: number;
  isActive: boolean;
  onSelect: () => void;
  onRemove: () => void;
}

const PlannerItem: React.FC<PlannerItemProps> = ({ chatId, index, isActive, onSelect, onRemove }) => {
  const thread = useAppSelector((state) => selectThreadById(state, chatId));
  const title = thread?.title;
  const hasGeneratedTitle = title && title !== "New Chat" && title.trim() !== "";
  const displayTitle = hasGeneratedTitle
    ? `Planner #${index + 1}: ${title}`
    : `Planner #${index + 1}`;

  return (
    <Box
      className={styles.panelItem}
      onClick={onSelect}
      style={{ background: isActive ? "var(--accent-4)" : undefined }}
    >
      <Badge size="1" color="violet">📋</Badge>
      <Text size="1" style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{displayTitle}</Text>
      <Button
        size="1"
        variant="ghost"
        color="gray"
        onClick={(e) => { e.stopPropagation(); onRemove(); }}
      >
        <Cross2Icon />
      </Button>
    </Box>
  );
};

const PlannerPanel: React.FC<PlannerPanelProps> = ({
  plannerChats,
  activeChat,
  onNewPlanner,
  onSelectPlanner,
  onRemovePlanner,
}) => {
  return (
    <Box className={styles.panel}>
      <Flex className={styles.panelHeader}>
        <Text size="2" weight="medium">Planners</Text>
        <Button size="1" variant="ghost" onClick={onNewPlanner}>
          <PlusIcon />
        </Button>
      </Flex>
      <Box className={styles.panelContent}>
        <ScrollArea scrollbars="vertical">
          <Flex direction="column" gap="1">
            {plannerChats.length === 0 && (
              <Text size="1" color="gray">No planner chats yet</Text>
            )}
            {plannerChats.map((chatId, idx) => (
              <PlannerItem
                key={chatId}
                chatId={chatId}
                index={idx}
                isActive={activeChat.type === "planner" && activeChat.chatId === chatId}
                onSelect={() => onSelectPlanner(chatId)}
                onRemove={() => onRemovePlanner(chatId)}
              />
            ))}
          </Flex>
        </ScrollArea>
      </Box>
    </Box>
  );
};

interface AgentsPanelProps {
  taskId: string;
  cards: BoardCard[];
  activeChat: ActiveChat;
  onSelectAgent: (cardId: string, chatId: string) => void;
}

const AgentsPanel: React.FC<AgentsPanelProps> = ({ cards, activeChat, onSelectAgent }) => {
  const activeAgents = cards.filter(c => c.column === "doing" && c.agent_chat_id);
  const completedAgents = cards.filter(c => c.column === "done" && c.agent_chat_id);
  const failedAgents = cards.filter(c => c.column === "failed" && c.agent_chat_id);

  const total = completedAgents.length + failedAgents.length + activeAgents.length;
  const done = completedAgents.length;

  const renderAgentItem = (card: BoardCard, icon: string, color: "blue" | "green" | "red") => {
    const isActive = activeChat.type === "agent" && activeChat.cardId === card.id;
    return (
      <Box
        key={card.id}
        className={styles.panelItem}
        onClick={() => card.agent_chat_id && onSelectAgent(card.id, card.agent_chat_id)}
        style={{ background: isActive ? "var(--accent-4)" : undefined }}
      >
        <Badge size="1" color={color}>{icon}</Badge>
        <Text size="1" style={{ flex: 1 }}>{card.title}</Text>
      </Box>
    );
  };

  return (
    <Box className={styles.panel}>
      <Flex className={styles.panelHeader}>
        <Text size="2" weight="medium">Agents</Text>
        {total > 0 && (
          <Badge size="1" color="gray">{done}/{total} done</Badge>
        )}
      </Flex>
      <Box className={styles.panelContent}>
        <ScrollArea scrollbars="vertical">
          <Flex direction="column" gap="1">
            {activeAgents.length === 0 && completedAgents.length === 0 && failedAgents.length === 0 && (
              <Text size="1" color="gray">No agents yet</Text>
            )}
            {activeAgents.map(card => renderAgentItem(card, "🔄", "blue"))}
            {completedAgents.map(card => renderAgentItem(card, "✅", "green"))}
            {failedAgents.map(card => renderAgentItem(card, "❌", "red"))}
          </Flex>
        </ScrollArea>
      </Box>
    </Box>
  );
};

interface CardDetailProps {
  card: BoardCard;
  onClose: () => void;
}

const CardDetail: React.FC<CardDetailProps> = ({ card, onClose }) => {
  return (
    <Box className={styles.cardDetailOverlay} onClick={onClose}>
      <Card className={styles.cardDetail} onClick={e => e.stopPropagation()}>
        <Flex direction="column" gap="3">
          <Flex justify="between" align="center">
            <Heading size="3">{card.title}</Heading>
            <Badge color={card.column === "done" ? "green" : card.column === "failed" ? "red" : "blue"}>
              {card.column}
            </Badge>
          </Flex>

          {card.depends_on.length > 0 && (
            <Box>
              <Text size="2" weight="medium" color="gray">Dependencies</Text>
              <Flex gap="1" mt="1">
                {card.depends_on.map(dep => (
                  <Badge key={dep} size="1" variant="soft">{dep}</Badge>
                ))}
              </Flex>
            </Box>
          )}

          {card.instructions && (
            <Box>
              <Text size="2" weight="medium" color="gray">Instructions</Text>
              <Box
                p="2"
                mt="1"
                style={{ background: "var(--gray-2)", borderRadius: "var(--radius-2)", whiteSpace: "pre-wrap" }}
              >
                <Text size="2">{card.instructions}</Text>
              </Box>
            </Box>
          )}

          {card.final_report && (
            <Box>
              <Text size="2" weight="medium" color="gray">Final Report</Text>
              <Box
                p="2"
                mt="1"
                style={{ background: "var(--green-2)", borderRadius: "var(--radius-2)", whiteSpace: "pre-wrap" }}
              >
                <Text size="2">{card.final_report}</Text>
              </Box>
            </Box>
          )}

          {card.status_updates.length > 0 && (
            <Box>
              <Text size="2" weight="medium" color="gray">Updates</Text>
              <Flex direction="column" gap="1" mt="1">
                {card.status_updates.map((update, i) => (
                  <Text key={i} size="1" color="gray">
                    {new Date(update.timestamp).toLocaleString()}: {update.message}
                  </Text>
                ))}
              </Flex>
            </Box>
          )}

          <Flex justify="end">
            <Button variant="soft" onClick={onClose}>Close</Button>
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
    pollingInterval: 2000,
  });
  const { data: board, isLoading: boardLoading } = useGetBoardQuery(taskId, {
    pollingInterval: 2000,
  });
  const { data: savedPlanners } = useListTaskTrajectoriesQuery({ taskId, role: "planner" });
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const currentTaskUI = openTasks.find((t) => t.id === taskId);
  const plannerChats = currentTaskUI?.plannerChats ?? [];
  const [selectedCard, setSelectedCard] = useState<BoardCard | null>(null);
  const [activeChat, setActiveChat] = useState<ActiveChat>({ type: "orchestrator" });
  const plannerCountRef = React.useRef(plannerChats.length);
  const plannersRestoredRef = React.useRef(false);

  const orchestratorChatId = `orch-${taskId}`;

  // Open task tab when task data is available
  useEffect(() => {
    if (task) {
      dispatch(openTask({ id: taskId, name: task.name }));
    }
  }, [dispatch, taskId, task]);

  // Initialize orchestrator chat (separate effect to avoid re-running on task change)
  useEffect(() => {
    dispatch(createChatWithId({
      id: orchestratorChatId,
      title: `Orchestrator`,
      isTaskChat: true,
      mode: "TASK_ORCHESTRATOR",
      taskMeta: { task_id: taskId, role: "orchestrator" },
    }));
    dispatch(switchToThread({ id: orchestratorChatId, openTab: false }));
    void updateChatParams(
      orchestratorChatId,
      { mode: "TASK_ORCHESTRATOR", task_meta: { task_id: taskId, role: "orchestrator" } },
      config.lspPort,
    );
  }, [dispatch, orchestratorChatId, taskId, config.lspPort]);

  useEffect(() => {
    if (!savedPlanners || plannersRestoredRef.current) return;
    plannersRestoredRef.current = true;

    for (const chatId of savedPlanners) {
      if (plannerChats.includes(chatId)) continue;

      dispatch(createChatWithId({
        id: chatId,
        title: "",
        isTaskChat: true,
        mode: "TASK_PLANNER",
        taskMeta: { task_id: taskId, role: "planner" },
      }));
      dispatch(addPlannerChat({ taskId, chatId }));

      const match = chatId.match(/-(\d+)$/);
      if (match) {
        const num = parseInt(match[1], 10);
        if (num > plannerCountRef.current) {
          plannerCountRef.current = num;
        }
      }
    }

    dispatch(switchToThread({ id: orchestratorChatId, openTab: false }));
  }, [dispatch, taskId, savedPlanners, plannerChats, orchestratorChatId]);

  // Switch chat when activeChat changes
  useEffect(() => {
    let chatId: string;
    if (activeChat.type === "orchestrator") {
      chatId = orchestratorChatId;
    } else if (activeChat.type === "planner") {
      chatId = activeChat.chatId;
    } else {
      chatId = activeChat.chatId;
    }
    dispatch(switchToThread({ id: chatId, openTab: false }));
  }, [dispatch, activeChat, orchestratorChatId]);

  const handleBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleCardClick = useCallback((card: BoardCard) => {
    setSelectedCard(card);
  }, []);

  const handleNewPlanner = useCallback(() => {
    plannerCountRef.current += 1;
    const newChatId = `planner-${taskId}-${plannerCountRef.current}`;
    dispatch(createChatWithId({
      id: newChatId,
      title: "",
      isTaskChat: true,
      mode: "TASK_PLANNER",
      taskMeta: { task_id: taskId, role: "planner" },
    }));
    dispatch(addPlannerChat({ taskId, chatId: newChatId }));
    setActiveChat({ type: "planner", chatId: newChatId });
    void updateChatParams(
      newChatId,
      { mode: "TASK_PLANNER", task_meta: { task_id: taskId, role: "planner" } },
      config.lspPort,
    );
  }, [dispatch, taskId, config.lspPort]);

  const handleRemovePlanner = useCallback((chatId: string) => {
    dispatch(removePlannerChat({ taskId, chatId }));
    if (activeChat.type === "planner" && activeChat.chatId === chatId) {
      setActiveChat({ type: "orchestrator" });
    }
  }, [dispatch, taskId, activeChat]);

  const handleSelectPlanner = useCallback((chatId: string) => {
    setActiveChat({ type: "planner", chatId });
  }, []);

  const handleSelectAgent = useCallback((cardId: string, chatId: string) => {
    const card = board?.cards.find(c => c.id === cardId);
    const cardTitle = card?.title ?? `Card ${cardId}`;

    dispatch(createChatWithId({
      id: chatId,
      title: `Agent: ${cardTitle}`,
      isTaskChat: true,
      mode: "TASK_AGENT",
      taskMeta: { task_id: taskId, role: "agents" },
    }));

    setActiveChat({ type: "agent", cardId, chatId });
  }, [board, taskId, dispatch]);

  const handleSwitchToOrchestrator = useCallback(() => {
    setActiveChat({ type: "orchestrator" });
  }, []);

  const handleInternalLink = useCallback((url: string): boolean => {
    const parsed = parseRefactLink(url);
    if (!parsed) return false;

    if (parsed.type === "chat") {
      const chatId = parsed.id;
      const card = board?.cards.find(c => c.agent_chat_id === chatId);

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

      dispatch(createChatWithId({
        id: chatId,
        title: `Agent: ${cardTitle}`,
        isTaskChat: true,
        mode: "TASK_AGENT",
        taskMeta: { task_id: taskId, role: "agents" },
      }));

      setActiveChat({ type: "agent", cardId, chatId });
      return true;
    }

    return false;
  }, [board, taskId, dispatch]);

  if (taskLoading || boardLoading || !task || !board) {
    return (
      <Flex align="center" justify="center" style={{ height: "100%" }}>
        <Text color="gray">Loading task...</Text>
      </Flex>
    );
  }

  const chatLabel = activeChat.type === "orchestrator"
    ? "Orchestrator"
    : activeChat.type === "planner"
      ? `Planner`
      : `Agent: ${board.cards.find(c => c.id === activeChat.cardId)?.title ?? ""}`;

  return (
    <Box className={styles.taskWorkspace}>
      <Flex className={styles.taskHeader} justify="between" align="center">
        <Flex align="center" gap="3">
          <Button variant="ghost" size="1" onClick={handleBack}>
            <ArrowLeftIcon />
          </Button>
          <Heading size="4">{task.name}</Heading>
          <Badge color={task.status === "active" ? "blue" : task.status === "completed" ? "green" : "gray"}>
            {task.status}
          </Badge>
        </Flex>
        <Text size="1" color="gray">
          {task.cards_done}/{task.cards_total} done
          {task.cards_failed > 0 && ` • ${task.cards_failed} failed`}
        </Text>
      </Flex>

      <Box className={styles.boardSection}>
        <KanbanBoard board={board} onCardClick={handleCardClick} />
      </Box>

      <Flex className={styles.panelsSection}>
        <PlannerPanel
          taskId={taskId}
          plannerChats={plannerChats}
          activeChat={activeChat}
          onNewPlanner={handleNewPlanner}
          onSelectPlanner={handleSelectPlanner}
          onRemovePlanner={handleRemovePlanner}
        />
        <AgentsPanel
          taskId={taskId}
          cards={board.cards}
          activeChat={activeChat}
          onSelectAgent={handleSelectAgent}
        />
      </Flex>

      <Box className={styles.chatSection}>
        <Flex className={styles.chatHeader} align="center" gap="2" px="3" py="2">
          <PersonIcon />
          <Text size="2" weight="medium">{chatLabel}</Text>
          {activeChat.type !== "orchestrator" && (
            <Button size="1" variant="soft" ml="auto" onClick={handleSwitchToOrchestrator}>
              ← Back to Orchestrator
            </Button>
          )}
        </Flex>
        <Box className={styles.chatContent}>
          <InternalLinkProvider onInternalLink={handleInternalLink}>
            <Chat
              host={config.host}
              tabbed={false}
              backFromChat={handleBack}
            />
          </InternalLinkProvider>
        </Box>
      </Box>

      {selectedCard && (
        <CardDetail card={selectedCard} onClose={() => setSelectedCard(null)} />
      )}
    </Box>
  );
};
