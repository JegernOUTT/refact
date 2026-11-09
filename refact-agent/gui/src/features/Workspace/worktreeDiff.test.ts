import { beforeEach, describe, expect, it } from "vitest";
import { setUpStore } from "../../app/store";
import { openWorktreeDiff } from "../Worktrees/worktreeDiffSlice";
import { collectTabIds } from "../ChatPanes/panesTree";
import {
  makeWorktreeDiffSurfaceKey,
  isWorkspaceSurfaceEnabled,
  parseSurfaceKey,
  worktreeDiffDescriptorFromSurfaceKey,
} from "./surfaceKey";
import {
  worktreeFileDescriptor,
  worktreeDiffEntryKey,
} from "./worktreeDiffModel";
import {
  loadPersistedWorkspace,
  savePersistedWorkspace,
  setProjectStorageNamespace,
} from "../../utils/chatUiPersistence";
import type { WorktreeDiffResponse } from "../../services/refact/worktrees";
import { reconcileWorkspaceState } from "./workspaceSlice";

const descriptor = {
  worktreeId: "wt:雪",
  sourceWorkspaceRoot: "/repo",
  root: "/repo/wt",
  label: "branch",
};
const response: WorktreeDiffResponse = {
  id: "wt",
  resolved_base_commit: "resolved-base",
  base_commit: "original-ref",
  status: {
    path_exists: true,
    is_git_worktree: true,
    dirty: true,
    staged_count: 1,
    unstaged_count: 1,
    untracked_count: 1,
    head_commit: "head-oid",
  },
  files: [],
  stats: {
    committed_files: 1,
    staged_files: 1,
    unstaged_files: 1,
    untracked_files: 1,
    files_changed: 4,
  },
  patch: "",
  patch_truncated: false,
};

describe("ephemeral worktree review", () => {
  beforeEach(() => {
    localStorage.clear();
    setProjectStorageNamespace("diff-test");
  });
  it("validates and round trips distinct descriptor keys", () => {
    const key = makeWorktreeDiffSurfaceKey(descriptor);
    expect(parseSurfaceKey(key).kind).toBe("worktreediff");
    expect(worktreeDiffDescriptorFromSurfaceKey(key)).toEqual(descriptor);
    expect(() => parseSurfaceKey("worktreediff:%7B%7D")).toThrow();
  });
  it("keeps dedicated reviews available without enabling IDE Git panel chrome", () => {
    const key = makeWorktreeDiffSurfaceKey(descriptor);
    const capabilities = {
      filesPanel: false,
      gitPanel: false,
      terminalPanel: false,
    };
    expect(isWorkspaceSurfaceEnabled(key, capabilities)).toBe(true);
    expect(isWorkspaceSurfaceEnabled("git:main", capabilities)).toBe(false);
    const store = setUpStore();
    store.dispatch(openWorktreeDiff(descriptor));
    const reconciled = reconcileWorkspaceState(
      store.getState().workspace,
      [],
      capabilities,
    );
    expect(
      Object.values(reconciled.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain(key);
  });
  it("deduplicates review tabs and strips them from saved groups and metadata", () => {
    const store = setUpStore();
    store.dispatch(openWorktreeDiff(descriptor));
    store.dispatch(
      openWorktreeDiff({ ...descriptor, label: "different label" }),
    );
    const workspace = store.getState().workspace;
    const surfaces = Object.values(workspace.groups).flatMap((group) =>
      group ? collectTabIds(group.root) : [],
    );
    expect(
      surfaces.filter((key) => key.startsWith("worktreediff:")),
    ).toHaveLength(1);
    savePersistedWorkspace(workspace);
    expect(JSON.stringify(loadPersistedWorkspace())).not.toContain(
      "worktreediff:",
    );
    expect(
      Array.from({ length: localStorage.length }, (_, index) =>
        localStorage.getItem(localStorage.key(index) ?? ""),
      ).join(""),
    ).not.toContain("worktreediff:");
  });
  it("preserves surviving tabs when an ephemeral group owner and split leaf disappear", () => {
    const key = makeWorktreeDiffSurfaceKey(descriptor);
    savePersistedWorkspace({
      tabs: [key],
      activeTabId: key,
      groups: {
        [key]: {
          focusedLeafId: "review",
          root: {
            kind: "split",
            id: "split",
            dir: "row",
            sizes: [0.5, 0.5],
            children: [
              { kind: "leaf", id: "review", tabIds: [key], activeTabId: key },
              {
                kind: "leaf",
                id: "file",
                tabIds: ["file:/repo/a.ts"],
                activeTabId: "file:/repo/a.ts",
              },
            ],
          },
        },
      },
    });
    const restored = loadPersistedWorkspace();
    expect(restored.tabs).toEqual(["file:/repo/a.ts"]);
    expect(restored.groups["file:/repo/a.ts"]?.root).toEqual({
      kind: "leaf",
      id: "file",
      tabIds: ["file:/repo/a.ts"],
      activeTabId: "file:/repo/a.ts",
    });
  });
  it.each([
    ["committed", "resolved-base", "head-oid"],
    ["staged", "head-oid", "INDEX"],
    ["unstaged", "INDEX", "WORKTREE"],
    ["untracked", null, "WORKTREE"],
  ])(
    "maps %s to the exact layer with independent source/path identity",
    (source, left, right) => {
      const file = {
        source,
        path: "new.ts",
        old_path: "old.ts",
        status: "R",
      };
      const diff = worktreeFileDescriptor("/wt", file, response);
      expect(diff?.left).toEqual(left ? { rev: left, path: "old.ts" } : null);
      expect(diff?.right).toEqual({ rev: right, path: "new.ts" });
      expect(worktreeDiffEntryKey(file)).not.toBe(
        worktreeDiffEntryKey({ ...file, source: "other" }),
      );
    },
  );
  it("handles added/deleted sides and refuses unknown bases or legacy rename paths", () => {
    expect(
      worktreeFileDescriptor(
        "/wt",
        { source: "staged", path: "a", status: "A" },
        response,
      )?.left,
    ).toBeNull();
    expect(
      worktreeFileDescriptor(
        "/wt",
        { source: "unstaged", path: "a", status: "D" },
        response,
      )?.right,
    ).toBeNull();
    expect(
      worktreeFileDescriptor(
        "/wt",
        { source: "committed", path: "a", status: "M" },
        { ...response, base_commit: null, resolved_base_commit: null },
      ),
    ).toBeNull();
    expect(
      worktreeFileDescriptor(
        "/wt",
        { source: "staged", path: "new", status: "R" },
        response,
      ),
    ).toBeNull();
  });
});
