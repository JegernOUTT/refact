import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../app/store";
import { selectAgentRosterSource } from "../features/AgentsPanel/agentsPanelSlice";
import { updateConfig } from "../features/Config/configSlice";
import { server } from "../utils/mockServer";
import { trajectoriesApi } from "../services/refact/trajectories";
import { backgroundAgentsApi } from "../services/refact/backgroundAgents";
import type { BackgroundAgentSummary } from "../services/refact";
import {
  tasksApi,
  type TaskMeta,
  type TaskBoard,
} from "../services/refact/tasks";
import { push } from "../features/Pages/pagesSlice";
import { setProjectStorageNamespace } from "../utils/chatUiPersistence";
import {
  INTERNAL_NAVIGATION_TIMEOUT_MS,
  useInternalNavigation,
} from "./useInternalNavigation";

const metadata = {
  id: "child",
  title: "Linked child",
  model: "test",
  mode: "AGENT",
  parent_id: "pane-parent",
  root_chat_id: "pane-root",
  link_type: "subagent",
};

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function setup(localTaskChatContext?: {
  task: TaskMeta;
  board: TaskBoard;
  plannerChats: { id: string }[];
  pendingPlannerIds?: ReadonlySet<string>;
  deletedPlannerIds?: ReadonlySet<string>;
}) {
  const store = setUpStore();
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return {
    store,
    ...renderHook(
      ({ scope, context }) =>
        useInternalNavigation({ scope, localTaskChatContext: context }),
      {
        wrapper,
        initialProps: { scope: "source", context: localTaskChatContext },
      },
    ),
  };
}

afterEach(() => {
  vi.useRealTimers();
  setProjectStorageNamespace(undefined);
});

function localContext() {
  const task: TaskMeta = {
    id: "task-1",
    name: "Task",
    status: "active",
    created_at: "",
    updated_at: "",
    cards_total: 0,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 0,
  };
  const board: TaskBoard = {
    schema_version: 1,
    rev: 1,
    columns: [],
    cards: [],
    owner_chat_id: "child",
  };
  return { task, board, plannerChats: [{ id: "child" }] };
}

describe("useInternalNavigation local task chats", () => {
  it.each(["pending", "deleted"])(
    "preserves the %s planner policy with a newer empty saved list",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const pendingPlannerIds = new Set(["child"]);
      const deletedPlannerIds = new Set<string>();
      const { store, result } = setup({
        ...localContext(),
        pendingPlannerIds,
        deletedPlannerIds,
      });
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigateLocalTaskChat("child", "task-1");
      });
      await started.promise;
      await act(async () => {
        await store.dispatch(
          tasksApi.util.upsertQueryData(
            "listTaskTrajectories",
            { taskId: "task-1", role: "planner" },
            [],
          ),
        );
        if (change === "deleted") deletedPlannerIds.add("child");
        response.resolve();
        expect(await navigation).toBe(change === "pending");
      });
      expect(Boolean(store.getState().chat.threads.child)).toBe(
        change === "pending",
      );
    },
  );

  it.each(["planner", "owner"])(
    "rejects stale %s membership from query cache without a context render",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const context = localContext();
      const { store, result } = setup(context);
      const source = store.getState().chat.current_thread_id;
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigateLocalTaskChat(
          "child",
          "task-1",
          undefined,
          change === "owner",
        );
      });
      await started.promise;
      await act(async () => {
        if (change === "planner") {
          await store.dispatch(
            tasksApi.util.upsertQueryData(
              "listTaskTrajectories",
              { taskId: "task-1", role: "planner" },
              [],
            ),
          );
        } else {
          await store.dispatch(
            tasksApi.util.upsertQueryData("getBoard", "task-1", {
              ...context.board,
              owner_chat_id: "other",
            }),
          );
        }
        response.resolve();
        expect(await navigation).toBe(false);
      });
      expect(result.current.error).toContain("not a current planner or agent");
      expect(store.getState().chat.current_thread_id).toBe(source);
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );

  it.each(["incomplete", "unknown", "generic"])(
    "keeps generic validation and feedback for %s context",
    async (kind) => {
      const calls: string[] = [];
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
        http.get("*/v1/tasks/task-1", () => {
          calls.push("task");
          return HttpResponse.json({ meta: localContext().task });
        }),
        http.get("*/v1/tasks/task-1/board", () => {
          calls.push("board");
          return HttpResponse.json(localContext().board);
        }),
        http.get("*/v1/tasks/task-1/trajectories/planner", () => {
          calls.push("planner");
          return HttpResponse.json([{ id: "child" }]);
        }),
      );
      const context = localContext();
      if (kind === "unknown") context.plannerChats = [];
      const { result } = setup(kind === "incomplete" ? undefined : context);
      let navigation!: Promise<boolean>;
      act(() => {
        navigation =
          kind === "generic"
            ? result.current.navigate("refact://chat/child", {
                taskId: "task-1",
              })
            : result.current.navigateLocalTaskChat("child", "task-1");
      });
      await started.promise;
      expect(result.current.showProgress).toBe(true);
      await act(async () => {
        response.resolve();
        expect(await navigation).toBe(true);
      });
      expect(calls).toEqual(["task", "board", "planner"]);
    },
  );

  it("rejects an agent moved to a different card while metadata is held", async () => {
    const started = deferred();
    const response = deferred();
    server.use(
      http.get("*/v1/trajectories/child", async () => {
        started.resolve();
        await response.promise;
        return HttpResponse.json(metadata);
      }),
    );
    const context = localContext();
    context.plannerChats = [];
    const card: TaskBoard["cards"][number] = {
      id: "T-1",
      title: "Agent",
      column: "doing",
      priority: "P1",
      depends_on: [],
      instructions: "",
      assignee: null,
      agent_chat_id: "child",
      status_updates: [],
      final_report: null,
      created_at: "",
      started_at: null,
      completed_at: null,
      target_files: [],
    };
    context.board.cards = [card];
    const { result, rerender, store } = setup(context);
    let navigation!: Promise<boolean>;
    act(() => {
      navigation = result.current.navigateLocalTaskChat(
        "child",
        "task-1",
        "T-1",
      );
    });
    await started.promise;
    rerender({
      scope: "source",
      context: {
        ...context,
        board: { ...context.board, cards: [{ ...card, id: "T-2" }] },
      },
    });
    await act(async () => {
      response.resolve();
      expect(await navigation).toBe(false);
    });
    expect(result.current.error).toContain("not a current planner or agent");
    expect(store.getState().chat.threads.child).toBeUndefined();
  });

  it.each(["planner removed", "owner changed", "context lost"])(
    "rechecks latest loaded membership after metadata: %s",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const context = localContext();
      const { store, result, rerender } = setup(context);
      const source = store.getState().chat.current_thread_id;
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigateLocalTaskChat(
          "child",
          "task-1",
          undefined,
          true,
        );
      });
      await started.promise;
      expect(result.current.pending).toBe(true);
      expect(result.current.showProgress).toBe(false);
      rerender({
        scope: "source",
        context:
          change === "context lost"
            ? undefined
            : {
                ...context,
                plannerChats:
                  change === "planner removed" ? [] : context.plannerChats,
                board: {
                  ...context.board,
                  owner_chat_id: change === "owner changed" ? "other" : "child",
                },
              },
      });
      await act(async () => {
        response.resolve();
        expect(await navigation).toBe(false);
      });
      expect(result.current.error).toContain("not a current planner or agent");
      expect(store.getState().chat.current_thread_id).toBe(source);
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );

  it.each(["missing", "denied", "id", "task", "role", "card"])(
    "retains source on %s metadata",
    async (failure) => {
      const context = localContext();
      if (failure === "card") {
        context.plannerChats = [];
        context.board.cards = [
          { id: "T-1", agent_chat_id: "child" } as TaskBoard["cards"][number],
        ];
      }
      server.use(
        http.get("*/v1/trajectories/child", () => {
          if (failure === "missing" || failure === "denied")
            return HttpResponse.json(
              { detail: "Unavailable" },
              { status: failure === "missing" ? 404 : 403 },
            );
          return HttpResponse.json({
            ...metadata,
            id: failure === "id" ? "wrong" : "child",
            task_meta: {
              task_id: failure === "task" ? "other-task" : "task-1",
              role:
                failure === "role" || failure === "card" ? "agents" : "planner",
              card_id: "wrong-card",
            },
          });
        }),
      );
      const { store, result } = setup(context);
      const source = store.getState().chat.current_thread_id;
      await act(async () => {
        expect(
          await result.current.navigateLocalTaskChat("child", "task-1"),
        ).toBe(false);
      });
      expect(result.current.error).toMatch(/Could not open link/);
      expect(result.current.pending).toBe(false);
      expect(store.getState().chat.current_thread_id).toBe(source);
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );

  it.each(["endpoint", "scope", "source", "project", "timeout"])(
    "cancels held local metadata on %s",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const context = localContext();
      const { store, result, rerender } = setup(context);
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigateLocalTaskChat("child", "task-1");
      });
      await started.promise;
      if (change === "scope") rerender({ scope: "other", context });
      await act(async () => {
        if (change === "endpoint")
          store.dispatch(updateConfig({ lspUrl: "http://other:9000" }));
        if (change === "source") store.dispatch(push({ name: "history" }));
        if (change === "project") {
          setProjectStorageNamespace("other-project");
          store.dispatch(updateConfig({ dev: true }));
        }
        if (change === "timeout")
          await vi.advanceTimersByTimeAsync(INTERNAL_NAVIGATION_TIMEOUT_MS);
        expect(await navigation).toBe(false);
      });
      expect(result.current.pending).toBe(false);
      if (change === "timeout")
        expect(result.current.error).toMatch(/timed out/);
      await act(async () => {
        response.resolve();
        await response.promise;
      });
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );
});

describe("useInternalNavigation direct agents", () => {
  const agent = {
    agent_id: "known-agent",
    child_chat_id: "child",
    parent_chat_id: "pane-parent",
    kind: "subagent",
    status: "running",
    change_seq: 1,
  } as BackgroundAgentSummary;

  it.each(["direct", "generic", "unknown", "descendant"])(
    "waits quietly only for known direct selections: %s",
    async (intent) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async ({ request }) => {
          expect(new URL(request.url).search).toBe("?metadata_only=true");
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const { store, result } = setup();
      const source = store.getState().chat.current_thread_id;
      const root = intent === "descendant" ? "ancestor" : "pane-parent";
      await store.dispatch(
        backgroundAgentsApi.util.upsertQueryData(
          "getBackgroundAgents",
          {
            chatId: root,
            source: selectAgentRosterSource(store.getState()),
          },
          [
            ...(intent === "descendant"
              ? [
                  {
                    ...agent,
                    agent_id: "ancestor-agent",
                    parent_chat_id: root,
                    child_chat_id: "pane-parent",
                  },
                ]
              : []),
            ...(intent === "unknown" ? [] : [agent]),
          ],
        ),
      );
      let navigation!: Promise<boolean>;
      act(() => {
        navigation =
          intent === "generic"
            ? result.current.navigate("refact://chat/child", { chatId: root })
            : result.current.navigateKnownAgent("child", root);
      });
      await started.promise;
      expect(result.current.pending).toBe(true);
      expect(result.current.showProgress).toBe(
        intent === "generic" || intent === "unknown",
      );
      expect(store.getState().chat.current_thread_id).toBe(source);
      await act(async () => {
        response.resolve();
        expect(await navigation).toBe(true);
      });
      expect(store.getState().chat.threads.child?.thread).toMatchObject({
        parent_id: "pane-parent",
        link_type: "subagent",
      });
    },
  );

  it.each(["removed", "parent", "child", "metadata parent", "404", "id"])(
    "retains the source for stale or invalid direct agents: %s",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return change === "404"
            ? HttpResponse.json({ detail: "Missing agent" }, { status: 404 })
            : HttpResponse.json({
                ...metadata,
                ...(change === "id" ? { id: "wrong" } : {}),
                ...(change === "metadata parent" ? { parent_id: "other" } : {}),
              });
        }),
      );
      const { store, result } = setup();
      await store.dispatch(
        backgroundAgentsApi.util.upsertQueryData(
          "getBackgroundAgents",
          {
            chatId: "pane-parent",
            source: selectAgentRosterSource(store.getState()),
          },
          [agent],
        ),
      );
      const source = store.getState().chat.current_thread_id;
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigateKnownAgent("child", "pane-parent");
      });
      await started.promise;
      if (["removed", "parent", "child"].includes(change)) {
        await act(async () => {
          await store.dispatch(
            backgroundAgentsApi.util.upsertQueryData(
              "getBackgroundAgents",
              {
                chatId: "pane-parent",
                source: selectAgentRosterSource(store.getState()),
              },
              change === "removed"
                ? []
                : [
                    {
                      ...agent,
                      parent_chat_id:
                        change === "parent" ? "other" : agent.parent_chat_id,
                      child_chat_id:
                        change === "child" ? "other" : agent.child_chat_id,
                      change_seq: 2,
                    },
                  ],
            ),
          );
        });
      }
      await act(async () => {
        response.resolve();
        expect(await navigation).toBe(false);
      });
      expect(result.current.error).toMatch(/Could not open link/);
      expect(store.getState().chat.current_thread_id).toBe(source);
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );

  it("does not bypass task membership verification for a known agent", async () => {
    const calls: string[] = [];
    server.use(
      http.get("*/v1/trajectories/child", () =>
        HttpResponse.json({
          ...metadata,
          task_meta: { task_id: "task-1", role: "agents", card_id: "T-1" },
        }),
      ),
      http.get("*/v1/tasks/task-1", () => {
        calls.push("task");
        return HttpResponse.json({ meta: localContext().task });
      }),
      http.get("*/v1/tasks/task-1/board", () => {
        calls.push("board");
        return HttpResponse.json({ cards: [] });
      }),
      http.get("*/v1/tasks/task-1/trajectories/planner", () => {
        calls.push("planner");
        return HttpResponse.json([]);
      }),
    );
    const { store, result } = setup();
    await store.dispatch(
      backgroundAgentsApi.util.upsertQueryData(
        "getBackgroundAgents",
        {
          chatId: "pane-parent",
          source: selectAgentRosterSource(store.getState()),
        },
        [agent],
      ),
    );
    await act(async () => {
      expect(
        await result.current.navigateKnownAgent("child", "pane-parent"),
      ).toBe(false);
    });
    expect(calls).toEqual(["task", "board", "planner"]);
    expect(result.current.error).toContain("not a current planner or agent");
    expect(store.getState().chat.threads.child).toBeUndefined();
  });
});

describe("useInternalNavigation metadata lifecycle", () => {
  it("waits for metadata only and preserves explicit provenance", async () => {
    const started = deferred();
    const response = deferred();
    const urls: string[] = [];
    server.use(
      http.get("*/v1/trajectories/child", async ({ request }) => {
        urls.push(request.url);
        started.resolve();
        await response.promise;
        return HttpResponse.json(metadata);
      }),
    );
    const { store, result } = setup();
    const source = store.getState().chat.current_thread_id;
    let navigation!: Promise<boolean>;
    act(() => {
      navigation = result.current.navigate("refact://chat/child", {
        chatId: "pane-parent",
      });
    });
    await started.promise;
    expect(result.current.pending).toBe(true);
    expect(store.getState().chat.current_thread_id).toBe(source);
    expect(urls).toHaveLength(1);
    expect(new URL(urls[0]).search).toBe("?metadata_only=true");
    await act(async () => {
      response.resolve();
      expect(await navigation).toBe(true);
    });
    expect(store.getState().chat.threads.child?.thread).toMatchObject({
      ...metadata,
      mode: "agent",
    });
    expect(result.current.pending).toBe(false);
  });

  it("times out a pending request, aborts it, accepts retry and ignores late completion", async () => {
    const started = deferred();
    const response = deferred();
    let count = 0;
    let aborted = false;
    server.use(
      http.get("*/v1/trajectories/child", async ({ request }) => {
        count += 1;
        if (count === 1) {
          request.signal.addEventListener("abort", () => {
            aborted = true;
          });
          started.resolve();
          await response.promise;
          return HttpResponse.json({ ...metadata, title: "Stale response" });
        }
        return HttpResponse.json(metadata);
      }),
    );
    const { store, result } = setup();
    const source = store.getState().chat.current_thread_id;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    let navigation!: Promise<boolean>;
    act(() => {
      navigation = result.current.navigate("refact://chat/child");
    });
    await started.promise;
    expect(result.current.pending).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(INTERNAL_NAVIGATION_TIMEOUT_MS);
      expect(await navigation).toBe(false);
    });
    expect(result.current.pending).toBe(false);
    expect(result.current.error).toMatch(/timed out.*try again/i);
    expect(aborted).toBe(true);
    expect(store.getState().chat.current_thread_id).toBe(source);
    vi.useRealTimers();
    await act(async () => {
      expect(await result.current.navigate("refact://chat/child")).toBe(true);
    });
    await act(async () => {
      response.resolve();
      await response.promise;
    });
    expect(count).toBe(2);
    expect(store.getState().chat.threads.child?.thread.title).toBe(
      metadata.title,
    );
  });

  it.each(["endpoint", "scope", "unmount"])(
    "releases a pending request when %s changes without late routing",
    async (change) => {
      const started = deferred();
      const response = deferred();
      server.use(
        http.get("*/v1/trajectories/child", async () => {
          started.resolve();
          await response.promise;
          return HttpResponse.json(metadata);
        }),
      );
      const { store, result, rerender, unmount } = setup();
      const source = store.getState().chat.current_thread_id;
      let navigation!: Promise<boolean>;
      act(() => {
        navigation = result.current.navigate("refact://chat/child");
      });
      await started.promise;
      act(() => {
        if (change === "endpoint")
          store.dispatch(updateConfig({ lspUrl: "http://other-engine:9000" }));
        else if (change === "scope")
          rerender({ scope: "other", context: undefined });
        else unmount();
      });
      await act(async () => {
        expect(await navigation).toBe(false);
      });
      await act(async () => {
        response.resolve();
        await response.promise;
      });
      expect(store.getState().chat.current_thread_id).toBe(source);
      expect(store.getState().chat.threads.child).toBeUndefined();
    },
  );

  it("does not abort another subscriber's deduplicated metadata request", async () => {
    const started = deferred();
    const response = deferred();
    let aborted = false;
    server.use(
      http.get("*/v1/trajectories/child", async ({ request }) => {
        request.signal.addEventListener("abort", () => {
          aborted = true;
        });
        started.resolve();
        await response.promise;
        return HttpResponse.json(metadata);
      }),
    );
    const { store, result, unmount } = setup();
    const shared = store.dispatch(
      trajectoriesApi.endpoints.getTrajectoryMetadata.initiate("child"),
    );
    await started.promise;
    let navigation!: Promise<boolean>;
    act(() => {
      navigation = result.current.navigate("refact://chat/child");
    });
    unmount();
    expect(await navigation).toBe(false);
    expect(aborted).toBe(false);
    response.resolve();
    expect(await shared.unwrap()).toEqual(metadata);
    shared.unsubscribe();
  });

  it.each(["planner", "agents", "linked"])(
    "resolves %s task context using metadata without messages",
    async (role) => {
      const calls: string[] = [];
      server.use(
        http.get("*/v1/trajectories/child", () =>
          HttpResponse.json({
            ...metadata,
            task_meta: {
              task_id: "task-1",
              role,
              ...(role === "agents" ? { card_id: "T-1" } : {}),
            },
          }),
        ),
        http.get("*/v1/tasks/task-1", () => {
          calls.push("task");
          return HttpResponse.json({ meta: { id: "task-1", name: "Task" } });
        }),
        http.get("*/v1/tasks/task-1/board", () => {
          calls.push("board");
          return HttpResponse.json({
            cards:
              role === "agents"
                ? [
                    {
                      id: "T-1",
                      agent_chat_id: "child",
                      title: "Card",
                      column: "doing",
                      priority: "P1",
                      depends_on: [],
                      instructions: "Work",
                    },
                  ]
                : [],
          });
        }),
        http.get("*/v1/tasks/task-1/trajectories/planner", () => {
          calls.push("planner");
          return HttpResponse.json(role !== "agents" ? [{ id: "child" }] : []);
        }),
      );
      const { store, result } = setup();
      await act(async () => {
        expect(await result.current.navigate("refact://chat/child")).toBe(true);
      });
      expect(calls).toEqual(["task", "board", "planner"]);
      expect(store.getState().pages.at(-1)).toMatchObject({
        name: "task workspace",
        taskId: "task-1",
      });
      expect(
        store.getState().chat.threads.child?.thread.task_meta,
      ).toMatchObject({ task_id: "task-1", role });
    },
  );

  it("applies one total deadline through the later board stage", async () => {
    const metadataStarted = deferred();
    const metadataResponse = deferred();
    const boardStarted = deferred();
    const boardResponse = deferred();
    server.use(
      http.get("*/v1/trajectories/child", async () => {
        metadataStarted.resolve();
        await metadataResponse.promise;
        return HttpResponse.json({
          ...metadata,
          task_meta: { task_id: "task-1", role: "planner" },
        });
      }),
      http.get("*/v1/tasks/task-1", () =>
        HttpResponse.json({ meta: { id: "task-1", name: "Task" } }),
      ),
      http.get("*/v1/tasks/task-1/board", async () => {
        boardStarted.resolve();
        await boardResponse.promise;
        return HttpResponse.json({ cards: [] });
      }),
    );
    const { store, result } = setup();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    let navigation!: Promise<boolean>;
    act(() => {
      navigation = result.current.navigate("refact://chat/child");
    });
    await metadataStarted.promise;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
      metadataResponse.resolve();
    });
    await boardStarted.promise;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(
        INTERNAL_NAVIGATION_TIMEOUT_MS - 10_000,
      );
      expect(await navigation).toBe(false);
    });
    expect(result.current.error).toMatch(/timed out/i);
    expect(result.current.pending).toBe(false);
    await act(async () => {
      boardResponse.resolve();
      await boardResponse.promise;
    });
    expect(store.getState().chat.threads.child).toBeUndefined();
  });

  it("keeps the source on denied or mismatched metadata", async () => {
    server.use(
      http.get("*/v1/trajectories/child", () =>
        HttpResponse.json({ detail: "Access denied" }, { status: 403 }),
      ),
    );
    const { store, result } = setup();
    const source = store.getState().chat.current_thread_id;
    await act(async () => {
      expect(await result.current.navigate("refact://chat/child")).toBe(false);
    });
    expect(result.current.error).toContain("Access denied");
    expect(store.getState().chat.current_thread_id).toBe(source);
    server.use(
      http.get("*/v1/trajectories/child", () =>
        HttpResponse.json({ ...metadata, id: "wrong" }),
      ),
    );
    await act(async () => {
      expect(await result.current.navigate("refact://chat/child")).toBe(false);
    });
    expect(result.current.error).toContain("did not match");
    expect(store.getState().chat.current_thread_id).toBe(source);
    await waitFor(() => expect(result.current.pending).toBe(false));
  });
});
