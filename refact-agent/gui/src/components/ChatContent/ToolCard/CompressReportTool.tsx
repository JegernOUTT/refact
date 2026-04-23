import React from "react";
import { BarChartIcon, ArchiveIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";

interface CompressReportToolProps {
  toolCall: ToolCall;
  toolType: "compress_chat_probe" | "compress_chat_apply";
}

function formatNumber(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return n.toString();
}

interface ProbeResult {
  type: "compress_chat_probe";
  messages_count: number;
  total_tokens: number;
  role_tokens: Record<string, number>;
  potential_gains: {
    duplicate_context_tokens: number;
    tool_output_tokens: number;
    memory_tokens: number;
    project_info_tokens: number;
  };
}

interface ApplyResult {
  type: "compress_chat_apply";
  before_message_count: number;
  after_message_count: number;
  before_tokens: number;
  after_tokens: number;
  context_files_dropped: number;
  context_messages_dropped: number;
  memories_dropped: number;
  tool_outputs_truncated: number;
  tool_outputs_dropped: number;
  project_info_dropped: number;
  dedup_context_files: number;
}

function extractProbeReport(content: string): ReportData | null {
  try {
    const raw = JSON.parse(content) as Record<string, unknown>;
    if (raw.type !== "compress_chat_probe") return null;
    const data = raw as unknown as ProbeResult;

    const roleLines = Object.entries(data.role_tokens)
      .map(([role, tokens]) => `| ${role} | ${formatNumber(tokens)} |`)
      .join("\n");

    const gains = data.potential_gains;
    const totalGains =
      gains.duplicate_context_tokens +
      gains.tool_output_tokens +
      gains.memory_tokens +
      gains.project_info_tokens;

    const lines: (string | null)[] = [
      `## Chat Analysis`,
      ``,
      `- **Messages**: ${data.messages_count}`,
      `- **Total tokens**: ~${formatNumber(data.total_tokens)}`,
      ``,
      `### Token Distribution`,
      `| Role | Tokens |`,
      `|------|--------|`,
      roleLines,
      ``,
      `### Potential Compression Gains (~${formatNumber(totalGains)} tokens)`,
      gains.duplicate_context_tokens > 0
        ? `- Duplicate context files: ~${formatNumber(
            gains.duplicate_context_tokens,
          )}`
        : null,
      gains.tool_output_tokens > 0
        ? `- Tool outputs: ~${formatNumber(gains.tool_output_tokens)}`
        : null,
      gains.memory_tokens > 0
        ? `- Memories: ~${formatNumber(gains.memory_tokens)}`
        : null,
      gains.project_info_tokens > 0
        ? `- Project info: ~${formatNumber(gains.project_info_tokens)}`
        : null,
    ];

    return {
      summary: `Chat analysis: ${data.messages_count} messages, ~${formatNumber(
        data.total_tokens,
      )} tokens`,
      markdown: lines.filter((l): l is string => l !== null).join("\n"),
    };
  } catch {
    return null;
  }
}

function extractApplyReport(content: string): ReportData | null {
  try {
    const raw = JSON.parse(content) as Record<string, unknown>;
    if (raw.type !== "compress_chat_apply") return null;
    const data = raw as unknown as ApplyResult;

    const saved = data.before_tokens - data.after_tokens;
    const actions: string[] = [];
    if (data.context_files_dropped > 0)
      actions.push(`- Context files dropped: ${data.context_files_dropped}`);
    if (data.context_messages_dropped > 0)
      actions.push(
        `- Context messages dropped: ${data.context_messages_dropped}`,
      );
    if (data.memories_dropped > 0)
      actions.push(`- Memories dropped: ${data.memories_dropped}`);
    if (data.tool_outputs_truncated > 0)
      actions.push(`- Tool outputs truncated: ${data.tool_outputs_truncated}`);
    if (data.tool_outputs_dropped > 0)
      actions.push(`- Tool outputs dropped: ${data.tool_outputs_dropped}`);
    if (data.project_info_dropped > 0)
      actions.push(`- Project info dropped: ${data.project_info_dropped}`);
    if (data.dedup_context_files > 0)
      actions.push(`- Deduplicated context files: ${data.dedup_context_files}`);

    const lines: (string | null)[] = [
      `## Compression Applied`,
      ``,
      `- **Messages**: ${data.before_message_count} → ${data.after_message_count}`,
      `- **Tokens**: ~${formatNumber(data.before_tokens)} → ~${formatNumber(
        data.after_tokens,
      )} (saved ~${formatNumber(saved)})`,
      ``,
      actions.length > 0 ? `### Actions\n${actions.join("\n")}` : null,
    ];

    return {
      summary: `Compressed: ${formatNumber(
        data.before_tokens,
      )} → ${formatNumber(data.after_tokens)} tokens`,
      markdown: lines.filter((l): l is string => l !== null).join("\n"),
    };
  } catch {
    return null;
  }
}

export const CompressReportTool: React.FC<CompressReportToolProps> = ({
  toolCall,
  toolType,
}) => {
  const isProbe = toolType === "compress_chat_probe";

  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={isProbe ? <BarChartIcon /> : <ArchiveIcon />}
      defaultSummary={isProbe ? "Analyze chat" : "Compress chat"}
      extractReport={isProbe ? extractProbeReport : extractApplyReport}
    />
  );
};

export default CompressReportTool;
