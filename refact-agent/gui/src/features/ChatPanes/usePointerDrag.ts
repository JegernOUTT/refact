import { useCallback, useEffect, useId, useRef, useState } from "react";

import { useConfig } from "../../hooks";
import {
  isPointerDragHost,
  pointerDragController,
  rectFromElement,
} from "./pointerDrag";
import type { TabDragPayload } from "./tabDrag";

/** True when the current host needs pointer-driven drag-and-drop (JCEF). */
export function usePointerDragHost(): boolean {
  const config = useConfig();
  return isPointerDragHost(config.host);
}

export type PointerDropZoneArgs = {
  /** Only registers while enabled (pointer host). */
  enabled: boolean;
  accepts: (payload: TabDragPayload) => boolean;
  onDrop: (payload: TabDragPayload) => void;
};

export type PointerDropZoneResult = {
  ref: (node: HTMLElement | null) => void;
  isOver: boolean;
};

/**
 * Register a DOM element as a pointer drag drop target. Returns a ref callback
 * to attach to the element and an `isOver` flag for hover styling. While the
 * host uses native HTML5 DnD this is inert (`enabled === false`).
 */
export function usePointerDropZone({
  enabled,
  accepts,
  onDrop,
}: PointerDropZoneArgs): PointerDropZoneResult {
  const id = useId();
  const elementRef = useRef<HTMLElement | null>(null);
  const [isOver, setIsOver] = useState(false);

  const acceptsRef = useRef(accepts);
  const onDropRef = useRef(onDrop);
  acceptsRef.current = accepts;
  onDropRef.current = onDrop;

  const ref = useCallback((node: HTMLElement | null) => {
    elementRef.current = node;
  }, []);

  useEffect(() => {
    if (!enabled) {
      setIsOver(false);
      return;
    }

    const unregister = pointerDragController.registerZone({
      id,
      getRect: () => rectFromElement(elementRef.current),
      accepts: (payload) => acceptsRef.current(payload),
      onDrop: (payload) => onDropRef.current(payload),
      setHover: setIsOver,
    });

    return () => {
      unregister();
      setIsOver(false);
    };
  }, [enabled, id]);

  return { ref, isOver: enabled && isOver };
}
