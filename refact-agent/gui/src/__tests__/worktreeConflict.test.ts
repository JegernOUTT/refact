import { describe, expect, it } from "vitest";

import { buildWorktreeConflictPrompt } from "../features/Worktrees/worktreeConflict";

describe("buildWorktreeConflictPrompt", () => {
  it("explains that stopped merges do not leave conflict markers", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree: {
        id: "wt_1",
        kind: "chat",
        root: "/tmp/worktree",
        source_workspace_root: "/tmp/repo",
        repo_root: "/tmp/repo",
        branch: "refact/chat/source",
        base_branch: "dev",
        enforce: true,
      },
      response: {
        source_branch: "refact/chat/source",
        target_branch: "dev",
        conflict: {
          files: ["src/App.tsx"],
          aborted: true,
          merge_in_progress: false,
          instructions:
            "Merge conflicts were detected during preflight; resolve the source branch against the target branch and retry.",
        },
      },
      files: ["src/App.tsx"],
    });

    expect(prompt).toContain("The merge was stopped and cleaned up");
    expect(prompt).toContain("conflict markers will not be present");
    expect(prompt).toContain("Please merge it yourself");
    expect(prompt).not.toContain("Please inspect the conflict markers");
  });
});
