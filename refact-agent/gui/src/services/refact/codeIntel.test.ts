import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { EngineApiConfig } from "./apiUrl";
import { codeIntelApi } from "./codeIntel";

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
      [codeIntelApi.reducerPath]: codeIntelApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(codeIntelApi.middleware),
  });
}

function jsonResponse(data: unknown): Response {
  return new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
}

function firstRequest(fetchMock: ReturnType<typeof vi.fn<FetchLike>>): Request {
  expect(fetchMock).toHaveBeenCalled();
  const [input, init] = fetchMock.mock.calls[0];
  return input instanceof Request ? input : new Request(input, init);
}

function requestPath(url: string): string {
  return url.startsWith("http") ? new URL(url).pathname : url;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Code Intel RTK Query API", () => {
  test("getCodeIntelGraph uses configured base URL auth and limit query", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        nodes: [{ id: 1, name: "main", path: "src/main.rs" }],
        edges: [],
      }),
    );
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: "test-token",
    });

    const request = store.dispatch(
      codeIntelApi.endpoints.getCodeIntelGraph.initiate({ limit: 42 }),
    );
    const result = await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(fetchRequest.url).toBe(
      "http://127.0.0.1:8123/v1/code-intel/graph?limit=42",
    );
    expect(fetchRequest.method).toBe("GET");
    expect(fetchRequest.headers.get("authorization")).toBe("Bearer test-token");
    expect(result.data).toEqual({
      nodes: [{ id: 1, name: "main", path: "src/main.rs" }],
      edges: [],
    });
  });

  test("prBlast posts changed files to a relative engine URL", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        changed_files: ["src/lib.rs"],
        directly_impacted: [],
        transitively_impacted: [],
        impacted_file_count: 0,
        risk_score: 0,
      }),
    );
    const store = createTestStore({
      host: "web",
      dev: true,
      apiKey: null,
    });

    const request = store.dispatch(
      codeIntelApi.endpoints.prBlast.initiate({
        changed_files: ["src/lib.rs"],
        max_depth: 2,
      }),
    );
    await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(requestPath(fetchRequest.url)).toBe("/v1/code-intel/pr-blast");
    expect(fetchRequest.method).toBe("POST");
    await expect(fetchRequest.clone().json()).resolves.toEqual({
      changed_files: ["src/lib.rs"],
      max_depth: 2,
    });
  });

  test("securityScan posts path requests", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse([
        {
          rule: "hardcoded_secret",
          severity: "High",
          line: 1,
          snippet: "password = secret",
        },
      ]),
    );
    const store = createTestStore({
      host: "web",
      engineServed: true,
      apiKey: null,
    });

    const request = store.dispatch(
      codeIntelApi.endpoints.securityScan.initiate({ path: "src/lib.rs" }),
    );
    const result = await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(requestPath(fetchRequest.url)).toBe("/v1/code-intel/security-scan");
    expect(fetchRequest.method).toBe("POST");
    await expect(fetchRequest.clone().json()).resolves.toEqual({
      path: "src/lib.rs",
    });
    expect(result.data).toEqual([
      {
        rule: "hardcoded_secret",
        severity: "High",
        line: 1,
        snippet: "password = secret",
      },
    ]);
  });
});
