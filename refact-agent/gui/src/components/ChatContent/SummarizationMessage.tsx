import React, { useMemo } from "react";
import type { LucideIcon } from "lucide-react";
import { Archive, Brain, GitMerge } from "lucide-react";
import {
  getAssistantCompressionMetadata,
  getReconstructedHistoryMetadata,
  getCompressionReportMetadata,
} from "../../services/refact/types";
import type {
  ReconstructedHistoryMetadata,
  SummarizationMessage as SummarizationMessageType,
  SummarizationTier,
} from "../../services/refact/types";
import { ToolMarkdown } from "../Markdown";
import { Icon } from "../ui";
import {
  TranscriptClamp,
  TranscriptFacts,
  TranscriptItem,
  type TranscriptFact,
} from "./TranscriptItem";
import {
  compressionMeta,
  finiteNumber,
  isRecord,
  isSourcePreservingReport,
  parseReactiveStats,
  reconstructionTriggerLabel,
  sourceMessageCountFromReport,
  statsFromCompressionReportMetadata,
  statsFromReconstructionMetrics,
  type StatCell,
} from "./SummarizationMessage.stats";
import styles from "./SummarizationMessage.module.css";

interface SummarizationMessageProps {
  message: SummarizationMessageType;
}

const RECONSTRUCTION_DESCRIPTION =
  "The model context was rebuilt so this chat can continue within its limit. Original messages remain in the transcript.";
const LLM_SEGMENT_SUMMARY_DESCRIPTION =
  "Older context was summarized so this chat can continue within the model limit.";
const SOURCE_PRESERVING_SUMMARY_DESCRIPTION =
  "Original messages remain visible. A compact summary was added for future model context.";

type TierMeta = { label: string; icon: LucideIcon };

function metaForTier(
  tier: SummarizationTier | undefined,
  isSegmentReport: boolean,
): TierMeta {
  if (isSegmentReport) return { label: "Context compressed", icon: Archive };
  switch (tier) {
    case "tier0_deterministic":
      return { label: "Deterministic compaction", icon: Archive };
    case "tier1_llm":
      return { label: "LLM summary", icon: Brain };
    case "tier1_merged":
      return { label: "Merged history summary", icon: GitMerge };
    case "tier2_reactive":
      return { label: "Reactive compaction", icon: Archive };
    default:
      return { label: "Context compression", icon: Archive };
  }
}

function statFacts(stats: StatCell[] | null): TranscriptFact[] {
  return (stats ?? []).map((stat) => ({
    label: stat.label,
    value: stat.value,
  }));
}

const SummaryBlock: React.FC<{
  description?: string | null;
  facts: TranscriptFact[];
}> = ({ description, facts }) => (
  <div className={styles.summary} data-testid="summarization-card-summary">
    {description && <p className={styles.description}>{description}</p>}
    {facts.length > 0 && (
      <div data-testid="summarization-card-stats">
        <TranscriptFacts facts={facts} />
      </div>
    )}
  </div>
);

function messageCountMeta(before: number | null, after: number | null) {
  if (before !== null && after !== null) {
    return `${before.toLocaleString()} → ${after.toLocaleString()} messages`;
  }
  if (before !== null) return `${before.toLocaleString()} messages`;
  return null;
}

const ReconstructedHistoryReport: React.FC<{
  metadata: ReconstructedHistoryMetadata;
}> = ({ metadata }) => {
  const stats = useMemo(
    () => statsFromReconstructionMetrics(metadata.metrics),
    [metadata.metrics],
  );
  const metrics = isRecord(metadata.metrics) ? metadata.metrics : {};
  const rebuiltCount = metadata.payload.messages.length;
  const meta = compressionMeta(
    messageCountMeta(
      finiteNumber(metrics.messages_before),
      finiteNumber(metrics.messages_after) ?? rebuiltCount,
    ) ?? `${rebuiltCount} ${rebuiltCount === 1 ? "message" : "messages"}`,
    finiteNumber(metrics.estimated_tokens_saved),
  );
  const facts: TranscriptFact[] = [
    { label: "Kind", value: "Context rebuilt" },
    ...(metadata.model ? [{ label: "Model", value: metadata.model }] : []),
    ...(metadata.trigger
      ? [
          {
            label: "Trigger",
            value: reconstructionTriggerLabel(metadata.trigger),
          },
        ]
      : []),
    ...((metadata.from_mode ?? metadata.to_mode) &&
    metadata.from_mode !== metadata.to_mode
      ? [
          {
            label: "Mode",
            value: `${metadata.from_mode ?? "—"} → ${metadata.to_mode ?? "—"}`,
          },
        ]
      : []),
    ...statFacts(stats),
  ];

  return (
    <TranscriptItem
      data-testid="reconstructed-history-report"
      icon={<Icon icon={Archive} size="sm" />}
      verb="Compressed"
      target="context"
      meta={meta}
      summary={
        <SummaryBlock description={RECONSTRUCTION_DESCRIPTION} facts={facts} />
      }
      sectionLabels={{ output: "Rebuilt messages" }}
      output={
        <TranscriptClamp>
          {metadata.payload.messages.map((item, index) => (
            <section className={styles.rebuilt} key={item.message_id ?? index}>
              <strong>{item.role}</strong>
              <ToolMarkdown>
                {typeof item.content === "string"
                  ? item.content
                  : JSON.stringify(item.content, null, 2)}
              </ToolMarkdown>
            </section>
          ))}
        </TranscriptClamp>
      }
      raw={metadata.payload.messages.flatMap((item, index) =>
        "tool_calls" in item && item.tool_calls
          ? [{ label: `Tool calls ${index + 1}`, value: item.tool_calls }]
          : [],
      )}
    />
  );
};

const LegacySummarizationMessage: React.FC<SummarizationMessageProps> = ({
  message,
}) => {
  const tier = message.summarization_tier;
  const contentText =
    typeof message.content === "string" ? message.content.trim() : "";
  const pairedSummary =
    typeof message.paired_summary_content === "string"
      ? message.paired_summary_content.trim()
      : "";
  const report = getCompressionReportMetadata(message);
  const isSegmentReport = report?.compression_kind === "llm_segment_summary";
  const reportStats = useMemo(
    () => statsFromCompressionReportMetadata(message),
    [message],
  );
  const bodyStats = useMemo(
    () =>
      reportStats === null && tier === "tier2_reactive"
        ? parseReactiveStats(contentText)
        : null,
    [tier, reportStats, contentText],
  );
  const tierMeta = metaForTier(tier, isSegmentReport);
  const compression = getAssistantCompressionMetadata(message);
  const sourceCount =
    (report ? sourceMessageCountFromReport(report) : null) ??
    (Array.isArray(compression?.source_message_ids)
      ? compression.source_message_ids.length
      : null);
  const summaryModel =
    typeof report?.summary_model === "string"
      ? report.summary_model
      : typeof compression?.summary_model === "string"
        ? compression.summary_model
        : null;
  const range = message.summarized_range
    ? `messages ${message.summarized_range[0] + 1}–${
        message.summarized_range[1] + 1
      }`
    : null;
  const tokensSaved =
    finiteNumber(report?.estimated_tokens_saved) ??
    finiteNumber(message.summarized_token_estimate);
  const meta = compressionMeta(
    messageCountMeta(sourceCount, null) ?? range,
    tokensSaved,
  );
  const description = isSegmentReport
    ? isSourcePreservingReport(report)
      ? SOURCE_PRESERVING_SUMMARY_DESCRIPTION
      : LLM_SEGMENT_SUMMARY_DESCRIPTION
    : null;
  const facts: TranscriptFact[] = [
    { label: "Kind", value: tierMeta.label },
    ...(range ? [{ label: "Range", value: range }] : []),
    ...(summaryModel ? [{ label: "Model", value: summaryModel }] : []),
    ...statFacts(reportStats ?? bodyStats),
  ];

  return (
    <TranscriptItem
      data-testid="summarization-card"
      icon={<Icon icon={tierMeta.icon} size="sm" />}
      verb="Compressed"
      target="context"
      meta={meta}
      summary={<SummaryBlock description={description} facts={facts} />}
      sectionLabels={{ output: "Summary" }}
      output={
        contentText || pairedSummary ? (
          <TranscriptClamp>
            <div data-testid="summarization-card-body">
              {contentText && <ToolMarkdown>{contentText}</ToolMarkdown>}
              {pairedSummary && (
                <>
                  <p className={styles.description}>
                    <strong>Model summary</strong>
                  </p>
                  <ToolMarkdown>{pairedSummary}</ToolMarkdown>
                </>
              )}
            </div>
          </TranscriptClamp>
        ) : null
      }
    />
  );
};

export const SummarizationMessage = React.memo(function SummarizationMessage({
  message,
}: SummarizationMessageProps) {
  const report = getReconstructedHistoryMetadata({
    ...message,
    role: "compression_report",
  });
  if (!report) return <LegacySummarizationMessage message={message} />;
  if (!report.ok) {
    return (
      <TranscriptItem
        data-testid="reconstructed-history-report"
        icon={<Icon icon={Archive} size="sm" />}
        verb="Compressed"
        target="context"
        meta="unreadable"
        status="failed"
        defaultOpen
        error={`Rebuilt context could not be read (${report.reason}). The archived transcript is still available.`}
      />
    );
  }
  return <ReconstructedHistoryReport metadata={report.metadata} />;
});
