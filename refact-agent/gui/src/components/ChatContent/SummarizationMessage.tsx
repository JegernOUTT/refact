import React, { useId, useMemo, useState } from "react";
import { Box, Flex } from "@radix-ui/themes";
import {
  getAssistantCompressionMetadata,
  getCompressionReportMetadata,
} from "../../services/refact/types";
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

const LLM_SEGMENT_SUMMARY_DESCRIPTION =
  "Older context was summarized so this chat can continue within the model limit.";

function metaForTier(
  tier: SummarizationTier | undefined,
  isSegmentCompressionReport: boolean,
): TierMeta {
  if (isSegmentCompressionReport) {
    return {
      label: "Context compressed",
      icon: "🗜️",
      badgeClass: styles.tierBadgeTier1,
    };
  }

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
        icon: "🗜️",
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
  isSegmentCompressionReport: boolean,
): string {
  const formatted = `~${estimate.toLocaleString()} tokens`;
  if (isSegmentCompressionReport) return `${formatted} saved`;
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

function parseStringStat(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function parseNumberStat(value: unknown, suffix = ""): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return `${value.toLocaleString()}${suffix}`;
}

function sourceMessageCountFromReport(
  report: ChatCompressionReportMetadata,
): number | null {
  if (
    typeof report.source_message_count === "number" &&
    Number.isFinite(report.source_message_count)
  ) {
    return report.source_message_count;
  }
  if (Array.isArray(report.source_message_ids)) {
    return report.source_message_ids.length;
  }
  return null;
}

function statCell(label: string, value: string | null): StatCell[] {
  return value ? [{ label, value }] : [];
}

function statsFromCompressionReportMetadata(
  message: SummarizationMessageType,
): StatCell[] | null {
  const report = getCompressionReportMetadata(message);
  if (!report) return null;

  const sourceMessageCount = sourceMessageCountFromReport(report);
  const stats: StatCell[] = [
    ...statCell(
      "Messages compressed",
      sourceMessageCount !== null ? sourceMessageCount.toLocaleString() : null,
    ),
    ...statCell(
      "Tokens saved",
      parseNumberStat(report.estimated_tokens_saved),
    ),
    ...statCell("Reduction", parseNumberStat(report.reduction_percent, "%")),
    ...statCell("Summary model", parseStringStat(report.summary_model)),
    ...statCell("Tokens before", parseNumberStat(report.tokens_before)),
    ...statCell("Tokens after", parseNumberStat(report.tokens_after)),
    ...statCell(
      "Context messages dropped",
      parseNumberStat(report.context_messages_dropped),
    ),
    ...statCell(
      "Context files removed",
      parseNumberStat(report.context_files_removed),
    ),
    ...statCell(
      "Tool outputs truncated",
      parseNumberStat(report.tool_results_truncated),
    ),
  ];
  return stats.length > 0 ? stats : null;
}

function isPrimaryReportStat(stat: StatCell): boolean {
  return (
    stat.label === "Messages compressed" ||
    stat.label === "Tokens saved" ||
    stat.label === "Reduction" ||
    stat.label === "Summary model"
  );
}

function primaryReportStats(stats: StatCell[] | null): StatCell[] | null {
  if (!stats) return null;
  const primary = stats.filter(isPrimaryReportStat);
  return primary.length > 0 ? primary : null;
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

function StatsGrid({ stats }: { stats: StatCell[] }) {
  return (
    <Box className={styles.statsGrid} data-testid="summarization-card-stats">
      {stats.map((s) => (
        <Box key={s.label} className={styles.statCell}>
          <span className={styles.statLabel}>{s.label}</span>
          <span className={styles.statValue}>{s.value}</span>
        </Box>
      ))}
    </Box>
  );
}

export const SummarizationMessage: React.FC<SummarizationMessageProps> = ({
  message,
}) => {
  const [open, setOpen] = useState(false);
  const bodyId = useId();

  const tier = message.summarization_tier;
  const contentText =
    typeof message.content === "string" ? message.content : "";
  const compressionReport = getCompressionReportMetadata(message);
  const isSegmentCompressionReport =
    compressionReport?.compression_kind === "llm_segment_summary";

  const reportStats = useMemo(
    () => statsFromCompressionReportMetadata(message),
    [message],
  );
  const bodyStats = useMemo(() => {
    if (reportStats !== null || tier !== "tier2_reactive") return null;
    return parseReactiveStats(contentText);
  }, [tier, reportStats, contentText]);

  const meta = metaForTier(tier, isSegmentCompressionReport);

  const rangeLabel = message.summarized_range
    ? `messages ${message.summarized_range[0] + 1}–${
        message.summarized_range[1] + 1
      }`
    : null;

  const compressionMeta = getAssistantCompressionMetadata(message);
  const reportSourceCount = compressionReport
    ? sourceMessageCountFromReport(compressionReport)
    : null;
  const sourceCount =
    reportSourceCount ??
    (Array.isArray(compressionMeta?.source_message_ids)
      ? compressionMeta.source_message_ids.length
      : null);
  const summaryModel =
    typeof compressionReport?.summary_model === "string"
      ? compressionReport.summary_model
      : typeof compressionMeta?.summary_model === "string"
        ? compressionMeta.summary_model
        : null;

  const tokenLabel =
    typeof message.summarized_token_estimate === "number"
      ? tokenLabelFor(
          tier,
          message.summarized_token_estimate,
          isSegmentCompressionReport,
        )
      : null;

  const showHeaderMetrics = reportStats === null;
  const reportSummaryStats = isSegmentCompressionReport
    ? primaryReportStats(reportStats)
    : reportStats;
  const hasReportSummary =
    isSegmentCompressionReport || reportSummaryStats !== null;
  const cardClassName = compressionReport
    ? `${styles.card} ${styles.reportCard}`
    : styles.card;
  const toggleOpen = () => setOpen((v) => !v);

  return (
    <Box className={cardClassName} data-testid="summarization-card">
      <Flex
        className={styles.header}
        onClick={toggleOpen}
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-controls={bodyId}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleOpen();
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
          {showHeaderMetrics && sourceCount !== null && (
            <span className={styles.rangeLabel}>{sourceCount} messages</span>
          )}
          {showHeaderMetrics && summaryModel && (
            <span className={styles.tokenLabel}>· {summaryModel}</span>
          )}
          {showHeaderMetrics && tokenLabel && (
            <span className={styles.tokenLabel}>· {tokenLabel}</span>
          )}
        </Flex>
        <span className={styles.toggle}>{open ? "▲" : "▼"}</span>
      </Flex>
      {hasReportSummary && (
        <Box
          className={styles.eventSummary}
          data-testid="summarization-card-summary"
        >
          {isSegmentCompressionReport && (
            <p className={styles.description}>
              {LLM_SEGMENT_SUMMARY_DESCRIPTION}
            </p>
          )}
          {reportSummaryStats && <StatsGrid stats={reportSummaryStats} />}
        </Box>
      )}
      {open && (
        <Box
          id={bodyId}
          className={styles.body}
          data-testid="summarization-card-body"
        >
          {contentText.length > 0 ? (
            <ToolMarkdown>{contentText}</ToolMarkdown>
          ) : (
            <span>No details available.</span>
          )}
          {bodyStats && bodyStats.length > 0 && <StatsGrid stats={bodyStats} />}
        </Box>
      )}
    </Box>
  );
};
