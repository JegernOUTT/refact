import * as chatSubscription from "../services/refact/chatSubscription";
import { updateChatRuntimeFromSessionState } from "../features/Chat/Thread/actions";
import { describe, it, expect, vi, afterEach } from "vitest";
import { act as rtlAct, renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { backgroundAgentsApi } from "../services/refact/backgroundAgents";
import { configureStore } from "@reduxjs/toolkit";
import { useChatSubscription } from "../hooks/useChatSubscription";
import { chatReducer } from "../features/Chat/Thread/reducer";
import { createChatWithId } from "../features/Chat/Thread/actions";
import {
  agentsPanelSlice,
  agentDisclosureDenied,
  agentRecoveryRequested,
  selectAgentRosterSource,
} from "../features/AgentsPanel/agentsPanelSlice";
import type { RootState } from "../app/store";
import {
  reducer as configReducer,
  updateConfig,
} from "../features/Config/configSlice";

const syncAct = rtlAct as unknown as (callback: () => void) => void;
const asyncAct = rtlAct as unknown as (
  callback: () => Promise<void>,
) => Promise<void>;

const createTestStore = () => {
  return configureStore({
    reducer: {
      chat: chatReducer,
      config: configReducer,
      current_project: (state = { name: "" }) => state,
      agentsPanel: agentsPanelSlice.reducer,
      [backgroundAgentsApi.reducerPath]: backgroundAgentsApi.reducer,
    },
  });
};

const mockFetch = vi.fn();

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <Provider store={createTestStore()}>{children}</Provider>
);

describe("useChatSubscription", () => {
  afterEach(() => {
    mockFetch.mockReset();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("should return disconnected status when disabled", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
    expect(result.current.isConnected).toBe(false);
    expect(result.current.isConnecting).toBe(false);
  });

  it("should return disconnected status when chatId is null", () => {
    const { result } = renderHook(
      () => useChatSubscription(null, { enabled: true }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
  });

  it("should return disconnected status when chatId is undefined", () => {
    const { result } = renderHook(
      () => useChatSubscription(undefined, { enabled: true }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
  });

  it("should have lastSeq as string", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(typeof result.current.lastSeq).toBe("string");
    expect(result.current.lastSeq).toBe("0");
  });

  it("should have null error initially", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(result.current.error).toBeNull();
  });

  it("connects to a remote web lspUrl when lspPort is zero", async () => {
    global.fetch = mockFetch as unknown as typeof fetch;
    mockFetch.mockResolvedValue({
      ok: true,
      body: {
        getReader: () => ({
          read: vi.fn().mockResolvedValue({ done: true }),
        }),
      },
    });
    const store = createTestStore();
    store.dispatch(
      updateConfig({
        host: "web",
        lspPort: 0,
        lspUrl: "https://remote.example.com/v1/ping",
      }),
    );
    const remoteWrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    renderHook(() => useChatSubscription("remote-chat"), {
      wrapper: remoteWrapper,
    });

    await waitFor(() => {
      expect(mockFetch).toHaveBeenCalledWith(
        "https://remote.example.com/v1/chats/subscribe?chat_id=remote-chat&snapshot_chunks=1",
        expect.objectContaining({ method: "GET" }),
      );
    });
  });

  it("keeps chat reconnect and sequence recovery working after invalid roster data without redisclosure", async () => {
    vi.useFakeTimers();
    const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
    vi.stubGlobal("fetch", mockFetch);
    mockFetch.mockImplementation(() =>
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
    const store = createTestStore();
    store.dispatch(updateConfig({ host: "vscode", lspPort: 8123 }));
    store.dispatch(createChatWithId({ id: "test-chat" }));
    const onEvent = vi.fn();
    const { result, unmount } = renderHook(
      () => useChatSubscription("test-chat", { onEvent, reconnectDelay: 10 }),
      {
        wrapper: ({ children }) => (
          <Provider store={store}>{children}</Provider>
        ),
      },
    );
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
      chat_id: "test-chat",
      seq,
      queue_size: 0,
      queued_items: [],
    });
    await send(0, {
      type: "background_agent_updated",
      chat_id: "test-chat",
      seq: "1",
      agent: { agent_id: "invalid" },
    });
    expect(store.getState().agentsPanel.denied).toBe(true);
    await send(0, queue("2"));
    expect(store.getState().chat.threads["test-chat"]?.last_applied_seq).toBe(
      "2",
    );
    await asyncAct(async () => {
      streams[0].close();
      await Promise.resolve();
    });
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    const privateEvent = {
      type: "background_agent_updated",
      chat_id: "test-chat",
      seq: "1",
      agent: {
        agent_id: "withheld",
        parent_chat_id: "test-chat",
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
      store.getState().chat.threads["test-chat"]?.background_agents,
    ).toEqual({});
    expect(onEvent).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "background_agent_updated" }),
    );
    await send(1, queue("5"));
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockFetch).toHaveBeenCalledTimes(3);
    await send(2, {
      type: "snapshot",
      chat_id: "test-chat",
      seq: "0",
      thread: {
        id: "test-chat",
        title: "Recovered chat",
        model: "",
        mode: "agent",
        tool_use: "agent",
      },
      runtime: {
        state: "idle",
        paused: false,
        pause_reasons: [],
        queued_items: [],
      },
      messages: [],
      background_agents: [privateEvent.agent],
    });
    await send(2, queue("1"));
    expect(result.current.status).toBe("connected");
    expect(store.getState().chat.threads["test-chat"]?.last_applied_seq).toBe(
      "1",
    );
    expect(store.getState().agentsPanel.liveAgents).toEqual({});
    expect(
      store.getState().chat.threads["test-chat"]?.background_agents,
    ).toEqual({});
    expect(onEvent).toHaveBeenCalledWith(
      expect.objectContaining({ type: "snapshot", background_agents: [] }),
    );
    unmount();
  });

  it("connects with a denied roster but does not retry an actual chat authorization denial", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", mockFetch);
    let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
    mockFetch
      .mockResolvedValueOnce(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              stream = controller;
            },
          }),
        ),
      )
      .mockResolvedValue(new Response(null, { status: 403 }));
    const store = createTestStore();
    store.dispatch(updateConfig({ host: "vscode", lspPort: 8123 }));
    store.dispatch(
      agentDisclosureDenied({
        source: selectAgentRosterSource(store.getState() as RootState),
        error: { status: 401, data: null },
      }),
    );
    const { result, unmount } = renderHook(
      () => useChatSubscription("test-chat", { reconnectDelay: 10 }),
      {
        wrapper: ({ children }) => (
          <Provider store={store}>{children}</Provider>
        ),
      },
    );
    await asyncAct(async () => {
      await Promise.resolve();
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("connected");
    expect(store.getState().agentsPanel.denied).toBe(true);
    await asyncAct(async () => {
      if (!stream) throw new Error("Chat stream did not connect");
      stream.close();
      await Promise.resolve();
    });
    await asyncAct(async () => {
      await vi.advanceTimersByTimeAsync(100);
      await Promise.resolve();
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
    expect(result.current.error?.message).toBe("SSE connection failed: 403");
    expect(store.getState().agentsPanel.denied).toBe(true);
    unmount();
  });
  it("releases status on disconnect and unmount and rejects retired attempt callbacks", () => {
    const callbacks: Parameters<
      typeof chatSubscription.subscribeToChatEvents
    >[2][] = [];
    vi.spyOn(chatSubscription, "subscribeToChatEvents").mockImplementation(
      (_id, _config, handlers) => {
        callbacks.push(handlers);
        return () => undefined;
      },
    );
    const store = createTestStore();
    store.dispatch(updateConfig({ host: "vscode", lspPort: 8123 }));
    store.dispatch(createChatWithId({ id: "test-chat" }));
    const { result, unmount } = renderHook(
      () => useChatSubscription("test-chat", { autoReconnect: false }),
      {
        wrapper: ({ children }) => (
          <Provider store={store}>{children}</Provider>
        ),
      },
    );
    const snapshot: chatSubscription.ChatEventEnvelope = {
      type: "snapshot",
      chat_id: "test-chat",
      seq: "0",
      thread: {
        id: "test-chat",
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
    };
    const sidebar = () =>
      store.dispatch(
        updateChatRuntimeFromSessionState({
          id: "test-chat",
          source: "sidebar",
          session_state: "generating",
        }),
      );
    const runtime = () => store.getState().chat.threads["test-chat"];
    syncAct(() => {
      callbacks[0].onConnected?.();
      callbacks[0].onEvent(snapshot);
      sidebar();
    });
    expect(runtime()?.session_state).toBe("idle");
    syncAct(() => {
      callbacks[0].onDisconnected?.();
      sidebar();
    });
    expect(runtime()?.session_state).toBe("generating");
    expect(runtime()?.snapshot_received).toBe(true);
    syncAct(() => result.current.connect());
    syncAct(() => {
      callbacks[1].onConnected?.();
      callbacks[1].onEvent(snapshot);
    });
    syncAct(() => {
      callbacks[0].onEvent({
        ...snapshot,
        runtime: { ...snapshot.runtime, state: "executing_tools" },
      });
      callbacks[0].onError(new Error("retired"));
      callbacks[0].onDisconnected?.();
      sidebar();
    });
    expect(runtime()?.session_state).toBe("idle");
    expect(result.current.status).toBe("connected");
    syncAct(() => {
      result.current.disconnect();
      sidebar();
    });
    expect(runtime()?.session_state).toBe("generating");
    syncAct(() => result.current.connect());
    syncAct(() => {
      callbacks[2].onConnected?.();
      callbacks[2].onEvent(snapshot);
    });
    unmount();
    sidebar();
    expect(runtime()?.session_state).toBe("generating");
    callbacks[2].onEvent(snapshot);
    expect(runtime()?.session_state).toBe("generating");
  });
});
