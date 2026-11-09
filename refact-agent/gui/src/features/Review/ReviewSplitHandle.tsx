import { useRef } from "react";
import styles from "./ReviewSplitHandle.module.css";

export type ReviewSplitHandleProps = {
  value: number;
  min: number;
  max: number;
  onChange: (width: number) => void;
  controls: string;
};

export function ReviewSplitHandle({
  value,
  min,
  max,
  onChange,
  controls,
}: ReviewSplitHandleProps) {
  const drag = useRef<{ pointer: number; x: number; value: number } | null>(
    null,
  );
  const clamp = (width: number) => Math.min(max, Math.max(min, width));
  return (
    <div
      className={styles.handle}
      role="separator"
      aria-label="Resize chat"
      aria-orientation="vertical"
      aria-controls={controls}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={clamp(value)}
      tabIndex={0}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        drag.current = { pointer: event.pointerId, x: event.clientX, value };
        event.currentTarget.setPointerCapture(event.pointerId);
        event.preventDefault();
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (start?.pointer === event.pointerId)
          onChange(clamp(start.value + event.clientX - start.x));
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        drag.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={(event) => {
        if (drag.current?.pointer === event.pointerId) drag.current = null;
      }}
      onLostPointerCapture={() => {
        drag.current = null;
      }}
      onKeyDown={(event) => {
        let next: number;
        switch (event.key) {
          case "ArrowLeft":
            next = value - 10;
            break;
          case "ArrowRight":
            next = value + 10;
            break;
          case "Home":
            next = min;
            break;
          case "End":
            next = max;
            break;
          default:
            return;
        }
        event.preventDefault();
        onChange(clamp(next));
      }}
    />
  );
}
