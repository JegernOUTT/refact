import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import type {
  GitChange,
  GitFileChange,
  GitStatusRoot,
} from "../../../services/refact/gitRead";
import type { WorktreeRecordView } from "../../../services/refact/worktrees";
import { GitDock } from "./GitDock";
import { GitPanel } from "./GitPanel";
import { selectGitDiffTargetForChat, setActiveGitRoot } from "./gitPanelSlice";
import { createChatWithId } from "../../Chat/Thread/actions";
import { openTab, setActiveTab } from "../workspaceSlice";
import { gitDiffDescriptorFromSurfaceKey } from "../surfaceKey";
import type { WorktreeMeta } from "../../../services/refact/worktrees";

const APP_CHANGE: GitFileChange = {
  relative_path: "src/app.ts",
  absolute_path: "/repo/src/app.ts",
  status: "MODIFIED",
};

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

function legacyRoot(
  root: string,
  staged: GitFileChange[],
  unstaged: GitFileChange[],
): GitStatusRoot {
  return {
    root,
    branch: root === "/repo" ? "main" : "feature/docs",
    head_detached: false,
    ahead: 0,
    behind: 0,
    staged,
    unstaged,
    untracked_included: true,
  };
}

function statusRoot(
  root: string,
  extra: Partial<GitStatusRoot> = {},
): GitStatusRoot {
  return {
    ...legacyRoot(root, [], []),
    head_oid: "abc1234567890",
    upstream: "origin/main",
    repo_state: "clean",
    truncated: false,
    changes: [change("src/app.ts")],
    untracked: [],
    conflicted: [],
    ...extra,
  };
}

const RICH_STATUS = statusRoot("/repo", {
  ahead: 2,
  behind: 1,
  repo_state: "merge",
  changes: [
    change("src/app.ts"),
    change("src/new.ts", "ADDED"),
    change("docs/guide.md", "RENAMED", "docs/old-guide.md"),
  ],
  untracked: [{ relative_path: "notes.txt", absolute_path: "/repo/notes.txt" }],
  conflicted: [
    {
      relative_path: "lib/conflict.ts",
      absolute_path: "/repo/lib/conflict.ts",
      ours: "a",
      theirs: "b",
      ancestor: "c",
    },
  ],
});

function worktreeRecord(): WorktreeRecordView {
  return {
    meta: {
      id: "wt-1",
      kind: "chat",
      root: "/tmp/wt-1",
      source_workspace_root: "/repo",
      repo_root: "/repo",
      branch: "refact/wt-1",
      base_branch: "main",
      base_commit: "abc123",
      enforce: false,
    },
    created_at: "2026-07-18T00:00:00Z",
    updated_at: "2026-07-18T00:00:00Z",
    references: [],
    reference_count: 0,
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty: false,
      staged_count: 0,
      unstaged_count: 0,
      untracked_count: 0,
      branch: "refact/wt-1",
      head_commit: "abc123",
    },
  };
}

function chatWorktree(id: string): WorktreeMeta {
  return {
    id,
    kind: "chat",
    root: `/worktrees/${id}`,
    source_workspace_root: "/repo",
    repo_root: "/repo",
    enforce: false,
  };
}

function chatWorktreeForSource(id: string, sourceWorkspaceRoot: string) {
  return {
    ...chatWorktree(id),
    source_workspace_root: sourceWorkspaceRoot,
    repo_root: sourceWorkspaceRoot,
  };
}

function renderPanel(workspaceRoots = ["/repo"]) {
  return render(
    <>
      <GitDock />
      <GitPanel />
    </>,
    {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          engineServed: true,
          themeProps: { appearance: "dark" },
        },
        current_project: {
          name: "repo",
          workspaceRoots,
        },
      },
    },
  );
}

function installHandlers(options?: {
  status?: () => GitStatusRoot[];
  statusCalls?: (string | null)[];
  diffCalls?: URLSearchParams[];
  commitBodies?: unknown[];
  rollbackBodies?: unknown[];
  stageBodies?: unknown[];
  generateBodies?: unknown[];
  generatedMessage?: string;
  commitError?: { status: number; body: { code: string; error: string } };
  worktreeCalls?: URLSearchParams[];
  openCalls?: string[];
  deleteCalls?: { id: string; sourceWorkspaceRoot: string | null }[];
}) {
  const record = worktreeRecord();
  server.use(
    http.get("*/v1/git/status", ({ request }) => {
      options?.statusCalls?.push(new URL(request.url).searchParams.get("root"));
      return HttpResponse.json({
        roots: options?.status?.() ?? [statusRoot("/repo")],
      });
    }),
    http.post("*/v1/git/stage", async ({ request }) => {
      options?.stageBodies?.push(await request.json());
      return HttpResponse.json({ staged: 1, skipped: 0 });
    }),
    http.post("*/v1/git/rollback", async ({ request }) => {
      const body = (await request.json()) as {
        paths: string[];
        delete_added_files?: boolean;
        delete_untracked?: boolean;
      };
      options?.rollbackBodies?.push(body);
      // Mirrors the engine: file lists are repo-relative path arrays.
      const added = new Set(["src/new.ts", "docs/guide.md"]);
      const untracked = body.paths.filter((path) => path === "notes.txt");
      const unstaged = body.paths.filter((path) => added.has(path));
      const restored = body.paths.filter(
        (path) => !added.has(path) && path !== "notes.txt",
      );
      return HttpResponse.json({
        restored,
        unstaged,
        deleted: [
          ...(body.delete_added_files ? unstaged : []),
          ...(body.delete_untracked ? untracked : []),
        ],
        skipped: [],
      });
    }),
    http.get("*/v1/git/diff", ({ request }) => {
      options?.diffCalls?.push(new URL(request.url).searchParams);
      return HttpResponse.json({
        roots: [
          {
            root: new URL(request.url).searchParams.get("root"),
            patch: "diff --git a/src/app.ts b/src/app.ts\n+const value = 1;",
            truncated: false,
          },
        ],
      });
    }),
    http.get("*/v1/git/branches", ({ request }) =>
      HttpResponse.json({
        roots: [
          {
            root: new URL(request.url).searchParams.get("root"),
            current: "main",
            branches: [
              { name: "main", is_head: true, upstream: "origin/main" },
              { name: "feature/docs", is_head: false, upstream: null },
            ],
          },
        ],
      }),
    ),
    http.get("*/v1/git/log", ({ request }) =>
      HttpResponse.json({
        roots: [
          {
            root: new URL(request.url).searchParams.get("root"),
            commits: [
              {
                oid: "abc123456789",
                short_oid: "abc1234",
                time_ms: 1,
                author_name: "Ada",
                author_email: "ada@example.com",
                message_first_line: "Initial commit",
                message: "Initial commit\n\nBody line\n",
              },
            ],
          },
        ],
      }),
    ),
    http.post("*/v1/git/commit", async ({ request }) => {
      options?.commitBodies?.push(await request.json());
      if (options?.commitError) {
        return HttpResponse.json(options.commitError.body, {
          status: options.commitError.status,
        });
      }
      return HttpResponse.json({
        oid: "0123456789abcdef",
        short_oid: "0123456",
        summary: "Update docs",
        used_git_cli: false,
        hooks_ran: false,
        skipped: [],
      });
    }),
    http.get("*/v1/git/commit", ({ request }) => {
      const params = new URL(request.url).searchParams;
      return HttpResponse.json({
        root: params.get("root"),
        oid: params.get("oid"),
        short_oid: "abc1234",
        parents: [],
        author_name: "Ada",
        author_email: "ada@example.com",
        author_time_ms: 1,
        committer_name: "Ada",
        committer_email: "ada@example.com",
        committer_time_ms: 1,
        message: "Initial commit\n\nBody line\n",
        refs: [],
        branches: ["main"],
        files: [],
        files_truncated: false,
      });
    }),
    http.post("*/v1/commit-message-from-diff", async ({ request }) => {
      options?.generateBodies?.push(await request.json());
      return HttpResponse.text(
        options?.generatedMessage ?? "Generated commit message",
      );
    }),
    http.get("*/v1/worktrees", ({ request }) => {
      options?.worktreeCalls?.push(new URL(request.url).searchParams);
      return HttpResponse.json({
        project_hash: "project",
        source_workspace_root: "/repo",
        source_current_branch: "main",
        source_branches: ["main"],
        worktrees: [record],
      });
    }),
    http.get("*/v1/worktrees/:id/diff", () =>
      HttpResponse.json({
        id: "wt-1",
        status: record.status,
        files: [],
        stats: {
          committed_files: 0,
          staged_files: 0,
          unstaged_files: 0,
          untracked_files: 0,
          files_changed: 0,
        },
        patch: "diff --git a/wt.txt b/wt.txt\n+worktree",
        patch_truncated: false,
      }),
    ),
    http.post("*/v1/worktrees/:id/open", ({ params }) => {
      options?.openCalls?.push(String(params.id));
      return HttpResponse.json({
        id: params.id,
        path: "/tmp/wt-1",
        can_open_folder: false,
      });
    }),
    http.delete("*/v1/worktrees/:id", ({ params, request }) => {
      options?.deleteCalls?.push({
        id: String(params.id),
        sourceWorkspaceRoot: new URL(request.url).searchParams.get(
          "source_workspace_root",
        ),
      });
      return HttpResponse.json({
        deleted: true,
        branch_deleted: false,
        stale_path: false,
        affected_references: [],
        affected_reference_count: 0,
        warnings: [],
      });
    }),
  );
}

const messageBox = () =>
  screen.getByRole("textbox", { name: "Commit message" });
const commitButton = () => screen.getByRole("button", { name: "Commit" });

async function openWorktreesTab(user: ReturnType<typeof renderPanel>["user"]) {
  await user.click(await screen.findByRole("tab", { name: "Worktrees" }));
}

async function openMenu(
  user: ReturnType<typeof renderPanel>["user"],
  name: string,
) {
  const row = await screen.findByRole("treeitem", { name });
  await user.pointer({ target: row, keys: "[MouseRight]" });
}

beforeEach(() => {
  installHandlers();
});

describe("GitDock commit tool window", () => {
  test("renders header, groups, compact directories and status tones", async () => {
    installHandlers({ status: () => [RICH_STATUS] });
    renderPanel();

    expect(
      await screen.findByRole("heading", { name: "Commit" }),
    ).toBeInTheDocument();
    expect(screen.getByText("main")).toBeInTheDocument();
    expect(screen.getByText("↑2 ↓1")).toBeInTheDocument();
    expect(screen.getByText("Merging")).toBeInTheDocument();
    expect(
      screen.getByRole("toolbar", { name: "Commit tool window actions" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Group by Directory" }),
    ).toHaveAttribute("aria-pressed", "true");

    const tree = screen.getByRole("tree", { name: "Changes" });
    const items = within(tree).getAllByRole("treeitem");
    expect(items.map((item) => item.getAttribute("aria-label"))).toEqual([
      "Changes",
      "docs",
      "docs/guide.md",
      "src",
      "src/app.ts",
      "src/new.ts",
      "Merge Conflicts",
      "lib",
      "lib/conflict.ts",
      "Unversioned Files",
      "notes.txt",
    ]);
    expect(items[0]).toHaveTextContent("3 files");
    expect(within(items[2]).getByText("renamed from docs/old-guide.md"));
    expect(within(items[5]).getByText("new.ts").getAttribute("data-tone")).toBe(
      "added",
    );
    expect(
      within(items[8]).getByText("conflict.ts").getAttribute("data-tone"),
    ).toBe("conflict");
    expect(
      within(items[10]).getByText("notes.txt").getAttribute("data-tone"),
    ).toBe("untracked");
    expect(within(items[8]).queryByRole("checkbox")).not.toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", { name: "Include notes.txt in commit" }),
    ).not.toBeChecked();
    expect(screen.getByText("3 of 4 files included")).toBeInTheDocument();
  });

  test("flat mode shows the parent directory after the file name", async () => {
    installHandlers({ status: () => [RICH_STATUS] });
    const { user } = renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Group by Directory" }),
    );

    const row = screen.getByRole("treeitem", { name: "src/new.ts" });
    expect(row).toHaveAttribute("aria-level", "2");
    expect(within(row).getByText("src")).toBeInTheDocument();
  });

  test("commits included paths with rename sources and included unversioned files", async () => {
    const commitBodies: unknown[] = [];
    const statusCalls: (string | null)[] = [];
    installHandlers({
      commitBodies,
      statusCalls,
      status: () => [RICH_STATUS],
    });
    const { user } = renderPanel();

    await user.click(
      await screen.findByRole("checkbox", {
        name: "Include src/app.ts in commit",
      }),
    );
    await user.click(
      screen.getByRole("checkbox", { name: "Include notes.txt in commit" }),
    );
    expect(
      screen.getByRole("checkbox", { name: "Include src in commit" }),
    ).toHaveProperty("indeterminate", true);
    expect(commitButton()).toBeDisabled();
    await user.type(messageBox(), "Update docs");
    const callsBefore = statusCalls.length;
    await user.click(commitButton());

    await waitFor(() => expect(commitBodies).toHaveLength(1));
    expect(commitBodies[0]).toEqual({
      root: "/repo",
      message: "Update docs",
      paths: ["src/new.ts", "docs/guide.md", "docs/old-guide.md", "notes.txt"],
    });
    expect(
      await screen.findByText("Committed 0123456: Update docs"),
    ).toBeInTheDocument();
    expect(messageBox()).toHaveValue("");
    await waitFor(() =>
      expect(statusCalls.length).toBeGreaterThan(callsBefore),
    );
  });

  test("Ctrl+Enter commits and hook failures show their output", async () => {
    const commitBodies: unknown[] = [];
    installHandlers({
      commitBodies,
      commitError: {
        status: 422,
        body: { code: "hook_failed", error: "lint: 2 errors" },
      },
    });
    const { user } = renderPanel();

    await user.type(
      await screen.findByRole("textbox", { name: "Commit message" }),
      "Fix",
    );
    await user.keyboard("{Control>}{Enter}{/Control}");

    await waitFor(() => expect(commitBodies).toHaveLength(1));
    expect(
      await screen.findByText("A Git hook rejected the commit."),
    ).toBeInTheDocument();
    expect(screen.getByText("lint: 2 errors")).toBeInTheDocument();
    expect(messageBox()).toHaveValue("Fix");
  });

  test("amend prefills the HEAD message and supports message-only amend", async () => {
    const commitBodies: unknown[] = [];
    installHandlers({ commitBodies });
    const { user } = renderPanel();

    await user.click(
      await screen.findByRole("checkbox", {
        name: "Include src/app.ts in commit",
      }),
    );
    await user.click(screen.getByRole("checkbox", { name: "Amend" }));
    await waitFor(() =>
      expect(messageBox()).toHaveValue("Initial commit\n\nBody line"),
    );
    await user.click(screen.getByRole("checkbox", { name: "Amend" }));
    expect(messageBox()).toHaveValue("");
    await user.click(screen.getByRole("checkbox", { name: "Amend" }));
    await waitFor(() => expect(messageBox()).not.toHaveValue(""));
    await user.click(screen.getByRole("button", { name: "Amend Commit" }));

    await waitFor(() => expect(commitBodies).toHaveLength(1));
    expect(commitBodies[0]).toEqual({
      root: "/repo",
      message: "Initial commit\n\nBody line",
      paths: [],
      amend: true,
    });
    expect(
      await screen.findByRole("checkbox", { name: "Amend" }),
    ).not.toBeChecked();
  });

  test("disables amend on an unborn HEAD", async () => {
    installHandlers({
      status: () => [statusRoot("/repo", { head_oid: null })],
    });
    renderPanel();

    expect(
      await screen.findByRole("checkbox", { name: "Amend" }),
    ).toBeDisabled();
  });

  test("disables amend with a reason while a merge is in progress", async () => {
    installHandlers({
      status: () => [statusRoot("/repo", { repo_state: "merge" })],
    });
    renderPanel();

    const amend = await screen.findByRole("checkbox", { name: "Amend" });
    expect(amend).toBeDisabled();
    expect(amend.closest("label")).toHaveAttribute(
      "title",
      "Merging is in progress — finish or abort it before amending.",
    );
  });

  test("generates a message from the HEAD diff of included paths", async () => {
    const diffCalls: URLSearchParams[] = [];
    const generateBodies: unknown[] = [];
    installHandlers({
      diffCalls,
      generateBodies,
      generatedMessage: "feat: generated",
    });
    const { user } = renderPanel();

    await user.type(
      await screen.findByRole("textbox", { name: "Commit message" }),
      "Use conventional commits",
    );
    await user.click(
      screen.getByRole("button", { name: "Generate commit message" }),
    );

    await waitFor(() => expect(generateBodies).toHaveLength(1));
    expect(generateBodies[0]).toEqual({
      diff: "diff --git a/src/app.ts b/src/app.ts\n+const value = 1;",
      text: "Use conventional commits",
    });
    const call = diffCalls.find((params) => params.get("against") === "head");
    expect(call?.getAll("path")).toEqual(["src/app.ts"]);
    await waitFor(() => expect(messageBox()).toHaveValue("feat: generated"));
  });

  test("rollback dialog sends paths and delete_added_files", async () => {
    const rollbackBodies: unknown[] = [];
    installHandlers({ rollbackBodies, status: () => [RICH_STATUS] });
    const { user } = renderPanel();

    await openMenu(user, "src");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rollback…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Rollback Changes",
    });
    expect(within(dialog).getByText("src/app.ts")).toBeInTheDocument();
    await user.click(
      within(dialog).getByRole("checkbox", {
        name: "Delete local copies of added files",
      }),
    );
    await user.click(within(dialog).getByRole("button", { name: "Rollback" }));

    await waitFor(() => expect(rollbackBodies).toHaveLength(1));
    expect(rollbackBodies[0]).toEqual({
      root: "/repo",
      paths: ["src/app.ts", "src/new.ts"],
      delete_added_files: true,
    });
    expect(await screen.findByText("Rolled back 2 files")).toBeInTheDocument();
  });

  test("rolling back a rename moves it back and keeps the added-files choice", async () => {
    const rollbackBodies: unknown[] = [];
    installHandlers({ rollbackBodies, status: () => [RICH_STATUS] });
    const { user } = renderPanel();

    await openMenu(user, "Changes");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rollback…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Rollback Changes",
    });
    expect(dialog).toHaveTextContent(
      "Renamed files will be moved back to their original paths.",
    );
    // Leave "Delete local copies of added files" unchecked.
    await user.click(within(dialog).getByRole("button", { name: "Rollback" }));

    await waitFor(() => expect(rollbackBodies).toHaveLength(2));
    expect(rollbackBodies).toEqual([
      {
        root: "/repo",
        paths: ["docs/old-guide.md", "src/app.ts", "src/new.ts"],
      },
      {
        root: "/repo",
        paths: ["docs/guide.md"],
        delete_added_files: true,
      },
    ]);
    expect(await screen.findByText("Rolled back 3 files")).toBeInTheDocument();
  });

  test("toolbar Rollback acts only on the selection and never widens it", async () => {
    const rollbackBodies: unknown[] = [];
    installHandlers({ rollbackBodies, status: () => [RICH_STATUS] });
    const { user } = renderPanel();

    const rollbackButton = await screen.findByRole("button", {
      name: "Rollback…",
    });
    // Nothing selected: the included changes are the scope.
    expect(rollbackButton).toBeEnabled();

    await user.click(screen.getByRole("treeitem", { name: "notes.txt" }));
    expect(screen.getByRole("button", { name: "Rollback…" })).toBeDisabled();
    await user.click(screen.getByRole("treeitem", { name: "lib/conflict.ts" }));
    expect(screen.getByRole("button", { name: "Rollback…" })).toBeDisabled();
    await user.click(
      screen.getByRole("treeitem", { name: "Unversioned Files" }),
    );
    expect(screen.getByRole("button", { name: "Rollback…" })).toBeDisabled();

    await user.click(screen.getByRole("treeitem", { name: "src/app.ts" }));
    await user.click(screen.getByRole("button", { name: "Rollback…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Rollback Changes",
    });
    expect(
      within(within(dialog).getByRole("list", { name: "Affected files" }))
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["src/app.ts"]);
    await user.click(within(dialog).getByRole("button", { name: "Rollback" }));
    await waitFor(() =>
      expect(rollbackBodies).toEqual([
        { root: "/repo", paths: ["src/app.ts"] },
      ]),
    );
    expect(await screen.findByText("Rolled back 1 file")).toBeInTheDocument();
  });

  test("Add to VCS stages and Delete rolls back with delete_untracked", async () => {
    const stageBodies: unknown[] = [];
    const rollbackBodies: unknown[] = [];
    installHandlers({
      stageBodies,
      rollbackBodies,
      status: () => [RICH_STATUS],
    });
    const { user } = renderPanel();

    await openMenu(user, "notes.txt");
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Add to VCS" }),
    );
    await waitFor(() =>
      expect(stageBodies).toEqual([{ root: "/repo", paths: ["notes.txt"] }]),
    );

    await openMenu(user, "notes.txt");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Delete Unversioned Files",
    });
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(rollbackBodies).toEqual([
        { root: "/repo", paths: ["notes.txt"], delete_untracked: true },
      ]),
    );
    expect(await screen.findByText("Deleted 1 file")).toBeInTheDocument();
  });

  test("Mark as Resolved stages the conflicted path", async () => {
    const stageBodies: unknown[] = [];
    installHandlers({ stageBodies, status: () => [RICH_STATUS] });
    const { user } = renderPanel();

    await openMenu(user, "lib/conflict.ts");
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Mark as Resolved" }),
    );
    await waitFor(() =>
      expect(stageBodies).toEqual([
        { root: "/repo", paths: ["lib/conflict.ts"] },
      ]),
    );
  });

  test("keyboard: arrows move, Space toggles include, Enter opens the HEAD diff", async () => {
    const { store, user } = renderPanel();

    const tree = await screen.findByRole("tree", { name: "Changes" });
    tree.focus();
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{End}");
    expect(
      screen.getByRole("treeitem", { name: "src/app.ts" }),
    ).toHaveAttribute("aria-selected", "true");
    await user.keyboard(" ");
    expect(
      screen.getByRole("checkbox", { name: "Include src/app.ts in commit" }),
    ).not.toBeChecked();
    await user.keyboard(" ");
    expect(
      screen.getByRole("checkbox", { name: "Include src/app.ts in commit" }),
    ).toBeChecked();
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("treeitem", { name: "src" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("treeitem", { name: "src" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await user.keyboard("{ArrowRight}{ArrowRight}{Enter}");

    const activeTabId = store.getState().workspace.activeTabId ?? "";
    expect(gitDiffDescriptorFromSurfaceKey(activeTabId)).toEqual({
      root: "/repo",
      path: "src/app.ts",
      old_path: null,
      left: { rev: "HEAD", path: "src/app.ts" },
      right: { rev: "WORKTREE", path: "src/app.ts" },
      kind: "working",
    });
    expect(selectGitDiffTargetForChat(store.getState(), null)).toMatchObject({
      root: "/repo",
      path: "src/app.ts",
      old_path: null,
      kind: "change",
    });
  });

  test("degrades to legacy staged/unstaged status", async () => {
    installHandlers({
      status: () => [legacyRoot("/repo", [APP_CHANGE], [APP_CHANGE])],
    });
    renderPanel();

    expect(
      await screen.findByRole("treeitem", { name: "src/app.ts" }),
    ).toBeInTheDocument();
    expect(screen.getByText("1 of 1 file included")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Amend" })).toBeEnabled();
  });

  test("renders a clean state without a tree", async () => {
    installHandlers({ status: () => [statusRoot("/repo", { changes: [] })] });
    renderPanel();

    expect(
      await screen.findByText("No changes — working tree clean"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("tree", { name: "Changes" }),
    ).not.toBeInTheDocument();
    expect(commitButton()).toBeDisabled();
  });

  test("switches roots with the multi-root picker", async () => {
    const commitBodies: unknown[] = [];
    installHandlers({
      commitBodies,
      status: () => [
        statusRoot("/repo"),
        statusRoot("/other", {
          changes: [
            {
              ...change("README.md"),
              absolute_path: "/other/README.md",
            },
          ],
        }),
      ],
    });
    const { store, user } = renderPanel(["/repo", "/other"]);

    await screen.findByRole("combobox", { name: "Git root" });
    store.dispatch(setActiveGitRoot({ chatId: null, root: "/other" }));
    expect(
      await screen.findByRole("treeitem", { name: "README.md" }),
    ).toBeInTheDocument();
    await user.type(messageBox(), "Docs");
    await user.click(commitButton());
    await waitFor(() =>
      expect(commitBodies).toEqual([
        { root: "/other", message: "Docs", paths: ["README.md"] },
      ]),
    );
  });

  test("refetches status for the focused chat worktree", async () => {
    const statusCalls: (string | null)[] = [];
    installHandlers({ statusCalls });
    const { store } = renderPanel();
    store.dispatch(
      createChatWithId({ id: "chat-a", worktree: chatWorktree("a") }),
    );
    store.dispatch(
      createChatWithId({ id: "chat-b", worktree: chatWorktree("b") }),
    );

    store.dispatch(openTab("chat:chat-a"));
    await waitFor(() => expect(statusCalls).toContain("/worktrees/a"));

    store.dispatch(openTab("chat:chat-b"));
    store.dispatch(setActiveTab("chat:chat-b"));
    await waitFor(() => expect(statusCalls).toContain("/worktrees/b"));
  });

  test("lists worktrees and supports diff, open, and cleanup actions", async () => {
    const openCalls: string[] = [];
    const deleteCalls: {
      id: string;
      sourceWorkspaceRoot: string | null;
    }[] = [];
    installHandlers({ openCalls, deleteCalls });
    const { user } = renderPanel();
    await openWorktreesTab(user);

    const section = await screen.findByRole("heading", { name: "Worktrees" });
    const worktreesSection = section.closest("section");
    expect(worktreesSection).not.toBeNull();
    if (!worktreesSection) return;

    await user.click(
      within(worktreesSection).getByRole("button", { name: "Diff" }),
    );
    expect(await screen.findByText("Worktree diff")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close" }));
    await user.click(
      within(worktreesSection).getByRole("button", { name: "Open" }),
    );
    await waitFor(() => expect(openCalls).toEqual(["wt-1"]));
    await user.click(
      within(worktreesSection).getByRole("button", { name: "Cleanup" }),
    );
    await waitFor(() =>
      expect(deleteCalls).toEqual([
        { id: "wt-1", sourceWorkspaceRoot: "/repo" },
      ]),
    );
  });

  test("drives git requests with status-provided roots for subdirectory workspaces", async () => {
    const branchRoots: (string | null)[] = [];
    const logRoots: (string | null)[] = [];
    const worktreeCalls: URLSearchParams[] = [];
    installHandlers({
      worktreeCalls,
    });
    server.use(
      http.get("*/v1/git/branches", ({ request }) => {
        branchRoots.push(new URL(request.url).searchParams.get("root"));
        return HttpResponse.json({
          roots: [
            {
              root: "/repo",
              current: "main",
              branches: [{ name: "main", is_head: true, upstream: null }],
            },
          ],
        });
      }),
      http.get("*/v1/git/log", ({ request }) => {
        logRoots.push(new URL(request.url).searchParams.get("root"));
        return HttpResponse.json({ roots: [{ root: "/repo", commits: [] }] });
      }),
    );
    const { user } = renderPanel(["/repo/refact-agent/engine"]);

    await waitFor(() => expect(branchRoots.length).toBeGreaterThan(0));
    await openWorktreesTab(user);
    await waitFor(() => expect(logRoots.length).toBeGreaterThan(0));
    await waitFor(() => expect(worktreeCalls).toHaveLength(1));
    expect(branchRoots.every((root) => root === "/repo")).toBe(true);
    expect(logRoots.every((root) => root === "/repo")).toBe(true);
    expect(worktreeCalls[0]?.get("source_workspace_root")).toBe(
      "/repo/refact-agent/engine",
    );
    expect(await screen.findByText("refact/wt-1")).toBeInTheDocument();
  });

  test("routes worktree actions through the focused chat explicit source root", async () => {
    const worktreeCalls: URLSearchParams[] = [];
    const deleteCalls: {
      id: string;
      sourceWorkspaceRoot: string | null;
    }[] = [];
    installHandlers({ worktreeCalls, deleteCalls });
    const { store, user } = renderPanel([
      "/repo/packages/a",
      "/repo/packages/b",
    ]);
    store.dispatch(
      createChatWithId({
        id: "chat-b",
        worktree: chatWorktreeForSource("b", "/repo/packages/b"),
      }),
    );
    store.dispatch(openTab("chat:chat-b"));
    await openWorktreesTab(user);

    const worktreesHeading = await screen.findByRole("heading", {
      name: "Worktrees",
    });
    const worktreesSection = worktreesHeading.closest("section");
    expect(worktreesSection).not.toBeNull();
    if (!worktreesSection) return;
    await waitFor(() => expect(worktreeCalls).toHaveLength(1));
    expect(worktreeCalls[0]?.get("source_workspace_root")).toBe(
      "/repo/packages/b",
    );

    await user.click(
      within(worktreesSection).getByRole("button", { name: "Cleanup" }),
    );
    await waitFor(() =>
      expect(deleteCalls).toEqual([
        { id: "wt-1", sourceWorkspaceRoot: "/repo/packages/b" },
      ]),
    );
  });

  test("shows a no-repository empty state when status returns zero roots", async () => {
    installHandlers({ status: () => [] });
    renderPanel(["/repo/refact-agent/engine"]);

    expect(
      await screen.findByText("No git repository found in this workspace."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Detached HEAD")).not.toBeInTheDocument();
    expect(
      screen.queryByText("No Git repository status is available."),
    ).not.toBeInTheDocument();
  });

  test("labels detached HEAD only from the status flag", async () => {
    installHandlers({
      status: () => [
        {
          ...legacyRoot("/repo", [], [APP_CHANGE]),
          branch: null,
          head_detached: true,
        },
      ],
    });
    renderPanel();

    expect(await screen.findByText("Detached HEAD")).toBeInTheDocument();
  });
});
