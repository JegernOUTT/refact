import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../app/store";
import { updateConfig } from "../features/Config/configSlice";
import { server } from "../utils/mockServer";
import { trajectoriesApi } from "../services/refact/trajectories";
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

function setup() {
  const store = setUpStore();
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return {
    store,
    ...renderHook(({ scope }) => useInternalNavigation({ scope }), {
      wrapper,
      initialProps: { scope: "source" },
    }),
  };
}

afterEach(() => vi.useRealTimers());

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
        else if (change === "scope") rerender({ scope: "other" });
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

  it.each(["planner", "agents"])(
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
          return HttpResponse.json(role === "planner" ? [{ id: "child" }] : []);
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
