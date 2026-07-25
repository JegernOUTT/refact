import { describe, expect, test } from "vitest";

import { workspaceRootForGitRoot } from "./gitRoots";

describe("workspaceRootForGitRoot", () => {
  test.each([
    [["/repo", "/repo/packages/app"], "/repo/packages/app"],
    [["/repo/packages/app", "/repo"], "/repo/packages/app"],
  ])(
    "prefers the deepest enclosing configured root independent of order",
    (configuredRoots, expected) => {
      expect(
        workspaceRootForGitRoot(configuredRoots, "/repo/packages/app/src"),
      ).toBe(expected);
    },
  );

  test("prefers an exact configured root over enclosing roots", () => {
    expect(
      workspaceRootForGitRoot(
        ["/repo/packages", "/repo", "/repo/packages/app"],
        "/repo/packages",
      ),
    ).toBe("/repo/packages");
  });

  test("prefers the explicit focused-chat worktree source root", () => {
    expect(
      workspaceRootForGitRoot(
        ["/repo", "/repo/packages/app"],
        "/worktrees/chat-a",
        "/repo/packages/app",
      ),
    ).toBe("/repo/packages/app");
  });

  test("does not route an ambiguous repository root to an array-order sibling", () => {
    expect(
      workspaceRootForGitRoot(
        ["/repo/packages/a", "/repo/packages/b"],
        "/repo",
      ),
    ).toBe("/repo");
  });
});
