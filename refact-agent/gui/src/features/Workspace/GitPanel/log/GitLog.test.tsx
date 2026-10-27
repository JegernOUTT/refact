import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { render } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import type {
  GitBranchesRoot,
  GitCommitDetails,
  GitCommitLogEntry,
} from "../../../../services/refact/gitRead";
import { GitPanel } from "../GitPanel";
import { gitDiffDescriptorFromSurfaceKey } from "../../surfaceKey";
import { LOG_PAGE_SIZE } from "./useCommitLog";

const ROOT = "/repo";
const DAY_MS = 86_400_000;

function commit(
  oid: string,
  parents: string[] | undefined,
  extra: Partial<GitCommitLogEntry> = {},
): GitCommitLogEntry {
  return {
    oid: oid.padEnd(40, "0"),
    short_oid: oid.slice(0, 7),
    time_ms: Date.now() - DAY_MS,
    author_name: "Ada",
    author_email: "ada@example.com",
    message_first_line: `Commit ${oid}`,
    message: `Commit ${oid}\n\nBody of ${oid}`,
    ...(parents ? { parents: parents.map((p) => p.padEnd(40, "0")) } : {}),
    ...extra,
  };
}

const HISTORY: GitCommitLogEntry[] = [
  commit("merge01", ["main002", "feat002"], {
    refs: [
      { name: "HEAD", kind: "head" },
      { name: "main", kind: "local" },
      { name: "origin/main", kind: "remote" },
    ],
  }),
  commit("feat002", ["feat001"], {
    author_name: "Grace",
    refs: [{ name: "feature/log", kind: "local" }],
  }),
  commit("main002", ["main001"], { refs: [{ name: "v1.0", kind: "tag" }] }),
  commit("feat001", ["main001"]),
  commit("main001", []),
];

const BRANCHES: GitBranchesRoot = {
  root: ROOT,
  current: "main",
  branches: [
    {
      name: "main",
      is_head: true,
      upstream: "origin/main",
      tip_oid: HISTORY[0].oid,
      ahead: 2,
      behind: 1,
      upstream_gone: false,
    },
    { name: "feature/log", is_head: false, upstream: null },
  ],
  head_detached: false,
  head_oid: HISTORY[0].oid,
  remotes: ["origin"],
  remote_branches: [
    {
      name: "origin/main",
      remote: "origin",
      short_name: "main",
      tip_oid: HISTORY[0].oid,
    },
  ],
  tags: [{ name: "v1.0", target_oid: HISTORY[2].oid }],
};

function details(entry: GitCommitLogEntry): GitCommitDetails {
  return {
    root: ROOT,
    oid: entry.oid,
    short_oid: entry.short_oid,
    parents: entry.parents ?? [],
    author_name: entry.author_name,
    author_email: entry.author_email,
    author_time_ms: entry.time_ms,
    committer_name: "Linus",
    committer_email: "linus@example.com",
    committer_time_ms: entry.time_ms,
    message: entry.message,
    refs: entry.refs ?? [],
    branches: ["main", "origin/main"],
    files: [
      {
        path: "src/app.ts",
        old_path: null,
        status: "MODIFIED",
        additions: 3,
        deletions: 1,
        binary: false,
      },
      {
        path: "docs/guide.md",
        old_path: "docs/old.md",
        status: "RENAMED",
        additions: 0,
        deletions: 0,
        binary: false,
      },
    ],
    files_truncated: true,
  };
}

type Options = {
  logCalls?: URLSearchParams[];
  commitCalls?: string[];
  log?: (params: URLSearchParams) => GitCommitLogEntry[];
  branches?: GitBranchesRoot;
};

function installHandlers(options: Options = {}) {
  server.use(
    http.get("*/v1/git/status", () =>
      HttpResponse.json({
        roots: [
          {
            root: ROOT,
            branch: "main",
            head_detached: false,
            ahead: 0,
            behind: 0,
            staged: [],
            unstaged: [],
            untracked_included: true,
          },
        ],
      }),
    ),
    http.get("*/v1/git/branches", () =>
      HttpResponse.json({ roots: [options.branches ?? BRANCHES] }),
    ),
    http.get("*/v1/git/log", ({ request }) => {
      const params = new URL(request.url).searchParams;
      options.logCalls?.push(params);
      const commits = options.log ? options.log(params) : HISTORY;
      return HttpResponse.json({
        roots: [
          {
            root: ROOT,
            commits,
            head_oid: HISTORY[0].oid,
            scan_truncated: false,
          },
        ],
      });
    }),
    http.get("*/v1/git/commit", ({ request }) => {
      const oid = new URL(request.url).searchParams.get("oid") ?? "";
      options.commitCalls?.push(oid);
      const entry =
        HISTORY.find((item) => item.oid === oid) ?? commit(oid, ["parent0"]);
      return HttpResponse.json(details(entry));
    }),
    http.get("*/v1/worktrees", () =>
      HttpResponse.json({
        project_hash: "project",
        source_workspace_root: ROOT,
        source_current_branch: "main",
        source_branches: ["main"],
        worktrees: [],
      }),
    ),
  );
}

function renderPanel() {
  return render(<GitPanel />, {
    preloadedState: {
      config: {
        host: "web",
        lspPort: 8001,
        engineServed: true,
        themeProps: { appearance: "dark" },
      },
      current_project: { name: "repo", workspaceRoots: [ROOT] },
    },
  });
}

const commitList = () => screen.getByRole("listbox", { name: "Commits" });
const rowFor = (subject: string) =>
  within(commitList()).getByRole("option", { name: new RegExp(subject) });

beforeEach(() => {
  installHandlers();
});

describe("Git log tool window", () => {
  test("renders commits with ref pills, graph and tracking hints", async () => {
    const logCalls: URLSearchParams[] = [];
    installHandlers({ logCalls });
    renderPanel();

    const list = await screen.findByRole("listbox", { name: "Commits" });
    const options = await within(list).findAllByRole("option");
    expect(options).toHaveLength(HISTORY.length);
    expect(within(options[0]).getByText("HEAD")).toHaveAttribute(
      "data-kind",
      "head",
    );
    expect(within(options[0]).getByText("origin/main")).toHaveAttribute(
      "data-kind",
      "remote",
    );
    expect(within(options[2]).getByText("v1.0")).toHaveAttribute(
      "data-kind",
      "tag",
    );
    expect(list).toHaveAttribute("data-graph", "true");
    expect(options[0].querySelector("svg")).not.toBeNull();
    expect(logCalls[0]?.get("scope")).toBe("all");
    expect(logCalls[0]?.get("limit")).toBe(String(LOG_PAGE_SIZE));

    const tree = screen.getByRole("tree", { name: "Branches" });
    expect(within(tree).getByText("↑2 ↓1")).toBeInTheDocument();
    expect(
      within(tree).getByRole("treeitem", { name: /^Tags/ }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(within(tree).queryByText("v1.0")).not.toBeInTheDocument();
  });

  test("branch click filters the log by ref and HEAD clears it", async () => {
    const logCalls: URLSearchParams[] = [];
    installHandlers({ logCalls });
    const { user } = renderPanel();

    const tree = await screen.findByRole("tree", { name: "Branches" });
    await user.click(
      await within(tree).findByRole("treeitem", { name: "feature/log" }),
    );
    await waitFor(() =>
      expect(logCalls.at(-1)?.get("ref")).toBe("feature/log"),
    );
    expect(logCalls.at(-1)?.get("scope")).toBeNull();
    const branchFilter = screen.getByRole("combobox", {
      name: "Branch filter",
    });
    expect(branchFilter).toHaveTextContent("feature/log");
    expect(
      within(tree).getByRole("treeitem", { name: "feature/log" }),
    ).toHaveAttribute("aria-selected", "true");

    await user.click(within(tree).getByRole("treeitem", { name: /^HEAD/ }));
    await waitFor(() => expect(branchFilter).toHaveTextContent("All"));
    expect(
      within(tree).getByRole("treeitem", { name: "feature/log" }),
    ).toHaveAttribute("aria-selected", "false");
  });

  test("text, user and path filters send params and hide the graph", async () => {
    const logCalls: URLSearchParams[] = [];
    installHandlers({ logCalls });
    const { user } = renderPanel();
    await screen.findAllByRole("option");

    await user.type(
      screen.getByRole("searchbox", { name: "Filter by text or hash" }),
      "fix",
    );
    await waitFor(() => expect(logCalls.at(-1)?.get("text")).toBe("fix"));
    await waitFor(() => expect(commitList()).not.toHaveAttribute("data-graph"));

    await user.type(
      screen.getByRole("combobox", { name: "Filter by user" }),
      "Gr",
    );
    await waitFor(() => expect(logCalls.at(-1)?.get("author")).toBe("Gr"));

    await user.type(
      screen.getByRole("searchbox", { name: "Filter by path" }),
      "src/app.ts",
    );
    await waitFor(() =>
      expect(logCalls.at(-1)?.get("path")).toBe("src/app.ts"),
    );
    expect(logCalls.at(-1)?.get("text")).toBe("fix");
  });

  test("selecting a commit loads details and keyboard moves selection", async () => {
    const commitCalls: string[] = [];
    installHandlers({ commitCalls });
    const { user } = renderPanel();

    await waitFor(() => expect(commitCalls).toContain(HISTORY[0].oid));
    expect(
      await screen.findByRole("heading", { name: "Commit merge01" }),
    ).toBeInTheDocument();

    await user.click(rowFor("Commit feat002"));
    await waitFor(() => expect(commitCalls).toContain(HISTORY[1].oid));
    expect(rowFor("Commit feat002")).toHaveAttribute("aria-selected", "true");

    fireEvent.keyDown(commitList(), { key: "ArrowDown" });
    expect(rowFor("Commit main002")).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(commitList(), { key: "End" });
    expect(rowFor("Commit main001")).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(commitList(), { key: "Home" });
    expect(rowFor("Commit merge01")).toHaveAttribute("aria-selected", "true");

    const region = screen.getByRole("region", { name: "Commit details" });
    expect(
      await within(region).findByText("In 2 branches: main, origin/main"),
    ).toBeInTheDocument();
    expect(within(region).getByText(/Committed by Linus/)).toBeInTheDocument();
    expect(
      within(region).getByText("Too many files; the list is truncated."),
    ).toBeInTheDocument();
    const files = within(region).getByRole("tree", { name: "Changed files" });
    expect(
      within(files).getByRole("treeitem", { name: "src/app.ts" }),
    ).toHaveTextContent("+3−1");
    expect(within(files).queryByRole("checkbox")).not.toBeInTheDocument();

    await user.click(within(region).getByRole("button", { name: "feat002" }));
    expect(rowFor("Commit feat002")).toHaveAttribute("aria-selected", "true");
  });

  test("double-click and Enter on a file open the commit diff tab", async () => {
    const { store, user } = renderPanel();
    const region = await screen.findByRole("region", {
      name: "Commit details",
    });
    const files = await within(region).findByRole("tree", {
      name: "Changed files",
    });

    await user.dblClick(
      within(files).getByRole("treeitem", { name: "src/app.ts" }),
    );
    expect(
      gitDiffDescriptorFromSurfaceKey(
        store.getState().workspace.activeTabId ?? "",
      ),
    ).toMatchObject({
      root: ROOT,
      path: "src/app.ts",
      left: { rev: HISTORY[0].parents?.[0], path: "src/app.ts" },
      right: { rev: HISTORY[0].oid, path: "src/app.ts" },
      kind: "commit",
    });

    await user.click(
      within(files).getByRole("treeitem", { name: "docs/guide.md" }),
    );
    await user.keyboard("{Enter}");
    expect(
      gitDiffDescriptorFromSurfaceKey(
        store.getState().workspace.activeTabId ?? "",
      ),
    ).toMatchObject({
      path: "docs/guide.md",
      left: { rev: HISTORY[0].parents?.[0], path: "docs/old.md" },
      right: { rev: HISTORY[0].oid, path: "docs/guide.md" },
    });
  });

  test("loads the next page with skip from the load more row", async () => {
    const logCalls: URLSearchParams[] = [];
    const page = (skip: number) =>
      Array.from({ length: skip === 0 ? LOG_PAGE_SIZE : 3 }, (_, index) =>
        commit(`c${String(skip + index).padStart(6, "0")}`, undefined),
      );
    installHandlers({
      logCalls,
      log: (params) => page(Number(params.get("skip"))),
    });
    const { user } = renderPanel();

    await user.click(await screen.findByRole("button", { name: "Load more" }));
    await waitFor(() =>
      expect(within(commitList()).getAllByRole("option")).toHaveLength(
        LOG_PAGE_SIZE + 3,
      ),
    );
    expect(logCalls.at(-1)?.get("skip")).toBe(String(LOG_PAGE_SIZE));
    expect(
      screen.queryByRole("button", { name: "Load more" }),
    ).not.toBeInTheDocument();
  });

  test("degrades without parents, refs or extended branch fields", async () => {
    installHandlers({
      log: () => HISTORY.map(({ parents: _p, refs: _r, ...rest }) => rest),
      branches: {
        root: ROOT,
        current: "main",
        branches: [{ name: "main", is_head: true, upstream: null }],
      },
    });
    renderPanel();

    const options = await screen.findAllByRole("option");
    expect(options).toHaveLength(HISTORY.length);
    expect(commitList()).not.toHaveAttribute("data-graph");
    expect(options[0].querySelector("svg")).toBeNull();
    const tree = screen.getByRole("tree", { name: "Branches" });
    expect(within(tree).getByRole("treeitem", { name: "main" })).toBeVisible();
    expect(
      within(tree).queryByRole("treeitem", { name: "Remote" }),
    ).not.toBeInTheDocument();
  });

  test("shows an empty state for unborn repositories and inline errors", async () => {
    installHandlers({ log: () => [] });
    renderPanel();
    expect(await screen.findByText("No commits yet")).toBeInTheDocument();

    server.use(
      http.get("*/v1/git/log", () =>
        HttpResponse.json(
          { code: "git_error", error: "bad object" },
          { status: 500 },
        ),
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh log" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Git reported an error. bad object",
    );
  });

  test("switches to the Worktrees tab", async () => {
    const { user } = renderPanel();
    await user.click(await screen.findByRole("tab", { name: "Worktrees" }));
    expect(
      await screen.findByText("No registered worktrees."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("listbox", { name: "Commits" })).toBeNull();
  });
});
