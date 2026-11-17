import { describe, expect, it } from "vitest";
import type { RecoveryRecord } from "./chatSubscription";
import type { ChatMessages, ErrorMessage } from "./types";
import {
  collectErrorEntries,
  collectErrorTexts,
  errorRecoveryStatuses,
  recoveryStatusHosts,
  recoveryStatusLabel,
  normalizeRecoveryErrorMetadata,
  partitionTranscriptErrors,
  recoveryRecordFromError,
  retryableErrorIndex,
} from "./recoveryPresentation";

const timeout = "LLM HTTP SSE response header timeout after 30s";
const transport =
  "LLM request failed: error sending request for url (https://example.test/responses)";
const recovery: RecoveryRecord = {
  operation_id: "operation",
  run_id: "run",
  revision: 5,
  state: "exhausted",
  policy: {
    mode: "transient",
    max_retries: 2,
    delay_ms: 10000,
    total_timeout_ms: 600000,
  },
  policy_source: "global",
  manual: false,
  retries_used: 2,
  latest_error: {
    category: "ProviderTransient",
    title: "Provider temporarily unavailable",
    explanation: "The provider returned a temporary error.",
    suggested_action: "retry",
    is_retryable: true,
    raw_error: timeout,
  },
  attempts: [
    {
      at_ms: 1,
      retry_number: 0,
      outcome: "failed",
      error: {
        category: "NetworkFailure",
        title: "Network failure",
        explanation: "Connection failed",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: transport,
      },
    },
    {
      at_ms: 2,
      retry_number: 1,
      outcome: "failed",
      error: {
        category: "ProviderTransient",
        title: "Timeout",
        explanation: "Timed out",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: timeout,
      },
    },
  ],
};

describe("recovery diagnostics", () => {
  it("preserves flattened archive identity and uses actual attempt causes instead of the category title", () => {
    const flattened = {
      role: "error" as const,
      content: "Provider temporarily unavailable",
      recovery_record: recovery,
      recovery_run_id: recovery.run_id,
      _ui_only: true,
    };
    const normalized = normalizeRecoveryErrorMetadata(flattened);
    expect(normalized.extra).toMatchObject({
      recovery_record: recovery,
      recovery_run_id: "run",
      recovery_operation_id: "operation",
      _ui_only: true,
    });
    expect(recoveryRecordFromError(normalized)).toEqual(recovery);
    expect(collectErrorTexts([flattened, normalized], [recovery])).toEqual([
      transport,
      timeout,
    ]);
    expect(
      collectErrorTexts([
        {
          ...flattened,
          error_info: {
            ...recovery.latest_error,
            category: "ProviderTransient",
            raw_error: recovery.latest_error.title,
          },
        },
        {
          ...flattened,
          content: "Timeout",
          error_info: {
            ...recovery.latest_error,
            category: "ProviderTransient",
            raw_error: "Timeout",
          },
        },
      ]),
    ).toEqual([transport, timeout]);
    expect(
      collectErrorTexts([
        { ...flattened, content: "Separate actionable cause" },
      ]),
    ).toEqual(["Separate actionable cause"]);
    expect(
      collectErrorTexts([
        {
          ...flattened,
          error_info: {
            ...recovery.latest_error,
            category: "ProviderTransient",
            title: "quota exceeded",
            raw_error: "quota exceeded",
          },
        },
      ]),
    ).toEqual(["quota exceeded"]);
  });

  it("shows only message-owned diagnostics for stamped messages and expands history only for legacy title-only archives", () => {
    const stamped: RecoveryRecord = {
      ...recovery,
      attempts: recovery.attempts.map((attempt) => ({
        ...attempt,
        error: null,
      })),
    };
    const first: ErrorMessage = {
      role: "error",
      content: transport,
      extra: { recovery_operation_id: "operation" },
    };
    const latest: ErrorMessage = {
      role: "error",
      content: timeout,
      extra: {
        recovery_operation_id: "operation",
        error_info: { ...recovery.latest_error },
        recovery_record: stamped,
      },
    };
    expect(collectErrorTexts([latest])).toEqual([timeout]);
    expect(
      collectErrorTexts([
        { ...latest, extra: { ...latest.extra, recovery_record: recovery } },
      ]),
    ).toEqual([timeout]);
    expect(collectErrorTexts([first, latest])).toEqual([transport, timeout]);
    const legacy: ErrorMessage = {
      role: "error",
      content: recovery.latest_error.title,
      extra: { recovery_record: recovery },
    };
    expect(collectErrorEntries([legacy])).toEqual([
      { text: transport, repeat: 1 },
      { text: timeout, repeat: 1 },
    ]);
    expect(
      collectErrorTexts([{ ...legacy, extra: { recovery_record: stamped } }]),
    ).toEqual([timeout]);
  });

  it("shows identical content/raw text, different errors and the full diagnostic beyond 300 characters", () => {
    const longError = `HTTP 400\n${"detail ".repeat(90)}\n...[truncated]`;
    const error: ErrorMessage = {
      role: "error",
      content: longError,
      error_info: {
        category: "InvalidRequest",
        title: "Invalid request",
        explanation: "Rejected",
        suggested_action: "none",
        is_retryable: false,
        raw_error: longError,
      },
    };
    expect(
      collectErrorTexts([error, error, { role: "error", content: transport }]),
    ).toEqual([longError, transport]);
    expect(collectErrorTexts([{ ...error, content: transport }])).toEqual([
      longError,
      transport,
    ]);
  });

  it("extracts structured JSON diagnostics and falls back from blank raw errors without losing plain text", () => {
    const messages: ErrorMessage[] = [
      {
        role: "error",
        content: JSON.stringify({
          error: { error_info: { raw_error: timeout } },
        }),
      },
      {
        role: "error",
        content: JSON.stringify({
          message: transport,
          error_info: { raw_error: "" },
        }),
      },
      {
        role: "error",
        content: "Plain error",
        error_info: {
          category: "Unknown",
          title: "Unknown error",
          explanation: "Unclassified",
          suggested_action: "none",
          is_retryable: false,
          raw_error: " ",
        },
      },
    ];
    expect(collectErrorTexts(messages)).toEqual([
      timeout,
      transport,
      "Plain error",
    ]);
  });

  it("separates the live recovery episode from history and offers Retry only on the last visible failure after the turn", () => {
    const old: ErrorMessage = {
      role: "error",
      content: recovery.latest_error.title,
      extra: { recovery_record: { ...recovery, run_id: "old" } },
    };
    const recovered: ErrorMessage = {
      role: "error",
      content: recovery.latest_error.title,
      extra: {
        recovery_record: { ...recovery, run_id: "ok", state: "succeeded" },
      },
    };
    const tail: ErrorMessage = { role: "error", content: transport };
    const later: ErrorMessage = { role: "error", content: timeout };
    const synthetic: ErrorMessage = {
      role: "error",
      content: "Context compression failed: summarizer unavailable",
    };
    const messages: ChatMessages = [
      { role: "user", content: "Start" },
      old,
      { role: "assistant", content: "Continued" },
      recovered,
      { role: "assistant", content: "Cut off", extra: { interrupted: true } },
      tail,
    ];
    expect(partitionTranscriptErrors(messages, null)).toEqual({
      episode: [],
      boundary: 2,
    });
    expect(
      partitionTranscriptErrors(messages, { ...recovery, run_id: "live" }),
    ).toEqual({ episode: [old, tail], boundary: 2 });
    expect(
      partitionTranscriptErrors(messages, {
        ...recovery,
        operation_id: "other",
        run_id: "live",
      }),
    ).toEqual({ episode: [tail], boundary: 2 });
    expect(
      partitionTranscriptErrors(messages, {
        ...recovery,
        run_id: "live",
        state: "succeeded",
      }),
    ).toEqual({ episode: [old], boundary: 2 });

    const items = [
      { messageIndex: 1, errors: [old] },
      { messageIndex: 3, errors: [recovered] },
      { messageIndex: 5, errors: [tail] },
    ];
    expect(retryableErrorIndex(items, 2, [])).toBe(5);
    expect(
      retryableErrorIndex(
        [...items, { messageIndex: 7, errors: [later] }],
        2,
        [],
      ),
    ).toBe(7);
    expect(
      retryableErrorIndex(
        [...items, { messageIndex: 7, errors: [recovered] }],
        2,
        [],
      ),
    ).toBe(5);
    expect(
      retryableErrorIndex(
        [...items, { messageIndex: 9, errors: [synthetic] }],
        2,
        [],
      ),
    ).toBe(9);
    expect(retryableErrorIndex(items, 2, [tail])).toBe(-1);
    expect(retryableErrorIndex(items, 2, [old])).toBe(5);
    expect(retryableErrorIndex(items, 5, [])).toBe(-1);
  });

  it("labels stamped records with compact recovered and terminal statuses", () => {
    const succeeded = { ...recovery, state: "succeeded" as const };
    expect(recoveryStatusLabel({ ...succeeded, retries_used: 1 })).toBe(
      "Recovered after 1 retry",
    );
    expect(recoveryStatusLabel({ ...succeeded, retries_used: 3 })).toBe(
      "Recovered after 3 retries",
    );
    expect(
      recoveryStatusLabel({ ...succeeded, manual: true, retries_used: 1 }),
    ).toBe("Recovered after 1 new request");
    expect(
      recoveryStatusLabel({ ...succeeded, manual: true, retries_used: 2 }),
    ).toBe("Recovered after 2 new requests");
    expect(recoveryStatusLabel({ ...succeeded, retries_used: 0 })).toBe(
      "Recovered",
    );
    expect(
      recoveryStatusLabel({
        ...recovery,
        terminal_reason: "retry_limit_exhausted",
      }),
    ).toBe("Retry limit reached");
    expect(recoveryStatusLabel({ ...recovery, state: "cancelled" })).toBe(
      "Cancelled",
    );
    expect(recoveryStatusLabel({ ...recovery, state: "waiting" })).toBe(
      "Interrupted",
    );
    expect(
      recoveryStatusLabel(
        { ...recovery, state: "attempting" },
        recovery.run_id,
      ),
    ).toBeNull();
  });

  it("reports repeat counts per distinct text and statuses only on the latest message of an operation", () => {
    const first: ErrorMessage = {
      role: "error",
      content: transport,
      extra: { recovery_operation_id: "operation", repeat_count: 3 },
    };
    const second: ErrorMessage = {
      role: "error",
      content: timeout,
      repeat_count: 2,
      extra: {
        recovery_operation_id: "operation",
        recovery_record: { ...recovery, state: "succeeded" },
      },
    };
    expect(
      collectErrorEntries([first, second, { ...first, extra: {} }]),
    ).toEqual([
      { text: transport, repeat: 4 },
      { text: timeout, repeat: 2 },
    ]);
    const older: ErrorMessage = {
      ...second,
      extra: { ...second.extra, recovery_record: recovery },
    };
    const hosts = recoveryStatusHosts([older, first, second]);
    expect(hosts.has(second)).toBe(true);
    expect(hosts.has(older)).toBe(false);
    expect(errorRecoveryStatuses([older, second], hosts)).toEqual([
      "Recovered after 2 retries",
    ]);
    expect(errorRecoveryStatuses([older], undefined)).toEqual(["Exhausted"]);
  });

  it("links episode errors by recovery operation ids even before a record is stamped", () => {
    const linked: ErrorMessage = {
      role: "error",
      content: transport,
      extra: { recovery_operation_id: "operation", recovery_run_id: "run" },
    };
    const other: ErrorMessage = {
      role: "error",
      content: timeout,
      extra: { recovery_operation_id: "other", recovery_run_id: "other" },
    };
    const messages: ChatMessages = [
      linked,
      { role: "user", content: "Next" },
      other,
    ];
    expect(
      partitionTranscriptErrors(messages, { ...recovery, state: "waiting" }),
    ).toEqual({ episode: [linked], boundary: 1 });
    expect(
      normalizeRecoveryErrorMetadata({
        role: "error",
        content: transport,
        recovery_operation_id: "operation",
      } as ErrorMessage).extra,
    ).toEqual({ recovery_operation_id: "operation" });
  });
});
