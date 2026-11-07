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
  it("carries operation evidence and native actions when the tool is available", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      sync,
      toolAvailable: true,
      files: [],
    });

    expect(prompt).toContain("operation_id=op-7");
    expect(prompt).toContain("expected_revision=3");
    expect(prompt).toContain("worktree_sync");
    expect(prompt).toContain("- src/App.tsx");
    expect(prompt).not.toContain("merge it yourself");
  });

  it("points to the panel instead of an unavailable tool", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      sync,
      toolAvailable: false,
      files: [],
    });

    expect(prompt).toContain("operation_id=op-7");
    expect(prompt).toContain("tool is not available");
    expect(prompt).toContain("panel controls");
  });

  it("never asks for a manual merge without an active operation", () => {
    const prompt = buildWorktreeConflictPrompt({
      worktree,
      files: ["src/App.tsx"],
    });

    expect(prompt).toContain("Do not merge manually");
    expect(prompt).not.toContain("operation_id=");
  });
});
