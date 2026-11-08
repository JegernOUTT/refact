import { WorkspaceSurfaceRoutes } from "../features/Workspace/WorkspaceSurfaceRoutes";
import { setTaskActiveChat } from "../features/Tasks/tasksSlice";
import { act } from "react-dom/test-utils";
import { sidebarSectionSnapshotReceived } from "../features/Sidebar/sidebarSlice";
import React from "react";
import { Provider } from "react-redux";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../app/store";
import { change } from "../features/Pages/pagesSlice";
import { updateConfig } from "../features/Config/configSlice";
import {
  openTab,
  selectFocusedWorkspaceSurfaceKey,
} from "../features/Workspace/workspaceSlice";
import { server } from "../utils/mockServer";
import { useContentRouting } from "./useContentRouting";

function setup(ready = true, restored = true, host: "web" | "vscode" = "web") {
  const store = setUpStore();
  store.dispatch(updateConfig({ host, capabilities: { filesPanel: true } }));
  store.dispatch(change({ name: "history" }));
  store.dispatch(
    sidebarSectionSnapshotReceived({ section: "workspace", status: "ready" }),
  );
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>
      <WorkspaceSurfaceRoutes>{children}</WorkspaceSurfaceRoutes>
    </Provider>
  );
  const hook = renderHook(
    ({ ready, restored }) => useContentRouting(ready, restored),
    { wrapper, initialProps: { ready, restored } },
  );
  return { ...hook, store };
}
function visit(hash: string) {
  window.history.pushState(null, "", `/${hash}`);
  window.dispatchEvent(new PopStateEvent("popstate"));
}
beforeEach(() => window.history.replaceState(null, "", "/"));

describe("useContentRouting", () => {
  it("waits for readiness then opens a direct page without pushing a duplicate", async () => {
    window.history.replaceState(null, "", "/?token=secret#/settings/privacy");
    const { result, rerender, store } = setup(false, false);
    expect(store.getState().pages.at(-1)?.name).toBe("history");
    rerender({ ready: true, restored: true });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(store.getState().pages.at(-1)?.name).toBe("privacy settings");
    expect(window.location.hash).toBe("#/settings/privacy");
    expect(window.location.search).toBe("");
  });
  it("tracks focus, restores Back/Forward destinations and preserves unrelated tabs", async () => {
    const { result, store } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => {
      store.dispatch(openTab("file:/workspace/a.ts"));
      store.dispatch(change({ name: "chat" }));
    });
    await waitFor(() =>
      expect(window.location.hash).toBe("#/file/%2Fworkspace%2Fa.ts"),
    );
    act(() => visit("#/settings/privacy"));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("privacy settings"),
    );
    act(() => visit("#/file/%2Fworkspace%2Fa.ts"));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("chat"),
    );
    expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(
      "file:/workspace/a.ts",
    );
    expect(store.getState().workspace.tabs).not.toHaveLength(0);
  });
  it("uses validated metadata for legacy chat and fails safely on unavailable targets", async () => {
    server.use(
      http.get("*/v1/trajectories/linked", () =>
        HttpResponse.json({
          id: "linked",
          title: "Linked",
          model: "test",
          mode: "agent",
        }),
      ),
    );
    window.history.replaceState(null, "", "/?chat=linked&token=secret");
    const { store, result } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(store.getState().chat.current_thread_id).toBe("linked");
    expect(window.location.href).not.toContain("secret");
    act(() => visit("#/app/absent"));
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toContain("unavailable");
    expect(store.getState().chat.current_thread_id).toBe("linked");
  });
  it("cancels held metadata when browser navigation changes destination", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let started = false;
    server.use(
      http.get("*/v1/trajectories/slow", async () => {
        started = true;
        await held;
        return HttpResponse.json({ id: "slow", title: "Slow" });
      }),
    );
    window.history.replaceState(null, "", "/#/chat/slow");
    const { result, store } = setup();
    await waitFor(() => expect(started).toBe(true));
    act(() => visit("#/settings/privacy"));
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => release());
    expect(store.getState().pages.at(-1)?.name).toBe("privacy settings");
    expect(window.location.hash).toBe("#/settings/privacy");
  });
  it("preserves task planner identity through surface routing and subsequent chat selections", async () => {
    server.use(
      http.get("*/v1/trajectories/:id", ({ params }) =>
        HttpResponse.json({
          id: params.id,
          title: "Planner",
          model: "test",
          mode: "task_planner",
          task_meta: { task_id: "task-1", role: "planner" },
        }),
      ),
      http.get("*/v1/tasks/task-1", () =>
        HttpResponse.json({ meta: { id: "task-1", name: "Task" } }),
      ),
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json({ cards: [], columns: [] }),
      ),
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([{ id: "planner-1" }, { id: "planner-2" }]),
      ),
    );
    window.history.replaceState(null, "", "/#/chat/planner-1");
    const { result, store } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("chat"),
    );
    expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(
      "task:task-1",
    );
    expect(window.location.hash).toBe("#/chat/planner-1");
    act(() => {
      store.dispatch(
        setTaskActiveChat({
          taskId: "task-1",
          activeChat: { type: "planner", chatId: "planner-2" },
        }),
      );
    });
    await waitFor(() => expect(window.location.hash).toBe("#/chat/planner-2"));
    act(() => visit("#/task/task-1"));
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(window.location.hash).toBe("#/task/task-1");
  });

  it("never reads or writes web routes in IDE hosts", async () => {
    window.history.replaceState(null, "", "/#/settings/privacy");
    const { store, result } = setup(true, true, "vscode");
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.hasInitialTarget).toBe(false);
    expect(store.getState().pages.at(-1)?.name).toBe("history");
  });
});
