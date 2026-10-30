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
import { applyChatEvent } from "../features/Chat/Thread/actions";
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

  it("clears a pending retry timer when a chat leaves the visible desired set", async () => {
    vi.useFakeTimers();
    const mockFetch = vi.fn().mockRejectedValue(new Error("network drop"));
    vi.stubGlobal("fetch", mockFetch);
    const store = createSubscriptionStore();

    syncAct(() => {
      store.dispatch(registerVisibleChatMount({ chatId: "chat-a" }));
    });
    const { unmount } = renderAllChatsSubscription(store);

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
    const send = async (index: number, event: object) => {
      await asyncAct(async () => {
        streams[index].enqueue(
          new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
        );
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
    expect(store.getState().chat.threads["chat-a"].last_applied_seq).toBe("2");
    await asyncAct(async () => {
      streams[0].close();
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
    expect(store.getState().chat.threads["chat-a"].background_agents).toEqual(
      {},
    );
    await send(1, queue("5"));
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(mockFetch).toHaveBeenCalledTimes(3);
    await send(2, { ...snapshot, background_agents: [privateEvent.agent] });
    await send(2, queue("1"));
    expect(store.getState().chat.threads["chat-a"].last_applied_seq).toBe("1");
    expect(store.getState().agentsPanel.liveAgents).toEqual({});
    expect(store.getState().chat.threads["chat-a"].background_agents).toEqual(
      {},
    );
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
      await Promise.resolve();
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(store.getState().agentsPanel.denied).toBe(true);
    expect(store.getState().connection.sseConnections["chat-a"]?.status).toBe(
      "connected",
    );
    await asyncAct(async () => {
      if (!stream) throw new Error("Chat stream did not connect");
      stream.close();
    });
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    expect(store.getState().agentsPanel.denied).toBe(true);
    unmount();
  });
});
