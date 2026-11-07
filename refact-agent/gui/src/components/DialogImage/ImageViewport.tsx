import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import styles from "./ImageViewport.module.css";

export type ImagePoint = { x: number; y: number };
export type ImageViewportActions = {
  zoomIn: () => void;
  zoomOut: () => void;
  fit: () => void;
  actualSize: () => void;
};
export type ImageViewportProps = {
  width: number;
  height: number;
  children: React.ReactNode;
  drawing?: boolean;
  fill?: boolean;
  overlay?: React.ReactNode;
  controlsContainer?: HTMLElement | null;
  onDrawStart?: (point: ImagePoint) => void;
  onDrawMove?: (point: ImagePoint) => void;
  onDrawEnd?: () => void;
  onDrawCancel?: () => void;
};

export const ImageViewport = React.forwardRef<
  ImageViewportActions,
  ImageViewportProps
>(function ImageViewport(
  {
    width,
    height,
    children,
    drawing = false,
    fill = false,
    overlay,
    controlsContainer,
    onDrawStart,
    onDrawMove,
    onDrawEnd,
    onDrawCancel,
  }: ImageViewportProps,
  ref,
) {
  const root = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 1, height: 1 });
  const [view, setView] = useState({ zoom: 1, x: 0, y: 0 });
  const drag = useRef<{
    id: number;
    x: number;
    y: number;
    panX: number;
    panY: number;
  } | null>(null);
  useEffect(() => {
    const node = root.current;
    if (!node) return;
    const observer = new ResizeObserver(() =>
      setSize({ width: node.clientWidth, height: node.clientHeight }),
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  useEffect(() => setView({ zoom: 1, x: 0, y: 0 }), [width, height]);
  const fit = Math.min(size.width / width, size.height / height, 1);
  const scale = fit * view.zoom;
  const point = (event: React.PointerEvent): ImagePoint => {
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      x: Math.max(
        0,
        Math.min(
          width,
          (event.clientX - rect.left - size.width / 2 - view.x) / scale +
            width / 2,
        ),
      ),
      y: Math.max(
        0,
        Math.min(
          height,
          (event.clientY - rect.top - size.height / 2 - view.y) / scale +
            height / 2,
        ),
      ),
    };
  };
  const zoom = (factor: number, x = 0, y = 0) =>
    setView((current) => {
      const next = Math.max(
        1,
        Math.min(Math.max(4, 1 / fit), current.zoom * factor),
      );
      const ratio = next / current.zoom;
      return {
        zoom: next,
        x: x - (x - current.x) * ratio,
        y: y - (y - current.y) * ratio,
      };
    });
  const actions = {
    zoomIn: () => zoom(1.25),
    zoomOut: () => zoom(1 / 1.25),
    fit: () => setView({ zoom: 1, x: 0, y: 0 }),
    actualSize: () => setView({ zoom: 1 / fit, x: 0, y: 0 }),
  };
  React.useImperativeHandle(ref, () => actions);
  const wheelZoom = useRef(zoom);
  wheelZoom.current = zoom;
  useEffect(() => {
    const node = root.current;
    if (!node) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = node.getBoundingClientRect();
      const units =
        event.deltaMode === 1
          ? 16
          : event.deltaMode === 2
            ? node.clientHeight
            : 1;
      wheelZoom.current(
        Math.exp(-event.deltaY * units * 0.002),
        event.clientX - rect.left - node.clientWidth / 2,
        event.clientY - rect.top - node.clientHeight / 2,
      );
    };
    node.addEventListener("wheel", wheel, { passive: false });
    return () => node.removeEventListener("wheel", wheel);
  }, []);
  const finish = (event: React.PointerEvent, cancel: boolean) => {
    if (drag.current?.id !== event.pointerId) return;
    drag.current = null;
    if (drawing) {
      if (cancel) onDrawCancel?.();
      else onDrawEnd?.();
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId);
  };
  const controls = (
    <div
      className={styles.controls}
      role="group"
      aria-label="Image view controls"
    >
      <button type="button" onClick={actions.zoomOut} aria-label="Zoom out">
        −
      </button>
      <output aria-label="Image zoom">{Math.round(scale * 100)}%</output>
      <button type="button" onClick={actions.zoomIn} aria-label="Zoom in">
        +
      </button>
      <button type="button" onClick={actions.fit}>
        Fit
      </button>
      <button type="button" onClick={actions.actualSize}>
        Actual size
      </button>
    </div>
  );
  return (
    <div className={`${styles.shell} ${fill ? styles.fill : ""}`}>
      {controlsContainer === undefined
        ? controls
        : controlsContainer && createPortal(controls, controlsContainer)}
      <div
        ref={root}
        className={styles.viewport}
        data-drawing={drawing || undefined}
        aria-label="Image pan and zoom"
        onPointerDown={(event) => {
          if (event.button !== 0 || !event.isPrimary || drag.current) return;
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = {
            id: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            panX: view.x,
            panY: view.y,
          };
          if (drawing) onDrawStart?.(point(event));
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (start?.id !== event.pointerId) return;
          if (drawing) onDrawMove?.(point(event));
          else
            setView((current) => ({
              ...current,
              x: start.panX + event.clientX - start.x,
              y: start.panY + event.clientY - start.y,
            }));
        }}
        onPointerUp={(event) => finish(event, false)}
        onPointerCancel={(event) => finish(event, true)}
        onLostPointerCapture={(event) => finish(event, true)}
      >
        <div
          className={styles.image}
          style={{
            width,
            height,
            transform: `translate(${view.x}px, ${view.y}px) scale(${scale})`,
          }}
        >
          {children}
          {overlay && <div className={styles.overlay}>{overlay}</div>}
        </div>
      </div>
    </div>
  );
});
