import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatContent } from "./ChatContent";
import { act } from "react-dom/test-utils";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import type { RootState } from "../../app/store";
import type { ChatMessages } from "../../services/refact";
import { switchToThread } from "../../features/Chat/Thread/actions";
import type {
  ChatThreadRuntime,
  QueuedItem,
} from "../../features/Chat/Thread/types";

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
  queuedItems = [],
  compressionPhase,
  compressionPulseSeq,
}: {
  messages?: ChatMessages;
  isCompressing?: boolean;
  snapshotReceived?: boolean;
  isStreaming?: boolean;
  queuedItems?: QueuedItem[];
  compressionPhase?: ChatThreadRuntime["compression_phase"];
  compressionPulseSeq?: string;
} = {}): ChatThreadRuntime {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const runtime = chat.threads[chatId];
  runtime.thread.messages = messages;
  runtime.is_compressing = isCompressing;
  runtime.snapshot_received = snapshotReceived;
  runtime.streaming = isStreaming;
  runtime.queued_items = queuedItems;
  runtime.compression_phase = compressionPhase;
  runtime.compression_pulse_seq = compressionPulseSeq;
  return runtime;
}

function makeChatState({
  messages = [],
  isCompressing = false,
  snapshotReceived = true,
  isStreaming = false,
  queuedItems = [],
  compressionPhase,
  compressionPulseSeq,
  sseStatus,
}: {
  messages?: ChatMessages;
  isCompressing?: boolean;
  snapshotReceived?: boolean;
  isStreaming?: boolean;
  queuedItems?: QueuedItem[];
  compressionPhase?: ChatThreadRuntime["compression_phase"];
  compressionPulseSeq?: string;
  sseStatus?: "disconnected" | "connecting" | "connected";
} = {}): Partial<RootState> {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const runtime = chat.threads[chatId];
  runtime.thread.messages = messages;
  runtime.is_compressing = isCompressing;
  runtime.snapshot_received = snapshotReceived;
  runtime.streaming = isStreaming;
  runtime.queued_items = queuedItems;
  runtime.compression_phase = compressionPhase;
  runtime.compression_pulse_seq = compressionPulseSeq;
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
  return render(
    <ChatContent onRetry={() => undefined} onStopStreaming={() => undefined} />,
    {
      preloadedState,
    },
  );
}

describe("ChatContent compression progress", () => {
  beforeEach(() => {
    vi.useRealTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders footer status for an empty compressing thread", async () => {
    renderChatContent(makeChatState({ isCompressing: true }));

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Compressing context…",
    );
    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();
    expect(
      screen.getByTestId("chat-virtualized-list-wrapper"),
    ).toBeInTheDocument();
    expect(
      screen
        .getByTestId("compression-progress")
        .closest("[data-testid='chat-virtuoso-item']"),
    ).toBeNull();
    expect(document.querySelector("canvas")).not.toBeInTheDocument();
  });

  it("renders footer status before snapshot while compressing", async () => {
    renderChatContent(
      makeChatState({ isCompressing: true, snapshotReceived: false }),
    );

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Compressing context…",
    );
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

  it("renders footer status with existing streaming messages", async () => {
    renderChatContent(
      makeChatState({
        messages: [userMessage("hello")],
        isCompressing: true,
        isStreaming: true,
      }),
    );

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Compressing context…",
    );
    expect(screen.getByText("hello")).toBeInTheDocument();
    expect(
      screen
        .getByTestId("compression-progress")
        .closest("[data-testid='chat-virtuoso-item']"),
    ).toBeNull();
  });

  it("does not add a transient compression item to virtualized rows", () => {
    renderChatContent(
      makeChatState({
        messages: [userMessage("hello")],
        isCompressing: true,
      }),
    );

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
    const rows = screen.getAllByTestId("chat-virtuoso-item");
    expect(rows).toHaveLength(1);
    expect(
      screen
        .getByTestId("compression-progress")
        .closest("[data-testid='chat-virtuoso-item']"),
    ).toBeNull();
  });

  it("renders queued messages inside the chat-width overlay", () => {
    const preview = "queued message ".repeat(20).trim();
    renderChatContent(
      makeChatState({
        messages: [userMessage("hello")],
        queuedItems: [
          {
            client_request_id: "queued-1",
            priority: false,
            command_type: "user_message",
            preview,
            content: preview,
          },
        ],
      }),
    );

    const queuedText = screen.getByText(preview);
    const queuedCard = queuedText.closest("[class*='queuedMessage']");
    const queuedContent = queuedText.closest(
      "[class*='queuedMessagesContent']",
    );
    const queuedContainer = queuedText.closest(
      "[class*='queuedMessagesContainer']",
    );

    expect(queuedCard?.className).toContain("queuedMessage");
    expect(queuedContent?.className).toContain("queuedMessagesContent");
    expect(queuedContainer?.className).toContain("queuedMessagesContainer");
  });

  it("shows preloaded compression pulse on first mount", () => {
    vi.useFakeTimers({ now: 0 });
    renderChatContent(
      makeChatState({
        compressionPhase: "applied",
        compressionPulseSeq: "preloaded-pulse",
      }),
    );

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(499);
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
  });

  it("shows progress when fast compression start and applied events are batched", () => {
    vi.useFakeTimers({ now: 0 });
    const { store } = renderChatContent(
      makeChatState({ messages: [userMessage("hello")] }),
    );
    const chatId = store.getState().chat.current_thread_id;

    act(() => {
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: chatId,
          type: "runtime_updated",
          seq: "1",
          state: "generating",
          is_compressing: true,
          compression_phase: "checking",
        },
      });
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: chatId,
          type: "runtime_updated",
          seq: "2",
          state: "idle",
          is_compressing: false,
          compression_phase: "applied",
        },
      });
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(499);
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
  });

  it("shows progress when a fast applied snapshot follows batched compression events", () => {
    vi.useFakeTimers({ now: 0 });
    const { store } = renderChatContent(
      makeChatState({ messages: [userMessage("hello")] }),
    );
    const chatId = store.getState().chat.current_thread_id;

    act(() => {
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: chatId,
          type: "runtime_updated",
          seq: "1",
          state: "generating",
          is_compressing: true,
          compression_phase: "checking",
        },
      });
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: chatId,
          type: "runtime_updated",
          seq: "2",
          state: "idle",
          is_compressing: false,
          compression_phase: "applied",
        },
      });
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: chatId,
          type: "snapshot",
          seq: "3",
          thread: {
            id: chatId,
            title: "Test",
            model: "gpt-4",
            mode: "AGENT",
            tool_use: "agent",
            boost_reasoning: false,
            context_tokens_cap: null,
            include_project_info: true,
            checkpoints_enabled: true,
            is_title_generated: false,
          },
          runtime: {
            state: "idle",
            paused: false,
            error: null,
            queue_size: 0,
            pause_reasons: [],
            queued_items: [],
            is_compressing: false,
            compression_phase: "applied",
          },
          background_agents: [],
          messages: [userMessage("hello")],
        },
      });
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(499);
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
  });

  it("keeps fast compression visible for the minimum duration", () => {
    vi.useFakeTimers({ now: 0 });
    const { store } = renderChatContent(makeChatState({ isCompressing: true }));

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          chat_id: store.getState().chat.current_thread_id,
          type: "runtime_updated",
          seq: "1",
          state: "idle",
          is_compressing: false,
        },
      });
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(499);
    });

    expect(screen.getByTestId("compression-progress")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(
      screen.queryByTestId("compression-progress"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("chat-virtualized-list-wrapper"),
    ).not.toBeInTheDocument();
  });

  it("does not replay an old chat compression pulse when switching back", () => {
    const oldRuntime = makeRuntime({
      compressionPhase: "applied",
      compressionPulseSeq: "old-pulse",
    });
    oldRuntime.thread.id = "old-chat";
    const targetRuntime = makeRuntime({
      messages: [userMessage("target chat content")],
    });
    targetRuntime.thread.id = "target-chat";
    const preloadedState: Partial<RootState> = {
      chat: {
        current_thread_id: "target-chat",
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
    let finishSwitch: FrameRequestCallback | undefined;
    const requestAnimationFrameSpy = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        finishSwitch = callback;
        return 1;
      });
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

      expect(screen.getByText("target chat content")).toBeInTheDocument();
      expect(
        screen.queryByTestId("compression-progress"),
      ).not.toBeInTheDocument();

      act(() => {
        store.dispatch(switchToThread({ id: "old-chat" }));
      });

      expect(finishSwitch).toBeDefined();

      act(() => {
        finishSwitch?.(0);
      });

      expect(
        screen.queryByTestId("compression-progress"),
      ).not.toBeInTheDocument();
    } finally {
      requestAnimationFrameSpy.mockRestore();
      cancelAnimationFrameSpy.mockRestore();
    }
  });

  it("shows switching loading instead of old compressing chat progress", () => {
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

      expect(screen.getByTestId("compression-progress")).toBeInTheDocument();
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
