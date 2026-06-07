export type LegacyStatusDotState =
  | "idle"
  | "in_progress"
  | "needs_attention"
  | "error"
  | "completed";

export type StatusDotStatus =
  | LegacyStatusDotState
  | "running"
  | "success"
  | "warning"
  | "paused";

export type StatusDotTone =
  | "muted"
  | "accent"
  | "success"
  | "warning"
  | "danger";

export const STATUS_DOT_STATUS_TONE: Record<StatusDotStatus, StatusDotTone> = {
  idle: "muted",
  in_progress: "accent",
  needs_attention: "warning",
  error: "danger",
  completed: "success",
  running: "accent",
  success: "success",
  warning: "warning",
  paused: "muted",
};
