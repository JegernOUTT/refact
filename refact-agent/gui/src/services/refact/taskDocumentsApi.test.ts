import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineApiConfig } from "./apiUrl";
import { tasksApi } from "./tasks";
import { trajectoriesApi } from "./trajectories";
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
  it("encodes task, planner and trajectory path segments without changing opaque body IDs", async () => {
    const config = {
      host: "web" as const,
      lspUrl: "https://engine.example/root",
      apiKey: null,
    };
    const store = configureStore({
      reducer: {
        config: () => config,
        [tasksApi.reducerPath]: tasksApi.reducer,
        [trajectoriesApi.reducerPath]: trajectoriesApi.reducer,
      },
      middleware: (getDefaultMiddleware) =>
        getDefaultMiddleware().concat(
          tasksApi.middleware,
          trajectoriesApi.middleware,
        ),
    });
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    const taskId = "task ?#%2F";
    const chatId = "planner/old ?#%2F";
    fetchMock.mockResolvedValueOnce(jsonResponse({ deleted: true }));
    await store
      .dispatch(
        tasksApi.endpoints.deletePlannerChat.initiate({ taskId, chatId }),
      )
      .unwrap();
    expect(firstRequest(fetchMock).method).toBe("DELETE");
    expect(firstRequest(fetchMock).url).toBe(
      "https://engine.example/root/v1/tasks/task%20%3F%23%252F/planner-chats/planner%2Fold%20%3F%23%252F",
    );
    fetchMock.mockClear();
    fetchMock.mockResolvedValueOnce(jsonResponse([]));
    const planners = store.dispatch(
      tasksApi.endpoints.listTaskTrajectories.initiate({
        taskId,
        role: "planner",
      }),
    );
    await planners.unwrap();
    planners.unsubscribe();
    expect(firstRequest(fetchMock).url).toBe(
      "https://engine.example/root/v1/tasks/task%20%3F%23%252F/trajectories/planner",
    );
    fetchMock.mockClear();
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ new_chat_id: "new", messages_count: 0 }),
    );
    await store
      .dispatch(
        tasksApi.endpoints.createPlannerChatFromTransition.initiate({
          taskId,
          sourceChatId: chatId,
        }),
      )
      .unwrap();
    expect(firstRequest(fetchMock).url).toBe(
      "https://engine.example/root/v1/tasks/task%20%3F%23%252F/planner-chats/from-transition",
    );
    expect(await firstRequest(fetchMock).json()).toMatchObject({
      source_chat_id: chatId,
    });
    fetchMock.mockClear();
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ path: "/stored/planner.json" }),
    );
    const trajectory = store.dispatch(
      trajectoriesApi.endpoints.getTrajectoryPath.initiate(chatId),
    );
    expect(await trajectory.unwrap()).toEqual({ path: "/stored/planner.json" });
    trajectory.unsubscribe();
    expect(firstRequest(fetchMock).url).toBe(
      "https://engine.example/root/v1/trajectories/planner%2Fold%20%3F%23%252F/path",
    );
    store.dispatch(tasksApi.util.resetApiState());
    store.dispatch(trajectoriesApi.util.resetApiState());
  });

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
        slug: "main plan?#%2F",
        version: 2,
      }),
    );
    await request;
    request.unsubscribe();

    const url = firstRequestUrl(fetchMock);
    expect(url).toBe(
      "https://remote.example.com/root/v1/task/task%2F1/documents/main%20plan%3F%23%252F?version=2",
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

  it("updateTaskDocument sends content and pin state in one request", async () => {
    const fetchMock = vi.fn<FetchLike>();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ ...validDetail, pinned: false }),
    );
    const store = createTestStore({
      host: "web",
      engineServed: true,
      lspPort: 8123,
      lspUrl: "http://127.0.0.1:8123",
      apiKey: null,
    });

    const request = store.dispatch(
      taskDocumentsApi.endpoints.updateTaskDocument.initiate({
        taskId: "task-1",
        slug: "main-plan",
        content: "# Updated",
        pinned: false,
      }),
    );
    await request;

    const fetchRequest = firstRequest(fetchMock);
    expect(fetchRequest.method).toBe("PUT");
    expect(relativeUrlPath(fetchRequest.url)).toBe(
      "/v1/task/task-1/documents/main-plan",
    );
    expect(await fetchRequest.json()).toEqual({
      content: "# Updated",
      pinned: false,
    });
  });
});
