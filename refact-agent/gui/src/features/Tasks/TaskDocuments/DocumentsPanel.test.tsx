import { describe, expect, it } from "vitest";
import { within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { DocumentsPanel } from "./DocumentsPanel";
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
    http.get("*/v1/task/:taskId/documents", () =>
      HttpResponse.json(response),
    ),
    http.get(
      "*/v1/task/:taskId/documents/:slug",
      ({ request, params }) => {
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
      },
    ),
    http.get(
      "*/v1/task/:taskId/documents/:slug/history",
      ({ params }) =>
        HttpResponse.json({
          task_id: String(params.taskId),
          slug: String(params.slug),
          history: [],
        }),
    ),
  );
}

describe("DocumentsPanel", () => {
  it("renders list of documents with kind badges", async () => {
    mockDocuments();

    render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText("Initial Plan")).toBeInTheDocument();
    expect(screen.getByText("API Design")).toBeInTheDocument();

    const planBadge = screen.getByTestId("kind-badge-initial-plan");
    expect(planBadge).toBeInTheDocument();
    expect(planBadge).toHaveTextContent("plan");

    const designBadge = screen.getByTestId("kind-badge-api-design");
    expect(designBadge).toBeInTheDocument();
    expect(designBadge).toHaveTextContent("design");
  });

  it("clicking a row expands inline content as markdown", async () => {
    mockDocuments();
    server.use(
      http.get(
        "*/v1/task/:taskId/documents/:slug",
        ({ params }) => {
          if (params.slug === "initial-plan") {
            return HttpResponse.json(detailResponse);
          }
          return HttpResponse.json({ status: 404 }, { status: 404 });
        },
      ),
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
      http.post(
        "*/v1/task/:taskId/documents",
        async ({ request }) => {
          const body = await request.json();
          createRequests.push(body);
          return HttpResponse.json({
            ...detailResponse,
            slug: (body as { slug: string }).slug,
            name: (body as { name: string }).name,
          });
        },
      ),
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
      http.get(
        "*/v1/task/:taskId/documents/:slug",
        ({ params }) => {
          if (params.slug === "initial-plan") {
            return HttpResponse.json(detailResponse);
          }
          return HttpResponse.json({ status: 404 }, { status: 404 });
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
  });

  it("delete shows confirm popover then calls delete mutation", async () => {
    const deleteRequests: string[] = [];
    mockDocuments();
    server.use(
      http.delete(
        "*/v1/task/:taskId/documents/:slug",
        ({ params }) => {
          deleteRequests.push(String(params.slug));
          return new HttpResponse(null, { status: 204 });
        },
      ),
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
      http.get(
        "*/v1/task/:taskId/documents/:slug/history",
        ({ params }) => {
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
        },
      ),
      http.get(
        "*/v1/task/:taskId/documents/:slug",
        ({ request, params }) => {
          const version = new URL(request.url).searchParams.get("version");
          if (params.slug === "initial-plan" && version === "1") {
            return HttpResponse.json(historicalDetailResponse);
          }
          return HttpResponse.json({ status: 404 }, { status: 404 });
        },
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
      http.get(
        "*/v1/task/:taskId/documents/:slug/history",
        () =>
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
      http.get(
        "*/v1/task/:taskId/documents/:slug/history",
        () =>
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

  it("empty state shown when no documents", async () => {
    mockDocuments({ task_id: "task-1", documents: [] });

    render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText(/No documents yet/)).toBeInTheDocument();
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

  it("header shows document count", async () => {
    mockDocuments();

    render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText("2 documents")).toBeInTheDocument();
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
      http.post(
        "*/v1/task/:taskId/documents/:slug/pin",
        () =>
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
    server.use(
      http.post(
        "*/v1/task/:taskId/documents/:slug/pin",
        () => HttpResponse.json({ error: "fail" }, { status: 500 }),
      ),
    );

    const { user } = render(<DocumentsPanel taskId="task-1" />, {
      preloadedState: CONFIG_STATE,
    });

    await screen.findByText("API Design");

    const pinBtn = screen.getByRole("button", { name: "Pin" });
    await user.click(pinBtn);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Pin" })).toBeInTheDocument();
    });
  });
});
