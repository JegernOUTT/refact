import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import { tasksApi, type TaskBoard } from "./tasks";

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("tasksApi comments", () => {
  it.each([
    ".",
    "..",
    "A/B",
    "x?y",
    " x?y&card_id=foreign#%2F+ ",
    "legacy".repeat(50),
  ])(
    "keeps opaque card ID %s in the body without URL retargeting",
    async (cardId) => {
      const config = {
        host: "web" as const,
        lspUrl: "https://engine.example/p/project-1",
        apiKey: "fixture-token",
      };
      const store = configureStore({
        reducer: {
          config: () => config,
          [tasksApi.reducerPath]: tasksApi.reducer,
        },
        middleware: (getDefaultMiddleware) =>
          getDefaultMiddleware().concat(tasksApi.middleware),
      });
      const board: TaskBoard = {
        schema_version: 1,
        rev: 1,
        columns: [],
        cards: [],
      };
      const fetchMock = vi.fn<FetchLike>();
      fetchMock.mockResolvedValue(
        new Response(JSON.stringify(board), {
          headers: { "Content-Type": "application/json" },
        }),
      );
      vi.stubGlobal("fetch", fetchMock);
      await store
        .dispatch(
          tasksApi.endpoints.addCardComment.initiate({
            taskId: "task-1",
            cardId,
            body: "Exact target comment",
            authorRole: "user",
            replyTo: "parent-comment",
          }),
        )
        .unwrap();
      expect(fetchMock).toHaveBeenCalledTimes(1);
      const [input, init] = fetchMock.mock.calls[0];
      const request =
        input instanceof Request ? input : new Request(input, init);
      const url = new URL(request.url);
      expect(url.href).toBe(
        "https://engine.example/p/project-1/v1/tasks/task-1/cards/comments",
      );
      expect(url.search).toBe("");
      expect(url.hash).toBe("");
      expect(request.method).toBe("POST");
      expect(request.headers.get("authorization")).toBe("Bearer fixture-token");
      expect(await request.json()).toStrictEqual({
        card_id: cardId,
        body: "Exact target comment",
        author_role: "user",
        reply_to: "parent-comment",
      });
      store.dispatch(tasksApi.util.resetApiState());
    },
  );
});
