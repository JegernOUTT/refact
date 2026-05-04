import React from "react";
import { describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";

import { setUpStore } from "../app/store";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { server } from "../utils/mockServer";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import { setBackendStatus } from "../features/Connection";

function sidebarSnapshotHandler(snapshot: Record<string, unknown>) {
  return http.get("http://127.0.0.1:8001/v1/sidebar/subscribe", () => {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(
          encoder.encode(`data: ${JSON.stringify(snapshot)}\n\n`),
        );
      },
    });

    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  });
}

function emptyTrajectoriesHandler(status = 200) {
  return http.get("http://127.0.0.1:8001/v1/trajectories", () => {
    if (status !== 200) {
      return HttpResponse.json({ detail: "failed" }, { status });
    }

    return HttpResponse.json({
      items: [],
      next_cursor: null,
      has_more: false,
    });
  });
}

function renderSidebarSubscription() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );

  renderHook(() => useSidebarSubscription(), { wrapper });

  return store;
}

describe("useSidebarSubscription", () => {
  it("marks the server snapshot as received without clearing local project info when workspace_roots is omitted", async () => {
    server.use(
      emptyTrajectoriesHandler(),
      sidebarSnapshotHandler({
        seq: 0,
        category: "snapshot",
        trajectories: [],
        tasks: [],
      }),
    );

    const store = renderSidebarSubscription();
    store.dispatch(
      setCurrentProjectInfo({
        name: "local-refact",
        workspaceRoots: ["/local/refact"],
      }),
    );

    await waitFor(() => {
      expect(store.getState().current_project.serverSnapshotReceived).toBe(
        true,
      );
    });

    expect(store.getState().current_project).toEqual({
      name: "local-refact",
      workspaceRoots: ["/local/refact"],
      serverSnapshotReceived: true,
    });
  });

  it("accepts an explicit empty server workspace snapshot as loaded", async () => {
    server.use(
      emptyTrajectoriesHandler(),
      sidebarSnapshotHandler({
        seq: 0,
        category: "snapshot",
        trajectories: [],
        tasks: [],
        workspace_roots: [],
      }),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().current_project).toEqual({
        name: "",
        workspaceRoots: [],
        serverSnapshotReceived: true,
      });
    });
  });

  it("keeps history loading false after the initial history request fails", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockReturnValue("true");
    server.use(
      emptyTrajectoriesHandler(500),
      sidebarSnapshotHandler({
        seq: 0,
        category: "snapshot",
        trajectories: [],
        tasks: [],
        workspace_roots: ["/workspace/refact"],
      }),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().history.isLoading).toBe(false);
    });
  });

  it("does not return to project loading when local IDE project info arrives after a server snapshot", async () => {
    server.use(
      emptyTrajectoriesHandler(),
      sidebarSnapshotHandler({
        seq: 0,
        category: "snapshot",
        trajectories: [],
        tasks: [],
        workspace_roots: ["/workspace/refact"],
      }),
    );

    const store = renderSidebarSubscription();
    store.dispatch(setBackendStatus({ status: "online" }));

    await waitFor(() => {
      expect(store.getState().current_project.serverSnapshotReceived).toBe(
        true,
      );
    });

    store.dispatch(
      setCurrentProjectInfo({
        name: "ide-refact",
        workspaceRoots: ["/ide/refact"],
      }),
    );

    expect(store.getState().current_project.serverSnapshotReceived).toBe(true);
  });
});
