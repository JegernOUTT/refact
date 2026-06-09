import type { LucideIcon } from "lucide-react";
import type { BadgeSize, BadgeTone, BadgeVariant } from "../Badge";

export type StatusBadgeTone = BadgeTone;
export type StatusBadgeSize = BadgeSize;
export type StatusBadgeVariant = BadgeVariant;
export type FileStatusBadgeStatus = "ADDED" | "MODIFIED" | "DELETED";
export type AgentStatusBadgeStatus = "running" | "success" | "error";
export type PriorityStatusBadgeStatus = "critical" | "high" | "medium" | "low";
export type CommonStatusBadgeStatus =
  | "idle"
  | "queued"
  | "paused"
  | "warning"
  | "blocked"
  | "completed";
export type StatusBadgeStatus =
  | FileStatusBadgeStatus
  | AgentStatusBadgeStatus
  | PriorityStatusBadgeStatus
  | CommonStatusBadgeStatus
  | (string & Record<never, never>);

export interface StatusBadgeRecipe {
  status: StatusBadgeStatus;
  tone: StatusBadgeTone;
  label: string;
  ariaLabel: string;
  pulse?: boolean;
}

export interface StatusBadgeProps
  extends Omit<React.ComponentPropsWithoutRef<"span">, "children" | "size"> {
  status: StatusBadgeStatus;
  tone?: StatusBadgeTone;
  label?: string;
  ariaLabel?: string;
  icon?: LucideIcon;
  size?: StatusBadgeSize;
  variant?: StatusBadgeVariant;
  pulse?: boolean;
}

const STATUS_RECIPES: Partial<Record<string, StatusBadgeRecipe>> = {
  ADDED: {
    status: "ADDED",
    tone: "success",
    label: "Added",
    ariaLabel: "Added file",
  },
  MODIFIED: {
    status: "MODIFIED",
    tone: "warning",
    label: "Modified",
    ariaLabel: "Modified file",
  },
  DELETED: {
    status: "DELETED",
    tone: "danger",
    label: "Deleted",
    ariaLabel: "Deleted file",
  },
  blocked: {
    status: "blocked",
    tone: "danger",
    label: "Blocked",
    ariaLabel: "Blocked",
  },
  completed: {
    status: "completed",
    tone: "success",
    label: "Completed",
    ariaLabel: "Completed",
  },
  critical: {
    status: "critical",
    tone: "danger",
    label: "Critical",
    ariaLabel: "Critical priority",
  },
  error: {
    status: "error",
    tone: "danger",
    label: "Error",
    ariaLabel: "Agent error",
  },
  high: {
    status: "high",
    tone: "warning",
    label: "High",
    ariaLabel: "High priority",
  },
  idle: {
    status: "idle",
    tone: "muted",
    label: "Idle",
    ariaLabel: "Idle",
  },
  low: {
    status: "low",
    tone: "muted",
    label: "Low",
    ariaLabel: "Low priority",
  },
  medium: {
    status: "medium",
    tone: "accent",
    label: "Medium",
    ariaLabel: "Medium priority",
  },
  paused: {
    status: "paused",
    tone: "muted",
    label: "Paused",
    ariaLabel: "Paused",
  },
  queued: {
    status: "queued",
    tone: "muted",
    label: "Queued",
    ariaLabel: "Queued",
  },
  running: {
    status: "running",
    tone: "accent",
    label: "Running",
    ariaLabel: "Agent running",
    pulse: true,
  },
  success: {
    status: "success",
    tone: "success",
    label: "Success",
    ariaLabel: "Agent success",
  },
  warning: {
    status: "warning",
    tone: "warning",
    label: "Warning",
    ariaLabel: "Warning",
  },
};

function humanizeStatus(status: StatusBadgeStatus): string {
  return String(status)
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

export function getStatusBadgeRecipe(
  status: StatusBadgeStatus,
): StatusBadgeRecipe {
  return (
    STATUS_RECIPES[status] ?? {
      status,
      tone: "muted",
      label: humanizeStatus(status),
      ariaLabel: humanizeStatus(status),
    }
  );
}

export function getFileStatusBadgeProps(
  status: FileStatusBadgeStatus,
): StatusBadgeRecipe {
  return getStatusBadgeRecipe(status);
}

export function getAgentStatusBadgeProps(
  status: AgentStatusBadgeStatus,
): StatusBadgeRecipe {
  return getStatusBadgeRecipe(status);
}

export function getPriorityStatusBadgeProps(
  status: PriorityStatusBadgeStatus,
): StatusBadgeRecipe {
  return getStatusBadgeRecipe(status);
}
