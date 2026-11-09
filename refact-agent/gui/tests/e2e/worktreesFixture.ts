import type {
  WorktreeDiffResponse,
  WorktreeInventory,
  WorktreeRecordView,
} from "../../src/services/refact/worktrees";

export const worktreeSourceRoot = "/workspace/refact";
const baseCommit = "a".repeat(40);
const headCommit = "b".repeat(40);

function record(id: string, branch: string, dirty = false): WorktreeRecordView {
  return {
    meta: {
      id,
      kind: "chat",
      root: `/workspace/worktrees/${id}`,
      source_workspace_root: worktreeSourceRoot,
      repo_root: worktreeSourceRoot,
      branch,
      base_branch: "main",
      base_commit: baseCommit,
      enforce: true,
    },
    created_at: "2026-11-08T10:00:00Z",
    updated_at: "2026-11-09T12:00:00Z",
    reference_count: id === "selected-checkout" ? 1 : 0,
    references:
      id === "selected-checkout"
        ? [{ kind: "chat", chat_id: "showcase-chat" }]
        : [],
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty,
      staged_count: dirty ? 1 : 0,
      unstaged_count: dirty ? 1 : 0,
      untracked_count: dirty ? 1 : 0,
      branch,
      head_commit: headCommit,
    },
  };
}

export const worktreeRecords = [
  record("merged-checkout", "refact/chat/finished-polish"),
  record("unknown-checkout", "refact/chat/recovered-checkout"),
  record("selected-checkout", "refact/chat/worktree-polish", true),
  {
    ...record("managed-checkout", "refact/task/workspace/cards/T-17"),
    meta: {
      ...record("managed-checkout", "refact/task/workspace/cards/T-17").meta,
      kind: "task_agent",
    },
  },
];
export const selectedWorktree = worktreeRecords[2];

export const worktreeInventory: WorktreeInventory = {
  project_hash: "route-showcase",
  source_workspace_root: worktreeSourceRoot,
  generated_at: "2026-11-09T12:00:00Z",
  summary: {
    total_registered: 4,
    total_discovered: 0,
    total: 4,
    clean: 3,
    dirty: 1,
    unknown: 0,
    stale: 0,
    conflicted: 0,
    shared: 0,
    abandoned_clean: 2,
    changed_files: 4,
    additions: 12,
    deletions: 3,
    missing_registry_paths: 0,
    unregistered_cache_dirs: 0,
    merged_branches: 1,
  },
  worktrees: worktreeRecords.map((item, index) => ({
    id: item.meta.id,
    root: item.meta.root,
    source: "registered",
    branch: item.meta.branch,
    base_branch: "main",
    base_commit: baseCommit,
    status: item.status,
    references: item.references,
    reference_count: item.reference_count,
    shared: false,
    stale: false,
    conflicted: false,
    changed_files: item.status.dirty ? 4 : 0,
    committed_files: item.status.dirty ? 1 : 0,
    staged_files: item.status.staged_count,
    unstaged_files: item.status.unstaged_count,
    untracked_files: item.status.untracked_count,
    additions: item.status.dirty ? 12 : 0,
    deletions: item.status.dirty ? 3 : 0,
    cleanup_candidate: index === 0,
    cleanup_blockers: [],
    disk_usage_bytes: [2097152, null, 318767104, 0][index],
    branch_merged: [true, null, false, false][index],
    registry_missing: false,
    cache_dir_missing_from_registry: false,
    attached_chat_ids: item.references.flatMap((ref) => ref.chat_id ?? []),
    attached_task_ids: [],
  })),
  cleanup_candidates: ["merged-checkout"],
};

export function worktreeFixtureResponse(url: URL): unknown | undefined {
  if (url.pathname === "/v1/worktrees/summary") return worktreeInventory;
  const match = /^\/v1\/worktrees\/([^/]+)(\/diff)?$/.exec(url.pathname);
  if (!match) return undefined;
  const item = worktreeRecords.find((entry) => entry.meta.id === match[1]);
  if (!item) return undefined;
  if (!match[2]) return item;
  const diff: WorktreeDiffResponse & { resolved_base_commit: string } = {
    id: item.meta.id,
    branch: item.meta.branch,
    base_branch: "main",
    base_commit: baseCommit,
    resolved_base_commit: baseCommit,
    ahead: 1,
    behind: 0,
    status: item.status,
    files: [
      {
        path: "src/worktrees.ts",
        status: "M",
        source: "committed",
        additions: 4,
        deletions: 1,
      },
      {
        path: "src/worktrees.ts",
        status: "M",
        source: "staged",
        additions: 3,
        deletions: 1,
      },
      {
        path: "src/worktrees.ts",
        status: "M",
        source: "unstaged",
        additions: 3,
        deletions: 1,
      },
      {
        path: "notes/review.md",
        status: "?",
        source: "untracked",
        additions: 2,
        deletions: 0,
      },
    ],
    stats: {
      committed_files: 1,
      staged_files: 1,
      unstaged_files: 1,
      untracked_files: 1,
      files_changed: 4,
      additions: 12,
      deletions: 3,
    },
    patch: "",
    patch_truncated: false,
  };
  return diff;
}

export function worktreeFileContent(rev: string, path: string): string {
  if (path.startsWith("notes/"))
    return "# Review checklist\nSelected checkout stays attached.\n";
  const stage =
    rev === baseCommit
      ? "base"
      : rev === "INDEX"
        ? "staged"
        : rev === "WORKTREE"
          ? "working"
          : "committed";
  return `export const checkout = "${stage}";\nexport const selectedFirst = ${
    rev !== baseCommit
  };\nexport const iconActions = true;\n`;
}
