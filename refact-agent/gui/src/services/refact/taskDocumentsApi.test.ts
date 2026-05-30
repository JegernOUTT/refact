import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineApiConfig } from "./apiUrl";
import {
  isTaskDocumentDetail,
  isTaskDocumentSummary,
  isTaskDocumentHistoryResponse,
  taskDocumentsApi,
} from "./taskDocumentsApi";

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
      [taskDocumentsApi.reducerPath]: taskDocumentsApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(taskDocumentsApi.middleware),
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

const validDetail = {
  slug: "main-plan",
  name: "Main Plan",
  kind: "plan",
  content: "# Plan\n\nDo stuff.",
  version: 3,
  pinned: true,
  created_at: "2026-05-22T00:00:00Z",
  updated_at: "2026-05-23T00:00:00Z",
  author_role: "planner",
  relevant_cards: [],
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("isTaskDocumentDetail", () => {
  it("transform_response_accepts_valid_shape", () => {
    expect(isTaskDocumentDetail(validDetail)).toBe(true);
  });

  it("transform_response_throws_on_missing_slug", () => {
    const { slug: _slug, ...withoutSlug } = validDetail;
    expect(isTaskDocumentDetail(withoutSlug)).toBe(false);
  });

  it("rejects null", () => {
    expect(isTaskDocumentDetail(null)).toBe(false);
  });

  it("rejects when version is a string", () => {
    expect(isTaskDocumentDetail({ ...validDetail, version: "3" })).toBe(false);
  });

  it("rejects when content is missing", () => {
    const { content: _content, ...withoutContent } = validDetail;
    expect(isTaskDocumentDetail(withoutContent)).toBe(false);
  });
});

describe("isTaskDocumentSummary", () => {
  it("accepts valid summary", () => {
    const summary = {
      slug: "main-plan",
      name: "Main Plan",
      kind: "plan",
      pinned: false,
      version: 1,
      updated_at: "2026-05-22T00:00:00Z",
      created_at: "2026-05-21T00:00:00Z",
      author_role: "planner",
      relevant_cards: [],
    };
    expect(isTaskDocumentSummary(summary)).toBe(true);
  });

  it("rejects when slug is missing", () => {
    expect(isTaskDocumentSummary({ name: "x", kind: "plan" })).toBe(false);
  });
});

describe("isTaskDocumentHistoryResponse", () => {
  it("accepts valid history response", () => {
    expect(
      isTaskDocumentHistoryResponse({
        task_id: "task-1",
        slug: "main-plan",
        history: [],
      }),
    ).toBe(true);
  });

  it("rejects when history is not an array", () => {
    expect(
      isTaskDocumentHistoryResponse({
        task_id: "task-1",
        slug: "main-plan",
        history: null,
      }),
    ).toBe(false);
  });
});

describe("taskDocumentsApi URLs", () => {
  it("listTaskDocuments uses relative URL in Vite dev web mode", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ task_id: "task/1", documents: [] }),
    );
    const store = createTestStore({
      host: "web",
      dev: true,
      lspPort: 8123,
      lspUrl: "http://127.0.0.1:8123",
      apiKey: "test-token",
    });

    const request = store.dispatch(
      taskDocumentsApi.endpoints.listTaskDocuments.initiate({
        taskId: "task/1",
      }),
    );
    await request;
    request.unsubscribe();

    const fetchRequest = firstRequest(fetchMock);
    expect(relativeUrlPath(fetchRequest.url)).toBe(
      "/v1/task/task%2F1/documents",
    );
    expect(fetchRequest.headers.get("authorization")).toBe("Bearer test-token");
  });

  it("getTaskDocument uses remote configured origin and version query", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(jsonResponse(validDetail));
    const store = createTestStore({
      host: "web",
      lspPort: 8123,
      lspUrl:
        "https://remote.example.com/root/v1/task/stale/documents?old=true",
      apiKey: null,
    });

    const request = store.dispatch(
      taskDocumentsApi.endpoints.getTaskDocument.initiate({
        taskId: "task/1",
        slug: "main plan",
        version: 2,
      }),
    );
    await request;
    request.unsubscribe();

    const url = firstRequestUrl(fetchMock);
    expect(url).toBe(
      "https://remote.example.com/root/v1/task/task%2F1/documents/main%20plan?version=2",
    );
  });

  it("getTaskDocumentHistory uses relative URL in engine-served web mode", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ task_id: "task-1", slug: "main-plan", history: [] }),
    );
    const store = createTestStore({
      host: "web",
      engineServed: true,
      lspPort: 8123,
      lspUrl: "http://127.0.0.1:8123",
      apiKey: null,
    });

    const request = store.dispatch(
      taskDocumentsApi.endpoints.getTaskDocumentHistory.initiate({
        taskId: "task-1",
        slug: "main-plan",
      }),
    );
    await request;
    request.unsubscribe();

    const url = firstRequestUrl(fetchMock);
    expect(relativeUrlPath(url)).toBe(
      "/v1/task/task-1/documents/main-plan/history",
    );
    expect(relativeUrlSearchParams(url).toString()).toBe("");
  });
});
