import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type AppStore, setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import {
  backUpMessages,
  createChatWithId,
  switchToThread,
} from "../features/Chat/Thread";
import { setBackendStatus } from "../features/Connection";
import { push } from "../features/Pages/pagesSlice";
import { openTask } from "../features/Tasks";
import { makeSurfaceKey, setActiveTab } from "../features/Workspace";
import {
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { render, screen, waitFor, within } from "../utils/test-utils";
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

vi.mock("../features/Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual =
    await vi.importActual<typeof import("../features/Tasks")>(
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

const chat = (id: string) => makeSurfaceKey("chat", id);

function createChat(store: AppStore, id: string, title: string) {
  store.dispatch(createChatWithId({ id, title, mode: "agent" }));
}

function keepChatOpenOnNewChat(store: AppStore, id: string) {
  store.dispatch(
    backUpMessages({
      id,
      messages: [{ role: "user", content: `Keep ${id}` }],
    }),
  );
}

function selectChat(store: AppStore, id: string) {
  store.dispatch(setActiveTab(chat(id)));
  store.dispatch(switchToThread({ id, openTab: false }));
}

function renderApp(setup: (store: AppStore) => void) {
  server.use(...appHandlers);
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
    pages: [{ name: "history" }, { name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  setup(store);

  return render(<InnerApp />, { store });
}

function selectedTabs() {
  return screen
    .getAllByRole("tab")
    .filter((tab) => tab.getAttribute("aria-selected") === "true");
}

afterEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
  vi.clearAllMocks();
});

describe("App unified tab regression", () => {
  it("renders the chat page with one unified tab row and no duplicated active label", async () => {
    const { store } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.getByRole("tablist", { name: "Open workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("tab", { name: /Chat Alpha/ })).toHaveLength(1);
    expect(screen.getAllByText("Chat Alpha")).toHaveLength(1);
    expect(
      await screen.findByRole("button", { name: "Split active tab" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });

  it("creates exactly one selected top-level tab for a new chat", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
      keepChatOpenOnNewChat(appStore, "chat-a");
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });
    await user.click(screen.getByRole("button", { name: "New Chat" }));

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).not.toBe("chat-a");
    });
    const newChatId = store.getState().chat.current_thread_id;
    const newSurfaceKey = chat(newChatId);

    await waitFor(() => {
      expect(store.getState().workspace.activeTabId).toBe(newSurfaceKey);
    });

    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      newSurfaceKey,
    ]);
    expect(new Set(store.getState().workspace.tabs).size).toBe(2);
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(selectedTabs()).toHaveLength(1);
  });

  it("switches an existing chat through one selected tab without adding a ghost tab", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
      createChat(appStore, "chat-b", "Chat Beta");
      selectChat(appStore, "chat-a");
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });
    await user.click(screen.getByRole("tab", { name: /Chat Beta/ }));

    await waitFor(() => {
      expect(store.getState().workspace.activeTabId).toBe(chat("chat-b"));
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });

    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-b"),
    ]);
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(selectedTabs()).toHaveLength(1);
    expect(selectedTabs()[0]).toHaveTextContent("Chat Beta");
    expect(screen.getAllByText("Chat Beta")).toHaveLength(1);
  });

  it("keeps task and buddy entries as navigation-only tabs in the unified bar", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
      appStore.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
      appStore.dispatch(push({ name: "buddy" }));
      appStore.dispatch(push({ name: "chat" }));
    });

    await screen.findByRole("tab", { name: /Task Alpha/ });
    expect(screen.getByRole("tab", { name: /Buddy/ })).toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);

    await user.click(screen.getByRole("tab", { name: /Task Alpha/ }));

    expect(await screen.findByTestId("task-workspace")).toHaveAttribute(
      "data-task-id",
      "task-a",
    );
    expect(store.getState().pages.at(-1)).toEqual({
      name: "task workspace",
      taskId: "task-a",
    });
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.queryByRole("button", { name: "Split active tab" }),
    ).toBeNull();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().workspace.groups).toEqual({});

    await user.click(screen.getByRole("tab", { name: /Buddy/ }));

    expect(await screen.findByTestId("buddy-home")).toBeInTheDocument();
    expect(store.getState().pages.at(-1)).toEqual({ name: "buddy" });
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Buddy/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.queryByRole("button", { name: "Split active tab" }),
    ).toBeNull();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });

  it("turns a split chat into one group tab in the same tab row", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });
    await user.click(screen.getByRole("button", { name: "Split active tab" }));

    await waitFor(() => {
      expect(store.getState().workspace.groups[chat("chat-a")]).toBeDefined();
    });

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    const groupTab = screen.getByRole("tab", { name: /Chat Alpha/ });
    expect(within(groupTab).getByLabelText("2 panes")).toHaveTextContent("2");
    expect(groupTab).toHaveAttribute("aria-selected", "true");
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });
});
