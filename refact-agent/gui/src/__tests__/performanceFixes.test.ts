import React from "react";
import { describe, it, expect, vi } from "vitest";
import { fireEvent } from "@testing-library/react";
import { applyDeltaOps, DeltaOp } from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";
import type { ToolCall } from "../services/refact/types";
import {
  selectManualPreviewItems,
  selectManualPreviewItemsById,
  selectManyToolResultsByIds,
  selectThreadIds,
  selectTabsDisplayData,
  selectToolResultById,
} from "../features/Chat/Thread/selectors";
import type { RootState } from "../app/store";
import type { Config } from "../features/Config/configSlice";
import { render, waitFor, createDefaultChatState } from "../utils/test-utils";
import { ChatContent } from "../components/ChatContent";
import { ToolCallTooltip } from "../components/ChatContent/ToolCard/ToolCallTooltip";
import { Chat } from "../components/Chat";
import { InnerApp } from "../features/App";
import { MARKDOWN_ISSUE } from "../__fixtures__";
import { applyChatEvent } from "../features/Chat/Thread";
import { chatReducer } from "../features/Chat/Thread/reducer";
import type {
  ChatMessages,
  DiffChunk,
  ToolConfirmationPauseReason,
} from "../services/refact";

function createTabLoadState(count: number): RootState {
  const base = createDefaultChatState();
  const template = base.threads[base.current_thread_id];
  const ids = Array.from({ length: count }, (_, index) => `tab-${index}`);
  const threads = Object.fromEntries(
    ids.map((id, index) => [
      id,
      {
        ...template,
        thread: {
          ...template.thread,
          id,
          title: `Chat ${index}`,
          messages: [],
        },
      },
    ]),
  );

  return {
    chat: {
      ...base,
      current_thread_id: ids[0] ?? "",
      open_thread_ids: ids,
      threads,
    },
    history: { chats: {} },
    notifications: { pendingByThread: {}, lastSeenByThread: {} },
  } as unknown as RootState;
}

const codeToHtmlSpy = vi.fn((code: string, options: { lang: string }) => {
  return `<pre><code class="language-${options.lang}"><span data-token="1">${code}</span></code></pre>`;
});

const createHighlighterSpy = vi.fn(() =>
  Promise.resolve({
    getLoadedLanguages: () => ["typescript", "plaintext", "text"],
    loadLanguage: () => Promise.resolve(),
    codeToHtml: codeToHtmlSpy,
  }),
);

function renderTooltip(toolCall: ToolCall) {
  const tooltip = ToolCallTooltip as React.FC<{
    toolCall: ToolCall;
    children?: React.ReactNode;
  }>;

  return React.createElement(
    tooltip,
    { toolCall },
    React.createElement("button", { type: "button" }, "show tool"),
  );
}

vi.mock("shiki", () => ({
  createHighlighter: createHighlighterSpy,
}));

function createTestSnapshot(chatId: string) {
  return {
    chat_id: chatId,
    seq: "1",
    type: "snapshot" as const,
    thread: {
      id: chatId,
      title: "Load test",
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
      state: "idle" as const,
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    background_agents: [],
    messages: [],
  };
}

function createThreadState(messages: ChatMessages) {
  const chat = createDefaultChatState();
  const id = MARKDOWN_ISSUE.id;
  const runtime = {
    ...chat.threads[chat.current_thread_id],
    thread: {
      ...MARKDOWN_ISSUE,
      id,
      messages,
    },
    snapshot_received: true,
  };

  const config: Config = {
    host: "web",
    lspPort: 8001,
    themeProps: { appearance: "dark" },
    features: {},
    apiKey: "test",
  };

  return {
    chat: {
      ...chat,
      current_thread_id: id,
      open_thread_ids: [id],
      threads: {
        [id]: runtime,
      },
    },
    config,
    pages: [{ name: "chat" as const }],
  };
}

describe("applyDeltaOps", () => {
  it("appends content correctly across multiple deltas", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const ops1: DeltaOp[] = [{ op: "append_content", text: "Hello" }];
    const ops2: DeltaOp[] = [{ op: "append_content", text: " World" }];

    const after1 = applyDeltaOps(initial, ops1);
    const after2 = applyDeltaOps(after1, ops2);

    expect(after1.content).toBe("Hello");
    expect(after2.content).toBe("Hello World");
  });

  it("appends reasoning correctly", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const ops: DeltaOp[] = [
      { op: "append_reasoning", text: "Thinking..." },
      { op: "append_reasoning", text: " More thoughts." },
    ];

    const result = applyDeltaOps(initial, ops);
    expect((result as { reasoning_content?: string }).reasoning_content).toBe(
      "Thinking... More thoughts.",
    );
  });

  it("add_citation does not mutate prior message objects", () => {
    const initial: ChatMessage = { role: "assistant", content: "test" };
    const citation1 = { url: "http://example.com/1", title: "Example 1" };
    const citation2 = { url: "http://example.com/2", title: "Example 2" };

    const after1 = applyDeltaOps(initial, [
      { op: "add_citation", citation: citation1 },
    ]);
    const after2 = applyDeltaOps(after1, [
      { op: "add_citation", citation: citation2 },
    ]);

    const initialCitations = (initial as { citations?: unknown[] }).citations;
    const after1Citations = (after1 as { citations?: unknown[] }).citations;
    const after2Citations = (after2 as { citations?: unknown[] }).citations;

    expect(initialCitations).toBeUndefined();
    expect(after1Citations).toHaveLength(1);
    expect(after2Citations).toHaveLength(2);
    expect(after1Citations).not.toBe(after2Citations);
  });

  it("handles set_tool_calls", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const toolCalls = [
      { id: "1", function: { name: "test", arguments: "{}" } },
    ];
    const ops: DeltaOp[] = [{ op: "set_tool_calls", tool_calls: toolCalls }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { tool_calls?: unknown[] }).tool_calls).toEqual(
      toolCalls,
    );
  });

  it("handles set_thinking_blocks", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const blocks = [{ thinking: "test thought" }];
    const ops: DeltaOp[] = [{ op: "set_thinking_blocks", blocks }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { thinking_blocks?: unknown[] }).thinking_blocks).toEqual(
      blocks,
    );
  });

  it("handles set_usage", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const usage = { prompt_tokens: 100, completion_tokens: 50 };
    const ops: DeltaOp[] = [{ op: "set_usage", usage }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { usage?: unknown }).usage).toEqual(usage);
  });

  it("handles merge_extra", () => {
    const initial: ChatMessage = {
      role: "assistant",
      content: "",
      extra: { a: 1 },
    } as ChatMessage & { extra: Record<string, unknown> };
    const ops: DeltaOp[] = [{ op: "merge_extra", extra: { b: 2 } }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { extra?: Record<string, unknown> }).extra).toEqual({
      a: 1,
      b: 2,
    });
  });
});

describe("selectToolResultById optimization", () => {
  it("keeps thread membership IDs stable across metadata updates and changes them on membership edits", () => {
    const state = createTabLoadState(3);
    const ids = selectThreadIds(state);
    const runtime = state.chat.threads[ids[0]];
    if (!runtime) throw new Error("Missing load-test runtime");
    const metadataOnly = {
      ...state,
      chat: {
        ...state.chat,
        threads: {
          ...state.chat.threads,
          [ids[0]]: {
            ...runtime,
            thread: { ...runtime.thread, title: "Only metadata changed" },
          },
        },
      },
    } as RootState;
    expect(selectThreadIds(metadataOnly)).toBe(ids);
    const added = {
      ...metadataOnly,
      chat: {
        ...metadataOnly.chat,
        threads: { ...metadataOnly.chat.threads, added: runtime },
      },
    } as RootState;
    const nextIds = selectThreadIds(added);
    expect(nextIds).not.toBe(ids);
    expect(nextIds).toEqual([...ids, "added"]);
  });

  it.each([10, 30, 60, 100])(
    "keeps tab array and unaffected entries stable for %i open chats",
    (count) => {
      const selectTabs = selectTabsDisplayData;
      const state = createTabLoadState(count);
      const first = selectTabs(state);
      const second = selectTabs(state);
      expect(second).toBe(first);

      const changedId = "tab-0";
      const changedRuntime = state.chat.threads[changedId];
      if (!changedRuntime) throw new Error("Missing load-test runtime");
      const nextState = {
        ...state,
        chat: {
          ...state.chat,
          threads: {
            ...state.chat.threads,
            [changedId]: {
              ...changedRuntime,
              thread: { ...changedRuntime.thread, title: "Renamed" },
            },
          },
        },
      } as RootState;
      const next = selectTabs(nextState);

      expect(next).not.toBe(first);
      expect(next[0]).not.toBe(first[0]);
      expect(next[0]?.title).toBe("Renamed");
      for (let index = 1; index < count; index++) {
        expect(next[index]).toBe(first[index]);
      }
      let streamingState = nextState;
      for (let chunk = 0; chunk < 200; chunk++) {
        const runtime = streamingState.chat.threads[changedId];
        if (!runtime) throw new Error("Missing load-test runtime");
        streamingState = {
          ...streamingState,
          chat: {
            ...streamingState.chat,
            threads: {
              ...streamingState.chat.threads,
              [changedId]: {
                ...runtime,
                thread: {
                  ...runtime.thread,
                  messages: [{ role: "assistant", content: String(chunk) }],
                },
              },
            },
          },
        };
        expect(selectTabs(streamingState)).toBe(next);
      }
    },
  );

  it("keeps tab order and history fallback while metadata, notification, and removal change", () => {
    const original = createTabLoadState(3);
    const ids = original.chat.open_thread_ids;
    const selected = selectTabsDisplayData(original);
    const withoutRuntime = {
      ...original,
      chat: {
        ...original.chat,
        threads: { ...original.chat.threads, [ids[1]]: undefined },
      },
      history: {
        chats: {
          [ids[1]]: {
            title: "Archived title",
            mode: "history-mode",
            session_state: "completed",
          },
        },
      },
    } as unknown as RootState;
    const fallback = selectTabsDisplayData(withoutRuntime);
    expect(fallback.map((tab) => tab.id)).toEqual(ids);
    expect(fallback[1]).toMatchObject({
      title: "Archived title",
      mode: "history-mode",
      session_state: "completed",
    });
    expect(fallback[0]).toBe(selected[0]);
    expect(fallback[2]).toBe(selected[2]);

    const pendingState = {
      ...withoutRuntime,
      notifications: {
        pendingByThread: { [ids[0]]: [{ receivedAt: 2 }, { receivedAt: 4 }] },
        lastSeenByThread: { [ids[0]]: 2 },
      },
    } as unknown as RootState;
    const withNotification = selectTabsDisplayData(pendingState);
    expect(withNotification[0]?.unreadNotificationCount).toBe(1);
    expect(withNotification[1]).toBe(fallback[1]);

    const removed = selectTabsDisplayData({
      ...pendingState,
      chat: { ...pendingState.chat, open_thread_ids: [ids[0], ids[2]] },
    } as RootState);
    expect(removed.map((tab) => tab.id)).toEqual([ids[0], ids[2]]);
    expect(removed[1]).toBe(fallback[2]);
  });

  it("applies ordered stream deltas without replacing unrelated tab rows", () => {
    const initial = createDefaultChatState();
    const chatId = initial.current_thread_id;
    let chat = chatReducer(initial, applyChatEvent(createTestSnapshot(chatId)));
    const selectTabs = selectTabsDisplayData;
    const first = selectTabs({
      chat,
      history: { chats: {} },
      notifications: { pendingByThread: {}, lastSeenByThread: {} },
    } as unknown as RootState);
    chat = chatReducer(
      chat,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "load-stream",
      }),
    );
    const afterStarted = selectTabs({
      chat,
      history: { chats: {} },
      notifications: { pendingByThread: {}, lastSeenByThread: {} },
    } as unknown as RootState);
    expect(afterStarted[0]?.activity).toBe("working");
    chat = chatReducer(
      chat,
      applyChatEvent({
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "load-stream",
        ops: [{ op: "append_content", text: "delta" }],
      }),
    );
    const afterDelta = selectTabs({
      chat,
      history: { chats: {} },
      notifications: { pendingByThread: {}, lastSeenByThread: {} },
    } as unknown as RootState);
    expect(afterDelta[0]).toBe(afterStarted[0]);
    expect(afterDelta).not.toBe(first);
    const messages = chat.threads[chatId]?.thread.messages ?? [];
    expect(messages[messages.length - 1]?.content).toBe("delta");
  });

  it("updates each tab-visible thread field only on its ordered metadata event", () => {
    const initial = createTabLoadState(2);
    const id = initial.chat.open_thread_ids[0];
    const otherId = initial.chat.open_thread_ids[1];
    const baseline = selectTabsDisplayData(initial);
    const runtime = initial.chat.threads[id];
    if (!runtime) throw new Error("Missing load-test runtime");
    const changed = {
      ...runtime,
      thread: {
        ...runtime.thread,
        title: "Updated title",
        mode: "updated-mode",
        is_task_chat: true,
        buddy_meta: { is_buddy_chat: true },
      },
      error: "failed",
      confirmation: { ...runtime.confirmation, pause: true },
    };
    const stateAfter = (rt: typeof changed): RootState =>
      ({
        ...initial,
        chat: {
          ...initial.chat,
          threads: { ...initial.chat.threads, [id]: rt },
        },
      }) as RootState;

    let state = initial;
    const prior = selectTabsDisplayData(state);
    state = stateAfter(changed);
    const next = selectTabsDisplayData(state);
    expect(next[0]).not.toBe(prior[0]);
    expect(next[1]).toBe(baseline[1]);
    expect(next[0]?.id).toBe(id);
    const final = selectTabsDisplayData(state);
    expect(final[0]).toMatchObject({
      title: "Updated title",
      mode: "updated-mode",
      is_task_chat: true,
      is_buddy_chat: true,
      session_state: "error",
      activity: "error",
    });
    expect(
      selectTabsDisplayData({
        ...state,
        chat: {
          ...state.chat,
          threads: { ...state.chat.threads, [otherId]: undefined },
        },
      } as RootState)[0],
    ).toBe(final[0]);
    const approval = selectTabsDisplayData({
      ...initial,
      chat: {
        ...initial.chat,
        threads: {
          ...initial.chat.threads,
          [id]: {
            ...runtime,
            confirmation: {
              ...runtime.confirmation,
              pause: true,
              pause_reasons: [
                { type: "confirmation", command: "shell", rule: "ask" },
              ],
            },
          },
        },
      },
    } as RootState);
    expect(approval[0]?.activity).toBe("needs_approval");
    expect(approval[1]).toEqual(final[1]);
  });

  it("treats tools with both result and diff as resolved once per id", async () => {
    const messages: ChatMessages = [
      {
        role: "assistant",
        message_id: "msg-both",
        content: "I'll edit the file.",
        tool_calls: [
          {
            id: "tool-both",
            type: "function",
            index: 0,
            function: {
              name: "update_textdoc",
              arguments:
                '{"path":"src/both.ts","old_str":"old","replacement":"new"}',
            },
          },
        ],
      },
      {
        role: "tool",
        tool_call_id: "tool-both",
        content: "done",
      },
      {
        role: "diff",
        tool_call_id: "tool-both",
        content: [
          {
            file_name: "src/both.ts",
            file_action: "edit",
            line1: 1,
            line2: 1,
            lines_remove: "old\n",
            lines_add: "new\n",
          },
        ],
      },
    ];
    const state = createThreadState(messages);
    state.chat.threads[MARKDOWN_ISSUE.id].streaming = true;
    state.chat.threads[MARKDOWN_ISSUE.id].waiting_for_response = true;

    const { container } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: state,
      },
    );

    await waitFor(() => {
      expect(container.textContent).toContain("both.ts");
      expect(container.textContent).toContain("+1");
      expect(container.textContent).toContain("−1");
    });
  });

  it("uses stable empty manual preview fallbacks", () => {
    const state = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {},
        },
      },
    } as unknown as RootState;

    const first = selectManualPreviewItems(state);
    const second = selectManualPreviewItems(state);
    const byIdFirst = selectManualPreviewItemsById(state, "test");
    const byIdSecond = selectManualPreviewItemsById(state, "missing");

    expect(first).toBe(second);
    expect(byIdFirst).toBe(first);
    expect(byIdSecond).toBe(first);
  });

  it("finds tool result from end without array copy", () => {
    const mockState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "tool", tool_call_id: "id1", content: "first" },
                { role: "tool", tool_call_id: "id2", content: "second" },
                { role: "tool", tool_call_id: "id1", content: "third" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;

    const result = selectToolResultById(mockState, "id1");
    expect(result?.content).toBe("third");
  });

  it("returns undefined for missing id", () => {
    const mockState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "tool", tool_call_id: "id1", content: "first" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;

    const result = selectToolResultById(mockState, "nonexistent");
    expect(result).toBeUndefined();
  });

  it("keeps many-result selector output stable across unrelated streaming updates", () => {
    const selectResults = selectManyToolResultsByIds(["id1"]);
    const toolMessage = { role: "tool", tool_call_id: "id1", content: "first" };
    const baseState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "assistant", content: "before" },
                toolMessage,
                { role: "assistant", message_id: "stream", content: "tick 1" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;
    const nextState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "assistant", content: "before" },
                toolMessage,
                { role: "assistant", message_id: "stream", content: "tick 2" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;

    const first = selectResults(baseState);
    const second = selectResults(nextState);

    expect(second).toBe(first);
    expect(second[0]).toBe(toolMessage);
  });
});

describe("ToolCallTooltip", () => {
  it("truncates oversized arguments and caps entries when visible", async () => {
    vi.useFakeTimers();
    const largeValue = "x".repeat(700);
    const args = Object.fromEntries(
      Array.from({ length: 14 }, (_, index) => [`arg${index}`, largeValue]),
    );
    const toolCall: ToolCall = {
      id: "tool-tooltip",
      type: "function",
      index: 0,
      function: {
        name: "update_textdoc",
        arguments: JSON.stringify(args),
      },
    };

    const { getByText, unmount } = render(renderTooltip(toolCall));

    try {
      fireEvent.mouseEnter(getByText("show tool"));
      vi.advanceTimersByTime(10_000);
      vi.useRealTimers();

      await waitFor(() => {
        expect(getByText("Update Textdoc")).toBeTruthy();
        expect(getByText(/arg0/)).toBeTruthy();
        expect(document.body.textContent).toContain("truncated (700 chars)");
        expect(getByText("more arguments hidden")).toBeTruthy();
      });
      expect(document.body.textContent).not.toContain("arg12");
    } finally {
      unmount();
      vi.useRealTimers();
    }
  });
});

describe("chat rendering regressions", () => {
  it("streaming markdown still renders immediately while deferring Shiki", async () => {
    const streamingState = createThreadState([
      {
        role: "assistant",
        message_id: "msg-stream",
        content: "## Streaming title\n\n```ts\nconst value = 1\n```\n\n- item",
      },
    ]);
    streamingState.chat.threads[MARKDOWN_ISSUE.id].streaming = true;

    const { container, unmount } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: streamingState,
      },
    );

    await waitFor(() => {
      expect(container.querySelector("h2")?.textContent).toBe(
        "Streaming title",
      );
    });

    await new Promise((resolve) => setTimeout(resolve, 450));

    expect(container.textContent).toContain("const value = 1");
    expect(container.querySelector("pre code span[style]")).toBeNull();

    unmount();

    const settled = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: createThreadState([
          {
            role: "assistant",
            message_id: "msg-stream",
            content:
              "## Streaming title\n\n```ts\nconst value = 1\n```\n\n- item",
          },
        ]),
      },
    );

    await waitFor(() => {
      expect(
        settled.container.querySelector("pre code span[style]"),
      ).not.toBeNull();
    });
  });

  it("incremental tail update renders appended tool context and diffs", async () => {
    const baseMessages: ChatMessages = [
      {
        role: "user",
        content: "show me the plan",
      },
      {
        role: "assistant",
        message_id: "msg-plan",
        content: "Here is the plan.",
        tool_calls: [
          {
            id: "tool-1",
            type: "function",
            index: 0,
            function: {
              name: "cat",
              arguments: '{"paths":"README.md"}',
            },
          },
        ],
      },
    ];

    const diffs: DiffChunk[] = [
      {
        file_name: "README.md",
        file_action: "edit",
        line1: 1,
        line2: 1,
        lines_remove: "old line\n",
        lines_add: "new line\n",
      },
    ];

    const { store, container } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: createThreadState(baseMessages),
      },
    );

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "1",
        type: "message_added",
        index: 2,
        message: {
          role: "context_file",
          tool_call_id: "tool-1",
          content: [
            {
              file_name: "README.md",
              file_content: "# Demo\n\n```ts\nconsole.log('hello')\n```",
              line1: 1,
              line2: 4,
            },
          ],
        },
      }),
    );

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "2",
        type: "message_added",
        index: 3,
        message: {
          role: "diff",
          tool_call_id: "tool-1",
          content: diffs,
        },
      }),
    );

    await waitFor(() => {
      expect(container.textContent).toContain("Here is the plan.");
      expect(container.textContent).toContain("README.md");
      expect(container.textContent).toContain("+1 −1");
    });
  });

  it("keeps streaming edit tool diffs inert until the tool finishes", async () => {
    const largeDiff: DiffChunk = {
      file_name: "src/generated.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "old line\n".repeat(2000),
      lines_add: "new line\n".repeat(2000),
    };
    const messages: ChatMessages = [
      {
        role: "assistant",
        message_id: "msg-edit",
        content: "I'll edit the file.",
        tool_calls: [
          {
            id: "tool-edit",
            type: "function",
            index: 0,
            function: {
              name: "update_textdoc",
              arguments:
                '{"path":"src/generated.ts","old_str":"old","replacement":"new"}',
            },
          },
        ],
      },
    ];
    const streamingState = createThreadState(messages);
    streamingState.chat.threads[MARKDOWN_ISSUE.id].streaming = false;
    streamingState.chat.threads[MARKDOWN_ISSUE.id].waiting_for_response = true;

    const { container, store } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: streamingState,
      },
    );

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "1",
        type: "message_added",
        index: 1,
        message: {
          role: "diff",
          tool_call_id: "tool-edit",
          content: [largeDiff],
        },
      }),
    );

    await waitFor(() => {
      expect(container.textContent).toContain("generated.ts");
      expect(container.textContent).toContain("+2000");
      expect(container.textContent).toContain("−2000");
    });

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "2",
        type: "runtime_updated",
        state: "waiting_user_input",
      }),
    );

    await waitFor(() => {
      expect(container.textContent).toContain("+2000");
      expect(container.textContent).toContain("−2000");
    });
  });

  it("keeps completed diff-only tools stable while a later assistant streams", async () => {
    const completedDiff: DiffChunk = {
      file_name: "src/completed.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "old line\n".repeat(3),
      lines_add: "new line\n".repeat(3),
    };
    const activeDiff: DiffChunk = {
      file_name: "src/active.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "active old\n".repeat(2000),
      lines_add: "active new\n".repeat(2000),
    };
    const messages: ChatMessages = [
      {
        role: "assistant",
        message_id: "msg-tools",
        content: "I'll edit one file.",
        tool_calls: [
          {
            id: "tool-completed",
            type: "function",
            index: 0,
            function: {
              name: "update_textdoc",
              arguments:
                '{"path":"src/completed.ts","old_str":"old","replacement":"new"}',
            },
          },
        ],
      },
      {
        role: "diff",
        tool_call_id: "tool-completed",
        content: [completedDiff],
      },
      {
        role: "assistant",
        message_id: "msg-active",
        content: "Now I'll edit another file.",
        tool_calls: [
          {
            id: "tool-active",
            type: "function",
            index: 0,
            function: {
              name: "update_textdoc",
              arguments:
                '{"path":"src/active.ts","old_str":"old","replacement":"new"}',
            },
          },
        ],
      },
      {
        role: "diff",
        tool_call_id: "tool-active",
        content: [activeDiff],
      },
    ];
    const streamingState = createThreadState(messages);
    streamingState.chat.threads[MARKDOWN_ISSUE.id].streaming = true;
    streamingState.chat.threads[MARKDOWN_ISSUE.id].waiting_for_response = true;

    const { container } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: streamingState,
      },
    );

    await waitFor(() => {
      expect(container.textContent).toContain("completed.ts");
      expect(container.textContent).toContain("+3");
      expect(container.textContent).toContain("−3");
      expect(container.textContent).not.toContain("+2000");
      expect(container.textContent).not.toContain("−2000");
      expect(container.textContent).not.toContain("active new");
    });

    const activeRow = container.querySelector(
      '[data-testid="edit-tool"][data-status="running"]',
    );
    expect(activeRow?.textContent).toContain("active.ts");
  });

  it("keeps appended context files grouped with the preceding read tool", async () => {
    const baseMessages: ChatMessages = [
      {
        role: "assistant",
        message_id: "msg-read",
        content: "I'll inspect the file.",
        tool_calls: [
          {
            id: "tool-read",
            type: "function",
            index: 0,
            function: {
              name: "cat",
              arguments: '{"paths":"README.md"}',
            },
          },
        ],
      },
    ];

    const { store, container } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: createThreadState(baseMessages),
      },
    );

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "1",
        type: "message_added",
        index: 1,
        message: {
          role: "context_file",
          tool_call_id: "tool-read",
          content: [
            {
              file_name: "README.md",
              file_content: "hello",
              line1: 1,
              line2: 1,
            },
          ],
        },
      }),
    );

    await waitFor(() => {
      expect(
        container.querySelector('[data-testid="read-tool"]')?.textContent,
      ).toContain("README.md");
    });

    expect(container.textContent).not.toContain("Memories (1)");
    expect(container.textContent).not.toContain("Project context (1)");
  });

  it("rebuilds grouped tool output when assistant tool calls change without changing message count", async () => {
    const messages: ChatMessages = [
      {
        role: "assistant",
        message_id: "msg-change",
        content: "I'll inspect the file.",
      },
      {
        role: "context_file",
        tool_call_id: "tool-read",
        content: [
          {
            file_name: "README.md",
            file_content: "hello",
            line1: 1,
            line2: 1,
          },
        ],
      },
    ];

    const { store, container } = render(
      React.createElement(ChatContent, {
        onRetry: () => undefined,
        onStopStreaming: () => undefined,
      }),
      {
        preloadedState: createThreadState(messages),
      },
    );

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "1",
        type: "message_updated",
        message_id: "msg-change",
        message: {
          role: "assistant",
          message_id: "msg-change",
          content: "I'll inspect the file.",
          tool_calls: [
            {
              id: "tool-read",
              type: "function",
              index: 0,
              function: {
                name: "cat",
                arguments: '{"paths":"README.md"}',
              },
            },
          ],
        },
      }),
    );

    await waitFor(() => {
      expect(
        container.querySelector('[data-testid="read-tool"]')?.textContent,
      ).toContain("README.md");
    });

    expect(container.textContent).not.toContain("Memories (1)");
    expect(container.textContent).not.toContain("Project context (1)");
  });

  it("chat form renders tool confirmation immediately alongside a large chat", async () => {
    const base = createThreadState(MARKDOWN_ISSUE.messages);
    const pauseReasons: ToolConfirmationPauseReason[] = [
      {
        type: "confirmation",
        tool_name: "apply_patch",
        command: "apply_patch",
        rule: "ask_user",
        tool_call_id: "tool-1",
        integr_config_path: null,
      },
    ];

    const { container } = render(
      React.createElement(Chat, {
        host: "web",
        tabbed: false,
        backFromChat: () => undefined,
        unCalledTools: true,
        maybeSendToSidebar: () => undefined,
      }),
      {
        preloadedState: {
          ...base,
          chat: {
            ...base.chat,
            threads: {
              [MARKDOWN_ISSUE.id]: {
                ...base.chat.threads[MARKDOWN_ISSUE.id],
                confirmation: {
                  pause: true,
                  pause_reasons: pauseReasons,
                  status: {
                    wasInteracted: false,
                    confirmationStatus: true,
                  },
                },
              },
            },
          },
        },
      },
    );

    await waitFor(() => {
      expect(container.textContent).toContain("Allow Once");
      expect(container.textContent).toContain("Stop");
    });
  });

  it("dispatches a resize after the IDE root recovers from zero height", async () => {
    const base = createThreadState(MARKDOWN_ISSUE.messages);
    const resizeSpy = vi.spyOn(window, "dispatchEvent");
    const originalResizeObserver = globalThis.ResizeObserver;
    let resizeCallback: ResizeObserverCallback | undefined;

    vi.stubGlobal(
      "ResizeObserver",
      vi.fn((cb: ResizeObserverCallback) => {
        resizeCallback = cb;
        return {
          observe: vi.fn(),
          unobserve: vi.fn(),
          disconnect: vi.fn(),
        };
      }),
    );

    let rootHeight = 0;
    const clientHeightSpy = vi
      .spyOn(HTMLElement.prototype, "clientHeight", "get")
      .mockImplementation(function mockClientHeight(this: HTMLElement) {
        if (this.getAttribute("data-element") === "app-root") {
          return rootHeight;
        }
        return 400;
      });

    const rectSpy = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function mockRect(this: HTMLElement) {
        if (this.getAttribute("data-element") === "app-root") {
          return {
            width: 400,
            height: rootHeight,
            top: 0,
            left: 0,
            right: 400,
            bottom: rootHeight,
            x: 0,
            y: 0,
            toJSON: () => undefined,
          } as DOMRect;
        }

        return {
          width: 400,
          height: 400,
          top: 0,
          left: 0,
          right: 400,
          bottom: 400,
          x: 0,
          y: 0,
          toJSON: () => undefined,
        } as DOMRect;
      });

    render(React.createElement(InnerApp), {
      preloadedState: {
        ...base,
        config: {
          ...base.config,
          host: "jetbrains",
        },
      },
    });

    resizeCallback?.([] as ResizeObserverEntry[], {} as ResizeObserver);
    rootHeight = 400;
    resizeCallback?.([] as ResizeObserverEntry[], {} as ResizeObserver);

    await waitFor(() => {
      expect(
        resizeSpy.mock.calls.some(
          ([event]) => event instanceof Event && event.type === "resize",
        ),
      ).toBe(true);
    });

    clientHeightSpy.mockRestore();
    rectSpy.mockRestore();
    vi.stubGlobal("ResizeObserver", originalResizeObserver);
  });
});
