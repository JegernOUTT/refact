import { Code, Copy, FileText, Rows3, Square } from "lucide-react";
import React, { useEffect, useMemo, useState } from "react";

import { useToolCallActivity } from "./useToolCallActivity";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useAppSelector } from "../../../hooks/useAppSelector";
import type { ToolCall } from "../../../services/refact/types";
import { getToolEnrichmentPathReferences } from "../../../services/refact/types";
import { Button } from "../../ui";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
} from "../TranscriptItem";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import { selectFoldedProcessEvent } from "../foldedEventSelectors";
import { ProcessStatusBadge } from "./ProcessStatusBadge";
import { ProcessOutputView } from "./ProcessOutputView";
import { ProcessStdinInput } from "./ProcessStdinInput";
import { useExecProcessActions } from "./useExecProcessActions";
import {
  displayProcess,
  durationText,
  execMetadataFromResult,
  execMeta,
  execStatusLabel,
  execTarget,
  execTranscriptStatus,
  execVerb,
  foldedProcessOutcome,
  isBusyStatus,
  isProcessSnapshotTool,
  isProcessStartCall,
  latestOutputLine,
  normalizeExecStatus,
  parseProcessArgs,
  persistedOutputPath,
  processListItemStatus,
  snapshotMeta,
  snapshotTranscriptStatus,
  withFoldedOutcome,
  type ExecResultSource,
  type ExecToolName,
  type ProcessArgs,
} from "./execToolModel";
import styles from "./ExecToolCard.module.css";

type ExecToolCardProps = {
  toolCall: ToolCall;
  toolName: ExecToolName;
};

const MAX_LISTED_PROCESSES = 20;
const MAX_TRANSCRIPT_CHARS = 30_000;

type ExecRowKind = "start" | "snapshot" | "action";

function execRowKind(toolName: ExecToolName, args: ProcessArgs): ExecRowKind {
  if (isProcessStartCall(toolName, args)) return "start";
  if (isProcessSnapshotTool(toolName)) return "snapshot";
  if (toolName === "shell_service" && args.action === "status") {
    return "snapshot";
  }
  return "action";
}

function resultSource(
  toolName: ExecToolName,
  kind: ExecRowKind,
): ExecResultSource {
  if (kind === "start") return "start";
  return toolName === "shell" || toolName === "exec" ? "none" : "snapshot";
}

function useRunningNowMs(isBusy: boolean): number {
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!isBusy) return undefined;
    const interval = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [isBusy]);
  return nowMs;
}

export const ExecToolCard: React.FC<ExecToolCardProps> = ({
  toolCall,
  toolName,
}) => {
  const copyToClipboard = useCopyToClipboard();
  const { result, isRunning, threadId } = useToolCallActivity(toolCall.id);
  const content = typeof result?.content === "string" ? result.content : null;
  const args = useMemo(
    () => parseProcessArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const rowKind = execRowKind(toolName, args);
  const isSnapshot = rowKind === "snapshot";
  const source = resultSource(toolName, rowKind);
  const metadata = useMemo(
    () => execMetadataFromResult(result?.extra, result?.content, source),
    [result?.extra, result?.content, source],
  );
  const pathReferences = getToolEnrichmentPathReferences(result?.extra);
  const toolFailed = result?.tool_failed === true;
  const foldedEvent = useAppSelector((state) =>
    selectFoldedProcessEvent(state, threadId, toolCall.id),
  );
  const outcome = useMemo(
    () => foldedProcessOutcome(foldedEvent),
    [foldedEvent],
  );
  const unrecorded = useUnrecordedResultState(toolCall.id, threadId);
  const status =
    outcome?.status ??
    normalizeExecStatus(
      metadata,
      toolName,
      Boolean(result),
      toolFailed,
      isRunning,
    );
  const process = withFoldedOutcome(
    displayProcess(metadata, args, status),
    outcome,
  );
  const hasRecordedOutcome = Boolean(result) || outcome !== null;
  const nowMs = useRunningNowMs(isRunning);
  const duration = isSnapshot ? null : durationText(process, nowMs, isRunning);
  const logPath = persistedOutputPath(metadata);
  const actions = useExecProcessActions(
    process.processId,
    threadId,
    toolCall.id,
    logPath,
  );
  const transcriptStatus = !hasRecordedOutcome
    ? unrecorded.status
    : isSnapshot && !isRunning
      ? snapshotTranscriptStatus(status, toolFailed)
      : execTranscriptStatus(status, isRunning, toolFailed);
  const statusText = !hasRecordedOutcome
    ? unrecorded.statusLabel ?? execStatusLabel(status)
    : isSnapshot && transcriptStatus === "failed" && isBusyStatus(status)
      ? execStatusLabel("failed")
      : execStatusLabel(status);
  const liveHint =
    isRunning || (!isSnapshot && isBusyStatus(status))
      ? latestOutputLine(metadata, isRunning ? content : null, toolCall)
      : null;
  const meta =
    isSnapshot && !isRunning
      ? snapshotMeta(toolName, process, metadata)
      : execMeta(toolName, process, metadata, duration, isRunning, liveHint);
  const canStop =
    !isSnapshot &&
    Boolean(process.processId) &&
    actions.canReadTranscript &&
    (isRunning || isBusyStatus(status));
  const showStdin = Boolean(
    process.processId && metadata?.tty === true && isBusyStatus(status),
  );
  const listed = metadata?.processes?.slice(0, MAX_LISTED_PROCESSES) ?? [];
  const hiddenProcesses = (metadata?.processes?.length ?? 0) - listed.length;
  const transcript = actions.state?.content;

  const summary: TranscriptFact[] = [];
  if (process.command) {
    summary.push({
      label: "Command",
      value: <code className={styles.code}>{process.command}</code>,
      title: process.command,
    });
  }
  if (process.cwd) {
    summary.push({
      label: "Directory",
      value: process.cwd,
      title: process.cwd,
    });
  }
  if (process.mode) summary.push({ label: "Mode", value: process.mode });
  if (typeof process.exitCode === "number") {
    summary.push({ label: "Exit", value: String(process.exitCode) });
  }
  if (duration) summary.push({ label: "Duration", value: duration });
  if (metadata?.tty === true) {
    summary.push({
      label: "Terminal",
      value: <span data-testid="exec-pty-chip">PTY</span>,
    });
  }
  if (process.processId) {
    summary.push({
      label: "Process",
      value: <code className={styles.code}>{process.processId}</code>,
      title: process.processId,
    });
  }

  const output = (
    <>
      {(content !== null || metadata?.transcript) && (
        <TranscriptClamp lineCount={content?.split("\n").length}>
          <ProcessOutputView
            content={content}
            transcript={metadata?.transcript}
          />
        </TranscriptClamp>
      )}
      {!hasRecordedOutcome && transcriptStatus === "unknown" && (
        <p className={styles.note}>Result not recorded; outcome unknown.</p>
      )}
      {!metadata && result && (
        <p className={styles.note}>
          Plain text result; structured process metadata was not available.
        </p>
      )}
      {listed.length > 0 && (
        <ul className={styles.processList} data-testid="exec-process-list">
          {listed.map((item) => (
            <li className={styles.processListItem} key={item.process_id}>
              <span className={styles.processLabel}>
                {item.short_description ?? item.command ?? item.process_id}
              </span>
              <code className={styles.code}>{item.process_id}</code>
              <ProcessStatusBadge status={processListItemStatus(item)} />
            </li>
          ))}
          {hiddenProcesses > 0 && (
            <li className={styles.note}>
              {hiddenProcesses} more processes hidden
            </li>
          )}
        </ul>
      )}
      {transcript !== undefined && (
        <TranscriptClamp>
          <p className={styles.note}>
            Retained in-memory transcript (up to 10,000 chunks). Not the full
            disk log.
          </p>
          <pre className={styles.outputPre}>
            {transcript.slice(0, MAX_TRANSCRIPT_CHARS) ||
              "No retained output returned."}
          </pre>
        </TranscriptClamp>
      )}
      {showStdin && process.processId && (
        <ProcessStdinInput processId={process.processId} />
      )}
    </>
  );

  const artifacts =
    pathReferences.length > 0 ? (
      <div className={styles.pathReferences} data-testid="exec-path-references">
        {pathReferences.map((reference) => {
          const label = reference.line1
            ? `${reference.target}:${reference.line1}`
            : reference.target;
          return (
            <Button
              className={styles.pathReference}
              disabled={!actions.canOpenInIde}
              key={`${label}:${reference.line2 ?? ""}`}
              size="sm"
              title={
                actions.canOpenInIde
                  ? label
                  : "Opening this path requires a supported IDE bridge"
              }
              type="button"
              variant="soft"
              onClick={() => actions.openPath(reference)}
            >
              {label}
            </Button>
          );
        })}
      </div>
    ) : null;

  const actionButtons = (
    <>
      {process.command && (
        <Button
          leftIcon={Copy}
          size="sm"
          variant="soft"
          onClick={() => copyToClipboard(process.command ?? "")}
        >
          Copy command
        </Button>
      )}
      {content && (
        <Button
          leftIcon={Copy}
          size="sm"
          variant="soft"
          onClick={() => copyToClipboard(content)}
        >
          Copy output
        </Button>
      )}
      {logPath && actions.canOpenInIde && (
        <Button
          leftIcon={FileText}
          size="sm"
          variant="soft"
          onClick={actions.openLog}
        >
          Open log in IDE
        </Button>
      )}
      {logPath && (
        <Button
          size="sm"
          title={
            actions.canOpenInIde
              ? logPath
              : "Full disk log unavailable here: opening it requires a supported IDE bridge"
          }
          variant="soft"
          onClick={() => copyToClipboard(logPath)}
        >
          Copy log path
        </Button>
      )}
      {process.processId && (
        <Button
          disabled={!actions.canReadTranscript || actions.state?.loading}
          size="sm"
          title={
            actions.canReadTranscript
              ? "Bounded in-memory transcript; not the full disk log"
              : "Requires a process ID, owning chat, and connected engine endpoint"
          }
          variant="soft"
          onClick={() => void actions.readTranscript()}
        >
          {actions.state?.loading
            ? "Reading transcript…"
            : "Read process transcript"}
        </Button>
      )}
      {transcript !== undefined && (
        <Button
          size="sm"
          variant="soft"
          onClick={() => copyToClipboard(transcript)}
        >
          Copy transcript
        </Button>
      )}
    </>
  );

  const stopButton = canStop ? (
    <Button
      disabled={actions.state?.stopping}
      leftIcon={Square}
      size="sm"
      variant="ghost"
      onClick={() => void actions.stop()}
    >
      Stop
    </Button>
  ) : null;

  return (
    <TranscriptItem
      actions={actionButtons}
      artifacts={artifacts}
      data-exec-process-id={process.processId}
      data-testid="exec-tool-card"
      error={
        actions.state?.error
          ? `Transcript/log unavailable: ${actions.state.error}`
          : null
      }
      data-exec-status={status}
      icon={
        toolName === "process_list" ? (
          <Rows3 data-testid={`exec-tool-${toolName}`} />
        ) : (
          <Code data-testid={`exec-tool-${toolName}`} />
        )
      }
      meta={meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      quickAction={stopButton}
      raw={[
        { label: "Arguments", value: toolCall.function.arguments },
        ...(foldedEvent?.payload
          ? [{ label: "Completion event", value: foldedEvent.payload }]
          : []),
      ]}
      status={transcriptStatus}
      statusLabel={
        transcriptStatus === "done" ? undefined : (
          <span data-testid={`exec-status-${status}`}>{statusText}</span>
        )
      }
      summary={summary}
      target={execTarget(toolName, args, process, metadata)}
      verb={execVerb(toolName)}
    />
  );
};

export default ExecToolCard;
