import * as chatSubscription from "../services/refact/chatSubscription";
import {
  selectRuntimeById,
  selectSubscriptionFailureById,
} from "../features/Chat/Thread/selectors";
import { createElement, type ReactElement, type ReactNode } from "react";
import { Provider } from "react-redux";
import { renderHook, act as rtlAct } from "@testing-library/react";
import { afterEach, describe, it, expect, vi } from "vitest";
import {
  pickDesiredChatSubscriptions,
  useAllChatsSubscription,
} from "../hooks/useAllChatsSubscription";
import {
  connectionSlice,
  registerVisibleChatMount,
  unregisterVisibleChatMount,
  selectVisibleChatMountIds,
} from "../features/Connection";
import { setUpStore, type AppStore, type RootState } from "../app/store";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import {
  newChatAction,
  createChatWithId,
  updateChatRuntimeFromSessionState,
  applyChatEvent,
  requestSseRefresh,
} from "../features/Chat/Thread/actions";
import {
  agentDisclosureDenied,
  agentRecoveryRequested,
  selectAgentRosterSource,
} from "../features/AgentsPanel/agentsPanelSlice";

const ReduxProvider = Provider as unknown as (props: {
  store: AppStore;
  children?: ReactNode;
}) => ReactElement | null;

const syncAct = rtlAct as unknown as (callback: () => void) => void;
const asyncAct = rtlAct as unknown as (
  callback: () => Promise<void>,
) => Promise<void>;

function renderAllChatsSubscription(store: AppStore) {
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(ReduxProvider, { store }, children);

  return renderHook(() => useAllChatsSubscription(), { wrapper });
}

function createSubscriptionStore() {
  return setUpStore({
    config: {
      apiKey: "test",
      host: "vscode",
      lspPort: 8001,
      themeProps: {},
    },
  });
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  // tinyspy restores an accessor it found deeper than one prototype level as a self-referential own getter.
  Reflect.deleteProperty(document, "visibilityState");
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("pickDesiredChatSubscriptions", () => {
  it("subscribes only chats visible on screen", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("deduplicates visible chats while preserving order", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-2", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("does not keep chat subscriptions while the page is hidden", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-1", "chat-2"],
      documentVisible: false,
    });

    expect(result).toEqual([]);
  });

  it("derives desired subscriptions from registered visible chat mounts", () => {
    let connection = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "chat-a" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      unregisterVisibleChatMount({ chatId: "chat-a" }),
    );

    const visibleThreadIds = selectVisibleChatMountIds({
      connection,
    } as unknown as RootState);
    const result = pickDesiredChatSubscriptions({ visibleThreadIds });

    expect(result).toEqual(["chat-b"]);
  });

  it("retries a failed group in one fetch despite varied per-chat jitter and cancels hidden retries", async () => {
    vi.useFakeTimers();
    const fetcher = vi.fn().mockRejectedValue(new Error("group failed"));
    vi.stubGlobal("fetch", fetcher);
    const store = createSubscriptionStore();
    const ids = ["chat-a", "chat-b", "chat-c"];
    ids.forEach((chatId) =>
      store.dispatch(registerVisibleChatMount({ chatId })),
    );
    const { unmount } = renderAllChatsSubscription(store);
    const visibility = Object.getOwnPropertyDescriptor(
      document,
      "visibilityState",
    );
    try {
      let jitter = 0;
      vi.spyOn(Math, "random").mockImplementation(
        () => [0, 0.5, 0.999][jitter++ % 3],
      );
      await asyncAct(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(fetcher).toHaveBeenCalledTimes(1);
      await asyncAct(async () => {
        await vi.advanceTimersByTimeAsync(2500);
      });
      expect(fetcher).toHaveBeenCalledTimes(2);
      for (const [url] of fetcher.mock.calls as [string][]) {
        const parsed = new URL(url, "http://localhost");
        expect(parsed.pathname).toBe("/v1/chats/subscribe-many");
        expect(
          JSON.parse(parsed.searchParams.get("chat_ids") ?? "null"),
        ).toEqual(ids);
      }
      expect(vi.getTimerCount()).toBe(1);
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value: "hidden",
      });
      syncAct(() => document.dispatchEvent(new Event("visibilitychange")));
      expect(vi.getTimerCount()).toBe(0);
      await asyncAct(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      expect(fetcher).toHaveBeenCalledTimes(2);
    } finally {
      unmount();
      if (visibility)
        Object.defineProperty(document, "visibilityState", visibility);
      else Reflect.deleteProperty(document, "visibilityState");
    }
    expect(vi.getTimerCount()).toBe(0);
  });

  it("clears a pending retry timer when a chat leaves the visible desired set", async () => {
    vi.useFakeTimers();
    const mockFetch = vi.fn().mockRejectedValue(new Error("network drop"));
    vi.stubGlobal("fetch", mockFetch);
    const store = createSubscriptionStore();

    syncAct(() => {
      store.dispatch(registerVisibleChatMount({ chatId: "chat-a" }));
    });
    const { unmount } = renderAllChatsSubscription(store);
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);

    await asyncAct(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(vi.getTimerCount()).toBe(1);

    syncAct(() => {
      store.dispatch(unregisterVisibleChatMount({ chatId: "chat-a" }));
    });

    expect(vi.getTimerCount()).toBe(0);

    syncAct(() => {
      vi.advanceTimersByTime(60_000);
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);
    unmount();
  });

  it("continues reconnecting and recovering sequence gaps after invalid roster data without redisclosure", async () => {
    vi.useFakeTimers();
    const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
    const mockFetch = vi.fn().mockImplementation(() =>
      Promise.resolve(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              streams.push(controller);
            },
          }),
        ),
      ),
    );
    vi.stubGlobal("fetch", mockFetch);
    const store = createSubscriptionStore();
    const snapshot = {
      type: "snapshot" as const,
      chat_id: "chat-a",
      seq: "0",
      thread: {
        id: "chat-a",
        title: "Recovered chat",
        model: "",
        mode: "agent",
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
      messages: [],
      background_agents: [],
    };
    store.dispatch(applyChatEvent(snapshot));
    store.dispatch(registerVisibleChatMount({ chatId: "chat-a" }));
    const { unmount } = renderAllChatsSubscription(store);
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const send = async (index: number, event: object) => {
      await asyncAct(async () => {
        streams[index].enqueue(
          new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
        );
        await Promise.resolve();
      });
    };
    const queue = (seq: string) => ({
      type: "queue_updated",
      chat_id: "chat-a",
      seq,
      queue_size: 0,
      queued_items: [],
    });
    await send(0, {
      type: "background_agent_updated",
      chat_id: "chat-a",
      seq: "1",
      agent: { agent_id: "invalid" },
    });
    expect(store.getState().agentsPanel.denied).toBe(true);
    await send(0, queue("2"));
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.last_applied_seq,
    ).toBe("2");
    await asyncAct(async () => {
      streams[0].close();
      await Promise.resolve();
    });
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    const privateEvent = {
      type: "background_agent_updated",
      chat_id: "chat-a",
      seq: "1",
      agent: {
        agent_id: "withheld",
        parent_chat_id: "chat-a",
        kind: "subagent",
        status: "running",
      },
    };
    await send(1, privateEvent);
    await send(1, queue("2"));
    syncAct(() => {
      store.dispatch(agentRecoveryRequested());
    });
    await send(1, { ...privateEvent, seq: "3" });
    expect(store.getState().agentsPanel.liveAgents).toEqual({});
    expect(store.getState().agentsPanel.authorized).toBe(false);
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.background_agents,
    ).toEqual({});
    await send(1, queue("5"));
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(101);
    });
    expect(mockFetch).toHaveBeenCalledTimes(3);
    await send(2, { ...snapshot, background_agents: [privateEvent.agent] });
    await send(2, queue("1"));
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.last_applied_seq,
    ).toBe("1");
    expect(store.getState().agentsPanel.liveAgents).toEqual({});
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.background_agents,
    ).toEqual({});
    unmount();
  });

  it("connects with a denied roster but stops retrying an actual chat authorization denial", async () => {
    vi.useFakeTimers();
    let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
    const mockFetch = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              stream = controller;
            },
          }),
        ),
      )
      .mockResolvedValue(new Response(null, { status: 401 }));
    vi.stubGlobal("fetch", mockFetch);
    const store = createSubscriptionStore();
    store.dispatch(
      agentDisclosureDenied({
        source: selectAgentRosterSource(store.getState()),
        error: { status: 403, data: null },
      }),
    );
    store.dispatch(registerVisibleChatMount({ chatId: "chat-a" }));
    const { unmount } = renderAllChatsSubscription(store);
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(store.getState().agentsPanel.denied).toBe(true);
    expect(store.getState().connection.sseConnections["chat-a"]?.status).toBe(
      "connecting",
    );
    await asyncAct(async () => {
      if (!stream) throw new Error("Chat stream did not connect");
      stream.enqueue(
        new TextEncoder().encode(
          `data: ${JSON.stringify({
            type: "snapshot",
            chat_id: "chat-a",
            seq: "0",
            thread: {
              id: "chat-a",
              title: "Live",
              model: "",
              mode: "agent",
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
            },
            messages: [],
            background_agents: [],
          })}\n\n`,
        ),
      );
      await Promise.resolve();
    });
    expect(store.getState().connection.sseConnections["chat-a"]?.status).toBe(
      "connected",
    );
    await asyncAct(async () => {
      if (!stream) throw new Error("Chat stream did not connect");
      stream.close();
      await Promise.resolve();
    });
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    expect(store.getState().agentsPanel.denied).toBe(true);
    unmount();
  });
});

describe("per-chat isolation on the shared stream", () => {
  const thread = (id: string) => ({
    id,
    title: id,
    model: "",
    mode: "agent",
    tool_use: "agent",
    boost_reasoning: false,
    context_tokens_cap: null,
    include_project_info: true,
    checkpoints_enabled: true,
    is_title_generated: false,
  });
  const snapshot = (chatId: string, seq = "0") => ({
    type: "snapshot",
    chat_id: chatId,
    seq,
    thread: thread(chatId),
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
  });
  const setup = (ids: string[]) => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
    const signals: AbortSignal[] = [];
    const fetcher = vi.fn((_url: string, init: RequestInit) => {
      if (init.signal) signals.push(init.signal);
      return Promise.resolve(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              streams.push(controller);
            },
          }),
        ),
      );
    });
    vi.stubGlobal("fetch", fetcher);
    const store = createSubscriptionStore();
    ids.forEach((chatId) =>
      store.dispatch(registerVisibleChatMount({ chatId })),
    );
    const view = renderAllChatsSubscription(store);
    const send = async (index: number, event: object) => {
      await asyncAct(async () => {
        streams[index].enqueue(
          new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
        );
        await vi.advanceTimersByTimeAsync(0);
      });
    };
    const advance = async (ms: number) => {
      await asyncAct(async () => {
        await vi.advanceTimersByTimeAsync(ms);
      });
    };
    const status = (chatId: string) =>
      store.getState().connection.sseConnections[chatId]?.status;
    return { fetcher, signals, streams, store, send, advance, status, view };
  };

  it("backs off a repeatedly failing chat on its own count while a sibling stays healthy", async () => {
    const { fetcher, signals, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await send(0, snapshot("chat-a"));
    let seq = 1;
    const keepAlive = () =>
      send(0, {
        type: "queue_updated",
        chat_id: "chat-a",
        seq: String(seq++),
        queue_size: 0,
        queued_items: [],
      });
    for (const delay of [2000, 4000, 8000, 16000, 30_000, 30_000]) {
      await advance(15_000);
      expect(status("chat-b")).toBe("disconnected");
      await keepAlive();
      await advance(delay - 1);
      expect(status("chat-b")).toBe("disconnected");
      await advance(1);
      expect(status("chat-b")).toBe("connecting");
      expect(status("chat-a")).toBe("connected");
      await keepAlive();
    }
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    await send(0, snapshot("chat-b", "3"));
    expect(status("chat-b")).toBe("connected");
    view.unmount();
  });

  it("does not retry a chat closed with retryable false, including after EOF", async () => {
    const { fetcher, streams, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, snapshot("chat-b"));
    await send(0, {
      chat_id: "chat-b",
      type: "subscription_closed",
      reason: "unavailable",
      retryable: false,
      message: "Chat no longer exists",
    });
    expect(status("chat-b")).toBe("disconnected");
    expect(status("chat-a")).toBe("connected");
    await advance(30_000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await send(0, {
      chat_id: "chat-a",
      type: "subscription_closed",
      reason: "ended",
      retryable: false,
    });
    await asyncAct(async () => {
      streams[0].close();
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(status("chat-a")).toBe("disconnected");
    expect(status("chat-b")).toBe("disconnected");
    view.unmount();
  });

  it("resubscribes a retryable closed chat with a new server stream", async () => {
    const { fetcher, signals, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, snapshot("chat-b"));
    await send(0, {
      chat_id: "chat-b",
      type: "subscription_closed",
      reason: "ended",
      retryable: true,
      message: "stream ended",
    });
    expect(status("chat-b")).toBe("disconnected");
    expect(signals[0].aborted).toBe(false);
    await advance(1999);
    expect(status("chat-b")).toBe("disconnected");
    await advance(1);
    expect(status("chat-b")).toBe("connecting");
    await advance(79);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    await advance(1);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
    await send(1, snapshot("chat-a", "1"));
    await send(1, snapshot("chat-b", "1"));
    expect(status("chat-b")).toBe("connected");
    view.unmount();
  });

  const queued = (chatId: string, seq: string) => ({
    type: "queue_updated",
    chat_id: chatId,
    seq,
    queue_size: 0,
    queued_items: [],
  });

  it("keeps a snapshot that arrives during the retry backoff after a bootstrap timeout", async () => {
    const { fetcher, signals, store, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    await advance(15_000);
    expect(status("chat-b")).toBe("disconnected");
    await send(0, snapshot("chat-b", "4"));
    await send(0, queued("chat-b", "5"));
    await send(0, queued("chat-a", "1"));
    await advance(2000);
    await send(0, queued("chat-b", "6"));
    await advance(1000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    expect(status("chat-b")).toBe("connected");
    expect(status("chat-a")).toBe("connected");
    expect(
      selectRuntimeById(store.getState(), "chat-b")?.last_applied_seq,
    ).toBe("6");
    await send(0, queued("chat-a", "2"));
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.last_applied_seq,
    ).toBe("2");
    view.unmount();
  });

  it("keeps a first snapshot that arrives inside the refresh gap", async () => {
    const { fetcher, signals, store, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    syncAct(() => {
      store.dispatch(requestSseRefresh({ chatId: "chat-b" }));
    });
    await send(0, snapshot("chat-b", "2"));
    await send(0, queued("chat-b", "3"));
    await advance(1000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    expect(status("chat-b")).toBe("connected");
    expect(
      selectRuntimeById(store.getState(), "chat-b")?.last_applied_seq,
    ).toBe("3");
    await send(0, queued("chat-a", "1"));
    expect(
      selectRuntimeById(store.getState(), "chat-a")?.last_applied_seq,
    ).toBe("1");
    view.unmount();
  });

  it("keeps a later group retry on time when an earlier one is cleared", async () => {
    const { fetcher, store, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
      "chat-c",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, snapshot("chat-b"));
    await send(0, snapshot("chat-c"));
    const close = (chatId: string) =>
      send(0, {
        chat_id: chatId,
        type: "subscription_closed",
        reason: "ended",
        retryable: true,
      });
    await close("chat-b");
    await advance(500);
    await close("chat-a");
    syncAct(() => {
      store.dispatch(requestSseRefresh({ chatId: "chat-b" }));
    });
    await advance(50);
    expect(status("chat-b")).toBe("connecting");
    await advance(80);
    expect(fetcher).toHaveBeenCalledTimes(2);
    await send(1, snapshot("chat-b", "1"));
    await send(1, snapshot("chat-c", "1"));
    await advance(1999 - 130);
    expect(status("chat-a")).toBe("disconnected");
    await advance(1);
    expect(status("chat-a")).toBe("connecting");
    await advance(80);
    expect(fetcher).toHaveBeenCalledTimes(3);
    const url = new URL(fetcher.mock.calls[2][0], "http://localhost");
    expect(
      [
        ...(JSON.parse(url.searchParams.get("chat_ids") ?? "[]") as string[]),
      ].sort(),
    ).toEqual(["chat-a", "chat-b", "chat-c"]);
    view.unmount();
  });

  it("records a final close before any snapshot and resumes on retry", async () => {
    const { fetcher, store, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    syncAct(() => {
      store.dispatch(
        createChatWithId({ id: "chat-b", openTab: false, activate: false }),
      );
    });
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, {
      chat_id: "chat-b",
      type: "subscription_closed",
      reason: "unavailable",
      retryable: false,
      message: "Chat no longer exists",
    });
    expect(status("chat-b")).toBe("disconnected");
    expect(selectSubscriptionFailureById(store.getState(), "chat-b")).toBe(
      "Chat no longer exists",
    );
    await advance(30_000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    syncAct(() => {
      store.dispatch(requestSseRefresh({ chatId: "chat-b" }));
    });
    expect(selectSubscriptionFailureById(store.getState(), "chat-b")).toBe(
      null,
    );
    await advance(130);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(status("chat-b")).toBe("connecting");
    await send(1, snapshot("chat-a", "1"));
    await send(1, snapshot("chat-b", "1"));
    expect(status("chat-b")).toBe("connected");
    expect(selectSubscriptionFailureById(store.getState(), "chat-b")).toBe(
      null,
    );
    view.unmount();
  });

  it("does not record a failure for a retryable close or a chat that already has a snapshot", async () => {
    const { store, send, advance, view } = setup(["chat-a", "chat-b"]);
    syncAct(() => {
      store.dispatch(
        createChatWithId({ id: "chat-a", openTab: false, activate: false }),
      );
      store.dispatch(
        createChatWithId({ id: "chat-b", openTab: false, activate: false }),
      );
    });
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, {
      chat_id: "chat-b",
      type: "subscription_closed",
      reason: "ended",
      retryable: true,
      message: "temporary",
    });
    await send(0, {
      chat_id: "chat-a",
      type: "subscription_closed",
      reason: "unavailable",
      retryable: false,
      message: "gone",
    });
    expect(selectSubscriptionFailureById(store.getState(), "chat-b")).toBe(
      null,
    );
    expect(selectSubscriptionFailureById(store.getState(), "chat-a")).toBe(
      null,
    );
    view.unmount();
  });

  it("keeps the shared agent roster healthy when one chat fails on its own", async () => {
    const { store, streams, send, advance, status, view } = setup([
      "chat-a",
      "chat-b",
    ]);
    await advance(0);
    await send(0, snapshot("chat-a"));
    await send(0, {
      chat_id: "chat-b",
      type: "subscription_closed",
      reason: "unavailable",
      retryable: true,
      message: "chat subscription unavailable: 403 Forbidden",
    });
    expect(status("chat-b")).toBe("disconnected");
    expect(status("chat-a")).toBe("connected");
    expect(store.getState().agentsPanel.transportStale).toBe(false);
    expect(store.getState().agentsPanel.denied).toBe(false);
    await asyncAct(async () => {
      streams[0].error(new Error("network reset"));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(status("chat-a")).toBe("disconnected");
    expect(store.getState().agentsPanel.transportStale).toBe(true);
    view.unmount();
  });
});

it("hands hidden status to sidebar and ignores callbacks from retired attempts", () => {
  const callbacks: Parameters<
    typeof chatSubscription.subscribeToChatEvents
  >[2][] = [];
  vi.spyOn(chatSubscription, "createChatEventMultiplexer").mockReturnValue({
    subscribe: (_id, handlers) => {
      callbacks.push(handlers);
      return () => undefined;
    },
  });
  const store = createSubscriptionStore();
  store.dispatch(newChatAction(undefined));
  const chatId = store.getState().chat.current_thread_id;
  const sidebar = () =>
    store.dispatch(
      updateChatRuntimeFromSessionState({
        id: chatId,
        session_state: "generating",
        source: "sidebar",
      }),
    );
  store.dispatch(registerVisibleChatMount({ chatId }));
  const { unmount } = renderAllChatsSubscription(store);
  const snapshot: ChatEventEnvelope = {
    type: "snapshot",
    chat_id: chatId,
    seq: "0",
    thread: {
      id: chatId,
      title: "Live",
      model: "",
      mode: "agent",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: true,
      is_title_generated: false,
    },
    runtime: {
      state: "executing_tools",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages: [],
    background_agents: [],
  };
  syncAct(() => {
    callbacks[0].onEvent(snapshot);
    sidebar();
  });
  expect(selectRuntimeById(store.getState(), chatId)?.session_state).toBe(
    "executing_tools",
  );
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  syncAct(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  syncAct(sidebar);
  expect(selectRuntimeById(store.getState(), chatId)?.session_state).toBe(
    "generating",
  );
  expect(selectRuntimeById(store.getState(), chatId)?.snapshot_received).toBe(
    true,
  );
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  syncAct(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(callbacks).toHaveLength(2);
  syncAct(() => {
    callbacks[1].onEvent({
      ...snapshot,
      runtime: { ...snapshot.runtime, state: "idle" },
    });
    callbacks[0].onEvent(snapshot);
    callbacks[0].onDisconnected?.();
    callbacks[0].onError(new Error("retired"));
    sidebar();
  });
  expect(selectRuntimeById(store.getState(), chatId)?.session_state).toBe(
    "idle",
  );
  expect(selectRuntimeById(store.getState(), chatId)?.last_applied_seq).toBe(
    "0",
  );
  expect(store.getState().connection.sseConnections[chatId]?.status).toBe(
    "connected",
  );
  syncAct(() => store.dispatch(unregisterVisibleChatMount({ chatId })));
  syncAct(sidebar);
  expect(selectRuntimeById(store.getState(), chatId)?.session_state).toBe(
    "generating",
  );
  unmount();
});
