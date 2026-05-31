import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineApiConfig } from "./apiUrl";
import { chatModesApi } from "./chatModes";
import { customizationApi } from "./customization";
import { setupStatusApi } from "./setupStatus";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type TestConfigState = EngineApiConfig & {
  apiKey: string | null;
};

function createTestStore(config: TestConfigState) {
  return configureStore({
    reducer: {
      config: (state: TestConfigState = config) => state,
      [chatModesApi.reducerPath]: chatModesApi.reducer,
      [customizationApi.reducerPath]: customizationApi.reducer,
      [setupStatusApi.reducerPath]: setupStatusApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(
        chatModesApi.middleware,
        customizationApi.middleware,
        setupStatusApi.middleware,
      ),
  });
}

function jsonResponse(data: unknown): Response {
  return new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
}

function firstRequestUrl(
  fetchMock: ReturnType<typeof vi.fn<FetchLike>>,
): string {
  expect(fetchMock).toHaveBeenCalled();
  const [input] = fetchMock.mock.calls[0];
  return input instanceof Request ? input.url : String(input);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("config RTK service endpoint gates", () => {
  it("chatModes uses remote lspUrl when lspPort is zero", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(jsonResponse({ modes: [], errors: [] }));
    const store = createTestStore({
      host: "web",
      lspUrl: "https://remote.example.com/proxy/v1/ping?stale=true",
      lspPort: 0,
      apiKey: null,
    });

    const request = store.dispatch(
      chatModesApi.endpoints.getChatModes.initiate(undefined),
    );
    await request;
    request.unsubscribe();

    expect(firstRequestUrl(fetchMock)).toBe(
      "https://remote.example.com/proxy/v1/chat-modes",
    );
  });

  it("customization uses remote lspUrl when lspPort is zero", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        modes: [],
        subagents: [],
        toolbox_commands: [],
        code_lens: [],
        errors: [],
      }),
    );
    const store = createTestStore({
      host: "web",
      lspUrl: "https://remote.example.com/proxy/v1/ping?stale=true",
      lspPort: 0,
      apiKey: null,
    });

    const request = store.dispatch(
      customizationApi.endpoints.getRegistry.initiate(undefined),
    );
    await request;
    request.unsubscribe();

    expect(firstRequestUrl(fetchMock)).toBe(
      "https://remote.example.com/proxy/v1/customization/registry",
    );
  });

  it("setupStatus uses remote lspUrl when lspPort is zero", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ configured: true, reasons: [], detail: {} }),
    );
    const store = createTestStore({
      host: "web",
      lspUrl: "https://remote.example.com/proxy/v1/ping?stale=true",
      lspPort: 0,
      apiKey: null,
    });

    const request = store.dispatch(
      setupStatusApi.endpoints.getSetupStatus.initiate(undefined),
    );
    await request;
    request.unsubscribe();

    expect(firstRequestUrl(fetchMock)).toBe(
      "https://remote.example.com/proxy/v1/setup/status",
    );
  });

  it("rejects local IDE fallback without valid port or lspUrl", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({
      host: "ide",
      lspPort: 0,
      apiKey: null,
    });

    const result = await store.dispatch(
      chatModesApi.endpoints.getChatModes.initiate(undefined),
    );

    expect(fetchMock).not.toHaveBeenCalled();
    expect(result.error).toEqual({
      status: 500,
      data: "Missing engine endpoint in config",
    });
  });
});
