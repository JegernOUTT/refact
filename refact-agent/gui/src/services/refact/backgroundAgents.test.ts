import { configureStore, type Middleware } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { EngineApiConfig } from "./apiUrl";
import { backgroundAgentsApi } from "./backgroundAgents";
import { subscribeToChatEvents } from "./chatSubscription";
import type { RootState } from "../../app/store";
import {
  agentsPanelSlice,
  agentDisclosureDenied,
  agentRecoveryRequested,
  agentRosterAuthorized,
  agentSourceChanged,
  selectAgentRosterSource,
} from "../../features/AgentsPanel/agentsPanelSlice";
import { errorSlice } from "../../features/Errors/errorsSlice";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type TestConfigState = EngineApiConfig & { apiKey: string | null };

function createTestStore(config: TestConfigState, actions: string[] = []) {
  const recordActions: Middleware = () => (next) => (action) => {
    const serialized = JSON.stringify(action);
    if (serialized) actions.push(serialized);
    return next(action);
  };
  return configureStore({
    reducer: {
      config: (state: TestConfigState = config) => state,
      current_project: (state = { name: "" }) => state,
      agentsPanel: agentsPanelSlice.reducer,
      error: errorSlice.reducer,
      [backgroundAgentsApi.reducerPath]: backgroundAgentsApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(
        backgroundAgentsApi.middleware,
        recordActions,
      ),
  });
}

function rosterQuery(
  store: ReturnType<typeof createTestStore>,
  chatId: string,
) {
  return {
    chatId,
    source: selectAgentRosterSource(store.getState() as RootState),
  };
}

function jsonResponse(data: unknown): Response {
  return new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
}

function requestAt(
  fetchMock: ReturnType<typeof vi.fn<FetchLike>>,
  index: number,
): Request {
  const [input, init] = fetchMock.mock.calls[index];
  return input instanceof Request ? input : new Request(input, init);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("backgroundAgentsApi", () => {
  test("reports malformed live roster schema without discarding ordinary chat events", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValue(
      new Response(
        new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(
              new TextEncoder().encode(
                [
                  'data: {"type":"background_agent_updated","chat_id":"chat-1","seq":"1","agent":{"agent_id":"invalid"}}',
                  "",
                  'data: {"type":"queue_updated","chat_id":"chat-1","seq":"2","queue_size":0,"queued_items":[]}',
                  "",
                  "",
                ].join("\n"),
              ),
            );
            controller.close();
          },
        }),
      ),
    );
    const onEvent = vi.fn();
    const onError = vi.fn();
    const onAgentRosterDenied = vi.fn();
    const stop = subscribeToChatEvents(
      "chat-1",
      { host: "ide", lspPort: 8123 },
      {
        onEvent,
        onError,
        onAgentRosterDenied,
      },
    );
    try {
      await vi.waitFor(() =>
        expect(onEvent).toHaveBeenCalledWith(
          expect.objectContaining({ type: "queue_updated" }),
        ),
      );
      expect(onAgentRosterDenied).toHaveBeenCalledTimes(1);
      expect(onEvent).toHaveBeenCalledTimes(1);
      expect(onError).not.toHaveBeenCalled();
    } finally {
      stop();
    }
  });

  test("uses the specified introspection URLs and request bodies", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock
      .mockResolvedValueOnce(jsonResponse([]))
      .mockResolvedValueOnce(jsonResponse({ success: true }))
      .mockResolvedValueOnce(jsonResponse({ success: true }));
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: "token",
    });

    const list = store.dispatch(
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
        rosterQuery(store, "chat one"),
      ),
    );
    await list;
    list.unsubscribe();
    const listRequest = requestAt(fetchMock, 0);
    expect(listRequest.method).toBe("GET");
    expect(new URL(listRequest.url).pathname).toBe("/v1/background-agents");
    expect(new URL(listRequest.url).searchParams.get("chat_id")).toBe(
      "chat one",
    );

    const cancel = store.dispatch(
      backgroundAgentsApi.endpoints.cancelBackgroundAgent.initiate({
        agentId: "agent/1",
        chatId: "chat one",
        source: rosterQuery(store, "chat one").source,
      }),
    );
    await cancel;
    const cancelRequest = requestAt(fetchMock, 1);
    expect(cancelRequest.method).toBe("POST");
    expect(new URL(cancelRequest.url).pathname).toBe(
      "/v1/background-agents/agent%2F1/cancel",
    );
    await expect(cancelRequest.clone().json()).resolves.toEqual({
      chat_id: "chat one",
      subtree: true,
    });

    const message = store.dispatch(
      backgroundAgentsApi.endpoints.messageBackgroundAgent.initiate({
        agentId: "agent-1",
        chatId: "chat one",
        source: rosterQuery(store, "chat one").source,
        text: "Please prioritize the tests.",
      }),
    );
    await message;
    const messageRequest = requestAt(fetchMock, 2);
    expect(messageRequest.method).toBe("POST");
    expect(new URL(messageRequest.url).pathname).toBe(
      "/v1/background-agents/agent-1/message",
    );
    await expect(messageRequest.clone().json()).resolves.toEqual({
      chat_id: "chat one",
      text: "Please prioritize the tests.",
    });
  });

  test("normalizes camelCase REST agents", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse([
        {
          agentId: "agent-1",
          parentChatId: "parent-1",
          childChatId: null,
          kind: "subagent",
          status: "running",
          model: "openai/gpt-5.6-terra",
          questions: [
            {
              id: "question-1",
              text: "Continue?",
              askedAt: "2026-01-01T00:00:00Z",
            },
          ],
        },
      ]),
    );
    const store = createTestStore({ host: "ide", lspPort: 8123, apiKey: null });

    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
            rosterQuery(store, "chat-1"),
          ),
        )
        .unwrap(),
    ).resolves.toEqual([
      expect.objectContaining({
        agent_id: "agent-1",
        parent_chat_id: "parent-1",
        model: "openai/gpt-5.6-terra",
        plan_present: false,
        questions: [
          {
            id: "question-1",
            text: "Continue?",
            asked_at: "2026-01-01T00:00:00Z",
          },
        ],
      }),
    ]);
  });

  test("returns a custom error for a non-array REST response", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(jsonResponse({ agents: [] }));
    const store = createTestStore({ host: "ide", lspPort: 8123, apiKey: null });

    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
            rosterQuery(store, "chat-1"),
          ),
        )
        .unwrap(),
    ).rejects.toMatchObject({ status: "CUSTOM_ERROR" });
    expect(store.getState().agentsPanel.denied).toBe(true);
  });

  test("rejects malformed rows rather than authorizing a partial roster", async () => {
    const fetchMock = vi
      .fn<FetchLike>()
      .mockResolvedValue(
        jsonResponse([
          { agentId: "missing-parent", kind: "subagent", status: "running" },
        ]),
      );
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({ host: "ide", lspPort: 8123, apiKey: null });
    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
            rosterQuery(store, "chat-1"),
          ),
        )
        .unwrap(),
    ).rejects.toMatchObject({ status: "CUSTOM_ERROR" });
    expect(store.getState().agentsPanel.authorized).toBe(false);
    expect(store.getState().agentsPanel.denied).toBe(true);
  });

  test("fences an in-flight success and old controls after denial, then permits current-request recovery", async () => {
    let release: (response: Response) => void = () => {
      throw new Error("Request not started");
    };
    const fetchMock = vi.fn<FetchLike>().mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: "private-token",
    });
    const oldQuery = rosterQuery(store, "chat-1");
    const pending = store.dispatch(
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(oldQuery),
    );
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    store.dispatch(
      agentDisclosureDenied({
        source: oldQuery.source,
        error: { status: 403, data: null },
      }),
    );
    release(
      jsonResponse([
        {
          agentId: "old",
          parentChatId: "chat-1",
          kind: "subagent",
          status: "running",
        },
      ]),
    );
    await expect(pending.unwrap()).rejects.toMatchObject({
      status: "CUSTOM_ERROR",
    });
    expect(
      backgroundAgentsApi.endpoints.getBackgroundAgents.select(oldQuery)(
        store.getState(),
      ).data,
    ).toBeUndefined();
    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.cancelBackgroundAgent.initiate({
            ...oldQuery,
            agentId: "old",
          }),
        )
        .unwrap(),
    ).rejects.toMatchObject({ status: "CUSTOM_ERROR" });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    store.dispatch(agentRecoveryRequested());
    fetchMock.mockResolvedValueOnce(jsonResponse([]));
    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
            rosterQuery(store, "chat-1"),
          ),
        )
        .unwrap(),
    ).resolves.toEqual([]);
    expect(store.getState().agentsPanel.authorized).toBe(true);
    expect(JSON.stringify(store.getState().backgroundAgentsApi)).not.toContain(
      "private-token",
    );
    pending.unsubscribe();
  });

  test("sanitizes credential echoes before query and mutation errors enter RTK state or dispatched receipts", async () => {
    const token = "a2-fixture-credential";
    const opaque = "Q7bR9nVm4kXa2cDf8pLs6tHy3wUz5eGj";
    const detail = `Ownership changed: ${token}; opaque ${opaque}`;
    expect(token.length).toBeGreaterThan(0);
    expect(detail).toContain(token);
    expect(detail).toContain(opaque);
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    const actions: string[] = [];
    const store = createTestStore(
      { host: "ide", lspPort: 8123, apiKey: token },
      actions,
    );
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ detail, nested: { credential: token } }), {
        status: 401,
      }),
    );
    const query = store.dispatch(
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
        rosterQuery(store, "chat-1"),
      ),
    );
    await expect(query.unwrap()).rejects.toEqual({
      status: 401,
      data: { detail: "Ownership changed: [REDACTED]; opaque [REDACTED]" },
    });
    store.dispatch(agentRecoveryRequested());
    fetchMock.mockResolvedValueOnce(jsonResponse([]));
    const recovery = store.dispatch(
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
        rosterQuery(store, "chat-1"),
      ),
    );
    await recovery.unwrap();
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ detail, nested: { credential: token } }), {
        status: 409,
      }),
    );
    const message = store.dispatch(
      backgroundAgentsApi.endpoints.messageBackgroundAgent.initiate({
        ...rosterQuery(store, "chat-1"),
        agentId: "agent-1",
        text: "Please continue",
      }),
    );
    await expect(message.unwrap()).rejects.toEqual({
      status: 409,
      data: { detail: "Ownership changed: [REDACTED]; opaque [REDACTED]" },
    });
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ error: detail }), { status: 403 }),
    );
    const cancel = store.dispatch(
      backgroundAgentsApi.endpoints.cancelBackgroundAgent.initiate({
        ...rosterQuery(store, "chat-1"),
        agentId: "agent-1",
      }),
    );
    await expect(cancel.unwrap()).rejects.toMatchObject({ status: 403 });
    expect(store.getState().error.message).toBe(
      "Ownership changed: [REDACTED]; opaque [REDACTED]",
    );
    const serialized = JSON.stringify({
      api: store.getState().backgroundAgentsApi,
      agents: store.getState().agentsPanel,
      error: store.getState().error,
      actions,
    });
    expect(serialized).toContain("Ownership changed");
    expect(serialized).not.toContain(token);
    expect(serialized).not.toContain(opaque);
    query.unsubscribe();
    recovery.unsubscribe();
    message.reset();
    cancel.reset();
  });

  test("sanitizes transport and parse failures and discards malformed response bodies", async () => {
    const token = "a2-transport-credential";
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    const actions: string[] = [];
    const store = createTestStore(
      { host: "ide", lspPort: 8123, apiKey: token },
      actions,
    );
    store.dispatch(agentRosterAuthorized(rosterQuery(store, "chat-1").source));
    fetchMock.mockRejectedValueOnce(new Error(`Connection failed: ${token}`));
    const transport = store.dispatch(
      backgroundAgentsApi.endpoints.messageBackgroundAgent.initiate({
        ...rosterQuery(store, "chat-1"),
        agentId: "agent-1",
        text: "Continue",
      }),
    );
    await expect(transport.unwrap()).rejects.toMatchObject({
      status: "FETCH_ERROR",
      error: expect.stringContaining("[REDACTED]"),
    });
    fetchMock.mockResolvedValueOnce(
      new Response(`Invalid JSON: ${token}`, { status: 409 }),
    );
    const parse = store.dispatch(
      backgroundAgentsApi.endpoints.cancelBackgroundAgent.initiate({
        ...rosterQuery(store, "chat-1"),
        agentId: "agent-1",
      }),
    );
    await expect(parse.unwrap()).rejects.toMatchObject({
      status: "PARSING_ERROR",
      originalStatus: 409,
    });
    store.dispatch(agentRecoveryRequested());
    fetchMock.mockResolvedValueOnce(jsonResponse({ detail: token }));
    const schema = store.dispatch(
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate(
        rosterQuery(store, "chat-1"),
      ),
    );
    await expect(schema.unwrap()).rejects.toMatchObject({
      status: "CUSTOM_ERROR",
    });
    expect(
      JSON.stringify({
        api: store.getState().backgroundAgentsApi,
        error: store.getState().error,
        actions,
      }),
    ).not.toContain(token);
    schema.unsubscribe();
    transport.reset();
    parse.reset();
  });

  test("does not dispatch stale mutation error details after the source changes", async () => {
    let release: (response: Response) => void = () => {
      throw new Error("Request not started");
    };
    const fetchMock = vi.fn<FetchLike>().mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const actions: string[] = [];
    const store = createTestStore(
      { host: "ide", lspPort: 8123, apiKey: "a2-old-principal" },
      actions,
    );
    const query = rosterQuery(store, "chat-1");
    store.dispatch(agentRosterAuthorized(query.source));
    const pending = store.dispatch(
      backgroundAgentsApi.endpoints.cancelBackgroundAgent.initiate({
        ...query,
        agentId: "agent-1",
      }),
    );
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    store.dispatch(agentSourceChanged());
    release(
      new Response(
        JSON.stringify({ detail: "Stale secret receipt a2-old-principal" }),
        { status: 401 },
      ),
    );
    await expect(pending.unwrap()).rejects.toMatchObject({
      status: "CUSTOM_ERROR",
    });
    expect(store.getState().error.message).toBeNull();
    expect(store.getState().agentsPanel.denied).toBe(false);
    expect(JSON.stringify(actions)).not.toContain("Stale secret receipt");
    pending.reset();
  });
});
