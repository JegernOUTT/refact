import type { ChatActivity } from "../../../services/refact/trajectories";

export type ActiveChatReason =
  | "streaming"
  | "working"
  | "recovering"
  | "needs_attention"
  | "scheduled"
  | "schedule_paused";

export type ActiveChatEntry = {
  id: string;
  title: string;
  updatedAtMs: number;
  reasons: ActiveChatReason[];
};

export const ACTIVE_CHAT_REASON_LABEL: Record<ActiveChatReason, string> = {
  streaming: "Streaming",
  working: "Working",
  recovering: "Recovering",
  needs_attention: "Needs attention",
  scheduled: "Scheduled",
  schedule_paused: "Schedule paused",
};

const REASON_RANK: Record<ActiveChatReason, number> = {
  needs_attention: 0,
  recovering: 1,
  streaming: 2,
  working: 3,
  scheduled: 4,
  schedule_paused: 5,
};

export const activeChatReasons = (chat: ChatActivity): ActiveChatReason[] => {
  const reasons: ActiveChatReason[] = [];
  if (chat.recovery_state === "needs_action") reasons.push("needs_attention");
  if (chat.recovery_state === "exhausted") reasons.push("needs_attention");
  if (
    chat.recovery_state === "waiting" ||
    chat.recovery_state === "attempting"
  ) {
    reasons.push("recovering");
  }
  if (chat.session_state === "generating") reasons.push("streaming");
  if (chat.session_state === "executing_tools") reasons.push("working");
  const pausedSchedules = Math.max(0, chat.paused_schedule_count);
  const activeSchedules = Math.max(0, chat.schedule_count - pausedSchedules);
  if (activeSchedules > 0) reasons.push("scheduled");
  if (pausedSchedules > 0) reasons.push("schedule_paused");
  return reasons;
};

const leadRank = (entry: ActiveChatEntry): number =>
  Math.min(...entry.reasons.map((reason) => REASON_RANK[reason]));

export const buildActiveChatEntries = (
  chats: readonly ChatActivity[],
): ActiveChatEntry[] => {
  const byId = new Map<string, ActiveChatEntry>();
  for (const chat of chats) {
    const reasons = activeChatReasons(chat);
    if (reasons.length === 0) continue;
    const existing = byId.get(chat.id);
    if (existing && existing.updatedAtMs >= chat.updated_at_ms) continue;
    byId.set(chat.id, {
      id: chat.id,
      title: chat.title.trim() || "Untitled chat",
      updatedAtMs: chat.updated_at_ms,
      reasons,
    });
  }
  return [...byId.values()].sort(
    (left, right) =>
      leadRank(left) - leadRank(right) ||
      right.updatedAtMs - left.updatedAtMs ||
      left.id.localeCompare(right.id),
  );
};

export const activeChatsNeedAttention = (
  entries: readonly ActiveChatEntry[],
): boolean =>
  entries.some((entry) => entry.reasons.includes("needs_attention"));
