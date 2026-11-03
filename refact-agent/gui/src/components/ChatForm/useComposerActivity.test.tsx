import { describe, expect, test } from "vitest";
import { render, screen } from "../../utils/test-utils";
import type { Chat, ChatThreadRuntime } from "../../features/Chat/Thread/types";
import { useComposerActivity } from "./useComposerActivity";

const chatId = "activity-chat";

const Probe = () => (
  <span data-testid="activity">{String(useComposerActivity(chatId))}</span>
);

function renderProbe(
  runtime: Partial<ChatThreadRuntime>,
  sse: "connecting" | "connected" | "disconnected" | null,
  hasMessages: boolean,
) {
  const base = {
    thread: {
      id: chatId,
      messages: hasMessages
        ? [{ role: "user", content: "hi", message_id: "m1" }]
        : [],
      title: "Activity",
      model: "gpt-4",
      mode: "agent",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      increase_max_tokens: false,
      include_project_info: true,
      auto_enrichment_enabled: false,
    },
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
    task_goal_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
    ...runtime,
  } as ChatThreadRuntime;
  const chat: Chat = {
    current_thread_id: chatId,
    open_thread_ids: [chatId],
    threads: { [chatId]: base },
    system_prompt: {},
    tool_use: "agent",
    sse_refresh_requested: null,
    stream_version: 0,
  };
  render(<Probe />, {
    preloadedState: {
      chat,
      connection: {
        browserOnline: true,
        backendStatus: "online",
        backendLastOkAt: 0,
        backendError: null,
        sseConnections: sse
          ? {
              [chatId]: {
                status: sse,
                lastEventAt: null,
                retryCount: 0,
                error: null,
              },
            }
          : {},
        visibleChatMounts: {},
        suspended: false,
      },
    },
  });
  return screen.getByTestId("activity").textContent;
}

describe("useComposerActivity", () => {
  test("an empty chat waiting for its first snapshot is loading", () => {
    expect(renderProbe({ snapshot_received: false }, "connecting", false)).toBe(
      "loading",
    );
  });

  test("a disconnected empty chat is not shown as loading", () => {
    expect(
      renderProbe({ snapshot_received: false }, "disconnected", false),
    ).toBe("null");
  });

  test("a streaming chat is working", () => {
    expect(renderProbe({ streaming: true }, "connected", true)).toBe("working");
  });

  test("an idle chat with messages has no activity", () => {
    expect(renderProbe({}, "connected", true)).toBe("null");
  });
});
