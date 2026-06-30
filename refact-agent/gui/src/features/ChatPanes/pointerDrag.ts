import type { Config } from "../Config/configSlice";
import type { TabDragPayload } from "./tabDrag";

/**
 * Pointer-based drag-and-drop transport.
 *
 * The workspace tab/pane drag-and-drop is built on native HTML5 DnD, which is
 * suppressed inside the JetBrains plugin: its JCEF browser registers no
 * `CefDragHandler`, so `dragstart`/`dragover`/`drop` + `DataTransfer` never
 * deliver. Plain pointer events do work (the split-divider resize relies on
 * them), so for JCEF hosts we drive the same Redux actions through a pointer
 * controller instead.
 *
 * This module is framework-agnostic and side-effect free until a drag starts.
 * The React glue lives in `usePointerDrag.tsx`.
 */

export const POINTER_DRAG_THRESHOLD_PX = 5;

/** Hosts whose embedded browser cannot deliver HTML5 drag-and-drop. */
export function isPointerDragHost(host: Config["host"]): boolean {
  return host === "jetbrains" || host === "ide";
}

export type Point = { x: number; y: number };

export type Rect = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

export type ActivePointerDrag = {
  payload: TabDragPayload;
  label?: string;
};

export type PointerDragSnapshot = {
  active: boolean;
  point: Point;
  label: string | null;
};

export type DropZoneConfig = {
  id: string;
  getRect: () => Rect | null;
  accepts: (payload: TabDragPayload) => boolean;
  onDrop: (payload: TabDragPayload) => void;
  setHover: (over: boolean) => void;
};

type RegisteredZone = DropZoneConfig & { hovered: boolean };

export function rectFromElement(element: HTMLElement | null): Rect | null {
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  return {
    left: rect.left,
    right: rect.right,
    top: rect.top,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height,
  };
}

export function pointInRect(point: Point, rect: Rect | null): boolean {
  if (!rect) return false;
  return (
    point.x >= rect.left &&
    point.x <= rect.right &&
    point.y >= rect.top &&
    point.y <= rect.bottom
  );
}

/**
 * Resolve the innermost accepting drop zone under the pointer. "Innermost" is
 * approximated by smallest rect area, so an edge strip nested inside a pane
 * wins over the pane itself, while non-overlapping tabs resolve to whichever
 * one contains the point.
 */
export function resolveActiveZoneId(
  zones: readonly { id: string; rect: Rect | null; accepts: boolean }[],
  point: Point,
): string | null {
  let bestId: string | null = null;
  let bestArea = Number.POSITIVE_INFINITY;

  for (const zone of zones) {
    if (!zone.accepts) continue;
    if (!pointInRect(point, zone.rect)) continue;
    const area = zone.rect
      ? zone.rect.width * zone.rect.height
      : Number.POSITIVE_INFINITY;
    if (area < bestArea) {
      bestArea = area;
      bestId = zone.id;
    }
  }

  return bestId;
}

const EMPTY_SNAPSHOT: PointerDragSnapshot = {
  active: false,
  point: { x: 0, y: 0 },
  label: null,
};

class PointerDragController {
  private zones = new Map<string, RegisteredZone>();
  private drag: ActivePointerDrag | null = null;
  private point: Point = { x: 0, y: 0 };
  private activeZoneId: string | null = null;
  private snapshot: PointerDragSnapshot = EMPTY_SNAPSHOT;
  private subscribers = new Set<() => void>();

  registerZone(config: DropZoneConfig): () => void {
    const zone: RegisteredZone = { ...config, hovered: false };
    this.zones.set(config.id, zone);
    if (this.drag) this.updateHover();
    return () => {
      const existing = this.zones.get(config.id);
      if (existing?.hovered) existing.setHover(false);
      this.zones.delete(config.id);
      if (this.activeZoneId === config.id) this.activeZoneId = null;
    };
  }

  isDragging(): boolean {
    return this.drag !== null;
  }

  getSnapshot = (): PointerDragSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.subscribers.add(listener);
    return () => {
      this.subscribers.delete(listener);
    };
  };

  startDrag(drag: ActivePointerDrag, point: Point): void {
    if (this.drag) this.cancel();
    this.drag = drag;
    this.point = point;
    if (typeof document !== "undefined") {
      document.body.setAttribute("data-rf-dragging", "");
    }
    window.addEventListener("pointermove", this.handlePointerMove, true);
    window.addEventListener("pointerup", this.handlePointerUp, true);
    window.addEventListener("pointercancel", this.handlePointerCancel, true);
    window.addEventListener("keydown", this.handleKeyDown, true);
    // Safety net: if the pointer is released outside the (JCEF) window we may
    // never see pointerup; losing focus aborts the drag instead of wedging.
    window.addEventListener("blur", this.handleWindowBlur);
    this.updateHover();
    this.publish();
  }

  /** Abort the active drag without dropping. */
  cancel(): void {
    if (!this.drag) return;
    this.teardown();
  }

  private updateHover(): void {
    const drag = this.drag;
    if (!drag) return;

    const resolved = resolveActiveZoneId(
      Array.from(this.zones.values(), (zone) => ({
        id: zone.id,
        rect: zone.getRect(),
        accepts: zone.accepts(drag.payload),
      })),
      this.point,
    );
    this.activeZoneId = resolved;

    for (const zone of this.zones.values()) {
      const shouldHover =
        zone.accepts(drag.payload) && pointInRect(this.point, zone.getRect());
      if (zone.hovered !== shouldHover) {
        zone.hovered = shouldHover;
        zone.setHover(shouldHover);
      }
    }
  }

  private handlePointerMove = (event: PointerEvent): void => {
    if (!this.drag) return;
    this.point = { x: event.clientX, y: event.clientY };
    this.updateHover();
    this.publish();
  };

  private handlePointerUp = (event: PointerEvent): void => {
    const drag = this.drag;
    if (!drag) return;
    this.point = { x: event.clientX, y: event.clientY };
    this.updateHover();
    const targetZone = this.activeZoneId
      ? this.zones.get(this.activeZoneId)
      : undefined;
    this.teardown();
    if (targetZone) targetZone.onDrop(drag.payload);
  };

  private handlePointerCancel = (): void => {
    this.teardown();
  };

  private handleWindowBlur = (): void => {
    this.teardown();
  };

  private handleKeyDown = (event: KeyboardEvent): void => {
    if (event.key === "Escape") this.teardown();
  };

  private teardown(): void {
    for (const zone of this.zones.values()) {
      if (zone.hovered) {
        zone.hovered = false;
        zone.setHover(false);
      }
    }
    this.activeZoneId = null;
    this.drag = null;
    if (typeof document !== "undefined") {
      document.body.removeAttribute("data-rf-dragging");
    }
    window.removeEventListener("pointermove", this.handlePointerMove, true);
    window.removeEventListener("pointerup", this.handlePointerUp, true);
    window.removeEventListener("pointercancel", this.handlePointerCancel, true);
    window.removeEventListener("keydown", this.handleKeyDown, true);
    window.removeEventListener("blur", this.handleWindowBlur);
    this.publish();
  }

  private publish(): void {
    this.snapshot = this.drag
      ? { active: true, point: this.point, label: this.drag.label ?? null }
      : EMPTY_SNAPSHOT;
    for (const listener of this.subscribers) listener();
  }
}

export const pointerDragController = new PointerDragController();

export type PointerGestureStart = {
  button: number;
  clientX: number;
  clientY: number;
  pointerId: number;
};

/**
 * Swallow the single synthetic `click` the browser emits at the end of a drag
 * gesture, so a drag never doubles as a tab activation. The listener removes
 * itself on the first click, with a macrotask fallback if no click follows
 * (e.g. the pointer was released over a different element).
 */
function suppressNextClick(): void {
  if (typeof window === "undefined") return;

  const onClick = (event: MouseEvent): void => {
    event.preventDefault();
    event.stopPropagation();
    window.removeEventListener("click", onClick, true);
  };

  window.addEventListener("click", onClick, true);
  window.setTimeout(() => {
    window.removeEventListener("click", onClick, true);
  }, 0);
}

/**
 * Begin a pointer "press, move past threshold, then drag" gesture. Until the
 * pointer travels {@link POINTER_DRAG_THRESHOLD_PX} the press is treated as a
 * plain click (so tab activation still works); once it exceeds the threshold a
 * controller drag starts. Returns a cleanup that detaches the temporary
 * listeners if the caller unmounts mid-gesture.
 */
export function beginPointerDragGesture(
  event: PointerGestureStart,
  resolveDrag: () => ActivePointerDrag | null,
  onDragStarted?: () => void,
): () => void {
  if (event.button !== 0) return () => undefined;

  const startX = event.clientX;
  const startY = event.clientY;
  const pointerId = event.pointerId;
  let started = false;

  const cleanup = (): void => {
    window.removeEventListener("pointermove", handleMove, true);
    window.removeEventListener("pointerup", handleUp, true);
    window.removeEventListener("pointercancel", handleUp, true);
  };

  function handleMove(moveEvent: PointerEvent): void {
    if (started || moveEvent.pointerId !== pointerId) return;
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    if (Math.hypot(dx, dy) < POINTER_DRAG_THRESHOLD_PX) return;

    const drag = resolveDrag();
    if (!drag) {
      cleanup();
      return;
    }
    started = true;
    cleanup();
    suppressNextClick();
    onDragStarted?.();
    pointerDragController.startDrag(drag, {
      x: moveEvent.clientX,
      y: moveEvent.clientY,
    });
  }

  function handleUp(): void {
    cleanup();
  }

  window.addEventListener("pointermove", handleMove, true);
  window.addEventListener("pointerup", handleUp, true);
  window.addEventListener("pointercancel", handleUp, true);

  return cleanup;
}
