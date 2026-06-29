import { afterEach, describe, expect, it, vi } from "vitest";

import {
  beginPointerDragGesture,
  isPointerDragHost,
  pointInRect,
  pointerDragController,
  resolveActiveZoneId,
  type ActivePointerDrag,
  type Rect,
} from "./pointerDrag";
import type { TabDragPayload } from "./tabDrag";

const rect = (
  left: number,
  top: number,
  width: number,
  height: number,
): Rect => ({
  left,
  top,
  right: left + width,
  bottom: top + height,
  width,
  height,
});

const chatPayload: TabDragPayload = {
  type: "chat",
  id: "a",
  surfaceKey: "chat:a",
};

const cleanups: (() => void)[] = [];

function registerZone(
  id: string,
  zoneRect: Rect,
  onDrop: (payload: TabDragPayload) => void,
  options: {
    accepts?: (payload: TabDragPayload) => boolean;
    setHover?: (over: boolean) => void;
  } = {},
) {
  const unregister = pointerDragController.registerZone({
    id,
    getRect: () => zoneRect,
    accepts: options.accepts ?? (() => true),
    onDrop,
    setHover: options.setHover ?? (() => undefined),
  });
  cleanups.push(unregister);
  return unregister;
}

function firePointer(
  type: "pointermove" | "pointerup",
  { clientX = 0, clientY = 0, pointerId = 1, button = 0 } = {},
): void {
  const event = new MouseEvent(type, { clientX, clientY, button });
  Object.defineProperty(event, "pointerId", {
    value: pointerId,
    configurable: true,
  });
  window.dispatchEvent(event);
}

afterEach(() => {
  pointerDragController.cancel();
  while (cleanups.length) cleanups.pop()?.();
});

describe("pointInRect", () => {
  it("includes the boundary and rejects points outside", () => {
    const r = rect(10, 10, 100, 100);
    expect(pointInRect({ x: 10, y: 10 }, r)).toBe(true);
    expect(pointInRect({ x: 110, y: 110 }, r)).toBe(true);
    expect(pointInRect({ x: 60, y: 60 }, r)).toBe(true);
    expect(pointInRect({ x: 9, y: 60 }, r)).toBe(false);
    expect(pointInRect({ x: 60, y: 111 }, r)).toBe(false);
    expect(pointInRect({ x: 0, y: 0 }, null)).toBe(false);
  });
});

describe("resolveActiveZoneId", () => {
  it("prefers the smallest accepting zone under the point (edge over pane)", () => {
    const zones = [
      { id: "pane", rect: rect(0, 0, 200, 200), accepts: true },
      { id: "edge", rect: rect(0, 0, 20, 200), accepts: true },
    ];
    expect(resolveActiveZoneId(zones, { x: 10, y: 100 })).toBe("edge");
    expect(resolveActiveZoneId(zones, { x: 100, y: 100 })).toBe("pane");
  });

  it("ignores zones that do not accept or do not contain the point", () => {
    const zones = [
      { id: "edge", rect: rect(0, 0, 20, 200), accepts: false },
      { id: "pane", rect: rect(0, 0, 200, 200), accepts: true },
    ];
    expect(resolveActiveZoneId(zones, { x: 10, y: 100 })).toBe("pane");
    expect(resolveActiveZoneId(zones, { x: 500, y: 500 })).toBeNull();
  });
});

describe("isPointerDragHost", () => {
  it("is true for JCEF hosts only", () => {
    expect(isPointerDragHost("jetbrains")).toBe(true);
    expect(isPointerDragHost("ide")).toBe(true);
    expect(isPointerDragHost("web")).toBe(false);
    expect(isPointerDragHost("vscode")).toBe(false);
  });
});

describe("pointerDragController", () => {
  it("drops on the innermost accepting zone and clears dragging state", () => {
    const paneDrop = vi.fn();
    const edgeDrop = vi.fn();
    registerZone("pane", rect(0, 0, 200, 200), paneDrop);
    registerZone("edge", rect(0, 0, 20, 200), edgeDrop);

    pointerDragController.startDrag(
      { payload: chatPayload },
      { x: 10, y: 100 },
    );
    expect(pointerDragController.isDragging()).toBe(true);

    firePointer("pointermove", { clientX: 10, clientY: 100 });
    firePointer("pointerup", { clientX: 10, clientY: 100 });

    expect(edgeDrop).toHaveBeenCalledWith(chatPayload);
    expect(paneDrop).not.toHaveBeenCalled();
    expect(pointerDragController.isDragging()).toBe(false);
  });

  it("drops on the pane center when the pointer is away from edges", () => {
    const paneDrop = vi.fn();
    const edgeDrop = vi.fn();
    registerZone("pane", rect(0, 0, 200, 200), paneDrop);
    registerZone("edge", rect(0, 0, 20, 200), edgeDrop);

    pointerDragController.startDrag(
      { payload: chatPayload },
      { x: 100, y: 100 },
    );
    firePointer("pointerup", { clientX: 100, clientY: 100 });

    expect(paneDrop).toHaveBeenCalledWith(chatPayload);
    expect(edgeDrop).not.toHaveBeenCalled();
  });

  it("toggles hover for accepting zones as the pointer moves", () => {
    const setHover = vi.fn();
    registerZone("pane", rect(0, 0, 200, 200), vi.fn(), { setHover });

    pointerDragController.startDrag({ payload: chatPayload }, { x: 10, y: 10 });
    expect(setHover).toHaveBeenLastCalledWith(true);

    firePointer("pointermove", { clientX: 500, clientY: 500 });
    expect(setHover).toHaveBeenLastCalledWith(false);
  });

  it("does not drop when no zone is under the pointer", () => {
    const onDrop = vi.fn();
    registerZone("pane", rect(0, 0, 50, 50), onDrop);

    pointerDragController.startDrag({ payload: chatPayload }, { x: 10, y: 10 });
    firePointer("pointerup", { clientX: 500, clientY: 500 });

    expect(onDrop).not.toHaveBeenCalled();
  });

  it("cancels the drag on Escape without dropping", () => {
    const onDrop = vi.fn();
    registerZone("pane", rect(0, 0, 200, 200), onDrop);

    pointerDragController.startDrag({ payload: chatPayload }, { x: 10, y: 10 });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(pointerDragController.isDragging()).toBe(false);
    firePointer("pointerup", { clientX: 10, clientY: 10 });
    expect(onDrop).not.toHaveBeenCalled();
  });

  it("publishes snapshot label while dragging", () => {
    const drag: ActivePointerDrag = { payload: chatPayload, label: "Chat A" };
    pointerDragController.startDrag(drag, { x: 5, y: 5 });

    const snapshot = pointerDragController.getSnapshot();
    expect(snapshot.active).toBe(true);
    expect(snapshot.label).toBe("Chat A");

    pointerDragController.cancel();
    expect(pointerDragController.getSnapshot().active).toBe(false);
  });
});

describe("beginPointerDragGesture", () => {
  it("starts a controller drag once the pointer passes the threshold", () => {
    const resolveDrag = vi.fn(
      (): ActivePointerDrag => ({ payload: chatPayload, label: "Chat A" }),
    );
    const onStarted = vi.fn();

    beginPointerDragGesture(
      { button: 0, clientX: 0, clientY: 0, pointerId: 1 },
      resolveDrag,
      onStarted,
    );

    firePointer("pointermove", { clientX: 2, clientY: 2, pointerId: 1 });
    expect(pointerDragController.isDragging()).toBe(false);
    expect(onStarted).not.toHaveBeenCalled();

    firePointer("pointermove", { clientX: 12, clientY: 0, pointerId: 1 });
    expect(pointerDragController.isDragging()).toBe(true);
    expect(onStarted).toHaveBeenCalledTimes(1);
    expect(resolveDrag).toHaveBeenCalledTimes(1);
  });

  it("ignores non-primary buttons", () => {
    const resolveDrag = vi.fn(
      (): ActivePointerDrag => ({ payload: chatPayload }),
    );
    beginPointerDragGesture(
      { button: 2, clientX: 0, clientY: 0, pointerId: 1 },
      resolveDrag,
    );

    firePointer("pointermove", { clientX: 50, clientY: 50, pointerId: 1 });
    expect(pointerDragController.isDragging()).toBe(false);
    expect(resolveDrag).not.toHaveBeenCalled();
  });
});
