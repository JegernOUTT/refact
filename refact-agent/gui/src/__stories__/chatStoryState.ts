import { http, HttpResponse, type HttpHandler } from "msw";
import {
  goodCaps,
  goodChatModes,
  goodPing,
  goodPrompts,
  goodUser,
  goodVoiceStatus,
  emptyWorktrees,
  emptyTasks,
  noCommandPreview,
  noCompletions,
  noTools,
  ToolConfirmation,
} from "../__fixtures__/msw";
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

export type StoryAppearance = "light" | "dark";

// The preview decorator mirrors the Storybook appearance global onto <html>
// (data-appearance), which is the only cross-portal channel available to a
// story-local provider tree. Falling back to "dark" keeps existing stories
// rendering exactly as before when the attribute is missing (e.g. in Node).
export function resolveStoryAppearance(
  explicit?: StoryAppearance,
): StoryAppearance {
  if (explicit === "light" || explicit === "dark") return explicit;
  if (typeof document === "undefined") return "dark";
  const fromDocument = document.documentElement.dataset.appearance;
  return fromDocument === "light" || fromDocument === "dark"
    ? fromDocument
    : "dark";
}

const noExecList: HttpHandler = http.get("*/v1/exec/list", () =>
  HttpResponse.json({ processes: [] }),
);

const noSkillsStatus: HttpHandler = http.get(
  "*/v1/chats/:chatId/skills-status",
  () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_excluded: [],
      active_skill: null,
    }),
);

const noBuddyOpportunities: HttpHandler = http.get(
  "*/v1/buddy/opportunities",
  () => HttpResponse.json({ opportunities: [] }),
);

const goodPrivacyPolicy: HttpHandler = http.get("*/v1/privacy/policy", () =>
  HttpResponse.json({
    policy: {
      blocked: [],
      zones: [],
      subagents: { report_declassifies: true },
      tool_access: { providers: {} },
    },
    destinations: [],
    match_counts: {},
    error: null,
    source_paths: [],
    has_project_overrides: false,
  }),
);

const goodPrivacyInspect: HttpHandler = http.post("*/v1/privacy/inspect", () =>
  HttpResponse.json({
    chat_id: STORY_CHAT_ID,
    destination: { id: "gpt-4o", kind: "provider", display_name: "gpt-4o" },
    sendable: true,
    would_send: [],
    records: [],
    blocked: [],
    refusal: null,
  }),
);

// Every endpoint the chat shell polls on mount. MSW resolves the first
// matching handler in the array, so a story that needs a different response
// for one of these should prepend its own handler to this list.
export const CHAT_STORY_MSW_HANDLERS: HttpHandler[] = [
  goodPing,
  goodCaps,
  goodChatModes,
  goodPrompts,
  goodUser,
  goodVoiceStatus,
  noTools,
  noCompletions,
  noCommandPreview,
  ToolConfirmation,
  emptyWorktrees,
  emptyTasks,
  noExecList,
  noSkillsStatus,
  noBuddyOpportunities,
  goodPrivacyPolicy,
  goodPrivacyInspect,
];
