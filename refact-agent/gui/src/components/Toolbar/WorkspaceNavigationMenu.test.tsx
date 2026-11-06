import { describe, expect, it } from "vitest";

import { setUpStore } from "../../app/store";
import { createChatWithId } from "../../features/Chat/Thread";
import { push } from "../../features/Pages/pagesSlice";
import { openTask } from "../../features/Tasks/tasksSlice";
import {
  makeGitDiffSurfaceKey,
  makeSurfaceKey,
} from "../../features/Workspace/surfaceKey";
import {
  openTab,
  setPanelsForced,
  selectFocusedWorkspaceSurfaceKey,
} from "../../features/Workspace/workspaceSlice";
import { render, screen } from "../../utils/test-utils";
import { WorkspaceNavigationMenu } from "./WorkspaceNavigationMenu";

function createStore() {
  const store = setUpStore({ tasksUI: { openTasks: [] } });
  store.dispatch(setPanelsForced(true));
  store.dispatch(
    createChatWithId({
      id: "editor-chat",
      title: "Editor chat",
      mode: "agent",
    }),
  );
  store.dispatch(openTab(makeSurfaceKey("chat", "editor-chat")));
  return store;
}

const diffKey = makeGitDiffSurfaceKey({
  root: "/workspace",
  path: "src/example.ts",
  old_path: null,
  left: { rev: "HEAD", path: "src/example.ts" },
  right: { rev: "WORKTREE", path: "src/example.ts" },
  kind: "working",
});

describe("WorkspaceNavigationMenu", () => {
  it("returns to Editor without reopening its anchor or changing local focus", async () => {
    const store = createStore();
    store.dispatch(openTab(makeSurfaceKey("file", "/workspace/current.ts")));
    const editorKey = store.getState().workspace.tabs[0];
    store.dispatch(openTab("git:main"));
    store.dispatch(push({ name: "history" }));
    const groups = store.getState().workspace.groups;
    const { user } = render(<WorkspaceNavigationMenu />, { store });

    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open views" }));
    await user.click(screen.getByRole("menuitem", { name: "Editor" }));

    expect(store.getState().workspace.activeTabId).toBe(editorKey);
    expect(store.getState().workspace.groups).toEqual(groups);
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
  });

  it.each([
    ["git:main", "Git"],
    ["design:main", "Design"],
    ["design:/workspace/mockups/settings.html", "Design · settings.html"],
    [diffKey, "example.ts"],
  ])(
    "keeps the open %s surface reachable with a readable title",
    async (key, title) => {
      const store = setUpStore({ tasksUI: { openTasks: [] } });
      store.dispatch(setPanelsForced(true));
      store.dispatch(openTab(key));
      const ownerKey = store.getState().workspace.activeTabId;
      store.dispatch(push({ name: "buddy" }));
      const { user } = render(<WorkspaceNavigationMenu />, { store });

      await user.click(screen.getByRole("button", { name: "Open views" }));
      const item = screen.getByRole("menuitem", { name: title });
      if (key === diffKey) {
        expect(item).toHaveAttribute(
          "title",
          "src/example.ts (HEAD ↔ Working tree)",
        );
      }
      await user.click(item);

      expect(store.getState().workspace.activeTabId).toBe(ownerKey);
      expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(key);
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    },
  );

  it("opens named tasks and Buddy without replacing the Editor workspace", async () => {
    const store = createStore();
    store.dispatch(openTask({ id: "task-id", name: "Ship navigation" }));
    const workspace = store.getState().workspace;
    const { user } = render(<WorkspaceNavigationMenu />, { store });

    await user.click(screen.getByRole("button", { name: "Open views" }));
    await user.click(screen.getByRole("menuitem", { name: "Ship navigation" }));
    expect(store.getState().pages.at(-1)).toEqual({
      name: "task workspace",
      taskId: "task-id",
    });

    await user.click(screen.getByRole("button", { name: "Open views" }));
    await user.click(screen.getByRole("menuitem", { name: "Buddy" }));
    expect(store.getState().pages.at(-1)).toEqual({ name: "buddy" });
    expect(store.getState().workspace).toEqual(workspace);
  });

  it("supports keyboard opening and selection", async () => {
    const store = createStore();
    store.dispatch(push({ name: "history" }));
    const { user } = render(<WorkspaceNavigationMenu />, { store });
    screen.getByRole("button", { name: "Open views" }).focus();

    await user.keyboard("{ArrowDown}");
    expect(
      await screen.findByRole("menuitem", { name: "Editor" }),
    ).toHaveFocus();
    await user.keyboard("{Enter}");

    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });
});
