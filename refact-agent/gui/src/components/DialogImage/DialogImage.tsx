import React, { useCallback, useEffect, useRef, useState } from "react";
import { Image, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { Dialog, Icon, IconButton, Tooltip } from "../ui";
import styles from "./DialogImage.module.css";

const SIZE_MAP = {
  "1": "24px",
  "2": "32px",
  "3": "40px",
  "4": "48px",
  "5": "56px",
  "6": "64px",
  "7": "72px",
  "8": "80px",
  "9": "96px",
  auto: "auto",
} as const;

const MIN_SCALE = 1;
const MAX_SCALE = 4;
const ZOOM_FACTOR = 1.25;

export const DialogImage: React.FC<{
  src: string;
  size?: keyof typeof SIZE_MAP;
  fallback?: React.ReactNode;
  alt?: string;
}> = ({
  size = "8",
  fallback = <Icon icon={Image} size="lg" />,
  src,
  alt = "",
}) => {
  const [open, setOpen] = useState(false);
  const [scale, setScale] = useState(MIN_SCALE);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const dragStart = useRef({ x: 0, y: 0, panX: 0, panY: 0 });
  const thumbnailStyle = {
    "--dialog-image-size": SIZE_MAP[size],
  } as React.CSSProperties;

  const resetView = useCallback(() => {
    setScale(MIN_SCALE);
    setPan({ x: 0, y: 0 });
  }, []);

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      setOpen(nextOpen);
      if (!nextOpen) resetView();
    },
    [resetView],
  );

  const zoomBy = useCallback((factor: number) => {
    setScale((current) =>
      Math.min(MAX_SCALE, Math.max(MIN_SCALE, current * factor)),
    );
  }, []);

  const handleMouseDown = useCallback(
    (event: React.MouseEvent) => {
      if (event.button !== 0 || scale === MIN_SCALE) return;
      event.preventDefault();
      dragStart.current = {
        x: event.clientX,
        y: event.clientY,
        panX: pan.x,
        panY: pan.y,
      };
      setDragging(true);
    },
    [pan.x, pan.y, scale],
  );

  useEffect(() => {
    if (!dragging) return;
    const handleMove = (event: MouseEvent) => {
      setPan({
        x: dragStart.current.panX + event.clientX - dragStart.current.x,
        y: dragStart.current.panY + event.clientY - dragStart.current.y,
      });
    };
    const handleUp = () => setDragging(false);
    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [dragging]);

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Trigger asChild>
        <button
          type="button"
          className={styles.trigger}
          style={thumbnailStyle}
          aria-label={alt ? `Open image: ${alt}` : "Open image viewer"}
        >
          <img className={styles.thumbnail} src={src} alt={alt} />
          <span className={styles.fallback}>{fallback}</span>
        </button>
      </Dialog.Trigger>
      <Dialog.Content
        className={styles.content}
        maxWidth="90vw"
        maxHeight="90vh"
      >
        <Dialog.Title className={styles.srOnly}>
          {alt || "Image viewer"}
        </Dialog.Title>
        <div className={styles.toolbar}>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Zoom in"
                icon={ZoomIn}
                size="sm"
                variant="ghost"
                onClick={() => zoomBy(ZOOM_FACTOR)}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Zoom in</Tooltip.Content>
          </Tooltip>
          <span className={styles.zoomInfo}>{Math.round(scale * 100)}%</span>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Zoom out"
                disabled={scale === MIN_SCALE}
                icon={ZoomOut}
                size="sm"
                variant="ghost"
                onClick={() => zoomBy(1 / ZOOM_FACTOR)}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Zoom out</Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Reset image view"
                icon={RotateCcw}
                size="sm"
                variant="ghost"
                onClick={resetView}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Reset view</Tooltip.Content>
          </Tooltip>
        </div>
        <div
          aria-label="Image pan and zoom"
          className={styles.viewport}
          data-dragging={dragging || undefined}
          role="application"
          onMouseDown={handleMouseDown}
        >
          <img
            className={styles.image}
            src={src}
            alt={alt}
            draggable={false}
            style={{
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
            }}
          />
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
