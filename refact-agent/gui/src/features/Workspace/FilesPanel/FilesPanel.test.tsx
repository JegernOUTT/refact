import { readFileSync } from "node:fs";

import { EditorView } from "@codemirror/view";
import { act } from "react-dom/test-utils";
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
import { useAppSelector } from "../../../hooks";
import { setProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import { filesApi, type FilesTreeEntry } from "../../../services/refact/files";
import type { PrivacyPolicyResponse } from "../../../services/refact/privacy";
import { applyChatEvent } from "../../Chat/Thread";
import { setCurrentProjectInfo } from "../../Chat/currentProject";
import { FilesPanel } from "./FilesPanel";
import { FileViewer } from "./FileViewer";
import {
  PaneHeaderActions,
  PaneHeaderProvider,
  PaneSurfaceActivityProvider,
} from "../PaneHeaderContext";
import {
  applyLiveFileUpdate,
  enqueueEditPlayerSteps,
  markLiveFileUpdateAuthoritative,
  openFileInFilesPanel,
} from "./filesPanelSlice";
import { createChatWithId } from "../../Chat/Thread/actions";
import {
  openTab,
  closeTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
} from "../workspaceSlice";
import type { WorktreeMeta } from "../../../services/refact/worktrees";

const rootPath = "/workspace";
const sourcePath = `${rootPath}/src`;
const filePath = `${sourcePath}/main.ts`;

const privacyResponse: PrivacyPolicyResponse = {
  policy: {
    blocked: [],
    zones: [
      {
        name: "normal",
        patterns: ["**"],
        send_to: ["*"],
        on_shell_read: "withhold",
      },
    ],
    subagents: { report_declassifies: true },
    tool_access: { providers: {} },
  },
  destinations: [],
  match_counts: { normal: 0 },
  error: null,
  source_paths: [],
  has_project_overrides: false,
};

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
  digest: "loaded-content-digest",
  ...overrides,
});

const setEditorContent = (editor: HTMLElement, content: string) => {
  const editorView = EditorView.findFromDOM(editor);
  if (!editorView) throw new Error("CodeMirror view not found for editor");
  editorView.dispatch({
    changes: { from: 0, to: editorView.state.doc.length, insert: content },
  });
};

function ActiveFileViewer() {
  const activeTabId = useAppSelector((state) => state.workspace.activeTabId);
  if (!activeTabId?.startsWith("file:")) return null;
  return <FileViewer path={activeTabId.slice("file:".length)} />;
}

const expectEditorText = (text: string) =>
  waitFor(() =>
    expect(document.querySelector(".cm-content")).toHaveTextContent(text),
  );

describe("FilesPanel", () => {
  beforeEach(() => {
    server.use(
      http.get("*/v1/privacy/policy", () => HttpResponse.json(privacyResponse)),
    );
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

  it("portals compact controls into one pane header and moves details into information", async () => {
    const markdownPath = `${sourcePath}/notes.md`;
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          readResponse({
            path: markdownPath,
            content: "# Compact\n",
            language: "markdown",
            can_edit: false,
          }),
        ),
      ),
    );
    const { user, store } = render(
      <PaneHeaderProvider>
        <header data-testid="pane-header">
          <span>notes.md</span>
          <PaneHeaderActions />
        </header>
        <FileViewer path={markdownPath} />
      </PaneHeaderProvider>,
    );
    act(() => {
      store.dispatch(openTab(`file:${markdownPath}`));
    });
    expect(
      await screen.findByRole("heading", { name: "Compact" }),
    ).toBeVisible();
    const header = screen.getByTestId("pane-header");
    expect(
      within(header).getByRole("button", { name: "Show Markdown source" }),
    ).toBeVisible();
    expect(screen.getAllByTestId("pane-header")).toHaveLength(1);
    expect(
      screen
        .getByRole("region", { name: "File viewer" })
        .querySelector("header"),
    ).toBeNull();
    expect(screen.queryByRole("navigation", { name: "File path" })).toBeNull();
    expect(
      within(header).getByRole("status", { name: "Read-only" }),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Show Markdown source" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "notes.md file contents" }),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Show rendered Markdown" }),
    );
    expect(
      await screen.findByRole("heading", { name: "Compact" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "File information" }));
    expect(await screen.findByText(markdownPath)).toBeVisible();
    expect(screen.getByText("markdown")).toBeVisible();
    expect(screen.getByText("34 bytes")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Copy file path" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Reload" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Split pane right" }));
    expect(
      store.getState().workspace.groups[`file:${markdownPath}`]?.root.kind,
    ).toBe("split");
  });

  it("retains a draft while inactive without leaking controls and keeps discard actions in the pane header", async () => {
    server.use(
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const contents = (active: boolean) => (
      <PaneHeaderProvider>
        <header data-testid="pane-header">
          <PaneHeaderActions />
        </header>
        <PaneSurfaceActivityProvider active={active}>
          <FileViewer path={filePath} />
        </PaneSurfaceActivityProvider>
      </PaneHeaderProvider>
    );
    const view = render(contents(true));
    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    setEditorContent(
      await screen.findByRole("textbox", { name: "Edit main.ts" }),
      "retained draft\n",
    );
    view.rerender(contents(false));
    expect(
      within(screen.getByTestId("pane-header")).queryByRole("button"),
    ).toBeNull();
    view.rerender(contents(true));
    expect(
      EditorView.findFromDOM(
        screen.getByRole("textbox", { name: "Edit main.ts" }),
      )?.state.doc.toString(),
    ).toBe("retained draft\n");
    const header = screen.getByTestId("pane-header");
    expect(within(header).getByRole("button", { name: "Save" })).toBeVisible();
    await view.user.click(
      within(header).getByRole("button", { name: "Cancel" }),
    );
    expect(within(header).getByRole("alert")).toHaveTextContent(
      "Discard changes?",
    );
    await view.user.click(
      within(header).getByRole("button", { name: "Keep editing" }),
    );
    await view.user.click(
      within(header).getByRole("button", { name: "Cancel" }),
    );
    await view.user.click(
      within(header).getByRole("button", { name: "Discard" }),
    );
    expect(screen.queryByRole("textbox", { name: "Edit main.ts" })).toBeNull();
    expect(
      within(header).getByRole("button", { name: "Edit this file" }),
    ).toBeVisible();
  });

  it("debounces project-wide metadata search outside expanded folders and requires explicit activation", async () => {
    const searches: URLSearchParams[] = [];
    server.use(
      rootHandler(),
      http.get("*/v1/files/search", ({ request }) => {
        searches.push(new URL(request.url).searchParams);
        return HttpResponse.json({
          entries: [
            {
              name: "hidden.ts",
              path: "/workspace/unexpanded/hidden.ts",
              relative_path: "unexpanded/hidden.ts",
              root: "/workspace",
              kind: "file",
              size: 12,
              ignored: false,
              privacy_zone: {
                name: "normal",
                send_to: ["*"],
                on_shell_read: "withhold",
              },
            },
          ],
          truncated: true,
          complete: false,
          index_state: "building",
        });
      }),
    );
    const { user, store } = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    const input = screen.getByRole("combobox", {
      name: "Search project files and folders",
    });
    await user.type(input, "hidden");
    expect(searches).toHaveLength(0);
    expect(
      await screen.findByRole("option", { name: /hidden.ts/ }),
    ).toHaveTextContent("unexpanded/hidden.ts");
    expect(searches).toHaveLength(1);
    expect(searches[0].get("q")).toBe("hidden");
    expect(searches[0].get("root")).toBeNull();
    expect(searches[0].get("show_ignored")).toBe("false");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Index building; results may be incomplete",
    );
    expect(screen.getByRole("status")).toHaveTextContent("Results truncated");
    expect(store.getState().workspace.tabs).toEqual([]);
    await user.keyboard("{Enter}");
    expect(store.getState().workspace.tabs).toEqual([]);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(store.getState().workspace.activeTabId).toBe(
      "file:/workspace/unexpanded/hidden.ts",
    );
    await user.keyboard("{Escape}");
    expect(input).toHaveValue("");
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("refreshes the same search from building to ready with bounded visible polling and stops on clear or unmount", async () => {
    const searches: string[] = [];
    let indexReady = false;
    let filename = "pending.ts";
    server.use(
      rootHandler(),
      http.get("*/v1/files/search", ({ request }) => {
        searches.push(request.url);
        return HttpResponse.json({
          entries: [
            {
              name: filename,
              path: `${sourcePath}/${filename}`,
              relative_path: `src/${filename}`,
              root: rootPath,
              kind: "file",
              size: 12,
              ignored: false,
              privacy_zone: privacyResponse.policy.zones[0],
            },
          ],
          truncated: false,
          complete: indexReady,
          index_state: indexReady ? "ready" : "building",
        });
      }),
    );
    vi.useFakeTimers();
    const view = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    const advance = async (milliseconds: number) => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(milliseconds);
      });
    };
    const visibilityDescriptor = Object.getOwnPropertyDescriptor(
      document,
      "visibilityState",
    );
    const setVisibility = (value: DocumentVisibilityState) =>
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value,
      });
    try {
      const input = screen.getByRole("combobox", {
        name: "Search project files and folders",
      });
      fireEvent.change(input, { target: { value: "pend" } });
      await advance(200);
      expect(searches).toHaveLength(0);
      fireEvent.change(input, { target: { value: "pending" } });
      await advance(200);
      expect(searches).toHaveLength(0);
      await advance(100);
      expect(searches).toHaveLength(1);
      expect(screen.getByRole("status")).toHaveTextContent("Index building");
      indexReady = true;
      await advance(4_000);
      expect(searches).toHaveLength(1);
      await advance(1_100);
      expect(searches).toHaveLength(2);
      expect(screen.getByRole("status")).not.toHaveTextContent("incomplete");
      filename = "pending-new.ts";
      await advance(5_100);
      expect(searches).toHaveLength(3);
      expect(screen.getByRole("option")).toHaveTextContent(filename);
      setVisibility("hidden");
      fireEvent(document, new Event("visibilitychange"));
      await advance(15_000);
      expect(searches).toHaveLength(3);
      setVisibility("visible");
      fireEvent(document, new Event("visibilitychange"));
      await advance(100);
      expect(searches).toHaveLength(4);
      fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
      await advance(15_000);
      expect(searches).toHaveLength(4);
      fireEvent.change(input, { target: { value: "pending" } });
      await advance(300);
      expect(searches).toHaveLength(5);
      view.unmount();
      await advance(15_000);
      expect(searches).toHaveLength(5);
      expect(new Set(searches).size).toBe(1);
    } finally {
      view.unmount();
      if (visibilityDescriptor)
        Object.defineProperty(
          document,
          "visibilityState",
          visibilityDescriptor,
        );
      else Reflect.deleteProperty(document, "visibilityState");
      vi.useRealTimers();
    }
  });

  it("manually refreshes degraded success and retains results with an honest retry after failure", async () => {
    let requests = 0;
    let resolve:
      | ((response: ReturnType<typeof HttpResponse.json>) => void)
      | undefined;
    server.use(
      rootHandler(),
      http.get("*/v1/files/search", () => {
        requests += 1;
        if (requests === 2)
          return new Promise<ReturnType<typeof HttpResponse.json>>((done) => {
            resolve = done;
          });
        return HttpResponse.json({
          entries: [
            {
              name: "partial.ts",
              path: `${sourcePath}/partial.ts`,
              relative_path: "src/partial.ts",
              root: rootPath,
              kind: "file",
              size: 12,
              ignored: false,
              privacy_zone: privacyResponse.policy.zones[0],
            },
          ],
          truncated: false,
          complete: requests > 2,
          index_state: requests > 2 ? "ready" : "degraded",
        });
      }),
    );
    const { user } = render(<FilesPanel />);
    await user.type(
      screen.getByRole("combobox", {
        name: "Search project files and folders",
      }),
      "partial",
    );
    expect(await screen.findByRole("option")).toHaveTextContent("partial.ts");
    expect(screen.getByRole("status")).toHaveTextContent("Index degraded");
    await user.click(screen.getByRole("button", { name: "Refresh search" }));
    await waitFor(() => expect(resolve).toBeDefined());
    expect(screen.getByRole("option")).toHaveTextContent("partial.ts");
    expect(
      screen.getByRole("button", { name: "Refresh search" }),
    ).toBeDisabled();
    resolve?.(HttpResponse.json({ detail: "unavailable" }, { status: 503 }));
    expect(
      await screen.findByRole("button", { name: "Retry search" }),
    ).toBeEnabled();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Refresh failed; showing previous results",
    );
    expect(screen.getByRole("option")).toHaveTextContent("partial.ts");
    await user.click(screen.getByRole("button", { name: "Retry search" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Refresh search" }),
      ).toBeNull(),
    );
    expect(screen.getByRole("status")).not.toHaveTextContent("incomplete");
    expect(requests).toBe(3);
  });

  it("sends sort and explicit project scope to search and browses folder results without reading contents", async () => {
    const searches: URLSearchParams[] = [];
    const reads: string[] = [];
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", ({ request }) => {
        reads.push(request.url);
        return HttpResponse.json(readResponse());
      }),
      http.get("*/v1/files/search", ({ request }) => {
        searches.push(new URL(request.url).searchParams);
        return HttpResponse.json({
          entries: [
            {
              name: "empty",
              path: "/workspace/empty",
              relative_path: "empty",
              root: "/workspace",
              kind: "dir",
              size: null,
              ignored: false,
              privacy_zone: {
                name: "normal",
                send_to: ["*"],
                on_shell_read: "withhold",
              },
            },
          ],
          truncated: false,
          complete: true,
          index_state: "ready",
        });
      }),
    );
    const { user, store } = render(<FilesPanel />, {
      preloadedState: {
        current_project: {
          name: "workspace",
          workspaceRoots: [rootPath, "/second"],
        },
      },
    });
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Search scope" }),
      rootPath,
    );
    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitemradio", { name: "Sort by size" }),
    );
    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(screen.getByRole("menuitem", { name: "Ascending" }));
    await user.type(
      screen.getByRole("combobox", {
        name: "Search project files and folders",
      }),
      "empty",
    );
    const folder = await screen.findByRole("option", { name: /empty/ });
    expect(searches.at(-1)?.get("sort")).toBe("size");
    expect(searches.at(-1)?.get("direction")).toBe("desc");
    expect(searches.at(-1)?.get("root")).toBe(rootPath);
    await user.click(folder);
    expect(
      screen.getByRole("combobox", {
        name: "Search project files and folders",
      }),
    ).toHaveValue("");
    expect(
      await screen.findByRole("button", { name: "Current worktree" }),
    ).toBeVisible();
    expect(reads).toEqual([]);
    expect(store.getState().workspace.tabs).toEqual([]);
    expect(store.getState().current_project.workspaceRoots).toEqual([
      rootPath,
      "/second",
    ]);
  });

  it("clears search on project switches and ignores a late previous-project response", async () => {
    let resolve: ((response: Response) => void) | undefined;
    server.use(
      rootHandler(),
      http.get(
        "*/v1/files/search",
        () =>
          new Promise<Response>((done) => {
            resolve = done;
          }),
      ),
    );
    const { user, store } = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });
    const input = screen.getByRole("combobox", {
      name: "Search project files and folders",
    });
    await user.type(input, "old");
    await waitFor(() => expect(resolve).toBeDefined());
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({ name: "other", workspaceRoots: ["/other"] }),
      );
    });
    expect(input).toHaveValue("");
    resolve?.(
      HttpResponse.json({
        entries: [{ name: "old", path: "/workspace/old" }],
        complete: true,
        truncated: false,
        index_state: "ready",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("option")).toBeNull());
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

    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitemcheckbox", { name: "Show ignored" }),
    );

    expect(
      await screen.findByRole("treeitem", { name: /debug\.log/i }),
    ).toHaveAttribute("data-ignored", "true");
  });

  it("persists the ignored toggle per project", async () => {
    server.use(rootHandler());
    setProjectStorageNamespace("project-a");
    const first = render(<FilesPanel />);
    await first.user.click(
      screen.getByRole("button", { name: "More options" }),
    );
    await first.user.click(
      screen.getByRole("menuitemcheckbox", { name: "Show ignored" }),
    );
    await first.user.click(
      screen.getByRole("button", { name: "More options" }),
    );
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Show ignored" }),
    ).toBeChecked();
    first.unmount();

    setProjectStorageNamespace("project-b");
    const second = render(<FilesPanel />);
    await second.user.click(
      screen.getByRole("button", { name: "More options" }),
    );
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Show ignored" }),
    ).not.toBeChecked();
    second.unmount();

    setProjectStorageNamespace("project-a");
    const third = render(<FilesPanel />);
    await third.user.click(
      screen.getByRole("button", { name: "More options" }),
    );
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Show ignored" }),
    ).toBeChecked();
  });

  it("toggles live edits for the focused chat from the explorer header", async () => {
    server.use(rootHandler());
    const view = render(<FilesPanel />);
    view.store.dispatch(createChatWithId({ id: "chat-a" }));
    view.store.dispatch(openTab("chat:chat-a"));
    view.store.dispatch(setActiveTab("chat:chat-a"));

    await view.user.click(screen.getByRole("button", { name: "More options" }));
    const liveEdits = await screen.findByRole("menuitemcheckbox", {
      name: "Live edits",
    });
    await waitFor(() => expect(liveEdits).toBeEnabled());
    expect(liveEdits).not.toBeChecked();

    await view.user.click(liveEdits);

    await waitFor(() =>
      expect(view.store.getState().workspace.liveEditsByChat?.["chat-a"]).toBe(
        true,
      ),
    );
    await view.user.click(screen.getByRole("button", { name: "More options" }));
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Live edits" }),
    ).toBeChecked();
  });

  it("disables live edits without a focused chat and explains why", async () => {
    server.use(rootHandler());
    const { user, store } = render(<FilesPanel />);
    const more = screen.getByRole("button", { name: "More options" });
    more.focus();
    await user.keyboard("{Enter}");
    const liveEdits = await screen.findByRole("menuitemcheckbox", {
      name: "Live edits",
    });
    expect(liveEdits).toHaveAttribute("aria-disabled", "true");
    expect(liveEdits).not.toBeChecked();
    expect(liveEdits).toHaveAccessibleDescription("Open a chat to enable");
    await user.click(liveEdits);
    expect(store.getState().workspace.liveEditsByChat).toBeUndefined();
    await user.keyboard("{Escape}");
    expect(more).toHaveFocus();
  });

  it("replaces refetched children while keeping the directory expanded", async () => {
    let sourceRequests = 0;
    let sourceEntries: FilesTreeEntry[] = [
      {
        name: "main.ts",
        path: filePath,
        kind: "file",
        size: 22,
        privacy_zone: privacyResponse.policy.zones[0],
      },
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
        privacy_zone: privacyResponse.policy.zones[0],
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
      {
        name: "old.ts",
        path: renamedFrom,
        kind: "file",
        size: 8,
        privacy_zone: privacyResponse.policy.zones[0],
      },
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
      {
        name: "created.ts",
        path: createdPath,
        kind: "file",
        size: 8,
        privacy_zone: privacyResponse.policy.zones[0],
      },
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
    testEntries = [
      {
        name: "new.ts",
        path: renamedTo,
        kind: "file",
        size: 8,
        privacy_zone: privacyResponse.policy.zones[0],
      },
    ];
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

    const targetLine = await waitFor(() => {
      const element = document.querySelector('[data-target-line="true"]');
      expect(element).not.toBeNull();
      return element as HTMLElement;
    });
    expect(targetLine).toHaveAttribute("data-line-number", "2");
    expect(targetLine).toHaveTextContent("const second = 2;");
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

  it("keeps noncollapsed editor selections while recording the caret", async () => {
    server.use(
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    const editor = await screen.findByRole("textbox", { name: "Edit main.ts" });
    const editorView = EditorView.findFromDOM(editor);
    if (!editorView) throw new Error("Missing editor view");
    editorView.contentDOM.blur();

    act(() => editorView.dispatch({ selection: { anchor: 0, head: 5 } }));
    fireEvent.keyUp(editor, { key: "ArrowRight", shiftKey: true });
    await waitFor(() => {
      expect(editorView.state.selection.main.anchor).toBe(0);
      expect(editorView.state.selection.main.head).toBe(5);
    });
    act(() => editorView.dispatch({ selection: { anchor: 2, head: 10 } }));
    fireEvent.mouseUp(editor);
    await waitFor(() => {
      expect(editorView.state.selection.main.anchor).toBe(2);
      expect(editorView.state.selection.main.head).toBe(10);
    });
  });

  it("retains an editor draft and conflict base across structural remounts and clears on actual close", async () => {
    const writes: unknown[] = [];
    server.use(
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
      http.post("*/v1/files/write", async ({ request }) => {
        writes.push(await request.json());
        return HttpResponse.json({ path: filePath, size: 6, mtime_ms: 2 });
      }),
    );
    const view = render(<FileViewer key="before" path={filePath} />);
    view.store.dispatch(openTab(`file:${filePath}`));
    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    setEditorContent(
      await screen.findByRole("textbox", { name: "Edit main.ts" }),
      "unsaved remount draft",
    );
    view.rerender(<FileViewer key="after" path={filePath} />);
    expect(
      EditorView.findFromDOM(
        await screen.findByRole("textbox", { name: "Edit main.ts" }),
      )?.state.doc.toString(),
    ).toBe("unsaved remount draft");
    await view.user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(writes).toEqual([
        expect.objectContaining({
          content: "unsaved remount draft",
          expected_digest: "loaded-content-digest",
        }),
      ]),
    );
    await view.user.click(
      screen.getByRole("button", { name: "Edit this file" }),
    );
    setEditorContent(
      await screen.findByRole("textbox", { name: "Edit main.ts" }),
      "must clear",
    );
    view.rerender(<div />);
    view.store.dispatch(closeTab(`file:${filePath}`));
    view.store.dispatch(openTab(`file:${filePath}`));
    view.rerender(<FileViewer path={filePath} />);
    expect(
      await screen.findByRole("button", { name: "Edit this file" }),
    ).toBeEnabled();
    expect(screen.queryByRole("textbox", { name: "Edit main.ts" })).toBeNull();
  });

  it("saves an edited file with the loaded mtime as the conflict token", async () => {
    const writes: unknown[] = [];
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
      http.post("*/v1/files/write", async ({ request }) => {
        writes.push(await request.json());
        return HttpResponse.json({ path: filePath, size: 6, mtime_ms: 2 });
      }),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    const editor = await screen.findByRole("textbox", {
      name: /Edit main\.ts/,
    });
    setEditorContent(editor, "edited\n");
    await view.user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(writes).toEqual([
        {
          path: filePath,
          content: "edited\n",
          expected_mtime_ms: 1,
          expected_digest: "loaded-content-digest",
        },
      ]);
    });
  });

  it("keeps the original draft precondition when a live refetch changes the file", async () => {
    const writes: unknown[] = [];
    let changed = false;
    let reads = 0;
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => {
        reads += 1;
        return HttpResponse.json(
          readResponse(
            changed
              ? {
                  content: "external\n",
                  mtime_ms: 2,
                  digest: "external-digest",
                }
              : {},
          ),
        );
      }),
      http.post("*/v1/files/write", async ({ request }) => {
        writes.push(await request.json());
        return HttpResponse.json({ detail: "changed" }, { status: 409 });
      }),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));
    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    setEditorContent(
      await screen.findByRole("textbox", { name: /Edit main\.ts/ }),
      "draft\n",
    );
    changed = true;
    view.store.dispatch(
      filesApi.util.invalidateTags([{ type: "File", id: filePath }]),
    );
    await waitFor(() => expect(reads).toBeGreaterThan(1));
    await view.user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(writes).toEqual([
        {
          path: filePath,
          content: "draft\n",
          expected_mtime_ms: 1,
          expected_digest: "loaded-content-digest",
        },
      ]),
    );
  });

  it("surfaces a save conflict instead of clobbering the file", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
      http.post("*/v1/files/write", () =>
        HttpResponse.json({ detail: "changed on disk" }, { status: 409 }),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    const editor = await screen.findByRole("textbox", {
      name: /Edit main\.ts/,
    });
    setEditorContent(editor, "edited\n");
    await view.user.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByText(/changed on disk since it was loaded/),
    ).toBeVisible();

    expect(
      EditorView.findFromDOM(
        screen.getByRole("textbox", { name: /Edit main\.ts/ }),
      )?.state.doc.toString(),
    ).toBe("edited\n");

    await view.user.click(screen.getByRole("button", { name: "Reload" }));
    expect(await screen.findByText("Discard changes?")).toBeVisible();
    expect(
      screen.getByRole("textbox", { name: /Edit main\.ts/ }),
    ).toBeInTheDocument();
  });

  it("enters edit mode when the code view is double-clicked", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    const line = await waitFor(() => {
      const element = document.querySelector('[data-line-number="2"]');
      expect(element).not.toBeNull();
      return element as HTMLElement;
    });
    expect(
      screen.queryByRole("textbox", { name: /Edit main\.ts/ }),
    ).not.toBeInTheDocument();

    fireEvent.dblClick(line);

    expect(
      await screen.findByRole("textbox", { name: /Edit main\.ts/ }),
    ).toBeVisible();
  });

  it("renders markdown as a preview and edits the raw source on double click", async () => {
    const markdownPath = `${sourcePath}/notes.md`;
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          readResponse({
            path: markdownPath,
            content: "# Heading\n\nSome prose.\n",
            language: "markdown",
          }),
        ),
      ),
    );
    const view = render(<FileViewer path={markdownPath} />);
    view.store.dispatch(openFileInFilesPanel({ path: markdownPath }));

    expect(
      await screen.findByRole("heading", { name: "Heading" }),
    ).toBeVisible();

    fireEvent.dblClick(screen.getByText("Some prose."));

    expect(
      await screen.findByRole("textbox", { name: /Edit notes\.md/ }),
    ).toBeVisible();
  });

  it("toggles markdown between the rendered preview and the source", async () => {
    const markdownPath = `${sourcePath}/notes.md`;
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          readResponse({
            path: markdownPath,
            content: "# Heading\n\nSome prose.\n",
            language: "markdown",
          }),
        ),
      ),
    );
    const view = render(<FileViewer path={markdownPath} />);
    view.store.dispatch(openFileInFilesPanel({ path: markdownPath }));

    expect(await screen.findByLabelText("Markdown preview")).toBeVisible();

    await view.user.click(
      screen.getByRole("button", { name: "Show Markdown source" }),
    );

    expect(screen.queryByLabelText("Markdown preview")).not.toBeInTheDocument();
    expect(
      await screen.findByLabelText(/notes\.md file contents/),
    ).toBeInTheDocument();
  });

  it.each(["page.html", "PAGE.HTM"])(
    "previews %s in an isolated sandbox and toggles source",
    async (filename) => {
      const htmlPath = `${sourcePath}/${filename}`;
      const content =
        '<!doctype html><html><head><base href="http://localhost:8001/"><meta http-equiv="refresh" content="0;url=https://example.com"></head><body><h1>Preview</h1><img src="/v1/private"><script>window.preview = true</script></body></html>';
      server.use(
        http.get("*/v1/files/read", () =>
          HttpResponse.json(
            readResponse({ path: htmlPath, content, language: null }),
          ),
        ),
      );
      const view = render(<FileViewer path={htmlPath} />);
      const frame = await screen.findByTitle("HTML preview");
      expect(frame).toHaveAttribute("sandbox", "allow-scripts");
      expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
      const document = new DOMParser().parseFromString(
        frame.getAttribute("srcdoc") ?? "",
        "text/html",
      );
      expect(document.querySelectorAll("base")).toHaveLength(1);
      expect(document.querySelector("base")).toHaveAttribute(
        "href",
        "about:srcdoc",
      );
      expect(document.querySelector('meta[http-equiv="refresh" i]')).toBeNull();
      expect(document.head.firstElementChild).toHaveAttribute(
        "http-equiv",
        "Content-Security-Policy",
      );
      expect(
        document.head.firstElementChild?.getAttribute("content"),
      ).toContain("default-src 'none'");
      expect(
        document.head.firstElementChild?.getAttribute("content"),
      ).toContain("img-src data: blob:");
      expect(
        document.querySelector("[data-refact-artifact-bootstrap]"),
      ).not.toBeNull();
      expect(document.body.textContent).toContain("Preview");
      await view.user.click(
        screen.getByRole("button", { name: "Show HTML source" }),
      );
      expect(screen.queryByTitle("HTML preview")).toBeNull();
      await expectEditorText("window.preview = true");
      await view.user.click(
        screen.getByRole("button", { name: "Show rendered HTML" }),
      );
      expect(await screen.findByTitle("HTML preview")).toBeInTheDocument();
      await view.user.click(
        screen.getByRole("button", { name: "Edit this file" }),
      );
      expect(screen.queryByTitle("HTML preview")).toBeNull();
      expect(
        await screen.findByRole("textbox", { name: `Edit ${filename}` }),
      ).toBeInTheDocument();
    },
  );

  it("confirms before discarding unsaved edits and cancels cleanly when untouched", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    await view.user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Discard changes?")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("textbox", { name: /Edit main\.ts/ }),
    ).not.toBeInTheDocument();

    await view.user.click(
      await screen.findByRole("button", { name: "Edit this file" }),
    );
    const editor = await screen.findByRole("textbox", {
      name: /Edit main\.ts/,
    });
    setEditorContent(editor, "changed\n");

    await view.user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText("Discard changes?")).toBeVisible();

    await view.user.click(screen.getByRole("button", { name: "Keep editing" }));
    expect(screen.queryByText("Discard changes?")).not.toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Cancel" }));
    await view.user.click(
      await screen.findByRole("button", { name: "Discard" }),
    );
    expect(
      screen.queryByRole("textbox", { name: /Edit main\.ts/ }),
    ).not.toBeInTheDocument();
  });

  it("locks editing while edits are playing back", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));
    view.store.dispatch(
      enqueueEditPlayerSteps({
        chatId: "chat-a",
        steps: [
          {
            id: "step-1",
            path: filePath,
            revision: "1",
            line: 2,
            chunks: [],
            operation: "write",
          },
        ],
      }),
    );

    expect(
      await screen.findByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Pause edit playback" }),
    ).toBeVisible();

    const playingLine = document.querySelector('[data-line-number="1"]');
    expect(playingLine).not.toBeNull();
    fireEvent.dblClick(playingLine as HTMLElement);
    expect(
      screen.queryByRole("textbox", { name: /Edit main\.ts/ }),
    ).not.toBeInTheDocument();
  });

  it("does not enter edit mode on double click when the file is not editable", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ truncated: true })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    const line = await waitFor(() => {
      const element = document.querySelector('[data-line-number="1"]');
      expect(element).not.toBeNull();
      return element as HTMLElement;
    });

    fireEvent.dblClick(line);

    expect(
      screen.queryByRole("textbox", { name: /Edit main\.ts/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it("collapses removed lines and reveals added lines for the playing step", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));
    view.store.dispatch(
      enqueueEditPlayerSteps({
        chatId: "chat-a",
        steps: [
          {
            id: "step-1",
            path: filePath,
            revision: "1",
            line: 2,
            chunks: [
              {
                file_name: filePath,
                file_action: "edit",
                line1: 2,
                line2: 3,
                lines_remove: "const second = 0;\n",
                lines_add: "const second = 2;\n",
              },
            ],
            operation: "write",
          },
        ],
      }),
    );

    await waitFor(() => {
      expect(
        document.querySelector('[data-live-removed="true"]'),
      ).toHaveTextContent("const second = 0;");
    });
    expect(document.querySelector('[data-live-change="true"]')).toHaveAttribute(
      "data-line-number",
      "2",
    );
  });

  it("shows an honest privacy-blocked state", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          { detail: "Blocked by privacy rules: privacy level Blocked" },
          { status: 403 },
        ),
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

  it("does not blame privacy for a non-privacy access denial", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          { detail: "Path 'x' is outside of project directories:\n[]" },
          { status: 403 },
        ),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("File unavailable")).toBeVisible();
    expect(
      screen.queryByText("This file is blocked by privacy rules."),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/outside of project directories/)).toBeVisible();
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
    expect(
      screen.getByText("new").closest("[data-line-number]"),
    ).toHaveAttribute("data-live-change", "true");
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

  it("keeps compact Explorer actions alongside the shrinkable path input", () => {
    const css = readFileSync(
      "src/features/Workspace/FilesPanel/FilesPanel.module.css",
      "utf8",
    );
    const pathInput = /\.openPathInput\s*\{[^}]*\}/u.exec(css)?.[0] ?? "";
    expect(pathInput).toContain("min-width: 0");
    expect(pathInput).toContain("min-height: var(--rf-control-h-sm)");
    expect(css).not.toContain(".headerToggles");
  });

  it("keeps the reduced-motion rule for live change highlights", () => {
    const css = readFileSync(
      "src/features/Workspace/FilesPanel/CodeMirrorEditor.module.css",
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
    const selectedRow = within(tree).getByRole("treeitem", {
      name: /README\.md/i,
    });
    expect(selectedRow.id).not.toBe("");
    expect(tree).toHaveAttribute("aria-activedescendant", selectedRow.id);
  });

  it("validates the absolute path input with an accessible error", async () => {
    server.use(rootHandler());
    const { user, store } = render(<FilesPanel />);
    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitem", { name: "Open file by path…" }),
    );
    const input = screen.getByLabelText("Absolute file path");

    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Enter an absolute file path.",
    );
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute(
      "aria-describedby",
      screen.getByRole("alert").id,
    );

    await user.type(input, "relative/file.txt{Enter}");
    expect(screen.getByRole("alert")).toHaveTextContent("Use an absolute path");
    expect(store.getState().workspace.tabs).toEqual([]);

    await user.clear(input);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(input).not.toHaveAttribute("aria-invalid");
  });

  it("opens an external absolute path read-only through POST /open without adding a root", async () => {
    const externalPath = "/etc/hosts";
    let openBody: unknown = null;
    let readCalls = 0;
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => {
        readCalls += 1;
        return HttpResponse.json(readResponse());
      }),
      http.post("*/v1/files/open", async ({ request }) => {
        openBody = await request.json();
        return HttpResponse.json(
          readResponse({
            path: externalPath,
            content: "127.0.0.1 localhost\n",
            language: null,
            can_edit: false,
            image_mime: null,
            image_data: null,
          }),
        );
      }),
    );
    const { user, store } = render(
      <>
        <FilesPanel />
        <ActiveFileViewer />
      </>,
      {
        preloadedState: {
          current_project: { name: "workspace", workspaceRoots: [rootPath] },
        },
      },
    );

    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitem", { name: "Open file by path…" }),
    );
    await user.type(
      screen.getByLabelText("Absolute file path"),
      `  ${externalPath}  {Enter}`,
    );

    await expectEditorText("127.0.0.1 localhost");
    expect(openBody).toEqual({ path: externalPath });
    expect(readCalls).toBe(0);
    expect(store.getState().workspace.activeTabId).toBe(`file:${externalPath}`);
    expect(store.getState().current_project.workspaceRoots).toEqual([rootPath]);
    expect(store.getState().filesPanel.viewerTargets[externalPath]).toEqual({
      path: externalPath,
      manual: true,
    });
    expect(screen.getByText("Read-only")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it("restores an external tab through protected open without remembered target metadata", async () => {
    let readCalls = 0;
    let openBody: unknown = null;
    server.use(
      http.get("*/v1/files/read", () => {
        readCalls += 1;
        return HttpResponse.json(
          { detail: "Outside workspace" },
          { status: 403 },
        );
      }),
      http.post("*/v1/files/open", async ({ request }) => {
        openBody = await request.json();
        return HttpResponse.json(
          readResponse({ path: "/tmp/restored.txt", can_edit: false }),
        );
      }),
    );
    const { store } = render(<FileViewer path="/tmp/restored.txt" />, {
      preloadedState: {
        current_project: { name: "workspace", workspaceRoots: [rootPath] },
      },
    });

    await expectEditorText("const first = 1;");
    expect(openBody).toEqual({ path: "/tmp/restored.txt" });
    expect(readCalls).toBe(0);
    expect(store.getState().filesPanel.viewerTargets).toEqual({});
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it.each([undefined, false])(
    "opens linked external images through protected open with manual metadata %s",
    async (manual) => {
      const path = "/tmp/refact-review-overhaul/header-1600.png";
      const reads = vi.fn(() =>
        HttpResponse.json(
          { detail: "Path is outside of project directories" },
          { status: 403 },
        ),
      );
      const opens = vi.fn();
      server.use(
        http.get("*/v1/files/read", reads),
        http.post("*/v1/files/open", async ({ request }) => {
          opens(await request.json());
          return HttpResponse.json(
            readResponse({
              path,
              content: "",
              language: null,
              binary: true,
              can_edit: false,
              image_mime: "image/png",
              image_data: "iVBORw0KGgo=",
            }),
          );
        }),
      );
      const { store } = render(<ActiveFileViewer />, {
        preloadedState: {
          current_project: { name: "workspace", workspaceRoots: [rootPath] },
        },
      });
      act(() => {
        store.dispatch(openFileInFilesPanel({ path, line: 1, manual }));
      });

      const image = await screen.findByRole("img", { name: "header-1600.png" });
      expect(image).toHaveAttribute(
        "src",
        "data:image/png;base64,iVBORw0KGgo=",
      );
      expect(opens).toHaveBeenCalledExactlyOnceWith({ path });
      expect(reads).not.toHaveBeenCalled();
      expect(store.getState().current_project.workspaceRoots).toEqual([
        rootPath,
      ]);
      expect(
        screen.getByRole("button", { name: "Edit this file" }),
      ).toBeDisabled();
    },
  );

  it("never treats a manual path as editable without can_edit true", async () => {
    server.use(
      http.post("*/v1/files/open", () =>
        HttpResponse.json(readResponse({ path: "/tmp/x.ts" })),
      ),
    );
    const view = render(<FileViewer path="/tmp/x.ts" />);
    view.store.dispatch(
      openFileInFilesPanel({ path: "/tmp/x.ts", manual: true }),
    );

    await expectEditorText("const first = 1;");
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it("allows editing a manual path only when the backend grants can_edit", async () => {
    server.use(
      http.post("*/v1/files/open", () =>
        HttpResponse.json(readResponse({ can_edit: true })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath, manual: true }));

    await expectEditorText("const first = 1;");
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeEnabled();
    expect(screen.queryByText("Read-only")).toBeNull();
  });

  it("disables editing for a workspace read that reports can_edit false", async () => {
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ can_edit: false })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    await expectEditorText("const first = 1;");
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it("renders a safe raster image with preview controls", async () => {
    server.use(
      http.post("*/v1/files/open", () =>
        HttpResponse.json(
          readResponse({
            path: "/tmp/pic.png",
            content: "",
            language: null,
            size: 8,
            binary: true,
            can_edit: false,
            image_mime: "image/png",
            image_data: "iVBORw0KGgo=",
          }),
        ),
      ),
    );
    const view = render(<FileViewer path="/tmp/pic.png" />);
    view.store.dispatch(
      openFileInFilesPanel({ path: "/tmp/pic.png", manual: true }),
    );

    const image = await screen.findByRole("img", { name: "pic.png" });
    expect(image).toHaveAttribute("src", "data:image/png;base64,iVBORw0KGgo=");
    expect(
      screen.getByRole("toolbar", { name: "Image controls" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Fit to view" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Edit this file" }),
    ).toBeDisabled();
  });

  it("refuses to render SVG or unknown image MIME types as data URLs", async () => {
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          readResponse({
            path: `${rootPath}/logo.svg`,
            content: "",
            binary: true,
            image_mime: "image/svg+xml",
            image_data: "PHN2Zz48L3N2Zz4=",
          }),
        ),
      ),
    );
    const svgPath = `${rootPath}/logo.svg`;
    const view = render(<FileViewer path={svgPath} />);
    view.store.dispatch(openFileInFilesPanel({ path: svgPath }));

    expect(await screen.findByText("Binary file")).toBeVisible();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("surfaces manual open errors from the backend", async () => {
    server.use(
      http.post("*/v1/files/open", () =>
        HttpResponse.json(
          { detail: "Unsupported binary file" },
          { status: 415 },
        ),
      ),
    );
    const view = render(<FileViewer path="/tmp/blob.bin" />);
    view.store.dispatch(
      openFileInFilesPanel({ path: "/tmp/blob.bin", manual: true }),
    );

    expect(await screen.findByText("File unavailable")).toBeVisible();
    expect(screen.getByText("Unsupported binary file")).toBeVisible();
  });

  it("shows the privacy-blocked state for a manual path", async () => {
    server.use(
      http.post("*/v1/files/open", () =>
        HttpResponse.json(
          { detail: "Blocked by privacy rules: privacy level Blocked" },
          { status: 403 },
        ),
      ),
    );
    const view = render(<FileViewer path="/tmp/secret.env" />);
    view.store.dispatch(
      openFileInFilesPanel({ path: "/tmp/secret.env", manual: true }),
    );

    expect(await screen.findByText("File blocked")).toBeVisible();
  });

  it("explains the image size limit for oversized images", async () => {
    const bigPath = `${rootPath}/huge.png`;
    server.use(
      http.get("*/v1/files/read", () =>
        HttpResponse.json(
          readResponse({
            path: bigPath,
            content: "",
            binary: true,
            size: 11 * 1024 * 1024,
            image_mime: null,
            image_data: null,
          }),
        ),
      ),
    );
    const view = render(<FileViewer path={bigPath} />);
    view.store.dispatch(openFileInFilesPanel({ path: bigPath }));

    expect(await screen.findByText("Image too large")).toBeVisible();
    expect(screen.getByText(/limited to 10 MiB/)).toBeVisible();
  });

  it("refetches a manual path when it is opened again", async () => {
    const externalPath = "/tmp/notes.txt";
    let openCalls = 0;
    server.use(
      http.post("*/v1/files/open", () => {
        openCalls += 1;
        return HttpResponse.json(
          readResponse({
            path: externalPath,
            content: `version ${String(openCalls)}\n`,
            can_edit: false,
          }),
        );
      }),
    );
    const { user } = render(
      <>
        <FilesPanel />
        <ActiveFileViewer />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitem", { name: "Open file by path…" }),
    );
    const input = screen.getByLabelText("Absolute file path");

    await user.type(input, `${externalPath}{Enter}`);
    await expectEditorText("version 1");
    await user.click(screen.getByRole("button", { name: "More options" }));
    await user.click(
      screen.getByRole("menuitem", { name: "Open file by path…" }),
    );
    await user.click(screen.getByRole("button", { name: "Open" }));
    await expectEditorText("version 2");
    expect(openCalls).toBe(2);
  });
});
