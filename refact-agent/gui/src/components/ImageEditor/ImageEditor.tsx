import React, { useEffect, useRef, useState } from "react";
import {
  ArrowUpRight,
  Brush,
  PanelRight,
  Circle,
  Hand,
  Minus,
  Redo2,
  RotateCcw,
  Square,
  Type,
  Undo2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { KitButton, KitDialog, KitTooltip } from "../ComposerKit/ComposerKit";
import { Icon } from "../ui/Icon";
import {
  ImageViewport,
  ImageViewportActions,
} from "../DialogImage/ImageViewport";
import {
  attachmentFileError,
  isSupportedImageFile,
} from "../../utils/attachmentFiles";
import { inspectImage } from "./decode";
import {
  Annotation,
  DEFAULT_SETTINGS,
  ExportSettings,
  MAX_ANNOTATIONS,
  MAX_POINTS,
  Point,
  Tool,
  exportImage,
  renderImage,
  unchanged,
} from "./model";
import styles from "./ImageEditor.module.css";

export type ImageEditorProps = {
  file: File;
  onApply: (file: File) => void;
  onCancel: () => void;
};
const tools: {
  value: Tool;
  label: string;
  icon: LucideIcon;
  shortcut: string;
}[] = [
  { value: "pan", shortcut: "V", label: "Pan", icon: Hand },
  { value: "brush", shortcut: "B", label: "Brush", icon: Brush },
  { value: "text", shortcut: "T", label: "Text", icon: Type },
  {
    value: "brush-text",
    shortcut: "Shift+B",
    label: "Brush + Text",
    icon: Brush,
  },
  { value: "arrow", shortcut: "A", label: "Arrow", icon: ArrowUpRight },
  { value: "line", shortcut: "L", label: "Line", icon: Minus },
  { value: "ellipse", shortcut: "E", label: "Ellipse", icon: Circle },
  { value: "rectangle", shortcut: "R", label: "Rectangle", icon: Square },
];
function ToolButton({
  label,
  icon,
  compound,
  shortcut,
  guidance,
  ...props
}: Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "children"> & {
  label: string;
  icon: LucideIcon;
  compound?: boolean;
  shortcut?: string;
  guidance?: string;
}) {
  return (
    <KitTooltip label={label} hint={guidance ?? shortcut}>
      <KitButton {...props} aria-label={label} className={styles.toolButton}>
        <Icon icon={icon} />
        {compound && <Icon icon={Type} size="sm" />}
      </KitButton>
    </KitTooltip>
  );
}

const fileKeys = new WeakMap<File, number>();
let nextFileKey = 0;
function fileKey(file: File) {
  let key = fileKeys.get(file);
  if (key === undefined) {
    key = ++nextFileKey;
    fileKeys.set(file, key);
  }
  return key;
}

export function ImageEditor(props: ImageEditorProps) {
  return <EditorSession key={fileKey(props.file)} {...props} />;
}
function EditorSession({ file, onApply, onCancel }: ImageEditorProps) {
  const [info, setInfo] = useState<Awaited<
    ReturnType<typeof inspectImage>
  > | null>(null);
  const [image, setImage] = useState<ImageBitmap | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [allowResize, setAllowResize] = useState(false);
  const [history, setHistory] = useState<Annotation[][]>([[]]);
  const [index, setIndex] = useState(0);
  const annotations = history[index];
  const [draft, setDraft] = useState<Annotation | null>(null);
  const draftRef = useRef<Annotation | null>(null);
  const [pendingLabel, setPendingLabel] = useState<Annotation | null>(null);
  const [textPoint, setTextPoint] = useState<Point | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const clearGesture = () => {
    setTextPoint(null);
    setText("");
    updateDraft(null);
    setPendingLabel(null);
  };
  const [tool, setTool] = useState<Tool>("pan");
  const [color, setColor] = useState("#ff0000");
  const [brushSize, setBrushSize] = useState(4);
  const [textSize, setTextSize] = useState(24);
  const [text, setText] = useState("");
  const composing = useRef(false);
  const textWidth = React.useMemo(() => {
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return 80;
    ctx.font = `${textSize}px sans-serif`;
    return Math.max(
      80,
      ...text
        .split("\n")
        .map((line) => ctx.measureText(line || "Type here").width + 8),
    );
  }, [text, textSize]);
  const [settings, setSettings] = useState<ExportSettings>(DEFAULT_SETTINGS);
  const [targetUnit, setTargetUnit] = useState("KB");
  const [acknowledged, setAcknowledged] = useState(false);
  const viewport = useRef<ImageViewportActions>(null);
  const [zoomControls, setZoomControls] = useState<HTMLDivElement | null>(null);
  const preview = useRef<HTMLCanvasElement>(null);
  const [result, setResult] = useState<{
    file: File;
    warning: string | null;
    key: object;
  } | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const [pending, setPending] = useState(true);
  const generation = useRef(0);
  const request = React.useMemo(
    () => ({ annotations, settings, image, allowResize }),
    [annotations, settings, image, allowResize],
  );
  const validation =
    attachmentFileError(file) ??
    (!isSupportedImageFile(file) ? "Unsupported image type." : null);
  const oversized =
    !!info &&
    (info.width !== info.safe.width || info.height !== info.safe.height);
  const width = info ? (allowResize ? info.safe.width : info.width) : 1;
  const height = info ? (allowResize ? info.safe.height : info.height) : 1;
  const original = unchanged(annotations, settings) && !allowResize;
  const flattenWarning = !original && info?.animationWarning;
  const updateDraft = (value: Annotation | null) => {
    draftRef.current = value;
    setDraft(value);
  };
  const commit = (value: Annotation) => {
    if (annotations.length >= MAX_ANNOTATIONS) return;
    setHistory((current) =>
      [...current.slice(0, index + 1), [...annotations, value]].slice(-101),
    );
    setIndex(Math.min(index + 1, 100));
    setAcknowledged(false);
  };
  useEffect(() => {
    let cancelled = false;
    if (validation) {
      setLoadError(validation);
      return;
    }
    void inspectImage(file)
      .then((value) => {
        if (!cancelled) setInfo(value);
      })
      .catch((error: unknown) => {
        if (!cancelled)
          setLoadError(
            error instanceof Error ? error.message : "Cannot read image.",
          );
      });
    return () => {
      cancelled = true;
    };
  }, [file, validation]);
  useEffect(() => {
    if (!info || (oversized && !allowResize)) return;
    let cancelled = false;
    let bitmap: ImageBitmap | undefined;
    void createImageBitmap(file, {
      imageOrientation: "from-image",
      resizeWidth: width,
      resizeHeight: height,
      resizeQuality: "high",
    })
      .then((value) => {
        bitmap = value;
        if (cancelled) value.close();
        else setImage(value);
      })
      .catch(() => {
        if (!cancelled)
          setLoadError(
            "Image decoding failed. You can attach the original without editing.",
          );
      });
    return () => {
      cancelled = true;
      bitmap?.close();
    };
  }, [file, info, oversized, allowResize, width, height]);
  useEffect(() => {
    if (!image || !preview.current) return;
    const canvas = preview.current;
    const frame = requestAnimationFrame(() =>
      renderImage(
        canvas,
        image,
        width,
        height,
        [
          ...annotations,
          ...(pendingLabel ? [pendingLabel] : []),
          ...(draft ? [draft] : []),
        ],
        { ...settings, scale: 100 },
      ),
    );
    return () => cancelAnimationFrame(frame);
  }, [image, width, height, annotations, draft, pendingLabel, settings]);
  useEffect(() => {
    const id = ++generation.current;
    let cancelled = false;
    setPending(true);
    setExportError(null);
    setResult(null);
    const timer = window.setTimeout(() => {
      const run = async () => {
        if (original) {
          setResult({ file, warning: null, key: request });
          return;
        }
        if (!image) return;
        const canvas = document.createElement("canvas");
        try {
          renderImage(canvas, image, width, height, annotations, settings);
          const output = await exportImage(
            canvas,
            settings,
            () => cancelled || generation.current !== id,
          );
          if (cancelled || generation.current !== id) return;
          const extension = settings.format
            .split("/")[1]
            .replace("jpeg", "jpg");
          const exported = new File(
            [output.blob],
            `${file.name.replace(/\.[^.]+$/, "")}.${extension}`,
            { type: settings.format, lastModified: file.lastModified },
          );
          const error = attachmentFileError(exported);
          if (error) throw new Error(error);
          setResult({ file: exported, warning: output.warning, key: request });
        } finally {
          canvas.width = 0;
          canvas.height = 0;
        }
      };
      void run()
        .catch((error: unknown) => {
          if (!cancelled)
            setExportError(
              error instanceof Error ? error.message : "Image export failed.",
            );
        })
        .finally(() => {
          if (!cancelled) setPending(false);
        });
    }, 220);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [file, original, image, width, height, annotations, settings, request]);
  const begin = (point: Point) => {
    if (tool === "pan" || annotations.length >= MAX_ANNOTATIONS) return;
    if (textPoint) return;
    if (tool === "text" || (tool === "brush-text" && pendingLabel)) {
      setText("");
      setTextPoint(point);
      return;
    }
    updateDraft({
      tool: tool === "brush-text" ? "brush" : tool,
      points: [point],
      color,
      size: brushSize,
    });
  };
  const move = (point: Point) => {
    const current = draftRef.current;
    if (!current) return;
    updateDraft({
      ...current,
      points:
        current.tool === "brush"
          ? current.points.length < MAX_POINTS
            ? [...current.points, point]
            : current.points
          : [current.points[0], point],
    });
  };
  const end = () => {
    const current = draftRef.current;
    if (current) {
      const a = current.points[0];
      const b = current.points[current.points.length - 1];
      const drawable =
        current.tool === "brush" ||
        (current.tool === "rectangle" || current.tool === "ellipse"
          ? Math.abs(a.x - b.x) >= 1 && Math.abs(a.y - b.y) >= 1
          : Math.hypot(a.x - b.x, a.y - b.y) >= 1);
      if (drawable) {
        if (tool === "brush-text") setPendingLabel(current);
        else commit(current);
      }
    }
    updateDraft(null);
  };
  const commitText = () => {
    if (!textPoint || !text.trim()) return;
    if (pendingLabel) {
      commit({
        ...pendingLabel,
        tool: "brush-text",
        label: { point: textPoint, text, size: textSize, color },
      });
    } else {
      commit({
        tool: "text",
        points: [textPoint],
        text,
        size: textSize,
        color,
      });
    }
    clearGesture();
    preview.current?.focus({ preventScroll: true });
  };
  const setOption = <K extends keyof ExportSettings>(
    key: K,
    value: ExportSettings[K],
  ) => {
    setSettings((current) => ({ ...current, [key]: value }));
    setAcknowledged(false);
  };
  const selectTool = (value: Tool) => {
    clearGesture();
    setTool(value);
  };
  const undo = () => {
    const hasGesture = !!(draftRef.current ?? pendingLabel ?? textPoint);
    clearGesture();
    if (!hasGesture) setIndex(Math.max(0, index - 1));
  };
  const redo = () => {
    const hasGesture = !!(draftRef.current ?? pendingLabel ?? textPoint);
    clearGesture();
    if (!hasGesture) setIndex(Math.min(history.length - 1, index + 1));
  };
  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (
      event.defaultPrevented ||
      event.nativeEvent.isComposing ||
      event.repeat ||
      event.altKey ||
      (event.target instanceof HTMLElement &&
        event.target.closest(
          'input, textarea, select, [contenteditable]:not([contenteditable="false"])',
        ))
    )
      return;
    const key = event.key.toLowerCase();
    let action: (() => void) | undefined;
    if (
      key === "enter" &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      ready &&
      !(
        event.target instanceof HTMLElement &&
        event.target.closest("button, summary")
      )
    ) {
      action = () => onApply(result.file);
    } else if (
      (event.ctrlKey || event.metaKey) &&
      !(event.ctrlKey && event.metaKey)
    ) {
      if (key === "z") action = event.shiftKey ? redo : undo;
      else if (key === "y" && event.ctrlKey && !event.shiftKey) action = redo;
    } else if (!event.ctrlKey && !event.metaKey && image) {
      const selected = tools.find(
        ({ shortcut }) =>
          shortcut.toLowerCase() === `${event.shiftKey ? "shift+" : ""}${key}`,
      );
      if (selected) action = () => selectTool(selected.value);
      else if (key === "+" || (key === "=" && !event.shiftKey))
        action = () => viewport.current?.zoomIn();
      else if (key === "-" && !event.shiftKey)
        action = () => viewport.current?.zoomOut();
      else if (key === "0" && !event.shiftKey)
        action = () => viewport.current?.fit();
      else if (key === "1" && !event.shiftKey)
        action = () => viewport.current?.actualSize();
    }
    if (action) {
      event.preventDefault();
      event.stopPropagation();
      action();
    }
  };
  const guidance = textPoint
    ? "Type on image · Ctrl/Cmd+Enter to commit · Escape to cancel"
    : tool === "brush-text"
      ? pendingLabel
        ? "Click beside the stroke to place your text label. Escape cancels."
        : "Draw a brush stroke, then place a text label beside it."
      : tool === "text"
        ? "Click the image to type directly on it."
        : undefined;
  const ready =
    !validation &&
    !!info &&
    !loadError &&
    !pending &&
    !exportError &&
    result?.key === request &&
    !draft &&
    !pendingLabel &&
    !textPoint &&
    (!flattenWarning || acknowledged);
  return (
    <div onKeyDown={handleKeyDown}>
      <KitDialog
        open
        onOpenChange={(open) => {
          if (!open) onCancel();
        }}
        title="Edit image"
        description={
          <span className={styles.srOnly}>
            Changes stay local. Apply attaches the image; it does not send your
            message.
          </span>
        }
        className={styles.dialog}
        width="min(1100px, 96vw)"
        maxHeight="94dvh"
        bodyClassName={styles.body}
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          if (event.target instanceof HTMLElement) event.target.focus();
        }}
        onEscapeKeyDown={(event) => {
          event.stopPropagation();
          if (composing.current) {
            event.preventDefault();
            return;
          }
          if (draftRef.current ?? pendingLabel ?? textPoint) {
            event.preventDefault();
            clearGesture();
          }
        }}
        footerStart={
          <div className={styles.footerStart}>
            <div className={styles.zoomControls} ref={setZoomControls} />
            <div
              className={styles.exportStatus}
              role="status"
              aria-live="polite"
            >
              {pending
                ? "Preparing export…"
                : result
                  ? `${original ? "Original bytes unchanged · " : ""}${Math.max(
                      1,
                      Math.round((width * settings.scale) / 100),
                    )} × ${Math.max(
                      1,
                      Math.round((height * settings.scale) / 100),
                    )} · ${result.file.size.toLocaleString()} bytes`
                  : ""}
              {result?.warning && <span>{result.warning}</span>}
              {exportError && <span role="alert">{exportError}</span>}
            </div>
          </div>
        }
        footer={
          <>
            <KitButton onClick={onCancel}>Cancel / Discard</KitButton>
            <KitButton
              disabled={!ready}
              onClick={() => {
                if (ready) onApply(result.file);
              }}
            >
              Apply
            </KitButton>
          </>
        }
      >
        <div className={styles.editor}>
          {!image && (
            <div className={styles.loadState}>
              {loadError && <p role="alert">{loadError}</p>}
              {oversized && !allowResize && (
                <div role="status">
                  <p>
                    This image exceeds the editing limit (16 megapixels / 8192
                    pixels per side). Editing requires an explicit resize to{" "}
                    {info.safe.width} × {info.safe.height}.
                  </p>
                  <KitButton onClick={() => setAllowResize(true)}>
                    Resize to safe resolution
                  </KitButton>
                </div>
              )}
              {(loadError !== null || (oversized && !allowResize)) &&
                !validation && (
                  <KitButton onClick={() => onApply(file)}>
                    Use original without editing
                  </KitButton>
                )}
              {!info && !loadError && <p role="status">Inspecting image…</p>}
            </div>
          )}
          {image && (
            <>
              <div
                className={styles.rail}
                role="group"
                aria-label="Annotation tools"
              >
                {tools.map(({ value, label, icon, shortcut }) => (
                  <ToolButton
                    key={value}
                    label={label}
                    shortcut={shortcut}
                    guidance={tool === value ? guidance : undefined}
                    icon={icon}
                    compound={value === "brush-text"}
                    aria-pressed={tool === value}
                    onClick={() => selectTool(value)}
                  />
                ))}
                <ToolButton
                  label="Undo"
                  icon={Undo2}
                  disabled={
                    index === 0 && !draft && !pendingLabel && !textPoint
                  }
                  shortcut="Ctrl/Cmd+Z"
                  onClick={undo}
                />
                <ToolButton
                  label="Redo"
                  icon={Redo2}
                  disabled={
                    index === history.length - 1 &&
                    !draft &&
                    !pendingLabel &&
                    !textPoint
                  }
                  shortcut="Ctrl/Cmd+Shift+Z or Ctrl+Y"
                  onClick={redo}
                />
                <ToolButton
                  label="Reset"
                  icon={RotateCcw}
                  onClick={() => {
                    clearGesture();
                    setHistory([[]]);
                    setIndex(0);
                    setSettings(DEFAULT_SETTINGS);
                  }}
                />
              </div>
              <div
                className={styles.options}
                role="group"
                aria-label="Tool options"
              >
                <span className={styles.activeTool}>
                  {tools.find(({ value }) => value === tool)?.label}
                </span>
                {tool !== "pan" && (
                  <>
                    <label>
                      <span className={styles.optionLabel}>Color</span>{" "}
                      <input
                        aria-label="Annotation color"
                        type="color"
                        value={color}
                        onChange={(event) => setColor(event.target.value)}
                      />
                    </label>
                    {tool !== "text" && (
                      <label>
                        <span className={styles.optionLabel}>Size</span>{" "}
                        <input
                          aria-label="Brush size"
                          type="number"
                          min="1"
                          max="80"
                          value={brushSize}
                          onChange={(event) =>
                            setBrushSize(
                              Math.max(
                                1,
                                Math.min(80, Number(event.target.value)),
                              ),
                            )
                          }
                        />
                      </label>
                    )}
                    {(tool === "text" || tool === "brush-text") && (
                      <label>
                        <span className={styles.optionLabel}>Text size</span>{" "}
                        <input
                          aria-label="Text size"
                          type="number"
                          min="8"
                          max="160"
                          value={textSize}
                          onChange={(event) =>
                            setTextSize(
                              Math.max(
                                8,
                                Math.min(160, Number(event.target.value)),
                              ),
                            )
                          }
                        />
                      </label>
                    )}
                  </>
                )}
                <span className={styles.optionSpacer} />
                <ToolButton
                  label="Export settings"
                  icon={PanelRight}
                  aria-expanded={exportOpen}
                  onClick={() => setExportOpen(!exportOpen)}
                />
              </div>
              <div className={styles.canvasArea}>
                <div className={styles.srOnly} role="status">
                  {guidance}
                </div>
                <ImageViewport
                  ref={viewport}
                  controlsContainer={zoomControls}
                  width={width}
                  height={height}
                  fill
                  overlay={
                    textPoint ? (
                      <div
                        className={styles.inlineEditor}
                        style={{
                          left: textPoint.x,
                          top: textPoint.y,
                          maxWidth: Math.max(40, width - textPoint.x),
                        }}
                        onPointerDown={(event) => event.stopPropagation()}
                      >
                        <textarea
                          autoFocus
                          wrap="off"
                          onCompositionStart={() => {
                            composing.current = true;
                          }}
                          onCompositionEnd={() => {
                            composing.current = false;
                          }}
                          aria-label="Annotation text"
                          placeholder="Type here"
                          maxLength={500}
                          value={text}
                          rows={Math.max(1, text.split("\n").length)}
                          style={{
                            fontSize: textSize,
                            color,
                            width: Math.max(
                              40,
                              Math.min(width - textPoint.x, textWidth),
                            ),
                          }}
                          onChange={(event) => setText(event.target.value)}
                          onKeyDown={(event) => {
                            if (
                              event.nativeEvent.isComposing ||
                              composing.current
                            )
                              return;
                            if (event.key === "Escape") {
                              event.preventDefault();
                              event.stopPropagation();
                              clearGesture();
                            } else if (
                              event.key === "Enter" &&
                              (event.ctrlKey || event.metaKey)
                            ) {
                              event.preventDefault();
                              event.stopPropagation();
                              commitText();
                            }
                          }}
                        />
                      </div>
                    ) : undefined
                  }
                  drawing={tool !== "pan"}
                  onDrawStart={begin}
                  onDrawMove={move}
                  onDrawEnd={end}
                  onDrawCancel={clearGesture}
                >
                  <canvas
                    ref={preview}
                    tabIndex={-1}
                    aria-label="Image annotation preview"
                  />
                </ImageViewport>
                {textPoint && (
                  <div className={styles.textActions}>
                    <KitButton disabled={!text.trim()} onClick={commitText}>
                      Commit text
                    </KitButton>
                    <KitButton onClick={clearGesture}>Cancel text</KitButton>
                  </div>
                )}
                <details className={styles.shortcuts}>
                  <summary>Keyboard shortcuts</summary>
                  <p>
                    V Pan · B Brush · T Text · Shift+B Brush + Text · L Line · A
                    Arrow · E Ellipse · R Rectangle. Ctrl/Cmd+Z Undo ·
                    Ctrl/Cmd+Shift+Z or Ctrl+Y Redo. +/− Zoom · 0 Fit · 1 Actual
                    size. Enter attaches the edited image without sending a
                    message. In inline text, Enter adds a line; Ctrl/Cmd+Enter
                    commits. Escape cancels a pending gesture, then closes the
                    editor.
                  </p>
                </details>
                {annotations.length >= MAX_ANNOTATIONS && (
                  <p role="status">
                    Annotation limit reached. Undo or Reset to continue.
                  </p>
                )}
              </div>
              {exportOpen && (
                <aside
                  className={styles.exportPanel}
                  aria-label="Export settings panel"
                >
                  <h3>Export</h3>
                  <div className={styles.settings}>
                    <label>
                      Format{" "}
                      <select
                        aria-label="Export format"
                        value={settings.format}
                        onChange={(event) =>
                          setOption(
                            "format",
                            event.target.value as ExportSettings["format"],
                          )
                        }
                      >
                        <option value="image/png">PNG</option>
                        <option value="image/webp">WebP</option>
                        <option value="image/jpeg">JPEG</option>
                      </select>
                    </label>
                    <label>
                      Quality {settings.quality}%
                      <input
                        aria-label="Export quality"
                        type="range"
                        min="1"
                        max="100"
                        disabled={settings.format === "image/png"}
                        value={settings.quality}
                        onChange={(event) =>
                          setOption("quality", Number(event.target.value))
                        }
                      />
                    </label>
                    <label>
                      Resolution {settings.scale}%
                      <input
                        aria-label="Resolution scale"
                        type="range"
                        min="10"
                        max="100"
                        value={settings.scale}
                        onChange={(event) =>
                          setOption("scale", Number(event.target.value))
                        }
                      />
                    </label>
                    <label>
                      <input
                        type="checkbox"
                        checked={settings.target !== null}
                        onChange={(event) =>
                          setOption(
                            "target",
                            event.target.checked ? 512 * 1024 : null,
                          )
                        }
                      />{" "}
                      Target file size (best effort)
                    </label>
                    {settings.target !== null && (
                      <>
                        <label>
                          Target{" "}
                          {Math.round(
                            (settings.target /
                              (targetUnit === "KB" ? 1024 : 1048576)) *
                              100,
                          ) / 100}{" "}
                          {targetUnit}
                          <input
                            aria-label="Target file size"
                            type="range"
                            min={targetUnit === "KB" ? "1" : String(1 / 1024)}
                            max={targetUnit === "KB" ? "32768" : "32"}
                            step={targetUnit === "KB" ? "1" : String(1 / 1024)}
                            value={
                              settings.target /
                              (targetUnit === "KB" ? 1024 : 1048576)
                            }
                            onChange={(event) =>
                              setOption(
                                "target",
                                Math.round(
                                  Number(event.target.value) *
                                    (targetUnit === "KB" ? 1024 : 1048576),
                                ),
                              )
                            }
                          />
                        </label>
                        <select
                          aria-label="Target size unit"
                          value={targetUnit}
                          onChange={(event) =>
                            setTargetUnit(event.target.value)
                          }
                        >
                          <option>KB</option>
                          <option>MB</option>
                        </select>
                      </>
                    )}
                  </div>
                  {settings.format === "image/png" && (
                    <p>
                      PNG is lossless; quality does not reduce PNG size. Use
                      resolution to reduce it.
                    </p>
                  )}
                  {settings.format === "image/jpeg" && (
                    <p>JPEG removes transparency by compositing onto white.</p>
                  )}
                  {flattenWarning && (
                    <label className={styles.warning}>
                      <input
                        type="checkbox"
                        checked={acknowledged}
                        onChange={(event) =>
                          setAcknowledged(event.target.checked)
                        }
                      />{" "}
                      This format may contain animation. Editing/exporting
                      flattens it to one frame. I accept losing animation.
                    </label>
                  )}
                </aside>
              )}
            </>
          )}
        </div>
      </KitDialog>
    </div>
  );
}
