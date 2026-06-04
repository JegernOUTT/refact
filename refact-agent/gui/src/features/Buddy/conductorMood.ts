import type {
  AnimType,
  ConductorGoal,
  ConductorMemo,
  GoalStatus,
  MoodType,
} from "./types";

export type ConductorDashboardState =
  | "conducting"
  | "waiting_human"
  | "blocker"
  | "merging"
  | "review"
  | "surgery"
  | "done"
  | "escalated";

export type ConductorMoodView = {
  state: ConductorDashboardState;
  label: string;
  emoji: string;
  mood: MoodType;
  animationType: AnimType;
  tone: "normal" | "info" | "warning" | "danger" | "success" | "purple";
};

const STATUS_PRIORITY: Record<ConductorDashboardState, number> = {
  escalated: 90,
  blocker: 80,
  waiting_human: 70,
  surgery: 65,
  merging: 60,
  review: 55,
  conducting: 40,
  done: 10,
};

const MOOD_VIEWS: Record<ConductorDashboardState, ConductorMoodView> = {
  conducting: {
    state: "conducting",
    label: "Conducting",
    emoji: "🎛️",
    mood: "focused",
    animationType: "work",
    tone: "info",
  },
  waiting_human: {
    state: "waiting_human",
    label: "Waiting on human",
    emoji: "🙋",
    mood: "curious",
    animationType: "perk",
    tone: "warning",
  },
  blocker: {
    state: "blocker",
    label: "Blocked",
    emoji: "🚧",
    mood: "alert",
    animationType: "shake",
    tone: "danger",
  },
  merging: {
    state: "merging",
    label: "Merging",
    emoji: "🔀",
    mood: "working",
    animationType: "work",
    tone: "purple",
  },
  review: {
    state: "review",
    label: "Reviewing",
    emoji: "🔎",
    mood: "thinking",
    animationType: "think",
    tone: "info",
  },
  surgery: {
    state: "surgery",
    label: "Surgery audit",
    emoji: "🩹",
    mood: "alert",
    animationType: "think",
    tone: "purple",
  },
  done: {
    state: "done",
    label: "Done",
    emoji: "✅",
    mood: "celebrate",
    animationType: "celebrate",
    tone: "success",
  },
  escalated: {
    state: "escalated",
    label: "Escalated",
    emoji: "🆘",
    mood: "alert",
    animationType: "shake",
    tone: "danger",
  },
};

function memoHas(memo: ConductorMemo, needle: string): boolean {
  return memo.content.toLowerCase().includes(needle);
}

function statusToState(status: GoalStatus): ConductorDashboardState {
  if (status === "waiting_for_human") return "waiting_human";
  if (status === "done") return "done";
  if (status === "failed" || status === "cancelled") return "blocker";
  if (status === "paused") return "blocker";
  return "conducting";
}

export function goalToConductorState(
  goal: ConductorGoal,
): ConductorDashboardState {
  const memos = goal.ledger.memos;
  if (memos.some((memo) => memo.kind === "escalation")) return "escalated";
  if (memos.some((memo) => memo.kind === "surgery")) return "surgery";
  if (memos.some((memo) => memoHas(memo, "merge"))) return "merging";
  if (memos.some((memo) => memoHas(memo, "review"))) return "review";
  return statusToState(goal.status);
}

export function conductorStateView(
  state: ConductorDashboardState,
): ConductorMoodView {
  return MOOD_VIEWS[state];
}

export function selectPrimaryConductorState(
  goals: ConductorGoal[],
): ConductorDashboardState | null {
  if (goals.length === 0) return null;
  const states = goals.map(goalToConductorState);
  states.sort((left, right) => STATUS_PRIORITY[right] - STATUS_PRIORITY[left]);
  return states[0] ?? null;
}

export function conductorMoodForGoals(
  goals: ConductorGoal[],
): ConductorMoodView | null {
  const state = selectPrimaryConductorState(goals);
  return state ? conductorStateView(state) : null;
}
