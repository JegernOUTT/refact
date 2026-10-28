import type { ReportData } from "./ReportToolCard";

export function extractSubagentReport(content: string): ReportData | null {
  const responseMarker = "## Response";
  const responseIndex = content.indexOf(responseMarker);
  if (!content.startsWith("# Subagent Result") || responseIndex === -1) {
    return null;
  }
  const taskMatch = content.match(/\*\*Task:\*\*\s*([^\n]+)/);
  const response = content.slice(responseIndex + responseMarker.length).trim();
  const task = taskMatch?.[1]?.trim();
  return {
    summary: task ? task : undefined,
    markdown: response || content,
  };
}
