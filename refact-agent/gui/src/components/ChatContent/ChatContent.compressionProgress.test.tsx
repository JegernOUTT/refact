import { describe, expect, it, vi } from "vitest";
import { ChatContent } from "./ChatContent";
import {
  act,
  createDefaultChatState,
  render,
  screen,
} from "../../utils/test-utils";
import type { RootState } from "../../app/store";
import type { ChatMessages } from "../../services/refact";
import { switchToThread } from "../../features/Chat/Thread/actions";
import type { ChatThreadRuntime } from "../../features/Chat/Thread/types";

function userMessage(content: string): ChatMessages[number] {
  return {
    role: "user",
    content,
    message_id: `user-${content}`,
  };
}

function makeRuntime({
  messages = [],
  isCompressing = false,
  snapshotReceived = true,
  isStreaming = false,
}: {
  messages?: ChatMessages;
  isCompressing?: boolean;
  snapshotReceived?: boolean;
  isStreaming?: boolean;
} = {}): ChatThreadRuntime {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const runtime = chat.threads[chatId];
  runtime.thread.messages = messages;
  runtime.is_compressing = isCompressing;
  runtime.snapshot_received = snapshotReceived;
  runtime.streaming = isStreaming;
  return runtime;
}

function makeChatState({
  messages = [],
  isCompressing = false,
  snapshotReceived = true,
  isStreaming = false,
  sseStatus,
}: {
  messages?: ChatMessages;
  isCompressing?: boolean;
  snapshotReceived?: boolean;
  isStreaming?: boolean;
  sseStatus?: "disconnected" | "connecting" | "connected";
} = {}): Partial<RootState> {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const runtime = chat.threads[chatId];
  runtime.thread.messages = messages;
  runtime.is_compressing = isCompressing;
  runtime.snapshot_received = snapshotReceived;
  runtime.streaming = isStreaming;
  return {
    chat,
    ...(sseStatus
      ? {
          connection: {
            browserOnline: true,
            backendStatus: "unknown" as const,
            backendLastOkAt: null,
            backendError: null,
            sseConnections: {
              [chatId]: {
                status: sseStatus,
                lastEventAt: null,
                retryCount: 0,
                error: null,
              },
            },
          },
        }
      : {}),
  };
}

function renderChatContent(preloadedState: Partial<RootState>) {
  render(
    <ChatContent onRetry={() => undefined} onStopStreaming={() => undefined} />,
    {
      preloadedState,
    },
  );
}

describe("ChatContent compression progress", () => {
  it("renders progress for an empty compressing thread", async () => {
    renderChatContent(makeChatState({ isCompressing: true }));

    expect(
      await screen.findByTestId("compression-progress"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("chat-virtualized-list-wrapper"),
    ).toBeInTheDocument();
  });

  it("renders progress before snapshot while compressing", async () => {
    renderChatContent(
      makeChatState({ isCompressing: true, snapshotReceived: false }),
    );

    expect(
      await screen.findByTestId("compression-progress"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("chat-virtualized-list-wrapper"),
    ).toBeInTheDocument();
  });

  it("keeps normal loading before snapshot when not compressing", () => {
    renderChatContent(makeChatState({ snapshotReceived: false }));

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("chat-virtualized-list-wrapper"),
    ).not.toBeInTheDocument();
    expect(document.querySelector("canvas")).not.toBeInTheDocument();
  });

  it("keeps normal empty chat display when not compressing", () => {
    renderChatContent(makeChatState());

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("chat-virtualized-list-wrapper"),
    ).not.toBeInTheDocument();
    expect(document.querySelector("canvas")).toBeInTheDocument();
  });

  it("renders progress with existing streaming messages", async () => {
    renderChatContent(
      makeChatState({
        messages: [userMessage("hello")],
        isCompressing: true,
        isStreaming: true,
      }),
    );

    expect(
      await screen.findByTestId("compression-progress"),
    ).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("shows switching loading instead of old compressing chat progress", async () => {
    const oldRuntime = makeRuntime({
      messages: [userMessage("old chat content")],
      isCompressing: true,
    });
    oldRuntime.thread.id = "old-chat";
    const targetRuntime = makeRuntime({ snapshotReceived: false });
    targetRuntime.thread.id = "target-chat";
    const preloadedState: Partial<RootState> = {
      chat: {
        current_thread_id: "old-chat",
        open_thread_ids: ["old-chat", "target-chat"],
        threads: {
          "old-chat": oldRuntime,
          "target-chat": targetRuntime,
        },
        system_prompt: {},
        tool_use: "explore",
        sse_refresh_requested: null,
        stream_version: 0,
      },
    };
    const requestAnimationFrameSpy = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation(() => 1);
    const cancelAnimationFrameSpy = vi
      .spyOn(window, "cancelAnimationFrame")
      .mockImplementation(() => undefined);

    try {
      const { store } = render(
        <ChatContent
          onRetry={() => undefined}
          onStopStreaming={() => undefined}
        />,
        { preloadedState },
      );

      expect(
        await screen.findByTestId("compression-progress"),
      ).toBeInTheDocument();
      expect(screen.getByText("old chat content")).toBeInTheDocument();

      act(() => {
        store.dispatch(switchToThread({ id: "target-chat" }));
      });

      expect(
        screen.queryByTestId("compression-progress"),
      ).not.toBeInTheDocument();
      expect(screen.queryByText("old chat content")).not.toBeInTheDocument();
      expect(
        screen.queryByTestId("chat-virtualized-list-wrapper"),
      ).not.toBeInTheDocument();
      expect(document.querySelector("canvas")).not.toBeInTheDocument();
    } finally {
      requestAnimationFrameSpy.mockRestore();
      cancelAnimationFrameSpy.mockRestore();
    }
  });
});
