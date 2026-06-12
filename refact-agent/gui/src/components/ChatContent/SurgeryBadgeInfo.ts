export type SurgeryBadgeInfo = {
  label: string;
  detail: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function valueSummary(value: unknown): string {
  if (typeof value === "string") return value;
  if (isRecord(value)) {
    const reason = value.reason;
    if (typeof reason === "string" && reason.length > 0) return reason;
    const action = value.action;
    if (typeof action === "string" && action.length > 0) return action;
    return `metadata keys: ${Object.keys(value).slice(0, 4).join(", ")}`;
  }
  return "Buddy edited this transcript message";
}

export function getSurgeryBadgeInfo(
  extra: Record<string, unknown> | undefined,
): SurgeryBadgeInfo | null {
  if (!extra) return null;
  for (const key of ["conductor_surgery", "trajectory_surgery", "surgery"]) {
    if (key in extra) {
      return {
        label: "Buddy surgery",
        detail: valueSummary(extra[key]),
      };
    }
  }
  return null;
}
