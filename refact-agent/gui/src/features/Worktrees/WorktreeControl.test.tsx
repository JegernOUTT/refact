import { beforeEach, describe, expect, test, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, render } from "../../utils/test-utils";
import type { Chat } from "../Chat/Thread/types";
import { ChatThreadProvider, selectThreadWorktreeById } from "../Chat/Thread";
import { worktreesApi } from "../../services/refact/worktrees";
import { updateConfig } from "../Config/configSlice";
import type {
  WorktreeDiffResponse,
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import { WorktreeControl } from "./WorktreeControl";
import { server } from "../../utils/mockServer";
import { resetChatParamsSyncState } from "../../services/refact/chatCommands";
import * as chatCommands from "../../services/refact/chatCommands";
import { setCurrentProjectInfo } from "../Chat/currentProject";
import {
  getProjectStorageNamespace,
  savePersistedChatTabs,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../../utils/chatUiPersistence";

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
    return HttpResponse.json({ status: "applied" });
  });
}

function commandCaptureWithChatIds(calls: JsonObject[]) {
  return http.post("*/v1/chats/:id/commands", async ({ params, request }) => {
    calls.push({
      ...((await request.json()) as JsonObject),
      route_chat_id: String(params.id),
    });
    return HttpResponse.json({ status: "applied" });
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
        "Attached checkout is not in this list. It may be stale or incomplete; attachment unchanged.",
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

  test("an attached worktree missing from the inventory keeps its diff, merge and delete actions", async () => {
    const record = makeWorktreeRecord("wt-unlisted", "refact/chat/unlisted");
    server.use(
      http.get("*/v1/worktrees", () => HttpResponse.json(makeWorktreeList([]))),
    );
    const { user } = render(<WorktreeControl />, {
      preloadedState: {
        chat: makeChatState("chat-1", record.meta),
        config: configState(),
      },
    });

    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(
      await screen.findByText(/Attached checkout is not in this list/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "View worktree diff" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Merge worktree" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Delete or discard worktree" }),
    ).toBeInTheDocument();
  });

  test("empty, mismatched and failed inventory refreshes never detach an attached checkout", async () => {
    const record = makeWorktreeRecord("wt-retained", "refact/chat/retained");
    const commandCalls: JsonObject[] = [];
    let list = makeWorktreeList([]);
    let fail = false;
    const listSources: string[] = [];
    server.use(
      http.get("*/v1/worktrees", () => {
        listSources.push(list.source_workspace_root);
        return fail
          ? HttpResponse.json(
              { error: "inventory unavailable" },
              { status: 503 },
            )
          : HttpResponse.json(list);
      }),
      commandCapture(commandCalls),
    );
    const { user, store } = render(<WorktreeControl />, {
      preloadedState: {
        chat: makeChatState("chat-1", record.meta),
        config: configState(),
      },
    });

    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(
      await screen.findByText(/Attached checkout is not in this list/),
    ).toBeInTheDocument();
    list = {
      ...list,
      source_workspace_root: "/other-repo",
      source_current_branch: "other-main",
    };
    // This test intentionally keeps the attached chat open in the destination
    // project. Seed its tabs before the real namespace hydration runs.
    const previousNamespace = getProjectStorageNamespace();
    setProjectStorageNamespaceFromProjectInfo({ projectRoot: "/other-repo" });
    savePersistedChatTabs({
      openThreadIds: ["chat-1"],
      currentThreadId: "chat-1",
      tabs: [{ id: "chat-1", mode: "agent" }],
    });
    setProjectStorageNamespace(previousNamespace ?? undefined);
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({
          name: "other-project",
          projectRoot: "/other-repo",
        }),
      );
    });
    expect(store.getState().current_project.projectRoot).toBe("/other-repo");
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    await user.click(screen.getByTestId("worktree-control-trigger"));
    const refresh = screen.getByRole("button", {
      name: "Refresh worktree list",
    });
    await waitFor(() => expect(refresh).toBeEnabled());
    await user.click(refresh);
    await waitFor(() => expect(listSources).toContain("/other-repo"));
    await waitFor(() => expect(refresh).toBeEnabled());
    fail = true;
    await user.click(refresh);
    expect(
      await screen.findByText(
        "Worktree list unavailable. Attached checkout unchanged.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/retained",
    );
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    expect(commandCalls).toEqual([]);
    expect(screen.queryByText("Using main workspace.")).not.toBeInTheDocument();
  });

  test.each(["agent", "agents", "worktree-kind"])(
    "managed task scope %s retains pane identity and refuses all scope controls",
    async (binding) => {
      const record = makeWorktreeRecord("wt-managed", "refact/task/managed");
      const target = makeWorktreeRecord(
        "wt-replacement",
        "refact/chat/replacement",
      );
      const global = makeWorktreeRecord("wt-global", "refact/chat/global");
      if (binding === "worktree-kind") record.meta.kind = "task_agent";
      const chat = makeChatState("pane-chat", record.meta);
      const runtime = chat.threads["pane-chat"];
      if (!runtime) throw new Error("pane test thread missing");
      if (binding !== "worktree-kind") {
        runtime.thread.task_meta = {
          task_id: "task-1",
          card_id: "T-1",
          role: binding,
        };
      }
      chat.threads["global-chat"] = makeChatState(
        "global-chat",
        global.meta,
      ).threads["global-chat"];
      chat.current_thread_id = "global-chat";
      const commandCalls: JsonObject[] = [];
      const createCalls: JsonObject[] = [];
      const deleteCalls: string[] = [];
      server.use(
        worktreesList([record, target, global]),
        commandCapture(commandCalls),
        createWorktreeHandler(target, createCalls),
        deleteWorktreeHandler(deleteCalls),
      );
      const { user, store } = render(
        <ChatThreadProvider chatId="pane-chat">
          <WorktreeControl />
        </ChatThreadProvider>,
        { preloadedState: { chat, config: configState() } },
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
      const trigger = screen.getByTestId("worktree-control-trigger");
      expect(trigger).toHaveTextContent("refact/task/managed");
      expect(trigger).not.toHaveTextContent("refact/chat/global");
      expect(trigger).toBeDisabled();
      expect(trigger).toHaveAttribute(
        "title",
        expect.stringContaining("Task-managed scope"),
      );
      await user.click(trigger);
      fireEvent.click(trigger);
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
      expect(
        screen.queryByRole("button", { name: "Merge worktree" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Delete or discard worktree" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("heading", { name: "Create worktree" }),
      ).not.toBeInTheDocument();
      expect(selectThreadWorktreeById(store.getState(), "pane-chat")).toEqual(
        record.meta,
      );
      expect(selectThreadWorktreeById(store.getState(), "global-chat")).toEqual(
        global.meta,
      );
      expect(commandCalls).toEqual([]);
      expect(createCalls).toEqual([]);
      expect(deleteCalls).toEqual([]);
    },
  );

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
        return HttpResponse.json({ status: "applied" });
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

  test("unconfirmed attachment retains the created worktree instead of deleting it", async () => {
    const created = makeWorktreeRecord(
      "wt-uncertain",
      "refact/chat/uncertain",
      0,
    );
    const createCalls: JsonObject[] = [];
    const deleteCalls: string[] = [];
    server.use(
      createWorktreeHandler(created, createCalls),
      deleteWorktreeHandler(deleteCalls),
      http.post("*/v1/chats/:id/commands", () => HttpResponse.error()),
    );
    const { user, store } = renderControl([]);
    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", { name: "Create worktree" }),
    );
    await user.click(screen.getByRole("button", { name: /^Create$/ }));
    expect(
      await screen.findByText(
        /Attachment not confirmed; created worktree retained \(wt-uncertain\)/,
      ),
    ).toBeInTheDocument();
    expect(createCalls).toHaveLength(1);
    expect(deleteCalls).toEqual([]);
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toBeNull();
  });

  test.each(["pane", "source"] as const)(
    "late applied attachment cannot mutate or report success after a %s change",
    async (change) => {
      const first = makeWorktreeRecord("wt-first", "refact/chat/first");
      const second = makeWorktreeRecord("wt-second", "refact/chat/second");
      const target = makeWorktreeRecord("wt-late", "refact/chat/late");
      const chat = makeChatState("first-chat", first.meta);
      chat.threads["second-chat"] = makeChatState(
        "second-chat",
        second.meta,
      ).threads["second-chat"];
      const commandCalls: JsonObject[] = [];
      const sendCommand = vi.spyOn(chatCommands, "sendChatCommand");
      const onOpenChange = vi.fn();
      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      server.use(
        worktreesList([first, second, target]),
        http.post("*/v1/chats/:id/commands", async ({ request }) => {
          commandCalls.push((await request.json()) as JsonObject);
          await gate;
          return HttpResponse.json({ status: "applied" });
        }),
      );
      const { user, store, rerender } = render(
        <ChatThreadProvider chatId="first-chat">
          <WorktreeControl onOpenChange={onOpenChange} />
        </ChatThreadProvider>,
        { preloadedState: { chat, config: configState() } },
      );
      try {
        await user.click(screen.getByTestId("worktree-control-trigger"));
        await user.click(
          await screen.findByRole("button", {
            name: "Select worktree refact/chat/late",
          }),
        );
        await waitFor(() => expect(commandCalls).toHaveLength(1));
        expect(commandCalls[0]).toMatchObject({
          type: "set_params",
          patch: { worktree_id: "wt-late" },
        });
        expect(
          selectThreadWorktreeById(store.getState(), "first-chat"),
        ).toEqual(first.meta);
        expect(
          selectThreadWorktreeById(store.getState(), "second-chat"),
        ).toEqual(second.meta);
        if (change === "source") {
          // Only the destination pane is persisted in the new source. Hydration
          // must remove the old chat before its pending command is acknowledged.
          const previousNamespace = getProjectStorageNamespace();
          setProjectStorageNamespaceFromProjectInfo({
            projectRoot: "/new-source",
          });
          savePersistedChatTabs({
            openThreadIds: ["second-chat"],
            currentThreadId: "second-chat",
            tabs: [{ id: "second-chat", mode: "agent" }],
          });
          setProjectStorageNamespace(previousNamespace ?? undefined);
          act(() => {
            store.dispatch(
              setCurrentProjectInfo({
                name: "new-source",
                projectRoot: "/new-source",
              }),
            );
            store.dispatch(
              updateConfig({
                apiKey: "different-principal",
                lspUrl: "http://127.0.0.1:9999",
              }),
            );
          });
          expect(store.getState().config.apiKey).toBe("different-principal");
          expect(store.getState().config.lspUrl).toBe("http://127.0.0.1:9999");
          expect(store.getState().current_project.projectRoot).toBe(
            "/new-source",
          );
          expect(store.getState().chat.threads["first-chat"]).toBeUndefined();
        }
        rerender(
          <ChatThreadProvider chatId="second-chat">
            <WorktreeControl onOpenChange={onOpenChange} />
          </ChatThreadProvider>,
        );
        expect(
          selectThreadWorktreeById(store.getState(), "second-chat"),
        ).toEqual(second.meta);
        expect(
          screen.getByTestId("worktree-control-trigger"),
        ).toHaveTextContent("refact/chat/second");
        onOpenChange.mockClear();
      } finally {
        await act(async () => {
          release();
          await sendCommand.mock.results[0]?.value;
        });
        sendCommand.mockRestore();
      }
      if (change === "source") {
        expect(store.getState().chat.threads["first-chat"]).toBeUndefined();
        expect(store.getState().chat.open_thread_ids).toEqual(["second-chat"]);
      } else {
        expect(
          selectThreadWorktreeById(store.getState(), "first-chat"),
        ).toEqual(first.meta);
      }
      expect(selectThreadWorktreeById(store.getState(), "second-chat")).toEqual(
        second.meta,
      );
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "refact/chat/second",
      );
      expect(onOpenChange).not.toHaveBeenCalledWith(false);
      await user.click(screen.getByTestId("worktree-control-trigger"));
      expect(screen.queryByText("Worktree attached.")).not.toBeInTheDocument();
      expect(commandCalls).toHaveLength(1);
    },
  );

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
    await waitFor(() =>
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "refact/chat/shared",
      ),
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

  test("explicit detach waits for applied ACK, blocks duplicates and never deletes", async () => {
    const record = makeWorktreeRecord("wt-detach", "refact/chat/detach");
    const commandCalls: JsonObject[] = [];
    const deleteCalls: string[] = [];
    let releaseDetach: () => void = () => undefined;
    const detachGate = new Promise<void>((resolve) => {
      releaseDetach = resolve;
    });
    server.use(
      worktreesList([record]),
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandCalls.push((await request.json()) as JsonObject);
        await detachGate;
        return HttpResponse.json({ status: "applied" });
      }),
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
    try {
      expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
        record.meta,
      );
      expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
        "refact/chat/detach",
      );
      expect(
        screen.queryByText("Using main workspace."),
      ).not.toBeInTheDocument();
      expect(
        screen.getByText("Updating checkout scope..."),
      ).toBeInTheDocument();
      const detach = screen.getByRole("button", {
        name: "Detach worktree and use main workspace",
      });
      expect(detach).toBeDisabled();
      await user.click(detach);
      expect(commandCalls).toHaveLength(1);
    } finally {
      act(() => {
        releaseDetach();
      });
    }
    await waitFor(() =>
      expect(selectThreadWorktreeById(store.getState(), "chat-1")).toBeNull(),
    );
    expect(deleteCalls).toHaveLength(0);
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toBeNull();
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "dev",
    );
    await user.click(screen.getByTestId("worktree-control-trigger"));
    expect(
      await screen.findByText("Using main workspace."),
    ).toBeInTheDocument();
  });

  test("rejected explicit detach keeps its binding and never announces main", async () => {
    const record = makeWorktreeRecord("wt-rejected", "refact/chat/rejected");
    const commandCalls: JsonObject[] = [];
    server.use(
      worktreesList([record]),
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandCalls.push((await request.json()) as JsonObject);
        return HttpResponse.json(
          { error: "checkout is managed" },
          { status: 409 },
        );
      }),
    );
    const { user, store } = renderControl([record], record.meta);
    await user.click(screen.getByTestId("worktree-control-trigger"));
    await user.click(
      await screen.findByRole("button", {
        name: "Detach worktree and use main workspace",
      }),
    );
    expect(
      await screen.findByText("Worktree update failed: checkout is managed"),
    ).toBeInTheDocument();
    expect(selectThreadWorktreeById(store.getState(), "chat-1")).toEqual(
      record.meta,
    );
    expect(screen.getByTestId("worktree-control-trigger")).toHaveTextContent(
      "refact/chat/rejected",
    );
    expect(screen.queryByText("Using main workspace.")).not.toBeInTheDocument();
    expect(commandCalls).toHaveLength(1);
    expect(commandCalls[0]).toMatchObject({
      type: "set_params",
      patch: { worktree: null },
    });
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
