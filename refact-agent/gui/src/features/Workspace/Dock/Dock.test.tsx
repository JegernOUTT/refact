import { readFileSync } from "node:fs";

import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { DOCK_MAX_WIDTH, setDockOpen, setDockSection } from "../workspaceSlice";
import { updateConfig } from "../../Config/configSlice";
import { Dock } from "./Dock";
import sheetStyles from "../../../components/ui/Sheet/Sheet.module.css";
import dockStyles from "./Dock.module.css";
import type { BackgroundAgentSummary } from "../../../services/refact";
import { openTab } from "../workspaceSlice";

const originalMatchMedia = window.matchMedia;
const dockCss = readFileSync(
  "src/features/Workspace/Dock/Dock.module.css",
  "utf8",
);
const toolbarCss = readFileSync(
  "src/components/Toolbar/Toolbar.module.css",
  "utf8",
);
const sheetCss = readFileSync(
  "src/components/ui/Sheet/Sheet.module.css",
  "utf8",
);
const tokensCss = readFileSync("src/styles/tokens.css", "utf8");

function cssBlock(css: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${escapedSelector} \\{[^}]*\\}`).exec(css)?.[0] ?? "";
}

function tokenPixels(name: string): number {
  const value = new RegExp(`${name}:\\s*(\\d+)px`).exec(tokensCss)?.[1];
  return Number(value);
}

function mockNarrow(narrow: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(
      (query: string): MediaQueryList => ({
        matches: narrow && query === "(max-width: 767px)",
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }),
    ),
  });
}

function stubFileTree() {
  server.use(
    http.get("*/v1/files/tree", () =>
      HttpResponse.json({ path: "", entries: [], truncated: false }),
    ),
  );
}

describe("Dock", () => {
  it("opens a discovered unloaded child through the actual dock handler and keeps denied links in place", async () => {
    mockNarrow(false);
    const chat = createDefaultChatState();
    const parentId = chat.current_thread_id;
    const childId = "unloaded-dock-child";
    const deniedId = "denied-dock-child";
    const summary: BackgroundAgentSummary = {
      agent_id: "agent-dock",
      parent_chat_id: parentId,
      child_chat_id: childId,
      kind: "subagent",
      status: "running",
      title: "Unloaded child",
      progress: null,
      step_count: 0,
      last_activity: null,
      target_files: [],
      edited_files: [],
      diff_summary: null,
      conflict_summary: null,
      result_summary: null,
      error: null,
      started_at: null,
      finished_at: null,
      change_seq: 1,
    };
    let reads = 0;
    let releaseMetadata!: () => void;
    const metadataResponse = new Promise<void>((resolve) => {
      releaseMetadata = resolve;
    });
    const commands = vi.fn();
    server.use(
      http.get("*/v1/background-agents", () =>
        HttpResponse.json([
          summary,
          {
            ...summary,
            agent_id: "agent-denied",
            child_chat_id: deniedId,
            title: "Denied child",
          },
        ]),
      ),
      http.get(`*/v1/trajectories/${childId}`, async ({ request }) => {
        reads += 1;
        expect(new URL(request.url).search).toBe("?metadata_only=true");
        expect(request.headers.get("authorization")).toBe("Bearer scoped-key");
        await metadataResponse;
        return HttpResponse.json({
          id: childId,
          title: "Stored child",
          model: "",
          mode: "AGENT",
          tool_use: "agent",
        });
      }),
      http.get(`*/v1/trajectories/${deniedId}`, () =>
        HttpResponse.json({ detail: "Access denied" }, { status: 403 }),
      ),
      http.post("*/v1/chats/:id/commands", () => {
        commands();
        return HttpResponse.json({});
      }),
    );
    const view = render(<Dock />, {
      preloadedState: {
        chat,
        config: {
          host: "web",
          dev: true,
          lspPort: 8001,
          apiKey: "scoped-key",
          themeProps: {},
        },
      },
    });
    view.store.dispatch(openTab(`chat:${parentId}`));
    view.store.dispatch(setDockSection("agents"));
    await view.user.click(
      await screen.findByRole("button", { name: "Denied child" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("Access denied");
    expect(view.store.getState().chat.current_thread_id).toBe(parentId);
    expect(view.store.getState().chat.threads[deniedId]).toBeUndefined();
    await view.user.click(
      screen.getByRole("button", { name: "Unloaded child" }),
    );
    await waitFor(() => expect(reads).toBe(1));
    expect(screen.queryByText("Opening linked target...")).toBeNull();
    expect(view.store.getState().chat.current_thread_id).toBe(parentId);
    releaseMetadata();
    await waitFor(() =>
      expect(view.store.getState().chat.current_thread_id).toBe(childId),
    );
    expect(view.store.getState().chat.threads[childId]).toMatchObject({
      thread: {
        title: "Stored child",
        parent_id: parentId,
        root_chat_id: parentId,
        link_type: "subagent",
      },
      snapshot_received: false,
    });
    expect(reads).toBe(1);
    expect(commands).not.toHaveBeenCalled();
  });

  beforeEach(() => {
    server.use(
      http.get("*/v1/git/status", () => HttpResponse.json({ roots: [] })),
    );
  });

  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
    vi.restoreAllMocks();
  });

  it("renders a single section body without an in-dock section switcher", () => {
    mockNarrow(false);
    stubFileTree();
    render(<Dock />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          themeProps: { appearance: "dark" },
          capabilities: {
            filesPanel: true,
            gitPanel: false,
          },
        },
      },
    });

    expect(screen.getByTestId("workspace-dock")).toBeInTheDocument();
    const section = screen.getByTestId("workspace-dock-section");
    expect(section).toHaveAttribute("data-section", "files");
    expect(section).toHaveClass("rf-enter");
    expect(screen.queryByRole("radio", { name: "Files" })).toBeNull();
    expect(screen.queryByRole("radio", { name: "Git" })).toBeNull();
    expect(screen.queryByRole("radio", { name: "Tasks" })).toBeNull();
    expect(screen.queryByRole("radiogroup")).toBeNull();
  });

  it("clamps a persisted resize width to the dock maximum", () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);
    const store = view.store;

    const dock = screen.getByTestId("workspace-dock");
    vi.spyOn(dock, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      width: 280,
      height: 600,
      top: 0,
      right: 280,
      bottom: 600,
      left: 0,
      toJSON: () => ({}),
    });
    const splitter = screen.getByRole("separator", {
      name: "Resize workspace dock",
    });
    fireEvent.pointerDown(splitter, { button: 0, clientX: 280 });
    fireEvent.pointerMove(window, { clientX: 900 });
    fireEvent.pointerUp(window, { clientX: 900 });

    expect(store.getState().workspace.dock?.width).toBe(DOCK_MAX_WIDTH);
  });

  it("clamps a persisted resize width to the dock minimum", () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);

    const dock = screen.getByTestId("workspace-dock");
    vi.spyOn(dock, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      width: 280,
      height: 600,
      top: 0,
      right: 280,
      bottom: 600,
      left: 0,
      toJSON: () => ({}),
    });
    const splitter = screen.getByRole("separator", {
      name: "Resize workspace dock",
    });
    fireEvent.pointerDown(splitter, { button: 0, clientX: 280 });
    fireEvent.pointerMove(window, { clientX: 0 });
    fireEvent.pointerUp(window, { clientX: 0 });

    expect(view.store.getState().workspace.dock?.width).toBe(240);
  });

  it("unmounts the wide dock once the collapse animation settles", async () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);

    expect(screen.getByTestId("workspace-dock")).toBeInTheDocument();
    view.store.dispatch(setDockOpen(false));

    await waitFor(() => {
      expect(screen.queryByTestId("workspace-dock")).toBeNull();
    });
  });

  it("renders the requested section body for each dock section", async () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);

    expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
      "data-section",
      "files",
    );

    view.store.dispatch(setDockSection("git"));
    await waitFor(() => {
      expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
        "data-section",
        "git",
      );
    });
    expect(
      await screen.findByText("No git repository found in this workspace."),
    ).toBeInTheDocument();

    view.store.dispatch(setDockSection("agents"));
    await waitFor(() => {
      expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
        "data-section",
        "agents",
      );
    });

    view.store.dispatch(setDockSection("activity"));
    await waitFor(() => {
      expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
        "data-section",
        "activity",
      );
    });
  });

  it("remounts the section body so each switch replays the enter animation", async () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);

    const filesSection = screen.getByTestId("workspace-dock-section");
    view.store.dispatch(setDockSection("git"));

    await waitFor(() => {
      const gitSection = screen.getByTestId("workspace-dock-section");
      expect(gitSection).not.toBe(filesSection);
      expect(gitSection).toHaveClass("rf-enter");
    });
  });

  it("falls back to the first available section when the stored one is gone", async () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          themeProps: { appearance: "dark" },
          capabilities: { filesPanel: false, gitPanel: true },
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
        "data-section",
        "git",
      );
    });
    expect(view.store.getState().workspace.dock?.section).toBe("git");
  });

  it("keeps agents and active chats available with no panel capabilities", async () => {
    mockNarrow(false);
    stubFileTree();
    const view = render(<Dock />);
    view.store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );

    await waitFor(() => {
      expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
        "data-section",
        "agents",
      );
    });
  });

  it("uses a Sheet on narrow viewports and follows open state", async () => {
    mockNarrow(true);
    stubFileTree();
    const view = render(<Dock />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    view.store.dispatch(setDockOpen(false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("offers a visible close control inside the narrow Sheet (audit L-01)", async () => {
    mockNarrow(true);
    stubFileTree();
    const view = render(<Dock />);

    const close = screen.getByRole("button", {
      name: "Close workspace panel",
    });
    fireEvent.click(close);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(view.store.getState().workspace.dock?.open).toBe(false);
  });

  it("keeps the narrow Sheet below the fixed toolbar without blocking toolbar input", () => {
    const sheetBlock = cssBlock(dockCss, ".sheet");
    const toolbarBlock = cssBlock(toolbarCss, ".toolbar");
    const toolbarHeight = tokenPixels("--rf-control-h-lg");
    const viewportInset = tokenPixels("--rf-space-3");

    expect(toolbarBlock).toContain("height: var(--rf-control-h-lg)");
    expect(sheetBlock).toContain(
      "top: calc(var(--rf-control-h-lg) + var(--rf-space-3))",
    );
    expect(sheetBlock).toContain(
      "100dvh - var(--rf-control-h-lg) - var(--rf-space-5)",
    );
    expect(sheetCss).toMatch(
      /\.left,\s*\.right\s*\{[\s\S]*?width:\s*min\([\s\S]*?calc\(100vw - 2 \* var\(--rf-space-3\)\)/,
    );
    for (const viewportWidth of [360, 480, 640]) {
      expect(toolbarHeight + viewportInset).toBeGreaterThan(toolbarHeight);
      expect(viewportWidth - 2 * viewportInset).toBeLessThan(viewportWidth);
    }

    mockNarrow(true);
    stubFileTree();
    render(<Dock />);

    expect(screen.getByRole("dialog")).toHaveClass(dockStyles.sheet);
    expect(document.querySelector(`.${sheetStyles.overlay}`)).toBeNull();
    expect(document.body.style.pointerEvents).not.toBe("none");
  });
});
