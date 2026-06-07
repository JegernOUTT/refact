import type { AnimType, ConductorGoal, GoalStatus, MoodType } from "./types";

export type ConductorDashboardState =
  | "conducting"
  | "waiting_human"
  | "blocker"
  | "abandoned"
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
  abandoned: 85,
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
  abandoned: {
    state: "abandoned",
    label: "Abandoned",
    emoji: "🗑️",
    mood: "concerned",
    animationType: "think",
    tone: "warning",
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

function statusToState(
  status: GoalStatus,
  openQuestionCount: number,
): ConductorDashboardState {
  switch (status) {
    case "active":
    case "proposed":
      return "conducting";
    case "paused":
      return openQuestionCount > 0 ? "waiting_human" : "blocker";
    case "escalated":
      return "escalated";
    case "abandoned":
      return "abandoned";
    case "done":
      return "done";
  }
}

export function goalToConductorState(
  goal: ConductorGoal,
): ConductorDashboardState {
  const state = statusToState(goal.status, goal.summary.open_question_count);
  if (state === "escalated" || state === "abandoned") return state;
  if ((goal.summary.surgery_memo_count ?? 0) > 0) return "surgery";
  return state;
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
