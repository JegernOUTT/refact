import {
  isAssistantMessage,
  getAssistantCompressionMetadata,
  isEventMessage,
  isToolMessage,
  normalizeEventMessageMetadata,
  type AssistantMessage,
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
import {
  isProcessStartCall,
  processIdFromExecMetadata,
  processIdFromStartResult,
} from "./ToolCard/execToolModel";

export type FoldedEventIndex = {
  foldedMessageIndices: ReadonlySet<number>;
  processEventByToolCallId: ReadonlyMap<string, EventMessage>;
  agentEventByToolCallId: ReadonlyMap<string, EventMessage>;
};

type StartKind = "process" | "structured" | "agent";

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

function callStartKind(
  name: string,
  rawArgs: string | undefined,
): StartKind | null {
  if (AGENT_LAUNCH_TOOLS.has(name)) return "agent";
  if (name !== "process_start" && name !== "shell" && name !== "shell_service")
    return null;
  if (isProcessStartCall(name, parseArgs(rawArgs))) return "process";
  return name === "shell" ? "structured" : null;
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

type CallKinds = readonly (readonly [string, StartKind])[];

type ToolFacts = {
  processId: string | null;
  structuredProcessId: string | null;
  agentId: string | null;
};

type EventFacts = {
  processId: string | null;
  agentFinished: boolean;
  parentCallId: string | null;
  agentId: string | null;
  event: EventMessage;
};

const callKindsCache = new WeakMap<AssistantMessage, CallKinds>();
const toolFactsCache = new WeakMap<ToolMessage, ToolFacts>();
const eventFactsCache = new WeakMap<EventMessage, EventFacts>();

function assistantCallKinds(message: AssistantMessage): CallKinds {
  const cached = callKindsCache.get(message);
  if (cached) return cached;
  const kinds: [string, StartKind][] = [];
  if (!getAssistantCompressionMetadata(message)) {
    for (const call of message.tool_calls ?? []) {
      const name = normalizeToolName(call.function.name);
      if (!call.id || !name) continue;
      const kind = callStartKind(name, call.function.arguments);
      if (kind) kinds.push([call.id, kind]);
    }
  }
  callKindsCache.set(message, kinds);
  return kinds;
}

function toolFacts(message: ToolMessage): ToolFacts {
  const cached = toolFactsCache.get(message);
  if (cached) return cached;
  const facts: ToolFacts = {
    processId: processIdFromStartResult(message.extra, message.content),
    structuredProcessId: processIdFromExecMetadata(message.extra),
    agentId: launchedAgentId(message),
  };
  toolFactsCache.set(message, facts);
  return facts;
}

function eventFacts(message: EventMessage): EventFacts {
  const cached = eventFactsCache.get(message);
  if (cached) return cached;
  const event = normalizeEventMessageMetadata(message);
  const facts: EventFacts = {
    processId:
      event.subkind === PROCESS_COMPLETED_SUBKIND
        ? payloadField(event, "process_id")
        : null,
    agentFinished:
      event.source === AGENT_FINISHED_SOURCE &&
      isAgentStatus(payloadField(event, "status")),
    parentCallId: payloadField(event, "parent_tool_call_id"),
    agentId: payloadField(event, "agent_id"),
    event,
  };
  eventFactsCache.set(message, facts);
  return facts;
}

function startedProcessId(kind: StartKind, facts: ToolFacts): string | null {
  if (kind === "process") return facts.processId;
  if (kind === "structured") return facts.structuredProcessId;
  return null;
}

export function buildFoldedEventIndex(
  messages: ChatMessages,
): FoldedEventIndex {
  const startKinds = new Map<string, StartKind>();
  const toolCallByProcessId = new Map<string, string>();
  const toolCallByAgentId = new Map<string, string>();
  let hasAgentLaunch = false;

  for (const message of messages) {
    if (isAssistantMessage(message)) {
      for (const [id, kind] of assistantCallKinds(message)) {
        startKinds.set(id, kind);
        if (kind === "agent") hasAgentLaunch = true;
      }
      continue;
    }
    if (!isToolMessage(message)) continue;
    const kind = startKinds.get(message.tool_call_id);
    if (!kind) continue;
    const facts = toolFacts(message);
    if (kind === "agent") {
      if (facts.agentId && !toolCallByAgentId.has(facts.agentId)) {
        toolCallByAgentId.set(facts.agentId, message.tool_call_id);
      }
      continue;
    }
    const processId = startedProcessId(kind, facts);
    if (processId && !toolCallByProcessId.has(processId)) {
      toolCallByProcessId.set(processId, message.tool_call_id);
    }
  }

  if (toolCallByProcessId.size === 0 && !hasAgentLaunch) return EMPTY_INDEX;

  const foldedMessageIndices = new Set<number>();
  const processEventByToolCallId = new Map<string, EventMessage>();
  const agentEventByToolCallId = new Map<string, EventMessage>();

  messages.forEach((message, index) => {
    if (!isEventMessage(message)) return;
    const facts = eventFacts(message);
    const processToolCallId = facts.processId
      ? toolCallByProcessId.get(facts.processId)
      : undefined;
    if (processToolCallId) {
      foldedMessageIndices.add(index);
      processEventByToolCallId.set(processToolCallId, facts.event);
      return;
    }
    if (!facts.agentFinished) return;
    const agentToolCallId =
      facts.parentCallId && startKinds.get(facts.parentCallId) === "agent"
        ? facts.parentCallId
        : facts.agentId
          ? toolCallByAgentId.get(facts.agentId)
          : undefined;
    if (agentToolCallId) {
      foldedMessageIndices.add(index);
      agentEventByToolCallId.set(agentToolCallId, facts.event);
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
let lastBuilt: { messages: ChatMessages; index: FoldedEventIndex } | null =
  null;

function sameCallKinds(left: CallKinds, right: CallKinds): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  return left.every(
    ([id, kind], position) =>
      right[position][0] === id && right[position][1] === kind,
  );
}

function onlyLastAssistantTextChanged(
  previous: ChatMessages,
  next: ChatMessages,
): boolean {
  if (previous.length !== next.length || next.length === 0) return false;
  const last = next.length - 1;
  for (let position = 0; position < last; position++) {
    if (previous[position] !== next[position]) return false;
  }
  const before = previous[last];
  const after = next[last];
  if (before === after) return true;
  if (!isAssistantMessage(before) || !isAssistantMessage(after)) return false;
  return sameCallKinds(assistantCallKinds(before), assistantCallKinds(after));
}

export function foldedEventIndexFor(messages: ChatMessages): FoldedEventIndex {
  const cached = indexCache.get(messages);
  if (cached) return cached;
  const index =
    lastBuilt && onlyLastAssistantTextChanged(lastBuilt.messages, messages)
      ? lastBuilt.index
      : buildFoldedEventIndex(messages);
  indexCache.set(messages, index);
  lastBuilt = { messages, index };
  return index;
}
