import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Box, Flex, Text } from "@radix-ui/themes";
import { CodeIcon, LapTimerIcon, RowsIcon } from "@radix-ui/react-icons";
import { FileText } from "lucide-react";
import classNames from "classnames";

import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { selectHost } from "../../../features/Config/configSlice";
import type {
  ExecOutputChunkMetadata,
  ExecProcessMetadata,
  ExecProcessStatus,
  ExecToolMetadata,
  ToolCall,
} from "../../../services/refact/types";
import {
  extractExecMetadata,
  isExecProcessStatus,
} from "../../../services/refact/types";
import { ideOpenFile } from "../../../hooks/useEventBusForIDE";
import { usePostMessage } from "../../../hooks/usePostMessage";
import { Button } from "../../ui";
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
  status: ExecProcessStatus;
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
): ExecProcessStatus {
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
  if (!hasResult) return "running";
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
  status: ExecProcessStatus,
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

function durationLabel(process: DisplayProcess, nowMs: number): string | null {
  if (
    (process.status === "running" ||
      process.status === "running_in_background" ||
      process.status === "starting") &&
    typeof process.startedAtMs === "number"
  ) {
    const elapsed = Math.max(0, nowMs - process.startedAtMs) / 1000;
    return `${elapsed.toFixed(0)}s running`;
  }
  if (typeof process.durationMs === "number") {
    return `${(process.durationMs / 1000).toFixed(1)}s`;
  }
  if (typeof process.startedAtMs !== "number") return null;
  const end = typeof process.endedAtMs === "number" ? process.endedAtMs : nowMs;
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

function isBusyStatus(status: ExecProcessStatus): boolean {
  return (
    status === "starting" ||
    status === "running" ||
    status === "running_in_background"
  );
}

function isTerminalStatus(status: ExecProcessStatus): boolean {
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

async function openLogInWeb(path: string): Promise<void> {
  const response = await fetch(path);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  window.open(url, "_blank", "noopener,noreferrer");
  window.setTimeout(() => URL.revokeObjectURL(url), 60000);
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
  const host = useAppSelector(selectHost);
  const postMessage = usePostMessage();
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, false);
  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const metadata = getExecMetadata(maybeResult?.extra);
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
  );
  const process = displayProcessFromMetadata(
    metadata,
    args,
    status,
    fallbackSummary,
  );
  const copyableOutput = useMemo(() => copyableOutputText(content), [content]);
  const isBusy = isBusyStatus(status);
  const livePreview = latestChunkPreview(metadata, content, toolCall);
  const nowMs = useRunningNowMs(isBusy);
  const duration = durationLabel(process, nowMs);
  const logPath = persistedOutputPath(metadata);
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
      if (!logPath) return;
      if (host === "web") {
        void openLogInWeb(logPath).catch(() => undefined);
        return;
      }
      postMessage(ideOpenFile({ file_path: logPath }));
    },
    [host, logPath, postMessage],
  );

  const icon = isBusy ? (
    <span className={styles.iconSpinner}>
      <span className={styles.spinnerDot} />
    </span>
  ) : toolName === "process_list" ? (
    <RowsIcon />
  ) : (
    <CodeIcon />
  );

  const summary = (
    <span className={styles.execTitle} title={process.shortDescription}>
      <span className={styles.summary}>{process.shortDescription}</span>
      <span className={styles.meta}>
        {meta && <span className={styles.metaText}>{meta}</span>}
        <ProcessStatusBadge status={status} />
        {metadata?.tty === true && (
          <Badge
            size="1"
            variant="soft"
            className={styles.ptyChip}
            data-testid="exec-pty-chip"
          >
            PTY
          </Badge>
        )}
        {process.processId && (
          <Badge size="1" variant="surface" className={styles.processChip}>
            {process.processId}
          </Badge>
        )}
      </span>
    </span>
  );

  return (
    <div data-testid="exec-tool-card" data-exec-process-id={process.processId}>
      <span data-testid={`exec-tool-${toolName}`} hidden />
      {isBusy && (
        <Text
          size="1"
          color={livePreview ? undefined : "gray"}
          className={styles.livePreview}
          data-testid="exec-live-preview"
          title={livePreview ?? undefined}
        >
          {livePreview ?? "Waiting for output…"}
        </Text>
      )}

      <ToolCard
        className={styles.card}
        icon={icon}
        summary={summary}
        status={isBusy ? "running" : maybeResult?.tool_failed ? "error" : "success"}
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

          {logPath && (
            <Flex gap="2" wrap="wrap" className={styles.controls}>
              <Button
                type="button"
                size="sm"
                variant="soft"
                className={styles.logButton}
                onClick={handleOpenLog}
                leftIcon={FileText}
              >
                Open log
              </Button>
            </Flex>
          )}

          <ProcessOutputView
            content={content}
            transcript={metadata?.transcript}
          />

          {listedProcesses.length > 0 && (
            <Box
              className={styles.processList}
              data-testid="exec-process-list"
            >
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
                      <Text
                        size="1"
                        color="gray"
                        className={styles.codeValue}
                      >
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

          {!metadata && (
            <Flex align="center" gap="1" mt="2">
              <LapTimerIcon />
              <Text size="1" color="gray">
                Plain text result; structured process metadata was not available.
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
