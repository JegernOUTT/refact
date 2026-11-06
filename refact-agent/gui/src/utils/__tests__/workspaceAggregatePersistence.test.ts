import { makeGitDiffSurfaceKey } from "../../features/Workspace/surfaceKey";
import { beforeEach, describe, expect, it } from "vitest";
import { collectTabIds } from "../../features/ChatPanes/panesTree";
import {
  loadPersistedWorkspace,
  savePersistedWorkspace,
  savePersistedChatTabs,
  setProjectStorageNamespace,
} from "../chatUiPersistence";
beforeEach(() => {
  localStorage.clear();
  setProjectStorageNamespace("aggregate-regression");
  savePersistedChatTabs({
    openThreadIds: ["a", "b"],
    currentThreadId: "b",
    tabs: [{ id: "a" }, { id: "b" }],
  });
});
function store(count: number) {
  const groups = Object.fromEntries(
    ["a", "b"].map((id, index) => {
      const tabIds = [
        `chat:${id}`,
        ...Array.from(
          { length: count / 2 - 1 },
          (_, n) => `file:/${index}/${n}`,
        ),
      ];
      return [
        `chat:${id}`,
        {
          root: { kind: "leaf", id: "root", tabIds, activeTabId: tabIds[0] },
          focusedLeafId: "root",
        },
      ];
    }),
  );
  localStorage.setItem(
    "refact:project:aggregate-regression:refact:chat-ui:workspace:v1",
    JSON.stringify({
      version: 4,
      tabs: ["chat:a", "chat:b"],
      activeTabId: "chat:a",
      groups,
    }),
  );
}
describe("aggregate persisted workspace bounds", () => {
  it("rejects an oversized aggregate using the explicit chat fallback", () => {
    store(902);
    const state = loadPersistedWorkspace();
    expect(state.tabs).toEqual(["chat:b"]);
    expect(state.groups).toEqual({});
  });
  it("retains every surface in a valid legacy 900-surface layout", () => {
    store(900);
    const state = loadPersistedWorkspace();
    expect(
      Object.values(state.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toHaveLength(900);
  });
});

describe("all-kind durable workspace", () => {
  it("round-trips mixed panes and independent non-chat roots without open thread IDs", () => {
    savePersistedChatTabs({ openThreadIds: [], currentThreadId: "", tabs: [] });
    const diff = makeGitDiffSurfaceKey({
      root: "/repo",
      path: "a",
      old_path: null,
      left: null,
      right: { rev: "WORKTREE", path: "a" },
      kind: "working",
    });
    const keys = [
      "task:a",
      "buddy:home",
      "design:a",
      diff,
      "git:main",
      "file:/a",
    ];
    for (const key of keys) {
      savePersistedWorkspace({
        tabs: [key],
        activeTabId: key,
        groups: {
          [key]: {
            focusedLeafId: "right",
            root: {
              kind: "split",
              id: "split",
              dir: "col",
              sizes: [0.4, 0.6],
              children: [
                { kind: "leaf", id: "left", tabIds: [key], activeTabId: key },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: keys.filter((other) => other !== key),
                  activeTabId: keys.find((other) => other !== key) ?? null,
                },
              ],
            },
          },
        },
      });
      const restored = loadPersistedWorkspace();
      expect(restored.tabs).toEqual([key]);
      expect(restored.groups[key]?.focusedLeafId).toBe("right");
      const group = restored.groups[key];
      if (!group) throw new Error("missing restored group");
      expect(collectTabIds(group.root)).toEqual([
        key,
        ...keys.filter((other) => other !== key),
      ]);
      expect(restored.contextChatByTab).toBeUndefined();
    }
    setProjectStorageNamespace(undefined);
    expect(loadPersistedWorkspace().tabs).toEqual([]);
  });
});
