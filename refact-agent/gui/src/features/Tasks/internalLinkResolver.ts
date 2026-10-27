import type { TaskBoard, BoardCard } from "../../services/refact/tasks";
import type { PlannerInfo } from "./tasksSlice";

export type ResolvedChatLink =
  | { kind: "planner"; chatId: string }
  | { kind: "agent"; cardId: string; chatId: string }
  | { kind: "unknown"; chatId: string };

export function resolveChatLink(
  chatId: string,
  plannerChats: Pick<PlannerInfo, "id">[],
  board: TaskBoard | null | undefined,
): ResolvedChatLink {
  if (plannerChats.some((planner) => planner.id === chatId)) {
    return { kind: "planner", chatId };
  }

  const card = board?.cards.find((c: BoardCard) => c.agent_chat_id === chatId);
  if (card) {
    return { kind: "agent", cardId: card.id, chatId };
  }

  return { kind: "unknown", chatId };
}
