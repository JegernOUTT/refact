import { readFileSync } from "node:fs";

import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setDockOpen } from "../workspaceSlice";
import { Dock } from "./Dock";
import badgeStyles from "../../../components/ui/Badge/Badge.module.css";
import sheetStyles from "../../../components/ui/Sheet/Sheet.module.css";
import dockStyles from "./Dock.module.css";

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

describe("Dock", () => {
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

  it("renders capability sections and clamps persisted resize width", () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    const view = render(<Dock />, {
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
    const store = view.store;

    expect(screen.getByRole("radio", { name: "Files" })).toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: "Git" })).toBeNull();
    expect(screen.getByRole("radio", { name: "Tasks" })).toBeInTheDocument();

    const dock = screen.getByTestId("workspace-dock");
    expect(dock).toHaveClass("rf-grow-in");
    expect(screen.getByTestId("workspace-dock-section")).toHaveClass(
      "rf-enter",
    );
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

    expect(store.getState().workspace.dock?.width).toBe(400);
  });

  it("uses a Sheet on narrow viewports and follows open state", async () => {
    mockNarrow(true);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    const view = render(<Dock />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    view.store.dispatch(setDockOpen(false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
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
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    render(<Dock />);

    expect(screen.getByRole("dialog")).toHaveClass(dockStyles.sheet);
    expect(document.querySelector(`.${sheetStyles.overlay}`)).toBeNull();
    expect(document.body.style.pointerEvents).not.toBe("none");
  });

  it("switches to the Git dock section", async () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
      http.get("*/v1/git/status", () => HttpResponse.json({ roots: [] })),
    );
    render(<Dock />);
    const filesSection = screen.getByTestId("workspace-dock-section");
    expect(filesSection).toHaveAttribute("data-section", "files");
    fireEvent.click(screen.getByRole("radio", { name: "Git" }));
    const gitSection = screen.getByTestId("workspace-dock-section");
    expect(gitSection).not.toBe(filesSection);
    expect(gitSection).toHaveClass("rf-enter");
    expect(gitSection).toHaveAttribute("data-section", "git");
    expect(
      await screen.findByText("No git repository found in this workspace."),
    ).toBeInTheDocument();
  });

  it("counts unique changed paths on the Git switcher entry", async () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
      http.get("*/v1/git/status", () =>
        HttpResponse.json({
          roots: [
            {
              root: "/repo",
              branch: "main",
              head_detached: false,
              ahead: 0,
              behind: 0,
              staged: [
                {
                  relative_path: "a",
                  absolute_path: "/repo/a",
                  status: "MODIFIED",
                },
              ],
              unstaged: [
                {
                  relative_path: "a",
                  absolute_path: "/repo/a",
                  status: "MODIFIED",
                },
                {
                  relative_path: "b",
                  absolute_path: "/repo/b",
                  status: "DELETED",
                },
              ],
              untracked_included: true,
            },
          ],
        }),
      ),
    );

    render(<Dock />);

    const badge = await screen.findByLabelText("2 changed files");
    expect(badge).toHaveTextContent("2");
    expect(badge).toHaveClass(badgeStyles.warning, badgeStyles["size-xs"]);
  });
});
