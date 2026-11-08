import { createElement } from "react";
import { act } from "react-dom/test-utils";
import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  projectPaneTabGeometry,
  usePaneTabGeometry,
} from "./usePaneTabGeometry";

const slot = { left: 36, right: 1167, top: 0, bottom: 36 };
const pane = (id: string, left: number, right: number, top = 36) => ({
  id,
  left,
  right,
  top,
  bottom: 800,
});

describe("projectPaneTabGeometry", () => {
  it("preserves dock offsets and clips only the last controls gutter", () => {
    expect(
      projectPaneTabGeometry(slot, [pane("a", 332, 881), pane("b", 881, 1430)]),
    ).toEqual([
      { id: "a", left: 296, width: 549 },
      { id: "b", left: 845, width: 286 },
    ]);
  });
  it("sorts three actual pane bounds without rounding fractional dividers", () => {
    expect(
      projectPaneTabGeometry(slot, [
        pane("c", 900.5, 1300),
        pane("a", 332, 600.25),
        pane("b", 600.25, 900.5),
      ]),
    ).toEqual([
      { id: "a", left: 296, width: 268.25 },
      { id: "b", left: 564.25, width: 300.25 },
      { id: "c", left: 864.5, width: 266.5 },
    ]);
  });
  it("clamps a pane extending before the slot", () => {
    expect(projectPaneTabGeometry(slot, [pane("a", 0, 1300)])).toEqual([
      { id: "a", left: 0, width: 1131 },
    ]);
  });
  it.each([
    [],
    [pane("a", 332, 900), pane("b", 800, 1300)],
    [pane("a", 332, 332)],
  ])("rejects invalid or overlapping top-row bounds %j", (...panes) => {
    expect(projectPaneTabGeometry(slot, panes)).toBeNull();
  });

  it("projects only the upper pane of a vertical split", () => {
    expect(
      projectPaneTabGeometry(slot, [
        pane("a", 332, 900),
        pane("b", 332, 900, 400),
      ]),
    ).toEqual([{ id: "a", left: 296, width: 568 }]);
  });

  it.each([1140, 1200])(
    "retains the first pane when the next is clipped at %s and needs a local header",
    (boundary) => {
      expect(
        projectPaneTabGeometry(slot, [
          pane("a", 332, boundary),
          pane("b", boundary, 1300),
        ]),
      ).toEqual([
        { id: "a", left: 296, width: Math.min(boundary, slot.right) - 332 },
      ]);
    },
  );
});

describe("usePaneTabGeometry lifecycle", () => {
  let frames: Map<number, FrameRequestCallback>;
  let nextFrame: number;
  let observer: {
    notify: () => void;
    observe: ReturnType<typeof vi.fn>;
    disconnect: ReturnType<typeof vi.fn>;
  };
  let latest: ReturnType<typeof usePaneTabGeometry>;
  let roots: HTMLElement[];

  function Harness() {
    latest = usePaneTabGeometry("group", '["a","b"]');
    return createElement("div", { ref: latest.slotRef });
  }

  function bounds(
    element: HTMLElement | null,
    left: number,
    right: number,
    top = 36,
  ) {
    if (!element) throw new Error("Missing geometry element");
    return vi
      .spyOn(element, "getBoundingClientRect")
      .mockReturnValue(new DOMRect(left, top, right - left, 800 - top));
  }

  function mountGroup(left = 332) {
    const root = document.createElement("div");
    root.dataset.workspaceGroupTabId = "group";
    const panes = ["a", "b"].map((id, index) => {
      const element = document.createElement("div");
      element.dataset.workspaceLeafId = id;
      root.append(element);
      bounds(element, left + index * 300, left + (index + 1) * 300);
      return element;
    });
    document.body.append(root);
    roots.push(root);
    return { root, panes };
  }

  function flushFrame() {
    act(() => {
      const pending = [...frames.values()];
      frames.clear();
      pending.forEach((callback) => callback(0));
    });
  }

  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    frames = new Map();
    nextFrame = 0;
    roots = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.set(++nextFrame, callback);
      return nextFrame;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe = vi.fn();
        disconnect = vi.fn();
        constructor(callback: () => void) {
          observer = {
            notify: callback,
            observe: this.observe,
            disconnect: this.disconnect,
          };
        }
      },
    );
  });

  afterEach(() => {
    cleanup();
    roots.forEach((root) => root.remove());
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("recovers when the group mounts after the first frame and cancels missing-DOM retries", () => {
    const view = render(createElement(Harness));
    bounds(latest.slotRef.current, slot.left, slot.right, 0);
    flushFrame();
    expect(latest.sections).toBeNull();
    expect(vi.getTimerCount()).toBe(1);
    act(() => {
      vi.advanceTimersByTime(100);
    });
    flushFrame();
    expect(latest.sections).toBeNull();
    expect(vi.getTimerCount()).toBe(1);

    const { root, panes } = mountGroup();
    act(() => {
      vi.advanceTimersByTime(100);
    });
    flushFrame();
    expect(latest.sections).toEqual([
      { id: "a", left: 296, width: 300 },
      { id: "b", left: 596, width: 300 },
    ]);
    expect(observer.observe).toHaveBeenCalledWith(root);
    panes.forEach((pane) =>
      expect(observer.observe).toHaveBeenCalledWith(pane),
    );
    expect(vi.getTimerCount()).toBe(0);

    root.remove();
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    flushFrame();
    expect(vi.getTimerCount()).toBe(1);
    view.unmount();
    expect(observer.disconnect).toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(frames.size).toBe(0);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(frames.size).toBe(0);
  });

  it("reprojects after pane animations settle without a resize and removes listeners", () => {
    const { root, panes } = mountGroup();
    const moving = bounds(panes[1], 632, 932, 42);
    const view = render(createElement(Harness));
    bounds(latest.slotRef.current, slot.left, slot.right, 0);
    flushFrame();
    expect(latest.sections).toEqual([{ id: "a", left: 296, width: 300 }]);
    expect(frames.size).toBe(0);

    moving.mockReturnValue(new DOMRect(632, 36, 300, 764));
    const content = document.createElement("div");
    panes[1].append(content);
    act(() => {
      content.dispatchEvent(new Event("animationend", { bubbles: true }));
      root.dispatchEvent(new Event("animationend", { bubbles: true }));
    });
    expect(frames.size).toBe(0);
    act(() => {
      panes[1].dispatchEvent(new Event("animationend", { bubbles: true }));
    });
    expect(frames.size).toBe(1);
    flushFrame();
    expect(latest.sections).toEqual([
      { id: "a", left: 296, width: 300 },
      { id: "b", left: 596, width: 300 },
    ]);
    act(() => {
      panes[1].dispatchEvent(new Event("animationcancel", { bubbles: true }));
    });
    expect(frames.size).toBe(1);
    view.unmount();
    expect(frames.size).toBe(0);
    act(() => {
      panes[1].dispatchEvent(new Event("animationend", { bubbles: true }));
      panes[1].dispatchEvent(new Event("animationcancel", { bubbles: true }));
    });
    expect(frames.size).toBe(0);
  });

  it("rebinds a replaced root with unchanged leaf IDs and cancels queued measurements", () => {
    const original = mountGroup();
    const view = render(createElement(Harness));
    const slotBounds = bounds(latest.slotRef.current, slot.left, slot.right, 0);
    flushFrame();
    expect(latest.sections?.[0].left).toBe(296);
    const queries = vi.spyOn(document, "querySelectorAll");
    view.rerender(createElement(Harness));
    expect(queries).not.toHaveBeenCalled();
    expect(slotBounds).toHaveBeenCalledTimes(1);

    original.root.remove();
    const replacement = mountGroup(400);
    act(() => {
      observer.notify();
      observer.notify();
    });
    expect(frames.size).toBe(1);
    flushFrame();
    expect(latest.sections).toEqual([
      { id: "a", left: 364, width: 300 },
      { id: "b", left: 664, width: 300 },
    ]);
    expect(queries).toHaveBeenCalledTimes(1);
    expect(observer.disconnect).toHaveBeenCalledTimes(2);
    expect(observer.observe).toHaveBeenCalledWith(replacement.root);
    replacement.panes.forEach((pane) =>
      expect(observer.observe).toHaveBeenCalledWith(pane),
    );
    act(() => observer.notify());
    expect(frames.size).toBe(1);
    view.unmount();
    expect(observer.disconnect).toHaveBeenCalledTimes(3);
    expect(frames.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(frames.size).toBe(0);
  });
});
