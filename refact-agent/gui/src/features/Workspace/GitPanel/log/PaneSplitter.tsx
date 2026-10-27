import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";

import styles from "./GitLog.module.css";

const KEYBOARD_STEP = 16;
const KEYBOARD_STEP_LARGE = 64;

export type PaneSplitterProps = {
  label: string;
  /** Element whose `cssVar` holds the live pane width. */
  containerRef: RefObject<HTMLElement>;
  cssVar: string;
  value: number;
  min: number;
  max: number;
  /** `end` panes grow when the pointer moves left. */
  edge: "start" | "end";
  onCommit: (value: number) => void;
};

const clamp = (value: number, min: number, max: number) =>
  Math.round(Math.min(max, Math.max(min, value)));

export function PaneSplitter({
  label,
  containerRef,
  cssVar,
  value,
  min,
  max,
  edge,
  onCommit,
}: PaneSplitterProps) {
  const [dragging, setDragging] = useState(false);
  const cleanupRef = useRef<(() => void) | null>(null);
  const direction = edge === "start" ? 1 : -1;

  const handlePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const container = containerRef.current;
      if (!container) return;
      event.preventDefault();
      cleanupRef.current?.();
      const startX = event.clientX;
      let live = value;
      setDragging(true);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handleMove = (moveEvent: PointerEvent) => {
        live = clamp(
          value + direction * (moveEvent.clientX - startX),
          min,
          max,
        );
        container.style.setProperty(cssVar, `${live}px`);
      };
      const detach = () => {
        cleanupRef.current = null;
        setDragging(false);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
        window.removeEventListener("pointercancel", handleUp);
      };
      const handleUp = () => {
        detach();
        onCommit(live);
      };
      cleanupRef.current = detach;
      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
      window.addEventListener("pointercancel", handleUp);
    },
    [containerRef, cssVar, direction, max, min, onCommit, value],
  );

  useEffect(() => () => cleanupRef.current?.(), []);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? KEYBOARD_STEP_LARGE : KEYBOARD_STEP;
    const keys: Record<string, number> = {
      ArrowLeft: value - direction * step,
      ArrowRight: value + direction * step,
      Home: min,
      End: max,
    };
    if (!(event.key in keys)) return;
    event.preventDefault();
    onCommit(clamp(keys[event.key], min, max));
  };

  return (
    <div
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemax={max}
      aria-valuemin={min}
      aria-valuenow={value}
      className={styles.splitter}
      data-dragging={dragging || undefined}
      onKeyDown={handleKeyDown}
      onPointerDown={handlePointerDown}
      role="separator"
      tabIndex={0}
    />
  );
}
