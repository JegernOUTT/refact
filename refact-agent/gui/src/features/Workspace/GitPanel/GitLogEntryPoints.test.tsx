import { beforeEach, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import {
  fireEvent,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { Provider } from "react-redux";

import { setUpStore } from "../../../app/store";
import { render } from "../../../utils/test-utils";
import { updateConfig } from "../../Config/configSlice";
import { useWorkspaceShortcuts } from "../useWorkspaceShortcuts";
import { server } from "../../../utils/mockServer";
import type { GitStatusRoot } from "../../../services/refact/gitRead";
import { GitDock } from "./GitDock";
import { GitPanel } from "./GitPanel";
import { selectGitLogLayout, selectGitLogRequest } from "./gitPanelSlice";

const ROOT = "/repo";

const STATUS: GitStatusRoot = {
  root: ROOT,
  branch: "main",
  head_detached: false,
  ahead: 0,
  behind: 0,
  staged: [],
  unstaged: [],
  untracked_included: true,
  head_oid: "abc1234567890",
  upstream: "origin/main",
  repo_state: "clean",
  truncated: false,
  changes: [
    {
      relative_path: "src/app.ts",
      absolute_path: "/repo/src/app.ts",
      old_path: null,
      status: "MODIFIED",
      staged: false,
      unstaged: true,
    },
  ],
  untracked: [{ relative_path: "notes.txt", absolute_path: "/repo/notes.txt" }],
  conflicted: [],
};

let logCalls: URLSearchParams[] = [];

function installHandlers() {
  logCalls = [];
  server.use(
    http.get("*/v1/git/status", () => HttpResponse.json({ roots: [STATUS] })),
    http.get("*/v1/git/branches", () =>
      HttpResponse.json({
        roots: [
          {
            root: ROOT,
            current: "main",
            branches: [
              { name: "main", is_head: true, upstream: "origin/main" },
              { name: "feature/login", is_head: false, upstream: null },
            ],
          },
        ],
      }),
    ),
    http.get("*/v1/git/log", ({ request }) => {
      logCalls.push(new URL(request.url).searchParams);
      return HttpResponse.json({
        roots: [
          {
            root: ROOT,
            commits: [
              {
                oid: "abc123456789",
                short_oid: "abc1234",
                time_ms: 1,
                author_name: "Ada",
                author_email: "ada@example.com",
                message_first_line: "Initial commit",
                message: "Initial commit\n",
              },
            ],
          },
        ],
      });
    }),
    http.get("*/v1/git/commit", () =>
      HttpResponse.json({
        root: ROOT,
        oid: "abc123456789",
        short_oid: "abc1234",
        parents: [],
        author_name: "Ada",
        author_email: "ada@example.com",
        author_time_ms: 1,
        committer_name: "Ada",
        committer_email: "ada@example.com",
        committer_time_ms: 1,
        message: "Initial commit\n",
        refs: [],
        branches: ["main"],
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
  );
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

function renderBoth() {
  return render(
    <>
      <GitDock />
      <GitPanel />
    </>,
    { preloadedState: PRELOADED },
  );
}

beforeEach(installHandlers);

describe("Git Log entry points", () => {
  test("toolbar Git Log button opens git:main on the Log tab", async () => {
    const { store, user } = render(<GitDock />, {
      preloadedState: {
        ...PRELOADED,
        gitPanel: {
          contexts: {},
          logLayout: {
            tab: "worktrees",
            branchesWidth: 200,
            detailsWidth: 300,
          },
        },
      },
    });
    const toolbar = await screen.findByRole("toolbar", {
      name: "Commit tool window actions",
    });
    await user.click(within(toolbar).getByRole("button", { name: "Git Log" }));

    const state = store.getState();
    expect(state.workspace.activeTabId).toBe("git:main");
    expect(selectGitLogLayout(state)).toEqual({
      tab: "log",
      branchesWidth: 200,
      detailsWidth: 300,
    });
    expect(selectGitLogRequest(state, null)).toBeNull();
  });

  test("Show History on a file filters the log by its path", async () => {
    const { store, user } = renderBoth();
    const row = await screen.findByRole("treeitem", { name: "src/app.ts" });
    await user.pointer({ target: row, keys: "[MouseRight]" });
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Show History" }),
    );

    await waitFor(() =>
      expect(logCalls.some((p) => p.get("path") === "src/app.ts")).toBe(true),
    );
    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(
      screen.getByRole("searchbox", { name: "Filter by path" }),
    ).toHaveValue("src/app.ts");
    // The one-shot request is consumed once applied.
    await waitFor(() =>
      expect(selectGitLogRequest(store.getState(), null)).toBeNull(),
    );
  });

  test("Show History is offered on unversioned files but not group roots", async () => {
    const { user } = renderBoth();
    const untracked = await screen.findByRole("treeitem", {
      name: "notes.txt",
    });
    await user.pointer({ target: untracked, keys: "[MouseRight]" });
    expect(
      await screen.findByRole("menuitem", { name: "Show History" }),
    ).toBeInTheDocument();
    await user.keyboard("{Escape}");

    const group = await screen.findByRole("treeitem", { name: "Changes" });
    await user.pointer({ target: group, keys: "[MouseRight]" });
    await screen.findByRole("menuitem", { name: /Collapse|Expand/ });
    expect(
      screen.queryByRole("menuitem", { name: "Show History" }),
    ).not.toBeInTheDocument();
  });

  test("branch popup Show Log filters the log by the branch ref", async () => {
    const { store, user } = renderBoth();
    await user.click(
      await screen.findByRole("button", { name: "Git branch: main" }),
    );
    const listbox = await screen.findByRole("listbox", {
      name: "Branches and actions",
    });
    const branch = await within(listbox).findByRole("option", {
      name: "feature/login",
    });
    await user.click(within(branch).getByTitle("Actions for feature/login"));
    await user.click(within(listbox).getByRole("option", { name: "Show Log" }));

    await waitFor(() =>
      expect(logCalls.some((p) => p.get("ref") === "feature/login")).toBe(true),
    );
    expect(store.getState().workspace.activeTabId).toBe("git:main");
  });

  test("Alt+9 opens the Git Log", () => {
    const store = setUpStore();
    store.dispatch(updateConfig({ host: "web" }));
    const wrapper = ({ children }: { children: ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );
    renderHook(() => useWorkspaceShortcuts(), { wrapper });

    fireEvent.keyDown(window, { key: "9", code: "Digit9", altKey: true });

    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(selectGitLogLayout(store.getState()).tab).toBe("log");
  });

  test("Alt+9 is ignored while typing in text fields", () => {
    const store = setUpStore();
    store.dispatch(updateConfig({ host: "web" }));
    const wrapper = ({ children }: { children: ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );
    renderHook(() => useWorkspaceShortcuts(), { wrapper });
    const before = store.getState().workspace.activeTabId;

    for (const tag of ["input", "textarea"] as const) {
      const field = document.createElement(tag);
      document.body.appendChild(field);
      field.focus();
      const notPrevented = fireEvent.keyDown(field, {
        key: "9",
        code: "Digit9",
        altKey: true,
      });
      expect(notPrevented).toBe(true);
      field.remove();
    }

    expect(store.getState().workspace.activeTabId).toBe(before);
    expect(store.getState().workspace.activeTabId).not.toBe("git:main");
  });
});
