import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { screen, waitFor, render } from "../../utils/test-utils";
import type { Chat } from "../Chat/Thread/types";
import type {
  WorktreeDiffResponse,
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import { WorktreeControl } from "./WorktreeControl";
import { server } from "../../utils/mockServer";

type JsonObject = Record<string, unknown>;
type Host = "web" | "ide" | "vscode" | "jetbrains";

function makeWorktreeRecord(
  id: string,
  branch: string,
  referenceCount = 1,
): WorktreeRecordView {
  const meta: WorktreeMeta = {
    id,
    kind: "chat",
    root: `/tmp/refact/${id}`,
    source_workspace_root: "/repo",
    repo_root: "/repo",
    branch,
    base_branch: "main",
    base_commit: "abc123",
    enforce: true,
    reference_count: referenceCount,
  };
  return {
    meta,
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    references: Array.from({ length: referenceCount }, (_, index) => ({
      kind: "chat",
      chat_id: index === 0 ? "other-chat" : `other-chat-${index}`,
    })),
    reference_count: referenceCount,
    referencing_chat_ids: Array.from({ length: referenceCount }, (_, index) =>
      index === 0 ? "other-chat" : `other-chat-${index}`,
    ),
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty: false,
      staged_count: 0,
      unstaged_count: 0,
      untracked_count: 0,
      branch,
      head_commit: "abc123",
    },
  };
}

function makeWorktreeList(
  records: WorktreeRecordView[],
  sourceCurrentBranch: string | null = "dev",
  sourceBranches: string[] = sourceCurrentBranch ? [sourceCurrentBranch] : [],
): WorktreeListResponse {
  return {
    project_hash: "project-hash",
    source_workspace_root: "/repo",
    source_current_branch: sourceCurrentBranch,
    source_branches: sourceBranches,
    worktrees: records,
  };
}

function worktreesList(
  records: WorktreeRecordView[],
  sourceCurrentBranch: string | null = "dev",
  sourceBranches?: string[],
) {
  return http.get("http://127.0.0.1:8001/v1/worktrees", () =>
    HttpResponse.json(
      makeWorktreeList(records, sourceCurrentBranch, sourceBranches),
    ),
  );
}

function worktreeDiff() {
  return http.get(
    "http://127.0.0.1:8001/v1/worktrees/:id/diff",
    ({ params }) => {
      const id = String(params.id);
      const response: WorktreeDiffResponse = {
        id,
        branch: null,
        base_branch: null,
        base_commit: null,
        status: {
          path_exists: true,
          is_git_worktree: true,
          dirty: false,
          staged_count: 0,
          unstaged_count: 0,
          untracked_count: 0,
          branch: null,
          head_commit: null,
        },
        files: [],
        stats: {
          committed_files: 0,
          staged_files: 0,
          unstaged_files: 0,
          untracked_files: 0,
          files_changed: 0,
          additions: 0,
          deletions: 0,
        },
        patch: "",
        patch_truncated: false,
      };
      return HttpResponse.json(response);
    },
  );
}

function chatModes() {
  return http.get("http://127.0.0.1:8001/v1/chat-modes", () =>
    HttpResponse.json({
      modes: [
        {
          id: "agent",
          title: "Agent",
          description: "Agent mode",
          tools_count: 0,
          thread_defaults: {
            include_project_info: true,
            checkpoints_enabled: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
          },
          ui: { order: 1, tags: ["agent"] },
        },
      ],
      errors: [],
    }),
  );
}

function commandCapture(calls: JsonObject[]) {
  return http.post(
    "http://127.0.0.1:8001/v1/chats/:id/commands",
    async ({ request }) => {
      calls.push((await request.json()) as JsonObject);
      return HttpResponse.json({ status: "queued" });
    },
  );
}

function createWorktreeHandler(
  record: WorktreeRecordView,
  calls: JsonObject[],
) {
  return http.post(
    "http://127.0.0.1:8001/v1/worktrees",
    async ({ request }) => {
      calls.push((await request.json()) as JsonObject);
      return HttpResponse.json({
        worktree: record,
        branch_was_created: true,
        dirty_source_warning: false,
        warnings: [],
      });
    },
  );
}

function openWorktreeHandler(
  record: WorktreeRecordView,
  canOpenFolder: boolean,
  calls?: string[],
) {
  return http.post(
    `http://127.0.0.1:8001/v1/worktrees/${record.meta.id}/open`,
    () => {
      calls?.push(record.meta.id);
      return HttpResponse.json({
        id: record.meta.id,
        path: record.meta.root,
        branch: record.meta.branch,
        can_open_folder: canOpenFolder,
      });
    },
  );
}

function deleteWorktreeHandler(calls: string[]) {
  return http.delete("http://127.0.0.1:8001/v1/worktrees/:id", ({ params }) => {
    calls.push(String(params.id));
    return HttpResponse.json({
      deleted: true,
      branch_deleted: false,
      stale_path: false,
      affected_references: [],
      affected_reference_count: 0,
      warnings: [],
    });
  });
}

function makeChatState(chatId: string, worktree?: WorktreeMeta | null): Chat {
  return {
    current_thread_id: chatId,
    open_thread_ids: [chatId],
    threads: {
      [chatId]: {
        thread: {
          id: chatId,
          messages: [],
          title: "",
          model: "",
          tool_use: "agent",
          mode: "agent",
          new_chat_suggested: { wasSuggested: false },
          boost_reasoning: false,
          include_project_info: true,
          auto_enrichment_enabled: true,
          worktree,
        },
        streaming: false,
        waiting_for_response: false,
        prevent_send: false,
        error: null,
        queued_items: [],
        send_immediately: false,
        attached_images: [],
        attached_text_files: [],
        confirmation: {
          pause: false,
          pause_reasons: [],
          status: { wasInteracted: false, confirmationStatus: true },
        },
        snapshot_received: true,
        task_widget_expanded: false,
        memory_enrichment_user_touched: false,
        manual_preview_items: [],
        manual_preview_ran: false,
      },
    },
    system_prompt: {},
    tool_use: "agent",
    checkpoints_enabled: true,
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function configState(host: Host = "web") {
  return {
    host,
    lspPort: 8001,
    apiKey: null,
    themeProps: { appearance: "dark" as const },
    features: { images: true, statistics: true, vecdb: true, ast: true },
  };
}

function renderControl(
  records: WorktreeRecordView[],
  worktree?: WorktreeMeta | null,
  host: Host = "web",
  sourceCurrentBranch: string | null = "dev",
  sourceBranches?: string[],
) {
  server.use(worktreesList(records, sourceCurrentBranch, sourceBranches));
  return render(<WorktreeControl />, {
    preloadedState: {
      chat: makeChatState("chat-1", worktree),
      config: configState(host),
    },
  });
}

beforeEach(() => {
  server.use(worktreesList([]), worktreeDiff(), chatModes());
});

describe("WorktreeControl", () => {
  test("label shows worktree branch", async () => {
    const record = makeWorktreeRecord("wt-branch", "refact/chat/branch");

    renderControl([record], record.meta);

    await waitFor(() => {
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "refact/chat/branch",
      );
    });
  });

  test("no-worktree label shows current source branch", async () => {
    renderControl([]);

    await waitFor(() => {
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "dev",
      );
    });
  });

  test("no-worktree label shows no branch without source branch", async () => {
    renderControl([], null, "web", null);

    await waitFor(() => {
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "No branch",
      );
    });
  });

  test("menu renders compact actions and disables current-worktree actions on main", async () => {
    const { user } = renderControl([]);

    await user.click(screen.getByTestId("worktree-control-trigger"));

    const create = await screen.findByRole("button", {
      name: "Create worktree",
    });
    expect(create).toHaveTextContent("Create");
    expect(create).toHaveAttribute("title", "Create worktree");

    const main = screen.getByRole("button", {
      name: "Already using main workspace",
    });
    expect(main).toHaveTextContent("Main");
    expect(main).toBeDisabled();

    const open = screen.getByRole("button", {
      name: "Open worktree in new window",
    });
    expect(open).toHaveTextContent("Open");
    expect(open).toBeDisabled();

    const copy = screen.getByRole("button", { name: "Copy workspace path" });
    expect(copy).toHaveTextContent("Copy");
    await waitFor(() => expect(copy).toBeEnabled());

    const diff = screen.getByRole("button", { name: "View worktree diff" });
    expect(diff).toHaveTextContent("Diff");
    expect(diff).toBeDisabled();

    const merge = screen.getByRole("button", { name: "Merge worktree" });
    expect(merge).toHaveTextContent("Merge");
    expect(merge).toBeDisabled();

    const deleteAction = screen.getByRole("button", {
      name: "Delete or discard worktree",
    });
    expect(deleteAction).toHaveTextContent("Delete");
    expect(deleteAction).toBeDisabled();
    expect(screen.getByText("Existing")).toBeInTheDocument();
    expect(screen.getByText("None yet")).toBeInTheDocument();
  });

  test("create modal submits API call and attaches worktree", async () => {
    const created = makeWorktreeRecord("wt-new", "refact/chat/new");
    const createCalls: JsonObject[] = [];
    const commandCalls: JsonObject[] = [];
    server.use(
      worktreesList([]),
      createWorktreeHandler(created, createCalls),
      commandCapture(commandCalls),
    );

    const { user } = renderControl([]);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Create worktree",
      }),
    );
    const branchInput = await screen.findByLabelText(/Branch name/);
    await user.clear(branchInput);
    await user.type(branchInput, "refact/chat/new");
    await user.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() => expect(createCalls).toHaveLength(1));
    expect(createCalls[0]).toMatchObject({
      branch: "refact/chat/new",
      base_branch: "dev",
      kind: "chat",
    });
    expect(createCalls[0]).not.toHaveProperty("chat_id");
    await waitFor(() => expect(commandCalls).toHaveLength(1));
    expect(commandCalls[0]).toMatchObject({
      type: "set_params",
      patch: { worktree_id: "wt-new" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "refact/chat/new",
      );
    });
  });

  test("create modal requires and sends selected base branch", async () => {
    const created = makeWorktreeRecord("wt-explicit-base", "refact/chat/base");
    const createCalls: JsonObject[] = [];
    server.use(
      worktreesList([], "dev", ["dev", "feature/base"]),
      createWorktreeHandler(created, createCalls),
      commandCapture([]),
    );

    const { user } = renderControl([], null, "web", "dev", [
      "dev",
      "feature/base",
    ]);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Create worktree",
      }),
    );
    const baseBranchInput = screen.getByLabelText("Base branch");
    await user.click(baseBranchInput);
    await user.click(screen.getByRole("button", { name: "feature/base" }));
    await user.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() => expect(createCalls).toHaveLength(1));
    expect(createCalls[0]).toMatchObject({
      base_branch: "feature/base",
      kind: "chat",
    });
  });

  test("create attach failure deletes unreferenced orphan and rolls back", async () => {
    const created = makeWorktreeRecord("wt-orphan", "refact/chat/orphan", 0);
    const createCalls: JsonObject[] = [];
    const deleteCalls: string[] = [];
    server.use(
      worktreesList([]),
      createWorktreeHandler(created, createCalls),
      deleteWorktreeHandler(deleteCalls),
      http.post("http://127.0.0.1:8001/v1/chats/:id/commands", () =>
        HttpResponse.json(
          { code: "bad_request", error: "attach failed" },
          { status: 400 },
        ),
      ),
    );

    const { user } = renderControl([]);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Create worktree",
      }),
    );
    const branchInput = await screen.findByLabelText(/Branch name/);
    await user.clear(branchInput);
    await user.type(branchInput, "refact/chat/orphan");
    await user.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() => expect(createCalls).toHaveLength(1));
    expect(createCalls[0]).not.toHaveProperty("chat_id");
    await waitFor(() => expect(deleteCalls).toEqual(["wt-orphan"]));
    expect(
      await screen.findByText(
        "Worktree attach failed; created worktree was deleted.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "dev",
    );
  });

  test("selecting existing shared worktree attaches it to current chat", async () => {
    const shared = makeWorktreeRecord("wt-shared", "refact/chat/shared", 2);
    const commandCalls: JsonObject[] = [];
    server.use(worktreesList([shared]), commandCapture(commandCalls));

    const { user } = renderControl([shared]);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: /Select worktree refact\/chat\/shared/,
      }),
    );

    await waitFor(() => expect(commandCalls).toHaveLength(1));
    expect(commandCalls[0]).toMatchObject({
      type: "set_params",
      patch: { worktree_id: "wt-shared" },
    });
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/shared",
    );
  });

  test("attach failure displays structured backend error and rolls back label", async () => {
    const record = makeWorktreeRecord("wt-error", "refact/chat/error");
    server.use(
      worktreesList([record]),
      http.post("http://127.0.0.1:8001/v1/chats/:id/commands", () =>
        HttpResponse.json(
          { code: "bad_request", error: "Worktree 'wt-error' not found" },
          { status: 400 },
        ),
      ),
    );

    const { user } = renderControl([record]);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: /Select worktree refact\/chat\/error/,
      }),
    );

    expect(
      await screen.findByText(
        "Worktree update failed: Worktree 'wt-error' not found",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "dev",
    );
  });

  test("open-in-new-window falls back to copied path when host cannot open folders", async () => {
    const record = makeWorktreeRecord("wt-open", "refact/chat/open");
    const openCalls: string[] = [];
    server.use(
      worktreesList([record]),
      openWorktreeHandler(record, true, openCalls),
    );

    const { user } = renderControl([record], record.meta, "web");

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Open worktree in new window",
      }),
    );

    await waitFor(() => expect(openCalls).toEqual(["wt-open"]));
    expect(
      await screen.findByText("Path copied to clipboard."),
    ).toBeInTheDocument();
  });

  test("detach clears worktree without delete call", async () => {
    const record = makeWorktreeRecord("wt-detach", "refact/chat/detach");
    const commandCalls: JsonObject[] = [];
    const deleteCalls: string[] = [];
    server.use(
      worktreesList([record]),
      commandCapture(commandCalls),
      deleteWorktreeHandler(deleteCalls),
    );

    const { user } = renderControl([record], record.meta);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Detach worktree and use main workspace",
      }),
    );

    await waitFor(() => expect(commandCalls).toHaveLength(1));
    expect(commandCalls[0]).toMatchObject({
      type: "set_params",
      patch: { worktree: null },
    });
    expect(deleteCalls).toHaveLength(0);
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "dev",
    );
  });
});
