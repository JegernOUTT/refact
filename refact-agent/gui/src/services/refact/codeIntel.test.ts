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
  if (!url.startsWith("http")) return url;
  const parsed = new URL(url);
  return `${parsed.pathname}${parsed.search}`;
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

  test("getCodeIntelHealth uses list query parameters", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        aggregate: {
          file_count: 0,
          function_count: 0,
          avg_score: 10,
          grade: "A",
          max_complexity: 0,
          avg_maintainability: 100,
          avg_duplication_pct: 0,
          biomarker_count: 0,
          refactoring_count: 0,
        },
        files: [],
      }),
    );
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: null,
    });

    const request = store.dispatch(
      codeIntelApi.endpoints.getCodeIntelHealth.initiate({
        path: "src/lib.rs",
        limit: 5,
      }),
    );
    await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(fetchRequest.url).toBe(
      "http://127.0.0.1:8123/v1/code-intel/health?path=src%2Flib.rs&limit=5",
    );
    expect(fetchRequest.method).toBe("GET");
  });

  test("getCodeIntelGitRisk and duplication target their endpoints", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock
      .mockResolvedValueOnce(
        jsonResponse({
          commits_analyzed: 0,
          agent_authored_pct: 0,
          hotspots: [],
          ownership: [],
          co_change: [],
          coupling: [],
          reviewers: [],
        }),
      )
      .mockResolvedValueOnce(
        jsonResponse({
          aggregate: {
            file_count: 0,
            clone_pair_count: 0,
            duplication_pct: 0,
            duplication_percent: 0,
          },
          clones: [],
          dry_violations: [],
          test_smells: [],
        }),
      );
    const store = createTestStore({
      host: "web",
      dev: true,
      apiKey: null,
    });

    await store.dispatch(
      codeIntelApi.endpoints.getCodeIntelGitRisk.initiate({ limit: 7 }),
    );
    await store.dispatch(
      codeIntelApi.endpoints.getCodeIntelDuplication.initiate({ limit: 8 }),
    );

    const [riskInput, riskInit] = fetchMock.mock.calls[0];
    const [duplicationInput, duplicationInit] = fetchMock.mock.calls[1];
    const riskRequest =
      riskInput instanceof Request
        ? riskInput
        : new Request(riskInput, riskInit);
    const duplicationRequest =
      duplicationInput instanceof Request
        ? duplicationInput
        : new Request(duplicationInput, duplicationInit);
    expect(requestPath(riskRequest.url)).toBe(
      "/v1/code-intel/git-risk?limit=7",
    );
    expect(requestPath(duplicationRequest.url)).toBe(
      "/v1/code-intel/duplication?limit=8",
    );
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
