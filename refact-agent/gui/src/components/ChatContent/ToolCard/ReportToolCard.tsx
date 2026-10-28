import { BookOpen, Check, Copy, FileText } from "lucide-react";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ToolCall } from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import { Button, Chip } from "../../ui";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../../hooks/useEventBusForIDE";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { isIdeHost } from "../../../utils/isIdeHost";
import { basename } from "./utils";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptRawBlock,
  type TranscriptStatus,
} from "../TranscriptItem";
import { parseFinalReport, verificationSummary } from "../finalReportJson";
import styles from "./ReportToolCard.module.css";

const MAX_MD_RENDER_CHARS = 50_000;
const MAX_STREAMING_PROGRESS_CHARS = 500;
const COPIED_RESET_MS = 2000;

export const ReportMarkdown: React.FC<{
  markdown: string;
  streaming?: boolean;
}> = ({ markdown, streaming = false }) => (
  <TranscriptClamp>
    <div className={styles.markdown}>
      {markdown.length <= MAX_MD_RENDER_CHARS ? (
        <Markdown isStreaming={streaming}>{markdown}</Markdown>
      ) : (
        <ShikiCodeBlock showLineNumbers={false}>{markdown}</ShikiCodeBlock>
      )}
    </div>
  </TranscriptClamp>
);

export const ReportActions: React.FC<{ markdown: string }> = ({ markdown }) => {
  const copyToClipboard = useCopyToClipboard();
  const { newFile } = useEventsBusForIDE();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const id = setTimeout(() => setCopied(false), COPIED_RESET_MS);
    return () => clearTimeout(id);
  }, [copied]);

  const handleCopy = useCallback(() => {
    copyToClipboard(markdown);
    setCopied(true);
  }, [copyToClipboard, markdown]);

  return (
    <>
      <Button
        aria-label="Copy report"
        size="sm"
        title="Copy report"
        variant="soft"
        onClick={handleCopy}
      >
        {copied ? <Check /> : <Copy />}
        {copied ? "Copied" : "Copy"}
      </Button>
      {isIdeHost() && (
        <Button
          aria-label="Save as file"
          size="sm"
          title="Save as file"
          variant="soft"
          onClick={() => newFile(markdown)}
        >
          <FileText />
          Save as file
        </Button>
      )}
    </>
  );
};

export const FileChips: React.FC<{ files: readonly string[] }> = ({
  files,
}) => {
  const { canOpen, openFile } = useOpenFileInApp();
  if (files.length === 0) return null;
  return (
    <div className={styles.files}>
      {files.map((file) =>
        canOpen ? (
          <button
            className={styles.fileButton}
            key={file}
            title={file}
            type="button"
            onClick={() => openFile({ path: file })}
          >
            <Chip radius="chip">{basename(file)}</Chip>
          </button>
        ) : (
          <Chip key={file} radius="chip" title={file}>
            {basename(file)}
          </Chip>
        ),
      )}
    </div>
  );
};

export type ReportVariant = "taskDone" | "plan" | "report";

export interface ReportData {
  summary?: string;
  markdown: string;
  filesChanged?: string[];
  knowledgePath?: string;
  success?: boolean;
}

interface ReportToolCardProps {
  toolCall: ToolCall;
  icon: React.ReactNode;
  defaultSummary: React.ReactNode;
  verb?: React.ReactNode;
  variant?: ReportVariant;
  meta?: string | null;
  extractReport?: (content: string) => ReportData | null;
  defaultOpen?: boolean;
  unboundedContent?: boolean;
  compact?: boolean;
  testId?: string;
  input?: React.ReactNode;
  facts?: readonly TranscriptFact[];
}

export const ReportToolCard: React.FC<ReportToolCardProps> = ({
  toolCall,
  icon,
  defaultSummary,
  verb = "Report",
  variant = "report",
  meta,
  extractReport,
  defaultOpen = false,
  testId,
  input,
  facts = [],
}) => {
  const threadId = useThreadId();
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const reportData = useMemo((): ReportData | null => {
    if (!content) return null;
    return extractReport?.(content) ?? { markdown: content };
  }, [content, extractReport]);
  const structured = useMemo(
    () => (content ? parseFinalReport(content) : null),
    [content],
  );

  const requestFailed = Boolean(maybeResult?.tool_failed);
  const reportedSuccess = structured?.success ?? reportData?.success;
  const running = !maybeResult && unrecorded.status === "running";
  const status: TranscriptStatus = !maybeResult
    ? unrecorded.status
    : requestFailed || reportedSuccess === false
      ? "failed"
      : "done";
  const statusLabel = !maybeResult
    ? unrecorded.statusLabel
    : requestFailed
      ? "Request failed"
      : reportedSuccess === false
        ? "Reported failure"
        : undefined;

  const verification = structured
    ? verificationSummary(structured)
    : maybeResult
      ? "No verification evidence"
      : null;
  const outcome =
    reportedSuccess === true
      ? "Success (self-reported)"
      : reportedSuccess === false
        ? "Failure (self-reported)"
        : null;
  const files = structured?.files_changed ?? reportData?.filesChanged ?? [];

  const progressText = useMemo(() => {
    if (!running) return null;
    const log = toolCall.subchat_log;
    if (!log || log.length === 0) return null;
    return log[log.length - 1].slice(-MAX_STREAMING_PROGRESS_CHARS);
  }, [running, toolCall.subchat_log]);
  const deferredProgress = useStreamingMarkdown(progressText, running);
  const deferredMarkdown = useStreamingMarkdown(
    reportData?.markdown ?? null,
    running,
  );

  useEffect(() => {
    if (running) {
      if (deferredProgress) {
        addBuddyCrashBreadcrumb("report_progress", deferredProgress);
      }
      return;
    }
    if (variant === "taskDone") {
      setBuddyCrashHotSlot(
        "report",
        deferredMarkdown ?? reportData?.summary ?? null,
      );
      if (deferredMarkdown)
        addBuddyCrashBreadcrumb("task_done", deferredMarkdown);
      return;
    }
    setBuddyCrashHotSlot("report", null);
  }, [
    deferredMarkdown,
    deferredProgress,
    reportData?.summary,
    running,
    variant,
  ]);

  const target = structured?.summary ?? reportData?.summary ?? defaultSummary;
  const metaText =
    variant === "taskDone" ? verification ?? undefined : meta ?? undefined;

  const summary: TranscriptFact[] = [
    ...facts,
    ...(outcome ? [{ label: "Outcome", value: outcome }] : []),
    ...(verification && variant !== "taskDone"
      ? [{ label: "Verification", value: verification }]
      : []),
    ...(files.length > 0
      ? [{ label: "Files changed", value: String(files.length) }]
      : []),
    ...(reportData?.knowledgePath
      ? [
          {
            label: "Knowledge",
            value: (
              <span className={styles.knowledge}>
                <BookOpen aria-hidden />
                Saved to knowledge
              </span>
            ),
            title: reportData.knowledgePath,
          },
        ]
      : []),
  ];

  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: toolCall.function.arguments },
    ...(maybeResult ? [{ label: "Response", value: maybeResult.content }] : []),
  ];

  return (
    <TranscriptItem
      data-testid={testId}
      actions={
        deferredMarkdown ? <ReportActions markdown={deferredMarkdown} /> : null
      }
      artifacts={files.length > 0 ? <FileChips files={files} /> : null}
      defaultOpen={defaultOpen && Boolean(maybeResult)}
      icon={icon}
      input={input}
      meta={metaText}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={
        deferredMarkdown ? (
          <ReportMarkdown markdown={deferredMarkdown} streaming={running} />
        ) : deferredProgress ? (
          <TranscriptClamp>
            <pre className={styles.progress}>{deferredProgress}</pre>
          </TranscriptClamp>
        ) : null
      }
      raw={raw}
      sectionLabels={{
        input: "Task",
        output: deferredMarkdown ? "Report" : "Progress",
      }}
      status={status}
      statusLabel={statusLabel}
      summary={summary}
      target={target}
      verb={verb}
    />
  );
};

export default ReportToolCard;
