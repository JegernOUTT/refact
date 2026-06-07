import type { StatusDotState } from "../components/StatusDot";
import type { TaskMeta } from "../services/refact/tasks";
export type SessionState = "idle" | "generating" | "executing_tools" | "paused" | "waiting_ide" | "waiting_user_input" | "completed" | "error";
export declare function getStatusFromSessionState(sessionState?: string | null): StatusDotState;
export declare function getTaskStatusDotState(task: TaskMeta): StatusDotState;
export declare function getStatusTooltip(sessionState?: string | null): string;
