import { readFileSync } from "node:fs";

import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import { filesApi, type FilesTreeEntry } from "../../../services/refact/files";
import { applyChatEvent } from "../../Chat/Thread";
import { FilesPanel } from "./FilesPanel";
import { FileViewer } from "./FileViewer";
import {
  applyLiveFileUpdate,
  markLiveFileUpdateAuthoritative,
  openFileInFilesPanel,
} from "./filesPanelSlice";
import { createChatWithId } from "../../Chat/Thread/actions";
import {
  openTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
} from "../workspaceSlice";
import type { WorktreeMeta } from "../../../services/refact/worktrees";

const rootPath = "/workspace";
const sourcePath = `${rootPath}/src`;
const filePath = `${sourcePath}/main.ts`;

const worktree = (id: string): WorktreeMeta => ({
  id,
  kind: "chat",
  root: `/worktrees/${id}`,
  source_workspace_root: rootPath,
  repo_root: rootPath,
  enforce: false,
});

const treeResponse = (path: string, entries: unknown[]) => ({
  path,
  entries,
  truncated: false,
});

const rootHandler = () =>
  http.get("*/v1/files/tree", ({ request }) => {
    const path = new URL(request.url).searchParams.get("path") ?? "";
    if (path === "") {
      return HttpResponse.json(
        treeResponse("", [
          { name: "workspace", path: rootPath, kind: "dir", size: null },
        ]),
      );
    }
    if (path === rootPath) {
      return HttpResponse.json(
        treeResponse(rootPath, [
          { name: "src", path: sourcePath, kind: "dir", size: null },
          {
            name: "README.md",
            path: `${rootPath}/README.md`,
            kind: "file",
            size: 4,
          },
        ]),
      );
    }
    if (path === sourcePath) {
      return HttpResponse.json(
        treeResponse(sourcePath, [
          { name: "main.ts", path: filePath, kind: "file", size: 22 },
        ]),
      );
    }
    return HttpResponse.json(treeResponse(path, []));
  });

const readResponse = (overrides: Record<string, unknown> = {}) => ({
  path: filePath,
  content: "const first = 1;\nconst second = 2;\n",
  language: "typescript",
  size: 34,
  truncated: false,
  line_start: null,
  line_end: null,
  mtime_ms: 1,
  ...overrides,
});

describe("FilesPanel", () => {
  beforeEach(() => {
    setProjectStorageNamespace("files-panel-test");
    vi.spyOn(Element.prototype, "scrollIntoView").mockImplementation(
      () => undefined,
    );
  });

  afterEach(() => {
    localStorage.clear();
    setProjectStorageNamespace(undefined);
    vi.restoreAllMocks();
  });

  it("renders one file icon with an empty chevron slot", async () => {
    server.use(rootHandler());
    const { user } = render(<FilesPanel />);
    const workspace = await screen.findByRole("treeitem", {
      name: /workspace/i,
    });

    expect(workspace.querySelectorAll("svg")).toHaveLength(2);
    expect(
      within(workspace).getByTestId("tree-chevron-slot").querySelector("svg"),
    ).not.toBeNull();

    await user.click(workspace);
    const file = await screen.findByRole("treeitem", { name: /README\.md/i });

    expect(file.querySelectorAll("svg")).toHaveLength(1);
    expect(
      within(file).getByTestId("tree-chevron-slot").querySelector("svg"),
    ).toBeNull();
    expect(file.querySelector(".lucide-file-text")).not.toBeNull();
  });

  it("hides ignored entries by default and reveals them muted", async () => {
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        if (path === "") {
          return HttpResponse.json(
            treeResponse("", [
              { name: "workspace", path: rootPath, kind: "dir", size: null },
            ]),
          );
        }
        return HttpResponse.json(
          treeResponse(rootPath, [
            {
              name: "main.ts",
              path: `${rootPath}/main.ts`,
              kind: "file",
              size: 10,
              ignored: false,
            },
            {
              name: "debug.log",
              path: `${rootPath}/debug.log`,
              kind: "file",
              size: 20,
              ignored: true,
            },
          ]),
        );
      }),
    );
    const { user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );

    expect(
      await screen.findByRole("treeitem", { name: /main\.ts/i }),
    ).toBeVisible();
    expect(screen.queryByRole("treeitem", { name: /debug\.log/i })).toBeNull();

    await user.click(screen.getByRole("switch", { name: "Show ignored" }));

    expect(
      await screen.findByRole("treeitem", { name: /debug\.log/i }),
    ).toHaveAttribute("data-ignored", "true");
  });

  it("persists the ignored toggle per project", async () => {
    server.use(rootHandler());
    setProjectStorageNamespace("project-a");
    const first = render(<FilesPanel />);
    const firstToggle = screen.getByRole("switch", { name: "Show ignored" });
    await first.user.click(firstToggle);
    expect(firstToggle).toBeChecked();
    first.unmount();

    setProjectStorageNamespace("project-b");
    const second = render(<FilesPanel />);
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: "Show ignored" }),
      ).not.toBeChecked(),
    );
    second.unmount();

    setProjectStorageNamespace("project-a");
    render(<FilesPanel />);
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: "Show ignored" }),
      ).toBeChecked(),
    );
  });

  it("replaces refetched children while keeping the directory expanded", async () => {
    let sourceRequests = 0;
    let sourceEntries: FilesTreeEntry[] = [
      { name: "main.ts", path: filePath, kind: "file", size: 22 },
    ];
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        if (path === sourcePath) sourceRequests += 1;
        if (path === "") {
          return HttpResponse.json(
            treeResponse("", [
              { name: "workspace", path: rootPath, kind: "dir", size: null },
            ]),
          );
        }
        if (path === rootPath) {
          return HttpResponse.json(
            treeResponse(rootPath, [
              { name: "src", path: sourcePath, kind: "dir", size: null },
            ]),
          );
        }
        return HttpResponse.json(treeResponse(sourcePath, sourceEntries));
      }),
    );

    const { store, user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );
    const source = await screen.findByRole("treeitem", { name: /src/i });
    await user.click(source);
    expect(
      await screen.findByRole("treeitem", { name: /main\.ts/i }),
    ).toBeVisible();
    expect(sourceRequests).toBe(1);

    sourceEntries = [
      {
        name: "replacement.ts",
        path: `${sourcePath}/replacement.ts`,
        kind: "file",
        size: 12,
      },
    ];
    store.dispatch(
      filesApi.util.invalidateTags([{ type: "Tree", id: sourcePath }]),
    );

    await waitFor(() => {
      expect(screen.queryByRole("treeitem", { name: /main\.ts/i })).toBeNull();
      expect(
        screen.getByRole("treeitem", { name: /replacement\.ts/i }),
      ).toBeVisible();
    });
    expect(source).toHaveAttribute("aria-expanded", "true");
    expect(sourceRequests).toBe(2);
  });

  it("refreshes expanded parents after live create delete and rename", async () => {
    const testsPath = `${rootPath}/tests`;
    const createdPath = `${sourcePath}/created.ts`;
    const renamedFrom = `${sourcePath}/old.ts`;
    const renamedTo = `${testsPath}/new.ts`;
    let sourceEntries: FilesTreeEntry[] = [
      { name: "old.ts", path: renamedFrom, kind: "file", size: 8 },
    ];
    let testEntries: FilesTreeEntry[] = [];
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        if (path === "") {
          return HttpResponse.json(
            treeResponse("", [
              { name: "workspace", path: rootPath, kind: "dir", size: null },
            ]),
          );
        }
        if (path === rootPath) {
          return HttpResponse.json(
            treeResponse(rootPath, [
              { name: "src", path: sourcePath, kind: "dir", size: null },
              { name: "tests", path: testsPath, kind: "dir", size: null },
            ]),
          );
        }
        if (path === sourcePath) {
          return HttpResponse.json(treeResponse(sourcePath, sourceEntries));
        }
        if (path === testsPath) {
          return HttpResponse.json(treeResponse(testsPath, testEntries));
        }
        return HttpResponse.json(treeResponse(path, []));
      }),
      http.get("*/v1/files/read", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        return HttpResponse.json(readResponse({ path, content: "content\n" }));
      }),
    );

    const view = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    const source = await screen.findByRole("treeitem", { name: /src/i });
    const tests = await screen.findByRole("treeitem", { name: /tests/i });
    await view.user.click(source);
    await view.user.click(tests);
    expect(
      await screen.findByRole("treeitem", { name: /old\.ts/i }),
    ).toBeVisible();

    const chatId = view.store.getState().chat.current_thread_id;
    const dispatchDiff = (
      seq: string,
      chunk: {
        file_name: string;
        file_action: string;
        file_name_rename?: string;
      },
    ) =>
      view.store.dispatch(
        applyChatEvent({
          chat_id: chatId,
          seq,
          type: "message_added",
          index: Number(seq),
          message: {
            role: "diff",
            tool_call_id: `edit-${seq}`,
            content: [
              {
                ...chunk,
                line1: 1,
                line2: 1,
                lines_remove: "",
                lines_add: "content\n",
              },
            ],
          },
        }),
      );

    sourceEntries = [
      ...sourceEntries,
      { name: "created.ts", path: createdPath, kind: "file", size: 8 },
    ];
    dispatchDiff("1", { file_name: createdPath, file_action: "add" });
    expect(
      await screen.findByRole("treeitem", { name: /created\.ts/i }),
    ).toBeVisible();

    sourceEntries = sourceEntries.filter((entry) => entry.path !== createdPath);
    dispatchDiff("2", { file_name: createdPath, file_action: "remove" });
    await waitFor(() =>
      expect(
        screen.queryByRole("treeitem", { name: /created\.ts/i }),
      ).toBeNull(),
    );

    sourceEntries = [];
    testEntries = [{ name: "new.ts", path: renamedTo, kind: "file", size: 8 }];
    dispatchDiff("3", {
      file_name: renamedFrom,
      file_action: "rename",
      file_name_rename: renamedTo,
    });
    await waitFor(() => {
      expect(screen.queryByRole("treeitem", { name: /old\.ts/i })).toBeNull();
      expect(screen.getByRole("treeitem", { name: /new\.ts/i })).toBeVisible();
    });
    expect(source).toHaveAttribute("aria-expanded", "true");
    expect(tests).toHaveAttribute("aria-expanded", "true");
  });

  it("roots and re-roots the tree with the focused chat worktree", async () => {
    const requestedPaths: string[] = [];
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        requestedPaths.push(path);
        const name = path.split("/").pop() ?? path;
        return HttpResponse.json(
          treeResponse(path, [{ name, path, kind: "dir", size: null }]),
        );
      }),
    );

    const view = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    view.store.dispatch(
      createChatWithId({ id: "chat-a", worktree: worktree("a") }),
    );
    view.store.dispatch(
      createChatWithId({ id: "chat-b", worktree: worktree("b") }),
    );
    view.store.dispatch(openTab("chat:chat-a"));

    expect(await screen.findByRole("treeitem", { name: "a" })).toBeVisible();
    await waitFor(() => expect(requestedPaths).toContain("/worktrees/a"));

    view.store.dispatch(openTab("chat:chat-b"));
    view.store.dispatch(setActiveTab("chat:chat-b"));

    expect(await screen.findByRole("treeitem", { name: "b" })).toBeVisible();
    await waitFor(() => expect(requestedPaths).toContain("/worktrees/b"));
    expect(screen.queryByRole("treeitem", { name: "a" })).toBeNull();
  });

  it("renders file content and highlights the requested line", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    view.store.dispatch(openFileInFilesPanel({ path: filePath, line: 2 }));

    expect(await screen.findByText("const second = 2;")).toBeVisible();
    expect(document.querySelector('[data-target-line="true"]')).toHaveAttribute(
      "data-line-number",
      "2",
    );
    expect(
      screen.getByRole("navigation", { name: "File path" }),
    ).toHaveTextContent("workspace/src/main.ts");
  });

  it("omits breadcrumbs above the enclosing workspace root", async () => {
    const deepRoot = "/w/a/b/engine";
    const deepFile = `${deepRoot}/src/x.rs`;
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ path: deepFile })),
      ),
    );
    const view = render(<FileViewer path={deepFile} />, {
      preloadedState: {
        current_project: {
          name: "engine",
          workspaceRoots: [deepRoot],
        },
      },
    });
    view.store.dispatch(setDockOpen(false));
    view.store.dispatch(setDockSection("git"));

    expect(await screen.findByRole("button", { name: "engine" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "src" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "x.rs" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "w" })).toBeNull();
    expect(screen.queryByRole("button", { name: "a" })).toBeNull();
    expect(screen.queryByRole("button", { name: "b" })).toBeNull();

    await view.user.click(screen.getByRole("button", { name: "engine" }));
    expect(view.store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
    expect(view.store.getState().filesPanel.expandedDirectories).toEqual([
      deepRoot,
    ]);
    expect(view.store.getState().filesPanel.selectedPath).toBe(deepRoot);
  });

  it("uses the enclosing root when multiple workspace roots are configured", async () => {
    const engineRoot = "/home/user/project/engine";
    const engineFile = `${engineRoot}/Cargo.toml`;
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ path: engineFile })),
      ),
    );
    render(<FileViewer path={engineFile} />, {
      preloadedState: {
        current_project: {
          name: "project",
          workspaceRoots: ["/home/user/other", engineRoot],
        },
      },
    });

    const breadcrumbs = await screen.findByRole("navigation", {
      name: "File path",
    });
    expect(breadcrumbs).toHaveTextContent("engine/Cargo.toml");
    expect(breadcrumbs).not.toHaveTextContent("home");
    expect(breadcrumbs).not.toHaveTextContent("user");
    expect(within(breadcrumbs).getAllByRole("button")).toHaveLength(2);
  });

  it("keeps file breadcrumbs relative to the focused chat worktree", async () => {
    const worktreeRoot = "/worktrees/a";
    const worktreeFile = `${worktreeRoot}/src/main.ts`;
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ path: worktreeFile })),
      ),
    );
    const view = render(<FileViewer path={worktreeFile} />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    view.store.dispatch(
      createChatWithId({ id: "chat-a", worktree: worktree("a") }),
    );
    view.store.dispatch(openTab("chat:chat-a"));

    const breadcrumbs = await screen.findByRole("navigation", {
      name: "File path",
    });
    await waitFor(() => expect(breadcrumbs).toHaveTextContent("a/src/main.ts"));
    expect(breadcrumbs).not.toHaveTextContent("worktrees");
  });

  it("shows an honest privacy-blocked state", async () => {
    server.use(
      rootHandler(),
      http.get(
        "*/v1/files/read",
        () => new HttpResponse(null, { status: 403 }),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("File blocked")).toBeVisible();
    expect(
      screen.getByText("This file is blocked by privacy rules."),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
  });

  it("shows the truncation banner returned by the backend", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ truncated: true })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("File truncated at 1 MiB")).toBeVisible();
  });

  it("renders reread content and keeps changed-line highlights", async () => {
    let readRequests = 0;
    let authoritative = false;
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => {
        readRequests += 1;
        return HttpResponse.json(
          readResponse({
            content: authoritative ? "new\nkeep\n" : "old\nkeep\n",
          }),
        );
      }),
    );
    const view = render(<FileViewer path={filePath} />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
        workspace: {
          tabs: [],
          activeTabId: null,
          groups: {},
        },
      },
    });
    view.store.dispatch(createChatWithId({ id: "chat-a" }));
    view.store.dispatch(openTab("chat:chat-a"));
    await screen.findByText("old");
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    authoritative = true;
    view.store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: filePath,
        update: {
          revision: "7",
          operation: "write",
          chunks: [
            {
              file_name: filePath,
              file_action: "edit",
              line1: 1,
              line2: 2,
              lines_remove: "old\n",
              lines_add: "new\n",
            },
          ],
        },
      }),
    );
    view.store.dispatch(
      markLiveFileUpdateAuthoritative({
        chatId: "chat-a",
        path: filePath,
        revision: "7",
      }),
    );

    expect(await screen.findByText("new")).toBeVisible();
    expect(screen.getByText("new").closest('[role="row"]')).toHaveAttribute(
      "data-live-change",
      "true",
    );
    expect(readRequests).toBeGreaterThan(1);
  });

  it("does not render stale content after delete or rename", async () => {
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ content: "stale\n" })),
      ),
    );
    const view = render(<FileViewer path={filePath} />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
        workspace: {
          tabs: [],
          activeTabId: null,
          groups: {},
        },
      },
    });
    view.store.dispatch(createChatWithId({ id: "chat-a" }));
    view.store.dispatch(openTab("chat:chat-a"));
    await screen.findByText("stale");
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    view.store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: filePath,
        update: { revision: "8", chunks: [], operation: "remove" },
      }),
    );
    expect(await screen.findByText("File deleted")).toBeVisible();
    expect(screen.queryByText("stale")).toBeNull();

    view.store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: filePath,
        update: {
          revision: "9",
          chunks: [],
          operation: "rename",
          renamedTo: "/workspace/src/renamed.ts",
        },
      }),
    );
    expect(await screen.findByText("File renamed")).toBeVisible();
    expect(
      screen.getByText("This file was renamed to /workspace/src/renamed.ts."),
    ).toBeVisible();
  });

  it("keeps the reduced-motion rule for live change highlights", () => {
    const css = readFileSync(
      "src/features/Workspace/FilesPanel/FilesPanel.module.css",
      "utf8",
    );
    expect(css).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*data-live-change[\s\S]*animation: none/u,
    );
  });

  it("identifies binary files without rendering an empty code view", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ binary: true, content: "" })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("Binary file")).toBeVisible();
    expect(screen.getByText(/cannot be previewed/)).toBeVisible();
  });

  it("keeps keyboard navigation in the tree and opens a file with Enter", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const { user, store } = render(<FilesPanel />);
    const tree = await screen.findByRole("tree", { name: "Workspace files" });
    const workspace = await screen.findByRole("treeitem", {
      name: /workspace/i,
    });
    await user.click(workspace);
    await screen.findByRole("treeitem", { name: /README\.md/i });

    tree.focus();
    fireEvent.keyDown(tree, { key: "ArrowDown" });
    fireEvent.keyDown(tree, { key: "ArrowDown" });
    fireEvent.keyDown(tree, { key: "Enter" });

    await waitFor(() =>
      expect(store.getState().workspace.activeTabId).toBe(
        "file:/workspace/README.md",
      ),
    );
    expect(
      within(tree).getByRole("treeitem", { name: /README\.md/i }),
    ).toHaveAttribute("aria-selected", "true");
  });
});
