import { isRecoveryRecord, type RecoveryRecord } from "./chatSubscription";
import type { ChatMessages, ErrorMessage } from "./types";

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

export function normalizeRecoveryErrorMetadata(
  error: ErrorMessage,
): ErrorMessage {
  const flattened = object(error);
  const recovery = recoveryRecordFromError(error);
  if (!recovery || !flattened) return error;
  return {
    ...error,
    extra: {
      ...error.extra,
      recovery_record: recovery,
      recovery_run_id: recovery.run_id,
      recovery_operation_id: recovery.operation_id,
      ...(typeof flattened._ui_only === "boolean"
        ? { _ui_only: flattened._ui_only }
        : {}),
    },
  };
}

function messageDiagnostics(error: ErrorMessage): string[] {
  const recovery = recoveryRecordFromError(error);
  if (recovery) {
    const info = object(error.error_info) ?? object(error.extra?.error_info);
    const labels = recoveryLabels(recovery);
    const redundant = new Set<unknown>([
      ...labels,
      info?.title,
      info?.explanation,
    ]);
    return [
      ...recoveryDiagnostics(recovery),
      ...(nonempty(info?.raw_error) && !labels.has(info.raw_error)
        ? [info.raw_error]
        : []),
      ...(nonempty(error.content) && !redundant.has(error.content)
        ? [error.content]
        : []),
    ];
  }

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

function recoveryDiagnostics(recovery: RecoveryRecord): string[] {
  return [
    ...recovery.attempts.flatMap((attempt) => {
      if (!attempt.error) return [];
      const error = attempt.error;
      return [
        nonempty(error.raw_error)
          ? error.raw_error
          : nonempty(error.explanation)
            ? error.explanation
            : error.title,
      ];
    }),
    nonempty(recovery.latest_error.raw_error)
      ? recovery.latest_error.raw_error
      : nonempty(recovery.latest_error.explanation)
        ? recovery.latest_error.explanation
        : recovery.latest_error.title,
  ];
}

export function collectErrorTexts(
  errors: ErrorMessage[],
  recoveries: RecoveryRecord[] = [],
): string[] {
  return Array.from(
    new Set(
      [
        ...errors.flatMap(messageDiagnostics),
        ...recoveries.flatMap(recoveryDiagnostics),
      ].filter(nonempty),
    ),
  );
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
  const episode = messages.filter((message, index): message is ErrorMessage => {
    if (message.role !== "error") return false;
    const archived = recoveryRecordFromError(message);
    return (
      archived?.state !== "succeeded" &&
      ((live && index > boundary) ||
        archived?.operation_id === recovery.operation_id ||
        archived?.run_id === recovery.run_id)
    );
  });
  return { episode, boundary };
}

export function visibleTranscriptErrors(
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
      collectErrorTexts(visibleTranscriptErrors(item.errors, episode)).length >
        0
        ? Math.max(last, item.messageIndex)
        : last,
    -1,
  );
}
