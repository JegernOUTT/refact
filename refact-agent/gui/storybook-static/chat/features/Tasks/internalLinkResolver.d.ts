import type { TaskBoard } from "../../services/refact/tasks";
import type { PlannerInfo } from "./tasksSlice";
/**
 * Pure resolution of a `refact://chat/<chat_id>` link in the context of a
 * task workspace. Returns the action the workspace should take.
 *
 * Resolution priority (matches T-179 spec):
 *   1. Planner chat in this task → activate planner
 *   2. Board card whose `agent_chat_id` matches → activate agent
 *   3. Legacy `agent-<cardId>-<suffix>` pattern with a known card → activate agent
 *   4. Otherwise → unknown (workspace should surface a notification)
 */
export type ResolvedChatLink = {
    kind: "planner";
    chatId: string;
} | {
    kind: "agent";
    cardId: string;
    chatId: string;
} | {
    kind: "unknown";
    chatId: string;
};
export declare function resolveChatLink(chatId: string, plannerChats: PlannerInfo[], board: TaskBoard | null | undefined): ResolvedChatLink;
