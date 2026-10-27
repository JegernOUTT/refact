import { describe, expect, it } from "vitest";

import {
  normalizeGitStatus,
  type GitChange,
  type GitStatusRoot,
} from "../../../services/refact/gitRead";
import {
  applyInclusion,
  buildChangesTree,
  collectEntries,
  commitPaths,
  containerIds,
  flattenChangesTree,
  includedEntries,
  nodeCheckState,
  pruneInclusion,
  type ChangesTreeNode,
  type InclusionState,
} from "./changesTreeModel";

function change(
  path: string,
  status: GitChange["status"] = "MODIFIED",
  oldPath: string | null = null,
): GitChange {
  return {
    relative_path: path,
    absolute_path: `/repo/${path}`,
    old_path: oldPath,
    status,
    staged: false,
    unstaged: true,
  };
}

function status(overrides: Partial<GitStatusRoot> = {}) {
  return normalizeGitStatus({
    root: "/repo",
    branch: "main",
    head_detached: false,
    ahead: 0,
    behind: 0,
    staged: [],
    unstaged: [],
    untracked_included: true,
    changes: [],
    untracked: [],
    conflicted: [],
    ...overrides,
  });
}

const NO_INCLUSION: InclusionState = {
  excludedChanges: new Set(),
  includedUnversioned: new Set(),
};

function labels(nodes: ChangesTreeNode[]): string[] {
  return nodes.map((node) =>
    node.type === "file" ? node.entry.name : node.label,
  );
}

describe("changesTreeModel", () => {
  it("builds only non-empty optional groups after Changes", () => {
    expect(labels(buildChangesTree(status(), true))).toEqual(["Changes"]);
    const groups = buildChangesTree(
      status({
        changes: [change("a.ts")],
        untracked: [{ relative_path: "new.ts", absolute_path: "/repo/new.ts" }],
        conflicted: [
          {
            relative_path: "c.ts",
            absolute_path: "/repo/c.ts",
            ours: null,
            theirs: null,
            ancestor: null,
          },
        ],
      }),
      true,
    );
    expect(labels(groups)).toEqual([
      "Changes",
      "Merge Conflicts",
      "Unversioned Files",
    ]);
    expect(groups.map((group) => group.fileCount)).toEqual([1, 1, 1]);
  });

  it("compacts single-child directory chains and sorts dirs before files", () => {
    const [changes] = buildChangesTree(
      status({
        changes: [
          change("src/app/deep/z.ts"),
          change("src/app/deep/a.ts"),
          change("README.md"),
          change("src/main.ts"),
          change("lib/only/one/file.ts"),
        ],
      }),
      true,
    );
    expect(labels(changes.children)).toEqual([
      "lib/only/one",
      "src",
      "README.md",
    ]);
    const src = changes.children[1];
    expect(src.type === "dir" && labels(src.children)).toEqual([
      "app/deep",
      "main.ts",
    ]);
    expect(src.type === "dir" && src.fileCount).toBe(3);
    const deep = src.type === "dir" ? src.children[0] : null;
    expect(deep?.type === "dir" && labels(deep.children)).toEqual([
      "a.ts",
      "z.ts",
    ]);
  });

  it("flattens without directories and keeps the parent path for display", () => {
    const [changes] = buildChangesTree(
      status({
        changes: [change("src/b.ts"), change("a/b.ts"), change("c.ts")],
      }),
      false,
    );
    expect(
      changes.children.map((node) =>
        node.type === "file" ? node.entry.path : "",
      ),
    ).toEqual(["a/b.ts", "src/b.ts", "c.ts"]);
  });

  it("respects collapsed containers when flattening", () => {
    const groups = buildChangesTree(
      status({ changes: [change("src/a.ts"), change("src/b.ts")] }),
      true,
    );
    expect(flattenChangesTree(groups, new Set())).toHaveLength(4);
    const rows = flattenChangesTree(groups, new Set(["dir:changes:src"]));
    expect(rows.map((row) => [row.node.id, row.depth])).toEqual([
      ["group:changes", 0],
      ["dir:changes:src", 1],
    ]);
    expect(containerIds(groups)).toEqual(["group:changes", "dir:changes:src"]);
  });

  it("includes changes and excludes unversioned files by default", () => {
    const current = status({
      changes: [change("a.ts")],
      untracked: [{ relative_path: "new.ts", absolute_path: "/repo/new.ts" }],
    });
    const groups = buildChangesTree(current, true);
    expect(nodeCheckState(groups[0], NO_INCLUSION)).toBe(true);
    expect(nodeCheckState(groups[1], NO_INCLUSION)).toBe(false);
    expect(includedEntries(current, NO_INCLUSION).map((e) => e.path)).toEqual([
      "a.ts",
    ]);
  });

  it("computes tri-state and toggles whole subtrees", () => {
    const current = status({
      changes: [change("src/a.ts"), change("src/b.ts")],
    });
    const [changes] = buildChangesTree(current, true);
    const half = applyInclusion(
      NO_INCLUSION,
      [collectEntries(changes)[0]],
      false,
    );
    const halfState: InclusionState = {
      excludedChanges: new Set(half.excludedChanges),
      includedUnversioned: new Set(half.includedUnversioned),
    };
    expect(nodeCheckState(changes, halfState)).toBe("indeterminate");
    const none = applyInclusion(halfState, collectEntries(changes), false);
    expect(none.excludedChanges).toEqual(["src/a.ts", "src/b.ts"]);
    const all = applyInclusion(
      {
        excludedChanges: new Set(none.excludedChanges),
        includedUnversioned: new Set(),
      },
      collectEntries(changes),
      true,
    );
    expect(all.excludedChanges).toEqual([]);
  });

  it("treats conflicts as non-checkable", () => {
    const [, conflicts] = buildChangesTree(
      status({
        changes: [change("a.ts")],
        conflicted: [
          {
            relative_path: "c.ts",
            absolute_path: "/repo/c.ts",
            ours: null,
            theirs: null,
            ancestor: null,
          },
        ],
      }),
      true,
    );
    expect(nodeCheckState(conflicts, NO_INCLUSION)).toBe(false);
    const next = applyInclusion(NO_INCLUSION, collectEntries(conflicts), true);
    expect(next).toEqual({ excludedChanges: [], includedUnversioned: [] });
  });

  it("prunes vanished paths and includes new changes by default", () => {
    const inclusion: InclusionState = {
      excludedChanges: new Set(["gone.ts", "kept.ts"]),
      includedUnversioned: new Set(["gone-new.ts", "new.ts"]),
    };
    const current = status({
      changes: [change("kept.ts"), change("fresh.ts")],
      untracked: [{ relative_path: "new.ts", absolute_path: "/repo/new.ts" }],
    });
    expect(pruneInclusion(inclusion, current)).toEqual({
      excludedChanges: ["kept.ts"],
      includedUnversioned: ["new.ts"],
    });
    expect(includedEntries(current, inclusion).map((e) => e.path)).toEqual([
      "fresh.ts",
      "new.ts",
    ]);
  });

  it("adds the old path of renames to the commit paths", () => {
    const current = status({
      changes: [change("b.ts", "RENAMED", "a.ts"), change("c.ts")],
    });
    expect(commitPaths(includedEntries(current, NO_INCLUSION))).toEqual([
      "b.ts",
      "a.ts",
      "c.ts",
    ]);
  });

  it("derives changes from legacy staged/unstaged fields", () => {
    const legacy = normalizeGitStatus({
      root: "/repo",
      branch: "main",
      head_detached: false,
      ahead: null,
      behind: null,
      staged: [
        { relative_path: "a.ts", absolute_path: "/repo/a.ts", status: "ADDED" },
      ],
      unstaged: [
        {
          relative_path: "a.ts",
          absolute_path: "/repo/a.ts",
          status: "MODIFIED",
        },
        {
          relative_path: "b.ts",
          absolute_path: "/repo/b.ts",
          status: "DELETED",
        },
      ],
      untracked_included: true,
    });
    expect(legacy.extended).toBe(false);
    expect(legacy.repoState).toBe("clean");
    expect(legacy.changes).toEqual([
      {
        relative_path: "a.ts",
        absolute_path: "/repo/a.ts",
        old_path: null,
        status: "ADDED",
        staged: true,
        unstaged: true,
      },
      {
        relative_path: "b.ts",
        absolute_path: "/repo/b.ts",
        old_path: null,
        status: "DELETED",
        staged: false,
        unstaged: true,
      },
    ]);
  });
});
