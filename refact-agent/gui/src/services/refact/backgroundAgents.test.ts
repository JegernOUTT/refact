import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { EngineApiConfig } from "./apiUrl";
import { backgroundAgentsApi } from "./backgroundAgents";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type TestConfigState = EngineApiConfig & { apiKey: string | null };

function createTestStore(config: TestConfigState) {
  return configureStore({
    reducer: {
      config: (state: TestConfigState = config) => state,
      [backgroundAgentsApi.reducerPath]: backgroundAgentsApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(backgroundAgentsApi.middleware),
  });
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
      backgroundAgentsApi.endpoints.getBackgroundAgents.initiate("chat one"),
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
      }),
    );
    await cancel;
    const cancelRequest = requestAt(fetchMock, 1);
    expect(cancelRequest.method).toBe("POST");
    expect(new URL(cancelRequest.url).pathname).toBe(
      "/v1/background-agents/agent%2F1/cancel",
    );
    await expect(cancelRequest.clone().json()).resolves.toEqual({
      subtree: true,
    });

    const message = store.dispatch(
      backgroundAgentsApi.endpoints.messageBackgroundAgent.initiate({
        agentId: "agent-1",
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
      text: "Please prioritize the tests.",
    });
  });

  test("normalizes camelCase REST agents and filters invalid entries", async () => {
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
        { agentId: "missing-parent", kind: "subagent", status: "running" },
      ]),
    );
    const store = createTestStore({ host: "ide", lspPort: 8123, apiKey: null });

    await expect(
      store
        .dispatch(
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate("chat-1"),
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
          backgroundAgentsApi.endpoints.getBackgroundAgents.initiate("chat-1"),
        )
        .unwrap(),
    ).rejects.toMatchObject({ status: "CUSTOM_ERROR" });
  });
});
