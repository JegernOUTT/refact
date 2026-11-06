import { Maximize, RotateCw, Scan, ZoomIn, ZoomOut } from "lucide-react";
import {
  type KeyboardEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { ErrorState, IconButton, Tooltip } from "../../../components/ui";
import { useSurfaceSessionState } from "../surfaceSessionState";
import styles from "./ImageFilePreview.module.css";

export const MIN_ZOOM = 0.001;
export const MAX_ZOOM = 8;
const ZOOM_STEP = 1.25;
export const MIN_PREVIEW_HEIGHT = 160;
const HEIGHT_STEP = 40;

type Size = { width: number; height: number };

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

export type ImageFilePreviewProps = {
  src: string;
  alt: string;
  sessionPath?: string;
};

export function ImageFilePreview(props: ImageFilePreviewProps) {
  return <ImageFilePreviewContent key={props.src} {...props} />;
}

function ImageFilePreviewContent({
  src,
  alt,
  sessionPath = alt,
}: ImageFilePreviewProps) {
  const previewRef = useRef<HTMLDivElement | null>(null);
  const handleRef = useRef<HTMLDivElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const [natural, setNatural] = useState<Size | null>(null);
  const [viewport, setViewport] = useState<Size | null>(null);
  const [zoom, setZoom] = useSurfaceSessionState<number | "fit">(
    sessionPath,
    "zoom",
    "fit",
    src,
  );
  const [rotation, setRotation] = useSurfaceSessionState(
    sessionPath,
    "rotation",
    0,
    src,
  );
  const [height, setHeight] = useSurfaceSessionState<number | null>(
    sessionPath,
    "height",
    null,
    src,
  );
  const [availableHeight, setAvailableHeight] = useState(MIN_PREVIEW_HEIGHT);
  const [failed, setFailed] = useState(false);

  useLayoutEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const measure = () => {
      setViewport({ width: element.clientWidth, height: element.clientHeight });
      const available =
        (previewRef.current?.clientHeight ?? 0) -
        (handleRef.current?.offsetHeight ?? 0);
      if (available > 0) setAvailableHeight(available);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    if (previewRef.current) observer.observe(previewRef.current);
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
  const minZoom = Math.min(MIN_ZOOM, fitScale / 100, scale);
  const minHeight = Math.min(MIN_PREVIEW_HEIGHT, availableHeight);
  const effectiveHeight =
    height === null
      ? availableHeight
      : clamp(height, minHeight, availableHeight);
  const percent = Math.max(1, Math.round(scale * 100));

  const zoomBy = useCallback(
    (factor: number) =>
      setZoom((current) =>
        clamp(
          (current === "fit" ? fitScale : current) * factor,
          minZoom,
          MAX_ZOOM,
        ),
      ),
    [fitScale, minZoom, setZoom],
  );

  useEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      zoomBy(event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP);
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, [failed, zoomBy]);

  const onResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const delta =
      event.key === "ArrowDown"
        ? HEIGHT_STEP
        : event.key === "ArrowUp"
          ? -HEIGHT_STEP
          : 0;
    if (event.key === "Home") {
      event.preventDefault();
      setHeight(minHeight);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      setHeight(null);
      return;
    }
    if (delta === 0) return;
    event.preventDefault();
    setHeight(clamp(effectiveHeight + delta, minHeight, availableHeight));
  };

  const onResizePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { startY: event.clientY, startHeight: effectiveHeight };
  };

  const onResizePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    setHeight(
      clamp(
        drag.startHeight + event.clientY - drag.startY,
        minHeight,
        availableHeight,
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
    <div
      className={styles.preview}
      data-testid="image-preview"
      ref={previewRef}
    >
      <div className={styles.imageArea} data-resized={height !== null}>
        <div
          aria-label="Image controls"
          className={styles.toolbar}
          role="toolbar"
        >
          <Tooltip content="Zoom out">
            <IconButton
              aria-label="Zoom out"
              disabled={scale <= minZoom}
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
          data-resized={height !== null}
          style={height === null ? undefined : { height: effectiveHeight }}
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
      </div>
      <div
        aria-label="Resize image preview"
        aria-orientation="horizontal"
        aria-valuemax={availableHeight}
        aria-valuemin={minHeight}
        aria-valuenow={effectiveHeight}
        className={styles.resizeHandle}
        onKeyDown={onResizeKeyDown}
        onPointerCancel={onResizePointerUp}
        onPointerDown={onResizePointerDown}
        onPointerMove={onResizePointerMove}
        onPointerUp={onResizePointerUp}
        ref={handleRef}
        role="separator"
        tabIndex={0}
      />
    </div>
  );
}
