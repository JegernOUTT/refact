import { describe, expect, it } from "vitest";
import { configureStore } from "@reduxjs/toolkit";
import { http, HttpResponse } from "msw";
import { server } from "../../../utils/mockServer";
import {
  taskDocumentMutationInvalidation,
  taskDocumentsApi,
} from "../../../services/refact/taskDocumentsApi";

describe("taskDocumentMutationInvalidation", () => {
  it("emits list tag for createTaskDocument", () => {
    expect(
      taskDocumentMutationInvalidation.createTaskDocument("task-1"),
    ).toEqual([{ type: "TaskDocuments", id: "task-1" }]);
  });

  it("emits list, detail, and history tags for updateTaskDocument", () => {
    expect(
      taskDocumentMutationInvalidation.updateTaskDocument(
        "task-1",
        "main-plan",
      ),
    ).toEqual([
      { type: "TaskDocuments", id: "task-1" },
      { type: "TaskDocuments", id: "task-1:main-plan:detail" },
      { type: "TaskDocuments", id: "task-1:main-plan:history" },
    ]);
  });

  it("emits list, detail, and history tags for pinTaskDocument", () => {
    expect(
      taskDocumentMutationInvalidation.pinTaskDocument("task-1", "main-plan"),
    ).toEqual([
      { type: "TaskDocuments", id: "task-1" },
      { type: "TaskDocuments", id: "task-1:main-plan:detail" },
      { type: "TaskDocuments", id: "task-1:main-plan:history" },
    ]);
  });

  it("emits list, detail, and history tags for deleteTaskDocument", () => {
    expect(
      taskDocumentMutationInvalidation.deleteTaskDocument(
        "task-1",
        "main-plan",
      ),
    ).toEqual([
      { type: "TaskDocuments", id: "task-1" },
      { type: "TaskDocuments", id: "task-1:main-plan:detail" },
      { type: "TaskDocuments", id: "task-1:main-plan:history" },
    ]);
  });

  it("emits list, detail, and history tags for appendTaskDocument", () => {
    expect(
      taskDocumentMutationInvalidation.appendTaskDocument(
        "task-1",
        "main-plan",
      ),
    ).toEqual([
      { type: "TaskDocuments", id: "task-1" },
      { type: "TaskDocuments", id: "task-1:main-plan:detail" },
      { type: "TaskDocuments", id: "task-1:main-plan:history" },
    ]);
  });
});

describe("task document update payload", () => {
  it("maps the optional expectedVersion without changing legacy requests", async () => {
    const requests: { method: string; path: string; body: unknown }[] = [];
    server.use(
      http.put("*/v1/task/:taskId/documents/:slug", async ({ request }) => {
        requests.push({
          method: request.method,
          path: new URL(request.url).pathname,
          body: await request.json(),
        });
        return HttpResponse.json({ version: 4 });
      }),
    );
    const store = configureStore({
      reducer: {
        config: () => ({
          host: "ide" as const,
          lspPort: 8001,
          apiKey: "test",
        }),
        [taskDocumentsApi.reducerPath]: taskDocumentsApi.reducer,
      },
      middleware: (getDefaultMiddleware) =>
        getDefaultMiddleware().concat(taskDocumentsApi.middleware),
    });
    try {
      await store
        .dispatch(
          taskDocumentsApi.endpoints.updateTaskDocument.initiate({
            taskId: "task/1",
            slug: "main plan",
            content: "Protected draft",
            pinned: false,
            expectedVersion: 3,
          }),
        )
        .unwrap();
      await store
        .dispatch(
          taskDocumentsApi.endpoints.updateTaskDocument.initiate({
            taskId: "task/1",
            slug: "main plan",
            content: "Legacy update",
          }),
        )
        .unwrap();
      expect(requests).toEqual([
        {
          method: "PUT",
          path: "/v1/task/task%2F1/documents/main%20plan",
          body: {
            content: "Protected draft",
            pinned: false,
            expected_version: 3,
          },
        },
        {
          method: "PUT",
          path: "/v1/task/task%2F1/documents/main%20plan",
          body: { content: "Legacy update" },
        },
      ]);
    } finally {
      store.dispatch(taskDocumentsApi.util.resetApiState());
    }
  });
});
