import { WorkspaceSurfaceRoutes } from "../features/Workspace/WorkspaceSurfaceRoutes";
import { setTaskActiveChat } from "../features/Tasks/tasksSlice";
import { act } from "react-dom/test-utils";
import { sidebarSectionSnapshotReceived } from "../features/Sidebar/sidebarSlice";
import React from "react";
import { Provider } from "react-redux";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
import type { NavigationOutcome } from "./useInternalNavigation";

const forced = vi.hoisted(() => ({
  outcome: null as NavigationOutcome | null,
  calls: 0,
}));
vi.mock("./useInternalNavigation", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("./useInternalNavigation")>();
  const redux = await import("react-redux");
  const pages = await import("../features/Pages/pagesSlice");
  return {
    ...actual,
    useInternalNavigation: (
      options: Parameters<typeof actual.useInternalNavigation>[0],
    ) => {
      const navigation = actual.useInternalNavigation(options);
      const store = redux.useStore();
      const outcome = forced.outcome;
      if (!outcome) return navigation;
      const run = () => {
        forced.calls += 1;
        if (outcome === "superseded")
          store.dispatch(pages.change({ name: "chat" }));
        return Promise.resolve(outcome);
      };
      return {
        ...navigation,
        navigate: () => run().then((result) => result === "opened"),
        navigateWithOutcome: run,
      };
    },
  };
});

function setup(
  ready = true,
  restored = true,
  host: "web" | "vscode" = "web",
  filesPanel = true,
) {
  const store = setUpStore();
  store.dispatch(updateConfig({ host, capabilities: { filesPanel } }));
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
afterEach(() => {
  forced.outcome = null;
  forced.calls = 0;
  vi.useRealTimers();
});

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

  it("reports an unrecognised address once and cleans it", async () => {
    window.history.replaceState(null, "", "/#/nonsense/route");
    const { result } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBe(
      "The address was not recognised as a content link.",
    );
    expect(window.location.hash).not.toContain("nonsense");
  });

  it("names a failed chat link with the navigation reason and does not retry it", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/trajectories/:id", () => {
        requests += 1;
        return HttpResponse.json(
          { detail: "trajectory not found" },
          { status: 404 },
        );
      }),
    );
    window.history.replaceState(null, "", "/#/chat/abcdef0123456789");
    const { result, store } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    await waitFor(() =>
      expect(result.current.error).toBe(
        "Chat abcdef01… could not be opened: trajectory not found.",
      ),
    );
    expect(window.location.hash).not.toContain("abcdef0123456789");
    expect(requests).toBe(1);
    act(() => result.current.dismissError());
    expect(result.current.error).toBeNull();
    act(() => {
      store.dispatch(updateConfig({ capabilities: { filesPanel: false } }));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBeNull();
    expect(requests).toBe(1);
    expect(window.location.hash).not.toContain("abcdef0123456789");
  });

  it("dismiss leaves a clean address and Back does not re-show the failure", async () => {
    const { result, store } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => visit("#/settings/privacy"));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("privacy settings"),
    );
    act(() => visit("#/file/relative.ts"));
    await waitFor(() =>
      expect(result.current.error).toBe(
        "File relative.ts could not be opened: the path is not a valid absolute path.",
      ),
    );
    expect(window.location.hash).toBe("#/settings/privacy");
    act(() => result.current.dismissError());
    expect(result.current.error).toBeNull();
    expect(window.location.hash).toBe("#/settings/privacy");
    act(() => visit("#/settings/shell"));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("shell settings"),
    );
    act(() => window.history.back());
    await waitFor(() =>
      expect(window.location.hash).toBe("#/settings/privacy"),
    );
    act(() => {
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBeNull();
    expect(store.getState().pages.at(-1)?.name).toBe("privacy settings");
  });

  it("keeps unrelated query parameters when a hash link fails", async () => {
    server.use(
      http.get("*/v1/trajectories/:id", () =>
        HttpResponse.json({ detail: "trajectory not found" }, { status: 404 }),
      ),
    );
    window.history.replaceState(
      null,
      "",
      "/?project=alpha&chat=other#/chat/abcdef0123456789",
    );
    const { result } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(window.location.hash).not.toContain("abcdef0123456789");
    expect(window.location.search).toBe("?project=alpha&chat=other");
  });

  it("removes only the failing chat parameter from the query", async () => {
    server.use(
      http.get("*/v1/trajectories/:id", () =>
        HttpResponse.json({ detail: "trajectory not found" }, { status: 404 }),
      ),
    );
    window.history.replaceState(
      null,
      "",
      "/?project=alpha&chat=abcdef0123456789&view=wide",
    );
    const { result } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(window.location.search).toBe("?project=alpha&view=wide");
    expect(window.location.href).not.toContain("abcdef0123456789");
  });

  it("opens a first-load file link once the files capability arrives", async () => {
    window.history.replaceState(null, "", "/#/file/%2Fworkspace%2Fa.ts");
    const { result, store } = setup(true, true, "web", false);
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBe(
      "File a.ts could not be opened: files cannot be opened in this workspace.",
    );
    act(() => {
      store.dispatch(updateConfig({ capabilities: { filesPanel: true } }));
    });
    await waitFor(() => expect(result.current.error).toBeNull());
    await waitFor(() =>
      expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(
        "file:/workspace/a.ts",
      ),
    );
    expect(window.location.hash).toBe("#/file/%2Fworkspace%2Fa.ts");
  });

  it("does not retry a file link that failed for another reason", async () => {
    window.history.replaceState(null, "", "/#/file/relative.ts");
    const { result, store } = setup(true, true, "web", false);
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toContain("not a valid absolute path");
    act(() => result.current.dismissError());
    act(() => {
      store.dispatch(updateConfig({ capabilities: { filesPanel: true } }));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBeNull();
    expect(store.getState().workspace.tabs).toHaveLength(0);
  });

  it("shows no card when the user navigates while a chat link is loading", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let started = false;
    let requests = 0;
    server.use(
      http.get("*/v1/trajectories/slow", async () => {
        started = true;
        requests += 1;
        await held;
        return HttpResponse.json({ id: "slow", title: "Slow" });
      }),
    );
    window.history.replaceState(null, "", "/#/chat/slow");
    const { result, store } = setup();
    await waitFor(() => expect(started).toBe(true));
    act(() => {
      store.dispatch(change({ name: "chat" }));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => release());
    expect(result.current.error).toBeNull();
    expect(store.getState().pages.at(-1)?.name).toBe("chat");
    expect(requests).toBe(1);
  });

  it("records a superseded chat link as handled and never re-runs it", async () => {
    const { result, store } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    forced.outcome = "superseded";
    act(() => visit("#/chat/abcdef0123456789"));
    await waitFor(() => expect(forced.calls).toBe(1));
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => {
      store.dispatch(updateConfig({ capabilities: { filesPanel: false } }));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    expect(forced.calls).toBe(1);
    expect(result.current.error).toBeNull();
  });

  it("shows no card when another navigation is already in flight", async () => {
    forced.outcome = "busy";
    window.history.replaceState(null, "", "/#/chat/abcdef0123456789");
    const { result } = setup();
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBeNull();
  });

  it.each([
    [
      "navigates inside the app",
      (env: ReturnType<typeof setup>) =>
        env.store.dispatch(change({ name: "chat" })),
    ],
    [
      "dismisses the card",
      (env: ReturnType<typeof setup>) => env.result.current.dismissError(),
    ],
    [
      "waits past the time limit",
      () => {
        vi.advanceTimersByTime(10_000);
      },
    ],
  ])(
    "stops waiting for the files capability when the user %s",
    async (_case, interrupt) => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      window.history.replaceState(null, "", "/#/file/%2Fworkspace%2Fa.ts");
      const env = setup(true, true, "web", false);
      await waitFor(() => expect(env.result.current.pending).toBe(false));
      expect(env.result.current.error).toContain("files cannot be opened");
      act(() => interrupt(env));
      act(() => {
        env.store.dispatch(
          updateConfig({ capabilities: { filesPanel: true } }),
        );
      });
      await waitFor(() => expect(env.result.current.pending).toBe(false));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
      });
      expect(env.store.getState().workspace.tabs).toHaveLength(0);
      expect(window.location.hash).not.toContain("workspace");
    },
  );

  it("stops waiting for the files capability when a newer address is applied", async () => {
    window.history.replaceState(null, "", "/#/file/%2Fworkspace%2Fa.ts");
    const { result, store } = setup(true, true, "web", false);
    await waitFor(() => expect(result.current.pending).toBe(false));
    act(() => visit("#/settings/privacy"));
    await waitFor(() =>
      expect(store.getState().pages.at(-1)?.name).toBe("privacy settings"),
    );
    act(() => {
      store.dispatch(updateConfig({ capabilities: { filesPanel: true } }));
    });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(store.getState().workspace.tabs).toHaveLength(0);
    expect(store.getState().pages.at(-1)?.name).toBe("privacy settings");
  });

  it("keeps the query string byte for byte when a hash link fails", async () => {
    window.history.replaceState(
      null,
      "",
      "/?q=a%20b&flag&chat=keep#/file/relative.ts",
    );
    const { result } = setup();
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(window.location.search).toBe("?q=a%20b&flag&chat=keep");
  });

  it("keeps the spelling of other parameters when removing a failing chat", async () => {
    server.use(
      http.get("*/v1/trajectories/:id", () =>
        HttpResponse.json({ detail: "trajectory not found" }, { status: 404 }),
      ),
    );
    window.history.replaceState(
      null,
      "",
      "/?q=a%20b&&flag&chat=abcdef0123456789&x=1+2&",
    );
    const { result } = setup();
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(window.location.search).toBe("?q=a%20b&&flag&x=1+2&");
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
