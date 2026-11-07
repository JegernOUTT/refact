import { describe, expect, it } from "vitest";
import { act } from "react-dom/test-utils";

import { type AppStore, setUpStore } from "../../app/store";
import { render, screen } from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import { updateConfig } from "../Config/configSlice";
import { popBackTo, push } from "../Pages/pagesSlice";
import { openTask, setTaskActiveChat } from "../Tasks/tasksSlice";
import { openFileInFilesPanel } from "./FilesPanel/filesPanelSlice";
import { openGitLog } from "./GitPanel/gitPanelSlice";
import { makeSurfaceKey } from "./surfaceKey";
import { WorkspaceShell } from "./WorkspaceShell";
import {
  openTab,
  selectFocusedWorkspaceChatId,
  selectFocusedWorkspaceSurfaceKey,
  selectSurfaceChatId,
  selectVisibleSurfaceKeys,
  selectWorkspaceFocusedChatId,
  setActiveTab,
  setDockOpen,
  setDockSection,
} from "./workspaceSlice";

const chat = (id: string) => makeSurfaceKey("chat", id);

function createStore(): AppStore {
  const store = setUpStore({ pages: [{ name: "history" }] });
  store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(setActiveTab(chat("chat-a")));
  store.dispatch(
    createChatWithId({
      id: "planner-1",
      title: "Planner",
      openTab: false,
      isTaskChat: true,
      taskMeta: { task_id: "task-1", role: "planner" },
    }),
  );
  store.dispatch(
    createChatWithId({
      id: "agent-1",
      title: "Agent",
      openTab: false,
      isTaskChat: true,
      taskMeta: { task_id: "task-1", role: "agent", card_id: "card-1" },
    }),
  );
  store.dispatch(openTask({ id: "task-1", name: "Task One" }));
  store.dispatch(
    setTaskActiveChat({
      taskId: "task-1",
      activeChat: { type: "planner", chatId: "planner-1" },
    }),
  );
  return store;
}

function Page({ store }: { store: AppStore }) {
  const pages = store.getState().pages;
  return <div data-testid="page">{pages[pages.length - 1]?.name}</div>;
}

function renderShell(store: AppStore) {
  return render(
    <WorkspaceShell>
      <Page store={store} />
    </WorkspaceShell>,
    { store },
  );
}

function goTo(store: AppStore, page: Parameters<typeof push>[0]) {
  act(() => {
    store.dispatch(popBackTo({ name: "history" }));
    if (page.name !== "history") store.dispatch(push(page));
  });
}

describe("WorkspaceShell", () => {
  it("closes initial narrow state even when the stored section is unavailable, then permits first manual open", async () => {
    const original = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: query === "(max-width: 767px)",
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => true,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    })) as typeof window.matchMedia;
    try {
      const store = createStore();
      store.dispatch(
        updateConfig({
          host: "web",
          capabilities: { filesPanel: false, gitPanel: false },
        }),
      );
      const width = store.getState().workspace.dock?.width;
      const view = renderShell(store);
      expect(store.getState().workspace.dock?.open).toBe(false);
      expect(store.getState().workspace.dock?.section).toBe("files");
      expect(screen.queryByRole("dialog")).toBeNull();
      await view.user.click(screen.getByRole("button", { name: /^Agents$/ }));
      expect(await screen.findByRole("dialog")).toBeVisible();
      expect(store.getState().workspace.dock?.open).toBe(true);
      expect(store.getState().workspace.dock?.width).toBe(width);
    } finally {
      window.matchMedia = original;
    }
  });

  it.each([
    { name: "history" },
    { name: "chat" },
    { name: "tasks list" },
    { name: "task workspace", taskId: "task-1" },
    { name: "knowledge graph" },
    { name: "buddy" },
  ] as const)("renders one rail and dock on the $name page", (page) => {
    const store = createStore();
    goTo(store, page);
    renderShell(store);

    expect(
      screen.getAllByRole("navigation", { name: "Workspace sections" }),
    ).toHaveLength(1);
    expect(screen.getAllByLabelText("Workspace dock")).toHaveLength(1);
  });

  it("keeps the same dock element across page navigation", () => {
    const store = createStore();
    goTo(store, { name: "chat" });
    store.dispatch(setDockSection("agents"));
    const view = renderShell(store);
    const dock = screen.getByLabelText("Workspace dock");
    const rail = screen.getByRole("navigation", { name: "Workspace sections" });

    for (const page of [
      { name: "tasks list" },
      { name: "task workspace", taskId: "task-1" },
      { name: "history" },
    ] as const) {
      goTo(store, page);
      view.rerender(
        <WorkspaceShell>
          <Page store={store} />
        </WorkspaceShell>,
      );
      expect(screen.getByLabelText("Workspace dock")).toBe(dock);
      expect(
        screen.getByRole("navigation", { name: "Workspace sections" }),
      ).toBe(rail);
    }
    expect(store.getState().workspace.dock?.section).toBe("agents");
  });

  it("registers dock shortcuts once", () => {
    const store = createStore();
    store.dispatch(setDockOpen(false));
    renderShell(store);

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "b", ctrlKey: true }),
      );
    });

    expect(store.getState().workspace.dock?.open).toBe(true);
  });

  it("keeps the rail for IDE hosts without mounting hidden panels", () => {
    const store = createStore();
    store.dispatch(updateConfig({ host: "vscode" }));
    goTo(store, { name: "task workspace", taskId: "task-1" });
    renderShell(store);

    expect(
      screen.getByRole("navigation", { name: "Workspace sections" }),
    ).toBeVisible();
    expect(screen.queryByLabelText("Workspace dock")).not.toBeInTheDocument();
  });

  it("scopes dock context to the active task chat on task pages", () => {
    const store = createStore();
    goTo(store, { name: "task workspace", taskId: "task-1" });

    expect(selectFocusedWorkspaceChatId(store.getState())).toBe("planner-1");
    expect(selectWorkspaceFocusedChatId(store.getState())).toBeNull();
    expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(
      "task:task-1",
    );

    store.dispatch(
      setTaskActiveChat({
        taskId: "task-1",
        activeChat: { type: "agent", cardId: "card-1", chatId: "agent-1" },
      }),
    );
    expect(selectFocusedWorkspaceChatId(store.getState())).toBe("agent-1");

    goTo(store, { name: "tasks list" });
    expect(selectFocusedWorkspaceChatId(store.getState())).toBe("agent-1");
    store.dispatch(setActiveTab(chat("chat-a")));
    expect(selectFocusedWorkspaceChatId(store.getState())).toBe("chat-a");
  });

  it("stays on the task page when clicking rail controls", async () => {
    const store = createStore();
    goTo(store, { name: "task workspace", taskId: "task-1" });
    const view = renderShell(store);

    for (const label of ["Agents", "Active chats", "Git", "Files"]) {
      await view.user.click(screen.getByRole("button", { name: label }));
    }

    const pages = store.getState().pages;
    expect(pages[pages.length - 1]).toEqual({
      name: "task workspace",
      taskId: "task-1",
    });
  });

  it("opens task-scoped files in the chat workspace and keeps them through task chat switches", () => {
    const store = createStore();
    goTo(store, { name: "task workspace", taskId: "task-1" });
    renderShell(store);
    const fileKey = makeSurfaceKey("file", "/repo/src/a.ts");

    act(() => {
      store.dispatch(openFileInFilesPanel({ path: "/repo/src/a.ts" }));
    });

    const state = store.getState();
    expect(state.pages[state.pages.length - 1]).toEqual({ name: "chat" });
    expect(selectFocusedWorkspaceSurfaceKey(state)).toBe(fileKey);
    expect(selectSurfaceChatId(state, fileKey)).toBe("planner-1");
    expect(state.workspace.tabs).not.toContain(chat("planner-1"));
    expect(state.chat.open_thread_ids).not.toContain("planner-1");

    act(() => {
      store.dispatch(
        setTaskActiveChat({
          taskId: "task-1",
          activeChat: { type: "agent", cardId: "card-1", chatId: "agent-1" },
        }),
      );
    });
    expect(selectVisibleSurfaceKeys(store.getState())).toContain(fileKey);
    expect(selectSurfaceChatId(store.getState(), fileKey)).toBe("planner-1");
  });

  it("keeps agent-owned file surfaces when the task switches back to its planner", () => {
    const store = createStore();
    store.dispatch(
      setTaskActiveChat({
        taskId: "task-1",
        activeChat: { type: "agent", cardId: "card-1", chatId: "agent-1" },
      }),
    );
    goTo(store, { name: "task workspace", taskId: "task-1" });
    renderShell(store);
    const fileKey = makeSurfaceKey("file", "/repo/src/b.ts");

    act(() => {
      store.dispatch(openFileInFilesPanel({ path: "/repo/src/b.ts" }));
      store.dispatch(
        setTaskActiveChat({
          taskId: "task-1",
          activeChat: { type: "planner", chatId: "planner-1" },
        }),
      );
    });

    expect(selectVisibleSurfaceKeys(store.getState())).toContain(fileKey);
    expect(selectSurfaceChatId(store.getState(), fileKey)).toBe("agent-1");
  });

  it("opens the Git log in the chat workspace from the task board", () => {
    const store = createStore();
    goTo(store, { name: "task workspace", taskId: "task-1" });
    renderShell(store);

    act(() => {
      void store.dispatch(openGitLog());
    });

    const state = store.getState();
    expect(state.pages[state.pages.length - 1]).toEqual({ name: "chat" });
    expect(selectFocusedWorkspaceSurfaceKey(state)).toBe("git:main");
    expect(selectSurfaceChatId(state, "git:main")).toBe("planner-1");
  });

  it("does not navigate when chat-page surfaces open", () => {
    const store = createStore();
    goTo(store, { name: "chat" });
    store.dispatch(setActiveTab(chat("chat-a")));
    renderShell(store);
    const pagesBefore = store.getState().pages;

    act(() => {
      store.dispatch(openFileInFilesPanel({ path: "/repo/src/c.ts" }));
    });

    expect(store.getState().pages).toEqual(pagesBefore);
    expect(
      selectSurfaceChatId(
        store.getState(),
        makeSurfaceKey("file", "/repo/src/c.ts"),
      ),
    ).toBe("chat-a");
  });
});
