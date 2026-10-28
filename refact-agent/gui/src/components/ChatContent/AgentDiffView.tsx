import React, { useMemo, useState } from "react";
import classNames from "classnames";
import { CircleAlert, FileText, Github } from "lucide-react";
import groupBy from "lodash.groupby";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
} from "../../features/Chat/Thread/selectors";
import { useThreadId } from "../../features/Chat/Thread";
import type { DiffChunk, ToolCall } from "../../services/refact/types";
import { ShikiCodeBlock } from "../Markdown";
import {
  COMPACT_HEADLINE_CHARS,
  compactText,
} from "../Tools/AgenticTool/compactPresentation";
import { Button, Icon } from "../ui";
import { TranscriptItem, type TranscriptStatus } from "./TranscriptItem";
import { DiffBlock } from "./ToolCard/DiffBlock";
import editToolStyles from "./ToolCard/EditTool.module.css";
import { parseAgentDiffOutput, type AgentDiffReport } from "./AgentDiffModel";
import styles from "./AgentDiffView.module.css";

type AgentDiffContentProps = {
  report: AgentDiffReport;
};

type AgentDiffViewProps = {
  toolCall: ToolCall;
};

function countLines(text: string): number {
  if (!text) return 0;
  return text.split("\n").filter((line) => line.trim()).length;
}

type GroupedDiffChunks = Record<string, DiffChunk[] | undefined>;

function groupDiffChunks(chunks: DiffChunk[]): GroupedDiffChunks {
  return groupBy(chunks, (chunk) => chunk.file_name);
}

function normalizeGroupedDiffs(
  groupedDiffs: GroupedDiffChunks,
): Record<string, DiffChunk[]> {
  return Object.fromEntries(
    Object.entries(groupedDiffs).map(([file, diffs]) => [file, diffs ?? []]),
  );
}

export const AgentDiffContent: React.FC<AgentDiffContentProps> = ({
  report,
}) => {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const groupedDiffs = useMemo(
    () => groupDiffChunks(report.diffChunks),
    [report.diffChunks],
  );
  const selectedDiffs = useMemo(() => {
    if (!selectedFile) return normalizeGroupedDiffs(groupedDiffs);
    return { [selectedFile]: groupedDiffs[selectedFile] ?? [] };
  }, [groupedDiffs, selectedFile]);

  const showRenderedDiff = report.diffChunks.length > 0;
  const shouldShowFileTree = report.files.length > 1;
  const selectedFileHasNoDiffs =
    selectedFile !== null && (groupedDiffs[selectedFile]?.length ?? 0) === 0;

  return (
    <div className={styles.root}>
      <section className={classNames(styles.summary, "rf-enter")}>
        <div className={styles.summaryTop}>
          <span className={styles.title}>Agent diff: {report.cardId}</span>
          <span className={styles.mode}>{report.mode}</span>
        </div>
        <div className={styles.metaGrid}>
          <div className={styles.metaItem}>
            <span className={styles.label}>Card</span>
            <span className={styles.value}>{report.cardTitle}</span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Branch</span>
            <span className={styles.value}>{report.branch}</span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Base</span>
            <span className={styles.value}>{report.base}</span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Files</span>
            <span className={styles.value}>{report.stats.files}</span>
          </div>
        </div>
        <div className={styles.stats}>
          <span className={styles.statMuted}>{report.stats.files} files</span>
          {report.stats.added > 0 && (
            <span className={styles.added}>+{report.stats.added}</span>
          )}
          {report.stats.removed > 0 && (
            <span className={styles.removed}>−{report.stats.removed}</span>
          )}
          <span className={styles.statMuted}>
            {countLines(report.body)} lines
          </span>
        </div>
        {report.truncated && (
          <div className={styles.truncation}>
            <Icon icon={CircleAlert} size="sm" tone="warning" />
            <span>{report.truncated}</span>
          </div>
        )}
      </section>

      {report.body.trim() === "(no changes detected)" ? (
        <div className={styles.empty}>No changes detected.</div>
      ) : (
        <div className={styles.content}>
          {shouldShowFileTree && (
            <nav className={styles.fileTree} aria-label="Diff files">
              <span className={styles.label}>Files</span>
              <div className={styles.fileList}>
                <Button
                  size="sm"
                  variant={selectedFile === null ? "primary" : "ghost"}
                  className={styles.fileButton}
                  onClick={() => setSelectedFile(null)}
                >
                  <span className={styles.fileButtonInner}>All files</span>
                </Button>
                {report.files.map((file) => (
                  <Button
                    key={file}
                    size="sm"
                    variant={selectedFile === file ? "primary" : "ghost"}
                    className={styles.fileButton}
                    onClick={() => setSelectedFile(file)}
                  >
                    <span className={styles.fileButtonInner}>
                      <Icon icon={FileText} size="sm" />
                      {file}
                    </span>
                  </Button>
                ))}
              </div>
            </nav>
          )}
          <div
            className={classNames(
              styles.diffPane,
              !shouldShowFileTree && styles.diffPaneFull,
            )}
          >
            {selectedFileHasNoDiffs ? (
              <div className={styles.emptyDiffMessage}>
                No diff hunks for this file.
              </div>
            ) : showRenderedDiff ? (
              <div className={editToolStyles.diffContent}>
                {Object.entries(selectedDiffs).flatMap(([fileName, diffs]) =>
                  diffs.map((diff, i) => (
                    <DiffBlock
                      key={`${fileName}-${i}`}
                      diff={diff}
                      fileName={fileName}
                      displayFileName={fileName}
                    />
                  )),
                )}
              </div>
            ) : (
              <div className={styles.rawDiff}>
                <ShikiCodeBlock
                  showLineNumbers={false}
                  className="language-text"
                >
                  {report.body}
                </ShikiCodeBlock>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export const AgentDiffView: React.FC<AgentDiffViewProps> = ({ toolCall }) => {
  const threadId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, threadId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, threadId),
  );

  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const report = useMemo(
    () => (content ? parseAgentDiffOutput(content) : null),
    [content],
  );

  const failed = maybeResult?.tool_failed === true;
  const status: TranscriptStatus = failed
    ? "failed"
    : !maybeResult
      ? isStreaming || isWaiting
        ? "running"
        : "unknown"
      : "done";
  const title =
    report && report.cardTitle !== report.cardId
      ? compactText(report.cardTitle, COMPACT_HEADLINE_CHARS)
      : report?.cardId ?? null;
  const files = report?.stats.files ?? 0;
  const meta =
    report === null
      ? undefined
      : files > 0
        ? `${files} ${files === 1 ? "file" : "files"}`
        : "no changes";

  return (
    <TranscriptItem
      data-testid="agent-diff-view"
      icon={<Github aria-hidden />}
      verb="Diff"
      target={title ?? undefined}
      meta={meta}
      status={status}
      statusLabel={report?.truncated ? "Partial" : undefined}
      statusDetail={report?.truncated ?? undefined}
      output={report ? <AgentDiffContent report={report} /> : undefined}
      error={failed ? content : undefined}
      raw={[
        { label: "Arguments", value: toolCall.function.arguments },
        ...(content !== null ? [{ label: "Response", value: content }] : []),
      ]}
      openKey={toolCall.id ? `agent_diff:${toolCall.id}` : undefined}
      defaultOpen={files > 0}
    />
  );
};

export default AgentDiffView;
