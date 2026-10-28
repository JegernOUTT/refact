import React from "react";
import { act } from "react-dom/test-utils";
import { renderHook, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { Provider } from "react-redux";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { applyChatEvent } from "../features/Chat/Thread";
import { replaceSnapshotHistory } from "../features/History/historySlice";
import { setBackendStatus } from "../features/Connection";
import { sidebarSectionSnapshotReceived } from "../features/Sidebar/sidebarSlice";
import { type Config } from "../features/Config/configSlice";
import { InnerApp } from "../features/App";
import { useChatDeepLink } from "../hooks/useChatDeepLink";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";
import {
  getProjectStorageNamespace,
  savePersistedActiveTab,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import {
  chatLinks,
  chatSessionAbort,
  chatSessionCommand,
  chatSessionSubscribe,
  emptyTasks,
  goodCaps,
  goodPing,
  goodPrompts,
  goodTools,
  goodUser,
  noCommandPreview,
  noCompletions,
  server,
} from "../utils/mockServer";
import { render, screen } from "../utils/test-utils";

vi.mock("../features/Buddy/BuddyHome", async () => {
  const actual = await vi.importActual<typeof import("react")>("react");
  return {
    BuddyHome: () =>
      actual.createElement("section", { "data-testid": "buddy-home" }, "Buddy"),
  };
});

vi.mock("../features/Workspace/WorkspaceView", async () => {
  const actual = await vi.importActual<typeof import("react")>("react");
  return {
    WorkspaceView: () =>
      actual.createElement(
        "section",
        { "data-testid": "workspace-view" },
        "Chat",
      ),
  };
});

vi.mock("../hooks", async () => {
  const actual = await vi.importActual<typeof import("../hooks")>("../hooks");
  return {
    ...actual,
    useProviderBootstrapState: () => ({
      status: "ready",
      canAccessApp: true,
      retry: () => undefined,
      isRetrying: false,
    }),
  };
});

const CHAT_ID = "chat-42";

type SnapshotEnvelope = Extract<ChatEventEnvelope, { type: "snapshot" }>;

function snapshot(messages: ChatMessage[]): SnapshotEnvelope {
  return {
    chat_id: CHAT_ID,
    seq: "1",
    type: "snapshot",
    thread: {
      id: CHAT_ID,
      title: "Deep linked chat",
      model: "claude",
      mode: "AGENT",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: true,
      is_title_generated: false,
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    background_agents: [],
    messages,
  };
}

function makeConfig(overrides: Partial<Config> = {}): Config {
  return {
    host: "web",
    engineServed: true,
    lspPort: 8001,
    themeProps: {},
    ...overrides,
  };
}

function trackTrajectoryRequests() {
  const counts = { full: 0, path: 0 };
  server.use(
    http.get(`*/v1/trajectories/:id`, () => {
      counts.full += 1;
      return HttpResponse.json({ detail: "unexpected" }, { status: 500 });
    }),
  );
  return counts;
}

function renderDeepLink(config: Config, ready = true) {
  const store = setUpStore({ config, pages: [{ name: "history" }] });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  const hook = renderHook(
    ({ isReady }: { isReady: boolean }) => useChatDeepLink(isReady),
    { wrapper, initialProps: { isReady: ready } },
  );
  return { store, ...hook };
}

function markWorkspaceReady(store: ReturnType<typeof setUpStore>) {
  act(() => {
    store.dispatch(
      sidebarSectionSnapshotReceived({ section: "workspace", status: "ready" }),
    );
  });
}

beforeEach(() => {
  window.history.replaceState(null, "", "/");
});

afterEach(() => {
  window.history.replaceState(null, "", "/");
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
});

describe("useChatDeepLink", () => {
  it("opens the linked chat from the subscription without fetching the trajectory", async () => {
    const counts = trackTrajectoryRequests();
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}&keep=1`);

    const { store, result } = renderDeepLink(makeConfig());
    expect(result.current.resolving).toBe(true);
    markWorkspaceReady(store);

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    });
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    expect(store.getState().chat.open_thread_ids).toContain(CHAT_ID);
    expect(result.current.resolving).toBe(false);
    expect(result.current.pending).toBe(true);
    expect(window.location.search).toBe(`?chat=${CHAT_ID}&keep=1`);

    act(() => {
      store.dispatch(
        applyChatEvent(snapshot([{ role: "user", content: "hello" }])),
      );
    });

    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(window.location.search).toBe("?keep=1");
    expect(result.current.error).toBeNull();
    expect(counts.full).toBe(0);
  });

  it("uses history metadata when the chat is already listed", async () => {
    const counts = trackTrajectoryRequests();
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);
    const { store, result } = renderDeepLink(makeConfig());
    act(() => {
      store.dispatch(
        replaceSnapshotHistory({
          items: [
            {
              id: CHAT_ID,
              title: "Listed chat",
              model: "claude",
              mode: "AGENT",
              created_at: "2026-07-18T00:00:00Z",
              updated_at: "2026-07-18T01:00:00Z",
              message_count: 2,
              total_lines_added: 0,
              total_lines_removed: 0,
              tasks_total: 0,
              tasks_done: 0,
              tasks_failed: 0,
            },
          ],
        }),
      );
    });
    markWorkspaceReady(store);

    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    expect(store.getState().chat.threads[CHAT_ID]?.thread.title).toBe(
      "Listed chat",
    );
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    expect(window.location.search).toBe("");
    expect(counts.full).toBe(0);
  });

  it("ignores the param on IDE hosts", async () => {
    const counts = trackTrajectoryRequests();
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store, result } = renderDeepLink(makeConfig({ host: "jetbrains" }));
    markWorkspaceReady(store);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(result.current.pending).toBe(false);
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();
    expect(window.location.search).toBe(`?chat=${CHAT_ID}`);
    expect(counts.full).toBe(0);
  });

  it("shows a failure and returns to the dashboard when the chat is unknown", async () => {
    const counts = trackTrajectoryRequests();
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}/path`, () => {
        counts.path += 1;
        return HttpResponse.json(
          { detail: "Trajectory not found" },
          { status: 404 },
        );
      }),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store, result } = renderDeepLink(makeConfig());
    markWorkspaceReady(store);
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    });

    act(() => {
      store.dispatch(applyChatEvent(snapshot([])));
    });

    await waitFor(() => {
      expect(result.current.error).toBe(`Chat not found: ${CHAT_ID}`);
    });
    expect(result.current.pending).toBe(false);
    expect(counts.path).toBe(1);
    expect(counts.full).toBe(0);
    expect(window.location.search).toBe("");
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();
    expect(store.getState().chat.open_thread_ids).not.toContain(CHAT_ID);
    expect(store.getState().pages.at(-1)).toEqual({ name: "history" });
  });

  it("keeps an existing empty chat open", async () => {
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}/path`, () =>
        HttpResponse.json({ path: "/tmp/chat-42.json" }),
      ),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store, result } = renderDeepLink(makeConfig());
    markWorkspaceReady(store);
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    });
    act(() => {
      store.dispatch(applyChatEvent(snapshot([])));
    });

    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.error).toBeNull();
    expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    expect(window.location.search).toBe("");
  });

  it("waits for readiness before opening and keeps the param", async () => {
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store, result, rerender } = renderDeepLink(makeConfig(), false);
    markWorkspaceReady(store);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(result.current.resolving).toBe(false);
    expect(window.location.search).toBe(`?chat=${CHAT_ID}`);
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();

    rerender({ isReady: true });
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    });
  });
});

const appHandlers = [
  goodPing,
  goodUser,
  goodCaps,
  goodTools,
  goodPrompts,
  chatLinks,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
  noCommandPreview,
  noCompletions,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/buddy/opportunities", () =>
    HttpResponse.json({ opportunities: [] }),
  ),
  http.get("*/v1/sidebar/subscribe", () => {
    const encoder = new TextEncoder();
    const event = {
      protocol_version: 2,
      seq: 0,
      subscription_id: "test-sidebar",
      event: {
        type: "section_snapshot",
        section: "workspace",
        status: "ready",
        snapshot: { workspace_roots: ["/tmp/project-a"] },
      },
    };
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(
          encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
        );
      },
    });
    return new HttpResponse(stream, {
      headers: { "Content-Type": "text/event-stream" },
    });
  }),
];

describe("App chat deep link", () => {
  it("opens the linked chat even when a Buddy tab is persisted", async () => {
    const counts = trackTrajectoryRequests();
    server.use(...appHandlers);
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/project-a"],
      projectName: "project-a",
    });
    expect(getProjectStorageNamespace()).not.toBeNull();
    savePersistedActiveTab({ type: "buddy" });
    setProjectStorageNamespace(undefined);
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const store = setUpStore({
      config: makeConfig({ apiKey: "test" }),
      pages: [{ name: "history" }],
    });
    store.dispatch(setBackendStatus({ status: "online" }));
    render(<InnerApp />, { store });

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    });
    expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);

    act(() => {
      store.dispatch(
        applyChatEvent(snapshot([{ role: "user", content: "hello" }])),
      );
    });

    expect(await screen.findByTestId("workspace-view")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText("Opening chat…")).not.toBeInTheDocument();
    });
    expect(
      screen.queryByText("Opening linked target…"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("buddy-home")).not.toBeInTheDocument();
    expect(store.getState().pages.some((page) => page.name === "buddy")).toBe(
      false,
    );
    expect(window.location.search).toBe("");
    expect(counts.full).toBe(0);
  });
});
