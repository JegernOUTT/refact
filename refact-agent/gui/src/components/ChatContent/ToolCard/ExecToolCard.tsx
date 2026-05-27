import React, { useEffect, useMemo, useState } from "react";
import { Badge, Box, Flex, Spinner, Text } from "@radix-ui/themes";
import { CodeIcon, LapTimerIcon, RowsIcon } from "@radix-ui/react-icons";
import classNames from "classnames";

import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import type {
  ExecProcessMetadata,
  ExecProcessStatus,
  ExecToolMetadata,
  ToolCall,
} from "../../../services/refact/types";
import {
  extractExecMetadata,
  isExecProcessStatus,
} from "../../../services/refact/types";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { useStoredOpen } from "../useStoredOpen";
import { ProcessStatusBadge } from "./ProcessStatusBadge";
import { ProcessOutputView } from "./ProcessOutputView";
import { ProcessControls } from "./ProcessControls";
import styles from "./ExecToolCard.module.css";

type ExecToolName =
  | "shell"
  | "shell_service"
  | "process_start"
  | "process_list"
  | "process_read"
  | "process_kill"
  | "process_wait"
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
  durationSecs?: number;
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
  hasResult: boolean,
  toolFailed: boolean | undefined,
): ExecProcessStatus {
  if (metadata && isExecProcessStatus(metadata.status)) return metadata.status;
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
    durationSecs: metadata?.duration_secs,
  };
}

function durationLabel(process: DisplayProcess, nowMs: number): string | null {
  if (typeof process.durationSecs === "number") {
    return `${process.durationSecs.toFixed(1)}s`;
  }
  if (typeof process.startedAtMs !== "number") return null;
  const end = typeof process.endedAtMs === "number" ? process.endedAtMs : nowMs;
  const elapsed = Math.max(0, end - process.startedAtMs) / 1000;
  return process.status === "running" || process.status === "starting"
    ? `${elapsed.toFixed(0)}s running`
    : `${elapsed.toFixed(1)}s`;
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

function processItemLabel(process: ExecProcessMetadata): string {
  return (
    process.short_description ??
    process.command ??
    process.process_id ??
    "process"
  );
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
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
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
  const isBusy = status === "starting" || status === "running";
  const nowMs = useRunningNowMs(isBusy);
  const duration = durationLabel(process, nowMs);
  const meta = [duration, listMeta(metadata)].filter(Boolean).join(" · ");
  const listedProcesses = metadata?.processes?.slice(0, 20) ?? [];
  const hiddenProcesses = Math.max(
    0,
    (metadata?.processes?.length ?? 0) - listedProcesses.length,
  );
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen,
    200,
    true,
  );
  const details = detailRows(process);

  const header = (
    <Flex
      className={styles.header}
      align="center"
      gap="2"
      onClick={handleToggle}
    >
      <span className={styles.icon}>
        {isBusy ? (
          <Spinner size="1" />
        ) : toolName === "process_list" ? (
          <RowsIcon />
        ) : (
          <CodeIcon />
        )}
      </span>
      <Text
        size="1"
        className={styles.summary}
        title={process.shortDescription}
      >
        {process.shortDescription}
      </Text>
      <Flex className={styles.meta} align="center" gap="2">
        {meta && (
          <Text size="1" color="gray">
            {meta}
          </Text>
        )}
        <ProcessStatusBadge status={status} />
        {process.processId && (
          <Badge size="1" variant="surface" className={styles.processChip}>
            {process.processId}
          </Badge>
        )}
      </Flex>
    </Flex>
  );

  return (
    <div className={styles.card} data-testid="exec-tool-card">
      <span data-testid={`exec-tool-${toolName}`} hidden />
      <ToolCallTooltip toolCall={toolCall}>{header}</ToolCallTooltip>

      {shouldRender && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
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
                      <ProcessStatusBadge
                        status={
                          isExecProcessStatus(item.status)
                            ? item.status
                            : "exited"
                        }
                      />
                    </Flex>
                  ))}
                  {hiddenProcesses > 0 && (
                    <Text size="1" color="gray">
                      {hiddenProcesses} more processes hidden
                    </Text>
                  )}
                </Box>
              )}

              <ProcessOutputView
                content={content}
                transcript={metadata?.transcript}
              />

              {!metadata && (
                <Flex align="center" gap="1" mt="2">
                  <LapTimerIcon />
                  <Text size="1" color="gray">
                    Plain text result; structured process metadata was not
                    available.
                  </Text>
                </Flex>
              )}
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

export default ExecToolCard;
