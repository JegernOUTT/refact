import { afterEach, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
} from "../ChatPanes/panesTree";
import { setTabDragData } from "../ChatPanes/tabDrag";
import {
  addSurfaceToPane,
  openTab,
  setDockSection,
  setPanelsForced,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { updateConfig } from "../Config/configSlice";
import { WORKSPACE_NARROW_QUERY } from "./workspaceBreakpoints";
import { WorkspaceView } from "./WorkspaceView";
import { Toolbar } from "../../components/Toolbar/Toolbar";
import { WorkspaceShell } from "./WorkspaceShell";
import { panePanelId, paneTabId } from "./paneTabIds";
function getChatGroup(store: AppStore) {
  const group =
    store.getState().workspace.groups[makeSurfaceKey("chat", "chat-a")];
  if (!group) throw new Error("Missing canonical chat group");
  return group;
}

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const task = (id: string): SurfaceKey => makeSurfaceKey("task", id);

vi.mock("../Tasks/TaskWorkspace", () => ({
  TaskWorkspace: ({ taskId }: { taskId: string }) => (
    <div data-testid="task-workspace">{taskId}</div>
  ),
}));

function createDataTransferStub(): DataTransfer {
  const data = new Map<string, string>();
  return {
    dropEffect: "none" as DataTransfer["dropEffect"],
    effectAllowed: "uninitialized" as DataTransfer["effectAllowed"],
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    get types() {
      return Array.from(data.keys());
    },
    clearData: (type?: string) => {
      if (type) {
        data.delete(type);
      } else {
        data.clear();
      }
    },
    getData: (type: string) => data.get(type) ?? "",
    setData: (type: string, value: string) => {
      data.set(type, value);
    },
    setDragImage: () => undefined,
  } as DataTransfer;
}

function mockPaneGeometry(pane: Element): void {
  pane.getBoundingClientRect = () => new DOMRect(100, 100, 400, 300);
  const controls = pane.querySelector("[data-pane-controls]");
  if (!controls) throw new Error("missing floating pane controls");
  controls.getBoundingClientRect = () => new DOMRect(450, 100, 50, 28);
}

function dragToPane(
  pane: HTMLElement,
  dataTransfer: DataTransfer,
  target: "right" | "center",
) {
  mockPaneGeometry(pane);
  const coordinates = {
    dataTransfer,
    clientX: target === "right" ? 490 : 300,
    clientY: 250,
  };
  const dispatchDrag = (type: "dragover" | "drop") => {
    const event = new MouseEvent(type, {
      ...coordinates,
      bubbles: true,
      cancelable: true,
    });
    Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
    fireEvent(pane, event);
  };
  dispatchDrag("dragover");
  expect(
    within(pane).getByTestId("workspace-pane-drop-preview"),
  ).toHaveAttribute("data-drop-target", target);
  expect(screen.getAllByTestId("workspace-pane-drop-preview")).toHaveLength(1);
  dispatchDrag("drop");
}

function createWorkspaceStore(): AppStore {
  const store = setUpStore({ pages: [{ name: "chat" }] });
  store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
  store.dispatch(createChatWithId({ id: "chat-b", title: "Chat Beta" }));
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(openTab(chat("chat-b")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
}

function createSplitWorkspaceStore(): AppStore {
  const store = createWorkspaceStore();
  store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
  const group = store.getState().workspace.groups[chat("chat-a")];
  if (!group) throw new Error("missing split group");
  store.dispatch(
    addSurfaceToPane({
      tabId: chat("chat-a"),
      leafId: group.focusedLeafId,
      surfaceKey: chat("chat-b"),
    }),
  );
  return store;
}

function createEmptySplitWorkspaceStore(): AppStore {
  const store = createWorkspaceStore();
  store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
  return store;
}

function renderWorkspaceView(store: AppStore) {
  return render(
    <>
      <Toolbar
        activeTab={{
          type: "chat",
          id: store.getState().chat.current_thread_id,
        }}
      />
      <WorkspaceShell>
        <WorkspaceView />
      </WorkspaceShell>
    </>,
    { store },
  );
}

function setWorkspaceAvailability(
  store: AppStore,
  {
    filesPanel,
    gitPanel,
    terminalPanel,
    panelsForced = false,
  }: {
    filesPanel: boolean;
    gitPanel: boolean;
    terminalPanel: boolean;
    panelsForced?: boolean;
  },
) {
  store.dispatch(
    updateConfig({ capabilities: { filesPanel, gitPanel, terminalPanel } }),
  );
  store.dispatch(setPanelsForced(panelsForced));
}

function expectSurface(key: SurfaceKey) {
  const element = document.querySelector(`[data-surface-key="${key}"]`);
  expect(element).toBeInTheDocument();
}

const originalMatchMedia = window.matchMedia;

function mockNarrow(narrow: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(
      (query: string): MediaQueryList =>
        ({
          matches: narrow && query === WORKSPACE_NARROW_QUERY,
          media: query,
          onchange: null,
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          addListener: vi.fn(),
          removeListener: vi.fn(),
          dispatchEvent: vi.fn(),
        }) as MediaQueryList,
    ),
  });
}

describe("WorkspaceView", () => {
  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("keeps the activity rail mounted for IDE hosts without forced panels", () => {
    const store = createWorkspaceStore();
    store.dispatch(updateConfig({ host: "vscode" }));
    renderWorkspaceView(store);

    expect(
      screen.getByRole("navigation", { name: "Workspace sections" }),
    ).toBeVisible();
    expect(screen.queryByLabelText("Workspace dock")).not.toBeInTheDocument();
  });

  it.each([
    ["terminal only", false, false, true, false, false],
    ["forced panels", false, false, false, true, true],
    ["files only", true, false, false, false, true],
    ["git only", false, true, false, false, false],
    ["no capabilities", false, false, false, false, false],
  ] as const)(
    "matches dock rendering for web %s with the default files section",
    (_name, filesPanel, gitPanel, terminalPanel, panelsForced, expected) => {
      const store = createWorkspaceStore();
      setWorkspaceAvailability(store, {
        filesPanel,
        gitPanel,
        terminalPanel,
        panelsForced,
      });

      renderWorkspaceView(store);

      const dock = screen.queryByLabelText("Workspace dock");
      if (expected) {
        expect(dock).toBeVisible();
      } else {
        expect(dock).not.toBeInTheDocument();
      }
    },
  );

  it("mounts the dock for a git-only workspace once the git section is selected", () => {
    const store = createWorkspaceStore();
    setWorkspaceAvailability(store, {
      filesPanel: false,
      gitPanel: true,
      terminalPanel: false,
    });
    store.dispatch(setDockSection("git"));

    renderWorkspaceView(store);

    expect(screen.getByLabelText("Workspace dock")).toBeVisible();
  });

  it("mounts the dock for the agents section without any panel capability", () => {
    const store = createWorkspaceStore();
    setWorkspaceAvailability(store, {
      filesPanel: false,
      gitPanel: false,
      terminalPanel: false,
    });
    store.dispatch(setDockSection("agents"));

    renderWorkspaceView(store);

    expect(screen.getByLabelText("Workspace dock")).toBeVisible();
  });

  it("renders forced IDE dock chrome without a duplicate workspace toggle", () => {
    const store = createWorkspaceStore();
    store.dispatch(updateConfig({ host: "vscode" }));
    setWorkspaceAvailability(store, {
      filesPanel: false,
      gitPanel: false,
      terminalPanel: false,
      panelsForced: true,
    });

    renderWorkspaceView(store);

    expect(screen.getByLabelText("Workspace dock")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Toggle workspace dock" }),
    ).not.toBeInTheDocument();
  });

  it.each(["ide", "vscode", "jetbrains"] as const)(
    "keeps %s capability overrides hidden until panels are forced",
    async (host) => {
      const store = createWorkspaceStore();
      store.dispatch(updateConfig({ host }));
      setWorkspaceAvailability(store, {
        filesPanel: true,
        gitPanel: true,
        terminalPanel: true,
      });

      renderWorkspaceView(store);

      expect(screen.queryByLabelText("Workspace dock")).not.toBeInTheDocument();
      expect(
        screen.queryByLabelText("Terminal workbench for chat-a"),
      ).not.toBeInTheDocument();

      store.dispatch(setPanelsForced(true));

      expect(await screen.findByLabelText("Workspace dock")).toBeVisible();
      expect(
        screen.getByLabelText("Terminal workbench for chat-a"),
      ).toBeInTheDocument();
    },
  );

  it("never renders the legacy global terminal drawer", () => {
    const webStore = createWorkspaceStore();
    const webView = renderWorkspaceView(webStore);
    expect(screen.queryByLabelText("Terminal drawer")).not.toBeInTheDocument();
    const workbench = screen.getByLabelText("Terminal workbench for chat-a");
    const composer = screen.getByTestId("chat-form-textarea");
    expect(
      workbench.compareDocumentPosition(composer) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    webView.unmount();

    const ideStore = createWorkspaceStore();
    ideStore.dispatch(updateConfig({ host: "vscode" }));
    renderWorkspaceView(ideStore);
    expect(screen.queryByLabelText("Terminal drawer")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("Terminal workbench for chat-a"),
    ).not.toBeInTheDocument();
  });

  it("renders only the dock for an opted-in IDE host", async () => {
    const store = createWorkspaceStore();
    store.dispatch(updateConfig({ host: "vscode" }));
    renderWorkspaceView(store);

    expect(screen.queryByLabelText("Workspace dock")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Terminal drawer")).not.toBeInTheDocument();

    store.dispatch(setPanelsForced(true));

    expect(await screen.findByLabelText("Workspace dock")).toBeInTheDocument();
    expect(screen.queryByLabelText("Terminal drawer")).not.toBeInTheDocument();
    expect(
      screen.getByLabelText("Terminal workbench for chat-a"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Files" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Git" })).toBeInTheDocument();
  });

  it("renders an unsplit surface with one global toolbar tab row", () => {
    renderWorkspaceView(createWorkspaceStore());

    expectSurface(chat("chat-a"));
    expect(screen.getByLabelText("Terminal sessions")).toBeInTheDocument();
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Close Pane" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Right" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Down" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
  });

  it.each([true, false])(
    "omits floating split controls at narrow=%s",
    (narrow) => {
      mockNarrow(narrow);
      renderWorkspaceView(createWorkspaceStore());
      expectSurface(chat("chat-a"));
      expect(
        screen.queryByRole("button", { name: "Split active tab" }),
      ).toBeNull();
      expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
      expect(
        screen.getByRole("button", { name: "Close Chat Alpha" }),
      ).toBeInTheDocument();
    },
  );

  it("renders an actual task pane without floating split controls", () => {
    const store = setUpStore({
      workspace: {
        tabs: [task("task-a")],
        activeTabId: task("task-a"),
        groups: {},
      },
    });
    renderWorkspaceView(store);

    expect(
      document.querySelector(`[data-surface-key="${task("task-a")}"]`),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Workspace pane controls"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
  });

  it("leaves the rail and dock to the shared workspace shell", () => {
    render(<WorkspaceView />, { store: createWorkspaceStore() });

    expectSurface(chat("chat-a"));
    expect(
      screen.queryByRole("navigation", { name: "Workspace sections" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Workspace dock")).not.toBeInTheDocument();
  });

  it("renders a single rail inside the shared workspace shell", () => {
    renderWorkspaceView(createWorkspaceStore());

    expect(
      screen.getAllByRole("navigation", { name: "Workspace sections" }),
    ).toHaveLength(1);
    expect(screen.getAllByLabelText("Workspace dock")).toHaveLength(1);
  });

  it("keeps the Files tree mounted in the dock while a chat is active", async () => {
    const store = createWorkspaceStore();
    renderWorkspaceView(store);

    const mountedPanel = await screen.findByLabelText("File explorer");
    expect(mountedPanel.closest('[aria-label="Workspace dock"]')).toBeVisible();

    store.dispatch(setActiveTab(chat("chat-b")));
    await waitFor(() => {
      expect(screen.getByLabelText("File explorer")).toBe(mountedPanel);
    });
  });

  it("reconciles current thread to the active workspace chat on entry", async () => {
    const store = createWorkspaceStore();
    store.dispatch(
      createChatWithId({
        id: "task-hidden",
        title: "Task Hidden",
        openTab: false,
      }),
    );
    store.dispatch(switchToThread({ id: "task-hidden", openTab: false }));

    renderWorkspaceView(store);

    expectSurface(chat("chat-a"));
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-a"));
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(collectTabIds(getChatGroup(store).root)).toEqual([
      chat("chat-a"),
      chat("chat-b"),
    ]);
  });

  it("renders split panes with distinct surfaces and normal tab close controls", () => {
    renderWorkspaceView(createSplitWorkspaceStore());

    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
    expect(
      screen.getAllByRole("tablist", { name: "Workspace tabs" }),
    ).toHaveLength(1);
    expect(screen.queryByRole("tablist", { name: "Pane tabs" })).toBeNull();
    const row = screen.getByRole("tablist", { name: "Workspace tabs" });
    expect(within(row).getAllByRole("tab")).toHaveLength(2);
    expect(row.closest("[data-workspace-leaf-id]")).toBeNull();
    for (const pane of document.querySelectorAll("[data-workspace-leaf-id]")) {
      expect(
        within(pane as HTMLElement).queryByRole("tablist", {
          name: "Workspace tabs",
        }),
      ).toBeNull();
      expect(
        within(pane as HTMLElement).queryByRole("tablist", {
          name: "Pane tabs",
        }),
      ).toBeNull();
    }
  });

  it("renders single-leaf groups as global tabs and keeps the active panel", () => {
    const store = createWorkspaceStore();
    store.dispatch(
      addSurfaceToPane({
        tabId: chat("chat-a"),
        leafId: "root",
        surfaceKey: chat("chat-b"),
      }),
    );
    renderWorkspaceView(store);
    expect(
      screen.getByRole("tablist", { name: "Workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Chat Beta/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toBeVisible();
    expect(
      document.querySelector("[data-workspace-unsplit-drop-target]"),
    ).toBeNull();
  });

  it("uses a vertically scrollable stacked layout at narrow breakpoints", async () => {
    const clientWidthSpy = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(360);

    try {
      renderWorkspaceView(createSplitWorkspaceStore());

      await waitFor(() => {
        const container = document.querySelector<HTMLElement>(
          `[data-workspace-group-tab-id="${chat("chat-a")}"]`,
        );
        expect(container).toBeInTheDocument();
        expect(container).toHaveAttribute("data-breakpoint", "narrow");
        expect(container).toHaveAttribute("data-stacked", "true");
      });
      expectSurface(chat("chat-a"));
      expectSurface(chat("chat-b"));
      expect(screen.queryByTestId("workspace-vertical-divider")).toBeNull();
    } finally {
      clientWidthSpy.mockRestore();
    }
  });

  it("dropping a chat tab on an unsplit chat surface creates a split", () => {
    const store = createWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const dropTarget = document.querySelector(
      "[data-workspace-leaf-id='root']",
    );
    if (!dropTarget) throw new Error("missing unsplit drop target");

    dragToPane(dropTarget as HTMLElement, dataTransfer, "right");

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
  });

  it("accepts native file splits and preserves normal file tab closing", async () => {
    const store = createWorkspaceStore();
    store.dispatch(setPanelsForced(true));
    const fileA = "file:/project/a.ts";
    const fileB = "file:/project/b.ts";
    store.dispatch(openTab(fileA));
    store.dispatch(openTab(fileB));
    store.dispatch(setActiveTab(fileA));
    const view = renderWorkspaceView(store);
    await view.user.click(screen.getByRole("tab", { name: "a.ts" }));
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "file", "/project/b.ts", fileB);
    const dropTarget = document.querySelector(
      "[data-workspace-leaf-id='root']",
    );
    if (!dropTarget) throw new Error("missing file drop target");
    dragToPane(dropTarget as HTMLElement, dataTransfer, "right");
    expect(screen.getByRole("tab", { name: "a.ts" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(screen.getByRole("tab", { name: "b.ts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "a.ts" })).toHaveAttribute(
      "aria-controls",
      panePanelId("root", fileA),
    );
    expect(screen.getByRole("tab", { name: "a.ts" })).toHaveAttribute(
      "id",
      paneTabId("toolbar", fileA),
    );
    expectSurface(fileA);
    expectSurface(fileB);
    const group = store.getState().workspace.groups[chat("chat-a")];
    if (!group) throw new Error("missing file split");
    expect(collectTabIds(group.root)).toEqual([
      chat("chat-a"),
      chat("chat-b"),
      fileA,
      fileB,
    ]);
    const pane = screen.getByLabelText("Workspace pane root");
    const chatDrag = createDataTransferStub();
    setTabDragData(chatDrag, "chat", "chat-b", chat("chat-b"));
    dragToPane(pane, chatDrag, "right");
    const mixedGroup = store.getState().workspace.groups[chat("chat-a")];
    if (!mixedGroup) throw new Error("missing mixed group");
    expect(collectTabIds(mixedGroup.root)).toEqual([
      chat("chat-a"),
      fileA,
      chat("chat-b"),
      fileB,
    ]);
    await view.user.click(screen.getByRole("button", { name: "Close b.ts" }));
    const remainingGroup = store.getState().workspace.groups[chat("chat-a")];
    if (!remainingGroup) throw new Error("missing remaining group");
    expect(collectTabIds(remainingGroup.root)).not.toContain(fileB);
  });

  it("closing a normal tab retains the canonical singleton and closes the removed chat", async () => {
    const store = createSplitWorkspaceStore();
    const view = renderWorkspaceView(store);
    expect(store.getState().chat.open_thread_ids).toEqual(["chat-a", "chat-b"]);
    expect(store.getState().chat.threads["chat-b"]).toBeDefined();

    await view.user.click(
      screen.getByRole("button", { name: "Close Chat Beta" }),
    );

    expect(store.getState().workspace.groups[chat("chat-a")]?.root).toEqual({
      kind: "leaf",
      id: "root",
      tabIds: [chat("chat-a")],
      activeTabId: chat("chat-a"),
    });
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
    expect(store.getState().chat.threads["chat-a"]).toBeDefined();
    expect(store.getState().chat.threads["chat-b"]).toBeUndefined();
    await waitFor(() =>
      expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(
        1,
      ),
    );
    expectSurface(chat("chat-a"));
  });

  it("dropping a workspace tab on a pane edge creates a split", () => {
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const targetPane = screen.getByLabelText("Workspace pane root");

    dragToPane(targetPane, dataTransfer, "right");

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
  });

  it("dropping a workspace tab on an empty pane fills the pane", () => {
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const emptyPane = screen.getByLabelText(
      "Workspace pane root:sibling:chat:chat-a",
    );

    dragToPane(emptyPane, dataTransfer, "center");

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    const leafIds = collectLeafIds(group.root);
    expect(leafIds).toHaveLength(2);
    expect(
      leafIds.map((leafId) => findLeaf(group.root, leafId)?.tabIds.length),
    ).toEqual([1, 1]);
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
  });

  it("dropping a workspace tab on an occupied pane center joins local tabs", () => {
    const store = createSplitWorkspaceStore();
    store.dispatch(createChatWithId({ id: "chat-c", title: "Chat Gamma" }));
    store.dispatch(openTab(chat("chat-c")));
    store.dispatch(setActiveTab(chat("chat-a")));
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-c", chat("chat-c"));
    const occupiedPane = screen.getByLabelText("Workspace pane root");

    dragToPane(occupiedPane, dataTransfer, "center");

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    const leafIds = collectLeafIds(group.root);
    expect(leafIds).toHaveLength(2);
    expect(
      leafIds.every((leafId) => {
        const leaf = findLeaf(group.root, leafId);
        return Boolean(leaf && leaf.tabIds.length <= 2);
      }),
    ).toBe(true);
    expect(collectTabIds(group.root).sort()).toEqual(
      [chat("chat-a"), chat("chat-b"), chat("chat-c")].sort(),
    );
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expectSurface(chat("chat-c"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
    expect(findLeaf(group.root, "root")?.tabIds).toEqual([
      chat("chat-a"),
      chat("chat-c"),
    ]);
  });

  it("keeps split sizes unchanged for non-finite resize input", async () => {
    const clientWidthSpy = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(1024);
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);

    try {
      const divider = await screen.findByTestId("workspace-vertical-divider");
      const split = document.querySelector<HTMLElement>(
        "[data-pane-split-id='root:split:row']",
      );
      if (!split) throw new Error("missing split");
      const groupBefore = store.getState().workspace.groups[chat("chat-a")];
      if (!groupBefore || groupBefore.root.kind !== "split") {
        throw new Error("missing split group");
      }
      const sizesBefore = [...groupBefore.root.sizes];
      const rectSpy = vi.spyOn(split, "getBoundingClientRect").mockReturnValue({
        x: Number.POSITIVE_INFINITY,
        y: 0,
        width: 1024,
        height: 768,
        top: 0,
        right: Number.POSITIVE_INFINITY,
        bottom: 768,
        left: Number.POSITIVE_INFINITY,
        toJSON: () => ({}),
      });

      try {
        fireEvent.mouseDown(divider);
        fireEvent.mouseMove(window, { clientX: 500 });
        fireEvent.mouseUp(window);
      } finally {
        rectSpy.mockRestore();
      }

      const groupAfter = store.getState().workspace.groups[chat("chat-a")];
      expect(groupAfter?.root.kind).toBe("split");
      if (!groupAfter || groupAfter.root.kind !== "split") {
        throw new Error("missing split group");
      }
      expect(groupAfter.root.sizes).toEqual(sizesBefore);
    } finally {
      clientWidthSpy.mockRestore();
    }
  });
});
