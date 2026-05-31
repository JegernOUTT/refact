import { v4 as uuidv4 } from "uuid";
import { buildApiUrl, type EngineApiConfig } from "./apiUrl";

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

export async function sendChatCommand(
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
