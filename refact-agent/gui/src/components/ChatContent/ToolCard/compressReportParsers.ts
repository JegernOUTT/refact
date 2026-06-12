import type { ReportData } from "./ReportToolCard";

function formatNumber(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return n.toString();
}

interface ProbeResult {
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNumberRecord(value: unknown): value is Record<string, number> {
  return (
    isRecord(value) && Object.values(value).every((v) => typeof v === "number")
  );
}

function numberOrZero(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

// The engine emits `"type": "ctx_probe"` / `"ctx_apply"`; older persisted
// trajectories may carry the legacy `compress_chat_*` payload type strings.
const PROBE_TYPES: readonly string[] = ["ctx_probe", "compress_chat_probe"];
const APPLY_TYPES: readonly string[] = ["ctx_apply", "compress_chat_apply"];

export function extractProbeReport(content: string): ReportData | null {
  try {
    const raw = JSON.parse(content) as unknown;
    if (!isRecord(raw)) return null;
    if (typeof raw.type !== "string" || !PROBE_TYPES.includes(raw.type)) {
      return null;
    }
    if (
      typeof raw.messages_count !== "number" ||
      typeof raw.total_tokens !== "number" ||
      !isNumberRecord(raw.role_tokens) ||
      !isRecord(raw.potential_gains) ||
      typeof raw.potential_gains.duplicate_context_tokens !== "number" ||
      typeof raw.potential_gains.tool_output_tokens !== "number" ||
      typeof raw.potential_gains.memory_tokens !== "number" ||
      typeof raw.potential_gains.project_info_tokens !== "number"
    ) {
      return null;
    }

    const data: ProbeResult = {
      messages_count: raw.messages_count,
      total_tokens: raw.total_tokens,
      role_tokens: raw.role_tokens,
      potential_gains: {
        duplicate_context_tokens: raw.potential_gains.duplicate_context_tokens,
        tool_output_tokens: raw.potential_gains.tool_output_tokens,
        memory_tokens: raw.potential_gains.memory_tokens,
        project_info_tokens: raw.potential_gains.project_info_tokens,
      },
    };

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

export function extractApplyReport(content: string): ReportData | null {
  try {
    const raw = JSON.parse(content) as unknown;
    if (!isRecord(raw)) return null;
    if (typeof raw.type !== "string" || !APPLY_TYPES.includes(raw.type)) {
      return null;
    }
    if (
      typeof raw.before_message_count !== "number" ||
      typeof raw.after_message_count !== "number" ||
      typeof raw.before_tokens !== "number" ||
      typeof raw.after_tokens !== "number"
    ) {
      return null;
    }

    const data: ApplyResult = {
      before_message_count: raw.before_message_count,
      after_message_count: raw.after_message_count,
      before_tokens: raw.before_tokens,
      after_tokens: raw.after_tokens,
      context_files_dropped: numberOrZero(raw.context_files_dropped),
      context_messages_dropped: numberOrZero(raw.context_messages_dropped),
      memories_dropped: numberOrZero(raw.memories_dropped),
      tool_outputs_truncated: numberOrZero(raw.tool_outputs_truncated),
      tool_outputs_dropped: numberOrZero(raw.tool_outputs_dropped),
      project_info_dropped: numberOrZero(raw.project_info_dropped),
      dedup_context_files: numberOrZero(raw.dedup_context_files),
    };

    const saved = Math.max(0, data.before_tokens - data.after_tokens);
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
