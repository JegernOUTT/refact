import { createPortal } from "react-dom";
import { act } from "react-dom/test-utils";
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { GroupSplitView } from "./GroupSplitView";
import { panePanelId } from "./paneTabIds";
import { usePaneHeaderTarget, usePaneSurfaceActive } from "./PaneHeaderContext";
import {
  addSurfaceToPane,
  openTab,
  setPaneActive,
  splitTab,
} from "./workspaceSlice";
const styles = readFileSync(
  "src/features/Workspace/GroupSplitView.module.css",
  "utf8",
);

vi.hoisted(() => vi.resetModules());
vi.mock("../Dashboard/hooks/useDashboardLayout", () => ({
  useDashboardLayout: () => "wide",
}));
vi.mock("./SurfacePane", () => ({
  SurfacePane: function Surface({ surfaceKey }: { surfaceKey: string }) {
    const target = usePaneHeaderTarget();
    const active = usePaneSurfaceActive();
    return (
      <>
        <input aria-label={`Draft ${surfaceKey}`} defaultValue="retained" />
        {target && active
          ? createPortal(<button>File controls {surfaceKey}</button>, target)
          : null}
      </>
    );
  },
}));

function setup() {
  const store = setUpStore();
  store.dispatch(openTab("file:/a.html"));
  const group = store.getState().workspace.groups["file:/a.html"];
  if (!group || group.root.kind !== "leaf") throw new Error("Missing leaf");
  const leafId = group.root.id;
  render(<GroupSplitView tabId="file:/a.html" />, { store });
  const pane = screen.getByLabelText(`Workspace pane ${leafId}`);
  return { store, pane, leafId };
}

describe("top-line-only pane controls", () => {
  it("animates only a newly introduced leaf, never the existing split tree", () => {
    const { store, pane } = setup();
    expect(pane).not.toHaveAttribute("data-pane-entering");
    act(() => {
      store.dispatch(
        splitTab({
          tabId: "file:/a.html",
          dir: "row",
          surfaceKey: "file:/b.html",
        }),
      );
    });
    const panes = document.querySelectorAll("[data-workspace-leaf-id]");
    expect(panes).toHaveLength(2);
    expect(panes[0]).not.toHaveAttribute("data-pane-entering");
    expect(panes[1]).toHaveAttribute("data-pane-entering", "true");
    expect(document.querySelectorAll(".rf-grow-in")).toHaveLength(0);
    expect(styles).toMatch(
      /@keyframes pane-enter[\s\S]*opacity:[\s\S]*transform:/,
    );
  });

  it("has no pane tablist or header and names panels even when toolbar tabs overflow", () => {
    const { pane, leafId } = setup();
    expect(within(pane).queryByRole("tablist")).not.toBeInTheDocument();
    expect(pane.querySelector("header")).toBeNull();
    const panel = within(pane).getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", panePanelId(leafId, "file:/a.html"));
    expect(panel).toHaveAttribute("aria-label", "a.html");
    expect(panel).not.toHaveAttribute("aria-labelledby");
    const controls = pane.querySelector("[data-pane-controls]");
    expect(controls).toContainElement(
      screen.getByText("File controls file:/a.html"),
    );
    expect(
      within(pane).queryByRole("button", {
        name: /Pane actions|Split right|Split down/,
      }),
    ).not.toBeInTheDocument();
    expect(within(pane).getAllByRole("button")).toEqual([
      screen.getByText("File controls file:/a.html"),
    ]);
  });

  it("portals only active controls while retaining visited panel drafts", () => {
    const { store, pane, leafId } = setup();
    fireEvent.change(screen.getByLabelText("Draft file:/a.html"), {
      target: { value: "unsaved" },
    });
    act(() => {
      store.dispatch(
        addSurfaceToPane({
          tabId: "file:/a.html",
          leafId,
          surfaceKey: "file:/b.html",
        }),
      );
    });
    expect(
      screen.queryByText("File controls file:/a.html"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("File controls file:/b.html")).toBeInTheDocument();
    const oldPanel = pane.querySelector(
      `#${CSS.escape(panePanelId(leafId, "file:/a.html"))}`,
    );
    expect(oldPanel).toHaveAttribute("hidden");
    act(() => {
      store.dispatch(
        setPaneActive({
          tabId: "file:/a.html",
          leafId,
          surfaceKey: "file:/a.html",
        }),
      );
    });
    expect(screen.getByLabelText("Draft file:/a.html")).toHaveValue("unsaved");
    expect(
      screen.queryByText("File controls file:/b.html"),
    ).not.toBeInTheDocument();
  });

  it("uses an out-of-flow bounded overlay revealed by hover, keyboard focus, or touch", () => {
    expect(styles).toMatch(/\.\w*paneControls[\s\S]*?position:\s*absolute/);
    expect(styles).toContain(":focus-within");
    expect(styles).toContain(":hover");
    expect(styles).toContain("(hover: none)");
    expect(styles).toContain("(pointer: coarse)");
    expect(styles).toMatch(/max-width:\s*calc\(100% - var\(--rf-space-6\)\)/);
    expect(styles).toMatch(/\.\w*previewBody[\s\S]*?inset:\s*0/);
  });
});
