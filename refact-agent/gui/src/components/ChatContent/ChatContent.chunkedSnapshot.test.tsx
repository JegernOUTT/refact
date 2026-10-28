import { afterEach, describe, expect, it, vi } from "vitest";
import type { UnknownAction } from "@reduxjs/toolkit";
import { setUpStore } from "../../app/store";
import { registerVisibleChatMount } from "../../features/Connection";
import { useAllChatsSubscription } from "../../hooks/useAllChatsSubscription";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import { ChatContent } from "./ChatContent";

const MESSAGE_COUNT = 60;
const CHUNK_SIZE = 8;

function chunkedSnapshotStream(chatId: string): string {
  const messages = Array.from({ length: MESSAGE_COUNT }, (_, i) =>
    i % 2 === 0
      ? { role: "user", content: `question ${i}`, message_id: `m-${i}` }
      : { role: "assistant", content: `answer ${i}`, message_id: `m-${i}` },
  );
  const chunks: (typeof messages)[] = [];
  for (let i = 0; i < messages.length; i += CHUNK_SIZE) {
    chunks.push(messages.slice(i, i + CHUNK_SIZE));
  }
  const events = [
    {
      type: "snapshot",
      seq: "1",
      chat_id: chatId,
      thread: {
        id: chatId,
        title: "Large chat",
        model: "model",
        mode: "agent",
        tool_use: "agent",
        boost_reasoning: false,
        context_tokens_cap: null,
        include_project_info: true,
        checkpoints_enabled: false,
        is_title_generated: false,
      },
      runtime: {
        state: "idle",
        paused: false,
        error: null,
        queue_size: 0,
        pause_reasons: [],
        queued_items: [],
      },
      messages: [],
      background_agents: [],
      snapshot_chunked: true,
      chunk_count: chunks.length,
      message_count: messages.length,
    },
    ...chunks.map((chunk, chunkIndex) => ({
      type: "snapshot_chunk",
      seq: "1",
      chat_id: chatId,
      chunk_index: chunkIndex,
      chunk_count: chunks.length,
      messages: chunk,
    })),
    {
      type: "snapshot_end",
      seq: "1",
      chat_id: chatId,
      chunk_count: chunks.length,
      message_count: messages.length,
    },
  ];
  return events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join("");
}

function mockSseFetch(payload: string, realFetch: typeof fetch) {
  const encoder = new TextEncoder();
  return vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    if (!String(input).includes("/v1/chats/subscribe")) {
      return realFetch(input, init);
    }
    return Promise.resolve({
      ok: true,
      body: {
        getReader: () => {
          let sent = false;
          return {
            read: () => {
              if (sent) return new Promise<never>(() => undefined);
              sent = true;
              return Promise.resolve({
                done: false,
                value: encoder.encode(payload),
              });
            },
          };
        },
      },
    });
  });
}

const SubscribedChat = () => {
  useAllChatsSubscription();
  return (
    <ChatContent onRetry={() => undefined} onStopStreaming={() => undefined} />
  );
};

describe("ChatContent with a chunked snapshot", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders the message list right after the snapshot is applied, without further store actions", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].snapshot_received = false;
    vi.stubGlobal(
      "fetch",
      mockSseFetch(chunkedSnapshotStream(chatId), globalThis.fetch),
    );

    const store = setUpStore({
      chat,
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
    });
    const actionsAfterSnapshot: string[] = [];
    let snapshotApplied = false;
    let listRendered = false;
    const dispatch = store.dispatch;
    store.dispatch = ((action: UnknownAction) => {
      const result = dispatch(action);
      if (snapshotApplied && !listRendered && typeof action === "object") {
        actionsAfterSnapshot.push(action.type);
      }
      if (
        typeof action === "object" &&
        action.type === "chatThread/applyChatEvent" &&
        (action.payload as { type?: string }).type === "snapshot"
      ) {
        snapshotApplied = true;
      }
      return result;
    }) as typeof store.dispatch;
    store.dispatch(registerVisibleChatMount({ chatId }));

    render(<SubscribedChat />, { store });

    const firstItems = await screen.findAllByTestId("chat-virtuoso-item");
    listRendered = true;
    expect(snapshotApplied).toBe(true);
    expect(firstItems.length).toBeGreaterThan(0);
    expect(screen.getByText("question 0")).toBeInTheDocument();
    expect(screen.getByText(`answer ${MESSAGE_COUNT - 1}`)).toBeInTheDocument();
    expect(screen.queryByTestId("chat-loading")).not.toBeInTheDocument();
    expect(
      actionsAfterSnapshot.filter(
        (type) =>
          !type.startsWith("connection/") &&
          !type.includes("/subscriptions/") &&
          !type.includes("/executeQuery/"),
      ),
    ).toEqual([]);
  });
});
