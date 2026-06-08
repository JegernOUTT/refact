import React, { useEffect, useState, useId, useCallback, useRef } from "react";
import { Code, Copy, Eye, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import { PreTag } from "./Pre";
import styles from "./Markdown.module.css";
import diagramStyles from "./DiagramBlock.module.css";
import classNames from "classnames";
import { useAppearance } from "../../hooks/useAppearance";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";

type MermaidTheme = "dark" | "light";

let mermaidInitialized: MermaidTheme | null = null;
const REPORTED_MERMAID_ERRORS = new Map<string, number>();
const MERMAID_ERROR_REPORT_INTERVAL_MS = 60_000;
const MAX_REPORTED_MERMAID_ERRORS = 50;

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

async function getMermaid(theme: MermaidTheme) {
  const mermaid = (await import("mermaid")).default;
  if (mermaidInitialized !== theme) {
    mermaid.initialize({
      startOnLoad: false,
      theme: theme === "dark" ? "dark" : "default",
      securityLevel: "strict",
      fontFamily:
        'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
      themeVariables: createMermaidThemeVariables(theme),
      flowchart: { curve: "basis", padding: 16, htmlLabels: false },
    });
    mermaidInitialized = theme;
  }
  return mermaid;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;
const ZOOM_SENSITIVITY = 0.003;

function clampScale(s: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));
}

type SvgMeta = { viewBox: string; width: number; height: number };

function parseSvgMeta(svgStr: string): SvgMeta | null {
  const parser = new DOMParser();
  const doc = parser.parseFromString(svgStr, "image/svg+xml");
  const svg = doc.querySelector("svg");
  if (!svg) return null;

  const vbAttr = svg.getAttribute("viewBox");
  const widthAttr = svg.getAttribute("width") ?? "";
  const heightAttr = svg.getAttribute("height") ?? "";

  const vbW = svg.viewBox.baseVal.width;
  const vbH = svg.viewBox.baseVal.height;

  const isAbsW = widthAttr !== "" && !widthAttr.includes("%");
  const isAbsH = heightAttr !== "" && !heightAttr.includes("%");

  const w = isAbsW ? parseFloat(widthAttr) || vbW : vbW;
  const h = isAbsH ? parseFloat(heightAttr) || vbH : vbH;

  const viewBox = vbAttr ?? (w && h ? `0 0 ${w} ${h}` : null);
  if (!viewBox || !w || !h) return null;

  return { viewBox, width: w, height: h };
}

function makeCrispSvg(svgStr: string, vb: string): string {
  return svgStr
    .replace(/\s*width="[^"]*"/, "")
    .replace(/\s*height="[^"]*"/, "")
    .replace(/\s*style="[^"]*"/, "")
    .replace(/\s*viewBox="[^"]*"/, "")
    .replace("<svg", `<svg viewBox="${vb}" width="100%" height="100%"`);
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
  const wheelCleanupRef = useRef<(() => void) | null>(null);
  const dragStart = useRef({ x: 0, y: 0, px: 0, py: 0 });
  const fittedRef = useRef(false);

  const uniqueId = useId().replace(/:/g, "_");
  const { appearance } = useAppearance();
  const theme = appearance === "dark" ? "dark" : "light";

  const fitToContainer = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !svgMeta) return;

    const cw = canvas.clientWidth;
    const ch = canvas.clientHeight;
    const { width: sw, height: sh } = svgMeta;

    if (sw === 0 || sh === 0) return;

    const pad = 16;
    const fitScale = Math.min((cw - pad * 2) / sw, (ch - pad * 2) / sh);
    const s = clampScale(fitScale);

    setPanX((cw - sw * s) / 2);
    setPanY((ch - sh * s) / 2);
    setScale(s);
  }, [svgMeta]);

  useEffect(() => {
    let cancelled = false;

    const renderDiagram = async () => {
      try {
        const mermaid = await getMermaid(theme);
        const { svg } = await mermaid.render(
          `mermaid_${uniqueId}`,
          code.trim(),
        );

        if (!cancelled) {
          const meta = parseSvgMeta(svg);
          setRawSvg(svg);
          setSvgMeta(meta);
          setError(null);
          fittedRef.current = false;
        }
      } catch (err) {
        document.getElementById(`mermaid_${uniqueId}`)?.remove();
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
    if (rawSvg && svgMeta && !fittedRef.current) {
      requestAnimationFrame(() => {
        fitToContainer();
        fittedRef.current = true;
      });
    }
  }, [rawSvg, svgMeta, fitToContainer]);

  const stateRef = useRef({ scale, panX, panY, svgMeta });
  stateRef.current = { scale, panX, panY, svgMeta };

  const canvasCallbackRef = useCallback((node: HTMLDivElement | null) => {
    if (wheelCleanupRef.current) {
      wheelCleanupRef.current();
      wheelCleanupRef.current = null;
    }

    canvasRef.current = node;
    if (!node) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const { scale: s, panX: px, panY: py, svgMeta: meta } = stateRef.current;
      if (!meta) return;

      const rect = node.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;

      const delta = -e.deltaY * ZOOM_SENSITIVITY;
      const newScale = clampScale(s * (1 + delta));
      const ratio = newScale / s;

      setPanX(mx - (mx - px) * ratio);
      setPanY(my - (my - py) * ratio);
      setScale(newScale);
    };

    node.addEventListener("wheel", onWheel, { passive: false });
    wheelCleanupRef.current = () => node.removeEventListener("wheel", onWheel);
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      setDragging(true);
      dragStart.current = { x: e.clientX, y: e.clientY, px: panX, py: panY };
    },
    [panX, panY],
  );

  useEffect(() => {
    if (!dragging) return;

    const handleMove = (e: MouseEvent) => {
      setPanX(dragStart.current.px + e.clientX - dragStart.current.x);
      setPanY(dragStart.current.py + e.clientY - dragStart.current.y);
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

  const handleZoomIn = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const cx = canvas.clientWidth / 2;
    const cy = canvas.clientHeight / 2;
    const newScale = clampScale(scale * 1.4);
    const ratio = newScale / scale;
    setPanX(cx - (cx - panX) * ratio);
    setPanY(cy - (cy - panY) * ratio);
    setScale(newScale);
  }, [scale, panX, panY]);

  const handleZoomOut = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const cx = canvas.clientWidth / 2;
    const cy = canvas.clientHeight / 2;
    const newScale = clampScale(scale / 1.4);
    const ratio = newScale / scale;
    setPanX(cx - (cx - panX) * ratio);
    setPanY(cy - (cy - panY) * ratio);
    setScale(newScale);
  }, [scale, panX, panY]);

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
                    onClick={fitToContainer}
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
            <Tooltip.Content>{showSource ? "Show diagram" : "Show source"}</Tooltip.Content>
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
          <PreTag className={styles.shiki_pre}>
            <code className={classNames(styles.code, styles.code_block)}>
              {code}
            </code>
          </PreTag>
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
        ) : (
          <div className={diagramStyles.diagram_loading}>Rendering…</div>
        )}
      </div>
    </div>
  );
};

export const MermaidBlock = React.memo(_MermaidBlock);
