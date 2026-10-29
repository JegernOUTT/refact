import { beforeEach, describe, expect, test, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, render } from "../../utils/test-utils";
import type { Chat } from "../Chat/Thread/types";
import { ChatThreadProvider, selectThreadWorktreeById } from "../Chat/Thread";
import { worktreesApi } from "../../services/refact/worktrees";
import type {
  WorktreeDiffResponse,
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import { WorktreeControl } from "./WorktreeControl";
import { server } from "../../utils/mockServer";
import { resetChatParamsSyncState } from "../../services/refact/chatCommands";

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
  requests?: string[],
) {
  return http.get("*/v1/worktrees", ({ request }) => {
    const includeBranches = new URL(request.url).searchParams.get(
      "include_branches",
    );
    requests?.push(includeBranches ?? "default");
    const list = makeWorktreeList(records, sourceCurrentBranch, sourceBranches);
    return HttpResponse.json(
      includeBranches === "false" ? { ...list, source_branches: [] } : list,
    );
  });
}

function worktreeDiff() {
  return http.get("*/v1/worktrees/:id/diff", ({ params }) => {
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
  });
}

function chatModes() {
  return http.get("*/v1/chat-modes", () =>
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
  return http.post("*/v1/chats/:id/commands", async ({ request }) => {
    calls.push((await request.json()) as JsonObject);
    return HttpResponse.json({ status: "queued" });
  });
}

function commandCaptureWithChatIds(calls: JsonObject[]) {
  return http.post("*/v1/chats/:id/commands", async ({ params, request }) => {
    calls.push({
      ...((await request.json()) as JsonObject),
      route_chat_id: String(params.id),
    });
    return HttpResponse.json({ status: "queued" });
  });
}

function createWorktreeHandler(
  record: WorktreeRecordView,
  calls: JsonObject[],
) {
  return http.post("*/v1/worktrees", async ({ request }) => {
    calls.push((await request.json()) as JsonObject);
    return HttpResponse.json({
      worktree: record,
      branch_was_created: true,
      dirty_source_warning: false,
      warnings: [],
    });
  });
}

function openWorktreeHandler(
  record: WorktreeRecordView,
  canOpenFolder: boolean,
  calls?: string[],
) {
  return http.post(`*/v1/worktrees/${record.meta.id}/open`, () => {
    calls?.push(record.meta.id);
    return HttpResponse.json({
      id: record.meta.id,
      path: record.meta.root,
      branch: record.meta.branch,
      can_open_folder: canOpenFolder,
    });
  });
}

function deleteWorktreeHandler(calls: string[]) {
  return http.delete("*/v1/worktrees/:id", ({ params }) => {
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
        background_agents: {},
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
  resetChatParamsSyncState();
  server.use(worktreesList([]), worktreeDiff(), chatModes());
});

describe("WorktreeControl", () => {
  test("fetches the list once without branches and loads branches only for create", async () => {
    const requests: string[] = [];
    server.use(worktreesList([], "dev", ["dev", "feature/base"], requests));
    const { user, rerender } = render(<WorktreeControl />, {
      preloadedState: {
        chat: makeChatState("chat-1", null),
        config: configState("web"),
      },
    });

    await waitFor(() => expect(requests).toEqual(["false"]));
    rerender(<WorktreeControl />);
    await user.click(screen.getByTestId("worktree-control-trigger"));
    await waitFor(() => expect(requests).toEqual(["false", "false"]));
    await user.click(
      await screen.findByRole("button", { name: "Create worktree" }),
    );
    await waitFor(() => expect(requests).toEqual(["false", "false", "true"]));
    await user.click(screen.getByLabelText("Base branch"));
    expect(
      await screen.findByRole("button", { name: "feature/base" }),
    ).toBeInTheDocument();
  });

  test("empty, stale, and failed inventory retain attached scope without commands", async () => {
    const record = makeWorktreeRecord("wt-branch", "refact/chat/branch");
    const other = makeWorktreeRecord("wt-other", "refact/chat/other");
    const commandCalls: JsonObject[] = [];
    server.use(commandCapture(commandCalls));
    const { store, user } = renderControl([], record.meta);

    await act(async () => {
      await store
        .dispatch(
          worktreesApi.endpoints.listWorktrees.initiate(undefined, {
            subscribe: false,
          }),
        )
        .unwrap();
    });
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/branch",
    );
    expect(commandCalls).toHaveLength(0);

    server.use(worktreesList([other]));
    await act(async () => {
      await store
        .dispatch(
          worktreesApi.endpoints.listWorktrees.initiate(undefined, {
            subscribe: false,
            forceRefetch: true,
          }),
        )
        .unwrap();
    });
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    expect(commandCalls).toHaveLength(0);
    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(
      await screen.findByText(
        "Worktree inventory is unavailable or incomplete; scope unchanged.",
      ),
    ).toBeInTheDocument();
    await act(async () => {
      await store
        .dispatch(
          worktreesApi.endpoints.listWorktrees.initiate(undefined, {
            subscribe: false,
          }),
        )
        .unwrap();
    });
    await user.keyboard("{Escape}");

    server.use(
      http.get("*/v1/worktrees", () =>
        HttpResponse.json({ error: "Inventory unavailable" }, { status: 503 }),
      ),
    );
    await act(async () => {
      const result = await store.dispatch(
        worktreesApi.endpoints.listWorktrees.initiate(undefined, {
          subscribe: false,
          forceRefetch: true,
        }),
      );
      expect(result).toMatchObject({ error: { status: 503 } });
    });
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/branch",
    );
    expect(commandCalls).toHaveLength(0);
  });

  test("pane providers retain managed worktree labels and refuse scope changes before transport", async () => {
    const global = makeWorktreeRecord("wt-global", "refact/chat/global");
    global.meta.kind = "task_agent";
    const left = makeWorktreeRecord("wt-left", "refact/chat/left");
    const right = makeWorktreeRecord("wt-right", "refact/chat/right");
    const chat = makeChatState("global-chat", global.meta);
    const globalRuntime = chat.threads["global-chat"];
    if (!globalRuntime) throw new Error("global test thread missing");
    chat.open_thread_ids = ["global-chat", "left-chat", "right-chat"];
    chat.threads["left-chat"] = {
      ...globalRuntime,
      thread: {
        ...globalRuntime.thread,
        id: "left-chat",
        worktree: left.meta,
        task_meta: { task_id: "task-1", role: "agent" },
      },
    };
    chat.threads["right-chat"] = {
      ...globalRuntime,
      thread: {
        ...globalRuntime.thread,
        id: "right-chat",
        worktree: right.meta,
        task_meta: { task_id: "task-1", role: "agents" },
      },
    };
    const commandCalls: JsonObject[] = [];
    const createCalls: JsonObject[] = [];
    const deleteCalls: string[] = [];
    server.use(
      worktreesList([]),
      commandCapture(commandCalls),
      createWorktreeHandler(global, createCalls),
      deleteWorktreeHandler(deleteCalls),
    );

    const { user, store } = render(
      <>
        <ChatThreadProvider chatId="left-chat">
          <WorktreeControl />
        </ChatThreadProvider>
        <ChatThreadProvider chatId="right-chat">
          <WorktreeControl />
        </ChatThreadProvider>
        <WorktreeControl />
      </>,
      {
        preloadedState: {
          chat,
          config: configState(),
        },
      },
    );

    await act(async () => {
      await store
        .dispatch(
          worktreesApi.endpoints.listWorktrees.initiate(undefined, {
            subscribe: false,
          }),
        )
        .unwrap();
    });
    const triggers = screen.getAllByTestId("worktree-control-trigger");
    expect(triggers).toHaveLength(3);
    expect(triggers[0]).toHaveTextContent("refact/chat/left");
    expect(triggers[1]).toHaveTextContent("refact/chat/right");
    expect(triggers[2]).toHaveTextContent("refact/chat/global");
    expect(triggers[0]).not.toHaveTextContent("refact/chat/global");
    expect(triggers[1]).not.toHaveTextContent("refact/chat/global");
    for (const trigger of triggers) {
      expect(trigger).toBeDisabled();
      expect(trigger).toHaveAttribute(
        "title",
        expect.stringContaining("Task-managed scope"),
      );
      await user.click(trigger);
      fireEvent.click(trigger);
    }
    expect(
      screen.queryByRole("button", { name: "Create worktree" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: "Detach worktree and use main workspace",
      }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Select worktree/ }),
    ).not.toBeInTheDocument();
    expect(selectThreadWorktreeById(store.getState(), "left-chat")).toEqual(
      left.meta,
    );
    expect(selectThreadWorktreeById(store.getState(), "right-chat")).toEqual(
      right.meta,
    );
    expect(selectThreadWorktreeById(store.getState(), "global-chat")).toEqual(
      global.meta,
    );
    expect(commandCalls).toHaveLength(0);
    expect(createCalls).toHaveLength(0);
    expect(deleteCalls).toHaveLength(0);
  });

  test("pane provider attaches selected worktree to pane thread", async () => {
    const global = makeWorktreeRecord("wt-global", "refact/chat/global");
    const paneTarget = makeWorktreeRecord("wt-pane", "refact/chat/pane");
    const chat = makeChatState("global-chat", global.meta);
    const globalRuntime = chat.threads["global-chat"];
    if (!globalRuntime) throw new Error("global test thread missing");
    chat.open_thread_ids = ["global-chat", "pane-chat"];
    chat.threads["pane-chat"] = {
      ...globalRuntime,
      thread: {
        ...globalRuntime.thread,
        id: "pane-chat",
        worktree: null,
      },
    };
    const commandCalls: JsonObject[] = [];
    server.use(
      worktreesList([global, paneTarget]),
      commandCaptureWithChatIds(commandCalls),
    );

    const { user } = render(
      <ChatThreadProvider chatId="pane-chat">
        <WorktreeControl />
      </ChatThreadProvider>,
      {
        preloadedState: {
          chat,
          config: configState(),
        },
      },
    );

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: /Select worktree refact\/chat\/pane/,
      }),
    );

    await waitFor(() => expect(commandCalls).toHaveLength(1));
    expect(commandCalls[0]).toMatchObject({
      route_chat_id: "pane-chat",
      type: "set_params",
      patch: { worktree_id: "wt-pane" },
    });
    expect(commandCalls[0]?.route_chat_id).not.toBe("global-chat");
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

  test("create modal fields stay constrained to the dialog content column", async () => {
    const css = await readFile(
      path.resolve(__dirname, "Worktrees.module.css"),
      "utf8",
    );
    const dialogInner =
      css.match(/\.dialogContentWrapper \{[^}]+\}/)?.[0] ?? "";
    const branchPicker =
      css.match(/\.branchPicker,\n\.branchPicker input \{[^}]+\}/)?.[0] ?? "";

    expect(dialogInner).toContain("max-width: 100%;");
    expect(dialogInner).toContain("min-width: 0;");
    expect(branchPicker).toContain("width: 100%;");
    expect(branchPicker).toContain("max-width: 100%;");
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

  test("create stays busy through attachment, blocks duplicates, and closes after attach", async () => {
    const created = makeWorktreeRecord("wt-slow", "refact/chat/slow");
    const createCalls: JsonObject[] = [];
    const commandCalls: JsonObject[] = [];
    let releaseAttach: () => void = () => undefined;
    const attachGate = new Promise<void>((resolve) => {
      releaseAttach = resolve;
    });
    const onOpenChange = vi.fn();
    server.use(
      worktreesList([]),
      createWorktreeHandler(created, createCalls),
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandCalls.push((await request.json()) as JsonObject);
        await attachGate;
        return HttpResponse.json({ status: "queued" });
      }),
    );

    const { user } = render(<WorktreeControl onOpenChange={onOpenChange} />, {
      preloadedState: {
        chat: makeChatState("chat-1"),
        config: configState(),
      },
    });

    try {
      await user.click(screen.getByTestId("worktree-control-trigger"));
      await user.click(
        await screen.findByRole("button", { name: "Create worktree" }),
      );
      const branchInput = await screen.findByLabelText(/Branch name/);
      await user.clear(branchInput);
      await user.type(branchInput, "refact/chat/slow");
      await user.click(screen.getByRole("button", { name: /^Create$/ }));

      await waitFor(() => expect(commandCalls).toHaveLength(1));
      const busyButton = screen.getByRole("button", {
        name: /^(Create|Creating\.\.\.)$/,
      });
      expect(busyButton).toBeDisabled();
      await user.click(busyButton);
      expect(createCalls).toHaveLength(1);
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      onOpenChange.mockClear();
    } finally {
      releaseAttach();
    }

    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(
      screen.queryByRole("button", { name: "Create worktree" }),
    ).not.toBeInTheDocument();
    expect(createCalls).toHaveLength(1);
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/slow",
    );
  });

  test("successful select and detach notify parent menu close", async () => {
    const record = makeWorktreeRecord("wt-notify", "refact/chat/notify");
    const onOpenChange = vi.fn();
    server.use(worktreesList([record]), commandCapture([]));

    const { user } = render(<WorktreeControl onOpenChange={onOpenChange} />, {
      preloadedState: {
        chat: makeChatState("chat-1"),
        config: configState(),
      },
    });

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: /Select worktree refact\/chat\/notify/,
      }),
    );
    await waitFor(() => expect(onOpenChange).toHaveBeenLastCalledWith(false));

    onOpenChange.mockClear();
    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
    await user.click(
      await screen.findByRole("button", {
        name: "Detach worktree and use main workspace",
      }),
    );
    await waitFor(() => expect(onOpenChange).toHaveBeenLastCalledWith(false));
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
      http.post("*/v1/chats/:id/commands", () =>
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
      http.post("*/v1/chats/:id/commands", () =>
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

    const { user, store } = renderControl([record], record.meta);

    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(commandCalls).toHaveLength(0);
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
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toBeNull();
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "dev",
    );
  });

  test("merge cleanup warnings remain inspectable after the registry and chat detach", async () => {
    const record = makeWorktreeRecord("wt-merged", "refact/chat/merged");
    const mergeCalls: JsonObject[] = [];
    const commandCalls: JsonObject[] = [];
    let removed = false;
    let removedInventoryReads = 0;
    const { user } = renderControl([record], record.meta);
    server.use(
      http.get("*/v1/worktrees", () => {
        if (removed) removedInventoryReads += 1;
        return HttpResponse.json(makeWorktreeList(removed ? [] : [record]));
      }),
      commandCapture(commandCalls),
      http.post("*/v1/worktrees/:id/merge", async ({ request }) => {
        mergeCalls.push((await request.json()) as JsonObject);
        removed = true;
        return HttpResponse.json({
          id: record.meta.id,
          status: "merged",
          merged: true,
          cleanup: {
            worktree_deleted: true,
            branch_deleted: false,
            registry_deleted: true,
            stale_path: false,
            warnings: ["Branch deletion failed; branch retained."],
          },
          warnings: ["Merge completed with a source warning."],
        });
      }),
    );

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", { name: "Merge worktree" }),
    );
    await user.click(screen.getByRole("button", { name: "Merge" }));

    await waitFor(() => {
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "dev",
      );
      expect(commandCalls.length).toBeGreaterThan(0);
      expect(removedInventoryReads).toBeGreaterThan(0);
    });
    expect(
      screen.getByRole("dialog", { name: "Merge worktree" }),
    ).toHaveTextContent("Merge refact/chat/merged into a target branch.");
    expect(screen.getByText("Merge completed.")).toBeInTheDocument();
    expect(
      screen.getByText("Merge completed with a source warning."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Cleanup: worktree deleted, branch kept, registry deleted",
      ),
    ).toBeInTheDocument();
    const warning = screen.getByText(
      "Branch deletion failed; branch retained.",
    );
    expect(
      fireEvent.pointerDown(warning, { button: 0, cancelable: true }),
    ).toBe(true);
    await user.pointer([
      { target: warning, offset: 0, keys: "[MouseLeft>]" },
      { target: warning, offset: warning.textContent?.length },
      { keys: "[/MouseLeft]" },
    ]);
    expect(window.getSelection()?.toString()).toBe(warning.textContent);
    expect(
      screen.getByRole("dialog", { name: "Merge worktree" }),
    ).toBeInTheDocument();
    const merge = screen.getByRole("button", { name: "Merge" });
    expect(merge).toBeDisabled();
    await user.click(merge);
    expect(mergeCalls).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: "Merge worktree" }),
      ).not.toBeInTheDocument();
    });
  });

  test("nothing-to-merge partial cleanup keeps the retained worktree outcome visible", async () => {
    const record = makeWorktreeRecord("wt-noop", "refact/chat/noop");
    const commandCalls: JsonObject[] = [];
    const { user } = renderControl([record], record.meta);
    server.use(
      commandCapture(commandCalls),
      http.post("*/v1/worktrees/:id/merge", () =>
        HttpResponse.json({
          id: record.meta.id,
          status: "nothing_to_merge",
          merged: false,
          cleanup: {
            worktree_deleted: false,
            branch_deleted: true,
            registry_deleted: false,
            stale_path: false,
            warnings: ["Worktree cleanup failed; worktree retained."],
          },
          warnings: ["No new commits were found."],
        }),
      ),
    );

    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", { name: "Merge worktree" }),
    );
    await user.click(screen.getByRole("button", { name: "Merge" }));

    expect(await screen.findByText("Nothing to merge.")).toBeInTheDocument();
    expect(
      screen.getByText("Cleanup: worktree kept, branch deleted, registry kept"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Worktree cleanup failed; worktree retained."),
    ).toBeInTheDocument();
    expect(screen.getByText("No new commits were found.")).toBeInTheDocument();
    expect(
      screen.queryByText("Worktree merged and removed."),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Merge" })).toBeEnabled();
    expect(commandCalls).toHaveLength(0);
    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: "Merge worktree" }),
      ).not.toBeInTheDocument();
    });
  });
});
