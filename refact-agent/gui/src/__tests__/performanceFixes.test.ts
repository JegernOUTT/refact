import React from "react";
import { describe, it, expect, vi } from "vitest";
import { applyDeltaOps, DeltaOp } from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";
import { selectToolResultById } from "../features/Chat/Thread/selectors";
import type { RootState } from "../app/store";
import type { Config } from "../features/Config/configSlice";
import { render, waitFor, createDefaultChatState } from "../utils/test-utils";
import { ChatContent } from "../components/ChatContent";
import { Chat } from "../components/Chat";
import { InnerApp } from "../features/App";
import { MARKDOWN_ISSUE } from "../__fixtures__";
import { applyChatEvent } from "../features/Chat/Thread";
import type {
  ChatMessages,
  DiffChunk,
  ToolConfirmationPauseReason,
} from "../services/refact";

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

vi.mock("shiki", () => ({
  createHighlighter: createHighlighterSpy,
}));

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
      expect(container.textContent).toContain("+2 -2");
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
      {
        role: "diff",
        tool_call_id: "tool-edit",
        content: [largeDiff],
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

    await waitFor(() => {
      expect(container.textContent).toContain("Edit generated.ts");
      expect(container.textContent).not.toContain("+2000");
      expect(container.textContent).not.toContain("−2000");
    });

    store.dispatch(
      applyChatEvent({
        chat_id: MARKDOWN_ISSUE.id,
        seq: "1",
        type: "runtime_updated",
        state: "waiting_user_input",
      }),
    );

    await waitFor(() => {
      expect(container.textContent).toContain("+2000");
      expect(container.textContent).toContain("−2000");
    });
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
      expect(container.textContent).toContain("Read README.md");
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
      expect(container.textContent).toContain("Read README.md");
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
