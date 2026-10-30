import { activityFromSessionState, CHAT_ACTIVITY } from "./chatActivity";
import type { StatusDotState } from "../components/StatusDot";
import type { TaskMeta } from "../services/refact/tasks";

export type SessionState =
  | "idle"
  | "generating"
  | "executing_tools"
  | "paused"
  | "waiting_ide"
  | "waiting_user_input"
  | "completed"
  | "error";

export function getStatusFromSessionState(
  sessionState?: string | null,
): StatusDotState {
  return CHAT_ACTIVITY[activityFromSessionState(sessionState)].status;
}

export function getTaskStatusDotState(task: TaskMeta): StatusDotState {
  const plannerState = task.planner_session_state;

  if (plannerState === "generating" || plannerState === "executing_tools") {
    return "in_progress";
  }
  if (plannerState === "paused" || plannerState === "waiting_ide") {
    return "needs_attention";
  }
  if (plannerState === "error" || task.status === "abandoned") {
    return "error";
  }
  if (task.status === "completed") {
    return "completed";
  }
  if (task.agents_active > 0) {
    return "in_progress";
  }
  return "idle";
}

export function getStatusTooltip(sessionState?: string | null): string {
  return CHAT_ACTIVITY[activityFromSessionState(sessionState)].label;
}
