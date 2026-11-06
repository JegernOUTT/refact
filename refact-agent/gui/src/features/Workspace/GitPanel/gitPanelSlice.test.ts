import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { createChatWithId } from "../../Chat/Thread";
import { gitDiffDescriptorFromSurfaceKey } from "../surfaceKey";
import {
  openTab,
  selectActiveGroup,
  selectFocusedWorkspaceSurfaceKey,
  setActiveTab,
} from "../workspaceSlice";
import { collectTabIds } from "../../ChatPanes/panesTree";
import {
  DEFAULT_GIT_ROOT_VIEW,
  openGitCommitDiff,
  openGitDiff,
  selectActiveGitRoot,
  selectGitDiffTargetForChat,
  selectGitDiffViewerPrefs,
  selectGitRootView,
  setActiveGitRoot,
  setGitDiffViewerPrefs,
  updateGitRootView,
  type GitDiffTarget,
} from "./gitPanelSlice";
import type { GitDiffSide } from "./diff/gitDiffDescriptor";

const target = (
  path: string,
  extra: Partial<GitDiffTarget> = {},
): GitDiffTarget => ({
  root: "/repo",
  path,
  old_path: null,
  kind: "change",
  status: "MODIFIED",
  ...extra,
});

function activeDescriptor(store: ReturnType<typeof setUpStore>) {
  const key = selectFocusedWorkspaceSurfaceKey(store.getState()) ?? "";
  expect(key.startsWith("gitdiff:")).toBe(true);
  return gitDiffDescriptorFromSurfaceKey(key);
}

const side = (rev: string, path: string): GitDiffSide => ({ rev, path });

describe("gitPanelSlice", () => {
  it("opens one gitdiff tab per file and focuses an existing tab again", () => {
    const store = setUpStore();

    const appKey = store.dispatch(openGitDiff(target("src/app.ts")));
    const libKey = store.dispatch(openGitDiff(target("src/lib.ts")));
    expect(store.getState().workspace.tabs).toEqual([appKey]);
    const group = selectActiveGroup(store.getState());
    if (!group) throw new Error("Expected an active workspace group");
    expect(collectTabIds(group.root)).toEqual([appKey, libKey]);
    expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(libKey);

    expect(store.dispatch(openGitDiff(target("src/app.ts")))).toBe(appKey);
    expect(store.getState().workspace.tabs).toEqual([appKey]);
    const refocusedGroup = selectActiveGroup(store.getState());
    if (!refocusedGroup)
      throw new Error("Expected the refocused workspace group");
    expect(collectTabIds(refocusedGroup.root)).toEqual([appKey, libKey]);
    expect(selectFocusedWorkspaceSurfaceKey(store.getState())).toBe(appKey);
    expect(selectGitDiffTargetForChat(store.getState(), null)).toEqual(
      target("src/app.ts"),
    );
  });

  it.each<[string, GitDiffTarget, GitDiffSide | null, GitDiffSide | null]>([
    [
      "modified",
      target("src/app.ts"),
      side("HEAD", "src/app.ts"),
      side("WORKTREE", "src/app.ts"),
    ],
    [
      "added",
      target("src/new.ts", { status: "ADDED" }),
      null,
      side("WORKTREE", "src/new.ts"),
    ],
    [
      "deleted",
      target("src/old.ts", { status: "DELETED" }),
      side("HEAD", "src/old.ts"),
      null,
    ],
    [
      "renamed",
      target("src/next.ts", { status: "RENAMED", old_path: "src/prev.ts" }),
      side("HEAD", "src/prev.ts"),
      side("WORKTREE", "src/next.ts"),
    ],
    [
      "untracked",
      target("notes.txt", { kind: "untracked", status: null }),
      null,
      side("WORKTREE", "notes.txt"),
    ],
    [
      "conflicted",
      target("lib/conflict.ts", { kind: "conflict", status: null }),
      side("HEAD", "lib/conflict.ts"),
      side("WORKTREE", "lib/conflict.ts"),
    ],
  ])(
    "opens %s working changes with the right revisions",
    (_, diffTarget, left, right) => {
      const store = setUpStore();
      store.dispatch(openGitDiff(diffTarget));
      const descriptor = activeDescriptor(store);
      expect(descriptor?.left).toEqual(left);
      expect(descriptor?.right).toEqual(right);
      expect(descriptor?.kind).toBe("working");
      expect(descriptor?.old_path).toBe(diffTarget.old_path);
    },
  );

  it("opens commit file diffs against the first parent", () => {
    const store = setUpStore();
    const commit = { oid: "c0ffee1234567890", short_oid: "c0ffee1" };
    store.dispatch(
      openGitCommitDiff({
        root: "/repo",
        path: "src/next.ts",
        old_path: "src/prev.ts",
        status: "RENAMED",
        commit,
        parent_oid: "abc1234",
      }),
    );
    expect(activeDescriptor(store)).toEqual({
      root: "/repo",
      path: "src/next.ts",
      old_path: "src/prev.ts",
      left: side("abc1234", "src/prev.ts"),
      right: side(commit.oid, "src/next.ts"),
      kind: "commit",
      commit,
    });

    store.dispatch(
      openGitCommitDiff({
        root: "/repo",
        path: "README.md",
        old_path: null,
        commit,
        parent_oid: null,
      }),
    );
    expect(activeDescriptor(store)?.left).toBeNull();
  });

  it("binds diff tabs to the focused chat", () => {
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "chat-a" }));
    store.dispatch(openTab("chat:chat-a"));
    const key = store.dispatch(openGitDiff(target("src/a.ts")));
    expect(store.getState().workspace.contextChatByTab?.[key]).toBe("chat-a");
  });

  it("remembers the diff viewer mode and collapse preference", () => {
    const store = setUpStore();
    expect(selectGitDiffViewerPrefs(store.getState())).toEqual({
      mode: "side-by-side",
      collapseUnchanged: true,
    });
    store.dispatch(setGitDiffViewerPrefs({ mode: "unified" }));
    store.dispatch(setGitDiffViewerPrefs({ collapseUnchanged: false }));
    expect(selectGitDiffViewerPrefs(store.getState())).toEqual({
      mode: "unified",
      collapseUnchanged: false,
    });
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
