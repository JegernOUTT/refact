import React, { useMemo } from "react";
import { CheckCircle2, CircleX, FileText } from "lucide-react";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Icon } from "../ui";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useThreadId } from "../../features/Chat/Thread";
import {
  selectIsStreamingById,
  selectIsWaitingById,
} from "../../features/Chat/Thread/selectors";
import type { ToolCall, ToolResult } from "../../services/refact/types";
import {
  TranscriptClamp,
  TranscriptFacts,
  TranscriptItem,
  type TranscriptRawBlock,
  type TranscriptStatus,
} from "./TranscriptItem";
import {
  FileChips,
  ReportActions,
  ReportMarkdown,
} from "./ToolCard/ReportToolCard";
import { useIsLatestFinalReport } from "./finalReportLatest";
import styles from "./FinalReportView.module.css";
import {
  finalReportPresentation,
  parseFinalReport,
  verificationPassed,
  verificationSummary,
  type FinalReport,
} from "./finalReportJson";

const List: React.FC<{ title: string; items: readonly string[] }> = ({
  title,
  items,
}) =>
  items.length === 0 ? null : (
    <section className={styles.section}>
      <h5 className={styles.sectionTitle}>{title}</h5>
      <ul className={styles.list}>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </section>
  );

const StructuredBody: React.FC<{ report: FinalReport }> = ({ report }) => (
  <div className={styles.root} data-testid="final-report-view">
    <div className={styles.markdown}>
      <Markdown>{report.summary}</Markdown>
    </div>
    <List
      title="Tests added or updated"
      items={report.tests_added_or_updated}
    />
    {report.verification.length > 0 && (
      <section className={styles.section}>
        <h5 className={styles.sectionTitle}>Verification</h5>
        {report.verification.map((item) => (
          <div
            className={styles.verificationItem}
            key={`${item.command}:${item.exit_code ?? ""}`}
          >
            <span className={styles.verificationHeader}>
              <Icon
                icon={verificationPassed(item) ? CheckCircle2 : CircleX}
                size="sm"
                tone={verificationPassed(item) ? "success" : "danger"}
              />
              <code>{item.command}</code>
              {item.exit_code !== undefined && item.exit_code !== null && (
                <span className={styles.exitCode}>({item.exit_code})</span>
              )}
            </span>
            {item.output_tail && (
              <ShikiCodeBlock showLineNumbers={false}>
                {item.output_tail}
              </ShikiCodeBlock>
            )}
          </div>
        ))}
      </section>
    )}
    {report.followup_cards.length > 0 && (
      <section className={styles.section}>
        <h5 className={styles.sectionTitle}>Followup cards</h5>
        {report.followup_cards.map((card) => (
          <article className={styles.followupCard} key={card.title}>
            <span className={styles.followupTitle}>
              {card.title} · {card.priority}
            </span>
            <p className={styles.followupInstructions}>{card.instructions}</p>
          </article>
        ))}
      </section>
    )}
    <List title="Risks" items={report.risks} />
    <List title="Assumptions" items={report.assumptions} />
  </div>
);

export type FinalReportViewProps = {
  content: string;
  compact?: boolean;
  technicalDetails?: React.ReactNode;
};

const FinalReportBody: React.FC<{ content: string }> = ({ content }) => {
  const report = useMemo(() => parseFinalReport(content), [content]);
  if (!report) return <ReportMarkdown markdown={content} />;
  return (
    <TranscriptClamp>
      <StructuredBody report={report} />
    </TranscriptClamp>
  );
};

export const FinalReportView: React.FC<FinalReportViewProps> = ({
  content,
  compact = false,
  technicalDetails,
}) =>
  compact ? (
    <div className={styles.root}>
      <TranscriptFacts facts={finalReportPresentation(content).facts} />
      <FinalReportBody content={content} />
      {technicalDetails}
    </div>
  ) : (
    <FinalReportBody content={content} />
  );

export type FinalReportToolProps = {
  toolCall: ToolCall;
  content: string | null;
  hasResult: boolean;
  rawContent?: ToolResult["content"];
  toolFailed?: boolean;
  extra?: Record<string, unknown>;
  latest?: boolean;
};

export const FinalReportTool: React.FC<FinalReportToolProps> = ({
  toolCall,
  content,
  hasResult,
  rawContent,
  toolFailed = false,
  extra,
  latest,
}) => {
  const threadId = useThreadId();
  const active = useAppSelector(
    (state) =>
      selectIsStreamingById(state, threadId) ||
      selectIsWaitingById(state, threadId),
  );
  const isLatestInThread = useIsLatestFinalReport(toolCall.id);
  const presentation = finalReportPresentation(
    content ?? (hasResult ? "" : null),
    toolFailed,
  );
  const report = useMemo(
    () => (content ? parseFinalReport(content) : null),
    [content],
  );
  const status: TranscriptStatus =
    toolFailed || report?.success === false
      ? "failed"
      : !hasResult
        ? active
          ? "running"
          : "waiting"
        : report
          ? "done"
          : "unknown";
  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: toolCall.function.arguments },
    ...(rawContent !== undefined
      ? [{ label: "Response", value: rawContent }]
      : []),
    ...(extra ? [{ label: "Metadata", value: extra }] : []),
  ];

  return (
    <TranscriptItem
      data-testid="final-report-tool"
      actions={content ? <ReportActions markdown={content} /> : null}
      artifacts={
        report && report.files_changed.length > 0 ? (
          <FileChips files={report.files_changed} />
        ) : null
      }
      defaultOpen={latest ?? isLatestInThread}
      icon={<FileText />}
      meta={report ? verificationSummary(report) : presentation.stateLabel}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={content ? <FinalReportBody content={content} /> : null}
      raw={raw}
      sectionLabels={{ output: "Report" }}
      status={status}
      statusLabel={status === "done" ? undefined : presentation.stateLabel}
      summary={presentation.facts}
      target={presentation.headline}
      verb="Report"
    />
  );
};

export default FinalReportView;
