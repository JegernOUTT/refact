import { PLAN_SYNTHESIS_SEPARATOR } from "../../../features/Chat/Thread/selectors";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringListFromValue(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .filter((item): item is string => typeof item === "string")
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  }

  if (typeof value === "string") {
    return value
      .split(",")
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  }

  return [];
}

function normalizeSinglePlanContent(content: string): string {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{")) return content;

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return content;
  }

  if (!isRecord(parsed) || parsed.type !== "task_done") return content;

  const summary =
    typeof parsed.summary === "string" ? parsed.summary.trim() : "";
  const report = typeof parsed.report === "string" ? parsed.report.trim() : "";
  const filesChanged = stringListFromValue(parsed.files_changed);
  const parts: string[] = [];

  if (summary.length > 0 && report.length > 0 && !report.startsWith(summary)) {
    parts.push(`**${summary}**`);
  }

  if (report.length > 0) {
    parts.push(report);
  } else if (summary.length > 0) {
    parts.push(summary);
  }

  if (filesChanged.length > 0) {
    parts.push(
      [
        "**Files changed:**",
        ...filesChanged.map((file) => `- \`${file}\``),
      ].join("\n"),
    );
  }

  return parts.length > 0 ? parts.join("\n\n") : content;
}

export function normalizePlanContent(content: string): string {
  const separatorIndex = content.indexOf(PLAN_SYNTHESIS_SEPARATOR);
  if (separatorIndex === -1) return normalizeSinglePlanContent(content);

  const base = content.slice(0, separatorIndex);
  const updates = content.slice(
    separatorIndex + PLAN_SYNTHESIS_SEPARATOR.length,
  );
  return normalizeSinglePlanContent(base) + PLAN_SYNTHESIS_SEPARATOR + updates;
}
