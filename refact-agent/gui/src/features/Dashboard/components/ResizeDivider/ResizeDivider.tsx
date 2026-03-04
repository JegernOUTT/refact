import React, { useCallback, useRef, useState } from "react";
import styles from "./ResizeDivider.module.css";

type Props = {
  onDrag: (clientY: number) => void;
  onReset?: () => void;
};

export const ResizeDivider: React.FC<Props> = ({ onDrag, onReset }) => {
  const [dragging, setDragging] = useState(false);
  const isDragging = useRef(false);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isDragging.current = true;
    setDragging(true);
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";

    const handleMouseMove = (e: MouseEvent) => {
      if (isDragging.current) onDrag(e.clientY);
    };

    const handleMouseUp = () => {
      isDragging.current = false;
      setDragging(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  }, [onDrag]);

  return (
    <div
      className={styles.divider}
      onMouseDown={handleMouseDown}
      onDoubleClick={onReset}
      data-dragging={dragging || undefined}
      role="separator"
      aria-orientation="horizontal"
    >
      <div className={styles.handle} />
    </div>
  );
};
