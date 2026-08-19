import type { RootState } from "../app/store";
import type { ChatThread } from "../features/Chat/Thread";
import type { ChatMessages } from "../services/refact";

export const STORY_CHAT_ID = "story-chat";

export type ChatSliceState = RootState["chat"];
export type ThreadRuntime = NonNullable<ChatSliceState["threads"][string]>;

export function makeChatThread(overrides?: Partial<ChatThread>): ChatThread {
  return {
    id: STORY_CHAT_ID,
    model: "gpt-4o",
    messages: [],
    new_chat_suggested: {
      wasSuggested: false,
    },
    ...overrides,
  };
}

export function makeThreadRuntime(
  thread: ChatThread,
  overrides?: Partial<ThreadRuntime>,
): ThreadRuntime {
  return {
    thread,
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    attached_text_files: [],
    background_agents: {},
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: { wasInteracted: false, confirmationStatus: true },
    },
    snapshot_received: true,
    task_widget_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
    ...overrides,
  };
}

export function makeChatSlice(
  thread: ChatThread,
  runtimeOverrides?: Partial<ThreadRuntime>,
): ChatSliceState {
  return {
    current_thread_id: thread.id,
    open_thread_ids: [thread.id],
    threads: {
      [thread.id]: makeThreadRuntime(thread, runtimeOverrides),
    },
    max_new_tokens: 4096,
    tool_use: "agent",
    system_prompt: {},
    sse_refresh_requested: null,
    stream_version: 0,
  } as ChatSliceState;
}

export function makeMessagesThread(messages: ChatMessages): ChatThread {
  return makeChatThread({ messages });
}
