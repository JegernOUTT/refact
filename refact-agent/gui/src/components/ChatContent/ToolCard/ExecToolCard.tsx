import { Code, Timer, Rows3 } from "lucide-react";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { FileText } from "lucide-react";
import classNames from "classnames";

import { useAppSelector } from "../../../hooks/useAppSelector";
import {
  selectConfig,
  selectCapabilities,
} from "../../../features/Config/configSlice";
import { useToolCallActivity } from "./useToolCallActivity";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../../../services/refact/apiUrl";
import { readExec } from "../../../services/refact/exec";
import type {
  ExecOutputChunkMetadata,
  ExecProcessMetadata,
  ExecProcessStatus,
  ExecToolMetadata,
  ToolCall,
  ToolEnrichmentReference,
} from "../../../services/refact/types";
import {
  extractExecMetadata,
  getToolEnrichmentPathReferences,
  isExecProcessStatus,
} from "../../../services/refact/types";
import { ideOpenFile } from "../../../hooks/useEventBusForIDE";
import { usePostMessage } from "../../../hooks/usePostMessage";
import { Badge, Button } from "../../ui";
import { ToolCard } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ProcessStatusBadge } from "./ProcessStatusBadge";
import { ProcessControls } from "./ProcessControls";
import { ProcessOutputView } from "./ProcessOutputView";
import { ProcessStdinInput } from "./ProcessStdinInput";
import styles from "./ExecToolCard.module.css";

type ExecToolName =
  | "shell"
  | "shell_service"
  | "process_start"
  | "process_list"
  | "process_read"
  | "process_kill"
  | "process_wait"
  | "process_write_stdin"
  | "exec";

type ExecToolCardProps = {
  toolCall: ToolCall;
  toolName: ExecToolName;
};

type ShellArgs = {
  command?: string;
  workdir?: string;
  description?: string;
};

type ProcessArgs = ShellArgs & {
  process_id?: string;
  mode?: string;
  service_name?: string;
  action?: string;
  status?: string;
  scope?: string;
};

type DisplayProcess = {
  processId?: string;
  shortDescription: string;
  status: ExecProcessStatus | "unknown";
  mode?: string;
  command?: string;
  cwd?: string | null;
  exitCode?: number | null;
  startedAtMs?: number;
  endedAtMs?: number | null;
  durationMs?: number;
};

function parseArgs(args: string): ProcessArgs {
  try {
    const parsed = JSON.parse(args) as unknown;
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return {};
    }
    return parsed as ProcessArgs;
  } catch {
    return {};
  }
}

function getExecMetadata(
  extra: Record<string, unknown> | undefined,
): ExecToolMetadata | null {
  return extractExecMetadata(extra) ?? null;
}

function normalizeStatus(
  metadata: ExecToolMetadata | null,
  toolName: ExecToolName,
  hasResult: boolean,
  toolFailed: boolean | undefined,
  isRunning: boolean,
): DisplayProcess["status"] {
  if (metadata && isExecProcessStatus(metadata.status)) {
    if (
      metadata.status === "running" &&
      (metadata.mode === "background" ||
        metadata.mode === "service" ||
        toolName === "process_start")
    ) {
      return "running_in_background";
    }
    return metadata.status;
  }
  if (!hasResult) return isRunning ? "running" : "unknown";
  return toolFailed ? "failed" : "exited";
}

function summarizeTool(toolName: ExecToolName, args: ProcessArgs): string {
  switch (toolName) {
    case "shell":
      return args.command ? `Run ${args.command}` : "Run shell command";
    case "shell_service": {
      const action = args.action ?? "manage";
      return `${action.charAt(0).toUpperCase()}${action.slice(1)} ${
        args.service_name ?? "service"
      }`;
    }
    case "process_start":
      return args.command ? `Start ${args.command}` : "Start process";
    case "process_list":
      return "List processes";
    case "process_read":
      return `Read ${args.process_id ?? "process"}`;
    case "process_kill":
      return `Kill ${args.process_id ?? "process"}`;
    case "process_wait":
      return `Wait for ${args.process_id ?? "process"}`;
    case "process_write_stdin":
      return `Write stdin to ${args.process_id ?? "process"}`;
    case "exec":
      return args.command ? `Run ${args.command}` : "Process";
  }
}

function displayProcessFromMetadata(
  metadata: ExecToolMetadata | null,
  args: ProcessArgs,
  status: DisplayProcess["status"],
  fallbackSummary: string,
): DisplayProcess {
  const startedAtMs = metadata?.started_at_ms ?? metadata?.started_at;
  const endedAtMs = metadata?.ended_at_ms ?? metadata?.ended_at;

  return {
    processId: metadata?.process_id ?? args.process_id,
    shortDescription:
      metadata?.short_description ?? args.description ?? fallbackSummary,
    status,
    mode: metadata?.mode ?? args.mode,
    command: metadata?.command ?? args.command,
    cwd: metadata?.cwd ?? args.workdir,
    exitCode: metadata?.exit_code,
    startedAtMs,
    endedAtMs,
    durationMs: metadata?.duration_ms,
  };
}

function durationLabel(
  process: DisplayProcess,
  nowMs: number,
  isLive: boolean,
): string | null {
  if (
    (process.status === "running" ||
      process.status === "running_in_background" ||
      process.status === "starting") &&
    typeof process.startedAtMs === "number"
  ) {
    if (!isLive) return null;
    const elapsed = Math.max(0, nowMs - process.startedAtMs) / 1000;
    return `${elapsed.toFixed(0)}s running`;
  }
  if (typeof process.durationMs === "number") {
    return `${(process.durationMs / 1000).toFixed(1)}s`;
  }
  if (
    typeof process.startedAtMs !== "number" ||
    typeof process.endedAtMs !== "number"
  )
    return null;
  const end = process.endedAtMs;
  const elapsed = Math.max(0, end - process.startedAtMs) / 1000;
  return `${elapsed.toFixed(1)}s`;
}

function detailRows(
  process: DisplayProcess,
): { label: string; value: string; code?: boolean }[] {
  const rows: { label: string; value: string; code?: boolean }[] = [];
  if (process.command)
    rows.push({ label: "Command", value: process.command, code: true });
  if (process.cwd) rows.push({ label: "CWD", value: process.cwd, code: true });
  if (process.mode) rows.push({ label: "Mode", value: process.mode });
  if (process.exitCode !== undefined && process.exitCode !== null) {
    rows.push({ label: "Exit code", value: String(process.exitCode) });
  }
  return rows;
}

function listMeta(metadata: ExecToolMetadata | null): string | null {
  if (!metadata?.processes) return null;
  const count = metadata.count ?? metadata.processes.length;
  const filter = [metadata.status_filter, metadata.scope_filter]
    .filter(
      (item): item is string => typeof item === "string" && item.length > 0,
    )
    .join(" · ");
  return filter ? `${count} processes · ${filter}` : `${count} processes`;
}

function writeStdinMeta(
  toolName: ExecToolName,
  metadata: ExecToolMetadata | null,
): string | null {
  if (toolName !== "process_write_stdin") return null;
  if (typeof metadata?.bytes_written !== "number") return null;
  if (typeof metadata.chunks_returned !== "number") return null;
  return `Wrote ${metadata.bytes_written} bytes, got ${metadata.chunks_returned} new chunks`;
}

function persistedOutputPath(metadata: ExecToolMetadata | null): string | null {
  return (
    metadata?.persisted_output_path ??
    metadata?.transcript?.persisted_output_path ??
    null
  );
}

function isBusyStatus(status: DisplayProcess["status"]): boolean {
  return (
    status === "starting" ||
    status === "running" ||
    status === "running_in_background"
  );
}

function isTerminalStatus(status: DisplayProcess["status"]): boolean {
  return !isBusyStatus(status);
}

function chunkPreviewText(chunk: ExecOutputChunkMetadata): string | null {
  if (typeof chunk.text !== "string") return null;
  const text = chunk.text.replace(/\s+/gu, " ").trim();
  if (!text) return null;
  const stream = typeof chunk.stream === "string" ? chunk.stream : null;
  return stream && stream !== "combined" ? `${stream}: ${text}` : text;
}

function stripOutputFence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed.startsWith("```")) return text.trim();
  return trimmed
    .replace(/^```[a-zA-Z0-9_-]*\n?/, "")
    .replace(/```$/, "")
    .trim();
}

function extractOutputSection(content: string, title: string): string | null {
  const pattern = new RegExp(
    `(?:^|\\n)${title}:\\n([\\s\\S]*?)(?=\\n(?:stdout|stderr|combined|transcript):|$)`,
    "i",
  );
  const match = pattern.exec(content);
  if (!match) return null;
  const value = stripOutputFence(match[1]);
  return value === "<empty>" ? "" : value;
}

function sectionPreviewText(
  stream: string,
  text: string | null,
): string | null {
  if (!text) return null;
  const line = text
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean)
    .at(-1);
  if (!line) return null;
  return stream === "combined" ? line : `${stream}: ${line}`;
}

function contentPreviewText(content: string | null): string | null {
  if (!content) return null;
  return (
    sectionPreviewText("combined", extractOutputSection(content, "combined")) ??
    sectionPreviewText("stderr", extractOutputSection(content, "stderr")) ??
    sectionPreviewText("stdout", extractOutputSection(content, "stdout"))
  );
}

function latestChunkPreview(
  metadata: ExecToolMetadata | null,
  content: string | null,
  toolCall: ToolCall,
): string | null {
  const chunks = metadata?.transcript?.chunks;
  if (Array.isArray(chunks)) {
    for (const chunk of chunks.slice().reverse()) {
      const text = chunkPreviewText(chunk);
      if (text) return text;
    }
  }
  const subchatLog = toolCall.subchat_log;
  if (subchatLog && subchatLog.length > 0) {
    return contentPreviewText(subchatLog[subchatLog.length - 1] ?? "");
  }
  return contentPreviewText(content);
}

function processItemLabel(process: ExecProcessMetadata): string {
  return (
    process.short_description ??
    process.command ??
    process.process_id ??
    "process"
  );
}

function processListItemStatus(
  process: ExecProcessMetadata,
): ExecProcessStatus {
  if (!isExecProcessStatus(process.status)) return "exited";
  if (
    process.status === "running" &&
    (process.mode === "background" || process.mode === "service")
  ) {
    return "running_in_background";
  }
  return process.status;
}

function copyableOutputText(content: string | null): string | undefined {
  if (!content) return undefined;
  return content;
}

function pathReferenceLabel(reference: ToolEnrichmentReference): string {
  return reference.line1
    ? `${reference.target}:${reference.line1}`
    : reference.target;
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
  const config = useAppSelector(selectConfig);
  const capabilities = useAppSelector(selectCapabilities);
  const host = config.host;
  const postMessage = usePostMessage();
  const copyToClipboard = useCopyToClipboard();
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, false);
  const {
    result: maybeResult,
    isRunning,
    threadId,
  } = useToolCallActivity(toolCall.id);
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const metadata = getExecMetadata(maybeResult?.extra);
  const pathReferences = getToolEnrichmentPathReferences(maybeResult?.extra);
  const args = useMemo(
    () => parseArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const fallbackSummary = useMemo(
    () => summarizeTool(toolName, args),
    [toolName, args],
  );
  const status = normalizeStatus(
    metadata,
    toolName,
    Boolean(maybeResult),
    maybeResult?.tool_failed,
    isRunning,
  );
  const process = displayProcessFromMetadata(
    metadata,
    args,
    status,
    fallbackSummary,
  );
  const copyableOutput = useMemo(() => copyableOutputText(content), [content]);
  const isBusy = isRunning;
  const livePreview = latestChunkPreview(metadata, content, toolCall);
  const showLivePreview =
    (isBusy || isBusyStatus(status)) && livePreview !== null;
  const nowMs = useRunningNowMs(isBusy);
  const duration = durationLabel(process, nowMs, isBusy);
  const logPath = persistedOutputPath(metadata);
  const endpointIdentity = getEngineEndpointIdentity(config);
  const canReadTranscript = Boolean(
    process.processId && threadId && hasUsableEngineEndpoint(config),
  );
  const canOpenInIde =
    capabilities.openFileInIde &&
    ((host === "vscode" && typeof window.acquireVsCodeApi === "function") ||
      (host === "jetbrains" &&
        typeof window.postIntellijMessage === "function") ||
      (host === "ide" &&
        (typeof window.acquireVsCodeApi === "function" ||
          typeof window.postIntellijMessage === "function")));
  const source = useMemo(
    () => ({
      endpointIdentity,
      threadId,
      toolCallId: toolCall.id,
      processId: process.processId,
      logPath,
      apiKey: config.apiKey,
      canReadTranscript,
      canOpenInIde,
    }),
    [
      endpointIdentity,
      threadId,
      toolCall.id,
      process.processId,
      logPath,
      config.apiKey,
      canReadTranscript,
      canOpenInIde,
    ],
  );
  const sourceRef = useRef<object | null>(source);
  sourceRef.current = source;
  const [logState, setLogState] = useState<{
    source: object;
    loading?: boolean;
    content?: string;
    error?: string;
  } | null>(null);
  const currentLogState = logState?.source === source ? logState : null;
  useEffect(() => {
    sourceRef.current = source;
    return () => {
      sourceRef.current = null;
    };
  }, [source]);
  const meta = [
    duration,
    writeStdinMeta(toolName, metadata),
    listMeta(metadata),
  ]
    .filter(Boolean)
    .join(" · ");
  const listedProcesses = metadata?.processes?.slice(0, 20) ?? [];
  const hiddenProcesses = Math.max(
    0,
    (metadata?.processes?.length ?? 0) - listedProcesses.length,
  );
  const details = detailRows(process);
  const showStdinInput = Boolean(
    process.processId && metadata?.tty === true && !isTerminalStatus(status),
  );
  const handleOpenLog = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (!logPath || !canOpenInIde) return;
      try {
        postMessage(ideOpenFile({ file_path: logPath }));
      } catch (cause) {
        setLogState({
          source,
          error: cause instanceof Error ? cause.message : String(cause),
        });
      }
    },
    [canOpenInIde, logPath, postMessage, source],
  );
  const handleReadTranscript = useCallback(async () => {
    if (!canReadTranscript || !process.processId || currentLogState?.loading)
      return;
    setLogState({ source, loading: true });
    try {
      const response = await readExec(
        process.processId,
        0,
        config,
        threadId,
        config.apiKey ?? undefined,
      );
      if (sourceRef.current !== source) return;
      const content = response.chunks.map((chunk) => chunk.text).join("");
      setLogState({ source, content });
    } catch (cause) {
      if (sourceRef.current !== source) return;
      setLogState({
        source,
        error: cause instanceof Error ? cause.message : String(cause),
      });
    }
  }, [
    canReadTranscript,
    config,
    currentLogState?.loading,
    process.processId,
    source,
    threadId,
  ]);
  const handleOpenPath = useCallback(
    (
      event: React.MouseEvent<HTMLButtonElement>,
      reference: ToolEnrichmentReference,
    ) => {
      event.stopPropagation();
      if (!canOpenInIde) return;
      postMessage(
        ideOpenFile({
          file_path: reference.target,
          line: reference.line1 ?? undefined,
        }),
      );
    },
    [canOpenInIde, postMessage],
  );

  const icon = toolName === "process_list" ? <Rows3 /> : <Code />;

  const summaryText = isBusy ? (
    <span className="rf-text-shimmer">{process.shortDescription}</span>
  ) : (
    process.shortDescription
  );

  const summary = (
    <span className={styles.execTitle} title={process.shortDescription}>
      <span className={styles.summary}>{summaryText}</span>
      <span className={styles.meta}>
        {meta && <span className={styles.metaText}>{meta}</span>}
        {maybeResult && <span>Recorded</span>}
        <span className={styles.recordedStatus}>
          <ProcessStatusBadge status={status} />
        </span>
        {metadata?.tty === true && (
          <Badge
            tone="accent"
            className={styles.ptyChip}
            data-testid="exec-pty-chip"
          >
            PTY
          </Badge>
        )}
        {process.processId && (
          <Badge tone="muted" className={styles.processChip}>
            {process.processId}
          </Badge>
        )}
      </span>
    </span>
  );

  return (
    <div data-testid="exec-tool-card" data-exec-process-id={process.processId}>
      <span data-testid={`exec-tool-${toolName}`} hidden />
      {showLivePreview && (
        <Text
          size="1"
          className={styles.livePreview}
          data-testid="exec-live-preview"
          title={livePreview}
        >
          <span className={isBusy ? "rf-text-shimmer" : undefined}>
            {!isBusy && "Recorded output: "}
            {livePreview}
          </span>
        </Text>
      )}

      <ToolCard
        className={styles.card}
        icon={icon}
        summary={summary}
        status={
          isBusy
            ? "running"
            : maybeResult?.tool_failed
              ? "error"
              : maybeResult
                ? "success"
                : "idle"
        }
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        <Box className={styles.content}>
          {details.length > 0 && (
            <Box className={styles.detailsGrid}>
              {details.map((row) => (
                <React.Fragment key={row.label}>
                  <Text size="1" className={styles.detailLabel}>
                    {row.label}
                  </Text>
                  <Text
                    size="1"
                    className={classNames(
                      styles.detailValue,
                      row.code && styles.codeValue,
                    )}
                    title={row.value}
                  >
                    {row.value}
                  </Text>
                </React.Fragment>
              ))}
            </Box>
          )}

          <ProcessControls
            command={process.command}
            output={copyableOutput}
            processId={process.processId}
          />

          {!maybeResult && !isRunning && (
            <Text size="1" color="gray">
              Result not recorded; process outcome unknown.
            </Text>
          )}

          {pathReferences.length > 0 && (
            <Flex
              gap="1"
              wrap="wrap"
              className={styles.pathReferences}
              data-testid="exec-path-references"
            >
              {pathReferences.map((reference) => (
                <Button
                  key={`${reference.target}:${reference.line1 ?? ""}:${
                    reference.line2 ?? ""
                  }`}
                  type="button"
                  size="sm"
                  variant="soft"
                  disabled={!canOpenInIde}
                  className={styles.pathReference}
                  onClick={(event) => handleOpenPath(event, reference)}
                  title={
                    canOpenInIde
                      ? pathReferenceLabel(reference)
                      : "Opening this path requires a supported IDE bridge"
                  }
                >
                  {pathReferenceLabel(reference)}
                </Button>
              ))}
            </Flex>
          )}

          {(Boolean(logPath) || Boolean(process.processId)) && (
            <Flex direction="column" gap="2">
              <Flex gap="2" wrap="wrap" className={styles.controls}>
                {logPath && canOpenInIde && (
                  <Button
                    type="button"
                    size="sm"
                    variant="soft"
                    className={styles.logButton}
                    onClick={handleOpenLog}
                    leftIcon={FileText}
                  >
                    Open log in IDE
                  </Button>
                )}
                {logPath && (
                  <Button
                    type="button"
                    size="sm"
                    variant="soft"
                    onClick={() => copyToClipboard(logPath)}
                  >
                    Copy log path
                  </Button>
                )}
                <Button
                  type="button"
                  size="sm"
                  variant="soft"
                  disabled={!canReadTranscript || currentLogState?.loading}
                  onClick={() => {
                    void handleReadTranscript();
                  }}
                >
                  {currentLogState?.loading
                    ? "Reading transcript…"
                    : "Read process transcript"}
                </Button>
              </Flex>
              <Text size="1" color="gray">
                Bounded in-memory transcript (up to 10,000 retained chunks);
                older output may be missing. Not the full disk log.
              </Text>
              {logPath && !canOpenInIde && (
                <Text size="1" color="gray">
                  Full disk log unavailable here: opening its path requires a
                  supported IDE bridge.
                </Text>
              )}
              {!canReadTranscript && (
                <Text size="1" color="gray">
                  Transcript unavailable: requires a process ID, owning chat,
                  and connected engine endpoint.
                </Text>
              )}
              {currentLogState?.error && (
                <Text size="1" color="red" role="alert">
                  Transcript/log unavailable: {currentLogState.error}
                </Text>
              )}
              {currentLogState?.content !== undefined && (
                <>
                  <Text size="1" weight="medium">
                    Retained process transcript
                  </Text>
                  <Button
                    type="button"
                    size="sm"
                    variant="soft"
                    onClick={() =>
                      copyToClipboard(currentLogState.content ?? "")
                    }
                  >
                    Copy transcript
                  </Button>
                  {currentLogState.content.length > 30_000 && (
                    <Text size="1" color="gray">
                      Preview capped at 30,000 characters; Copy transcript
                      retains all returned text.
                    </Text>
                  )}
                  <Box className={styles.outputBody}>
                    <pre className={styles.outputPre}>
                      {currentLogState.content.slice(0, 30_000) ||
                        "No retained output returned."}
                    </pre>
                  </Box>
                </>
              )}
            </Flex>
          )}

          <ProcessOutputView
            content={content}
            transcript={metadata?.transcript}
          />

          {listedProcesses.length > 0 && (
            <Box
              className={classNames(styles.processList, styles.recordedStatus)}
              data-testid="exec-process-list"
            >
              <Text size="1" color="gray">
                Recorded process list
              </Text>
              {listedProcesses.map((item) => (
                <Flex
                  key={item.process_id}
                  className={styles.processListItem}
                  align="center"
                  justify="between"
                  gap="2"
                >
                  <Flex direction="column" gap="1">
                    <Text size="1" weight="medium">
                      {processItemLabel(item)}
                    </Text>
                    {item.process_id && (
                      <Text size="1" color="gray" className={styles.codeValue}>
                        {item.process_id}
                      </Text>
                    )}
                  </Flex>
                  <ProcessStatusBadge status={processListItemStatus(item)} />
                </Flex>
              ))}
              {hiddenProcesses > 0 && (
                <Text size="1" color="gray">
                  {hiddenProcesses} more processes hidden
                </Text>
              )}
            </Box>
          )}

          {!metadata && maybeResult && (
            <Flex align="center" gap="1" mt="2">
              <Timer />
              <Text size="1" color="gray">
                Plain text result; structured process metadata was not
                available.
              </Text>
            </Flex>
          )}

          {showStdinInput && process.processId && (
            <ProcessStdinInput processId={process.processId} />
          )}
        </Box>
      </ToolCard>
    </div>
  );
};

export default ExecToolCard;
