import { useAppearance } from "../../../hooks";
import { formatCompact } from "./formatters";

/**
 * Concrete fallbacks per appearance. ECharts cannot parse `currentColor` or an
 * unresolved `var(...)`; when it receives one it silently falls back to its
 * built-in cobalt/green palette that clashes with the app theme. We therefore
 * always resolve tokens to a real color string, defaulting to these values.
 */
const FALLBACK_DARK = {
  fg: "rgba(255,255,255,0.92)",
  muted: "rgba(255,255,255,0.48)",
  faint: "rgba(255,255,255,0.28)",
  grid: "rgba(255,255,255,0.08)",
  axis: "rgba(255,255,255,0.32)",
  surface: "#16181d",
  border: "rgba(255,255,255,0.11)",
  accent: "#7f93d8",
  success: "#5fae8b",
  warning: "#cda04e",
  danger: "#d8736d",
  info: "#6cb6c9",
  palette: [
    "#7f93d8",
    "#5fae8b",
    "#cda04e",
    "#d8736d",
    "#6cb6c9",
    "#b08ad1",
    "#d39a6a",
    "#8fa3b8",
  ],
} as const;

const FALLBACK_LIGHT = {
  fg: "rgba(0,0,0,0.88)",
  muted: "rgba(0,0,0,0.55)",
  faint: "rgba(0,0,0,0.4)",
  grid: "rgba(0,0,0,0.09)",
  axis: "rgba(0,0,0,0.45)",
  surface: "#ffffff",
  border: "rgba(0,0,0,0.14)",
  accent: "#5566c4",
  success: "#2f9e74",
  warning: "#b8862f",
  danger: "#cc5b54",
  info: "#3f93a8",
  palette: [
    "#5566c4",
    "#2f9e74",
    "#b8862f",
    "#cc5b54",
    "#3f93a8",
    "#8f63b8",
    "#c07a3f",
    "#5e7490",
  ],
} as const;

function isUsable(value: string): boolean {
  return (
    value.trim() !== "" &&
    !value.includes("currentColor") &&
    !value.includes("var(")
  );
}

/**
 * Resolve design tokens from the element that actually carries the active
 * `data-appearance` (the Radix Theme root), NOT `document.documentElement`.
 * `useToken`/`useTokens` read the document root, which never receives the
 * nested theme's appearance, so chart text would render with the wrong
 * (often invisible) color in dark mode.
 */
function themedElement(appearance: "light" | "dark"): Element | null {
  if (typeof document === "undefined") return null;
  return (
    document.querySelector(`[data-appearance="${appearance}"]`) ??
    document.querySelector(".radix-themes") ??
    document.documentElement
  );
}

export interface ChartTheme {
  fg: string;
  muted: string;
  faint: string;
  grid: string;
  axis: string;
  accent: string;
  success: string;
  warning: string;
  danger: string;
  info: string;
  /** Distinct categorical palette for series (8 hues). */
  palette: string[];
  tooltip: { bg: string; border: string; text: string };
}

export function useChartTheme(): ChartTheme {
  // Reactive to in-app theme toggles, host theme changes, and system scheme.
  const { appearance } = useAppearance();
  const fallback = appearance === "light" ? FALLBACK_LIGHT : FALLBACK_DARK;
  const el = themedElement(appearance);

  const read = (name: string, fb: string): string => {
    if (!el || typeof window === "undefined") return fb;
    const value = window.getComputedStyle(el).getPropertyValue(name).trim();
    return isUsable(value) ? value : fb;
  };

  const palette = fallback.palette.map((fb, i) =>
    read(`--rf-chart-${i + 1}`, fb),
  );
  const fg = read("--rf-color-fg", fallback.fg);

  return {
    fg,
    muted: read("--rf-color-muted", fallback.muted),
    faint: read("--rf-color-faint", fallback.faint),
    grid: read("--rf-chart-grid", fallback.grid),
    axis: read("--rf-chart-axis", fallback.axis),
    accent: read("--rf-chart-1", fallback.accent),
    success: read("--rf-color-success", fallback.success),
    warning: read("--rf-color-warning", fallback.warning),
    danger: read("--rf-color-danger", fallback.danger),
    info: read("--rf-chart-5", fallback.info),
    palette,
    tooltip: {
      bg: read("--rf-surface-overlay", fallback.surface),
      border: read("--rf-border-strong", fallback.border),
      text: fg,
    },
  };
}

/** Standard tooltip block, themed. */
export function chartTooltip(
  theme: ChartTheme,
  trigger: "axis" | "item" = "axis",
  extra: Record<string, unknown> = {},
) {
  return {
    trigger,
    axisPointer: trigger === "axis" ? { type: "shadow" } : undefined,
    backgroundColor: theme.tooltip.bg,
    borderColor: theme.tooltip.border,
    borderWidth: 1,
    textStyle: { color: theme.tooltip.text },
    ...extra,
  };
}

export function chartGrid(extra: Record<string, unknown> = {}) {
  return {
    left: "3%",
    right: "4%",
    bottom: "3%",
    top: "16%",
    containLabel: true,
    ...extra,
  };
}

export function chartLegend(
  theme: ChartTheme,
  extra: Record<string, unknown> = {},
) {
  return {
    textStyle: { color: theme.muted },
    inactiveColor: theme.faint,
    icon: "roundRect",
    itemWidth: 10,
    itemHeight: 10,
    ...extra,
  };
}

export function categoryAxis(
  theme: ChartTheme,
  data: string[],
  extra: Record<string, unknown> = {},
) {
  return {
    type: "category",
    data,
    axisLine: { lineStyle: { color: theme.axis } },
    axisTick: { show: false },
    axisLabel: { color: theme.muted },
    ...extra,
  };
}

export function valueAxis(
  theme: ChartTheme,
  extra: Record<string, unknown> = {},
) {
  return {
    type: "value",
    axisLine: { show: false },
    axisLabel: {
      color: theme.muted,
      formatter: (value: number) => formatCompact(value),
    },
    splitLine: { lineStyle: { color: theme.grid } },
    ...extra,
  };
}

/** Percentage value axis (0-100). */
export function percentAxis(
  theme: ChartTheme,
  extra: Record<string, unknown> = {},
) {
  return {
    type: "value",
    min: 0,
    max: 100,
    axisLine: { show: false },
    axisLabel: { color: theme.muted, formatter: "{value}%" },
    splitLine: { lineStyle: { color: theme.grid } },
    ...extra,
  };
}
