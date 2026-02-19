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
  if (sessionState === "generating" || sessionState === "executing_tools") {
    return "in_progress";
  }
  if (
    sessionState === "paused" ||
    sessionState === "waiting_ide" ||
    sessionState === "waiting_user_input"
  ) {
    return "needs_attention";
  }
  if (sessionState === "completed") {
    return "completed";
  }
  if (sessionState === "error") {
    return "error";
  }
  return "idle";
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
  if (sessionState === "generating" || sessionState === "executing_tools") {
    return "In progress...";
  }
  if (sessionState === "waiting_user_input") {
    return "Waiting for your answer";
  }
  if (sessionState === "paused" || sessionState === "waiting_ide") {
    return "Needs your attention";
  }
  if (sessionState === "completed") {
    return "Task completed";
  }
  if (sessionState === "error") {
    return "An error occurred";
  }
  return "Idle";
}
