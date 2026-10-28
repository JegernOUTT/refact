import { Maximize, RotateCw, Scan, ZoomIn, ZoomOut } from "lucide-react";
import {
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { ErrorState, IconButton, Tooltip } from "../../../components/ui";
import styles from "./ImageFilePreview.module.css";

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;
const ZOOM_STEP = 1.25;
export const MIN_PREVIEW_HEIGHT = 160;
export const MAX_PREVIEW_HEIGHT = 1600;
export const DEFAULT_PREVIEW_HEIGHT = 360;
const HEIGHT_STEP = 40;

type Size = { width: number; height: number };

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

export type ImageFilePreviewProps = {
  src: string;
  alt: string;
};

export function ImageFilePreview(props: ImageFilePreviewProps) {
  return <ImageFilePreviewContent key={props.src} {...props} />;
}

function ImageFilePreviewContent({ src, alt }: ImageFilePreviewProps) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const [natural, setNatural] = useState<Size | null>(null);
  const [viewport, setViewport] = useState<Size | null>(null);
  const [zoom, setZoom] = useState<number | "fit">("fit");
  const [rotation, setRotation] = useState(0);
  const [height, setHeight] = useState(DEFAULT_PREVIEW_HEIGHT);
  const [failed, setFailed] = useState(false);

  useLayoutEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const measure = () =>
      setViewport({ width: element.clientWidth, height: element.clientHeight });
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [failed]);

  const rotated =
    natural && rotation % 180 !== 0
      ? { width: natural.height, height: natural.width }
      : natural;
  const fitScale =
    rotated &&
    viewport &&
    viewport.width > 0 &&
    viewport.height > 0 &&
    rotated.width > 0 &&
    rotated.height > 0
      ? Math.min(
          viewport.width / rotated.width,
          viewport.height / rotated.height,
          1,
        )
      : 1;
  const scale = zoom === "fit" ? fitScale : zoom;
  const percent = Math.max(1, Math.round(scale * 100));

  const zoomBy = useCallback(
    (factor: number) =>
      setZoom((current) =>
        clamp(
          (current === "fit" ? fitScale : current) * factor,
          MIN_ZOOM,
          MAX_ZOOM,
        ),
      ),
    [fitScale],
  );

  const onResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const delta =
      event.key === "ArrowDown"
        ? HEIGHT_STEP
        : event.key === "ArrowUp"
          ? -HEIGHT_STEP
          : 0;
    if (event.key === "Home") {
      event.preventDefault();
      setHeight(MIN_PREVIEW_HEIGHT);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      setHeight(MAX_PREVIEW_HEIGHT);
      return;
    }
    if (delta === 0) return;
    event.preventDefault();
    setHeight((current) =>
      clamp(current + delta, MIN_PREVIEW_HEIGHT, MAX_PREVIEW_HEIGHT),
    );
  };

  const onResizePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { startY: event.clientY, startHeight: height };
  };

  const onResizePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    setHeight(
      clamp(
        drag.startHeight + event.clientY - drag.startY,
        MIN_PREVIEW_HEIGHT,
        MAX_PREVIEW_HEIGHT,
      ),
    );
  };

  const onResizePointerUp = (event: PointerEvent<HTMLDivElement>) => {
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  if (failed) {
    return (
      <ErrorState
        description={`${alt} could not be decoded as an image.`}
        title="Image unavailable"
        variant="full"
      />
    );
  }

  const stageSize = rotated
    ? { width: rotated.width * scale, height: rotated.height * scale }
    : undefined;
  const imageSize = natural
    ? { width: natural.width * scale, height: natural.height * scale }
    : undefined;

  return (
    <div className={styles.preview} data-testid="image-preview">
      <div
        aria-label="Image controls"
        className={styles.toolbar}
        role="toolbar"
      >
        <Tooltip content="Zoom out">
          <IconButton
            aria-label="Zoom out"
            disabled={scale <= MIN_ZOOM}
            icon={ZoomOut}
            onClick={() => zoomBy(1 / ZOOM_STEP)}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <output
          aria-live="polite"
          aria-label="Zoom level"
          className={styles.zoomLevel}
        >
          {`${String(percent)}%`}
        </output>
        <Tooltip content="Zoom in">
          <IconButton
            aria-label="Zoom in"
            disabled={scale >= MAX_ZOOM}
            icon={ZoomIn}
            onClick={() => zoomBy(ZOOM_STEP)}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <Tooltip content="Fit to view">
          <IconButton
            aria-label="Fit to view"
            aria-pressed={zoom === "fit"}
            icon={Maximize}
            onClick={() => setZoom("fit")}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <Tooltip content="Actual size">
          <IconButton
            aria-label="Actual size"
            aria-pressed={zoom === 1}
            icon={Scan}
            onClick={() => setZoom(1)}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <Tooltip content="Rotate clockwise">
          <IconButton
            aria-label="Rotate clockwise"
            icon={RotateCw}
            onClick={() => setRotation((current) => (current + 90) % 360)}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        {natural ? (
          <span className={styles.dimensions}>
            {`${String(natural.width)} × ${String(natural.height)}`}
          </span>
        ) : null}
      </div>
      <div
        aria-label={`${alt} preview`}
        className={styles.viewport}
        data-testid="image-preview-viewport"
        ref={viewportRef}
        role="region"
        style={{ height }}
        tabIndex={0}
      >
        <div className={styles.stage} style={stageSize}>
          <img
            alt={alt}
            className={styles.image}
            data-rotation={rotation}
            draggable={false}
            onError={() => setFailed(true)}
            onLoad={(event) =>
              setNatural({
                width: event.currentTarget.naturalWidth,
                height: event.currentTarget.naturalHeight,
              })
            }
            src={src}
            style={imageSize}
          />
        </div>
      </div>
      <div
        aria-label="Resize image preview"
        aria-orientation="horizontal"
        aria-valuemax={MAX_PREVIEW_HEIGHT}
        aria-valuemin={MIN_PREVIEW_HEIGHT}
        aria-valuenow={height}
        className={styles.resizeHandle}
        onKeyDown={onResizeKeyDown}
        onPointerCancel={onResizePointerUp}
        onPointerDown={onResizePointerDown}
        onPointerMove={onResizePointerMove}
        onPointerUp={onResizePointerUp}
        role="separator"
        tabIndex={0}
      />
    </div>
  );
}
