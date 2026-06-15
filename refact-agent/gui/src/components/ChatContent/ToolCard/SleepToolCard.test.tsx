import { describe, expect, test, vi, afterEach, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { Theme } from "@radix-ui/themes";
import { act } from "react-dom/test-utils";

import { setUpStore } from "../../../app/store";
import { SleepToolCard } from "./SleepToolCard";
import type {
  ChatMessage,
  EventMessage,
  ToolCall,
  ToolMessage,
} from "../../../services/refact/types";
import { createDefaultChatState } from "../../../utils/test-utils";

const chatId = "sleep-chat";
type FetchMock = ReturnType<typeof vi.fn<typeof fetch>>;
type ActGlobal = typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };

function toolCall(args: Record<string, unknown> = {}): ToolCall {
  return {
    id: "call-sleep",
    index: 0,
    type: "function",
    function: {
      name: "sleep",
      arguments: JSON.stringify({ duration_ms: 30_000, ...args }),
    },
  };
}

function eventTick(
  id: string,
  elapsedMs: number,
  remainingMs: number,
): EventMessage {
  return {
    role: "event",
    message_id: id,
    content: `tick ${elapsedMs}`,
    subkind: "tick",
    source: "tool.sleep",
    payload: { elapsed_ms: elapsedMs, remaining_ms: remainingMs },
  };
}

function toolResult(result: {
  sleptMs: number;
  interrupted?: boolean;
}): ToolMessage {
  return {
    role: "tool",
    tool_call_id: "call-sleep",
    content: JSON.stringify({
      slept_ms: result.sleptMs,
      interrupted: result.interrupted === true,
    }),
    tool_failed: false,
    extra: {
      sleep: {
        slept_ms: result.sleptMs,
        interrupted: result.interrupted === true,
      },
    },
  };
}

function renderSleepCard(messages: ChatMessage[] = [], args = {}) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = chatId;
  runtime.thread.messages = messages;
  runtime.streaming = true;
  runtime.waiting_for_response = false;
  chat.current_thread_id = chatId;
  chat.open_thread_ids = [chatId];
  chat.threads = { [chatId]: runtime };

  const store = setUpStore({
    chat,
    config: {
      host: "web",
      lspPort: 4321,
      apiKey: null,
      features: { statistics: true, vecdb: true, ast: true, images: true },
      themeProps: { appearance: "dark" },
      shiftEnterToSubmit: false,
    },
  });

  return render(
    <Provider store={store}>
      <Theme>
        <SleepToolCard toolCall={toolCall(args)} />
      </Theme>
    </Provider>,
  );
}

function renderSleepCardForThread(
  messages: ChatMessage[] = [],
  args = {},
  activeChatId = "active-chat",
  activeMessages: ChatMessage[] = [],
) {
  const chat = createDefaultChatState();
  const activeRuntime = chat.threads[chat.current_thread_id];
  activeRuntime.thread.id = activeChatId;
  activeRuntime.thread.messages = activeMessages;
  const sleepRuntime = {
    ...activeRuntime,
    thread: {
      ...activeRuntime.thread,
      id: chatId,
      messages,
    },
  };
  chat.current_thread_id = activeChatId;
  chat.open_thread_ids = [activeChatId, chatId];
  chat.threads = {
    [activeChatId]: activeRuntime,
    [chatId]: sleepRuntime,
  };

  const store = setUpStore({
    chat,
    config: {
      host: "web",
      lspPort: 4321,
      apiKey: null,
      features: { statistics: true, vecdb: true, ast: true, images: true },
      themeProps: { appearance: "dark" },
      shiftEnterToSubmit: false,
    },
  });

  return render(
    <Provider store={store}>
      <Theme>
        <SleepToolCard toolCall={toolCall(args)} threadId={chatId} />
      </Theme>
    </Provider>,
  );
}

describe("SleepToolCard", () => {
  beforeEach(() => {
    (globalThis as ActGlobal).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2026-05-28T00:00:00.000Z"));
    window.localStorage.clear();
  });

  afterEach(() => {
    (globalThis as ActGlobal).IS_REACT_ACT_ENVIRONMENT = undefined;
    window.localStorage.clear();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  test("renders countdown for in-progress sleep", () => {
    renderSleepCard([], { description: "Let the gremlin nap" });

    expect(
      screen.getAllByText(/Sleeping… 30s remaining/u).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("Let the gremlin nap")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    expect(
      screen.getAllByText(/Sleeping… 29s remaining/u).length,
    ).toBeGreaterThan(0);
  });

  test("Wake-up button dispatches abort", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(new Response());
    vi.stubGlobal("fetch", fetchMock);

    renderSleepCard();

    await user.click(screen.getByRole("button", { name: /wake up/i }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/v1/chats/sleep-chat/commands",
      expect.objectContaining({ method: "POST" }),
    );
    const init = firstFetchInit(fetchMock);
    const body = JSON.parse(String(init.body)) as {
      type?: string;
    };
    expect(body.type).toBe("abort");
  });

  test("Wake-up button dispatches abort to rendered thread", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(new Response());
    vi.stubGlobal("fetch", fetchMock);

    renderSleepCardForThread();

    await user.click(screen.getByRole("button", { name: /wake up/i }));

    expect(fetchMock).toHaveBeenCalledWith(
      "/v1/chats/sleep-chat/commands",
      expect.objectContaining({ method: "POST" }),
    );
  });

  test("keeps fallback countdown after remount", () => {
    const { unmount } = renderSleepCard();

    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(
      screen.getAllByText(/Sleeping… 25s remaining/u).length,
    ).toBeGreaterThan(0);

    unmount();
    renderSleepCard();

    expect(
      screen.getAllByText(/Sleeping… 25s remaining/u).length,
    ).toBeGreaterThan(0);
  });

  test("uses ticks from rendered thread", () => {
    renderSleepCardForThread(
      [eventTick("rendered-tick", 10_000, 20_000)],
      {},
      "active-chat",
      [eventTick("active-tick", 25_000, 5_000)],
    );

    expect(
      screen.getAllByText(/Sleeping… 20s remaining/u).length,
    ).toBeGreaterThan(0);
    expect(screen.queryByText(/Sleeping… 5s remaining/u)).toBeNull();
  });

  test("tick events animate", () => {
    renderSleepCard([
      eventTick("tick-1", 5_000, 25_000),
      eventTick("tick-2", 10_000, 20_000),
      eventTick("tick-3", 15_000, 15_000),
    ]);

    const dots = screen.getAllByTestId("sleep-tick-dot");
    expect(dots).toHaveLength(3);
    expect(dots[0]?.className).toMatch(/tickDot/u);
    expect(
      screen.getAllByText(/Sleeping… 15s remaining/u).length,
    ).toBeGreaterThan(0);
  });

  test("completion shows summary", () => {
    renderSleepCard([
      eventTick("tick-1", 5_000, 25_000),
      eventTick("tick-2", 10_000, 20_000),
      eventTick("tick-3", 15_000, 15_000),
      eventTick("tick-4", 20_000, 10_000),
      eventTick("tick-5", 25_000, 5_000),
      toolResult({ sleptMs: 30_000 }),
    ]);

    expect(screen.getByText("Slept 30s · 5 ticks")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /wake up/i })).toBeNull();
  });

  test("abort shows interrupted summary", () => {
    renderSleepCard([
      eventTick("tick-1", 5_000, 25_000),
      eventTick("tick-2", 10_000, 20_000),
      toolResult({ sleptMs: 12_000, interrupted: true }),
    ]);

    expect(screen.getByText("Interrupted after 12s")).toBeInTheDocument();
  });
});

function firstFetchInit(fetchMock: FetchMock): RequestInit {
  expect(fetchMock).toHaveBeenCalled();
  const [, init] = fetchMock.mock.calls[0];
  if (!init) throw new Error("expected fetch init");
  return init;
}
