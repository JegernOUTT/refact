import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import { restoreChat } from "../features/Chat/Thread";
import type { ChatHistoryItem } from "../features/History/historySlice";
import { setBackendStatus } from "../features/Connection";
import { render, screen } from "../utils/test-utils";
import { openTask } from "../features/Tasks";
import {
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
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

vi.mock("../features/Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const thread = await vi.importActual<
    typeof import("../features/Chat/Thread")
  >("../features/Chat/Thread");
  const selectorHook = await vi.importActual<
    typeof import("../hooks/useAppSelector")
  >("../hooks/useAppSelector");

  return {
    Chat: ({ chatId }: { chatId?: string }) => {
      const currentThreadId = selectorHook.useAppSelector(
        thread.selectCurrentThreadId,
      );
      const resolvedChatId = chatId ?? currentThreadId;
      const messages = selectorHook.useAppSelector((state) =>
        thread.selectMessagesById(state, resolvedChatId),
      );

      return React.createElement(
        "section",
        { "data-testid": "single-chat", "data-chat-id": resolvedChatId },
        messages.map((message, index) =>
          React.createElement(
            "p",
            {
              key:
                "message_id" in message && message.message_id
                  ? message.message_id
                  : index,
            },
            typeof message.content === "string" ? message.content : "",
          ),
        ),
      );
    },
  };
});

vi.mock("../features/Workspace/WorkspaceView", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const thread = await vi.importActual<
    typeof import("../features/Chat/Thread")
  >("../features/Chat/Thread");
  const selectorHook = await vi.importActual<
    typeof import("../hooks/useAppSelector")
  >("../hooks/useAppSelector");

  return {
    WorkspaceView: () => {
      const currentThreadId = selectorHook.useAppSelector(
        thread.selectCurrentThreadId,
      );
      const messages = selectorHook.useAppSelector((state) =>
        thread.selectMessagesById(state, currentThreadId),
      );

      return React.createElement(
        "section",
        { "data-testid": "workspace-view", "data-chat-id": currentThreadId },
        messages.map((message, index) =>
          React.createElement(
            "p",
            {
              key:
                "message_id" in message && message.message_id
                  ? message.message_id
                  : index,
            },
            typeof message.content === "string" ? message.content : "",
          ),
        ),
      );
    },
  };
});

vi.mock("../features/Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual = await vi.importActual<typeof import("../features/Tasks")>(
    "../features/Tasks",
  );

  return {
    ...actual,
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

function chatHistoryItem({
  id,
  content,
  buddy,
}: {
  id: string;
  content: string;
  buddy: boolean;
}): ChatHistoryItem {
  return {
    id,
    title: buddy ? "Buddy Investigation" : "Normal Chat",
    model: "",
    mode: buddy ? "buddy" : "agent",
    tool_use: "agent",
    messages: [
      {
        role: "assistant",
        content,
        message_id: `${id}-message`,
      },
    ],
    boost_reasoning: false,
    context_tokens_cap: undefined,
    include_project_info: true,
    increase_max_tokens: false,
    last_user_message_id: "",
    createdAt: "2024-01-01T00:00:00Z",
    updatedAt: "2024-01-01T00:00:00Z",
    buddy_meta: buddy
      ? {
          is_buddy_chat: true,
          buddy_chat_kind: "investigation",
          workflow_id: null,
        }
      : undefined,
  };
}

function renderChatPage(item: ChatHistoryItem) {
  server.use(...appHandlers);
  const store = setUpStore({
    config: baseConfig,
    current_project: {
      name: "refact-test",
      workspaceRoots: ["/tmp/refact-test"],
    },
    pages: [{ name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots: ["/tmp/refact-test"],
    projectName: "refact-test",
  });
  store.dispatch(restoreChat(item));

  return render(<InnerApp />, { store });
}

function renderAppWithNavigationTabs() {
  server.use(...appHandlers);
  const store = setUpStore({
    config: baseConfig,
    current_project: {
      name: "refact-test",
      workspaceRoots: ["/tmp/refact-test"],
    },
    pages: [{ name: "history" }, { name: "buddy" }, { name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots: ["/tmp/refact-test"],
    projectName: "refact-test",
  });

  return render(<InnerApp />, { store });
}

afterEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
  vi.clearAllMocks();
});

describe("App buddy chat page rendering", () => {
  it("renders the current buddy chat inside the workspace view", async () => {
    renderChatPage(
      chatHistoryItem({
        id: "buddy-chat-1",
        content: "Buddy investigation transcript squeak",
        buddy: true,
      }),
    );

    const workspaceView = await screen.findByTestId("workspace-view");

    expect(workspaceView).toHaveAttribute("data-chat-id", "buddy-chat-1");
    expect(workspaceView).toHaveTextContent(
      "Buddy investigation transcript squeak",
    );
    expect(screen.queryByTestId("single-chat")).not.toBeInTheDocument();
  });

  it("keeps normal current chats on the workspace view", async () => {
    renderChatPage(
      chatHistoryItem({
        id: "normal-chat-1",
        content: "Normal transcript stays pane routed",
        buddy: false,
      }),
    );

    const workspaceView = await screen.findByTestId("workspace-view");

    expect(workspaceView).toHaveAttribute("data-chat-id", "normal-chat-1");
    expect(screen.queryByTestId("single-chat")).not.toBeInTheDocument();
  });

  it("routes task and buddy tabs to their existing full pages", async () => {
    const { store, user } = renderAppWithNavigationTabs();

    await screen.findByTestId("workspace-view");
    await user.click(screen.getByRole("tab", { name: /Task Alpha/ }));

    expect(store.getState().pages.at(-1)).toEqual({
      name: "task workspace",
      taskId: "task-a",
    });
    expect(await screen.findByTestId("task-workspace")).toHaveAttribute(
      "data-task-id",
      "task-a",
    );
    expect(screen.queryByTestId("workspace-view")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /Buddy/ }));

    expect(store.getState().pages.at(-1)).toEqual({ name: "buddy" });
    expect(await screen.findByTestId("buddy-home")).toBeInTheDocument();
    expect(screen.queryByTestId("workspace-view")).not.toBeInTheDocument();
  });
});
