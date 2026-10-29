import {
  isAssistantMessage,
  getAssistantCompressionMetadata,
  isEventMessage,
  isToolMessage,
  normalizeEventMessageMetadata,
  type BackgroundAgentStatus,
  type ChatMessages,
  type EventMessage,
  type ToolMessage,
} from "../../services/refact";
import { normalizeToolName } from "../../utils/toolNameAliases";
import {
  eventPayloadRecord,
  isRecord,
  payloadString,
} from "./EventRow/eventPayload";
import { processIdFromStartResult } from "./ToolCard/execToolModel";

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
  const action = payloadString(parseArgs(rawArgs), "action");
  return action !== null && SERVICE_START_ACTIONS.has(action);
}

function startedProcessId(message: ToolMessage): string | null {
  return processIdFromStartResult(message.extra, message.content);
}

function launchedAgentId(message: ToolMessage): string | null {
  const direct = message.background_agent_id?.trim();
  return direct
    ? direct
    : payloadString(message.extra ?? null, "background_agent_id");
}

function payloadField(event: EventMessage, field: string): string | null {
  return payloadString(eventPayloadRecord(event), field);
}

export function isAgentStatus(value: unknown): value is BackgroundAgentStatus {
  return AGENT_STATUSES.some((status) => status === value);
}

function processCompletedId(event: EventMessage): string | null {
  if (event.subkind !== PROCESS_COMPLETED_SUBKIND) return null;
  return payloadField(event, "process_id");
}

function isAgentFinishedEvent(event: EventMessage): boolean {
  return (
    event.source === AGENT_FINISHED_SOURCE &&
    isAgentStatus(payloadField(event, "status"))
  );
}

function agentLaunchCallId(
  event: EventMessage,
  startKinds: ReadonlyMap<string, "process" | "agent">,
  toolCallByAgentId: ReadonlyMap<string, string>,
): string | undefined {
  const parentCallId = payloadField(event, "parent_tool_call_id");
  if (parentCallId && startKinds.get(parentCallId) === "agent") {
    return parentCallId;
  }
  const agentId = payloadField(event, "agent_id");
  return agentId ? toolCallByAgentId.get(agentId) : undefined;
}

export function buildFoldedEventIndex(
  messages: ChatMessages,
): FoldedEventIndex {
  const startKinds = new Map<string, "process" | "agent">();
  const toolCallByProcessId = new Map<string, string>();
  const toolCallByAgentId = new Map<string, string>();

  for (const message of messages) {
    if (isAssistantMessage(message)) {
      if (getAssistantCompressionMetadata(message)) continue;
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

  const hasAgentLaunch = [...startKinds.values()].includes("agent");
  if (toolCallByProcessId.size === 0 && !hasAgentLaunch) return EMPTY_INDEX;

  const foldedMessageIndices = new Set<number>();
  const processEventByToolCallId = new Map<string, EventMessage>();
  const agentEventByToolCallId = new Map<string, EventMessage>();

  messages.forEach((message, index) => {
    if (!isEventMessage(message)) return;
    const event = normalizeEventMessageMetadata(message);
    const processId = processCompletedId(event);
    const processToolCallId = processId
      ? toolCallByProcessId.get(processId)
      : undefined;
    if (processToolCallId) {
      foldedMessageIndices.add(index);
      processEventByToolCallId.set(processToolCallId, event);
      return;
    }
    if (!isAgentFinishedEvent(event)) return;
    const agentToolCallId = agentLaunchCallId(
      event,
      startKinds,
      toolCallByAgentId,
    );
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
