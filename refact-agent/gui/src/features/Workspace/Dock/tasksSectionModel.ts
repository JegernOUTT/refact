import type {
  BoardCard,
  TaskBoard,
  TaskMeta,
} from "../../../services/refact/tasks";

const AGENT_STUCK_AFTER_MS = 20 * 60 * 1000;

export type TaskDockAgentStatus = "running" | "stuck" | "failed" | "done";

export type TaskDockEntry = {
  cardId: string;
  taskId: string;
  taskName: string;
  title: string;
  columnLabel: string;
  agentStatus: TaskDockAgentStatus;
  recencyAt: number;
};

const attentionRank = (status: TaskDockAgentStatus): number =>
  status === "stuck" || status === "failed" ? 0 : 1;

export const sortTaskDockEntries = (
  entries: TaskDockEntry[],
): TaskDockEntry[] =>
  [...entries].sort((left, right) => {
    const attentionDifference =
      attentionRank(left.agentStatus) - attentionRank(right.agentStatus);
    if (attentionDifference !== 0) return attentionDifference;
    return right.recencyAt - left.recencyAt;
  });

const parseTimestamp = (value?: string | null): number => {
  if (!value) return 0;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : 0;
};

const cardRecency = (card: BoardCard): number =>
  Math.max(
    parseTimestamp(card.last_heartbeat_at),
    parseTimestamp(card.completed_at),
    parseTimestamp(card.started_at),
    parseTimestamp(card.created_at),
    ...card.status_updates.map((update) => parseTimestamp(update.timestamp)),
  );

const agentStatusForCard = (
  card: BoardCard,
  nowMs: number,
): TaskDockAgentStatus | null => {
  if (!card.assignee && !card.agent_chat_id) return null;
  if (card.column === "failed") return "failed";
  if (card.column === "done") return "done";
  if (card.column !== "doing") return null;

  const heartbeatAt = parseTimestamp(card.last_heartbeat_at);
  return heartbeatAt > 0 && nowMs - heartbeatAt >= AGENT_STUCK_AFTER_MS
    ? "stuck"
    : "running";
};

const columnLabel = (board: TaskBoard, columnId: string): string => {
  const title = board.columns.find((column) => column.id === columnId)?.title;
  if (title) return title;
  return columnId.replace(/[_-]+/g, " ");
};

export const buildTaskDockEntries = (
  tasks: TaskMeta[],
  boardsByTask: Readonly<Record<string, TaskBoard | undefined>>,
  nowMs: number,
): TaskDockEntry[] =>
  tasks.flatMap((task) => {
    const board = boardsByTask[task.id];
    if (!board) return [];
    return board.cards.flatMap((card) => {
      const agentStatus = agentStatusForCard(card, nowMs);
      if (!agentStatus) return [];
      return [
        {
          cardId: card.id,
          taskId: task.id,
          taskName: task.name,
          title: card.title,
          columnLabel: columnLabel(board, card.column),
          agentStatus,
          recencyAt: cardRecency(card),
        },
      ];
    });
  });
