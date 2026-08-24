import React, { useEffect, useState, useId, useCallback, useRef } from "react";
import { Code, Copy, Eye, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import { PreTag } from "./Pre";
import styles from "./Markdown.module.css";
import diagramStyles from "./DiagramBlock.module.css";
import classNames from "classnames";
import { useAppearance } from "../../hooks/useAppearance";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";
import { makeCrispSvg, parseSvgMeta, type SvgMeta } from "./renderUtils";

type MermaidTheme = "dark" | "light";
type FlowchartRenderer = "dagre-wrapper" | "elk";

let mermaidInitializedConfig: {
  theme: MermaidTheme;
  flowchartRenderer: FlowchartRenderer;
} | null = null;
let mermaidElkAvailable: boolean | null = null;
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

function isRenderTimeout(error: unknown): boolean {
  return (
    error instanceof Error &&
    error.message.startsWith("Mermaid render timed out after ")
  );
}

function isFlowchart(
  mermaid: (typeof import("mermaid"))["default"],
  code: string,
): boolean {
  try {
    const type = mermaid.detectType(code);
    return (
      type === "flowchart" ||
      type === "flowchart-v2" ||
      type === "flowchart-elk"
    );
  } catch {
    return false;
  }
}

function sourceRequestsElkLayout(code: string): boolean {
  const frontmatter =
    /^\s*---\s*\r?\n([\s\S]*?)\r?\n---\s*(?:\r?\n|$)/u.exec(code)?.[1] ?? "";
  const initDirective = /^\s*%%\{init:\s*(.*?)\s*\}%%/su.exec(code)?.[1] ?? "";

  return (
    /(?:^|\n)\s*flowchart-elk\b/imu.test(code) ||
    /\b(?:defaultRenderer|layout)\b[^,\n}]*\belk\b/iu.test(frontmatter) ||
    /\b(?:defaultRenderer|layout)\b[^,\n}]*\belk\b/iu.test(initDirective)
  );
}

async function canUseElkLayout(
  mermaid: (typeof import("mermaid"))["default"],
): Promise<boolean> {
  if (mermaidElkAvailable !== null) return mermaidElkAvailable;

  try {
    const elkLayouts = (
      await withTimeout(
        import("@mermaid-js/layout-elk"),
        MERMAID_RENDER_TIMEOUT_MS,
      )
    ).default;
    mermaid.registerLayoutLoaders(elkLayouts);
    mermaidElkAvailable = true;
  } catch {
    mermaidElkAvailable = false;
    return false;
  }

  return mermaidElkAvailable;
}

function initializeMermaid(
  mermaid: (typeof import("mermaid"))["default"],
  theme: MermaidTheme,
  flowchartRenderer: FlowchartRenderer,
): void {
  if (
    mermaidInitializedConfig?.theme === theme &&
    mermaidInitializedConfig.flowchartRenderer === flowchartRenderer
  ) {
    return;
  }

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
    flowchart: {
      defaultRenderer: flowchartRenderer,
      curve: "linear",
      htmlLabels: false,
      nodeSpacing: 70,
      padding: 16,
      rankSpacing: 90,
      wrappingWidth: 240,
    },
  });
  mermaidInitializedConfig = { theme, flowchartRenderer };
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
  const task = mermaidTaskQueue.then(async () => {
    const mermaidModule = await withTimeout(
      import("mermaid"),
      MERMAID_RENDER_TIMEOUT_MS,
    );
    const mermaid = mermaidModule.default;
    const flowchart = isFlowchart(mermaid, code);
    const flowchartRenderer: FlowchartRenderer =
      flowchart && (await canUseElkLayout(mermaid)) ? "elk" : "dagre-wrapper";
    const retriesWithDagre =
      flowchartRenderer === "elk" && !sourceRequestsElkLayout(code);
    initializeMermaid(mermaid, theme, flowchartRenderer);

    try {
      return await withTimeout(
        mermaid.render(id, code),
        MERMAID_RENDER_TIMEOUT_MS,
      );
    } catch (error) {
      if (!retriesWithDagre || isRenderTimeout(error)) throw error;

      document.getElementById(id)?.remove();
      initializeMermaid(mermaid, theme, "dagre-wrapper");
      return withTimeout(mermaid.render(id, code), MERMAID_RENDER_TIMEOUT_MS);
    }
  });
  mermaidTaskQueue = task.then(
    () => undefined,
    () => undefined,
  );
  return task;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;

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
  const [scale, setScale] = useState(1);

  const canvasRef = useRef<HTMLDivElement | null>(null);
  const renderSeqRef = useRef(0);

  const uniqueId = useId().replace(/:/g, "_");
  const { appearance } = useAppearance();
  const theme: MermaidTheme = appearance === "dark" ? "dark" : "light";

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
          const canvas = canvasRef.current;
          if (canvas) {
            canvas.scrollLeft = 0;
            canvas.scrollTop = 0;
          }
          setRawSvg(svg);
          setSvgMeta(meta);
          setError(null);
          setScale(1);
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

  const handleToggleSource = useCallback(() => {
    setShowSource((v) => !v);
  }, []);

  const handleCopy = useCallback(() => {
    onCopyClick?.(code);
  }, [onCopyClick, code]);

  const setZoom = useCallback(
    (nextScale: number) => {
      const canvas = canvasRef.current;
      const previousScale = scale;
      if (canvas && previousScale > 0) {
        const ratio = nextScale / previousScale;
        requestAnimationFrame(() => {
          canvas.scrollLeft =
            (canvas.scrollLeft + canvas.clientWidth / 2) * ratio -
            canvas.clientWidth / 2;
          canvas.scrollTop =
            (canvas.scrollTop + canvas.clientHeight / 2) * ratio -
            canvas.clientHeight / 2;
        });
      }
      setScale(nextScale);
    },
    [scale],
  );

  const handleResetZoom = useCallback(() => setZoom(1), [setZoom]);

  const zoomBy = useCallback(
    (factor: number) => {
      setZoom(clampScale(scale * factor));
    },
    [scale, setZoom],
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
                    onClick={handleResetZoom}
                    aria-label="Reset zoom to 100%"
                    icon={RotateCcw}
                  />
                </Tooltip.Trigger>
                <Tooltip.Content>Reset zoom to 100%</Tooltip.Content>
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
            data-testid="mermaid-canvas"
            ref={canvasRef}
            tabIndex={0}
            aria-label="Mermaid diagram"
            className={classNames("scrollX", diagramStyles.diagram_canvas)}
          >
            <div
              className={diagramStyles.diagram_render}
              style={{
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
