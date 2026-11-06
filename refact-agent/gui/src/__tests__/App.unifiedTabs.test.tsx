import { openFileInFilesPanel } from "../features/Workspace/FilesPanel/filesPanelSlice";
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
import { collectTabIds, findLeaf } from "../features/ChatPanes/panesTree";
import {
  makeSurfaceKey,
  setPanelsForced,
  setActiveTab,
  openTab,
} from "../features/Workspace";
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

function getChatGroup(store: AppStore) {
  const group =
    store.getState().workspace.groups[makeSurfaceKey("chat", "chat-a")];
  if (!group) throw new Error("Missing canonical chat group");
  return group;
}

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
      screen.getByRole("tablist", { name: "Workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("tab", { name: /Chat Alpha/ })).toHaveLength(1);
    expect(
      screen
        .getAllByText("Chat Alpha")
        .filter((node) => !node.closest('[aria-hidden="true"]')),
    ).toHaveLength(1);
    expect(
      await screen.findByRole("button", { name: "Pane actions" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });

  it("creates exactly one selected global tab for a new chat", async () => {
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
      expect(findLeaf(getChatGroup(store).root, "root")?.activeTabId).toBe(
        newSurfaceKey,
      );
    });

    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(collectTabIds(getChatGroup(store).root)).toEqual([
      chat("chat-a"),
      newSurfaceKey,
    ]);
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(selectedTabs()).toHaveLength(1);
  });

  it("keeps both local file tabs when Toolbar New Chat replaces an empty chat", async () => {
    const files = ["file:/project/first.ts", "file:/project/second.ts"];
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Empty chat");
      appStore.dispatch(setPanelsForced(true));
      files.forEach((key) =>
        appStore.dispatch(openFileInFilesPanel({ path: key.slice(5) })),
      );
      selectChat(appStore, "chat-a");
    });
    await screen.findByRole("tab", { name: /Empty chat/ });
    expect(screen.getByRole("tab", { name: "first.ts" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "second.ts" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New Chat" }));
    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).not.toBe("chat-a"),
    );
    const group = Object.values(store.getState().workspace.groups)[0];
    expect(group).toBeDefined();
    expect(group && collectTabIds(group.root)).toEqual(
      expect.arrayContaining(files),
    );
    expect(screen.getByRole("tab", { name: "first.ts" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "second.ts" })).toBeInTheDocument();
    expect(
      screen.getAllByRole("tablist", { name: "Workspace tabs" }),
    ).toHaveLength(1);
    expect(selectedTabs()).toHaveLength(1);
    expect(
      group && findLeaf(group.root, group.focusedLeafId)?.activeTabId,
    ).toBe(chat(store.getState().chat.current_thread_id));
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
      expect(findLeaf(getChatGroup(store).root, "root")?.activeTabId).toBe(
        chat("chat-b"),
      );
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });

    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(collectTabIds(getChatGroup(store).root)).toEqual([
      chat("chat-a"),
      chat("chat-b"),
    ]);
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(selectedTabs()).toHaveLength(1);
    expect(selectedTabs()[0]).toHaveTextContent("Chat Beta");
    expect(
      screen
        .getAllByText("Chat Beta")
        .filter((node) => !node.closest('[aria-hidden="true"]')),
    ).toHaveLength(1);
  });

  it("keeps task and Buddy entries in the same tab band without changing pane ownership", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
      appStore.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
      appStore.dispatch(openTab("buddy:home"));
      appStore.dispatch(push({ name: "chat" }));
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });
    const paneRow = screen.getByRole("tablist", { name: "Workspace tabs" });
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      within(paneRow).getByRole("tab", { name: /Task Alpha/ }),
    ).toBeInTheDocument();
    expect(
      within(paneRow).getByRole("tab", { name: /Buddy/ }),
    ).toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);

    const paneOrder = collectTabIds(getChatGroup(store).root);
    await user.click(screen.getByRole("tab", { name: /Task Alpha/ }));

    expect(await screen.findByTestId("task-workspace")).toHaveAttribute(
      "data-task-id",
      "task-a",
    );
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    expect(findLeaf(getChatGroup(store).root, "root")?.activeTabId).toBe(
      "task:task-a",
    );
    expect(selectedTabs()).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.getByRole("tablist", { name: "Workspace tabs" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Pane actions" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(collectTabIds(getChatGroup(store).root)).toEqual(paneOrder);

    await user.click(screen.getByRole("tab", { name: /Buddy/ }));

    expect(await screen.findByTestId("buddy-home")).toBeInTheDocument();
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    expect(findLeaf(getChatGroup(store).root, "root")?.activeTabId).toBe(
      "buddy:home",
    );
    expect(collectTabIds(getChatGroup(store).root)).toEqual(paneOrder);
    expect(selectedTabs()).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Buddy/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.getByRole("tablist", { name: "Workspace tabs" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Pane actions" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });

  it("keeps one global toolbar row when splitting a chat", async () => {
    const { store, user } = renderApp((appStore) => {
      createChat(appStore, "chat-a", "Chat Alpha");
    });

    await screen.findByRole("tab", { name: /Chat Alpha/ });
    await user.click(screen.getByRole("button", { name: "Pane actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Split Right" }));

    await waitFor(() => {
      expect(store.getState().workspace.groups[chat("chat-a")]).toBeDefined();
    });

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    const row = screen.getByRole("tablist", { name: "Workspace tabs" });
    expect(within(row).getAllByRole("tab")).toHaveLength(1);
    expect(row.closest("[data-workspace-leaf-id]")).toBeNull();
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
    for (const pane of document.querySelectorAll("[data-workspace-leaf-id]")) {
      expect(within(pane as HTMLElement).queryByRole("tablist")).toBeNull();
    }
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
  });
});
