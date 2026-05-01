import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { Popover } from "@radix-ui/themes";
import { setUpStore } from "../../app/store";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import type { Chat } from "../Chat/Thread/types";
import type {
  MergeWorktreeResponse,
  WorktreeDiffResponse,
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import type { TaskBoard, TaskMeta } from "../../services/refact/tasks";
import { WorktreeDiffPanel } from "./WorktreeDiffPanel";
import { MergeWorktreeModal } from "./MergeWorktreeModal";
import { WorktreeMenu } from "./WorktreeMenu";
import { TaskWorkspace } from "../Tasks/TaskWorkspace";

type JsonObject = Record<string, unknown>;

type Host = "web" | "ide" | "vscode" | "jetbrains";

function configState(host: Host = "web") {
  return {
    host,
    lspPort: 8001,
    apiKey: null,
    themeProps: { appearance: "dark" as const },
    features: { images: true, statistics: true, vecdb: true, ast: true },
  };
}

function makeMeta(id = "wt-1", branch = "refact/task/T-1"): WorktreeMeta {
  return {
    id,
    kind: "task_agent",
    root: `/tmp/refact/${id}`,
    source_workspace_root: "/repo",
    repo_root: "/repo",
    branch,
    base_branch: "main",
    base_commit: "abc123",
    task_id: "task-1",
    card_id: "T-1",
    enforce: true,
  };
}

function makeRecord(
  id = "wt-1",
  branch = "refact/task/T-1",
  referenceCount = 1,
): WorktreeRecordView {
  const meta = makeMeta(id, branch);
  meta.reference_count = referenceCount;
  return {
    meta,
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    references: Array.from({ length: referenceCount }, (_, index) => ({
      kind: "chat",
      chat_id: index === 0 ? "chat-1" : `chat-${index + 1}`,
    })),
    reference_count: referenceCount,
    referencing_chat_ids: Array.from({ length: referenceCount }, (_, index) =>
      index === 0 ? "chat-1" : `chat-${index + 1}`,
    ),
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty: true,
      staged_count: 1,
      unstaged_count: 1,
      untracked_count: 0,
      branch,
      head_commit: "def456",
    },
  };
}

function makeDiff(id = "wt-1"): WorktreeDiffResponse {
  return {
    id,
    branch: "refact/task/T-1",
    base_branch: "main",
    base_commit: "abc123",
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty: true,
      staged_count: 1,
      unstaged_count: 1,
      untracked_count: 0,
      branch: "refact/task/T-1",
      head_commit: "def456",
    },
    files: [
      {
        path: "src/lib.rs",
        status: "modified",
        source: "committed",
        additions: 2,
        deletions: 1,
      },
    ],
    stats: {
      committed_files: 1,
      staged_files: 0,
      unstaged_files: 1,
      untracked_files: 0,
      files_changed: 1,
    },
    patch: "diff --git a/src/lib.rs b/src/lib.rs\n+new line",
    patch_truncated: true,
  };
}

function makeWorktreeList(records: WorktreeRecordView[]): WorktreeListResponse {
  return {
    project_hash: "project-hash",
    source_workspace_root: "/repo",
    source_current_branch: "main",
    worktrees: records,
  };
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

function worktreesList(records: WorktreeRecordView[]) {
  return http.get("http://127.0.0.1:8001/v1/worktrees", () =>
    HttpResponse.json(makeWorktreeList(records)),
  );
}

function diffHandler(diff = makeDiff()) {
  return http.get("http://127.0.0.1:8001/v1/worktrees/:id/diff", () =>
    HttpResponse.json(diff),
  );
}

function mergeHandler(response: MergeWorktreeResponse, calls: JsonObject[]) {
  return http.post(
    "http://127.0.0.1:8001/v1/worktrees/:id/merge",
    async ({ request }) => {
      calls.push((await request.json()) as JsonObject);
      return HttpResponse.json(response);
    },
  );
}

function taskHandlers(record: WorktreeRecordView) {
  const task: TaskMeta = {
    id: "task-1",
    name: "Task with worktree",
    status: "active",
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    cards_total: 1,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 1,
    base_branch: "main",
  };
  const board: TaskBoard = {
    schema_version: 1,
    rev: 1,
    columns: [
      { id: "planned", title: "Planned" },
      { id: "doing", title: "Doing" },
      { id: "done", title: "Done" },
      { id: "failed", title: "Failed" },
    ],
    cards: [
      {
        id: "T-1",
        title: "Implement worktree",
        column: "doing",
        priority: "P1",
        depends_on: [],
        instructions: "Use a worktree.",
        assignee: "agent-1",
        agent_chat_id: "agent-T-1",
        status_updates: [],
        final_report: null,
        created_at: "2026-04-30T00:00:00Z",
        started_at: "2026-04-30T00:00:00Z",
        completed_at: null,
        agent_branch: record.meta.branch ?? undefined,
        agent_worktree: record.meta.id,
        agent_worktree_name: "agent-wt",
        target_files: [],
      },
    ],
  };
  return [
    http.get("http://127.0.0.1:8001/v1/tasks/task-1", () =>
      HttpResponse.json({ meta: task }),
    ),
    http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () =>
      HttpResponse.json(board),
    ),
    http.get("http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner", () =>
      HttpResponse.json([]),
    ),
    http.get("http://127.0.0.1:8001/v1/ping", () =>
      HttpResponse.json({ status: "ok" }),
    ),
    http.get("http://127.0.0.1:8001/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("http://127.0.0.1:8001/v1/caps", () =>
      HttpResponse.json({ chat_models: [], completion_models: [] }),
    ),
    http.post("http://127.0.0.1:8001/v1/buddy/diagnostics/collect", () =>
      HttpResponse.json({}),
    ),
    worktreesList([record]),
  ];
}

beforeEach(() => {
  server.use(worktreesList([]));
});

describe("Worktree lifecycle GUI", () => {
  test("diff panel renders file list and patch", async () => {
    const record = makeRecord();
    server.use(diffHandler());

    render(
      <WorktreeDiffPanel
        open
        worktreeId={record.meta.id}
        record={record}
        onOpenChange={() => undefined}
      />,
      { preloadedState: { config: configState() } },
    );

    expect(await screen.findByText("src/lib.rs")).toBeInTheDocument();
    expect(screen.getByText("committed · modified")).toBeInTheDocument();
    expect(screen.getByText(/diff --git/)).toBeInTheDocument();
    expect(screen.getByText("+new line")).toBeInTheDocument();
    expect(
      screen.getByText("Patch preview was truncated by the backend."),
    ).toBeInTheDocument();
  });

  test("merge modal success path invalidates task queries and shows summary", async () => {
    const record = makeRecord();
    const mergeCalls: JsonObject[] = [];
    server.use(
      mergeHandler(
        {
          id: record.meta.id,
          status: "merged",
          merged: true,
          strategy: "squash",
          source_branch: record.meta.branch ?? "refact/task/T-1",
          target_branch: "main",
          cleanup: null,
          conflict: null,
          affected_references: [],
          affected_reference_count: 1,
          warnings: [],
        },
        mergeCalls,
      ),
    );
    const store = setUpStore({ config: configState() });
    const dispatchSpy = vi.spyOn(store, "dispatch");

    const { user } = render(
      <MergeWorktreeModal
        open
        worktreeId={record.meta.id}
        record={record}
        taskId="task-1"
        onOpenChange={() => undefined}
      />,
      { store },
    );

    await user.click(screen.getByRole("button", { name: "Merge" }));

    expect(await screen.findByText("Merge completed.")).toBeInTheDocument();
    expect(mergeCalls[0]).toMatchObject({
      strategy: "squash",
      target_branch: "main",
      delete_after_merge: true,
      include_uncommitted: false,
    });
    expect(dispatchSpy).toHaveBeenCalledWith(
      expect.objectContaining({ type: "tasksApi/invalidateTags" }),
    );
  });

  test("merge modal displays structured backend error text", async () => {
    const record = makeRecord();
    server.use(
      http.post("http://127.0.0.1:8001/v1/worktrees/:id/merge", () =>
        HttpResponse.json(
          {
            code: "conflict",
            error:
              "Worktree has uncommitted changes; include them or commit first.",
          },
          { status: 409 },
        ),
      ),
    );

    const { user } = render(
      <MergeWorktreeModal
        open
        worktreeId={record.meta.id}
        record={record}
        onOpenChange={() => undefined}
      />,
      { preloadedState: { config: configState() } },
    );

    await user.click(screen.getByRole("button", { name: "Merge" }));

    expect(
      await screen.findByText(
        "Worktree has uncommitted changes; include them or commit first.",
      ),
    ).toBeInTheDocument();
  });

  test("merge conflict path lists files and exposes Ask Refact action", async () => {
    const record = makeRecord();
    const askRefact = vi.fn();
    server.use(
      mergeHandler(
        {
          id: record.meta.id,
          status: "conflict",
          merged: false,
          strategy: "merge",
          source_branch: record.meta.branch ?? "refact/task/T-1",
          target_branch: "main",
          conflict: {
            files: ["src/conflict.ts"],
            aborted: true,
            merge_in_progress: false,
            instructions: "Resolve conflict markers.",
          },
          affected_references: [],
          affected_reference_count: 1,
          warnings: [],
        },
        [],
      ),
    );

    const { user } = render(
      <MergeWorktreeModal
        open
        worktreeId={record.meta.id}
        record={record}
        onOpenChange={() => undefined}
        onAskRefact={askRefact}
      />,
      { preloadedState: { config: configState() } },
    );

    await user.click(screen.getByRole("button", { name: "Merge" }));

    expect(
      await screen.findByText("Merge conflicts detected."),
    ).toBeInTheDocument();
    expect(screen.getByText("src/conflict.ts")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Ask Refact to resolve conflicts" }),
    );
    expect(askRefact).toHaveBeenCalledWith(
      ["src/conflict.ts"],
      expect.objectContaining({ status: "conflict" }),
    );
  });

  test("menu review row opens diff and merge flows", async () => {
    const record = makeRecord("wt-menu", "refact/task/menu");
    const mergeCalls: JsonObject[] = [];
    server.use(
      diffHandler(makeDiff(record.meta.id)),
      mergeHandler(
        {
          id: record.meta.id,
          status: "merged",
          merged: true,
          strategy: "squash",
          source_branch: record.meta.branch ?? "refact/task/menu",
          target_branch: "main",
          cleanup: null,
          conflict: null,
          affected_references: [],
          affected_reference_count: 1,
          warnings: [],
        },
        mergeCalls,
      ),
    );

    const { user } = render(
      <Popover.Root open>
        <WorktreeMenu
          currentWorktree={record.meta}
          currentRecord={record}
          records={[record]}
          isLoading={false}
          canCopyPath
          onCreate={() => undefined}
          onSelect={() => undefined}
          onDetach={() => undefined}
          onOpenInNewWindow={() => undefined}
          onCopyPath={() => undefined}
        />
      </Popover.Root>,
      {
        preloadedState: {
          config: configState(),
          chat: makeChatState("chat-1", record.meta),
        },
      },
    );

    const diffButton = screen.getByRole("button", {
      name: "View worktree diff",
    });
    expect(diffButton).toHaveTextContent("Diff");
    await user.click(diffButton);
    expect(await screen.findByText("src/lib.rs")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close" }));

    const mergeButton = screen.getByRole("button", { name: "Merge worktree" });
    expect(mergeButton).toHaveTextContent("Merge");
    await user.click(mergeButton);
    await user.click(await screen.findByRole("button", { name: "Merge" }));

    expect(await screen.findByText("Merge completed.")).toBeInTheDocument();
    expect(mergeCalls[0]).toMatchObject({
      strategy: "squash",
      target_branch: "main",
      delete_after_merge: true,
      include_uncommitted: false,
    });
  });

  test("delete confirmation calls endpoint and shows shared-use warning", async () => {
    const record = makeRecord("wt-shared", "refact/task/shared", 2);
    const deleteCalls: string[] = [];
    const onDetach = vi.fn();
    server.use(
      http.delete("http://127.0.0.1:8001/v1/worktrees/:id", ({ params }) => {
        deleteCalls.push(String(params.id));
        return HttpResponse.json({
          deleted: true,
          branch_deleted: false,
          stale_path: false,
          affected_references: record.references,
          affected_reference_count: 2,
          warnings: [],
        });
      }),
    );

    const { user } = render(
      <Popover.Root open>
        <WorktreeMenu
          currentWorktree={record.meta}
          currentRecord={record}
          records={[record]}
          isLoading={false}
          canCopyPath
          onCreate={() => undefined}
          onSelect={() => undefined}
          onDetach={onDetach}
          onOpenInNewWindow={() => undefined}
          onCopyPath={() => undefined}
        />
      </Popover.Root>,
      {
        preloadedState: {
          config: configState(),
          chat: makeChatState("chat-1", record.meta),
        },
      },
    );

    await user.click(
      screen.getByRole("button", { name: "Delete or discard worktree" }),
    );

    expect(
      await screen.findByRole("heading", { name: "Delete worktree" }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(/shared by 2 references/).length,
    ).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "Delete worktree" }));

    await waitFor(() => expect(deleteCalls).toEqual(["wt-shared"]));
    expect(onDetach).toHaveBeenCalled();
  });

  test("task workspace shows worktree badge and card detail actions", async () => {
    const record = makeRecord("wt-task", "refact/task/T-1");
    server.use(...taskHandlers(record));

    const { user } = render(<TaskWorkspace taskId="task-1" />, {
      preloadedState: { config: configState() },
    });

    const worktreeTitles = await screen.findAllByText("Implement worktree");
    expect(worktreeTitles.length).toBeGreaterThan(0);
    expect(screen.getByText("agent-wt")).toBeInTheDocument();

    await user.click(worktreeTitles[0]);

    expect(await screen.findByText("Worktree")).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: "View Diff" }),
    ).not.toHaveLength(0);
    expect(screen.getAllByRole("button", { name: "Merge" })).not.toHaveLength(
      0,
    );
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: "Discard/Delete" }),
    ).not.toHaveLength(0);
  });
});
