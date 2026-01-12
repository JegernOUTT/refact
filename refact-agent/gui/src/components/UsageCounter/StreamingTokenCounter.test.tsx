import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { StreamingTokenCounter } from "./StreamingTokenCounter";
import { chatReducer } from "../../features/Chat/Thread/reducer";
import { newChatAction } from "../../features/Chat/Thread/actions";
import { AssistantMessage, UserMessage } from "../../services/refact";

// Helper to create a minimal store
function createTestStore(overrides: {
  streaming?: boolean;
  waiting?: boolean;
  messages?: (UserMessage | AssistantMessage)[];
  maxTokens?: number;
}) {
  const emptyState = chatReducer(undefined, { type: "@@INIT" });
  const initialState = chatReducer(emptyState, newChatAction(undefined));
  const threadId = initialState.current_thread_id;
  const runtime = initialState.threads[threadId];

  if (!runtime) {
    throw new Error("Failed to create initial thread runtime");
  }

  return configureStore({
    reducer: {
      chat: chatReducer,
    },
    preloadedState: {
      chat: {
        ...initialState,
        threads: {
          [threadId]: {
            ...runtime,
            thread: {
              ...runtime.thread,
              messages: overrides.messages ?? [],
              currentMaximumContextTokens: overrides.maxTokens ?? 8000,
            },
            streaming: overrides.streaming ?? false,
            waiting_for_response: overrides.waiting ?? false,
            prevent_send: false,
            snapshot_received: true,
          },
        },
      },
    },
  });
}

describe("StreamingTokenCounter", () => {
  describe("Visibility", () => {
    it("should be hidden when not streaming or waiting", () => {
      const store = createTestStore({
        streaming: false,
        waiting: false,
      });

      const { container } = render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      expect(container.firstChild).toBeNull();
    });

    it("should show immediately when waiting (before first assistant message)", () => {
      const store = createTestStore({
        streaming: false,
        waiting: true,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show placeholder "…"
      expect(screen.getByText("…")).toBeInTheDocument();
    });

    it("should show immediately when streaming starts", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "H",
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show estimated token count
      expect(screen.getByText(/~\d+/)).toBeInTheDocument();
    });
  });

  describe("Token counting", () => {
    it("should show placeholder when no assistant message yet", () => {
      const store = createTestStore({
        streaming: false,
        waiting: true,
        messages: [
          {
            role: "user",
            content: "Test question",
          } as UserMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      expect(screen.getByText("…")).toBeInTheDocument();
    });

    it("should show estimated tokens during streaming", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "Hello world", // ~3 tokens (11 chars / 4)
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show "~3" (estimated)
      expect(screen.getByText(/~3/)).toBeInTheDocument();
    });

    it("should show actual tokens when usage data available", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "Hello world",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 10,
              total_tokens: 15,
            },
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show "5" (actual, no ~)
      expect(screen.getByText("5")).toBeInTheDocument();
      expect(screen.queryByText(/~/)).not.toBeInTheDocument();
    });
  });

  describe("Context percentage", () => {
    it("should show fallback context when waiting for new assistant", () => {
      const store = createTestStore({
        streaming: false,
        waiting: true,
        maxTokens: 8000,
        messages: [
          {
            role: "user",
            content: "First question",
          } as UserMessage,
          {
            role: "assistant",
            content: "First answer",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 1000,
              total_tokens: 1005,
            },
          } as AssistantMessage,
          {
            role: "user",
            content: "Second question",
          } as UserMessage,
          // No assistant yet - waiting for response
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show placeholder and fallback context percentage
      expect(screen.getByText("…")).toBeInTheDocument();
      // Context from previous message: 1000/8000 = 12.5% → 13%
      expect(screen.getByText(/~13%/)).toBeInTheDocument();
    });

    it("should show current context when assistant message exists", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        maxTokens: 8000,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "Hello world",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 2000,
              total_tokens: 2005,
            },
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show actual context: 2000/8000 = 25%
      expect(screen.getByText("(25%)")).toBeInTheDocument();
    });

    it("should show warning percentage at 70%", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        maxTokens: 8000,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "Response",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 5600, // 70%
              total_tokens: 5605,
            },
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Just verify the percentage is shown (CSS class is applied via CSS Modules)
      expect(screen.getByText("(70%)")).toBeInTheDocument();
    });

    it("should show critical percentage at 90%", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        maxTokens: 8000,
        messages: [
          {
            role: "user",
            content: "Hello",
          } as UserMessage,
          {
            role: "assistant",
            content: "Response",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 7200, // 90%
              total_tokens: 7205,
            },
          } as AssistantMessage,
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Just verify the percentage is shown (CSS class is applied via CSS Modules)
      expect(screen.getByText("(90%)")).toBeInTheDocument();
    });
  });

  describe("Turn detection", () => {
    it("should detect waiting for NEW assistant (user after assistant)", () => {
      const store = createTestStore({
        streaming: false,
        waiting: true,
        messages: [
          {
            role: "user",
            content: "First",
          } as UserMessage,
          {
            role: "assistant",
            content: "First response",
            usage: {
              completion_tokens: 5,
              prompt_tokens: 1000,
              total_tokens: 1005,
            },
          } as AssistantMessage,
          {
            role: "user",
            content: "Second",
          } as UserMessage,
          // Waiting for new assistant
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show placeholder (not continuing previous assistant)
      expect(screen.getByText("…")).toBeInTheDocument();
    });

    it("should use current assistant when continuing same turn", () => {
      const store = createTestStore({
        streaming: true,
        waiting: false,
        messages: [
          {
            role: "user",
            content: "Question",
          } as UserMessage,
          {
            role: "assistant",
            content: "Streaming response...",
          } as AssistantMessage,
          // Still streaming same assistant message
        ],
      });

      render(
        <Provider store={store}>
          <StreamingTokenCounter />
        </Provider>,
      );

      // Should show estimated tokens from current assistant
      expect(screen.getByText(/~\d+/)).toBeInTheDocument();
      expect(screen.queryByText("…")).not.toBeInTheDocument();
    });
  });
});
