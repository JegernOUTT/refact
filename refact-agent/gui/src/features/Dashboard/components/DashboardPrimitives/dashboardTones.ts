import type { BadgeTone } from "../../../../components/ui";

export function dashboardToneFromMode(_mode: string): BadgeTone {
  return "accent";
}

export function dashboardToneFromTaskStatus(status: string): BadgeTone {
  if (status === "completed") return "success";
  if (status === "paused") return "warning";
  if (status === "abandoned") return "danger";
  if (status === "active" || status === "planning") return "accent";
  return "muted";
}
