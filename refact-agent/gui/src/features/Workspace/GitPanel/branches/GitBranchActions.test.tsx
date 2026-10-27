import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { render } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import type {
  GitBranchesRoot,
  GitCommitLogEntry,
  GitStatusRoot,
} from "../../../../services/refact/gitRead";
import { GitDock } from "../GitDock";
import { GitPanel } from "../GitPanel";

const ROOT = "/repo";

const BRANCHES: GitBranchesRoot = {
  root: ROOT,
  current: "main",
  head_detached: false,
  head_oid: "a".repeat(40),
  branches: [
    {
      name: "main",
      is_head: true,
      upstream: "origin/main",
      tip_oid: "a".repeat(40),
      ahead: 2,
      behind: 0,
      upstream_gone: false,
    },
    {
      name: "feature/login",
      is_head: false,
      upstream: "origin/feature/login",
      tip_oid: "b".repeat(40),
      ahead: 0,
      behind: 0,
      upstream_gone: true,
    },
  ],
  remotes: ["origin", "upstream"],
  remote_branches: [
    {
      name: "origin/main",
      remote: "origin",
      short_name: "main",
      tip_oid: "a".repeat(40),
    },
    {
      name: "origin/release",
      remote: "origin",
      short_name: "release",
      tip_oid: "c".repeat(40),
    },
  ],
  tags: [{ name: "v1.0", target_oid: "d".repeat(40) }],
};

function commit(id: string, subject: string): GitCommitLogEntry {
  return {
    oid: id.repeat(40).slice(0, 40),
    short_oid: id.repeat(7).slice(0, 7),
    time_ms: Date.now(),
    author_name: "Ada",
    author_email: "ada@example.com",
    message_first_line: subject,
    message: subject,
    parents: [],
    refs: [],
  };
}

const COMMITS = [commit("1", "Add login form"), commit("2", "Wire auth API")];

function statusRoot(extra: Partial<GitStatusRoot> = {}): GitStatusRoot {
  return {
    root: ROOT,
    branch: "main",
    head_detached: false,
    ahead: 2,
    behind: 0,
    staged: [],
    unstaged: [],
    untracked_included: true,
    head_oid: "a".repeat(40),
    upstream: "origin/main",
    repo_state: "clean",
    truncated: false,
    changes: [
      {
        relative_path: "src/app.ts",
        absolute_path: "/repo/src/app.ts",
        status: "MODIFIED",
        old_path: null,
        staged: false,
        unstaged: true,
      },
    ],
    untracked: [],
    conflicted: [],
    ...extra,
  };
}

type ErrorReply = { status: number; code: string; error: string };
type Calls = Partial<Record<string, unknown[]>>;

function installHandlers(
  options: {
    status?: GitStatusRoot;
    errors?: Record<string, ErrorReply | ErrorReply[] | undefined>;
    summaries?: Record<string, string>;
    logCalls?: URLSearchParams[];
  } = {},
): Calls {
  const calls: Calls = {};
  const record =
    (path: string) =>
    async ({ request }: { request: Request }) => {
      const body: unknown = await request.json();
      (calls[path] ??= []).push(body);
      const configured = options.errors?.[path];
      const error = Array.isArray(configured) ? configured.shift() : configured;
      if (error) {
        return HttpResponse.json(
          { code: error.code, error: error.error },
          { status: error.status },
        );
      }
      switch (path) {
        case "checkout":
          return HttpResponse.json({
            branch: "feature/login",
            head_oid: "b".repeat(40),
            detached: false,
          });
        case "create":
          return HttpResponse.json({
            name: (body as { name: string }).name,
            tip_oid: "e".repeat(40),
            checked_out: true,
          });
        case "rename":
          return HttpResponse.json({
            name: (body as { new_name: string }).new_name,
          });
        case "delete":
          return HttpResponse.json({ deleted: true });
        case "commit":
          return HttpResponse.json({
            oid: "f".repeat(40),
            short_oid: "fffffff",
            summary: "Ship it",
            used_git_cli: false,
            hooks_ran: false,
            skipped: [],
          });
        default:
          return HttpResponse.json({
            ok: true,
            summary: options.summaries?.[path] ?? "",
            output: "",
          });
      }
    };
  server.use(
    http.get("*/v1/git/status", () =>
      HttpResponse.json({ roots: [options.status ?? statusRoot()] }),
    ),
    http.get("*/v1/git/branches", () =>
      HttpResponse.json({ roots: [BRANCHES] }),
    ),
    http.get("*/v1/git/log", ({ request }) => {
      const params = new URL(request.url).searchParams;
      options.logCalls?.push(params);
      const limit = Number(params.get("limit"));
      return HttpResponse.json({
        roots: [{ root: ROOT, commits: COMMITS.slice(0, limit) }],
      });
    }),
    http.get("*/v1/git/commit", () =>
      HttpResponse.json({
        root: ROOT,
        ...COMMITS[0],
        author_time_ms: 0,
        committer_name: "Ada",
        committer_email: "ada@example.com",
        committer_time_ms: 0,
        branches: [],
        files: [],
        files_truncated: false,
      }),
    ),
    http.get("*/v1/worktrees", () =>
      HttpResponse.json({
        project_hash: "project",
        source_workspace_root: ROOT,
        source_current_branch: "main",
        source_branches: ["main"],
        worktrees: [],
      }),
    ),
    http.post("*/v1/git/branch/checkout", record("checkout")),
    http.post("*/v1/git/branch/create", record("create")),
    http.post("*/v1/git/branch/rename", record("rename")),
    http.post("*/v1/git/branch/delete", record("delete")),
    http.post("*/v1/git/fetch", record("fetch")),
    http.post("*/v1/git/pull", record("pull")),
    http.post("*/v1/git/push", record("push")),
    http.post("*/v1/git/commit", record("commit")),
  );
  return calls;
}

const PRELOADED = {
  config: {
    host: "web" as const,
    lspPort: 8001,
    engineServed: true,
    themeProps: { appearance: "dark" as const },
  },
  current_project: { name: "repo", workspaceRoots: [ROOT] },
};

const renderDock = () => render(<GitDock />, { preloadedState: PRELOADED });
const renderPanel = () => render(<GitPanel />, { preloadedState: PRELOADED });

async function openPopup(user: ReturnType<typeof renderDock>["user"]) {
  await user.click(
    await screen.findByRole("button", { name: "Git branch: main" }),
  );
  const search = await screen.findByRole("combobox", {
    name: "Search branches and actions",
  });
  await waitFor(() =>
    expect(
      screen.getByRole("option", { name: "feature/login" }),
    ).toBeInTheDocument(),
  );
  return search;
}

const listbox = () =>
  screen.getByRole("listbox", { name: "Branches and actions" });

beforeEach(() => {
  installHandlers();
});

describe("branch popup", () => {
  test("search filters actions and branches; Enter checks out a local branch", async () => {
    const calls = installHandlers();
    const { user } = renderDock();
    const search = await openPopup(user);

    expect(within(listbox()).getByText("Update Project")).toBeInTheDocument();
    expect(
      within(listbox()).getByRole("option", { name: "main (current)" }),
    ).toBeInTheDocument();
    expect(within(listbox()).getByText("gone")).toBeInTheDocument();

    await user.type(search, "login");
    const options = within(listbox()).getAllByRole("option");
    expect(options.map((option) => option.getAttribute("aria-label"))).toEqual([
      "feature/login",
    ]);

    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(calls.checkout).toEqual([
        { root: ROOT, name: "feature/login", kind: "local" },
      ]),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Checked out feature/login",
    );
  });

  test("search matches actions and arrow keys reach remote branches", async () => {
    const calls = installHandlers();
    const { user } = renderDock();
    const search = await openPopup(user);

    await user.type(search, "fetch");
    expect(
      within(listbox())
        .getAllByRole("option")
        .map((option) => option.getAttribute("aria-label")),
    ).toEqual(["Fetch"]);

    await user.clear(search);
    await user.type(search, "release");
    expect(within(listbox()).getByText("origin")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{ArrowUp}{Enter}");
    await waitFor(() =>
      expect(calls.checkout).toEqual([
        { root: ROOT, name: "origin/release", kind: "remote" },
      ]),
    );
  });

  test("checkout_conflict shows the conflicting files and raw output", async () => {
    installHandlers({
      errors: {
        checkout: {
          status: 409,
          code: "checkout_conflict",
          error:
            "error: Your local changes to the following files would be overwritten by checkout:\n\tsrc/app.ts\n\tsrc/auth.ts\nPlease commit your changes or stash them before you switch branches.",
        },
      },
    });
    const { user } = renderDock();
    await openPopup(user);
    await user.click(
      within(listbox()).getByRole("option", { name: "feature/login" }),
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Checkout would overwrite local changes");
    const files = within(alert).getByRole("list", {
      name: "Conflicting files",
    });
    expect(
      within(files)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["src/app.ts", "src/auth.ts"]);
    expect(within(alert).getByText("Git output")).toBeInTheDocument();
  });

  test("current branch row is a no-op and its submenu offers no Delete", async () => {
    const calls = installHandlers();
    const { user } = renderDock();
    await openPopup(user);
    const current = within(listbox()).getByRole("option", {
      name: "main (current)",
    });
    await user.click(within(current).getByTitle("Actions for main"));
    expect(
      within(listbox()).getByRole("option", { name: "Rename…" }),
    ).toBeInTheDocument();
    expect(
      within(listbox()).queryByRole("option", { name: "Delete…" }),
    ).not.toBeInTheDocument();
    await user.click(current);
    expect(calls.checkout).toBeUndefined();
  });

  test("New Branch from a remote branch sends start_point and checkout", async () => {
    const calls = installHandlers();
    const { user } = renderDock();
    const search = await openPopup(user);
    await user.type(search, "release");
    await user.keyboard("{ArrowRight}");
    await user.click(
      within(listbox()).getByRole("option", {
        name: "New Branch from 'release'…",
      }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "Create New Branch",
    });
    expect(dialog).toHaveTextContent("From 'origin/release'");
    const name = within(dialog).getByRole("textbox", {
      name: "New branch name",
    });
    expect(name).toHaveValue("release");
    expect(
      within(dialog).getByRole("checkbox", { name: "Checkout branch" }),
    ).toBeChecked();
    await user.clear(name);
    await user.type(name, "bad..name");
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "not a valid Git branch name",
    );
    expect(
      within(dialog).getByRole("button", { name: "Create" }),
    ).toBeDisabled();
    await user.clear(name);
    await user.type(name, "release-fix");
    await user.click(within(dialog).getByRole("button", { name: "Create" }));

    await waitFor(() =>
      expect(calls.create).toEqual([
        {
          root: ROOT,
          name: "release-fix",
          checkout: true,
          start_point: "origin/release",
        },
      ]),
    );
  });

  test("Delete on not_merged offers Force Delete", async () => {
    const calls = installHandlers({
      errors: {
        delete: {
          status: 409,
          code: "not_merged",
          error: "error: the branch 'feature/login' is not fully merged",
        },
      },
    });
    const { user } = renderDock();
    await openPopup(user);
    await user.click(within(listbox()).getByTitle("Actions for feature/login"));
    await user.click(
      within(listbox()).getByRole("option", { name: "Delete…" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Delete Branch" });
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    const force = await within(
      await screen.findByRole("dialog", { name: "Delete Branch" }),
    ).findByRole("button", { name: "Force Delete" });
    expect(screen.getByRole("alert")).toHaveTextContent("not fully merged");
    await user.click(force);
    await waitFor(() =>
      expect(calls.delete).toEqual([
        { root: ROOT, name: "feature/login" },
        { root: ROOT, name: "feature/login", force: true },
      ]),
    );
  });

  test("checkout is disabled while a merge is in progress", async () => {
    const calls = installHandlers({
      status: statusRoot({ repo_state: "merge" }),
    });
    const { user } = renderDock();
    await openPopup(user);
    expect(
      within(listbox()).getByRole("option", { name: "Update Project" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("button", { name: "Update Project" }),
    ).toBeDisabled();
    await user.click(
      within(listbox()).getByRole("option", { name: "feature/login" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Merging — finish or abort it",
    );
    expect(calls.checkout).toBeUndefined();
  });
});

describe("push dialog", () => {
  test("lists outgoing commits for the ahead count and pushes", async () => {
    const logCalls: URLSearchParams[] = [];
    const calls = installHandlers({ logCalls });
    const { user } = renderDock();
    await user.click(await screen.findByRole("button", { name: "Push…" }));

    const dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    const list = await within(dialog).findByRole("list", {
      name: "Outgoing commits",
    });
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
    expect(dialog).toHaveTextContent("origin/main");
    expect(logCalls.at(-1)?.get("limit")).toBe("2");

    await user.click(within(dialog).getByRole("button", { name: "Push" }));
    await waitFor(() =>
      expect(calls.push).toEqual([
        { root: ROOT, remote: "origin", branch: "main" },
      ]),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Pushed 2 commits to origin/main",
    );
  });

  test("no_upstream switches to set-upstream mode with the chosen remote", async () => {
    const calls = installHandlers({
      errors: {
        push: [
          { status: 409, code: "no_upstream", error: "no upstream configured" },
        ],
      },
    });
    const { user } = renderDock();
    await user.click(await screen.findByRole("button", { name: "Push…" }));
    let dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    await user.click(within(dialog).getByRole("button", { name: "Push" }));

    dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    expect(await within(dialog).findByText("New")).toBeInTheDocument();
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "no upstream branch",
    );
    await user.click(
      within(dialog).getByRole("checkbox", {
        name: "Force with lease",
      }),
    );
    await user.click(
      within(dialog).getByRole("button", { name: "Force Push" }),
    );
    await waitFor(() =>
      expect(calls.push?.at(-1)).toEqual({
        root: ROOT,
        remote: "origin",
        branch: "main",
        set_upstream: true,
        force_with_lease: true,
      }),
    );
  });
});

describe("push upstream mismatch", () => {
  test("upstream_mismatch reopens Push with an explicit target", async () => {
    const calls = installHandlers({
      status: statusRoot({ upstream: "origin/trunk" }),
      errors: {
        push: [
          {
            status: 409,
            code: "upstream_mismatch",
            error: "Branch main tracks origin/trunk, whose name differs",
          },
        ],
      },
    });
    const { user } = renderDock();
    await user.click(await screen.findByRole("button", { name: "Push…" }));
    let dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    await user.click(within(dialog).getByRole("button", { name: "Push" }));

    dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    expect(
      await within(dialog).findByRole("combobox", { name: "Remote" }),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "The local branch and its upstream have different names — choose the push target in the Push dialog.",
    );
    expect(dialog).toHaveTextContent("main tracks origin/trunk");
    await user.click(within(dialog).getByRole("button", { name: "Push" }));
    await waitFor(() =>
      expect(calls.push?.at(-1)).toEqual({
        root: ROOT,
        remote: "origin",
        branch: "main",
        set_upstream: true,
      }),
    );
  });
});

describe("dock toolbar remote actions", () => {
  test("fetch and update report the engine summary", async () => {
    const calls = installHandlers({
      summaries: { fetch: "Fetched", pull: "Updated: 3 commits" },
    });
    const { user } = renderDock();
    await user.click(await screen.findByRole("button", { name: "Fetch" }));
    expect(await screen.findByText("Fetched")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Update Project" }));
    expect(await screen.findByText("Updated: 3 commits")).toBeInTheDocument();
    expect(calls.fetch).toEqual([{ root: ROOT }]);
    expect(calls.pull).toEqual([{ root: ROOT }]);
  });

  test("Update Project explains why it is blocked without an upstream", async () => {
    const calls = installHandlers({ status: statusRoot({ upstream: null }) });
    const { user } = renderDock();
    await openPopup(user);
    const update = within(listbox()).getByRole("option", {
      name: "Update Project",
    });
    expect(update).toHaveAttribute("aria-disabled", "true");
    expect(update).toHaveTextContent("main has no upstream branch");
    expect(
      screen.getByRole("button", { name: "Update Project" }),
    ).toBeDisabled();
    expect(calls.pull).toBeUndefined();
  });

  test("auth_failed renders a friendly message with raw output", async () => {
    installHandlers({
      errors: {
        fetch: {
          // The engine now reports auth failures as 502; handling is by code.
          status: 502,
          code: "auth_failed",
          error: "fatal: Authentication failed for 'https://example.com/repo'",
        },
      },
    });
    const { user } = renderDock();
    await user.click(await screen.findByRole("button", { name: "Fetch" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "Authentication failed — configure a credential helper or SSH agent for this remote.",
    );
    fireEvent.click(within(alert).getByText("Git output"));
    expect(alert).toHaveTextContent("fatal: Authentication failed");
  });
});

describe("commit and push", () => {
  const messageBox = () =>
    screen.getByRole("textbox", { name: "Commit message" });

  test("commits, refreshes status, then opens the push dialog", async () => {
    const logCalls: URLSearchParams[] = [];
    const calls = installHandlers({
      logCalls,
      status: statusRoot({ ahead: 1 }),
    });
    server.use(
      http.get("*/v1/git/status", () =>
        HttpResponse.json({
          roots: [statusRoot({ ahead: calls.commit ? 2 : 1 })],
        }),
      ),
    );
    const { user } = renderDock();
    await screen.findByRole("treeitem", { name: "src/app.ts" });
    await user.type(messageBox(), "Ship it");
    await user.click(screen.getByRole("button", { name: "Commit and Push…" }));
    const dialog = await screen.findByRole("dialog", { name: "Push Commits" });
    expect(calls.commit).toEqual([
      { root: ROOT, message: "Ship it", paths: ["src/app.ts"] },
    ]);
    // The outgoing list uses the post-commit ahead count, not the stale one.
    const list = await within(dialog).findByRole("list", {
      name: "Outgoing commits",
    });
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
    expect(
      logCalls.some(
        (params) =>
          params.get("scope") === "head" && params.get("limit") === "1",
      ),
    ).toBe(false);
  });

  test("a failed commit does not open the push dialog", async () => {
    const calls = installHandlers({
      errors: {
        commit: {
          status: 422,
          code: "hook_failed",
          error: "pre-commit failed",
        },
      },
    });
    const { user } = renderDock();
    await screen.findByRole("treeitem", { name: "src/app.ts" });
    await user.type(messageBox(), "Ship it");
    await user.click(screen.getByRole("button", { name: "Commit and Push…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A Git hook rejected the commit.",
    );
    expect(calls.commit).toHaveLength(1);
    expect(
      screen.queryByRole("dialog", { name: "Push Commits" }),
    ).not.toBeInTheDocument();
  });
});

describe("git log menus", () => {
  test("branch context menu checks out and renames", async () => {
    const calls = installHandlers();
    const { user } = renderPanel();
    const tree = await screen.findByRole("tree", { name: "Branches" });
    const row = await within(tree).findByRole("treeitem", {
      name: "feature/login",
    });
    await user.pointer({ target: row, keys: "[MouseRight]" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Checkout" }));
    await waitFor(() =>
      expect(calls.checkout).toEqual([
        { root: ROOT, name: "feature/login", kind: "local" },
      ]),
    );

    await user.pointer({ target: row, keys: "[MouseRight]" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    const dialog = await screen.findByRole("dialog", { name: "Rename Branch" });
    const input = within(dialog).getByRole("textbox", {
      name: "New branch name",
    });
    await user.clear(input);
    await user.type(input, "feature/sign-in{Enter}");
    await waitFor(() =>
      expect(calls.rename).toEqual([
        { root: ROOT, old_name: "feature/login", new_name: "feature/sign-in" },
      ]),
    );
  });

  test("tag context menu only offers checkout with kind tag", async () => {
    const calls = installHandlers();
    const { user } = renderPanel();
    const tree = await screen.findByRole("tree", { name: "Branches" });
    await user.click(within(tree).getByRole("treeitem", { name: /^Tags/ }));
    const tag = await within(tree).findByRole("treeitem", { name: "v1.0" });
    await user.pointer({ target: tag, keys: "[MouseRight]" });
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual(["Checkout"]);
    fireEvent.click(items[0]);
    await waitFor(() =>
      expect(calls.checkout).toEqual([
        { root: ROOT, name: "v1.0", kind: "tag" },
      ]),
    );
  });

  test("commit context menu checks out a revision and branches from it", async () => {
    const calls = installHandlers();
    const { user } = renderPanel();
    const list = await screen.findByRole("listbox", { name: "Commits" });
    const row = await within(list).findByRole("option", {
      name: /Wire auth API/,
    });
    await user.pointer({ target: row, keys: "[MouseRight]" });
    expect(
      await screen.findByRole("menuitem", { name: "Copy Revision Number" }),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Checkout Revision" }),
    );
    await waitFor(() =>
      expect(calls.checkout).toEqual([
        { root: ROOT, name: COMMITS[1].oid, kind: "revision" },
      ]),
    );

    await user.pointer({ target: row, keys: "[MouseRight]" });
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "New Branch…" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Create New Branch",
    });
    await user.type(
      within(dialog).getByRole("textbox", { name: "New branch name" }),
      "hotfix",
    );
    await user.click(within(dialog).getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(calls.create).toEqual([
        {
          root: ROOT,
          name: "hotfix",
          checkout: true,
          start_point: COMMITS[1].oid,
        },
      ]),
    );
  });
});
