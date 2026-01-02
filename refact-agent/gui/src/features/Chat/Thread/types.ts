import { ToolConfirmationPauseReason, Usage } from "../../../services/refact";
import { SystemPrompts } from "../../../services/refact/prompts";
import { ChatMessages } from "../../../services/refact/types";
import { parseOrElse } from "../../../utils/parseOrElse";

export type ImageFile = {
  name: string;
  content: string | ArrayBuffer | null;
  type: string;
};

export type ToolConfirmationStatus = {
  wasInteracted: boolean;
  confirmationStatus: boolean;
};

export type QueuedItem = {
  client_request_id: string;
  priority: boolean;
  command_type: string;
  preview: string;
};

export type IntegrationMeta = {
  name?: string;
  path?: string;
  project?: string;
  shouldIntermediatePageShowUp?: boolean;
};

export type ChatThread = {
  id: string;
  messages: ChatMessages;
  model: string;
  title?: string;
  createdAt?: string;
  updatedAt?: string;
  tool_use?: ToolUse;
  read?: boolean;
  isTitleGenerated?: boolean;
  boost_reasoning?: boolean;
  integration?: IntegrationMeta | null;
  mode?: LspChatMode;
  project_name?: string;
  last_user_message_id?: string;
  new_chat_suggested: SuggestedChat;
  automatic_patch?: boolean;
  currentMaximumContextTokens?: number;
  currentMessageContextTokens?: number;
  increase_max_tokens?: boolean;
  include_project_info?: boolean;
  context_tokens_cap?: number;
  checkpoints_enabled?: boolean;
  /** If true, this chat belongs to a task workspace and should not appear in regular chat tabs */
  is_task_chat?: boolean;
  /** Task metadata for task-related chats */
  task_meta?: {
    task_id: string;
    role: string;
    agent_id?: string;
    card_id?: string;
  };
};

export type SuggestedChat = {
  wasSuggested: boolean;
  wasRejectedByUser?: boolean;
};

export type ToolUse = "quick" | "explore" | "agent";

export type ThreadConfirmation = {
  pause: boolean;
  pause_reasons: ToolConfirmationPauseReason[];
  status: ToolConfirmationStatus;
};

export type ChatThreadRuntime = {
  thread: ChatThread;
  streaming: boolean;
  waiting_for_response: boolean;
  prevent_send: boolean;
  error: string | null;
  queued_items: QueuedItem[];
  send_immediately: boolean;
  attached_images: ImageFile[];
  confirmation: ThreadConfirmation;
  /** Whether the initial snapshot has been received from the backend */
  snapshot_received: boolean;
};

export type Chat = {
  current_thread_id: string;
  open_thread_ids: string[];
  threads: Record<string, ChatThreadRuntime | undefined>;
  system_prompt: SystemPrompts;
  tool_use: ToolUse;
  checkpoints_enabled?: boolean;
  follow_ups_enabled?: boolean;
  use_compression?: boolean;
  max_new_tokens?: number;
};

export type PayloadWithId = { id: string };
export type PayloadWithChatAndNumber = { chatId: string; value: number };
export type PayloadWithChatAndMessageId = { chatId: string; messageId: string };
export type PayloadWithChatAndBoolean = { chatId: string; value: boolean };
export type PayloadWithChatAndUsage = { chatId: string; usage: Usage };
export type PayloadWithChatAndCurrentUsage = {
  chatId: string;
  n_ctx: number;
  prompt_tokens: number;
};
export type PayloadWithIdAndTitle = {
  title: string;
  isTitleGenerated: boolean;
} & PayloadWithId;

export type DetailMessage = { detail: string };

// LiteLLM streaming error format: {"error": {"message": "...", "type": "...", "code": "..."}}
export type StreamingErrorChunk = {
  error: {
    message: string;
    type: string;
    code?: string;
  };
};

function isDetailMessage(json: unknown): json is DetailMessage {
  if (!json) return false;
  if (typeof json !== "object") return false;
  return "detail" in json && typeof json.detail === "string";
}

function isStreamingError(json: unknown): json is StreamingErrorChunk {
  if (!json || typeof json !== "object") return false;
  const obj = json as Record<string, unknown>;
  if (!obj.error || typeof obj.error !== "object") return false;
  const err = obj.error as Record<string, unknown>;
  return typeof err.message === "string";
}

export function checkForDetailMessage(str: string): DetailMessage | false {
  const json = parseOrElse(str, {});
  if (isDetailMessage(json)) return json;
  // Handle LiteLLM error format by converting it to DetailMessage
  if (isStreamingError(json)) {
    return { detail: json.error.message };
  }
  return false;
}

export function isToolUse(str: string): str is ToolUse {
  if (!str) return false;
  if (typeof str !== "string") return false;
  return str === "quick" || str === "explore" || str === "agent";
}

export type LspChatMode =
  | "NO_TOOLS"
  | "EXPLORE"
  | "AGENT"
  | "CONFIGURE"
  | "PROJECT_SUMMARY";

export function isLspChatMode(mode: string): mode is LspChatMode {
  return (
    mode === "NO_TOOLS" ||
    mode === "EXPLORE" ||
    mode === "AGENT" ||
    mode === "CONFIGURE" ||
    mode === "PROJECT_SUMMARY"
  );
}

export function chatModeToLspMode({
  toolUse,
  mode,
  defaultMode,
}: {
  toolUse?: ToolUse;
  mode?: LspChatMode;
  defaultMode?: LspChatMode;
}): LspChatMode {
  if (defaultMode) {
    return defaultMode;
  }
  if (mode) {
    return mode;
  }
  if (toolUse === "agent") return "AGENT";
  if (toolUse === "quick") return "NO_TOOLS";
  return "EXPLORE";
}

// Helper to detect server-executed tools (already executed by LLM provider)
// These tools have IDs starting with "srvtoolu_" and should NOT be sent to backend for execution
export function isServerExecutedTool(toolCallId: string | undefined): boolean {
  return toolCallId?.startsWith("srvtoolu_") ?? false;
}
