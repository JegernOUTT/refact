/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { configureStore, type UnknownAction } from "@reduxjs/toolkit";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  appWorkspacePersistenceMiddleware,
  openRequestedApp,
} from "./appWorkspacePersistence";
import { appsSlice, openRailApp, retryAppWorkspaceSave } from "./appsSlice";
import {
  workspaceSlice,
  openTab,
  resizeGroupSplit,
  setActiveTab,
  closePaneTab,
  addSurfaceToPane,
} from "../Workspace/workspaceSlice";
import { collectTabIds } from "../ChatPanes/panesTree";
import type { AppWorkspace } from "./appWorkspace";
import { paneToAppLayout } from "./appLayout";
import { AppWindow, Calendar } from "lucide-react";
import { appRailIcon } from "./appRailIcon";

test("rail icons resolve only allowlisted lucide identifiers", () => {
  expect(appRailIcon("lucide:calendar")).toBe(Calendar);
  for (const icon of [
    null,
    undefined,
    "",
    "lucide:unknown",
    "lucide:constructor",
    "lucide:__proto__",
    "calendar",
    "https://example.com/icon.svg",
  ])
    expect(appRailIcon(icon)).toBe(AppWindow);
});

const bindingId = "22222222-2222-4222-8222-222222222222";
const remoteWorkspace = (
  revision: number,
  sizes = [0.72, 0.28],
): AppWorkspace => ({
  schema_version: 1,
  revision,
  bindings: [{ app_id: appId, binding_id: bindingId, params: {}, state: {} }],
  layout: {
    kind: "split",
    direction: "horizontal",
    sizes,
    children: [
      { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }] },
      { kind: "leaf", tabs: [{ kind: "app", binding_id: bindingId }] },
    ],
  },
});

const appId = "11111111-1111-4111-8111-111111111111";
function setup(open = true, snapshotReceived = true) {
  const initial = {
    open_thread_ids: open ? ["a"] : [],
    threads: {
      a: {
        snapshot_received: snapshotReceived,
        thread: {} as { app_workspace?: AppWorkspace },
      },
    },
  };
  const store = configureStore({
    reducer: {
      workspace: workspaceSlice.reducer,
      apps: appsSlice.reducer,
      config: (
        state: { lspUrl: string; host: string; engineServed: boolean } = {
          lspUrl: "http://localhost:8001",
          host: "web",
          engineServed: true,
        },
        action: UnknownAction,
      ) =>
        action.type === "test/endpoint"
          ? { ...state, engineServed: false, lspUrl: String(action.payload) }
          : state,
      chat: (state: typeof initial = initial, action: UnknownAction) =>
        action.type === "test/snapshot"
          ? {
              ...state,
              threads: {
                a: {
                  snapshot_received: true,
                  thread: { app_workspace: action.payload as AppWorkspace },
                },
              },
            }
          : action.type === "test/close"
            ? { ...state, open_thread_ids: [] }
            : action.type === "test/reopen"
              ? { ...state, open_thread_ids: ["a"] }
              : state,
    },
    middleware: (getDefault) =>
      getDefault().prepend(appWorkspacePersistenceMiddleware),
  });
  if (open) store.dispatch(openTab("chat:a"));
  return store;
}
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});
describe("app workspace coordinator", () => {
  test.each(["addition", "update", "detach"] as const)(
    "newer remote binding %s is not mistaken for a post-send local edit",
    async (change) => {
      vi.useFakeTimers();
      let resolve!: (response: Response) => void;
      const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(
        () =>
          new Promise((done) => {
            resolve = done;
          }),
      );
      const store = setup();
      store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
      const root = store.getState().workspace.groups["chat:a"]!.root;
      store.dispatch(
        resizeGroupSplit({
          tabId: "chat:a",
          splitId: root.id,
          sizes: [0.6, 0.4],
        }),
      );
      await vi.advanceTimersByTimeAsync(150);
      const remote = remoteWorkspace(9, [0.8, 0.2]);
      if (change === "addition")
        remote.bindings.push({
          ...remote.bindings[0],
          binding_id: "33333333-3333-4333-8333-333333333333",
        });
      if (change === "update") remote.bindings[0].state = { remote: true };
      if (change === "detach") {
        remote.bindings = [];
        remote.layout = { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }] };
      }
      store.dispatch({ type: "test/snapshot", payload: remote });
      resolve(
        new Response(
          JSON.stringify({ app_workspace: remoteWorkspace(8, [0.6, 0.4]) }),
        ),
      );
      await vi.advanceTimersByTimeAsync(500);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      expect(
        paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
      ).toMatchObject(remote.layout);
    },
  );

  test.each(["update", "detach"] as const)(
    "post-send local resize preserves remote binding %s without resurrecting records",
    async (change) => {
      vi.useFakeTimers();
      let resolve!: (response: Response) => void;
      const fetchMock = vi
        .spyOn(globalThis, "fetch")
        .mockImplementationOnce(
          () =>
            new Promise((done) => {
              resolve = done;
            }),
        )
        .mockImplementation((_url, options) => {
          const body = JSON.parse(String(options?.body)) as {
            workspace: AppWorkspace;
            expected_revision: number;
          };
          return Promise.resolve(
            new Response(
              JSON.stringify({
                app_workspace: {
                  ...body.workspace,
                  revision: body.expected_revision + 1,
                },
              }),
            ),
          );
        });
      const store = setup();
      store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
      const root = store.getState().workspace.groups["chat:a"]!.root;
      store.dispatch(
        resizeGroupSplit({
          tabId: "chat:a",
          splitId: root.id,
          sizes: [0.6, 0.4],
        }),
      );
      await vi.advanceTimersByTimeAsync(150);
      store.dispatch(
        resizeGroupSplit({
          tabId: "chat:a",
          splitId: root.id,
          sizes: [0.55, 0.45],
        }),
      );
      const remote = remoteWorkspace(9, [0.8, 0.2]);
      if (change === "update") remote.bindings[0].state = { remote: true };
      else {
        remote.bindings = [];
        remote.layout = { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }] };
      }
      store.dispatch({ type: "test/snapshot", payload: remote });
      resolve(
        new Response(
          JSON.stringify({ app_workspace: remoteWorkspace(8, [0.6, 0.4]) }),
        ),
      );
      await vi.advanceTimersByTimeAsync(500);
      expect(fetchMock).toHaveBeenCalledTimes(change === "detach" ? 1 : 2);
      if (change === "detach") {
        expect(
          paneToAppLayout(
            store.getState().workspace.groups["chat:a"]!.root,
            "a",
          ),
        ).toMatchObject(remote.layout);
        expect(store.getState().apps.saveStatus?.a).toBe("saved");
        return;
      }
      const saved = JSON.parse(String(fetchMock.mock.calls[1][1]?.body)) as {
        workspace: AppWorkspace;
        expected_revision: number;
      };
      expect(saved.expected_revision).toBe(9);
      expect(saved.workspace.bindings).toEqual(remote.bindings);
      expect(saved.workspace.layout).toMatchObject({ sizes: [0.55, 0.45] });
    },
  );

  test("newer remote snapshot wins over a delayed ack but not an unsaved local resize", async () => {
    vi.useFakeTimers();
    let resolve!: (response: Response) => void;
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const store = setup();
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.6, 0.4],
      }),
    );
    await vi.advanceTimersByTimeAsync(150);
    store.dispatch({
      type: "test/snapshot",
      payload: remoteWorkspace(9, [0.8, 0.2]),
    });
    resolve(
      new Response(
        JSON.stringify({ app_workspace: remoteWorkspace(8, [0.6, 0.4]) }),
      ),
    );
    await vi.advanceTimersByTimeAsync(500);
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toMatchObject({ sizes: [0.8, 0.2] });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const latest = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: latest.id,
        sizes: [0.55, 0.45],
      }),
    );
    store.dispatch({
      type: "test/snapshot",
      payload: remoteWorkspace(10, [0.9, 0.1]),
    });
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toMatchObject({ sizes: [0.55, 0.45] });
  });

  test("tool open waits for its exact binding and repeated opens preserve the linked tree", () => {
    const store = setup(true, false);
    const intent = openRequestedApp({
      appId,
      name: "App",
      chatId: "a",
      bindingId,
    });
    store.dispatch(intent);
    expect(Object.keys(store.getState().apps.views)).toEqual([]);
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(intent);
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toEqual(paneToAppLayout(root, "a"));
    expect(store.getState().workspace.tabs).toEqual(["chat:a"]);
    store.dispatch({ type: "test/close" });
    store.dispatch({ type: "test/reopen" });
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toEqual(paneToAppLayout(root, "a"));
  });

  test("two idle clients accept remote defaults and resize snapshots without echo writes", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.workspace.revision + 1,
              },
            }),
          ),
        );
      });
    const clients = [setup(), setup()];
    for (const client of clients)
      client.dispatch({
        type: "test/snapshot",
        payload: remoteWorkspace(1001),
      });
    for (let revision = 1002; revision < 1006; revision++) {
      for (const client of clients) {
        client.dispatch({
          type: "test/snapshot",
          payload: remoteWorkspace(revision, [0.65, 0.35]),
        });
        client.dispatch({ type: "test/unrelated" });
      }
      await vi.advanceTimersByTimeAsync(500);
    }
    expect(fetchMock).not.toHaveBeenCalled();
    for (const client of clients)
      expect(
        paneToAppLayout(
          client.getState().workspace.groups["chat:a"]!.root,
          "a",
        ),
      ).toMatchObject({ sizes: [0.65, 0.35] });
  });

  test("remote restore preserves unrelated surfaces in the chat group", () => {
    const store = setup();
    store.dispatch(openTab("file:/keep"));
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    store.dispatch(openTab("file:/also-keep"));
    const root = store.getState().workspace.groups["chat:a"]!.root;
    const leaf = root.kind === "split" ? root.children[0] : root;
    store.dispatch(
      addSurfaceToPane({
        tabId: "chat:a",
        leafId: leaf.id,
        surfaceKey: "file:/embedded",
      }),
    );
    store.dispatch({
      type: "test/snapshot",
      payload: remoteWorkspace(8, [0.6, 0.4]),
    });
    expect(
      Object.values(store.getState().workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/keep");
    expect(
      Object.values(store.getState().workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/also-keep");
    expect(
      Object.values(store.getState().workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/embedded");
  });

  test("does not resave an acknowledged layout when server JSON keys have a different order", async () => {
    vi.useFakeTimers();
    const reorder = (value: unknown): unknown => {
      if (Array.isArray(value)) return value.map(reorder);
      if (typeof value !== "object" || value === null) return value;
      return Object.fromEntries(
        Object.entries(value)
          .sort(([left], [right]) => left.localeCompare(right))
          .map(([key, child]) => [key, reorder(child)]),
      );
    };
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: object;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: reorder({
                ...body.workspace,
                revision: body.expected_revision + 1,
              }),
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(500);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    for (let index = 0; index < 4; index++) {
      store.dispatch({ type: "test/unrelated" });
      await vi.advanceTimersByTimeAsync(500);
    }
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  test("opens bound app on right, reuses focused app binding, serializes delayed ack and latest resize", async () => {
    vi.useFakeTimers();
    let resolve!: (value: Response) => void;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementationOnce(
        () =>
          new Promise((done) => {
            resolve = done;
          }),
      )
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: object;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.expected_revision + 1,
              },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    const first = store.getState().workspace.groups["chat:a"]!.root;
    expect(first.kind).toBe("split");
    if (first.kind !== "split") throw new Error("missing split");
    expect(collectTabIds(first.children[0])).toEqual(["chat:a"]);
    const appKey = collectTabIds(first.children[1])[0];
    expect(appKey).toMatch(/^app:/);
    store.dispatch(openRailApp({ appId, name: "App" }));
    expect(Object.keys(store.getState().apps.views)).toEqual([appKey]);
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: first.id,
        sizes: [0.7, 0.3],
      }),
    );
    await vi.advanceTimersByTimeAsync(150);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const sent = JSON.parse(String(fetchMock.mock.calls[0][1]?.body)) as {
      workspace: object;
    };
    resolve(
      new Response(
        JSON.stringify({ app_workspace: { ...sent.workspace, revision: 1 } }),
      ),
    );
    await vi.advanceTimersByTimeAsync(150);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    const second = JSON.parse(String(fetchMock.mock.calls[1][1]?.body)) as {
      expected_revision: number;
      workspace: { layout: { sizes: number[] } };
    };
    expect(second.expected_revision).toBe(1);
    expect(second.workspace.layout.sizes).toEqual([0.7, 0.3]);
  });
  test("immediate close/reopen preserves latest sizes and app closures persist", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: object;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.expected_revision + 1,
              },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(1);
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.65, 0.35],
      }),
    );
    store.dispatch({ type: "test/close" });
    store.dispatch({ type: "test/reopen" });
    const restored = store.getState().workspace.groups["chat:a"]!.root;
    expect(paneToAppLayout(restored, "a")).toMatchObject({
      sizes: [0.65, 0.35],
    });
    const group = store.getState().workspace.groups["chat:a"]!;
    const appKey = collectTabIds(group.root).find((key) =>
      key.startsWith("app:"),
    )!;
    const leaf =
      group.root.kind === "split" ? group.root.children[1] : group.root;
    store.dispatch(
      closePaneTab({ tabId: "chat:a", leafId: leaf.id, surfaceKey: appKey }),
    );
    await vi.advanceTimersByTimeAsync(200);
    const last = JSON.parse(String(fetchMock.mock.calls.at(-1)?.[1]?.body)) as {
      workspace: { layout: unknown };
    };
    expect(last.workspace.layout).toMatchObject({
      kind: "leaf",
      tabs: [{ kind: "chat" }],
    });
  });
  test("no focused open chat opens standalone and foreign chat selection does not use current id", () => {
    const store = setup(false);
    store.dispatch(openRailApp({ appId, name: "App" }));
    expect(store.getState().workspace.activeTabId).toBe(
      `app:standalone:${appId}`,
    );
    store.dispatch(setActiveTab(`app:standalone:${appId}`));
    expect(
      store.getState().apps.views[`app:standalone:${appId}`]?.chatId,
    ).toBeUndefined();
  });
});

test("portable projection filters foreign apps/files, collapses singleton and preserves live tree", () => {
  const root = {
    kind: "split" as const,
    id: "root",
    dir: "row" as const,
    sizes: [0.2, 0.8],
    children: [
      {
        kind: "leaf" as const,
        id: "files",
        tabIds: ["file:/a", "app:foreign"],
        activeTabId: "file:/a",
      },
      {
        kind: "leaf" as const,
        id: "chat",
        tabIds: ["file:/b", "chat:a", "app:own"],
        activeTabId: "app:own",
      },
    ],
  };
  expect(paneToAppLayout(root, "a", new Set(["own"]))).toEqual({
    kind: "leaf",
    tabs: [
      { kind: "chat", ref: "self" },
      { kind: "app", binding_id: "own" },
    ],
    active: 1,
  });
  expect(collectTabIds(root)).toEqual([
    "file:/a",
    "app:foreign",
    "file:/b",
    "chat:a",
    "app:own",
  ]);
});

test.each([false, true])(
  "lost save response after authoritative snapshot keeps correct status (post-send edit: %s)",
  async (edit) => {
    vi.useFakeTimers();
    let reject!: (reason: Error) => void;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValue(new Error("Unexpected request"))
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, fail) => {
            reject = fail;
          }),
      );
    const store = setup();
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.6, 0.4],
      }),
    );
    await vi.advanceTimersByTimeAsync(150);
    if (edit)
      store.dispatch(
        resizeGroupSplit({
          tabId: "chat:a",
          splitId: root.id,
          sizes: [0.55, 0.45],
        }),
      );
    store.dispatch({
      type: "test/snapshot",
      payload: remoteWorkspace(8, [0.6, 0.4]),
    });
    reject(new Error("Lost response"));
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState().apps.saveStatus?.a).toBe(edit ? "error" : "saved");
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toMatchObject({ sizes: edit ? [0.55, 0.45] : [0.6, 0.4] });
  },
);

test("reverted post-send edits are clean when the sent layout is confirmed", async () => {
  vi.useFakeTimers();
  let reject!: (reason: Error) => void;
  const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementationOnce(
    () =>
      new Promise((_done, fail) => {
        reject = fail;
      }),
  );
  const store = setup();
  store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
  const root = store.getState().workspace.groups["chat:a"]!.root;
  const resize = (sizes: number[]) =>
    store.dispatch(
      resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes }),
    );
  resize([0.6, 0.4]);
  await vi.advanceTimersByTimeAsync(150);
  resize([0.55, 0.45]);
  resize([0.6, 0.4]);
  store.dispatch({
    type: "test/snapshot",
    payload: remoteWorkspace(8, [0.6, 0.4]),
  });
  reject(new Error("Lost response"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(store.getState().apps.saveStatus?.a).toBe("saved");
  expect(fetchMock).toHaveBeenCalledTimes(1);
});

test.each([false, true])(
  "authoritative confirmation after a failed response clears the warning (local edit: %s)",
  async (edit) => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValue(new Error("Lost response"));
    const store = setup();
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.6, 0.4],
      }),
    );
    await vi.advanceTimersByTimeAsync(150);
    expect(store.getState().apps.saveStatus?.a).toBe("error");
    if (edit)
      store.dispatch(
        resizeGroupSplit({
          tabId: "chat:a",
          splitId: root.id,
          sizes: [0.55, 0.45],
        }),
      );
    store.dispatch({
      type: "test/snapshot",
      payload: remoteWorkspace(8, [0.6, 0.4]),
    });
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState().apps.saveStatus?.a).toBe(edit ? "error" : "saved");
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toMatchObject({ sizes: edit ? [0.55, 0.45] : [0.6, 0.4] });
  },
);

test.each(["snapshot", "retry"] as const)(
  "confirmed first attachment releases its pending binding through %s after a lost response",
  async (confirmation) => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValue(new Error("Lost response"));
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(150);
    const sent = JSON.parse(String(fetchMock.mock.calls[0][1]?.body)) as {
      workspace: AppWorkspace;
    };
    const id = sent.workspace.bindings[0].binding_id;
    expect(store.getState().apps.pendingBindings?.[id]).toBe(true);
    const confirmed = { ...sent.workspace, revision: 1 };
    if (confirmation === "snapshot")
      store.dispatch({ type: "test/snapshot", payload: confirmed });
    else {
      fetchMock.mockResolvedValueOnce(
        new Response(JSON.stringify({ id: "a", app_workspace: confirmed })),
      );
      store.dispatch(retryAppWorkspaceSave("a"));
    }
    await vi.advanceTimersByTimeAsync(1000);
    expect(store.getState().apps.saveStatus?.a).toBe("saved");
    expect(store.getState().apps.pendingBindings?.[id]).toBe(false);
    expect(fetchMock).toHaveBeenCalledTimes(
      confirmation === "snapshot" ? 1 : 2,
    );
  },
);

test("authoritative confirmation during a failed retry refresh remains saved", async () => {
  vi.useFakeTimers();
  let reject!: (reason: Error) => void;
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValueOnce(new Error("Lost response"))
    .mockImplementationOnce(
      () =>
        new Promise((_done, fail) => {
          reject = fail;
        }),
    );
  const store = setup();
  store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
  const root = store.getState().workspace.groups["chat:a"]!.root;
  store.dispatch(
    resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes: [0.6, 0.4] }),
  );
  await vi.advanceTimersByTimeAsync(150);
  store.dispatch(retryAppWorkspaceSave("a"));
  store.dispatch({
    type: "test/snapshot",
    payload: remoteWorkspace(8, [0.6, 0.4]),
  });
  reject(new Error("Refresh response lost"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(store.getState().apps.saveStatus?.a).toBe("saved");
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

test("retry refresh from a previous endpoint cannot write into the new endpoint", async () => {
  vi.useFakeTimers();
  let resolve!: (response: Response) => void;
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValueOnce(new Error("Offline"))
    .mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
  const store = setup();
  store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
  const root = store.getState().workspace.groups["chat:a"]!.root;
  store.dispatch(
    resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes: [0.6, 0.4] }),
  );
  await vi.advanceTimersByTimeAsync(150);
  store.dispatch(retryAppWorkspaceSave("a"));
  store.dispatch({ type: "test/endpoint", payload: "http://localhost:8002" });
  resolve(new Response(JSON.stringify({ app_workspace: remoteWorkspace(9) })));
  await vi.advanceTimersByTimeAsync(1000);
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

test.each(["update", "detach"] as const)(
  "explicit retry refreshes revision and preserves local layout with remote binding %s",
  async (change) => {
    vi.useFakeTimers();
    const remote = remoteWorkspace(9, [0.8, 0.2]);
    if (change === "update") remote.bindings[0].state = { remote: true };
    else {
      remote.bindings = [];
      remote.layout = { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }] };
    }
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ current_revision: 9 }), { status: 409 }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ app_workspace: remote })),
      )
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.expected_revision + 1,
              },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.6, 0.4],
      }),
    );
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    store.dispatch(retryAppWorkspaceSave("a"));
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(change === "detach" ? 2 : 3);
    expect(fetchMock.mock.calls[1][0]).toBe("/v1/trajectories/a");
    expect(fetchMock.mock.calls[1][1]?.method).toBe("GET");
    if (change === "detach") {
      expect(
        paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
      ).toMatchObject(remote.layout);
      expect(store.getState().apps.saveStatus?.a).toBe("saved");
      return;
    }
    const saved = JSON.parse(String(fetchMock.mock.calls[2][1]?.body)) as {
      workspace: AppWorkspace;
      expected_revision: number;
    };
    expect(saved.expected_revision).toBe(9);
    expect(saved.workspace.bindings).toEqual(remote.bindings);
    expect(saved.workspace.layout).toMatchObject({ sizes: [0.6, 0.4] });
    expect(store.getState().apps.saveStatus?.a).toBe("saved");
  },
);

test.each([null, undefined])(
  "first save retry accepts authoritative absent workspace: %s",
  async (workspace) => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValueOnce(new Error("Offline"))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ id: "a", app_workspace: workspace })),
      )
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: { ...body.workspace, revision: 1 },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(1000);
    store.dispatch(retryAppWorkspaceSave("a"));
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(JSON.parse(String(fetchMock.mock.calls[2][1]?.body))).toMatchObject({
      expected_revision: 0,
    });
    expect(store.getState().apps.saveStatus?.a).toBe("saved");
  },
);

test("retry confirms a lost acknowledgment without an unnecessary write", async () => {
  vi.useFakeTimers();
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValueOnce(new Error("Lost response"))
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({ app_workspace: remoteWorkspace(8, [0.6, 0.4]) }),
      ),
    );
  const store = setup();
  store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
  const root = store.getState().workspace.groups["chat:a"]!.root;
  store.dispatch(
    resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes: [0.6, 0.4] }),
  );
  await vi.advanceTimersByTimeAsync(1000);
  store.dispatch(retryAppWorkspaceSave("a"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(fetchMock).toHaveBeenCalledTimes(2);
  expect(store.getState().apps.saveStatus?.a).toBe("saved");
});

test("retry refresh keeps edits made while reading and does not replace a newer snapshot", async () => {
  vi.useFakeTimers();
  let resolve!: (response: Response) => void;
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValueOnce(new Error("Lost response"))
    .mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    )
    .mockImplementation((_url, options) => {
      const body = JSON.parse(String(options?.body)) as {
        workspace: AppWorkspace;
        expected_revision: number;
      };
      return Promise.resolve(
        new Response(
          JSON.stringify({
            app_workspace: {
              ...body.workspace,
              revision: body.expected_revision + 1,
            },
          }),
        ),
      );
    });
  const store = setup();
  store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
  const root = store.getState().workspace.groups["chat:a"]!.root;
  store.dispatch(
    resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes: [0.6, 0.4] }),
  );
  await vi.advanceTimersByTimeAsync(1000);
  store.dispatch(retryAppWorkspaceSave("a"));
  store.dispatch(
    resizeGroupSplit({
      tabId: "chat:a",
      splitId: root.id,
      sizes: [0.55, 0.45],
    }),
  );
  const latest = remoteWorkspace(10);
  latest.bindings[0].state = { latest: true };
  store.dispatch({ type: "test/snapshot", payload: latest });
  resolve(new Response(JSON.stringify({ app_workspace: remoteWorkspace(9) })));
  await vi.advanceTimersByTimeAsync(1000);
  expect(fetchMock).toHaveBeenCalledTimes(3);
  const saved = JSON.parse(String(fetchMock.mock.calls[2][1]?.body)) as {
    workspace: AppWorkspace;
    expected_revision: number;
  };
  expect(saved.expected_revision).toBe(10);
  expect(saved.workspace.bindings).toEqual(latest.bindings);
  expect(saved.workspace.layout).toMatchObject({ sizes: [0.55, 0.45] });
  expect(store.getState().apps.saveStatus?.a).toBe("saved");
});

test.each(["refresh", "save"] as const)(
  "failed retry %s stops without looping or losing local edits",
  async (failure) => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValueOnce(new Error("Lost response"))
      .mockResolvedValueOnce(
        failure === "refresh"
          ? new Response("{}", { status: 503 })
          : new Response(JSON.stringify({ app_workspace: remoteWorkspace(9) })),
      )
      .mockResolvedValue(
        new Response(JSON.stringify({ current_revision: 10 }), { status: 409 }),
      );
    const store = setup();
    store.dispatch({ type: "test/snapshot", payload: remoteWorkspace(7) });
    const root = store.getState().workspace.groups["chat:a"]!.root;
    store.dispatch(
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.6, 0.4],
      }),
    );
    await vi.advanceTimersByTimeAsync(1000);
    store.dispatch(retryAppWorkspaceSave("a"));
    store.dispatch(retryAppWorkspaceSave("a"));
    await vi.advanceTimersByTimeAsync(10000);
    expect(fetchMock).toHaveBeenCalledTimes(failure === "refresh" ? 2 : 3);
    expect(store.getState().apps.saveStatus?.a).toBe("error");
    expect(
      paneToAppLayout(store.getState().workspace.groups["chat:a"]!.root, "a"),
    ).toMatchObject({ sizes: [0.6, 0.4] });
  },
);

test("409 without authoritative workspace stops with retry status, never loops", async () => {
  vi.useFakeTimers();
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(
      new Response(JSON.stringify({ current_revision: 3 }), { status: 409 }),
    );
  const store = setup();
  store.dispatch(openRailApp({ appId, name: "App" }));
  await vi.advanceTimersByTimeAsync(1000);
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(store.getState().apps.saveStatus?.a).toBe("error");
});

test("rail click before initial snapshot waits and reuses authoritative revision and binding", async () => {
  vi.useFakeTimers();
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response("{}"));
  const store = setup(true, false);
  store.dispatch(openRailApp({ appId, name: "App" }));
  await vi.advanceTimersByTimeAsync(300);
  expect(fetchMock).not.toHaveBeenCalled();
  expect(Object.keys(store.getState().apps.views)).toEqual([]);
  const bindingId = "22222222-2222-4222-8222-222222222222";
  const workspace: AppWorkspace = {
    schema_version: 1,
    revision: 7,
    bindings: [{ app_id: appId, binding_id: bindingId, params: {}, state: {} }],
    layout: {
      kind: "split",
      direction: "horizontal",
      sizes: [0.72, 0.28],
      children: [
        { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }], active: 0 },
        { kind: "leaf", tabs: [{ kind: "app", binding_id: bindingId }] },
      ],
    },
  };
  store.dispatch({ type: "test/snapshot", payload: workspace });
  await vi.advanceTimersByTimeAsync(300);
  expect(Object.keys(store.getState().apps.views)).toEqual([
    `app:${bindingId}`,
  ]);
  const root = store.getState().workspace.groups["chat:a"]!.root;
  expect(paneToAppLayout(root, "a")).toMatchObject({ sizes: [0.72, 0.28] });
  store.dispatch(
    resizeGroupSplit({ tabId: "chat:a", splitId: root.id, sizes: [0.6, 0.4] }),
  );
  await vi.advanceTimersByTimeAsync(300);
  const sent = JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body)) as {
    expected_revision: number;
  };
  expect(sent.expected_revision).toBe(7);
});

const UUID_V4_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe("HTTP-compat: works without crypto.randomUUID", () => {
  let savedRandomUUID: typeof crypto.randomUUID;

  beforeEach(() => {
    savedRandomUUID = crypto.randomUUID; // eslint-disable-line @typescript-eslint/unbound-method
    Object.defineProperty(crypto, "randomUUID", {
      value: undefined,
      configurable: true,
      writable: true,
    });
  });

  afterEach(() => {
    Object.defineProperty(crypto, "randomUUID", {
      value: savedRandomUUID,
      configurable: true,
      writable: true,
    });
  });

  test("attach generates a valid uuid v4 binding_id and client_request_id", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.expected_revision + 1,
              },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(200);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const body = JSON.parse(String(fetchMock.mock.calls[0][1]?.body)) as {
      workspace: AppWorkspace;
      client_request_id: string;
    };
    expect(body.workspace.bindings).toHaveLength(1);
    expect(body.workspace.bindings[0].binding_id).toMatch(UUID_V4_RE);
    expect(body.client_request_id).toMatch(UUID_V4_RE);
  });

  test("saved close then fresh store reload restores app and removes closed app", async () => {
    vi.useFakeTimers();
    let savedWorkspace: AppWorkspace | undefined;
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
          expected_revision: number;
        };
        savedWorkspace = {
          ...body.workspace,
          revision: body.expected_revision + 1,
        };
        return Promise.resolve(
          new Response(JSON.stringify({ app_workspace: savedWorkspace })),
        );
      });
    const store1 = setup();
    store1.dispatch(openRailApp({ appId, name: "App" }));
    await vi.advanceTimersByTimeAsync(200);
    expect(fetchMock).toHaveBeenCalled();
    expect(savedWorkspace).toBeDefined();
    const newBindingId = savedWorkspace!.bindings[0].binding_id;
    expect(newBindingId).toMatch(UUID_V4_RE);

    const group1 = store1.getState().workspace.groups["chat:a"]!;
    const appKey = collectTabIds(group1.root).find((k) =>
      k.startsWith("app:"),
    )!;
    const leaf =
      group1.root.kind === "split" ? group1.root.children[1] : group1.root;
    store1.dispatch(
      closePaneTab({ tabId: "chat:a", leafId: leaf.id, surfaceKey: appKey }),
    );
    await vi.advanceTimersByTimeAsync(200);
    const closedLayout = JSON.parse(
      String(fetchMock.mock.calls.at(-1)?.[1]?.body),
    ) as { workspace: AppWorkspace };
    savedWorkspace = {
      ...closedLayout.workspace,
      revision: (savedWorkspace?.revision ?? 0) + 1,
    };

    fetchMock.mockClear();
    const store2 = setup();
    store2.dispatch({ type: "test/snapshot", payload: savedWorkspace });
    await vi.advanceTimersByTimeAsync(200);
    expect(fetchMock).not.toHaveBeenCalled();
    const layout2 = paneToAppLayout(
      store2.getState().workspace.groups["chat:a"]!.root,
      "a",
    );
    expect(layout2).toMatchObject({
      kind: "leaf",
      tabs: [{ kind: "chat" }],
    });
  });

  test("openRequestedApp creates a valid uuid v4 binding without crypto.randomUUID", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation((_url, options) => {
        const body = JSON.parse(String(options?.body)) as {
          workspace: AppWorkspace;
          expected_revision: number;
        };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              app_workspace: {
                ...body.workspace,
                revision: body.expected_revision + 1,
              },
            }),
          ),
        );
      });
    const store = setup();
    store.dispatch(openRequestedApp({ appId, name: "App", chatId: "a" }));
    await vi.advanceTimersByTimeAsync(200);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const body = JSON.parse(String(fetchMock.mock.calls[0][1]?.body)) as {
      workspace: AppWorkspace;
      client_request_id: string;
    };
    expect(body.workspace.bindings[0].binding_id).toMatch(UUID_V4_RE);
    expect(body.client_request_id).toMatch(UUID_V4_RE);
  });
});
