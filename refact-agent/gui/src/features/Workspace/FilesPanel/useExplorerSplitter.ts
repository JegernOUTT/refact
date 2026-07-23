import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

import {
  loadPersistedFilesExplorerWidth,
  savePersistedFilesExplorerWidth,
} from "../../../utils/chatUiPersistence";

export const EXPLORER_MIN_WIDTH = 200;
export const EXPLORER_MAX_WIDTH = 480;
export const EXPLORER_DEFAULT_WIDTH = 260;

export function clampExplorerWidth(value: number): number {
  if (!Number.isFinite(value)) return EXPLORER_DEFAULT_WIDTH;
  return Math.min(EXPLORER_MAX_WIDTH, Math.max(EXPLORER_MIN_WIDTH, value));
}

export function useExplorerSplitter() {
  const panelRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(() =>
    clampExplorerWidth(
      loadPersistedFilesExplorerWidth() ?? EXPLORER_DEFAULT_WIDTH,
    ),
  );
  const [dragging, setDragging] = useState(false);
  const liveWidthRef = useRef(width);
  const dragCleanupRef = useRef<(() => void) | null>(null);

  const handleSplitterPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const panel = panelRef.current;
      if (!panel) return;

      event.preventDefault();
      dragCleanupRef.current?.();
      setDragging(true);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const rect = panel.getBoundingClientRect();
        const next = clampExplorerWidth(moveEvent.clientX - rect.left);
        liveWidthRef.current = next;
        panel.style.setProperty("--files-explorer-w", `${next}px`);
      };

      const detach = () => {
        dragCleanupRef.current = null;
        setDragging(false);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerup", handlePointerUp);
        window.removeEventListener("pointercancel", handlePointerUp);
      };

      const handlePointerUp = () => {
        detach();
        setWidth(liveWidthRef.current);
        savePersistedFilesExplorerWidth(liveWidthRef.current);
      };

      dragCleanupRef.current = detach;
      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp);
      window.addEventListener("pointercancel", handlePointerUp);
    },
    [],
  );

  useEffect(() => () => dragCleanupRef.current?.(), []);

  return { panelRef, width, dragging, handleSplitterPointerDown };
}
