import type {
  EventMessage,
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
import {
  eventPayloadRecord,
  payloadNumber,
  payloadString,
} from "../EventRow/eventPayload";
import {
  fromProcessStatus,
  type TranscriptStatus,
} from "../TranscriptItem/statusVocabulary";

export type ExecToolName =
  | "shell"
  | "shell_service"
  | "process_start"
  | "process_list"
  | "process_read"
  | "process_kill"
  | "process_wait"
  | "process_write_stdin"
  | "exec";

export type ProcessArgs = {
  command?: string;
  workdir?: string;
  description?: string;
  process_id?: string;
  mode?: string;
  service_name?: string;
  action?: string;
  status?: string;
  scope?: string;
};

export type ExecDisplayStatus = ExecProcessStatus | "unknown";

export type DisplayProcess = {
  processId?: string;
  description?: string;
  status: ExecDisplayStatus;
  mode?: string;
  command?: string;
  cwd?: string | null;
  exitCode?: number | null;
  startedAtMs?: number;
  endedAtMs?: number | null;
  durationMs?: number;
};

const LIVE_HINT_CHARS = 60;

const VERBS: Record<ExecToolName, string> = {
  shell: "Run",
  exec: "Run",
  process_start: "Run",
  shell_service: "Service",
  process_read: "Read output",
  process_list: "Processes",
  process_wait: "Wait",
  process_kill: "Stop",
  process_write_stdin: "Send input",
};

const STATUS_LABELS: Record<ExecDisplayStatus, string> = {
  starting: "Starting",
  running: "Running",
  running_in_background: "Background",
  exited: "Exited",
  failed: "Failed",
  killed: "Killed",
  timed_out: "Timed out",
  unknown: "Unknown",
};

export function execVerb(toolName: ExecToolName): string {
  return VERBS[toolName];
}

export function execStatusLabel(status: ExecDisplayStatus): string {
  return STATUS_LABELS[status];
}

export function parseProcessArgs(args: string): ProcessArgs {
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

export function normalizeExecStatus(
  metadata: ExecToolMetadata | null,
  toolName: ExecToolName,
  hasResult: boolean,
  toolFailed: boolean | undefined,
  isRunning: boolean,
): ExecDisplayStatus {
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

export function execTranscriptStatus(
  status: ExecDisplayStatus,
  isRunning: boolean,
  toolFailed: boolean,
): TranscriptStatus {
  if (isRunning) return "running";
  if (toolFailed) return "failed";
  if (status === "running_in_background") return "waiting";
  if (status === "unknown") return "unknown";
  return fromProcessStatus(status);
}

export function displayProcess(
  metadata: ExecToolMetadata | null,
  args: ProcessArgs,
  status: ExecDisplayStatus,
): DisplayProcess {
  return {
    processId: metadata?.process_id ?? args.process_id,
    description: metadata?.short_description ?? args.description,
    status,
    mode: metadata?.mode ?? args.mode,
    command: metadata?.command ?? args.command,
    cwd: metadata?.cwd ?? args.workdir,
    exitCode: metadata?.exit_code,
    startedAtMs: metadata?.started_at_ms ?? metadata?.started_at,
    endedAtMs: metadata?.ended_at_ms ?? metadata?.ended_at,
    durationMs: metadata?.duration_ms,
  };
}

export function execTarget(
  toolName: ExecToolName,
  args: ProcessArgs,
  process: DisplayProcess,
  metadata: ExecToolMetadata | null,
): string {
  if (toolName === "process_list") {
    if (!metadata?.processes) return "all";
    const filter = [metadata.status_filter, metadata.scope_filter]
      .filter((item): item is string => Boolean(item))
      .join(" · ");
    return filter || "all";
  }
  if (toolName === "shell_service") {
    const action = args.action ?? "manage";
    return `${action} ${args.service_name ?? "service"}`;
  }
  return (
    process.description ?? process.command ?? process.processId ?? "process"
  );
}

export function formatSeconds(ms: number, digits: number): string {
  return `${(Math.max(0, ms) / 1000).toFixed(digits)}s`;
}

export function isBusyStatus(status: ExecDisplayStatus): boolean {
  return (
    status === "starting" ||
    status === "running" ||
    status === "running_in_background"
  );
}

export function durationText(
  process: DisplayProcess,
  nowMs: number,
  isLive: boolean,
): string | null {
  if (isBusyStatus(process.status) && typeof process.startedAtMs === "number") {
    return isLive ? formatSeconds(nowMs - process.startedAtMs, 0) : null;
  }
  if (typeof process.durationMs === "number") {
    return formatSeconds(process.durationMs, 1);
  }
  if (
    typeof process.startedAtMs !== "number" ||
    typeof process.endedAtMs !== "number"
  ) {
    return null;
  }
  return formatSeconds(process.endedAtMs - process.startedAtMs, 1);
}

function compactLine(text: string): string {
  const line = text.replace(/\s+/gu, " ").trim();
  return line.length > LIVE_HINT_CHARS
    ? `${line.slice(0, LIVE_HINT_CHARS)}…`
    : line;
}

export function execMeta(
  toolName: ExecToolName,
  process: DisplayProcess,
  metadata: ExecToolMetadata | null,
  duration: string | null,
  isRunning: boolean,
  liveHint: string | null,
): string {
  if (toolName === "process_list") {
    if (!metadata?.processes) return "";
    return `${metadata.count ?? metadata.processes.length} processes`;
  }
  if (
    toolName === "process_write_stdin" &&
    typeof metadata?.bytes_written === "number" &&
    typeof metadata.chunks_returned === "number"
  ) {
    return `Wrote ${metadata.bytes_written} bytes, got ${metadata.chunks_returned} new chunks`;
  }
  const parts: string[] = [];
  if (isRunning) {
    parts.push("running");
  } else if (typeof process.exitCode === "number") {
    parts.push(`exit ${process.exitCode}`);
  }
  if (duration) parts.push(duration);
  if (liveHint) parts.push(compactLine(liveHint));
  return parts.join(" · ");
}

function chunkPreviewText(chunk: ExecOutputChunkMetadata): string | null {
  if (typeof chunk.text !== "string") return null;
  const text = chunk.text.trim();
  return text ? text.split("\n").at(-1) ?? null : null;
}

function lastContentLine(content: string | null | undefined): string | null {
  if (!content) return null;
  const lines = content
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("```") && !line.endsWith(":"));
  return lines.at(-1) ?? null;
}

export function latestOutputLine(
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
    return lastContentLine(subchatLog[subchatLog.length - 1]);
  }
  return lastContentLine(content);
}

export function processListItemStatus(
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

export function persistedOutputPath(
  metadata: ExecToolMetadata | null,
): string | null {
  return (
    metadata?.persisted_output_path ??
    metadata?.transcript?.persisted_output_path ??
    null
  );
}

export type FoldedProcessOutcome = {
  status: ExecDisplayStatus;
  exitCode: number | null;
  durationMs: number | null;
};

const HEADER_LINE = /^([a-z_]+):\s?(.*)$/u;
const HEADER_NONE = "<none>";
const HEADER_SOME = /^Some\((.*)\)$/u;
const HEADER_ID_MAX_LINE = 1;

export type ProcessStartHeader = Readonly<Record<string, string | null>>;

export function parseProcessStartHeader(
  content: unknown,
): ProcessStartHeader | null {
  if (typeof content !== "string") return null;
  const fields: Record<string, string | null> = {};
  const lines = content.split("\n");
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index].trim();
    if (line === "") break;
    const match = HEADER_LINE.exec(line);
    if (!match) continue;
    if (match[1] === "process_id" && index > HEADER_ID_MAX_LINE) return null;
    const value = match[2].trim();
    const unwrapped = (HEADER_SOME.exec(value)?.[1] ?? value).trim();
    fields[match[1]] =
      value === HEADER_NONE || unwrapped === "" ? null : unwrapped;
  }
  return fields.process_id ? fields : null;
}

function headerStatus(header: ProcessStartHeader): ExecProcessStatus | null {
  const status = header.status;
  if (isExecProcessStatus(status)) return status;
  return status?.includes("failed") ? "failed" : null;
}

function headerNumber(header: ProcessStartHeader, key: string): number | null {
  const value = header[key];
  if (!value) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function headerText(
  header: ProcessStartHeader,
  key: string,
): string | undefined {
  return header[key] ?? undefined;
}

export function processIdFromStartResult(
  extra: Record<string, unknown> | undefined,
  content: unknown,
): string | null {
  const metadata = extractExecMetadata(extra);
  if (metadata && !Array.isArray(metadata.processes)) {
    const id = metadata.process_id?.trim();
    if (id) return id;
  }
  return parseProcessStartHeader(content)?.process_id ?? null;
}

export function execMetadataFromResult(
  extra: Record<string, unknown> | undefined,
  content: unknown,
): ExecToolMetadata | null {
  const metadata = extractExecMetadata(extra);
  if (metadata) return metadata;
  const header = parseProcessStartHeader(content);
  const processId = header?.process_id;
  const status = header ? headerStatus(header) : null;
  if (!header || !processId || !status) return null;
  return {
    process_id: processId,
    status,
    short_description: headerText(header, "short_description"),
    mode: headerText(header, "mode"),
    service_name: header.service_name ?? null,
    command: headerText(header, "command"),
    cwd: header.cwd ?? null,
    started_at_ms: headerNumber(header, "started_at") ?? undefined,
    exit_code: headerNumber(header, "exit_code"),
  };
}

export function foldedProcessOutcome(
  event: EventMessage | undefined,
): FoldedProcessOutcome | null {
  if (!event) return null;
  const payload = eventPayloadRecord(event);
  const rawStatus = payloadString(payload, "status");
  const exitCode = payloadNumber(payload, "exit_code");
  const nonZeroExit = exitCode !== null && exitCode !== 0;
  const status: ExecDisplayStatus =
    rawStatus === "exited" && nonZeroExit
      ? "failed"
      : isExecProcessStatus(rawStatus)
        ? rawStatus
        : rawStatus?.includes("failed")
          ? "failed"
          : exitCode === null
            ? "unknown"
            : nonZeroExit
              ? "failed"
              : "exited";
  return {
    status,
    exitCode,
    durationMs: payloadNumber(payload, "duration_ms"),
  };
}

export function withFoldedOutcome(
  process: DisplayProcess,
  outcome: FoldedProcessOutcome | null,
): DisplayProcess {
  if (!outcome) return process;
  return {
    ...process,
    status: outcome.status,
    exitCode: outcome.exitCode ?? process.exitCode,
    durationMs: outcome.durationMs ?? process.durationMs,
  };
}
