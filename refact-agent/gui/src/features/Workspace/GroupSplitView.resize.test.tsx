import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { GroupSplitView } from "./GroupSplitView";
import type { SplitNode } from "../ChatPanes/panesTree";

vi.hoisted(() => vi.resetModules());
vi.mock("./SurfacePane", () => ({
  SurfacePane: () => <iframe title="App preview" />,
}));
vi.mock("../Dashboard/hooks/useDashboardLayout", () => ({
  useDashboardLayout: () => "wide",
}));

afterEach(() => vi.restoreAllMocks());

function setup(dir: "row" | "col" = "row") {
  const initial = setUpStore().getState();
  const root: SplitNode = {
    kind: "split",
    id: "split",
    dir,
    sizes: [0.5, 0.5],
    children: [
      { kind: "leaf", id: "a", tabIds: ["file:/a"], activeTabId: "file:/a" },
      { kind: "leaf", id: "b", tabIds: ["file:/b"], activeTabId: "file:/b" },
    ],
  };
  const store = setUpStore({
    ...initial,
    workspace: {
      ...initial.workspace,
      tabs: ["file:/a"],
      activeTabId: "file:/a",
      groups: { "file:/a": { root, focusedLeafId: "a" } },
    },
  });
  const view = render(<GroupSplitView tabId="file:/a" />, { store });
  const divider = screen.getByRole("separator", {
    name: "Resize workspace panes",
  });
  const split = view.container.querySelector<HTMLElement>(
    "[data-pane-split-id]",
  );
  if (!split) throw new Error("Missing split");
  vi.spyOn(split, "getBoundingClientRect").mockReturnValue({
    left: 100,
    top: 50,
    width: 1000,
    height: 500,
  } as DOMRect);
  let captured: number | null = null;
  const capture = vi.fn((id: number) => {
    captured = id;
  });
  const release = vi.fn(() => {
    captured = null;
  });
  Object.assign(divider, {
    setPointerCapture: capture,
    hasPointerCapture: (id: number) => captured === id,
    releasePointerCapture: release,
  });
  const sizes = () => {
    const node = store.getState().workspace.groups["file:/a"]?.root;
    if (node?.kind !== "split") throw new Error("Missing state split");
    return node.sizes;
  };
  return { ...view, divider, split, sizes, capture, release };
}

function pointer(
  target: HTMLElement,
  type: string,
  pointerId = 7,
  clientX = 800,
  clientY = 400,
  button = 0,
) {
  fireEvent(
    target,
    new PointerEvent(type, {
      bubbles: true,
      pointerId,
      clientX,
      clientY,
      button,
    }),
  );
}

describe("workspace divider resizing", () => {
  it("captures the pointer and dispatches actual width changes while an app iframe is present, then stops on release", () => {
    const { divider, split, sizes, capture, release } = setup();
    expect(screen.getAllByTitle("App preview")).toHaveLength(2);
    const originalCursor = document.body.style.cursor;
    const originalSelect = document.body.style.userSelect;
    pointer(divider, "pointerdown");
    expect(capture).toHaveBeenCalledWith(7);
    expect(split).toHaveAttribute("data-resizing", "true");
    expect(divider).toHaveFocus();
    pointer(divider, "pointermove", 9);
    expect(sizes()).toEqual([0.5, 0.5]);
    pointer(divider, "pointermove");
    expect(sizes()[0]).toBeCloseTo(0.7);
    expect(sizes()[1]).toBeCloseTo(0.3);
    pointer(divider, "pointermove", 7, 2000);
    expect(sizes()[0]).toBeCloseTo(0.88);
    pointer(divider, "pointerup");
    expect(release).toHaveBeenCalledOnce();
    expect(split).not.toHaveAttribute("data-resizing");
    pointer(divider, "pointermove", 7, 300);
    expect(sizes()[0]).toBeCloseTo(0.88);
    expect(document.body.style.cursor).toBe(originalCursor);
    expect(document.body.style.userSelect).toBe(originalSelect);
  });

  it("resizes height and cleans up cancel, capture loss, blur and unmount without leaving an active session", () => {
    const { divider, split, sizes, release, unmount } = setup("col");
    for (const end of [
      "pointercancel",
      "lostpointercapture",
      "blur",
      "Escape",
    ]) {
      pointer(divider, "pointerdown");
      pointer(divider, "pointermove");
      expect(sizes()[0]).toBeCloseTo(0.7);
      if (end === "blur") fireEvent.blur(window);
      else if (end === "Escape") fireEvent.keyDown(divider, { key: end });
      else pointer(divider, end);
      expect(split).not.toHaveAttribute("data-resizing");
      pointer(divider, "pointermove", 7, 800, 150);
      expect(sizes()[0]).toBeCloseTo(0.7);
    }
    pointer(divider, "pointerdown");
    unmount();
    expect(release).toHaveBeenCalledTimes(5);
    expect(split).not.toBeInTheDocument();
  });

  it("ignores non-primary buttons and exposes bounded axis-appropriate keyboard resizing", () => {
    const { divider, split, sizes, capture } = setup();
    pointer(divider, "pointerdown", 7, 800, 400, 2);
    expect(capture).not.toHaveBeenCalled();
    expect(split).not.toHaveAttribute("data-resizing");
    expect(divider).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(divider, { key: "ArrowDown" });
    expect(sizes()).toEqual([0.5, 0.5]);
    fireEvent.keyDown(divider, { key: "ArrowRight" });
    expect(sizes()[0]).toBeCloseTo(0.52);
    expect(divider).toHaveAttribute("aria-valuenow", "52");
    fireEvent.keyDown(divider, { key: "Home" });
    expect(sizes()[0]).toBeCloseTo(0.12);
    fireEvent.keyDown(divider, { key: "End" });
    expect(sizes()[0]).toBeCloseTo(0.88);
  });
});
