import { describe, expect, it } from "vitest";
import { ChatContent } from "./ChatContent";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import type { RootState } from "../../app/store";
import type { ChatMessages } from "../../services/refact";

function userMessage(content: string): ChatMessages[number] {
  return {
    role: "user",
    content,
    message_id: `user-${content}`,
  };
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
  if (!runtime) throw new Error("missing runtime");
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
});
