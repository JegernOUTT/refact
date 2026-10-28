import {
  extractExecMetadata,
  isAssistantMessage,
  isEventMessage,
  isToolMessage,
  normalizeEventMessageMetadata,
  type BackgroundAgentStatus,
  type ChatMessages,
  type EventMessage,
  type ToolMessage,
} from "../../services/refact";
import { normalizeToolName } from "../../utils/toolNameAliases";

export type FoldedEventIndex = {
  foldedMessageIndices: ReadonlySet<number>;
  processEventByToolCallId: ReadonlyMap<string, EventMessage>;
  agentEventByToolCallId: ReadonlyMap<string, EventMessage>;
};

const PROCESS_START_TOOLS = new Set(["process_start", "shell"]);
const SERVICE_START_ACTIONS = new Set(["start", "restart"]);
const AGENT_LAUNCH_TOOLS = new Set(["subagent"]);
const AGENT_STATUSES: readonly BackgroundAgentStatus[] = [
  "queued",
  "running",
  "waiting_for_approval",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];

const PROCESS_COMPLETED_SUBKIND = "process_completed";
const AGENT_FINISHED_SOURCE = "agents.spawn";

const EMPTY_INDEX: FoldedEventIndex = {
  foldedMessageIndices: new Set(),
  processEventByToolCallId: new Map(),
  agentEventByToolCallId: new Map(),
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function parseArgs(raw: string | undefined): Record<string, unknown> {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function startsProcess(name: string, rawArgs: string | undefined): boolean {
  if (PROCESS_START_TOOLS.has(name)) return true;
  if (name !== "shell_service") return false;
  const action = nonEmptyString(parseArgs(rawArgs).action);
  return action !== null && SERVICE_START_ACTIONS.has(action);
}

function startedProcessId(message: ToolMessage): string | null {
  const metadata = extractExecMetadata(message.extra);
  if (!metadata || "processes" in metadata) return null;
  return nonEmptyString(metadata.process_id);
}

function launchedAgentId(message: ToolMessage): string | null {
  return (
    nonEmptyString(message.background_agent_id) ??
    nonEmptyString(message.extra?.background_agent_id)
  );
}

function payloadField(event: EventMessage, field: string): string | null {
  const payload = event.payload;
  return isRecord(payload) ? nonEmptyString(payload[field]) : null;
}

export function isAgentStatus(value: unknown): value is BackgroundAgentStatus {
  return AGENT_STATUSES.some((status) => status === value);
}

function processCompletedId(event: EventMessage): string | null {
  if (event.subkind !== PROCESS_COMPLETED_SUBKIND) return null;
  return payloadField(event, "process_id");
}

function finishedAgentId(event: EventMessage): string | null {
  if (event.source !== AGENT_FINISHED_SOURCE) return null;
  const status = payloadField(event, "status");
  if (!isAgentStatus(status)) return null;
  return payloadField(event, "agent_id");
}

function normalizedEvent(
  message: ChatMessages[number],
): EventMessage | undefined {
  return isEventMessage(message)
    ? normalizeEventMessageMetadata(message)
    : undefined;
}

export function buildFoldedEventIndex(
  messages: ChatMessages,
): FoldedEventIndex {
  const startKinds = new Map<string, "process" | "agent">();
  const toolCallByProcessId = new Map<string, string>();
  const toolCallByAgentId = new Map<string, string>();

  for (const message of messages) {
    if (isAssistantMessage(message)) {
      for (const call of message.tool_calls ?? []) {
        const name = normalizeToolName(call.function.name);
        if (!call.id || !name) continue;
        if (startsProcess(name, call.function.arguments)) {
          startKinds.set(call.id, "process");
        } else if (AGENT_LAUNCH_TOOLS.has(name)) {
          startKinds.set(call.id, "agent");
        }
      }
      continue;
    }
    if (!isToolMessage(message)) continue;
    const kind = startKinds.get(message.tool_call_id);
    if (kind === "process") {
      const processId = startedProcessId(message);
      if (processId && !toolCallByProcessId.has(processId)) {
        toolCallByProcessId.set(processId, message.tool_call_id);
      }
    } else if (kind === "agent") {
      const agentId = launchedAgentId(message);
      if (agentId && !toolCallByAgentId.has(agentId)) {
        toolCallByAgentId.set(agentId, message.tool_call_id);
      }
    }
  }

  if (toolCallByProcessId.size === 0 && toolCallByAgentId.size === 0) {
    return EMPTY_INDEX;
  }

  const foldedMessageIndices = new Set<number>();
  const processEventByToolCallId = new Map<string, EventMessage>();
  const agentEventByToolCallId = new Map<string, EventMessage>();

  messages.forEach((message, index) => {
    const event = normalizedEvent(message);
    if (!event) return;
    const processId = processCompletedId(event);
    const processToolCallId = processId
      ? toolCallByProcessId.get(processId)
      : undefined;
    if (processToolCallId) {
      foldedMessageIndices.add(index);
      processEventByToolCallId.set(processToolCallId, event);
      return;
    }
    const agentId = finishedAgentId(event);
    const agentToolCallId = agentId
      ? toolCallByAgentId.get(agentId)
      : undefined;
    if (agentToolCallId) {
      foldedMessageIndices.add(index);
      agentEventByToolCallId.set(agentToolCallId, event);
    }
  });

  if (foldedMessageIndices.size === 0) return EMPTY_INDEX;
  return {
    foldedMessageIndices,
    processEventByToolCallId,
    agentEventByToolCallId,
  };
}

const indexCache = new WeakMap<ChatMessages, FoldedEventIndex>();

export function foldedEventIndexFor(messages: ChatMessages): FoldedEventIndex {
  const cached = indexCache.get(messages);
  if (cached) return cached;
  const index = buildFoldedEventIndex(messages);
  indexCache.set(messages, index);
  return index;
}
