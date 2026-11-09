import type { BadgeTone } from "../../components/ui/Badge/Badge";
import type {
  Applicability,
  Assessment,
  Disposition,
  RunStatus,
  Severity,
} from "../../services/refact/review";
import { humanizeIdentifier } from "../../utils/displayNames";
import type { FileStatus } from "./model";

export const label = humanizeIdentifier;

export const SEVERITY_TONE: Record<Severity, BadgeTone> = {
  info: "muted",
  low: "default",
  medium: "warning",
  high: "danger",
  critical: "danger",
};

export const ASSESSMENT_TONE: Record<Assessment, BadgeTone> = {
  unassessed: "muted",
  supported: "success",
  disputed: "warning",
  refuted: "muted",
};

export const DISPOSITION_TONE: Record<Disposition, BadgeTone> = {
  open: "accent",
  repair_proposed: "warning",
  fixed: "success",
  risk_accepted: "muted",
  dismissed: "muted",
};

export const APPLICABILITY_TONE: Record<Applicability, BadgeTone> = {
  current: "success",
  needs_revalidation: "warning",
  historical: "muted",
  no_longer_applicable: "muted",
  unknown: "default",
};

export const RUN_TONE: Record<RunStatus, BadgeTone> = {
  queued: "muted",
  running: "accent",
  cancel_requested: "warning",
  completed: "success",
  partial: "warning",
  failed: "danger",
  cancelled: "muted",
  interrupted: "warning",
};

export const FILE_STATUS_LETTER: Record<FileStatus, string> = {
  added: "A",
  deleted: "D",
  modified: "M",
  unchanged: "·",
};

export function isRunActive(status: RunStatus): boolean {
  return (
    status === "queued" || status === "running" || status === "cancel_requested"
  );
}

export function formatTime(ms: number): string {
  return new Date(ms).toLocaleString();
}
