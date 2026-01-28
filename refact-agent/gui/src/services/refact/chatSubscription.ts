import type { ChatMessage } from "./types";

export type SessionState =
  | "idle"
  | "generating"
  | "executing_tools"
  | "paused"
  | "waiting_ide"
  | "error";

export type ThreadParams = {
  id: string;
  title: string;
  model: string;
  mode: string;
  tool_use: string;
  boost_reasoning: boolean;
  context_tokens_cap: number | null;
  include_project_info: boolean;
  checkpoints_enabled: boolean;
  is_title_generated: boolean;
  use_compression?: boolean;
  automatic_patch?: boolean;
  task_meta?: {
    task_id: string;
    role: string;
    agent_id?: string;
    card_id?: string;
  };
};

export type PauseReason = {
  type: string;
  command: string;
  rule: string;
  tool_call_id: string;
  integr_config_path: string | null;
};

export type QueuedItem = {
  client_request_id: string;
  priority: boolean;
  command_type: string;
  preview: string;
};

export type RuntimeState = {
  state: SessionState;
  paused: boolean;
  error: string | null;
  queue_size: number;
  pause_reasons: PauseReason[];
  queued_items: QueuedItem[];
};

export type DeltaOp =
  | { op: "append_content"; text: string }
  | { op: "append_reasoning"; text: string }
  | { op: "set_tool_calls"; tool_calls: unknown[] }
  | { op: "set_thinking_blocks"; blocks: unknown[] }
  | { op: "add_citation"; citation: unknown }
  | { op: "set_usage"; usage: unknown }
  | { op: "merge_extra"; extra: Record<string, unknown> };

export type EventEnvelope =
  | {
      chat_id: string;
      seq: string;
      type: "snapshot";
      thread: ThreadParams;
      runtime: RuntimeState;
      messages: ChatMessage[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "thread_updated";
      [key: string]: unknown;
    }
  | {
      chat_id: string;
      seq: string;
      type: "message_added";
      message: ChatMessage;
      index: number;
    }
  | {
      chat_id: string;
      seq: string;
      type: "message_updated";
      message_id: string;
      message: ChatMessage;
    }
  | {
      chat_id: string;
      seq: string;
      type: "message_removed";
      message_id: string;
    }
  | {
      chat_id: string;
      seq: string;
      type: "messages_truncated";
      from_index: number;
    }
  | {
      chat_id: string;
      seq: string;
      type: "stream_started";
      message_id: string;
    }
  | {
      chat_id: string;
      seq: string;
      type: "stream_delta";
      message_id: string;
      ops: DeltaOp[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "stream_finished";
      message_id: string;
      finish_reason: string | null;
    }
  | {
      chat_id: string;
      seq: string;
      type: "pause_required";
      reasons: PauseReason[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "pause_cleared";
    }
  | {
      chat_id: string;
      seq: string;
      type: "ide_tool_required";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
    }
  | {
      chat_id: string;
      seq: string;
      type: "subchat_update";
      tool_call_id: string;
      subchat_id: string;
      attached_files?: string[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "ack";
      client_request_id: string;
      accepted: boolean;
      result: unknown;
    }
  | {
      chat_id: string;
      seq: string;
      type: "queue_updated";
      queue_size: number;
      queued_items: QueuedItem[];
    };

export type ChatEventEnvelope = EventEnvelope;

export type ChatEventType = EventEnvelope["type"];

export type ChatSubscriptionCallbacks = {
  onEvent: (event: EventEnvelope) => void;
  onError: (error: Error) => void;
  onConnected?: () => void;
  onDisconnected?: () => void;
  onActivity?: () => void;
};

export type SubscriptionOptions = {
  connectTimeoutMs?: number;
  idleTimeoutMs?: number;
};

const DEFAULT_CONNECT_TIMEOUT_MS = 15_000;
const DEFAULT_IDLE_TIMEOUT_MS = 45_000;

export function subscribeToChatEvents(
  chatId: string,
  port: number,
  callbacks: ChatSubscriptionCallbacks,
  apiKey?: string,
  options: SubscriptionOptions = {},
): () => void {
  const url = `http://127.0.0.1:${port}/v1/chats/subscribe?chat_id=${encodeURIComponent(
    chatId,
  )}`;

  const connectTimeoutMs =
    options.connectTimeoutMs ?? DEFAULT_CONNECT_TIMEOUT_MS;
  const idleTimeoutMs = options.idleTimeoutMs ?? DEFAULT_IDLE_TIMEOUT_MS;

  const abortController = new AbortController();
  const state = { connected: false };
  let abortReason: string | null = null;
  let connectTimer: ReturnType<typeof setTimeout> | null = null;
  let idleTimer: ReturnType<typeof setTimeout> | null = null;

  const headers: Record<string, string> = {};
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const clearTimers = () => {
    if (connectTimer) {
      clearTimeout(connectTimer);
      connectTimer = null;
    }
    if (idleTimer) {
      clearTimeout(idleTimer);
      idleTimer = null;
    }
  };

  const armIdleTimer = () => {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      abortReason = abortReason ?? "SSE idle timeout";
      abortController.abort();
    }, idleTimeoutMs);
  };

  const disconnect = (notify: boolean) => {
    if (state.connected) {
      state.connected = false;
      if (notify) callbacks.onDisconnected?.();
    }
  };

  connectTimer = setTimeout(() => {
    if (!state.connected) {
      abortReason = abortReason ?? "SSE connect timeout";
      abortController.abort();
    }
  }, connectTimeoutMs);

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

      clearTimers();
      state.connected = true;
      callbacks.onConnected?.();
      armIdleTimer();

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;

        armIdleTimer();
        callbacks.onActivity?.();
        buffer += decoder.decode(value, { stream: true });
        buffer = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

        const blocks = buffer.split("\n\n");
        buffer = blocks.pop() ?? "";

        for (const block of blocks) {
          const trimmed = block.trim();
          if (!trimmed) continue;
          if (trimmed.startsWith(":")) continue;

          const dataLines: string[] = [];
          for (const rawLine of block.split("\n")) {
            if (!rawLine.startsWith("data:")) continue;
            dataLines.push(rawLine.slice(5).replace(/^\s*/, ""));
          }

          if (dataLines.length === 0) continue;

          const dataStr = dataLines.join("\n");
          if (dataStr === "[DONE]") continue;

          try {
            const parsed = JSON.parse(dataStr) as unknown;
            if (!isValidChatEventBasic(parsed)) {
              if (process.env.NODE_ENV === "development") {
                // eslint-disable-next-line no-console
                console.warn(
                  "[SSE] Invalid event structure:",
                  dataStr.slice(0, 200),
                );
              }
              continue;
            }
            normalizeSeq(parsed);
            if (parsed.chat_id !== chatId) {
              continue;
            }
            callbacks.onEvent(parsed);
          } catch (e) {
            if (process.env.NODE_ENV === "development") {
              // eslint-disable-next-line no-console
              console.warn("[SSE] Parse error:", e, dataStr.slice(0, 200));
            }
            continue;
          }
        }
      }

      clearTimers();
      if (abortController.signal.aborted) {
        if (abortReason) {
          callbacks.onError(new Error(abortReason));
        }
        abortReason = null;
        disconnect(false);
        return;
      }
      disconnect(true);
    })
    .catch((err: unknown) => {
      clearTimers();
      const error = err as Error;

      if (error.name === "AbortError") {
        if (abortReason) {
          callbacks.onError(new Error(abortReason));
        }
        abortReason = null;
        disconnect(false);
        return;
      }

      callbacks.onError(error);
      disconnect(false);
    });

  return () => {
    abortReason = null;
    clearTimers();
    abortController.abort();
    disconnect(false);
  };
}

function isValidChatEventBasic(data: unknown): data is EventEnvelope {
  if (typeof data !== "object" || data === null) return false;
  const obj = data as Record<string, unknown>;
  if (typeof obj.chat_id !== "string") return false;
  if (typeof obj.seq !== "string" && typeof obj.seq !== "number") return false;
  if (typeof obj.type !== "string") return false;
  return true;
}

function normalizeSeq(obj: EventEnvelope): void {
  const s = obj.seq as string | number;
  if (typeof s === "string") {
    const trimmed = s.trim();
    if (!/^\d+$/.test(trimmed)) {
      throw new Error("Invalid seq string");
    }
    (obj as { seq: string }).seq = trimmed;
    return;
  }
  if (typeof s === "number") {
    if (!Number.isFinite(s) || !Number.isInteger(s) || s < 0) {
      throw new Error("Invalid seq number");
    }
    (obj as { seq: string }).seq = String(s);
    return;
  }
  throw new Error("Missing/invalid seq");
}

export function applyDeltaOps(
  message: ChatMessage,
  ops: DeltaOp[],
): ChatMessage {
  const updated = { ...message } as ChatMessage & {
    content?: string;
    reasoning_content?: string;
    tool_calls?: unknown[];
    thinking_blocks?: unknown[];
    citations?: unknown[];
    usage?: unknown;
    extra?: Record<string, unknown>;
  };

  for (const op of ops) {
    switch (op.op) {
      case "append_content":
        if (typeof updated.content === "string") {
          updated.content = updated.content + op.text;
        } else {
          updated.content = op.text;
        }
        break;

      case "append_reasoning":
        updated.reasoning_content = (updated.reasoning_content ?? "") + op.text;
        break;

      case "set_tool_calls":
        updated.tool_calls = op.tool_calls;
        break;

      case "set_thinking_blocks":
        updated.thinking_blocks = op.blocks;
        break;

      case "add_citation":
        updated.citations = [...(updated.citations ?? []), op.citation];
        break;

      case "set_usage":
        updated.usage = op.usage;
        break;

      case "merge_extra":
        updated.extra = { ...(updated.extra ?? {}), ...op.extra };
        break;
    }
  }

  return updated;
}
