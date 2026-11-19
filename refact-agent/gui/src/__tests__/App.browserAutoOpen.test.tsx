import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import { applyChatEvent, createChatWithId } from "../features/Chat/Thread";
import { setBackendStatus } from "../features/Connection";
import { push } from "../features/Pages/pagesSlice";
import {
  selectBrowserViewOwners,
  selectWorkspaceFocusedChatId,
} from "../features/Workspace/workspaceSlice";
import {
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { render, waitFor } from "../utils/test-utils";
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
  sidebarSubscribe,
} from "../utils/mockServer";

vi.hoisted(() => {
  vi.resetModules();
});

vi.mock("../features/Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    Chat: ({ chatId }: { chatId?: string }) =>
      React.createElement(
        "section",
        { "data-testid": "chat-surface", "data-chat-id": chatId ?? "" },
        `Chat surface ${chatId ?? ""}`,
      ),
  };
});

vi.mock("../features/Tasks/TaskWorkspace", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    TaskWorkspace: ({ taskId }: { taskId: string }) =>
      React.createElement(
        "section",
        { "data-testid": "task-workspace", "data-task-id": taskId },
        `Task workspace ${taskId}`,
      ),
  };
});

vi.mock("../features/Buddy/BuddyHome", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyHome: () =>
      React.createElement(
        "section",
        { "data-testid": "buddy-home" },
        "Buddy home",
      ),
  };
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
  sidebarSubscribe,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.get("*/v1/setup/status", () =>
    HttpResponse.json({
      configured: true,
      reasons: [],
      detail: {
        project_root: "/tmp/refact-test",
        has_agents_md: true,
        has_knowledge: true,
        has_trajectories: true,
      },
    }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/chats/:chatId/skills-status", () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    }),
  ),
  http.get("*/v1/buddy/opportunities", () =>
    HttpResponse.json({ opportunities: [] }),
  ),
  http.get("*/v1/worktrees", () =>
    HttpResponse.json({
      project_hash: "test",
      source_workspace_root: "/tmp/refact-test",
      worktrees: [],
    }),
  ),
];

const baseConfig = {
  host: "vscode" as const,
  lspPort: 8001,
  apiKey: "test",
  themeProps: {},
};

const browserTab = {
  tab_id: "tab-1",
  url: "https://example.test/",
  title: "Example",
  active: true,
  name: null,
  opened_by_chat: "chat-a",
  last_chat: "chat-a",
  last_used_ms: 1,
};

const browserHandlers = [
  http.post("*/v1/browser/project/status", () =>
    HttpResponse.json({
      project_id: "refact-test",
      running: true,
      connected: true,
      runtime_id: "runtime-a",
      active_tab: "tab-1",
      tabs: [browserTab],
    }),
  ),
  http.post("*/v1/browser/project/resize", () =>
    HttpResponse.json({ applied: true }),
  ),
  http.post("*/v1/browser/project/frame", () =>
    HttpResponse.json({
      runtime_id: "runtime-a",
      tab_id: "tab-1",
      mime: "image/png",
      data: "iVBORw0KGgo=",
      width: 600,
      height: 300,
      viewport: { width: 1200, height: 600 },
      url: "https://example.test/",
      title: "Example",
    }),
  ),
];

afterEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
});

describe("App project browser auto-open", () => {
  it("opens a chat's browser view while another page is shown, without changing the page", async () => {
    server.use(...appHandlers, ...browserHandlers);
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const store = setUpStore({
      config: baseConfig,
      current_project: {
        name: "refact-test",
        workspaceRoots: ["/tmp/refact-test"],
      },
      pages: [{ name: "history" }],
    });
    store.dispatch(setBackendStatus({ status: "online" }));
    store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat A", mode: "agent" }),
    );
    store.dispatch(
      applyChatEvent({
        chat_id: "chat-a",
        seq: "1",
        type: "snapshot",
        thread: {
          id: "chat-a",
          title: "Chat A",
          model: "test-model",
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
        messages: [],
        background_agents: [],
      }),
    );
    render(<InnerApp />, { store });
    await new Promise((resolve) => setTimeout(resolve, 200));
    const pagesBefore = store.getState().pages;
    expect(pagesBefore.at(-1)).toEqual({ name: "history" });
    const focusedBefore = selectWorkspaceFocusedChatId(store.getState());

    store.dispatch(
      applyChatEvent({
        chat_id: "chat-a",
        seq: "2",
        type: "message_added",
        index: 0,
        message: {
          role: "tool",
          tool_call_id: "call-1",
          content: "done",
          browser_project: {
            open_browser_tab: true,
            runtime_id: "runtime-a",
            tab_id: "tab-1",
            tab_name: null,
          },
        } as never,
      }),
    );

    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-1": "chat-a",
      }),
    );
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(store.getState().pages).toEqual(pagesBefore);
    expect(selectWorkspaceFocusedChatId(store.getState())).toBe(focusedBefore);
    expect(
      document.querySelector('[data-browser-tab-id="tab-1"]'),
    ).not.toBeInTheDocument();

    store.dispatch(push({ name: "chat" }));
    await waitFor(() =>
      expect(
        document.querySelector('[data-browser-tab-id="tab-1"]'),
      ).toBeInTheDocument(),
    );
  });
});
