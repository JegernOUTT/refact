import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { createChatWithId } from "../../Chat/Thread";
import { openTab, setActiveTab } from "../workspaceSlice";
import {
  DEFAULT_GIT_ROOT_VIEW,
  openGitDiff,
  selectActiveGitRoot,
  selectGitDiffTargetForChat,
  selectGitRootView,
  setActiveGitRoot,
  updateGitRootView,
  type GitDiffTarget,
} from "./gitPanelSlice";

const target = (path: string): GitDiffTarget => ({
  root: "/repo",
  path,
  old_path: null,
  kind: "change",
});

describe("gitPanelSlice", () => {
  it("opens and focuses one main Git surface for diff targets", () => {
    const store = setUpStore();

    store.dispatch(openGitDiff(target("src/app.ts")));
    store.dispatch(openGitDiff(target("src/lib.ts")));

    expect(store.getState().workspace.tabs).toEqual(["git:main"]);
    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(selectGitDiffTargetForChat(store.getState(), null)).toEqual(
      target("src/lib.ts"),
    );
  });

  it("clears the diff target but keeps per-root view state when the root changes", () => {
    const store = setUpStore();
    store.dispatch(openGitDiff(target("src/app.ts")));
    store.dispatch(
      updateGitRootView({
        chatId: null,
        root: "/repo",
        patch: { excludedChanges: ["src/app.ts"] },
      }),
    );

    store.dispatch(setActiveGitRoot({ chatId: null, root: "/other" }));

    expect(selectActiveGitRoot(store.getState(), null)).toBe("/other");
    expect(selectGitDiffTargetForChat(store.getState(), null)).toBeNull();
    expect(selectGitRootView(store.getState(), null, "/repo")).toEqual({
      ...DEFAULT_GIT_ROOT_VIEW,
      excludedChanges: ["src/app.ts"],
    });
    expect(selectGitRootView(store.getState(), null, "/other")).toBe(
      DEFAULT_GIT_ROOT_VIEW,
    );
  });

  it("keeps diff targets independent per chat sharing one repository root", () => {
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "chat-a" }));
    store.dispatch(createChatWithId({ id: "chat-b" }));
    store.dispatch(openTab("chat:chat-a"));
    store.dispatch(openGitDiff(target("src/a.ts")));

    store.dispatch(openTab("chat:chat-b"));
    store.dispatch(setActiveTab("chat:chat-b"));
    expect(selectActiveGitRoot(store.getState(), "chat-b")).toBe("");
    expect(selectGitDiffTargetForChat(store.getState(), "chat-b")).toBeNull();
    store.dispatch(openGitDiff(target("src/b.ts")));

    expect(selectGitDiffTargetForChat(store.getState(), "chat-a")).toEqual(
      target("src/a.ts"),
    );
    expect(selectGitDiffTargetForChat(store.getState(), "chat-b")).toEqual(
      target("src/b.ts"),
    );
  });
});
