import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineApiConfig } from "./apiUrl";
import {
  isTaskMemoryEntry,
  isTaskMemoriesResponse,
  isTaskMemoryFacetsResponse,
  taskMemoriesApi,
} from "./taskMemoriesApi";

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
      [taskMemoriesApi.reducerPath]: taskMemoriesApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(taskMemoriesApi.middleware),
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

function firstRequestUrl(
  fetchMock: ReturnType<typeof vi.fn<FetchLike>>,
): string {
  expect(fetchMock).toHaveBeenCalled();
  const [input] = fetchMock.mock.calls[0];
  return input instanceof Request ? input.url : String(input);
}

function relativeUrlPath(url: string): string {
  return url.startsWith("http") ? new URL(url).pathname : url.split("?")[0];
}

function relativeUrlSearchParams(url: string): URLSearchParams {
  if (url.startsWith("http")) return new URL(url).searchParams;
  const queryString = url.split("?")[1] ?? "";
  return new URLSearchParams(queryString);
}

const validEntry = {
  filename: "decision.md",
  created_at: "2026-05-22T01:00:00Z",
  created_at_known: true,
  title: "Use scoped memory index",
  content: "Keep memory search local.",
  tags: ["planner", "search"],
  kind: "decision",
  namespace: "task",
  pinned: false,
  status: "active",
};

const validMemoriesResponse = {
  task_id: "task-1",
  since: "2026-05-22T00:00:00Z",
  new_count: 0,
  memories: [],
  warnings: [],
};

const validFacetsResponse = {
  task_id: "task-1",
  namespaces: ["task"],
  tags: ["planner"],
  kinds: ["decision"],
  total_count: 1,
  pinned_count: 0,
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("isTaskMemoryEntry", () => {
  it("accepts valid entry", () => {
    expect(isTaskMemoryEntry(validEntry)).toBe(true);
  });

  it("is_task_memory_entry_rejects_string_in_tags_array", () => {
    expect(
      isTaskMemoryEntry({ ...validEntry, tags: ["ok", 42, "also-ok"] }),
    ).toBe(false);
  });

  it("rejects when filename is missing", () => {
    const { filename: _filename, ...withoutFilename } = validEntry;
    expect(isTaskMemoryEntry(withoutFilename)).toBe(false);
  });

  it("rejects when tags is not an array", () => {
    expect(isTaskMemoryEntry({ ...validEntry, tags: "planner" })).toBe(false);
  });

  it("rejects null", () => {
    expect(isTaskMemoryEntry(null)).toBe(false);
  });
});

describe("isTaskMemoriesResponse", () => {
  it("accepts valid response", () => {
    expect(isTaskMemoriesResponse(validMemoriesResponse)).toBe(true);
  });

  it("rejects when memories is not an array", () => {
    expect(
      isTaskMemoriesResponse({
        task_id: "task-1",
        since: "2026-05-22T00:00:00Z",
        new_count: 0,
        memories: null,
        warnings: [],
      }),
    ).toBe(false);
  });
});

describe("isTaskMemoryFacetsResponse", () => {
  it("accepts valid facets response", () => {
    expect(isTaskMemoryFacetsResponse(validFacetsResponse)).toBe(true);
  });

  it("rejects when kinds is missing", () => {
    expect(
      isTaskMemoryFacetsResponse({
        task_id: "task-1",
        namespaces: ["task"],
        tags: [],
        total_count: 0,
        pinned_count: 0,
      }),
    ).toBe(false);
  });
});

describe("use_get_task_memory_facets_calls_facets_endpoint_not_full_list", () => {
  it("facets URL contains /facets suffix, not the list URL", () => {
    const taskId = "test-task";
    const facetsPath = `/v1/task/${encodeURIComponent(taskId)}/memories/facets`;
    const listPath = `/v1/task/${encodeURIComponent(taskId)}/memories`;
    expect(facetsPath).toContain("/memories/facets");
    expect(facetsPath).not.toBe(listPath);
    expect(facetsPath).not.toMatch(/\/memories$/);
  });
});

describe("taskMemoriesApi URLs", () => {
  it("listTaskMemories uses relative URL and preserves filtered query params in Vite dev web mode", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(jsonResponse(validMemoriesResponse));
    const store = createTestStore({
      host: "web",
      dev: true,
      lspPort: 8123,
      lspUrl: "http://127.0.0.1:8123",
      apiKey: "test-token",
    });

    const request = store.dispatch(
      taskMemoriesApi.endpoints.listTaskMemories.initiate({
        taskId: "task/1",
        since: "2026-05-22T00:00:00Z",
        kind: "all",
        namespace: "task",
        search: "cache hints",
      }),
    );
    await request;
    request.unsubscribe();

    const fetchRequest = firstRequest(fetchMock);
    const params = relativeUrlSearchParams(fetchRequest.url);
    expect(relativeUrlPath(fetchRequest.url)).toBe(
      "/v1/task/task%2F1/memories",
    );
    expect(params.get("since")).toBe("2026-05-22T00:00:00Z");
    expect(params.get("kind")).toBeNull();
    expect(params.get("namespace")).toBe("task");
    expect(params.get("search")).toBe("cache hints");
    expect(fetchRequest.headers.get("authorization")).toBe("Bearer test-token");
  });

  it("getTaskMemoryFacets uses remote configured origin", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(jsonResponse(validFacetsResponse));
    const store = createTestStore({
      host: "web",
      lspPort: 8123,
      lspUrl: "https://remote.example.com/root/v1/task/stale/memories?old=true",
      apiKey: null,
    });

    const request = store.dispatch(
      taskMemoriesApi.endpoints.getTaskMemoryFacets.initiate({
        taskId: "task/1",
      }),
    );
    await request;
    request.unsubscribe();

    expect(firstRequestUrl(fetchMock)).toBe(
      "https://remote.example.com/root/v1/task/task%2F1/memories/facets",
    );
  });

  it("pinTaskMemory uses relative URL in engine-served web mode", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        ok: true,
        filename: "decision.md",
        pinned: true,
        changed: true,
      }),
    );
    const store = createTestStore({
      host: "web",
      engineServed: true,
      lspPort: 8123,
      lspUrl: "http://127.0.0.1:8123",
      apiKey: null,
    });

    const request = store.dispatch(
      taskMemoriesApi.endpoints.pinTaskMemory.initiate({
        taskId: "task-1",
        filename: "notes/decision.md",
        pinned: true,
      }),
    );
    await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(relativeUrlPath(fetchRequest.url)).toBe(
      "/v1/task/task-1/memories/notes%2Fdecision.md/pin",
    );
    expect(fetchRequest.method).toBe("POST");
    await expect(fetchRequest.clone().json()).resolves.toEqual({
      pinned: true,
    });
  });
});
