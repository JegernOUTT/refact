import type { BackgroundAgentSummary, ChatMessage } from "./types";
import type { WorktreeMeta } from "./worktrees";

export type SessionState =
  | "idle"
  | "generating"
  | "executing_tools"
  | "paused"
  | "waiting_ide"
  | "waiting_user_input"
  | "completed"
  | "error";

export type CompressionPhase =
  | "checking"
  | "running"
  | "applied"
  | "skipped"
  | "failed";

export type CompressionReason =
  | "auto_compact_disabled"
  | "session_compaction_disabled"
  | "max_attempts_reached"
  | "pending_tool_calls"
  | "no_eligible_segment"
  | "effective_context_unknown"
  | "pressure_low"
  | "no_summary_model"
  | "input_too_large"
  | "transient_failure"
  | "source_changed";

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
  auto_approve_editing_tools?: boolean;
  auto_approve_dangerous_commands?: boolean;
  reasoning_effort?: string | null;
  thinking_budget?: number | null;
  temperature?: number | null;
  frequency_penalty?: number | null;
  max_tokens?: number | null;
  parallel_tool_calls?: boolean | null;
  task_meta?: {
    task_id: string;
    role: string;
    agent_id?: string;
    card_id?: string;
  };

  previous_response_id?: string;
  auto_enrichment_enabled?: boolean | null;
  auto_compact_enabled?: boolean | null;
  reactive_compact_attempts?: number | null;
  worktree?: WorktreeMeta | null;
  parent_id?: string | null;
  link_type?: string | null;
  root_chat_id?: string | null;
};

export type PauseReason = {
  type: string;
  tool_name: string;
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
  is_compressing?: boolean;
  compression_phase?: CompressionPhase | null;
  compression_reason?: CompressionReason | null;
};

type BackgroundAgentSummaryWithDefaults = Omit<
  BackgroundAgentSummary,
  "target_files" | "edited_files" | "step_count" | "change_seq"
> & {
  target_files?: string[] | null;
  edited_files?: string[] | null;
  step_count?: number | null;
  change_seq?: number | null;
};

type BackgroundAgentSummaryWire =
  | BackgroundAgentSummaryWithDefaults
  | BackgroundAgentSummaryCamelCase;

type BackgroundAgentSummaryCamelCase = {
  agentId: string;
  parentChatId: string;
  childChatId: string | null;
  kind: BackgroundAgentSummary["kind"];
  status: BackgroundAgentSummary["status"];
  title: string;
  progress: string | null;
  stepCount?: number | null;
  lastActivity: string | null;
  targetFiles?: string[] | null;
  editedFiles?: string[] | null;
  diffSummary: string | null;
  conflictSummary: string | null;
  resultSummary: string | null;
  error: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  changeSeq?: number | null;
};

export type DeltaOp =
  | { op: "append_content"; text: string }
  | { op: "append_reasoning"; text: string }
  | { op: "set_tool_calls"; tool_calls: unknown[] }
  | { op: "set_thinking_blocks"; blocks: unknown[] }
  | { op: "add_citation"; citation: unknown }
  | { op: "add_server_content_block"; block: unknown }
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
      background_agents: BackgroundAgentSummary[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "background_agent_updated";
      agent: BackgroundAgentSummary;
    }
  | {
      chat_id: string;
      seq: string;
      type: "thread_updated";
      worktree?: WorktreeMeta | null;
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
      type: "process_completed";
      process_id: string;
      status: string;
      exit_code: number | null;
      short_description: string;
      mode: string;
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
    }
  | {
      chat_id: string;
      seq: string;
      type: "runtime_updated";
      state: string;
      error?: string;
      is_compressing?: boolean;
      compression_phase?: CompressionPhase | null;
      compression_reason?: CompressionReason | null;
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_context_oversize";
      total_bytes: number;
      action_count: number;
      action_bytes: number;
      console_count: number;
      console_bytes: number;
      network_count: number;
      network_bytes: number;
      mutation_bytes: number;
      pending_message_id: string;
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_frame";
      tab_id: string;
      mime: string;
      data: string;
      diff_boxes?: { x: number; y: number; width: number; height: number }[];
      changed_text?: string;
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_status";
      runtime_id: string;
      connected: boolean;
      active_tab?: string | null;
      url?: string | null;
      title?: string | null;
      tabs?: { tab_id: string; url: string; title: string }[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_closed";
      runtime_id: string;
      reason: string;
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_timeline";
      events: {
        timestamp: string;
        source: string;
        type: string;
        summary: string;
        details?: Record<string, unknown>;
      }[];
    }
  | {
      chat_id: string;
      seq: string;
      type: "browser_toolbar_action";
      action: string;
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
const MAX_SSE_BUFFER_CHARS = 8_000_000;
const MAX_SSE_EVENT_CHARS = 4_000_000;

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
  let terminalErrorEmitted = false;
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
      emitTerminalError(abortReason);
      abortController.abort();
      disconnect(false);
    }, idleTimeoutMs);
  };

  const disconnect = (notify: boolean) => {
    if (state.connected) {
      state.connected = false;
      if (notify) callbacks.onDisconnected?.();
    }
  };

  const emitTerminalError = (message: string) => {
    if (terminalErrorEmitted) return;
    terminalErrorEmitted = true;
    callbacks.onError(new Error(message));
  };

  connectTimer = setTimeout(() => {
    if (!state.connected) {
      abortReason = abortReason ?? "SSE connect timeout";
      emitTerminalError(abortReason);
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
        const chunk = decoder
          .decode(value, { stream: true })
          .replace(/\r\n/g, "\n")
          .replace(/\r/g, "\n");
        buffer += chunk;

        if (buffer.length > MAX_SSE_BUFFER_CHARS) {
          abortReason = `SSE buffer exceeded ${MAX_SSE_BUFFER_CHARS} chars`;
          abortController.abort();
          break;
        }

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
          if (dataStr.length > MAX_SSE_EVENT_CHARS) {
            if (process.env.NODE_ENV === "development") {
              // eslint-disable-next-line no-console
              console.warn(
                `[SSE] Event too large (${dataStr.length} chars), skipping`,
              );
            }
            continue;
          }

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
            normalizeBackgroundAgentFields(parsed);
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
          emitTerminalError(abortReason);
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
          emitTerminalError(abortReason);
        }
        abortReason = null;
        disconnect(true);
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

function normalizeBackgroundAgentFields(obj: EventEnvelope): void {
  if (obj.type === "background_agent_updated") {
    if (!isValidBackgroundAgent(obj.agent)) {
      throw new Error("Invalid background agent");
    }
    obj.agent = normalizeBackgroundAgentSummary(obj.agent);
    return;
  }
  if (obj.type === "snapshot") {
    const backgroundAgents = Array.isArray(obj.background_agents)
      ? obj.background_agents
      : [];
    obj.background_agents = backgroundAgents
      .filter(isValidBackgroundAgent)
      .map((agent) => normalizeBackgroundAgentSummary(agent));
  }
}

function isValidBackgroundAgent(
  agent: unknown,
): agent is BackgroundAgentSummaryWire {
  if (agent === null || typeof agent !== "object") return false;
  const fields = agent as Record<string, unknown>;
  if (typeof fields.kind !== "string") return false;
  if (typeof fields.status !== "string") return false;
  if ("agent_id" in fields) return typeof fields.agent_id === "string";
  return typeof fields.agentId === "string";
}

function safeAgent(agent: BackgroundAgentSummaryWire): BackgroundAgentSummary {
  if (!("agent_id" in agent)) {
    return safeAgent({
      agent_id: agent.agentId,
      parent_chat_id: agent.parentChatId,
      child_chat_id: agent.childChatId,
      kind: agent.kind,
      status: agent.status,
      title: agent.title,
      progress: agent.progress,
      step_count: agent.stepCount,
      last_activity: agent.lastActivity,
      target_files: agent.targetFiles,
      edited_files: agent.editedFiles,
      diff_summary: agent.diffSummary,
      conflict_summary: agent.conflictSummary,
      result_summary: agent.resultSummary,
      error: agent.error,
      started_at: agent.startedAt,
      finished_at: agent.finishedAt,
      change_seq: agent.changeSeq,
    });
  }

  return {
    ...agent,
    target_files: agent.target_files ?? [],
    edited_files: agent.edited_files ?? [],
    step_count: agent.step_count ?? 0,
    change_seq: agent.change_seq ?? -1,
  };
}

function normalizeBackgroundAgentSummary(
  agent: BackgroundAgentSummaryWire,
): BackgroundAgentSummary {
  return safeAgent(agent);
}

export function applyDeltaOps(
  message: ChatMessage,
  ops: DeltaOp[],
): ChatMessage {
  if (ops.length === 0) return message;

  const updated = { ...message } as ChatMessage & {
    content?: string;
    reasoning_content?: string;
    tool_calls?: unknown[];
    thinking_blocks?: unknown[];
    citations?: unknown[];
    server_content_blocks?: unknown[];
    usage?: unknown;
    extra?: Record<string, unknown>;
  };

  // Two-pass: accumulate all chunks first, apply once — avoids O(n²)
  // string concatenation and repeated array spreading.
  const contentChunks: string[] = [];
  const reasoningChunks: string[] = [];
  const newCitations: unknown[] = [];
  const newServerBlocks: unknown[] = [];
  let lastToolCalls: unknown[] | undefined;
  let lastThinkingBlocks: unknown[] | undefined;
  let lastUsage: unknown;
  let mergedExtra: Record<string, unknown> | undefined;

  for (const op of ops) {
    switch (op.op) {
      case "append_content":
        contentChunks.push(op.text);
        break;
      case "append_reasoning":
        reasoningChunks.push(op.text);
        break;
      case "set_tool_calls":
        lastToolCalls = op.tool_calls;
        break;
      case "set_thinking_blocks":
        lastThinkingBlocks = op.blocks;
        break;
      case "add_citation":
        newCitations.push(op.citation);
        break;
      case "add_server_content_block":
        newServerBlocks.push(op.block);
        break;
      case "set_usage":
        lastUsage = op.usage;
        break;
      case "merge_extra":
        if (mergedExtra) {
          Object.assign(mergedExtra, op.extra);
        } else {
          mergedExtra = { ...op.extra };
        }
        break;
    }
  }

  if (contentChunks.length > 0) {
    const appended = contentChunks.join("");
    updated.content =
      typeof updated.content === "string"
        ? updated.content + appended
        : appended;
  }

  if (reasoningChunks.length > 0) {
    updated.reasoning_content =
      (updated.reasoning_content ?? "") + reasoningChunks.join("");
  }

  if (lastToolCalls !== undefined) {
    updated.tool_calls = lastToolCalls;
  }

  if (lastThinkingBlocks !== undefined) {
    updated.thinking_blocks = lastThinkingBlocks;
  }

  if (newCitations.length > 0) {
    const existing = updated.citations ?? [];
    updated.citations = existing.concat(newCitations);
  }

  if (newServerBlocks.length > 0) {
    const existing = updated.server_content_blocks ?? [];
    updated.server_content_blocks = existing.concat(newServerBlocks);
  }

  if (lastUsage !== undefined) {
    updated.usage = lastUsage;
  }

  if (mergedExtra) {
    updated.extra = updated.extra
      ? Object.assign({}, updated.extra, mergedExtra)
      : mergedExtra;
  }

  return updated;
}
