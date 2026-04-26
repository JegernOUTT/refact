import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { useEnsureSubscriptionConnected } from "../hooks/useEnsureSubscriptionConnected";
import { chatReducer } from "../features/Chat/Thread/reducer";
import { reducer as configReducer } from "../features/Config/configSlice";
import type { Chat, ChatThreadRuntime } from "../features/Chat/Thread/types";
import React from "react";

const createThreadRuntime = (
  overrides: Partial<ChatThreadRuntime> & {
    thread: ChatThreadRuntime["thread"];
  },
): ChatThreadRuntime => ({
  streaming: false,
  waiting_for_response: false,
  snapshot_received: false,
  prevent_send: false,
  error: null,
  queued_items: [],
  send_immediately: false,
  attached_images: [],
  attached_text_files: [],
  confirmation: {
    pause: false,
    pause_reasons: [],
    status: { wasInteracted: false, confirmationStatus: true },
  },
  task_widget_expanded: false,
  memory_enrichment_user_touched: false,
  manual_preview_items: [],
  manual_preview_ran: false,
  ...overrides,
});

const chatInitialState: Chat = {
  current_thread_id: "",
  open_thread_ids: [],
  threads: {},
  system_prompt: {},
  tool_use: "agent",
  sse_refresh_requested: null,
  stream_version: 0,
};

const createTestStore = (chatOverrides?: Partial<Chat>) =>
  configureStore({
    reducer: {
      chat: chatReducer,
      config: configReducer,
    },
    preloadedState: chatOverrides
      ? { chat: { ...chatInitialState, ...chatOverrides } }
      : undefined,
  });

describe("useEnsureSubscriptionConnected", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns isConnected true if snapshot already received", () => {
    const store = createTestStore({
      current_thread_id: "test-chat-id",
      open_thread_ids: ["test-chat-id"],
      threads: {
        "test-chat-id": createThreadRuntime({
          thread: {
            id: "test-chat-id",
            messages: [],
            model: "",
            title: undefined,
            new_chat_suggested: { wasSuggested: false },
          },
          snapshot_received: true,
        }),
      },
    });

    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    const { result } = renderHook(
      () => useEnsureSubscriptionConnected("test-chat-id"),
      { wrapper },
    );

    expect(result.current.isConnected).toBe(true);
    expect(result.current.isConnecting).toBe(false);
  });

  it("provides ensureConnected function", () => {
    const store = createTestStore({
      current_thread_id: "test-chat-id",
      open_thread_ids: ["test-chat-id"],
      threads: {
        "test-chat-id": createThreadRuntime({
          thread: {
            id: "test-chat-id",
            messages: [{ role: "user", content: "hello" }],
            model: "",
            title: undefined,
            new_chat_suggested: { wasSuggested: false },
          },
          snapshot_received: false,
        }),
      },
    });

    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    const { result } = renderHook(
      () => useEnsureSubscriptionConnected("test-chat-id"),
      { wrapper },
    );

    expect(typeof result.current.ensureConnected).toBe("function");
  });

  it("returns isConnected false when no snapshot received", () => {
    const store = createTestStore({
      current_thread_id: "test-chat-id",
      open_thread_ids: ["test-chat-id"],
      threads: {
        "test-chat-id": createThreadRuntime({
          thread: {
            id: "test-chat-id",
            messages: [],
            model: "",
            title: undefined,
            new_chat_suggested: { wasSuggested: false },
          },
          snapshot_received: false,
        }),
      },
    });

    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    const { result } = renderHook(
      () => useEnsureSubscriptionConnected("test-chat-id"),
      { wrapper },
    );

    expect(result.current.isConnected).toBe(false);
    expect(result.current.isConnecting).toBe(true);
  });

  it("returns isConnected true when chatId is null", () => {
    const store = createTestStore();

    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    const { result } = renderHook(() => useEnsureSubscriptionConnected(null), {
      wrapper,
    });

    expect(result.current.isConnected).toBe(true);
  });
});
