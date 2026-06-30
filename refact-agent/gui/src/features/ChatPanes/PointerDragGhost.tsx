import { useSyncExternalStore } from "react";
import { createPortal } from "react-dom";

import { pointerDragController } from "./pointerDrag";
import styles from "./pointerDrag.module.css";

/**
 * Floating chip that follows the cursor during a pointer drag, standing in for
 * the native drag image that JCEF cannot render. Renders nothing unless a drag
 * is active.
 */
export function PointerDragGhost() {
  const snapshot = useSyncExternalStore(
    pointerDragController.subscribe,
    pointerDragController.getSnapshot,
    pointerDragController.getSnapshot,
  );

  if (!snapshot.active || typeof document === "undefined") return null;

  return createPortal(
    <div
      className={styles.ghost}
      style={{ left: snapshot.point.x, top: snapshot.point.y }}
      aria-hidden="true"
      data-testid="pointer-drag-ghost"
    >
      {snapshot.label ?? "Moving…"}
    </div>,
    document.body,
  );
}
