import { isRecoveryRecord, type RecoveryRecord } from "./chatSubscription";
import type { ChatMessages, ErrorMessage } from "./types";
import { humanizeIdentifier } from "../../utils/displayNames";

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function nonempty(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

export function recoveryRecordFromError(
  error: ErrorMessage,
): RecoveryRecord | null {
  const nested = error.extra?.recovery_record;
  if (isRecoveryRecord(nested)) return nested;
  const flattened = object(error)?.recovery_record;
  return isRecoveryRecord(flattened) ? flattened : null;
}

function recoveryExtra(error: ErrorMessage, key: string): string | null {
  const nested = error.extra?.[key];
  if (nonempty(nested)) return nested;
  const flattened = object(error)?.[key];
  return nonempty(flattened) ? flattened : null;
}

export function recoveryOperationIdFromError(
  error: ErrorMessage,
): string | null {
  return (
    recoveryExtra(error, "recovery_operation_id") ??
    recoveryRecordFromError(error)?.operation_id ??
    null
  );
}

export function recoveryRunIdFromError(error: ErrorMessage): string | null {
  return (
    recoveryExtra(error, "recovery_run_id") ??
    recoveryRecordFromError(error)?.run_id ??
    null
  );
}

export function errorRepeatCount(error: ErrorMessage): number {
  const count = error.repeat_count ?? error.extra?.repeat_count;
  return typeof count === "number" && Number.isSafeInteger(count) && count > 1
    ? count
    : 1;
}

export function normalizeRecoveryErrorMetadata(
  error: ErrorMessage,
): ErrorMessage {
  const flattened = object(error);
  const recovery = recoveryRecordFromError(error);
  const operationId = recoveryOperationIdFromError(error);
  const runId = recoveryRunIdFromError(error);
  if (!flattened || (!recovery && !operationId && !runId)) return error;
  return {
    ...error,
    extra: {
      ...error.extra,
      ...(recovery ? { recovery_record: recovery } : {}),
      ...(runId ? { recovery_run_id: runId } : {}),
      ...(operationId ? { recovery_operation_id: operationId } : {}),
      ...(typeof flattened._ui_only === "boolean"
        ? { _ui_only: flattened._ui_only }
        : {}),
    },
  };
}

export function recoveryStatusLabel(
  recovery: RecoveryRecord,
  liveRunId?: string | null,
): string | null {
  if (recovery.state === "succeeded") {
    const count = recovery.retries_used;
    if (count === 0) return "Recovered";
    const unit = recovery.manual
      ? count === 1
        ? "new request"
        : "new requests"
      : count === 1
        ? "retry"
        : "retries";
    return `Recovered after ${count} ${unit}`;
  }
  if (recovery.state === "waiting" || recovery.state === "attempting")
    return recovery.run_id === liveRunId
      ? null
      : humanizeIdentifier("interrupted");
  return humanizeIdentifier(recovery.terminal_reason ?? recovery.state);
}

function messageDiagnostics(error: ErrorMessage): {
  history: string[];
  own: string[];
} {
  const recovery = recoveryRecordFromError(error);
  if (recovery) {
    const info = object(error.error_info) ?? object(error.extra?.error_info);
    const labels = recoveryLabels(recovery);
    const redundant = new Set<unknown>([
      ...labels,
      info?.title,
      info?.explanation,
    ]);
    const own = [
      ...(nonempty(info?.raw_error) && !labels.has(info.raw_error)
        ? [info.raw_error]
        : []),
      ...(nonempty(error.content) && !redundant.has(error.content)
        ? [error.content]
        : []),
    ];
    if (own.length > 0) return { history: [], own };
    const recorded = recoveryDiagnostics(recovery);
    return { history: recorded.slice(0, -1), own: recorded.slice(-1) };
  }
  return { history: [], own: plainDiagnostics(error) };
}

function plainDiagnostics(error: ErrorMessage): string[] {
  const info = object(error.error_info) ?? object(error.extra?.error_info);
  if (info) {
    const raw = info.raw_error;
    const texts = nonempty(raw) ? [raw] : [];
    if (
      nonempty(error.content) &&
      (!texts.length ||
        (error.content !== info.title && error.content !== info.explanation))
    ) {
      texts.push(error.content);
    }
    return texts;
  }

  try {
    const parsed = object(JSON.parse(error.content));
    const nested = object(parsed?.error);
    const structured = nested ?? parsed;
    const structuredInfo = object(structured?.error_info);
    if (nonempty(structuredInfo?.raw_error)) {
      return [
        structuredInfo.raw_error,
        ...(nonempty(structured?.message) &&
        structured.message !== structuredInfo.title &&
        structured.message !== structuredInfo.explanation
          ? [structured.message]
          : []),
      ];
    }
    if (nonempty(structured?.message)) return [structured.message];
  } catch {
    return nonempty(error.content) ? [error.content] : [];
  }
  return nonempty(error.content) ? [error.content] : [];
}

function recoveryLabels(recovery: RecoveryRecord): Set<string> {
  return new Set(
    [
      recovery.latest_error,
      ...recovery.attempts.map((attempt) => attempt.error),
    ].flatMap((error) => (error ? [error.title, error.explanation] : [])),
  );
}

function recoveryErrorDiagnostic(error: RecoveryRecord["latest_error"]) {
  return nonempty(error.raw_error)
    ? error.raw_error
    : nonempty(error.explanation)
      ? error.explanation
      : error.title;
}

function recoveryDiagnostics(recovery: RecoveryRecord): string[] {
  return [
    ...recovery.attempts.flatMap((attempt) =>
      attempt.error ? [recoveryErrorDiagnostic(attempt.error)] : [],
    ),
    recoveryErrorDiagnostic(recovery.latest_error),
  ];
}

export function recoveryStatusHosts(messages: ChatMessages): Set<ErrorMessage> {
  const latest = new Map<string, ErrorMessage>();
  messages.forEach((message) => {
    if (message.role !== "error") return;
    const record = recoveryRecordFromError(message);
    const key = recoveryOperationIdFromError(message) ?? record?.run_id;
    if (key) latest.set(key, message);
  });
  return new Set(
    Array.from(latest.values()).filter(
      (message) => recoveryRecordFromError(message) !== null,
    ),
  );
}

export function errorRecoveryStatuses(
  errors: ErrorMessage[],
  hosts: ReadonlySet<ErrorMessage> | undefined,
  liveRunId?: string | null,
): string[] {
  return Array.from(
    new Set(
      errors.flatMap((error) => {
        if (hosts && !hosts.has(error)) return [];
        const record = recoveryRecordFromError(error);
        const label = record && recoveryStatusLabel(record, liveRunId);
        return label ? [label] : [];
      }),
    ),
  );
}

export type ErrorTextEntry = { text: string; repeat: number };

export function collectErrorEntries(
  errors: ErrorMessage[],
  recoveries: RecoveryRecord[] = [],
): ErrorTextEntry[] {
  const counts = new Map<string, number>();
  const add = (texts: string[], repeat: number) =>
    texts
      .filter(nonempty)
      .forEach((text) => counts.set(text, (counts.get(text) ?? 0) + repeat));
  errors.forEach((error) => {
    const { history, own } = messageDiagnostics(error);
    add(history, 0);
    add(Array.from(new Set(own)), errorRepeatCount(error));
  });
  recoveries.forEach((recovery) => add(recoveryDiagnostics(recovery), 0));
  return Array.from(counts, ([text, repeat]) => ({
    text,
    repeat: Math.max(repeat, 1),
  }));
}

export function collectErrorTexts(
  errors: ErrorMessage[],
  recoveries: RecoveryRecord[] = [],
): string[] {
  return collectErrorEntries(errors, recoveries).map((entry) => entry.text);
}

export function partitionTranscriptErrors(
  messages: ChatMessages,
  recovery: RecoveryRecord | null | undefined,
): { episode: ErrorMessage[]; boundary: number } {
  let boundary = -1;
  messages.forEach((message, index) => {
    if (
      message.role === "user" ||
      (message.role === "assistant" &&
        message.extra?._ui_only !== true &&
        message.extra?.interrupted !== true)
    )
      boundary = index;
  });
  if (!recovery) return { episode: [], boundary };
  const live = recovery.state !== "succeeded";
  const linked = (message: ErrorMessage) =>
    recoveryOperationIdFromError(message) === recovery.operation_id ||
    recoveryRunIdFromError(message) === recovery.run_id ||
    recoveryRecordFromError(message)?.run_id === recovery.run_id;
  const errors = messages.flatMap((message, index) =>
    message.role === "error" &&
    recoveryRecordFromError(message)?.state !== "succeeded"
      ? [{ message, index }]
      : [],
  );
  const episode = errors
    .filter(({ message, index }) =>
      recoveryOperationIdFromError(message) === null &&
      recoveryRunIdFromError(message) === null
        ? live && index > boundary
        : linked(message),
    )
    .map(({ message }) => message);
  return { episode, boundary };
}

function retryableErrors(
  errors: ErrorMessage[],
  episode: readonly ErrorMessage[],
): ErrorMessage[] {
  return errors.filter(
    (error) =>
      recoveryRecordFromError(error)?.state !== "succeeded" &&
      !episode.includes(error),
  );
}

export function retryableErrorIndex(
  items: readonly { messageIndex: number; errors: ErrorMessage[] }[],
  boundary: number,
  episode: readonly ErrorMessage[],
): number {
  return items.reduce(
    (last, item) =>
      item.messageIndex > boundary &&
      collectErrorTexts(retryableErrors(item.errors, episode)).length > 0
        ? Math.max(last, item.messageIndex)
        : last,
    -1,
  );
}
