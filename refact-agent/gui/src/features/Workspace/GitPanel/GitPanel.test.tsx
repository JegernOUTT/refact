import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { screen, waitFor, within } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import type {
  GitFileChange,
  GitStatusRoot,
} from "../../../services/refact/gitRead";
import type { WorktreeRecordView } from "../../../services/refact/worktrees";
import { GitPanel } from "./GitPanel";

const APP_CHANGE: GitFileChange = {
  relative_path: "src/app.ts",
  absolute_path: "/repo/src/app.ts",
  status: "MODIFIED",
};

const OTHER_CHANGE: GitFileChange = {
  relative_path: "README.md",
  absolute_path: "/other/README.md",
  status: "MODIFIED",
};

function statusRoot(
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

function renderPanel(workspaceRoots = ["/repo"]) {
  return render(<GitPanel />, {
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
  });
}

function installHandlers(options?: {
  status?: () => GitStatusRoot[];
  statusCalls?: string[];
  diffCalls?: URLSearchParams[];
  commitBodies?: unknown[];
  worktreeCalls?: URLSearchParams[];
  openCalls?: string[];
  deleteCalls?: string[];
}) {
  const record = worktreeRecord();
  server.use(
    http.get("*/v1/git/status", () => {
      options?.statusCalls?.push("status");
      return HttpResponse.json({
        roots: options?.status?.() ?? [statusRoot("/repo", [], [APP_CHANGE])],
      });
    }),
    http.post("*/v1/git/stage", () =>
      HttpResponse.json({ staged: 1, skipped: 0 }),
    ),
    http.post("*/v1/git/unstage", () => HttpResponse.json({ unstaged: 1 })),
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
                message: "Initial commit",
              },
            ],
          },
        ],
      }),
    ),
    http.post("*/v1/git-commit", async ({ request }) => {
      options?.commitBodies?.push(await request.json());
      return HttpResponse.json({
        commits_applied: [
          {
            project_path: "/other",
            project_name: "other",
            commit_oid: "0123456789abcdef",
          },
        ],
        error_log: [],
      });
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
    http.delete("*/v1/worktrees/:id", ({ params }) => {
      options?.deleteCalls?.push(String(params.id));
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

beforeEach(() => {
  installHandlers();
});

describe("GitPanel", () => {
  test("renders status and invalidates it after stage and unstage", async () => {
    let staged = false;
    const statusCalls: string[] = [];
    installHandlers({
      statusCalls,
      status: () => [
        statusRoot(
          "/repo",
          staged ? [APP_CHANGE] : [],
          staged ? [] : [APP_CHANGE],
        ),
      ],
    });
    server.use(
      http.post("*/v1/git/stage", () => {
        staged = true;
        return HttpResponse.json({ staged: 1, skipped: 0 });
      }),
      http.post("*/v1/git/unstage", () => {
        staged = false;
        return HttpResponse.json({ unstaged: 1 });
      }),
    );

    const { user } = renderPanel();
    await user.click(
      await screen.findByRole("checkbox", { name: "Stage src/app.ts" }),
    );
    await screen.findByRole("checkbox", { name: "Unstage src/app.ts" });
    await user.click(
      screen.getByRole("checkbox", { name: "Unstage src/app.ts" }),
    );
    await screen.findByRole("checkbox", { name: "Stage src/app.ts" });

    expect(statusCalls.length).toBeGreaterThanOrEqual(3);
  });

  test("fetches and renders a file diff and read-only branch history", async () => {
    const diffCalls: URLSearchParams[] = [];
    installHandlers({ diffCalls });
    const { user } = renderPanel();

    await user.click(
      await screen.findByRole("button", { name: /src\/app.ts/ }),
    );

    await waitFor(() => expect(diffCalls).toHaveLength(1));
    expect(diffCalls[0]?.get("root")).toBe("/repo");
    expect(diffCalls[0]?.get("path")).toBe("src/app.ts");
    expect(diffCalls[0]?.get("staged")).toBe("false");
    expect(await screen.findByText("+const value = 1;")).toBeInTheDocument();
    expect(await screen.findByText("Initial commit")).toBeInTheDocument();
    expect(screen.getByText("feature/docs")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /checkout|switch/i }),
    ).not.toBeInTheDocument();
  });

  test("commits only the active root staged files and refreshes status", async () => {
    const commitBodies: unknown[] = [];
    const statusCalls: string[] = [];
    installHandlers({
      commitBodies,
      statusCalls,
      status: () => [
        statusRoot("/repo", [APP_CHANGE], []),
        statusRoot("/other", [OTHER_CHANGE], []),
      ],
    });
    const { user } = renderPanel(["/repo", "/other"]);

    await user.click(await screen.findByRole("tab", { name: "other" }));
    await user.type(
      screen.getByRole("textbox", { name: "Commit message" }),
      "Update docs",
    );
    expect(
      screen.queryByRole("button", { name: /generate message/i }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Commit staged changes" }),
    );

    await waitFor(() => expect(commitBodies).toHaveLength(1));
    expect(commitBodies[0]).toEqual({
      commits: [
        {
          root: "/other",
          commit_message: "Update docs",
          staged_changes: [OTHER_CHANGE],
          unstaged_changes: [],
        },
      ],
    });
    expect(await screen.findByText("Committed 01234567")).toBeInTheDocument();
    expect(statusCalls.length).toBeGreaterThanOrEqual(2);
  });

  test.each(["C:\\repo", "/repo#reserved"])(
    "submits the plain status root without URL mangling: %s",
    async (root) => {
      const change: GitFileChange = {
        relative_path: "src/app.ts",
        absolute_path: `${root}/src/app.ts`,
        status: "MODIFIED",
      };
      const commitBodies: unknown[] = [];
      installHandlers({
        commitBodies,
        status: () => [statusRoot(root, [change], [])],
      });
      const { user } = renderPanel([root]);

      await user.type(
        await screen.findByRole("textbox", { name: "Commit message" }),
        "Keep root literal",
      );
      await user.click(
        screen.getByRole("button", { name: "Commit staged changes" }),
      );

      await waitFor(() => expect(commitBodies).toHaveLength(1));
      expect(commitBodies[0]).toEqual({
        commits: [
          {
            root,
            commit_message: "Keep root literal",
            staged_changes: [change],
            unstaged_changes: [],
          },
        ],
      });
    },
  );

  test("lists worktrees and supports diff, open, and cleanup actions", async () => {
    const openCalls: string[] = [];
    const deleteCalls: string[] = [];
    installHandlers({ openCalls, deleteCalls });
    const { user } = renderPanel();

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
    await waitFor(() => expect(deleteCalls).toEqual(["wt-1"]));
  });

  test("drives git requests with status-provided roots for subdirectory workspaces", async () => {
    const branchRoots: (string | null)[] = [];
    const logRoots: (string | null)[] = [];
    const stageBodies: unknown[] = [];
    const worktreeCalls: URLSearchParams[] = [];
    installHandlers({
      status: () => [statusRoot("/repo", [], [APP_CHANGE])],
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
      http.post("*/v1/git/stage", async ({ request }) => {
        stageBodies.push(await request.json());
        return HttpResponse.json({ staged: 1, skipped: 0 });
      }),
    );
    const { user } = renderPanel(["/repo/refact-agent/engine"]);

    await user.click(
      await screen.findByRole("checkbox", { name: "Stage src/app.ts" }),
    );

    await waitFor(() => expect(stageBodies).toHaveLength(1));
    expect(stageBodies[0]).toEqual({ root: "/repo", paths: ["src/app.ts"] });
    await waitFor(() => expect(branchRoots.length).toBeGreaterThan(0));
    await waitFor(() => expect(logRoots.length).toBeGreaterThan(0));
    await waitFor(() => expect(worktreeCalls).toHaveLength(1));
    expect(branchRoots.every((root) => root === "/repo")).toBe(true);
    expect(logRoots.every((root) => root === "/repo")).toBe(true);
    expect(worktreeCalls[0]?.get("source_workspace_root")).toBe(
      "/repo/refact-agent/engine",
    );
    expect(await screen.findByText("refact/wt-1")).toBeInTheDocument();
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
          ...statusRoot("/repo", [], [APP_CHANGE]),
          branch: null,
          head_detached: true,
        },
      ],
    });
    renderPanel();

    expect(await screen.findByText("Detached HEAD")).toBeInTheDocument();
  });
});
