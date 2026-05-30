import React, { useMemo, useState } from "react";
import { Box, Flex } from "@radix-ui/themes";
import type {
  ChatCompressionReportMetadata,
  SummarizationMessage as SummarizationMessageType,
  SummarizationTier,
} from "../../services/refact/types";
import { ToolMarkdown } from "../Markdown";
import styles from "./SummarizationMessage.module.css";

interface SummarizationMessageProps {
  message: SummarizationMessageType;
}

type TierMeta = {
  label: string;
  icon: string;
  badgeClass: string;
};

function metaForTier(tier?: SummarizationTier): TierMeta {
  switch (tier) {
    case "tier0_deterministic":
      return {
        label: "Deterministic compaction",
        icon: "🗜️",
        badgeClass: styles.tierBadgeTier0,
      };
    case "tier1_llm":
      return {
        label: "LLM summary",
        icon: "🧠",
        badgeClass: styles.tierBadgeTier1,
      };
    case "tier1_merged":
      return {
        label: "Merged history summary",
        icon: "🪡",
        badgeClass: styles.tierBadgeTier1Merged,
      };
    case "tier2_reactive":
      return {
        label: "Reactive compaction",
        icon: "🛟",
        badgeClass: styles.tierBadgeTier2,
      };
    default:
      return {
        label: "Context compression",
        icon: "🗜️",
        badgeClass: styles.tierBadgeTier0,
      };
  }
}

function tokenLabelFor(
  tier: SummarizationTier | undefined,
  estimate: number,
): string {
  const formatted = `~${estimate.toLocaleString()} tokens`;
  switch (tier) {
    case "tier1_llm":
    case "tier1_merged":
      return `${formatted} summarized`;
    case "tier0_deterministic":
    case "tier2_reactive":
    default:
      return `${formatted} saved`;
  }
}

type StatCell = { label: string; value: string };

type MetadataStat = {
  key: keyof Omit<ChatCompressionReportMetadata, "kind">;
  label: string;
  suffix?: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseNumberStat(value: unknown, suffix = ""): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return `${value.toLocaleString()}${suffix}`;
}

function parseCompressionReportMetadata(
  extra: Record<string, unknown> | undefined,
): ChatCompressionReportMetadata | null {
  const report = extra?.compression_report;
  if (!isRecord(report)) return null;
  if (report.kind !== "chat_compression_report") return null;
  return report as ChatCompressionReportMetadata;
}

function statsFromCompressionReportMetadata(
  extra: Record<string, unknown> | undefined,
): StatCell[] | null {
  const report = parseCompressionReportMetadata(extra);
  if (!report) return null;

  const STAT_FIELDS: MetadataStat[] = [
    { key: "context_files_removed", label: "Context files removed" },
    { key: "context_messages_dropped", label: "Context messages dropped" },
    { key: "tool_results_truncated", label: "Tool outputs truncated" },
    { key: "tokens_before", label: "Tokens before" },
    { key: "tokens_after", label: "Tokens after" },
    { key: "estimated_tokens_saved", label: "Tokens saved" },
    { key: "reduction_percent", label: "Reduction", suffix: "%" },
  ];

  const stats = STAT_FIELDS.flatMap(({ key, label, suffix }) => {
    const value = parseNumberStat(report[key], suffix);
    return value ? [{ label, value }] : [];
  });
  return stats.length > 0 ? stats : null;
}

function parseReactiveStats(content: string): StatCell[] | null {
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

export const SummarizationMessage: React.FC<SummarizationMessageProps> = ({
  message,
}) => {
  const [open, setOpen] = useState(false);

  const tier = message.summarization_tier;
  const contentText =
    typeof message.content === "string" ? message.content : "";

  const reactiveStats = useMemo(() => {
    if (tier !== "tier2_reactive") return null;
    return (
      statsFromCompressionReportMetadata(message.extra) ??
      parseReactiveStats(contentText)
    );
  }, [tier, contentText, message.extra]);

  const meta = metaForTier(tier);

  const rangeLabel = message.summarized_range
    ? `messages ${message.summarized_range[0] + 1}–${
        message.summarized_range[1] + 1
      }`
    : null;

  const compressionMeta = message.extra?.compression as
    | Record<string, unknown>
    | undefined;
  const sourceCount = Array.isArray(compressionMeta?.source_message_ids)
    ? (compressionMeta.source_message_ids as unknown[]).length
    : null;
  const summaryModel =
    typeof compressionMeta?.summary_model === "string"
      ? compressionMeta.summary_model
      : null;

  const tokenLabel =
    typeof message.summarized_token_estimate === "number"
      ? tokenLabelFor(tier, message.summarized_token_estimate)
      : null;

  return (
    <Box className={styles.card} data-testid="summarization-card">
      <Flex
        className={styles.header}
        onClick={() => setOpen((v) => !v)}
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setOpen((v) => !v);
          }
        }}
        data-testid="summarization-card-header"
      >
        <Flex className={styles.headerLeft}>
          <span className={styles.icon} aria-hidden>
            {meta.icon}
          </span>
          <span
            className={`${styles.tierBadge} ${meta.badgeClass}`}
            data-testid="summarization-card-tier"
          >
            {meta.label}
          </span>
          {rangeLabel && (
            <span className={styles.rangeLabel}>{rangeLabel}</span>
          )}
          {sourceCount !== null && (
            <span className={styles.rangeLabel}>{sourceCount} messages</span>
          )}
          {summaryModel && (
            <span className={styles.tokenLabel}>· {summaryModel}</span>
          )}
          {tokenLabel && (
            <span className={styles.tokenLabel}>· {tokenLabel}</span>
          )}
        </Flex>
        <span className={styles.toggle}>{open ? "▲" : "▼"}</span>
      </Flex>
      {open && (
        <Box className={styles.body} data-testid="summarization-card-body">
          {contentText.length > 0 ? (
            <ToolMarkdown>{contentText}</ToolMarkdown>
          ) : (
            <span>No details available.</span>
          )}
          {reactiveStats && reactiveStats.length > 0 && (
            <Box
              className={styles.statsGrid}
              data-testid="summarization-card-stats"
            >
              {reactiveStats.map((s) => (
                <Box key={s.label} className={styles.statCell}>
                  <span className={styles.statLabel}>{s.label}</span>
                  <span className={styles.statValue}>{s.value}</span>
                </Box>
              ))}
            </Box>
          )}
        </Box>
      )}
    </Box>
  );
};
