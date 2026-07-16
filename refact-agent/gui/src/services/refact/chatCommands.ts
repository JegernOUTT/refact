import { v4 as uuidv4 } from "uuid";
import {
  buildApiUrl,
  getEngineEndpointIdentity,
  type EngineApiConfig,
} from "./apiUrl";

export type EngineApiConnection = EngineApiConfig;
export type PortOrConnection = number | EngineApiConnection;

export function normalizeConnection(input: PortOrConnection): EngineApiConfig {
  if (typeof input === "number") {
    return { host: "ide", lspPort: input };
  }
  return input;
}

export type MessageContent =
  | string
  | (
      | { type: "text"; text: string }
      | { type: "image_url"; image_url: { url: string } }
    )[];

export type GoalBudgetCommand = {
  max_turns?: number;
  max_minutes?: number;
  max_tokens?: number;
  cooldown_ms?: number;
  no_progress_token_threshold?: number;
  no_progress_turns?: number;
};

export type ChatCommandBase =
  | {
      type: "user_message";
      content: MessageContent;
      attachments?: unknown[];
    }
  | {
      type: "retry_from_index";
      index: number;
      content?: MessageContent;
      attachments?: unknown[];
    }
  | {
      type: "set_params";
      patch: Record<string, unknown>;
    }
  | {
      type: "set_goal";
      content: string;
      budget?: GoalBudgetCommand;
    }
  | {
      type: "set_goal_budget";
      budget: GoalBudgetCommand;
    }
  | {
      type: "update_goal";
      note: string;
    }
  | {
      type: "goal_control";
      action: GoalControlAction;
    }
  | {
      type: "abort";
    }
  | {
      type: "tool_decision";
      tool_call_id: string;
      accepted: boolean;
    }
  | {
      type: "tool_decisions";
      decisions: { tool_call_id: string; accepted: boolean }[];
    }
  | {
      type: "ide_tool_result";
      tool_call_id: string;
      content: string;
      tool_failed: boolean;
    }
  | {
      type: "update_message";
      message_id: string;
      content: MessageContent;
      attachments?: unknown[];
      regenerate?: boolean;
    }
  | {
      type: "remove_message";
      message_id: string;
      regenerate?: boolean;
    }
  | {
      type: "regenerate";
    }
  | {
      type: "branch_from_chat";
      source_chat_id: string;
      up_to_message_id: string;
    }
  | {
      type: "browser_context_decision";
      pending_message_id: string;
      include_actions: boolean;
      include_console: boolean;
      include_network: boolean;
      include_mutations: boolean;
      include_screenshot: boolean;
      last_n_actions?: number | null;
      last_n_console?: number | null;
      last_n_network?: number | null;
    };

export type ChatCommand = ChatCommandBase & {
  client_request_id: string;
  priority?: boolean;
};

export type GoalControlAction = "pause" | "resume" | "stop";

const appliedChatParams = new Map<string, Record<string, unknown>>();
const chatParamsSyncTails = new Map<string, Promise<void>>();
const chatParamsSyncEpochs = new Map<string, number>();

function chatParamsKey(chatId: string, connection: PortOrConnection): string {
  return `${getEngineEndpointIdentity(
    normalizeConnection(connection),
  )}\n${chatId}`;
}

function normalizeParamsPatch(
  patch: Record<string, unknown>,
): Record<string, unknown> {
  return JSON.parse(JSON.stringify(patch)) as Record<string, unknown>;
}

function jsonValuesEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right)) return false;
    return (
      left.length === right.length &&
      left.every((value, index) => jsonValuesEqual(value, right[index]))
    );
  }
  if (
    left === null ||
    right === null ||
    typeof left !== "object" ||
    typeof right !== "object"
  ) {
    return false;
  }
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord).sort();
  const rightKeys = Object.keys(rightRecord).sort();
  return (
    leftKeys.length === rightKeys.length &&
    leftKeys.every(
      (key, index) =>
        key === rightKeys[index] &&
        jsonValuesEqual(leftRecord[key], rightRecord[key]),
    )
  );
}

function changedParams(
  applied: Record<string, unknown>,
  requested: Record<string, unknown>,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(requested).filter(
      ([key, value]) =>
        !Object.prototype.hasOwnProperty.call(applied, key) ||
        !jsonValuesEqual(applied[key], value),
    ),
  );
}

function mergeAppliedParams(
  applied: Record<string, unknown>,
  delta: Record<string, unknown>,
): Record<string, unknown> {
  const next = { ...applied, ...delta };
  if (Object.prototype.hasOwnProperty.call(delta, "worktree_id")) {
    delete next.worktree;
  } else if (delta.worktree === null) {
    delete next.worktree_id;
  }
  return next;
}

export function resetChatParamsSyncState(): void {
  appliedChatParams.clear();
  chatParamsSyncTails.clear();
  chatParamsSyncEpochs.clear();
}

export function invalidateChatParamsSyncState(
  chatId: string,
  connection: PortOrConnection,
): void {
  const key = chatParamsKey(chatId, connection);
  appliedChatParams.delete(key);
  if (chatParamsSyncTails.has(key)) {
    chatParamsSyncEpochs.set(key, (chatParamsSyncEpochs.get(key) ?? 0) + 1);
  } else {
    chatParamsSyncEpochs.delete(key);
  }
}

function commandUrl(connection: PortOrConnection, chatId: string): string {
  return buildApiUrl(
    normalizeConnection(connection),
    `/v1/chats/${encodeURIComponent(chatId)}/commands`,
  );
}

function queueItemUrl(
  connection: PortOrConnection,
  chatId: string,
  clientRequestId: string,
): string {
  return buildApiUrl(
    normalizeConnection(connection),
    `/v1/chats/${encodeURIComponent(chatId)}/queue/${encodeURIComponent(
      clientRequestId,
    )}`,
  );
}

async function sendRawChatCommand(
  chatId: string,
  connection: PortOrConnection,
  apiKey: string | undefined,
  command: ChatCommandBase,
  priority?: boolean,
): Promise<void> {
  const commandWithId: ChatCommand = {
    ...command,
    client_request_id: uuidv4(),
    priority,
  };

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };

  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const response = await fetch(commandUrl(connection, chatId), {
    method: "POST",
    headers,
    body: JSON.stringify(commandWithId),
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Failed to send command: ${response.status} ${response.statusText} - ${text}`,
    );
  }
}

async function sendChangedChatParams(
  chatId: string,
  connection: PortOrConnection,
  apiKey: string | undefined,
  patch: Record<string, unknown>,
  priority?: boolean,
): Promise<void> {
  const normalizedPatch = normalizeParamsPatch(patch);
  if (Object.keys(normalizedPatch).length === 0) return;

  const key = chatParamsKey(chatId, connection);
  const previousTail = chatParamsSyncTails.get(key) ?? Promise.resolve();
  const currentSync = previousTail
    .catch(() => undefined)
    .then(async () => {
      const epoch = chatParamsSyncEpochs.get(key) ?? 0;
      const applied = appliedChatParams.get(key) ?? {};
      const delta = changedParams(applied, normalizedPatch);
      if (Object.keys(delta).length === 0) return;

      await sendRawChatCommand(
        chatId,
        connection,
        apiKey,
        { type: "set_params", patch: delta },
        priority,
      );
      if ((chatParamsSyncEpochs.get(key) ?? 0) === epoch) {
        appliedChatParams.set(key, mergeAppliedParams(applied, delta));
      }
    });
  chatParamsSyncTails.set(key, currentSync);

  try {
    await currentSync;
  } finally {
    if (chatParamsSyncTails.get(key) === currentSync) {
      chatParamsSyncTails.delete(key);
      if (!appliedChatParams.has(key)) {
        chatParamsSyncEpochs.delete(key);
      }
    }
  }
}

export async function sendChatCommand(
  chatId: string,
  connection: PortOrConnection,
  apiKey: string | undefined,
  command: ChatCommandBase,
  priority?: boolean,
): Promise<void> {
  if (command.type === "set_params") {
    await sendChangedChatParams(
      chatId,
      connection,
      apiKey,
      command.patch,
      priority,
    );
    return;
  }
  await sendRawChatCommand(chatId, connection, apiKey, command, priority);
}

export async function sendUserMessage(
  chatId: string,
  content: MessageContent,
  connection: PortOrConnection,
  apiKey?: string,
  priority?: boolean,
  contextFiles?: unknown[],
  suppressAutoEnrichment?: boolean,
): Promise<void> {
  const cmd: Record<string, unknown> = { type: "user_message", content };
  if (contextFiles && contextFiles.length > 0) {
    cmd.context_files = contextFiles;
  }
  if (suppressAutoEnrichment) {
    cmd.suppress_auto_enrichment = true;
  }
  await sendChatCommand(
    chatId,
    connection,
    apiKey,
    cmd as ChatCommandBase,
    priority,
  );
}

export async function retryFromIndex(
  chatId: string,
  index: number,
  content: MessageContent,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "retry_from_index",
    index,
    content,
  } as ChatCommandBase);
}

export async function regenerate(
  chatId: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "regenerate",
  } as ChatCommandBase);
}

export async function updateChatParams(
  chatId: string,
  params: Record<string, unknown>,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "set_params",
    patch: params,
  } as ChatCommandBase);
}

export async function setGoal(
  chatId: string,
  content: string,
  connection: PortOrConnection,
  apiKey?: string,
  budget?: GoalBudgetCommand,
): Promise<void> {
  const command: ChatCommandBase = {
    type: "set_goal",
    content,
    ...(budget === undefined ? {} : { budget }),
  };
  await sendChatCommand(chatId, connection, apiKey, command);
}

export async function setGoalBudget(
  chatId: string,
  budget: GoalBudgetCommand,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "set_goal_budget",
    budget,
  });
}

export async function updateGoal(
  chatId: string,
  note: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "update_goal",
    note,
  });
}

export async function goalControl(
  chatId: string,
  action: GoalControlAction,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "goal_control",
    action,
  });
}

export async function abortGeneration(
  chatId: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "abort",
  } as ChatCommandBase);
}

export async function respondToToolConfirmation(
  chatId: string,
  toolCallId: string,
  accepted: boolean,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "tool_decision",
    tool_call_id: toolCallId,
    accepted,
  } as ChatCommandBase);
}

export async function respondToToolConfirmations(
  chatId: string,
  decisions: { tool_call_id: string; accepted: boolean }[],
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "tool_decisions",
    decisions,
  } as ChatCommandBase);
}

export async function updateMessage(
  chatId: string,
  messageId: string,
  content: MessageContent,
  connection: PortOrConnection,
  apiKey?: string,
  regenerate?: boolean,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "update_message",
    message_id: messageId,
    content,
    regenerate,
  } as ChatCommandBase);
}

export async function removeMessage(
  chatId: string,
  messageId: string,
  connection: PortOrConnection,
  apiKey?: string,
  regenerate?: boolean,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "remove_message",
    message_id: messageId,
    regenerate,
  } as ChatCommandBase);
}

export async function branchFromChat(
  targetChatId: string,
  sourceChatId: string,
  upToMessageId: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(targetChatId, connection, apiKey, {
    type: "branch_from_chat",
    source_chat_id: sourceChatId,
    up_to_message_id: upToMessageId,
  } as ChatCommandBase);
}

export async function sendBrowserContextDecision(
  chatId: string,
  connection: PortOrConnection,
  decision: {
    pending_message_id: string;
    include_actions: boolean;
    include_console: boolean;
    include_network: boolean;
    include_mutations: boolean;
    include_screenshot: boolean;
    last_n_actions?: number | null;
    last_n_console?: number | null;
    last_n_network?: number | null;
  },
  apiKey?: string,
): Promise<void> {
  await sendChatCommand(chatId, connection, apiKey, {
    type: "browser_context_decision",
    ...decision,
  });
}

export async function cancelQueuedItem(
  chatId: string,
  clientRequestId: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<boolean> {
  const headers: Record<string, string> = {};
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }
  const response = await fetch(
    queueItemUrl(connection, chatId, clientRequestId),
    {
      method: "DELETE",
      headers,
    },
  );
  return response.ok;
}
