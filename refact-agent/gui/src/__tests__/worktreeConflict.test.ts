import { describe, expect, it } from "vitest";

import { buildWorktreeConflictPrompt } from "../features/Worktrees/worktreeConflict";
import type { WorktreeSyncResponse } from "../services/refact";

const worktree = {
  id: "wt_1",
  kind: "chat",
  root: "/tmp/worktree",
  source_workspace_root: "/tmp/repo",
  repo_root: "/tmp/repo",
  branch: "refact/chat/source",
  base_branch: "dev",
  enforce: true,
};

const sync: WorktreeSyncResponse = {
  id: "wt_1",
  status: "conflicts",
  source_head: "src1",
  target_branch: "dev",
  target_head: "tgt1",
  operation: {
    operation_id: "op-7",
    revision: 3,
    source_head: "src1",
    target_branch: "dev",
    target_head: "tgt1",
    phase: "integrating",
  },
  conflicts: [{ path: "src/App.tsx", kind: "unmerged" }],
  warnings: [],
  instructions: "Resolve listed conflicts.",
};

describe("buildWorktreeConflictPrompt", () => {
  it("uses selected landing target and files instead of a stale native receipt", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      sync,
      response: { target_branch: "release" },
      files: ["selected.ts"],
    });
    expect(prompt).toContain("Target: release");
    expect(prompt).toContain("- selected.ts");
    expect(prompt).not.toContain("- src/App.tsx");
    expect(prompt).toContain("Pinned source src1 against dev");
  });

  it("carries operation evidence and shell Git guidance", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      sync,
      files: [],
    });

    expect(prompt).toContain("operation_id=op-7");
    expect(prompt).toContain("expected_revision=3");
    expect(prompt).not.toContain("worktree_sync");
    expect(prompt).toContain("shell Git");
    expect(prompt).toContain("- src/App.tsx");
    expect(prompt).not.toContain("merge it yourself");
  });

  it("keeps the native panel optional", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      sync,
      files: [],
    });

    expect(prompt).toContain("operation_id=op-7");
    expect(prompt).toContain("git add only");
    expect(prompt).toContain("optional synchronization panel");
  });

  it("recommends ordinary Git without an active native operation", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      files: ["src/App.tsx"],
    });

    expect(prompt).toContain("git commit");
    expect(prompt).not.toContain("Do not merge manually");
    expect(prompt).not.toContain("operation_id=");
  });
});
