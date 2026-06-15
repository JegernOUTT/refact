import React, { useEffect, useState, useId, useCallback, useRef } from "react";
import { Code, Copy, Eye, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import { PreTag } from "./Pre";
import styles from "./Markdown.module.css";
import diagramStyles from "./DiagramBlock.module.css";
import classNames from "classnames";
import { useAppearance } from "../../hooks/useAppearance";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";
import {
  clampPan,
  makeCrispSvg,
  parseSvgMeta,
  type SvgMeta,
} from "./renderUtils";

type MermaidTheme = "dark" | "light";

let mermaidInitializedTheme: MermaidTheme | null = null;
let mermaidTaskQueue: Promise<unknown> = Promise.resolve();
const REPORTED_MERMAID_ERRORS = new Map<string, number>();
const MERMAID_ERROR_REPORT_INTERVAL_MS = 60_000;
const MAX_REPORTED_MERMAID_ERRORS = 50;

const FALLBACK_FONT_STACK =
  'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';

const MERMAID_THEME_TOKENS = {
  primaryColor: {
    token: "--rf-surface-2",
    dark: "#1a1c22",
    light: "#eef1f5",
  },
  primaryTextColor: {
    token: "--rf-color-fg",
    dark: "#f5f7fb",
    light: "#1f2328",
  },
  primaryBorderColor: {
    token: "--rf-border-strong",
    dark: "#343946",
    light: "#c8d0dc",
  },
  lineColor: {
    token: "--rf-color-muted",
    dark: "#8b93a3",
    light: "#5f6772",
  },
  secondaryColor: {
    token: "--rf-surface-1",
    dark: "#14161b",
    light: "#f7f8fa",
  },
  tertiaryColor: { token: "--rf-bg", dark: "#0c0d0f", light: "#fcfcfd" },
  nodeTextColor: {
    token: "--rf-color-fg",
    dark: "#f5f7fb",
    light: "#1f2328",
  },
  mainBkg: { token: "--rf-surface-1", dark: "#14161b", light: "#f7f8fa" },
  nodeBorder: {
    token: "--rf-border-strong",
    dark: "#343946",
    light: "#c8d0dc",
  },
  clusterBkg: {
    token: "--rf-surface-2",
    dark: "#1a1c22",
    light: "#eef1f5",
  },
  clusterBorder: { token: "--rf-border", dark: "#282d38", light: "#d9dee7" },
  titleColor: {
    token: "--rf-color-fg",
    dark: "#f5f7fb",
    light: "#1f2328",
  },
  edgeLabelBackground: {
    token: "--rf-bg",
    dark: "#0c0d0f",
    light: "#fcfcfd",
  },
  noteBkgColor: {
    token: "--rf-surface-2",
    dark: "#1a1c22",
    light: "#eef1f5",
  },
  noteTextColor: {
    token: "--rf-color-fg",
    dark: "#f5f7fb",
    light: "#1f2328",
  },
  noteBorderColor: {
    token: "--rf-border-strong",
    dark: "#343946",
    light: "#c8d0dc",
  },
} as const;

function getThemeRoot(): Element | null {
  if (typeof document === "undefined") return null;

  return (
    document.querySelector("[data-radix-themes], .radix-themes") ??
    document.documentElement
  );
}

function isResolvedColor(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return (
    normalized !== "" &&
    !normalized.includes("var(") &&
    !normalized.includes("color-mix(")
  );
}

function resolveTokenColor(token: string, fallback: string): string {
  if (typeof window === "undefined" || typeof document === "undefined") {
    return fallback;
  }

  const root = getThemeRoot();
  if (!root) return fallback;

  const target = root instanceof HTMLElement ? root : document.documentElement;
  const probe = document.createElement("span");
  probe.style.color = `var(${token}, ${fallback})`;
  probe.style.display = "none";
  target.append(probe);
  const resolved = window.getComputedStyle(probe).color.trim();
  probe.remove();

  if (isResolvedColor(resolved)) return resolved;

  const direct = window.getComputedStyle(root).getPropertyValue(token).trim();
  if (isResolvedColor(direct)) return direct;

  return fallback;
}

function resolveAppFontFamily(): string {
  if (typeof window === "undefined") return FALLBACK_FONT_STACK;
  const root = getThemeRoot();
  if (!root) return FALLBACK_FONT_STACK;
  const family = window.getComputedStyle(root).fontFamily.trim();
  return family !== "" ? family : FALLBACK_FONT_STACK;
}

function createMermaidThemeVariables(theme: MermaidTheme) {
  return Object.fromEntries(
    Object.entries(MERMAID_THEME_TOKENS).map(([key, config]) => [
      key,
      resolveTokenColor(config.token, config[theme]),
    ]),
  );
}

function shouldReportMermaidError(key: string): boolean {
  const now = Date.now();
  const previous = REPORTED_MERMAID_ERRORS.get(key) ?? 0;
  if (now - previous < MERMAID_ERROR_REPORT_INTERVAL_MS) return false;

  REPORTED_MERMAID_ERRORS.set(key, now);
  if (REPORTED_MERMAID_ERRORS.size > MAX_REPORTED_MERMAID_ERRORS) {
    const oldest = REPORTED_MERMAID_ERRORS.keys().next().value;
    if (oldest) REPORTED_MERMAID_ERRORS.delete(oldest);
  }
  return true;
}

const MERMAID_RENDER_TIMEOUT_MS = 15_000;

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`Mermaid render timed out after ${ms}ms`)),
      ms,
    );
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err: unknown) => {
        clearTimeout(timer);
        reject(err instanceof Error ? err : new Error(String(err)));
      },
    );
  });
}

// Serializes mermaid.initialize + mermaid.render pairs. mermaid.initialize is
// global, so without serialization two blocks rendering concurrently after a
// theme change can race and render with the wrong theme variables. Each task
// is bounded by a timeout so one hung render cannot starve every later
// diagram in the queue.
function enqueueMermaidRender(
  theme: MermaidTheme,
  id: string,
  code: string,
): Promise<{ svg: string }> {
  const task = mermaidTaskQueue.then(() =>
    withTimeout(
      (async () => {
        const mermaid = (await import("mermaid")).default;
        if (mermaidInitializedTheme !== theme) {
          const fontFamily = resolveAppFontFamily();
          mermaid.initialize({
            startOnLoad: false,
            theme: theme === "dark" ? "dark" : "default",
            securityLevel: "strict",
            fontFamily,
            themeVariables: {
              ...createMermaidThemeVariables(theme),
              fontFamily,
            },
            flowchart: { curve: "basis", padding: 16, htmlLabels: false },
          });
          mermaidInitializedTheme = theme;
        }
        return mermaid.render(id, code);
      })(),
      MERMAID_RENDER_TIMEOUT_MS,
    ),
  );
  mermaidTaskQueue = task.then(
    () => undefined,
    () => undefined,
  );
  return task;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;
const ZOOM_SENSITIVITY = 0.003;
const FIT_PADDING = 16;

function clampScale(s: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));
}

export type MermaidBlockProps = {
  code: string;
  onCopyClick?: (str: string) => void;
};

const _MermaidBlock: React.FC<MermaidBlockProps> = ({ code, onCopyClick }) => {
  const [rawSvg, setRawSvg] = useState<string | null>(null);
  const [svgMeta, setSvgMeta] = useState<SvgMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showSource, setShowSource] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [panX, setPanX] = useState(0);
  const [panY, setPanY] = useState(0);
  const [scale, setScale] = useState(1);

  const canvasRef = useRef<HTMLDivElement | null>(null);
  const canvasCleanupRef = useRef<(() => void) | null>(null);
  const dragStart = useRef({ x: 0, y: 0, px: 0, py: 0 });
  const userInteractedRef = useRef(false);
  const renderSeqRef = useRef(0);

  const uniqueId = useId().replace(/:/g, "_");
  const { appearance } = useAppearance();
  const theme: MermaidTheme = appearance === "dark" ? "dark" : "light";

  const fitToContainer = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !svgMeta) return;

    const cw = canvas.clientWidth;
    const ch = canvas.clientHeight;
    const { width: sw, height: sh } = svgMeta;

    const availW = cw - FIT_PADDING * 2;
    const availH = ch - FIT_PADDING * 2;
    if (sw <= 0 || sh <= 0 || availW <= 0 || availH <= 0) return;

    const s = clampScale(Math.min(availW / sw, availH / sh));

    setPanX((cw - sw * s) / 2);
    setPanY((ch - sh * s) / 2);
    setScale(s);
  }, [svgMeta]);

  const fitRef = useRef(fitToContainer);
  fitRef.current = fitToContainer;

  useEffect(() => {
    let cancelled = false;
    const renderId = `mermaid_${uniqueId}_${++renderSeqRef.current}`;

    const renderDiagram = async () => {
      try {
        const { svg } = await enqueueMermaidRender(
          theme,
          renderId,
          code.trim(),
        );

        if (!cancelled) {
          const meta = parseSvgMeta(svg);
          userInteractedRef.current = false;
          setRawSvg(svg);
          setSvgMeta(meta);
          setError(null);
        }
      } catch (err) {
        // Mermaid can leave a temporary element with the render id in the
        // document on failure. The id is unique per render attempt, so this
        // never touches the SVG currently on screen.
        document.getElementById(renderId)?.remove();
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          setError(msg);
          const reportKey = `${msg}\n${code}`.slice(0, 2000);
          if (shouldReportMermaidError(reportKey)) {
            void reportBuddyFrontendError({
              source: "mermaid_render",
              error: `${msg}\n\n${code}`,
              sourceFile: "frontend/mermaid_render",
              toolName: "mermaid_render",
            });
          }
          setRawSvg(null);
          setSvgMeta(null);
        }
      }
    };

    const timer = setTimeout(() => {
      void renderDiagram();
    }, 100);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [code, uniqueId, theme]);

  useEffect(() => {
    if (!rawSvg || !svgMeta) return;
    const raf = requestAnimationFrame(() => {
      if (!userInteractedRef.current) fitRef.current();
    });
    return () => cancelAnimationFrame(raf);
  }, [rawSvg, svgMeta]);

  const stateRef = useRef({ scale, panX, panY, svgMeta });
  stateRef.current = { scale, panX, panY, svgMeta };

  const canvasCallbackRef = useCallback((node: HTMLDivElement | null) => {
    if (canvasCleanupRef.current) {
      canvasCleanupRef.current();
      canvasCleanupRef.current = null;
    }

    canvasRef.current = node;
    if (!node) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const { scale: s, panX: px, panY: py, svgMeta: meta } = stateRef.current;
      if (!meta) return;

      userInteractedRef.current = true;

      const rect = node.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;

      const delta = -e.deltaY * ZOOM_SENSITIVITY;
      const newScale = clampScale(s * (1 + delta));
      const ratio = newScale / s;

      setPanX(
        clampPan(
          mx - (mx - px) * ratio,
          node.clientWidth,
          meta.width * newScale,
        ),
      );
      setPanY(
        clampPan(
          my - (my - py) * ratio,
          node.clientHeight,
          meta.height * newScale,
        ),
      );
      setScale(newScale);
    };

    node.addEventListener("wheel", onWheel, { passive: false });

    let resizeObserver: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      // Refit whenever the canvas is (re)laid out — window resizes, panel
      // resizes, and virtualization remounts — until the user pans or zooms.
      resizeObserver = new ResizeObserver(() => {
        if (!userInteractedRef.current) fitRef.current();
      });
      resizeObserver.observe(node);
    }

    canvasCleanupRef.current = () => {
      node.removeEventListener("wheel", onWheel);
      resizeObserver?.disconnect();
    };
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      userInteractedRef.current = true;
      setDragging(true);
      dragStart.current = { x: e.clientX, y: e.clientY, px: panX, py: panY };
    },
    [panX, panY],
  );

  useEffect(() => {
    if (!dragging) return;

    const handleMove = (e: MouseEvent) => {
      const canvas = canvasRef.current;
      const { scale: s, svgMeta: meta } = stateRef.current;
      const nx = dragStart.current.px + e.clientX - dragStart.current.x;
      const ny = dragStart.current.py + e.clientY - dragStart.current.y;
      if (canvas && meta) {
        setPanX(clampPan(nx, canvas.clientWidth, meta.width * s));
        setPanY(clampPan(ny, canvas.clientHeight, meta.height * s));
      } else {
        setPanX(nx);
        setPanY(ny);
      }
    };

    const handleUp = () => setDragging(false);

    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [dragging]);

  const handleToggleSource = useCallback(() => {
    setShowSource((v) => !v);
  }, []);

  const handleCopy = useCallback(() => {
    onCopyClick?.(code);
  }, [onCopyClick, code]);

  const handleFit = useCallback(() => {
    userInteractedRef.current = false;
    fitToContainer();
  }, [fitToContainer]);

  const zoomBy = useCallback(
    (factor: number) => {
      const canvas = canvasRef.current;
      const meta = stateRef.current.svgMeta;
      if (!canvas || !meta) return;
      userInteractedRef.current = true;
      const cx = canvas.clientWidth / 2;
      const cy = canvas.clientHeight / 2;
      const newScale = clampScale(scale * factor);
      const ratio = newScale / scale;
      setPanX(
        clampPan(
          cx - (cx - panX) * ratio,
          canvas.clientWidth,
          meta.width * newScale,
        ),
      );
      setPanY(
        clampPan(
          cy - (cy - panY) * ratio,
          canvas.clientHeight,
          meta.height * newScale,
        ),
      );
      setScale(newScale);
    },
    [scale, panX, panY],
  );

  const handleZoomIn = useCallback(() => zoomBy(1.4), [zoomBy]);
  const handleZoomOut = useCallback(() => zoomBy(1 / 1.4), [zoomBy]);

  const zoomPercent = Math.round(scale * 100);

  if (error) {
    return (
      <div className={styles.shiki_wrapper}>
        <PreTag className={styles.shiki_pre}>
          <code className={classNames(styles.code, styles.code_block)}>
            {code}
          </code>
        </PreTag>
      </div>
    );
  }

  const crispSvg =
    rawSvg && svgMeta ? makeCrispSvg(rawSvg, svgMeta.viewBox) : null;
  const displayW = svgMeta ? svgMeta.width * scale : 0;
  const displayH = svgMeta ? svgMeta.height * scale : 0;

  return (
    <div className={styles.shiki_wrapper}>
      <div className={diagramStyles.diagram_container}>
        <div className={diagramStyles.diagram_toolbar}>
          {!showSource && crispSvg && (
            <>
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <IconButton
                    size="sm"
                    variant="ghost"
                    onClick={handleZoomIn}
                    aria-label="Zoom in"
                    icon={ZoomIn}
                  />
                </Tooltip.Trigger>
                <Tooltip.Content>Zoom in</Tooltip.Content>
              </Tooltip>
              <span className={diagramStyles.diagram_zoom_info}>
                {zoomPercent}%
              </span>
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <IconButton
                    size="sm"
                    variant="ghost"
                    onClick={handleZoomOut}
                    aria-label="Zoom out"
                    icon={ZoomOut}
                  />
                </Tooltip.Trigger>
                <Tooltip.Content>Zoom out</Tooltip.Content>
              </Tooltip>
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <IconButton
                    size="sm"
                    variant="ghost"
                    onClick={handleFit}
                    aria-label="Fit to view"
                    icon={RotateCcw}
                  />
                </Tooltip.Trigger>
                <Tooltip.Content>Fit to view</Tooltip.Content>
              </Tooltip>
            </>
          )}
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                onClick={handleToggleSource}
                aria-label={showSource ? "Show diagram" : "Show source"}
                icon={showSource ? Eye : Code}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>
              {showSource ? "Show diagram" : "Show source"}
            </Tooltip.Content>
          </Tooltip>
          {onCopyClick && (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="ghost"
                  onClick={handleCopy}
                  aria-label="Copy mermaid source"
                  icon={Copy}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>Copy source</Tooltip.Content>
            </Tooltip>
          )}
        </div>
        {showSource ? (
          <div className="scrollX">
            <PreTag className={styles.shiki_pre}>
              <code className={classNames(styles.code, styles.code_block)}>
                {code}
              </code>
            </PreTag>
          </div>
        ) : crispSvg ? (
          <div
            ref={canvasCallbackRef}
            className={classNames(
              diagramStyles.diagram_canvas,
              dragging && diagramStyles.diagram_canvas_dragging,
            )}
            onMouseDown={handleMouseDown}
          >
            <div
              className={diagramStyles.diagram_render}
              style={{
                position: "absolute",
                left: panX,
                top: panY,
                width: displayW,
                height: displayH,
              }}
              dangerouslySetInnerHTML={{ __html: crispSvg }}
            />
          </div>
        ) : rawSvg ? (
          <div
            className={diagramStyles.diagram_fallback}
            dangerouslySetInnerHTML={{ __html: rawSvg }}
          />
        ) : (
          <div className={diagramStyles.diagram_loading}>Rendering…</div>
        )}
      </div>
    </div>
  );
};

export const MermaidBlock = React.memo(_MermaidBlock);
