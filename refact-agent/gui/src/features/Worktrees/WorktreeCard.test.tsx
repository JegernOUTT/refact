import { describe, expect, test, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import type {
  WorktreeMeta,
  WorktreeInventory,
  WorktreeRecordView,
} from "../../services/refact/worktrees";
import { WorktreeCard } from "./WorktreeCard";
import {
  sameWorktree,
  selectedFirst,
  worktreeInspection,
  worktreeSize,
} from "./WorktreeCard.helpers";
const meta: WorktreeMeta = {
  id: "selected",
  kind: "chat",
  root: "/cache/selected",
  source_workspace_root: "/repo",
  repo_root: "/repo",
  branch: "feature/selected",
  enforce: true,
};
describe("worktree cards", () => {
  test("distinguishes unknown sizes and zero", () => {
    expect(worktreeSize(null)).toBe("Size unknown");
    expect(worktreeSize(undefined)).toBe("Size unknown");
    expect(worktreeSize(0)).toBe("0 B");
    expect(worktreeSize(1024)).toBe("1.0 KiB");
  });
  test("orders selected first stably without mutating inventory and matches source roots", () => {
    const records = [
      { meta: { ...meta, id: "a" } },
      { meta },
      { meta: { ...meta, id: "b" } },
    ] as WorktreeRecordView[];
    expect(
      selectedFirst(records, meta).map((record) => record.meta.id),
    ).toEqual(["selected", "a", "b"]);
    expect(records[0].meta.id).toBe("a");
    expect(
      sameWorktree(meta, { ...meta, source_workspace_root: "/other" }),
    ).toBe(false);
    const inventory = {
      source_workspace_root: "/other",
      worktrees: [{ id: meta.id, disk_usage_bytes: 0 }],
    } as WorktreeInventory;
    expect(worktreeInspection(inventory, meta)).toBeUndefined();
  });
  test("renders truthful metadata and icon actions without selecting", async () => {
    const onSelect = vi.fn();
    const onDelete = vi.fn();
    const { user } = render(
      <WorktreeCard
        worktree={meta}
        selected
        onSelect={onSelect}
        onDiff={vi.fn()}
        onMerge={vi.fn()}
        onDelete={onDelete}
        onCopy={vi.fn()}
        onOpen={vi.fn()}
      />,
    );
    expect(screen.getByText("Selected")).toBeInTheDocument();
    expect(screen.getByText("Size unknown")).toBeInTheDocument();
    expect(screen.getByText("Merge status unknown")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Delete or discard worktree" }),
    );
    expect(onDelete).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();
    expect(
      screen.getByRole("group", { name: "Manage feature/selected" }),
    ).toBeInTheDocument();
  });
  test("task-managed cards preserve inspect actions but block destructive actions", () => {
    render(
      <WorktreeCard
        worktree={{ ...meta, kind: "task_agent" }}
        selected={false}
        onSelect={vi.fn()}
        onDiff={vi.fn()}
        onMerge={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Merge worktree feature/selected" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", {
        name: "Delete or discard worktree feature/selected",
      }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", {
        name: "View worktree diff feature/selected",
      }),
    ).toBeEnabled();
  });
});
