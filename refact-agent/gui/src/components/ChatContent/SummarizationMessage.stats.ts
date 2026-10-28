import { getCompressionReportMetadata } from "../../services/refact/types";
import type {
  ChatCompressionReportMetadata,
  SummarizationMessage as SummarizationMessageType,
} from "../../services/refact/types";

export type StatCell = { label: string; value: string };

export function parseNumberStat(value: unknown, suffix = ""): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return `${value.toLocaleString()}${suffix}`;
}

export function sourceMessageCountFromReport(
  report: ChatCompressionReportMetadata,
): number | null {
  if (
    typeof report.source_message_count === "number" &&
    Number.isFinite(report.source_message_count)
  ) {
    return report.source_message_count;
  }
  if (Array.isArray(report.summarized_source_message_ids)) {
    return report.summarized_source_message_ids.length;
  }
  if (Array.isArray(report.source_message_ids)) {
    return report.source_message_ids.length;
  }
  return null;
}

export function isSourcePreservingReport(
  report: ChatCompressionReportMetadata | null,
): boolean {
  return report?.insert_mode === "source_preserving";
}

function statCell(label: string, value: string | null): StatCell[] {
  return value ? [{ label, value }] : [];
}

export function statsFromCompressionReportMetadata(
  message: SummarizationMessageType,
): StatCell[] | null {
  const report = getCompressionReportMetadata(message);
  if (!report) return null;

  const sourceMessageCount = sourceMessageCountFromReport(report);
  const sourcePreserving = isSourcePreservingReport(report);
  const stats: StatCell[] = [
    ...statCell(
      sourcePreserving ? "Messages summarized" : "Messages compressed",
      sourceMessageCount !== null ? sourceMessageCount.toLocaleString() : null,
    ),
    ...statCell("Tokens saved", parseNumberStat(report.estimated_tokens_saved)),
    ...statCell("Reduction", parseNumberStat(report.reduction_percent, "%")),
    ...statCell(
      sourcePreserving ? "Context files preserved" : "Context files removed",
      parseNumberStat(
        sourcePreserving
          ? report.preserved_context_file_count
          : report.context_files_removed,
      ),
    ),
    ...statCell(
      sourcePreserving ? "Tool outputs compressed" : "Tool outputs truncated",
      parseNumberStat(
        sourcePreserving
          ? report.compressed_tool_output_count
          : report.tool_results_truncated,
      ),
    ),
    ...statCell("Tokens before", parseNumberStat(report.tokens_before)),
    ...statCell("Tokens after", parseNumberStat(report.tokens_after)),
    ...statCell(
      "Context messages dropped",
      parseNumberStat(report.context_messages_dropped),
    ),
  ];
  return stats.length > 0 ? stats : null;
}

export function parseReactiveStats(content: string): StatCell[] | null {
  const STAT_PATTERNS: { key: string; label: string }[] = [
    { key: "Attempt", label: "Attempt" },
    {
      key: "Context file entries deduplicated",
      label: "Context files deduped",
    },
    { key: "Context files removed", label: "Context files removed" },
    { key: "Tool outputs truncated", label: "Tool outputs truncated" },
    { key: "Tokens before", label: "Tokens before" },
    { key: "Tokens after", label: "Tokens after" },
    { key: "Estimated tokens saved", label: "Tokens saved" },
    { key: "Reduction", label: "Reduction" },
  ];
  const lines = content.split("\n");
  const stats: StatCell[] = [];
  for (const { key, label } of STAT_PATTERNS) {
    const re = new RegExp(`^\\s*-\\s*${key}:\\s*(.+?)\\s*$`);
    for (const line of lines) {
      const m = re.exec(line);
      if (m && typeof m[1] === "string") {
        stats.push({ label, value: m[1] });
        break;
      }
    }
  }
  return stats.length > 0 ? stats : null;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function statsFromReconstructionMetrics(
  metrics: unknown,
): StatCell[] | null {
  if (!isRecord(metrics)) return null;
  const stats: StatCell[] = [
    ...statCell("Messages before", parseNumberStat(metrics.messages_before)),
    ...statCell("Messages after", parseNumberStat(metrics.messages_after)),
    ...statCell("Tokens before", parseNumberStat(metrics.tokens_before)),
    ...statCell("Tokens after", parseNumberStat(metrics.tokens_after)),
    ...statCell(
      "Tokens saved",
      parseNumberStat(metrics.estimated_tokens_saved),
    ),
    ...statCell("Reduction", parseNumberStat(metrics.reduction_percent, "%")),
  ];
  return stats.length > 0 ? stats : null;
}

export function reconstructionTriggerLabel(trigger: string): string {
  switch (trigger) {
    case "automatic":
      return "automatic at cap";
    case "overflow":
      return "after provider limit";
    case "manual":
      return "manual";
    default:
      return trigger;
  }
}

const THOUSAND = 1000;
const MILLION = 1_000_000;

export function compactTokens(value: number): string {
  const abs = Math.abs(value);
  if (abs >= MILLION) return `${(value / MILLION).toFixed(1)}M`;
  if (abs >= THOUSAND) return `${Math.round(value / THOUSAND)}k`;
  return String(Math.round(value));
}

export function compressionMeta(
  messages: string | null,
  tokensSaved: number | null,
): string | null {
  const parts = [
    messages,
    tokensSaved !== null && tokensSaved > 0
      ? `−${compactTokens(tokensSaved)} tokens`
      : null,
  ].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
