import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { EngineApiConfig } from "./apiUrl";
import { gitReadApi } from "./gitRead";

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
      [gitReadApi.reducerPath]: gitReadApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(gitReadApi.middleware),
  });
}

function firstRequest(fetchMock: ReturnType<typeof vi.fn<FetchLike>>): Request {
  expect(fetchMock).toHaveBeenCalled();
  const [input, init] = fetchMock.mock.calls[0];
  return input instanceof Request ? input : new Request(input, init);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Git read RTK Query API", () => {
  test("generateCommitMessage parses the plain-text response", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      new Response("feat: describe staged changes", {
        headers: { "Content-Type": "text/plain" },
      }),
    );
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: "test-token",
    });

    const result = await store.dispatch(
      gitReadApi.endpoints.generateCommitMessage.initiate({
        diff: "diff --git a/src/app.ts b/src/app.ts",
        text: "Use conventional commits",
      }),
    );

    expect(result.data).toBe("feat: describe staged changes");
    const request = firstRequest(fetchMock);
    expect(request.url).toBe(
      "http://127.0.0.1:8123/v1/commit-message-from-diff",
    );
    expect(request.method).toBe("POST");
    await expect(request.clone().json()).resolves.toEqual({
      diff: "diff --git a/src/app.ts b/src/app.ts",
      text: "Use conventional commits",
    });
  });
});
