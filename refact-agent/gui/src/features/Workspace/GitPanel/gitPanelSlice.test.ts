import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { createChatWithId } from "../../Chat/Thread";
import { openTab, setActiveTab } from "../workspaceSlice";
import {
  openGitFile,
  selectActiveGitRoot,
  selectSelectedGitFile,
  setActiveGitRoot,
} from "./gitPanelSlice";

describe("gitPanelSlice", () => {
  it("opens and focuses one main Git surface for selected files", () => {
    const store = setUpStore();

    store.dispatch(
      openGitFile({ root: "/repo", path: "src/app.ts", staged: false }),
    );
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/lib.ts", staged: true }),
    );

    expect(store.getState().workspace.tabs).toEqual(["git:main"]);
    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(selectSelectedGitFile(store.getState(), null)).toEqual({
      root: "/repo",
      path: "src/lib.ts",
      staged: true,
    });
  });

  it("clears a selected file when the active root changes", () => {
    const store = setUpStore();
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/app.ts", staged: false }),
    );

    store.dispatch(setActiveGitRoot({ chatId: null, root: "/other" }));

    expect(selectActiveGitRoot(store.getState(), null)).toBe("/other");
    expect(selectSelectedGitFile(store.getState(), null)).toBeNull();
  });

  it("restores Git selection independently for chats sharing one repository root", () => {
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "chat-a" }));
    store.dispatch(createChatWithId({ id: "chat-b" }));
    store.dispatch(openTab("chat:chat-a"));
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/a.ts", staged: false }),
    );

    store.dispatch(openTab("chat:chat-b"));
    store.dispatch(setActiveTab("chat:chat-b"));
    expect(selectActiveGitRoot(store.getState(), "chat-b")).toBe("");
    expect(selectSelectedGitFile(store.getState(), "chat-b")).toBeNull();
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/b.ts", staged: true }),
    );

    store.dispatch(setActiveTab("chat:chat-a"));
    expect(selectActiveGitRoot(store.getState(), "chat-a")).toBe("/repo");
    expect(selectSelectedGitFile(store.getState(), "chat-a")).toEqual({
      root: "/repo",
      path: "src/a.ts",
      staged: false,
    });

    store.dispatch(setActiveTab("chat:chat-b"));
    expect(selectActiveGitRoot(store.getState(), "chat-b")).toBe("/repo");
    expect(selectSelectedGitFile(store.getState(), "chat-b")).toEqual({
      root: "/repo",
      path: "src/b.ts",
      staged: true,
    });
  });
});
