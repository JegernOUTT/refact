import { debugRefact } from "../../debugConfig";
import type { TrajectoryMeta, TrajectoryEvent } from "./trajectories";
import type { TaskMeta, TaskBoard } from "./tasks";
import type { BuddySnapshot, BuddySSEEvent } from "../../features/Buddy/types";
import { buildApiUrl, type EngineApiConfig } from "./apiUrl";

export type { TrajectoryMeta, TrajectoryEvent };

export type TaskEvent =
  | { type: "snapshot"; tasks: TaskMeta[] }
  | { type: "task_created"; task_id: string; meta: TaskMeta }
  | { type: "task_updated"; task_id: string; meta: TaskMeta }
  | { type: "task_deleted"; task_id: string }
  | { type: "board_changed"; task_id: string; rev: number; board: TaskBoard }
  | { type: "task_comments_changed"; task_id: string; card_id: string }
  | { type: "task_document_changed"; task_id: string; slug?: string }
  | { type: "task_memories_changed"; task_id: string };

export type NotificationEvent =
  | {
      type: "task_done";
      chat_id: string;
      tool_call_id: string;
      summary: string;
      knowledge_path?: string;
    }
  | {
      type: "ask_questions";
      chat_id: string;
      tool_call_id: string;
      questions: {
        id: string;
        type: string;
        text: string;
        options?: string[];
      }[];
    };

export type SidebarSection = "workspace" | "chats" | "tasks" | "buddy";
export type SidebarSectionStatus = "ready" | "error";
export type BuddySnapshotPayload = BuddySnapshot | null;

export type SidebarPagination = {
  next_cursor: string | null;
  has_more: boolean;
  total_count: number;
};

export type SidebarSectionSnapshot =
  | { workspace_roots: string[] }
  | { trajectories: TrajectoryMeta[]; pagination?: SidebarPagination }
  | { tasks: TaskMeta[] }
  | { buddy: BuddySnapshotPayload };

export type SidebarSectionUpdate = TrajectoryEvent | TaskEvent | BuddySSEEvent;

export type SidebarKnownEvent =
  | {
      type: "section_snapshot";
      section: SidebarSection;
      status: SidebarSectionStatus;
      snapshot: SidebarSectionSnapshot;
      elapsed_ms?: number;
      error?: string;
    }
  | {
      type: "section_update";
      section: SidebarSection;
      update: SidebarSectionUpdate;
    }
  | {
      type: "notification";
      notification: NotificationEvent;
    }
  | {
      type: "heartbeat";
      payload: { ts: string };
    };

export type SidebarEvent =
  | SidebarKnownEvent
  | {
      type: string;
      payload: unknown;
    };

export type SidebarEventEnvelope = {
  protocol_version: 2;
  seq: number;
  subscription_id: string;
  event: SidebarEvent;
};

export type SidebarDispatchedEvent = Exclude<
  SidebarKnownEvent,
  { type: "heartbeat" }
>;

export type SidebarDispatchedEventEnvelope = Omit<
  SidebarEventEnvelope,
  "event"
> & {
  event: SidebarDispatchedEvent;
};

export type SidebarSubscriptionCallbacks = {
  onEvent: (event: SidebarDispatchedEventEnvelope) => void;
  onError: (error: Error) => void;
  onConnected?: () => void;
  onDisconnected?: () => void;
  onLiveness?: () => void;
};

const IDLE_TIMEOUT_MS = 30_000;
const CONNECT_TIMEOUT_MS = 10_000;
const MAX_SSE_BLOCK_BYTES = 4 * 1024 * 1024;

function hasArrayProperty(obj: Record<string, unknown>, key: string): boolean {
  return Array.isArray(obj[key]);
}

function isValidTrajectoryEvent(obj: Record<string, unknown>): boolean {
  return typeof obj.type === "string" && typeof obj.id === "string";
}

function isValidTaskEvent(obj: Record<string, unknown>): boolean {
  if (typeof obj.type !== "string") return false;
  switch (obj.type) {
    case "snapshot":
      return Array.isArray(obj.tasks);
    case "task_created":
    case "task_updated":
      return typeof obj.task_id === "string" && obj.meta !== undefined;
    case "task_deleted":
      return typeof obj.task_id === "string";
    case "board_changed":
      return (
        typeof obj.task_id === "string" &&
        typeof obj.rev === "number" &&
        obj.board !== undefined
      );
    case "task_comments_changed":
      return typeof obj.task_id === "string" && typeof obj.card_id === "string";
    case "task_document_changed":
      return (
        typeof obj.task_id === "string" &&
        (obj.slug === undefined || typeof obj.slug === "string")
      );
    case "task_memories_changed":
      return typeof obj.task_id === "string";
    default:
      return false;
  }
}

function isValidNotificationEvent(obj: Record<string, unknown>): boolean {
  if (typeof obj.type !== "string") return false;
  if (typeof obj.chat_id !== "string") return false;
  if (typeof obj.tool_call_id !== "string") return false;

  if (obj.type === "task_done") {
    return typeof obj.summary === "string";
  }

  if (obj.type === "ask_questions") {
    return Array.isArray(obj.questions);
  }

  return false;
}

function isValidSection(value: unknown): value is SidebarSection {
  return (
    value === "workspace" ||
    value === "chats" ||
    value === "tasks" ||
    value === "buddy"
  );
}

function isValidSectionStatus(value: unknown): value is SidebarSectionStatus {
  return value === "ready" || value === "error";
}

function isValidSidebarPagination(value: unknown): value is SidebarPagination {
  if (typeof value !== "object" || value === null) return false;
  const obj = value as Record<string, unknown>;
  return (
    (typeof obj.next_cursor === "string" || obj.next_cursor === null) &&
    typeof obj.has_more === "boolean" &&
    typeof obj.total_count === "number"
  );
}

function isValidSectionSnapshot(
  section: SidebarSection,
  snapshot: unknown,
): snapshot is SidebarSectionSnapshot {
  if (typeof snapshot !== "object" || snapshot === null) return false;
  const obj = snapshot as Record<string, unknown>;

  if (section === "workspace") return hasArrayProperty(obj, "workspace_roots");
  if (section === "chats") {
    return (
      hasArrayProperty(obj, "trajectories") &&
      (obj.pagination === undefined || isValidSidebarPagination(obj.pagination))
    );
  }
  if (section === "tasks") return hasArrayProperty(obj, "tasks");
  return "buddy" in obj;
}

function isValidSectionUpdate(
  section: SidebarSection,
  update: unknown,
): update is SidebarSectionUpdate {
  if (typeof update !== "object" || update === null) return false;
  const obj = update as Record<string, unknown>;

  if (section === "chats") return isValidTrajectoryEvent(obj);
  if (section === "tasks") return isValidTaskEvent(obj);
  if (section === "buddy") return typeof obj.event_type === "string";
  return false;
}

function isDispatchedSidebarEvent(
  event: SidebarEvent,
): event is SidebarDispatchedEvent {
  if (event.type === "section_snapshot") return true;
  if (event.type === "section_update") return true;
  return event.type === "notification";
}

function toSeq(value: unknown): number | null {
  if (typeof value !== "number") return null;
  if (!Number.isInteger(value) || value < 0) return null;
  return value;
}

function getEnvelopeParts(parsed: unknown): {
  seq: number;
  subscriptionId: string;
  event: Record<string, unknown>;
} | null {
  if (typeof parsed !== "object" || parsed === null) return null;
  const envelope = parsed as Record<string, unknown>;
  if (envelope.protocol_version !== 2) return null;
  const seq = toSeq(envelope.seq);
  if (seq === null) return null;
  if (typeof envelope.subscription_id !== "string") return null;
  if (typeof envelope.event !== "object" || envelope.event === null)
    return null;
  const event = envelope.event as Record<string, unknown>;
  if (typeof event.type !== "string") return null;
  return { seq, subscriptionId: envelope.subscription_id, event };
}

function tryParseSidebarEvent(raw: string): SidebarEventEnvelope | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  const envelope = getEnvelopeParts(parsed);
  if (!envelope) return null;
  const { seq, subscriptionId, event } = envelope;

  switch (event.type) {
    case "section_snapshot": {
      const section = event.section;
      const status = event.status;
      const snapshot = event.snapshot;
      if (!isValidSection(section)) return null;
      if (!isValidSectionStatus(status)) return null;
      if (!isValidSectionSnapshot(section, snapshot)) return null;
      if (
        event.elapsed_ms !== undefined &&
        typeof event.elapsed_ms !== "number"
      ) {
        return null;
      }
      if (event.error !== undefined && typeof event.error !== "string") {
        return null;
      }
      return {
        protocol_version: 2,
        seq,
        subscription_id: subscriptionId,
        event: {
          type: "section_snapshot",
          section,
          status,
          snapshot,
          ...(event.elapsed_ms !== undefined
            ? { elapsed_ms: event.elapsed_ms }
            : {}),
          ...(event.error !== undefined ? { error: event.error } : {}),
        },
      };
    }

    case "section_update": {
      const section = event.section;
      const update = event.update;
      if (!isValidSection(section)) return null;
      if (!isValidSectionUpdate(section, update)) return null;
      return {
        protocol_version: 2,
        seq,
        subscription_id: subscriptionId,
        event: {
          type: "section_update",
          section,
          update,
        },
      };
    }

    case "notification":
      if (
        typeof event.notification !== "object" ||
        event.notification === null
      ) {
        return null;
      }
      if (
        !isValidNotificationEvent(event.notification as Record<string, unknown>)
      ) {
        return null;
      }
      return {
        protocol_version: 2,
        seq,
        subscription_id: subscriptionId,
        event: {
          type: "notification",
          notification: event.notification as NotificationEvent,
        },
      };

    case "heartbeat":
      return {
        protocol_version: 2,
        seq,
        subscription_id: subscriptionId,
        event: {
          type: "heartbeat",
          payload: {
            ts:
              typeof event.payload === "object" && event.payload !== null
                ? String((event.payload as Record<string, unknown>).ts ?? "")
                : "",
          },
        },
      };

    default:
      return {
        protocol_version: 2,
        seq,
        subscription_id: subscriptionId,
        event: { type: event.type as string, payload: event.payload },
      };
  }
}

function tryReadSidebarEnvelopeSeq(raw: string): number | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  return getEnvelopeParts(parsed)?.seq ?? null;
}

function warnSkippedSidebarEvent(raw: string): void {
  debugRefact("[sidebar] skipped malformed event:", raw.slice(0, 200));
}

function debugIgnoredSidebarEvent(type: string): void {
  debugRefact("[sidebar] ignored event type:", type);
}

function errorSidebarBlockTooLarge(): void {
  debugRefact(
    `[sidebar] SSE block exceeded ${MAX_SSE_BLOCK_BYTES} bytes; reconnecting`,
  );
}

function parseSseBlock(block: string): string | null {
  if (!block.trim()) return null;
  if (block.startsWith(":")) return null;

  const dataLines: string[] = [];
  for (const rawLine of block.split("\n")) {
    if (rawLine.startsWith(":")) continue;
    if (!rawLine.startsWith("data:")) continue;
    dataLines.push(rawLine.slice(5).replace(/^\s*/, ""));
  }

  if (dataLines.length === 0) return null;

  const dataStr = dataLines.join("\n");
  return dataStr === "[DONE]" ? null : dataStr;
}

export function subscribeToSidebarEvents(
  config: EngineApiConfig,
  apiKey: string | null,
  callbacks: SidebarSubscriptionCallbacks,
): () => void {
  const url = buildApiUrl(config, "/v1/sidebar/subscribe");
  const abortController = new AbortController();
  const state = { connected: false, lastSeq: -1, aborted: false };
  let reportedStreamError = false;
  let connectTimedOut = false;
  let connectTimer: ReturnType<typeof setTimeout> | null = setTimeout(() => {
    if (!state.connected && !state.aborted) {
      connectTimedOut = true;
      abortController.abort();
    }
  }, CONNECT_TIMEOUT_MS);
  let idleTimer: ReturnType<typeof setTimeout> | null = null;

  const headers: Record<string, string> = {};
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const resetIdleTimer = () => {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      abortController.abort();
    }, IDLE_TIMEOUT_MS);
  };

  const advanceSeq = (seq: number) => {
    if (state.lastSeq >= 0 && seq !== state.lastSeq + 1) {
      throw new Error(`Seq gap: expected ${state.lastSeq + 1}, got ${seq}`);
    }
    state.lastSeq = seq;
  };

  const reportStreamError = (error: Error) => {
    reportedStreamError = true;
    callbacks.onError(error);
  };

  const cleanup = () => {
    if (connectTimer) {
      clearTimeout(connectTimer);
      connectTimer = null;
    }
    if (idleTimer) {
      clearTimeout(idleTimer);
      idleTimer = null;
    }
    if (!state.aborted) {
      state.aborted = true;
      abortController.abort();
    }
    if (state.connected) {
      state.connected = false;
      callbacks.onDisconnected?.();
    }
  };

  void fetch(url, {
    method: "GET",
    headers,
    signal: abortController.signal,
  })
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(`SSE connection failed: ${response.status}`);
      }
      if (!response.body) {
        throw new Error("Response body is null");
      }

      state.connected = true;
      if (connectTimer) {
        clearTimeout(connectTimer);
        connectTimer = null;
      }
      callbacks.onConnected?.();
      resetIdleTimer();
      callbacks.onLiveness?.();

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;

          resetIdleTimer();
          callbacks.onLiveness?.();
          buffer += decoder
            .decode(value, { stream: true })
            .replace(/\r\n/g, "\n")
            .replace(/\r/g, "\n");

          let idx = buffer.indexOf("\n\n");
          while (idx !== -1) {
            const block = buffer.slice(0, idx);
            if (block.length > MAX_SSE_BLOCK_BYTES) {
              errorSidebarBlockTooLarge();
              const blockTooLarge = new Error("sse_block_too_large");
              reportStreamError(blockTooLarge);
              throw blockTooLarge;
            }
            buffer = buffer.slice(idx + 2);
            const dataStr = parseSseBlock(block);
            if (dataStr !== null) {
              const parsed = tryParseSidebarEvent(dataStr);
              if (!parsed) {
                const skippedSeq = tryReadSidebarEnvelopeSeq(dataStr);
                warnSkippedSidebarEvent(dataStr);
                if (skippedSeq !== null) {
                  const malformedEvent = new Error("malformed_sidebar_event");
                  reportStreamError(malformedEvent);
                  throw malformedEvent;
                }
              } else {
                advanceSeq(parsed.seq);
                if (isDispatchedSidebarEvent(parsed.event)) {
                  callbacks.onEvent({
                    protocol_version: parsed.protocol_version,
                    seq: parsed.seq,
                    subscription_id: parsed.subscription_id,
                    event: parsed.event,
                  });
                } else {
                  debugIgnoredSidebarEvent(parsed.event.type);
                }
              }
            }
            idx = buffer.indexOf("\n\n");
          }

          if (buffer.length > MAX_SSE_BLOCK_BYTES) {
            errorSidebarBlockTooLarge();
            const blockTooLarge = new Error("sse_block_too_large");
            reportStreamError(blockTooLarge);
            throw blockTooLarge;
          }
        }
      } finally {
        await reader.cancel().catch(() => undefined);
      }

      cleanup();
    })
    .catch((err: unknown) => {
      const error = err as Error;
      if (error.name === "AbortError" && connectTimedOut) {
        callbacks.onError(new Error("sse_connect_timeout"));
      } else if (error.name !== "AbortError" && !reportedStreamError) {
        callbacks.onError(error);
      }
      cleanup();
    });

  return cleanup;
}
