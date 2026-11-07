import { describe, expect, it, vi } from "vitest";
import { within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { DocumentsPanel } from "./DocumentsPanel";
import { DocumentEditor } from "./DocumentEditor";
import { taskDocumentsApi } from "../../../services/refact/taskDocumentsApi";
import type {
  TaskDocumentListResponse,
  TaskDocumentDetail,
} from "../../../services/refact/taskDocumentsApi";

HTMLElement.prototype.hasPointerCapture = () => false;

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

const listResponse: TaskDocumentListResponse = {
  task_id: "task-1",
  documents: [
    {
      slug: "initial-plan",
      name: "Initial Plan",
      kind: "plan",
      pinned: true,
      version: 3,
      updated_at: "2026-05-25T10:00:00Z",
      created_at: "2026-05-20T10:00:00Z",
      author_role: "planner",
      relevant_cards: [],
    },
    {
      slug: "api-design",
      name: "API Design",
      kind: "design",
      pinned: false,
      version: 1,
      updated_at: "2026-05-24T10:00:00Z",
      created_at: "2026-05-24T10:00:00Z",
      author_role: "planner",
      relevant_cards: [],
    },
  ],
};

const detailResponse: TaskDocumentDetail = {
  slug: "initial-plan",
  name: "Initial Plan",
  kind: "plan",
  pinned: true,
  version: 3,
  content: "# Initial Plan\n\nThis is the initial plan content.",
  created_at: "2026-05-20T10:00:00Z",
  updated_at: "2026-05-25T10:00:00Z",
  author_role: "planner",
  relevant_cards: [],
};

const historicalDetailResponse: TaskDocumentDetail = {
  ...detailResponse,
  version: 1,
  content: "# Historical Plan\n\nThis is the historical content.",
  updated_at: "2026-05-21T10:00:00Z",
};

function mockDocuments(response: TaskDocumentListResponse = listResponse) {
  server.use(
    http.get("*/v1/task/:taskId/documents", () => HttpResponse.json(response)),
    http.get("*/v1/task/:taskId/documents/:slug", ({ request, params }) => {
      const slug = String(params.slug);
      const version = new URL(request.url).searchParams.get("version");
      if (version) {
        return HttpResponse.json({ error: "not found" }, { status: 404 });
      }
      const found = response.documents.find((d) => d.slug === slug);
      if (found) {
        return HttpResponse.json({ ...found, content: "" });
      }
      return HttpResponse.json({ error: "not found" }, { status: 404 });
    }),
    http.get("*/v1/task/:taskId/documents/:slug/history", ({ params }) =>
      HttpResponse.json({
        task_id: String(params.taskId),
        slug: String(params.slug),
        history: [],
      }),
    ),
  );
}

describe("DocumentsPanel", () => {
  it("clicking a row expands inline content as markdown", async () => {
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", ({ params }) => {
        if (params.slug === "initial-plan") {
          return HttpResponse.json(detailResponse);
        }
        return HttpResponse.json({ status: 404 }, { status: 404 });
      }),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByText("Initial Plan"));

    await waitFor(() => {
      expect(
        screen.getByText("This is the initial plan content."),
      ).toBeInTheDocument();
    });
  });

  it("clicking pin icon toggles pinned state", async () => {
    const pinRequests: unknown[] = [];
    mockDocuments();
    server.use(
      http.post(
        "*/v1/task/:taskId/documents/:slug/pin",
        async ({ request }) => {
          pinRequests.push(await request.json());
          return HttpResponse.json({
            ...detailResponse,
            pinned: false,
          });
        },
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    const row = screen.getByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Unpin" }));

    await waitFor(() => {
      expect(pinRequests).toEqual([{ pinned: false }]);
    });
  });

  it("clicking new opens editor in create mode", async () => {
    mockDocuments();

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    await user.click(screen.getByRole("button", { name: /New/i }));

    expect(await screen.findByText("New document")).toBeInTheDocument();
    expect(screen.getByLabelText("Slug")).toHaveValue("");
    expect(screen.getByLabelText("Content")).toHaveValue("");
  });

  it("editor save calls create mutation and closes", async () => {
    const createRequests: unknown[] = [];
    mockDocuments();
    server.use(
      http.post("*/v1/task/:taskId/documents", async ({ request }) => {
        const body = await request.json();
        createRequests.push(body);
        return HttpResponse.json({
          ...detailResponse,
          slug: (body as { slug: string }).slug,
          name: (body as { name: string }).name,
        });
      }),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    await user.click(screen.getByRole("button", { name: /New/i }));

    await screen.findByText("New document");

    await user.type(screen.getByLabelText("Slug"), "my-design");
    await user.type(screen.getByLabelText("Name"), "My Design");
    await user.type(screen.getByLabelText("Content"), "Some content here");

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(createRequests).toHaveLength(1);
      const req = createRequests[0] as {
        slug: string;
        name: string;
        content: string;
      };
      expect(req.slug).toBe("my-design");
      expect(req.name).toBe("My Design");
      expect(req.content).toBe("Some content here");
    });

    await waitFor(() => {
      expect(screen.queryByText("New document")).not.toBeInTheDocument();
    });
  });

  it("clicking edit opens editor in edit mode prefilled", async () => {
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", ({ params }) => {
        if (params.slug === "initial-plan") {
          return HttpResponse.json(detailResponse);
        }
        return HttpResponse.json({ status: 404 }, { status: 404 });
      }),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    const row = screen.getByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Edit" }));

    await screen.findByText("Edit document");

    await waitFor(() => {
      expect(screen.getByLabelText("Content")).toHaveValue(
        detailResponse.content,
      );
    });
  });

  it("editor save sends content and pin changes in one update request", async () => {
    const updateRequests: unknown[] = [];
    const pinRequests: unknown[] = [];
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", ({ params }) => {
        if (params.slug === "initial-plan") {
          return HttpResponse.json(detailResponse);
        }
        return HttpResponse.json({ status: 404 }, { status: 404 });
      }),
      http.put("*/v1/task/:taskId/documents/:slug", async ({ request }) => {
        const body = await request.json();
        updateRequests.push(body);
        return HttpResponse.json({
          ...detailResponse,
          content: (body as { content: string }).content,
          pinned: (body as { pinned: boolean }).pinned,
          version: 4,
        });
      }),
      http.post(
        "*/v1/task/:taskId/documents/:slug/pin",
        async ({ request }) => {
          pinRequests.push(await request.json());
          return HttpResponse.json(detailResponse);
        },
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    const row = screen.getByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    await screen.findByText("Edit document");
    await waitFor(() => {
      expect(screen.getByLabelText("Content")).toHaveValue(
        detailResponse.content,
      );
    });

    await user.clear(screen.getByLabelText("Content"));
    await user.type(screen.getByLabelText("Content"), "Updated body");
    await user.click(screen.getByRole("checkbox", { name: "Pinned" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(updateRequests).toEqual([
        { content: "Updated body", pinned: false, expected_version: 3 },
      ]);
    });
    expect(pinRequests).toEqual([]);
  });

  it("keeps a dirty draft and pin through detail refresh and requires explicit conflict resolution", async () => {
    mockDocuments();
    let current = detailResponse;
    const updates: unknown[] = [];
    const reads = vi.fn(() => HttpResponse.json(current));
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", reads),
      http.put("*/v1/task/:taskId/documents/:slug", async ({ request }) => {
        updates.push(await request.json());
        return HttpResponse.json(current);
      }),
    );
    const { user, store } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });
    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(current.content),
    );
    const initialReads = reads.mock.calls.length;
    await user.clear(screen.getByLabelText("Content"));
    await user.type(screen.getByLabelText("Content"), "Human draft");
    await user.click(screen.getByRole("checkbox", { name: "Pinned" }));
    expect(reads).toHaveBeenCalledTimes(initialReads);
    expect(updates).toEqual([]);

    current = { ...detailResponse, version: 4, content: "External change" };
    await act(async () => {
      await store.dispatch(
        taskDocumentsApi.endpoints.getTaskDocument.initiate(
          { taskId: "task-1", slug: "initial-plan" },
          { forceRefetch: true, subscribe: false },
        ),
      );
    });
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    expect(screen.getByRole("checkbox", { name: "Pinned" })).not.toBeChecked();
    expect(
      screen.getByText(/This document changed on the server/),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(updates).toEqual([]);

    await user.click(
      screen.getByRole("button", { name: "Reload server copy" }),
    );
    expect(screen.getByText(/Discard your draft and reload/)).toBeVisible();
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    await user.click(screen.getByRole("button", { name: "Keep editing" }));
    await user.click(
      screen.getByRole("button", { name: "Keep draft for next save" }),
    );
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(screen.getByText(/Draft based on v4/)).toBeVisible();
    expect(updates).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(updates).toEqual([
        { content: "Human draft", pinned: false, expected_version: 4 },
      ]),
    );
  });

  it("retains the edit-session base and all input on a stale 409 until explicit reconciliation", async () => {
    let current = detailResponse;
    let readsReturnStaleCopy = true;
    const updates: unknown[] = [];
    const posts = vi.fn(() => HttpResponse.json(detailResponse));
    const onOpenChange = vi.fn();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", () =>
        HttpResponse.json(readsReturnStaleCopy ? detailResponse : current),
      ),
      http.put("*/v1/task/:taskId/documents/:slug", async ({ request }) => {
        const body = (await request.json()) as {
          content: string;
          pinned: boolean;
          expected_version: number;
        };
        updates.push(body);
        if (body.expected_version !== current.version) {
          return HttpResponse.json(
            {
              code: "task_document_version_conflict",
              expected_version: body.expected_version,
              actual_version: current.version,
            },
            { status: 409 },
          );
        }
        current = {
          ...current,
          content: body.content,
          pinned: body.pinned,
          version: current.version + 1,
        };
        return HttpResponse.json(current);
      }),
      http.post("*/v1/task/:taskId/documents", posts),
    );
    const { user } = render(
      <DocumentEditor
        taskId="task-1"
        slug="initial-plan"
        mode="edit"
        open
        onOpenChange={onOpenChange}
      />,
      { preloadedState: CONFIG_STATE },
    );
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(current.content),
    );
    await user.clear(screen.getByLabelText("Content"));
    await user.type(screen.getByLabelText("Content"), "Human draft");
    await user.click(screen.getByRole("checkbox", { name: "Pinned" }));
    current = { ...detailResponse, version: 4, content: "External change" };
    await user.dblClick(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText(/This document changed on the server/),
    ).toBeVisible();
    expect(updates).toEqual([
      { content: "Human draft", pinned: false, expected_version: 3 },
    ]);
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    expect(screen.getByLabelText("Slug")).toHaveValue("initial-plan");
    expect(screen.getByLabelText("Name")).toHaveValue("Initial Plan");
    expect(screen.getByRole("checkbox", { name: "Pinned" })).not.toBeChecked();
    expect(screen.getByText(/Draft based on v3/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Keep draft for next save" }),
    ).toBeDisabled();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(
      screen.queryByText(/save outcome could not be confirmed/),
    ).toBeNull();

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Refresh server copy" }),
      ).toBeEnabled(),
    );
    readsReturnStaleCopy = false;
    await user.click(
      screen.getByRole("button", { name: "Refresh server copy" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Keep draft for next save" }),
      ).toBeEnabled(),
    );
    expect(screen.getByText(/Draft based on v3/)).toBeVisible();
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    await user.click(
      screen.getByRole("button", { name: "Keep draft for next save" }),
    );
    expect(screen.getByText(/Draft based on v4/)).toBeVisible();
    expect(screen.getByLabelText("Content")).toHaveValue("Human draft");
    expect(screen.getByRole("checkbox", { name: "Pinned" })).not.toBeChecked();
    expect(updates).toHaveLength(1);
    expect(
      screen.queryByText(/This document changed on the server/),
    ).toBeNull();

    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(updates).toEqual([
      { content: "Human draft", pinned: false, expected_version: 3 },
      { content: "Human draft", pinned: false, expected_version: 4 },
    ]);
    expect(screen.getByText(/Draft based on v5/)).toBeVisible();
    expect(posts).not.toHaveBeenCalled();
  });

  it("follows a clean refresh but confirms before discarding a dirty draft for a reload", async () => {
    mockDocuments();
    let current = detailResponse;
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", () =>
        HttpResponse.json(current),
      ),
    );
    const { user, store } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });
    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(current.content),
    );
    current = {
      ...detailResponse,
      version: 4,
      content: "Clean refresh",
      pinned: false,
    };
    await act(async () => {
      await store.dispatch(
        taskDocumentsApi.endpoints.getTaskDocument.initiate(
          { taskId: "task-1", slug: "initial-plan" },
          { forceRefetch: true, subscribe: false },
        ),
      );
    });
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue("Clean refresh"),
    );
    expect(screen.getByText(/Draft based on v4/)).toBeVisible();
    expect(screen.getByRole("checkbox", { name: "Pinned" })).not.toBeChecked();
    expect(
      screen.queryByText(/This document changed on the server/),
    ).toBeNull();
    await user.type(screen.getByLabelText("Content"), " with local edits");
    current = {
      ...current,
      version: 5,
      content: "New server copy",
      pinned: true,
    };
    await act(async () => {
      await store.dispatch(
        taskDocumentsApi.endpoints.getTaskDocument.initiate(
          { taskId: "task-1", slug: "initial-plan" },
          { forceRefetch: true, subscribe: false },
        ),
      );
    });
    await user.click(
      screen.getByRole("button", { name: "Reload server copy" }),
    );
    expect(screen.getByLabelText("Content")).toHaveValue(
      "Clean refresh with local edits",
    );
    await user.click(
      screen.getByRole("button", { name: "Discard draft and reload" }),
    );
    expect(screen.getByLabelText("Content")).toHaveValue("New server copy");
    expect(screen.getByText(/Draft based on v5/)).toBeVisible();
    expect(screen.getByRole("checkbox", { name: "Pinned" })).toBeChecked();
    expect(
      screen.queryByText(/This document changed on the server/),
    ).toBeNull();
  });

  it("isolates drafts by task and slug and preserves them across a controlled close and reopen", async () => {
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", ({ params }) =>
        HttpResponse.json({
          ...detailResponse,
          slug: String(params.slug),
          content: `${String(params.taskId)}/${String(params.slug)}`,
        }),
      ),
    );
    const onOpenChange = vi.fn();
    const editor = (taskId: string, slug: string, open = true) => (
      <DocumentEditor
        taskId={taskId}
        slug={slug}
        mode="edit"
        open={open}
        onOpenChange={onOpenChange}
      />
    );
    const { user, rerender } = render(editor("task-1", "initial-plan"), {
      preloadedState: CONFIG_STATE,
    });
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(
        "task-1/initial-plan",
      ),
    );
    await user.clear(screen.getByLabelText("Content"));
    await user.type(screen.getByLabelText("Content"), "Task one draft");
    rerender(editor("task-2", "initial-plan"));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(
        "task-2/initial-plan",
      ),
    );
    rerender(editor("task-1", "api-design"));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue("task-1/api-design"),
    );
    rerender(editor("task-1", "initial-plan", false));
    rerender(editor("task-1", "initial-plan"));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue("Task one draft"),
    );
    await user.keyboard("{Escape}");
    expect(screen.getByText("Discard unsaved changes?")).toBeVisible();
    expect(onOpenChange).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Keep editing" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Discard changes" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    rerender(editor("task-1", "initial-plan", false));
    rerender(editor("task-1", "initial-plan"));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(
        "task-1/initial-plan",
      ),
    );
  });

  it("shows failed fetch retry and retains the draft after a guarded failed save", async () => {
    mockDocuments();
    let failFetch = true;
    let finishSave: (() => void) | undefined;
    const updates: string[] = [];
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug", () =>
        failFetch
          ? HttpResponse.json({ error: "fail" }, { status: 500 })
          : HttpResponse.json(detailResponse),
      ),
      http.put("*/v1/task/:taskId/documents/:slug", async ({ params }) => {
        updates.push(`${String(params.taskId)}/${String(params.slug)}`);
        await new Promise<void>((resolve) => {
          finishSave = resolve;
        });
        return HttpResponse.json({ error: "fail" }, { status: 500 });
      }),
    );
    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });
    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    expect(
      await screen.findByText(
        "Failed to load document initial-plan in task task-1.",
      ),
    ).toBeVisible();
    expect(screen.queryByLabelText("Loading document")).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    failFetch = false;
    await user.click(screen.getByRole("button", { name: "Retry document" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Content")).toHaveValue(
        detailResponse.content,
      ),
    );
    await user.clear(screen.getByLabelText("Content"));
    await user.type(screen.getByLabelText("Content"), "Retained draft");
    await user.dblClick(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(updates).toEqual(["task-1/initial-plan"]));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    finishSave?.();
    expect(
      await screen.findByText(
        /Failed to save document initial-plan in task task-1/,
      ),
    ).toBeVisible();
    expect(screen.getByLabelText("Content")).toHaveValue("Retained draft");
    expect(
      screen.getByText(/save outcome could not be confirmed/),
    ).toBeVisible();
    expect(
      screen.queryByText(/This document changed on the server/),
    ).toBeNull();
    expect(screen.getByRole("dialog", { name: "Edit document" })).toBeVisible();
  });

  it("delete shows confirm popover then calls delete mutation", async () => {
    const deleteRequests: string[] = [];
    mockDocuments();
    server.use(
      http.delete("*/v1/task/:taskId/documents/:slug", ({ params }) => {
        deleteRequests.push(String(params.slug));
        return new HttpResponse(null, { status: 204 });
      }),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    const row = screen.getByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Delete" }));

    expect(
      await screen.findByText("Delete this document?"),
    ).toBeInTheDocument();
    expect(deleteRequests).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: "Confirm delete" }));

    await waitFor(() => {
      expect(deleteRequests).toEqual(["initial-plan"]);
    });
  });

  it("clicking a history version loads and renders historical content", async () => {
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug/history", ({ params }) => {
        if (params.slug === "initial-plan") {
          return HttpResponse.json({
            task_id: "task-1",
            slug: "initial-plan",
            history: [
              {
                version: 1,
                updated_at: "2026-05-21T10:00:00Z",
                author_role: "planner",
                size_bytes: 128,
              },
            ],
          });
        }
        return HttpResponse.json({ status: 404 }, { status: 404 });
      }),
      http.get("*/v1/task/:taskId/documents/:slug", ({ request, params }) => {
        const version = new URL(request.url).searchParams.get("version");
        if (params.slug === "initial-plan" && version === "1") {
          return HttpResponse.json(historicalDetailResponse);
        }
        return HttpResponse.json({ status: 404 }, { status: 404 });
      }),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "History" }));

    const dialog = await screen.findByRole("dialog", {
      name: "History: initial-plan",
    });
    await user.click(within(dialog).getByRole("button", { name: /v1/i }));

    await waitFor(() => {
      expect(
        screen.getByText("This is the historical content."),
      ).toBeInTheDocument();
    });
  });

  it("escape key closes the history dialog", async () => {
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug/history", () =>
        HttpResponse.json({
          task_id: "task-1",
          slug: "initial-plan",
          history: [],
        }),
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "History" }));

    expect(
      await screen.findByRole("dialog", { name: "History: initial-plan" }),
    ).toBeInTheDocument();

    await user.keyboard("{Escape}");

    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: "History: initial-plan" }),
      ).not.toBeInTheDocument();
    });
  });

  it("tab_cycles_focus_within_history_dialog", async () => {
    mockDocuments();
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug/history", () =>
        HttpResponse.json({
          task_id: "task-1",
          slug: "initial-plan",
          history: [],
        }),
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "History" }));

    const dialog = await screen.findByRole("dialog", {
      name: "History: initial-plan",
    });
    expect(dialog).toBeInTheDocument();

    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);

    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it("pinned documents render before unpinned in the list", async () => {
    mockDocuments({
      task_id: "task-1",
      documents: [
        {
          slug: "api-design",
          name: "API Design",
          kind: "design",
          pinned: false,
          version: 1,
          updated_at: "2026-05-24T10:00:00Z",
          created_at: "2026-05-24T10:00:00Z",
          author_role: "planner",
          relevant_cards: [],
        },
        {
          slug: "initial-plan",
          name: "Initial Plan",
          kind: "plan",
          pinned: true,
          version: 3,
          updated_at: "2026-05-25T10:00:00Z",
          created_at: "2026-05-20T10:00:00Z",
          author_role: "planner",
          relevant_cards: [],
        },
      ],
    });

    render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");

    const allDocNames = screen
      .getAllByRole("button", { name: /Unpin|Pin/ })
      .map(
        (btn) =>
          btn
            .closest("[data-testid^='document-row-']")
            ?.getAttribute("data-testid"),
      );

    expect(allDocNames[0]).toBe("document-row-initial-plan");
    expect(allDocNames[1]).toBe("document-row-api-design");
  });

  it("kind filter narrows visible documents", async () => {
    mockDocuments();

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    expect(screen.getByText("API Design")).toBeInTheDocument();

    await user.click(screen.getByRole("combobox", { name: "Kind filter" }));
    await user.click(await screen.findByRole("option", { name: "plan" }));

    await waitFor(() => {
      expect(screen.getByText("Initial Plan")).toBeInTheDocument();
      expect(screen.queryByText("API Design")).not.toBeInTheDocument();
    });
  });

  it("pin_success_does_not_leave_stale_optimistic_override", async () => {
    mockDocuments({
      task_id: "task-1",
      documents: [
        {
          slug: "api-design",
          name: "API Design",
          kind: "design",
          pinned: false,
          version: 1,
          updated_at: "2026-05-24T10:00:00Z",
          created_at: "2026-05-24T10:00:00Z",
          author_role: "planner",
          relevant_cards: [],
        },
      ],
    });
    server.use(
      http.post("*/v1/task/:taskId/documents/:slug/pin", () =>
        HttpResponse.json({
          slug: "api-design",
          name: "API Design",
          kind: "design",
          pinned: true,
          version: 1,
          content: "",
          created_at: "2026-05-24T10:00:00Z",
          updated_at: "2026-05-24T10:00:00Z",
          author_role: "planner",
          relevant_cards: [],
        }),
      ),
      http.get("*/v1/task/:taskId/documents", () =>
        HttpResponse.json({
          task_id: "task-1",
          documents: [
            {
              slug: "api-design",
              name: "API Design",
              kind: "design",
              pinned: false,
              version: 1,
              updated_at: "2026-05-24T10:00:00Z",
              created_at: "2026-05-24T10:00:00Z",
              author_role: "planner",
              relevant_cards: [],
            },
          ],
        }),
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("API Design");
    const row = screen.getByTestId("document-row-api-design");
    await user.click(within(row).getByRole("button", { name: "Pin" }));

    await waitFor(() => {
      expect(
        within(row).getByRole("button", { name: "Pin" }),
      ).toBeInTheDocument();
    });
  });

  it("editor save disabled until slug name and content are valid", async () => {
    mockDocuments();

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    await user.click(screen.getByRole("button", { name: /New/i }));

    await screen.findByText("New document");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(screen.getByLabelText("Slug"), "my-doc");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(screen.getByLabelText("Name"), "My Doc");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(screen.getByLabelText("Content"), "Some content");

    expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled();
  });

  it("editor shows error for invalid slug format", async () => {
    mockDocuments();

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("Initial Plan");
    await user.click(screen.getByRole("button", { name: /New/i }));

    await screen.findByText("New document");

    await user.type(screen.getByLabelText("Slug"), "Invalid Slug!");

    expect(
      await screen.findByText(
        "Slug must start with a-z or 0-9 and contain only a-z, 0-9, _, -",
      ),
    ).toBeInTheDocument();
  });

  it("optimistic pin state reverts on error", async () => {
    mockDocuments({
      task_id: "task-1",
      documents: [
        {
          slug: "api-design",
          name: "API Design",
          kind: "design",
          pinned: false,
          version: 1,
          updated_at: "2026-05-24T10:00:00Z",
          created_at: "2026-05-24T10:00:00Z",
          author_role: "planner",
          relevant_cards: [],
        },
      ],
    });
    let finishPin: (() => void) | undefined;
    const pinRequests: unknown[] = [];
    server.use(
      http.post(
        "*/v1/task/:taskId/documents/:slug/pin",
        async ({ request }) => {
          pinRequests.push(await request.json());
          await new Promise<void>((resolve) => {
            finishPin = resolve;
          });
          return HttpResponse.json({ error: "fail" }, { status: 500 });
        },
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("API Design");

    const pinBtn = screen.getByRole("button", { name: "Pin" });
    await user.click(pinBtn);
    await waitFor(() => expect(pinRequests).toEqual([{ pinned: true }]));
    expect(screen.getByRole("button", { name: "Unpin" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Unpin" }));
    expect(pinRequests).toHaveLength(1);
    finishPin?.();

    expect(
      await screen.findByText(
        /Failed to pin document api-design in task task-1/,
      ),
    ).toBeVisible();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Pin" })).toBeEnabled();
    });
  });

  it("keeps delete confirmation and the document after failure and guards duplicate requests", async () => {
    mockDocuments();
    const deletes: string[] = [];
    let finishDelete: (() => void) | undefined;
    server.use(
      http.delete("*/v1/task/:taskId/documents/:slug", async ({ params }) => {
        deletes.push(String(params.slug));
        await new Promise<void>((resolve) => {
          finishDelete = resolve;
        });
        return HttpResponse.json({ error: "fail" }, { status: 500 });
      }),
    );
    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });
    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "Delete" }));
    expect(deletes).toEqual([]);
    await user.dblClick(
      await screen.findByRole("button", { name: "Confirm delete" }),
    );
    await waitFor(() => expect(deletes).toEqual(["initial-plan"]));
    expect(
      screen.getByRole("button", { name: "Confirm delete" }),
    ).toBeDisabled();
    expect(screen.getByTestId("document-row-initial-plan")).toBeInTheDocument();
    finishDelete?.();
    expect(
      await screen.findByText(
        /Failed to delete document initial-plan in task task-1/,
      ),
    ).toBeVisible();
    expect(screen.getByText("Delete this document?")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Confirm delete" }),
    ).toBeEnabled();
    expect(screen.getByTestId("document-row-initial-plan")).toBeInTheDocument();
  });

  it("shows history and historical content failures with explicit retries", async () => {
    mockDocuments();
    let failHistory = true;
    let failContent = true;
    server.use(
      http.get("*/v1/task/:taskId/documents/:slug/history", () =>
        failHistory
          ? HttpResponse.json({ error: "fail" }, { status: 500 })
          : HttpResponse.json({
              task_id: "task-1",
              slug: "initial-plan",
              history: [
                {
                  version: 1,
                  updated_at: historicalDetailResponse.updated_at,
                  author_role: "planner",
                  size_bytes: 128,
                },
              ],
            }),
      ),
      http.get("*/v1/task/:taskId/documents/:slug", () =>
        failContent
          ? HttpResponse.json({ error: "fail" }, { status: 500 })
          : HttpResponse.json(historicalDetailResponse),
      ),
    );
    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });
    const row = await screen.findByTestId("document-row-initial-plan");
    await user.click(within(row).getByRole("button", { name: "History" }));
    expect(
      await screen.findByText("Failed to load history for initial-plan."),
    ).toBeVisible();
    expect(screen.queryByText("No history available.")).toBeNull();
    failHistory = false;
    await user.click(screen.getByRole("button", { name: "Retry history" }));
    const dialog = screen.getByRole("dialog", {
      name: "History: initial-plan",
    });
    await user.click(
      await within(dialog).findByRole("button", { name: /v1/i }),
    );
    expect(
      await screen.findByText(
        "Failed to load historical content for initial-plan v1.",
      ),
    ).toBeVisible();
    failContent = false;
    await user.click(
      screen.getByRole("button", { name: "Retry historical content" }),
    );
    expect(
      await screen.findByText("This is the historical content."),
    ).toBeVisible();
  });
});
