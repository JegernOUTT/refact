import type { StatusDotState } from "../components/StatusDot";

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
