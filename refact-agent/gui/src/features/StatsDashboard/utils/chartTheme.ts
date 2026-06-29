import { useTokens } from "../../../components/ui";
import { formatCompact } from "./formatters";

/**
 * Concrete dark-theme fallbacks. ECharts cannot parse `currentColor` or an
 * unresolved `var(...)`; when it receives one it silently falls back to its
 * built-in cobalt/green palette that clashes with the app theme. We therefore
 * always resolve tokens to a real color string, defaulting to these values.
 */
const FALLBACK = {
  fg: "rgba(255,255,255,0.92)",
  muted: "rgba(255,255,255,0.48)",
  faint: "rgba(255,255,255,0.28)",
  grid: "rgba(255,255,255,0.08)",
  axis: "rgba(255,255,255,0.32)",
  surface: "#16181d",
  border: "rgba(255,255,255,0.11)",
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

const CHART_TOKENS = [
  "--rf-color-fg",
  "--rf-color-muted",
  "--rf-color-faint",
  "--rf-chart-grid",
  "--rf-chart-axis",
  "--rf-surface-overlay",
  "--rf-border-strong",
  "--rf-color-accent",
  "--rf-color-success",
  "--rf-color-warning",
  "--rf-color-danger",
  "--rf-color-info",
  "--rf-chart-1",
  "--rf-chart-2",
  "--rf-chart-3",
  "--rf-chart-4",
  "--rf-chart-5",
  "--rf-chart-6",
  "--rf-chart-7",
  "--rf-chart-8",
];

function isUsable(value: string | undefined): value is string {
  return Boolean(
    value &&
      value.trim() !== "" &&
      !value.includes("currentColor") &&
      !value.includes("var("),
  );
}

function resolve(value: string | undefined, fallback: string): string {
  return isUsable(value) ? value.trim() : fallback;
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
  const t = useTokens(CHART_TOKENS);
  const palette = [
    resolve(t["--rf-chart-1"], FALLBACK.palette[0]),
    resolve(t["--rf-chart-2"], FALLBACK.palette[1]),
    resolve(t["--rf-chart-3"], FALLBACK.palette[2]),
    resolve(t["--rf-chart-4"], FALLBACK.palette[3]),
    resolve(t["--rf-chart-5"], FALLBACK.palette[4]),
    resolve(t["--rf-chart-6"], FALLBACK.palette[5]),
    resolve(t["--rf-chart-7"], FALLBACK.palette[6]),
    resolve(t["--rf-chart-8"], FALLBACK.palette[7]),
  ];
  const fg = resolve(t["--rf-color-fg"], FALLBACK.fg);
  return {
    fg,
    muted: resolve(t["--rf-color-muted"], FALLBACK.muted),
    faint: resolve(t["--rf-color-faint"], FALLBACK.faint),
    grid: resolve(t["--rf-chart-grid"], FALLBACK.grid),
    axis: resolve(t["--rf-chart-axis"], FALLBACK.axis),
    accent: resolve(t["--rf-color-accent"], FALLBACK.palette[0]),
    success: resolve(t["--rf-color-success"], FALLBACK.palette[1]),
    warning: resolve(t["--rf-color-warning"], FALLBACK.palette[2]),
    danger: resolve(t["--rf-color-danger"], FALLBACK.palette[3]),
    info: resolve(t["--rf-color-info"], FALLBACK.palette[4]),
    palette,
    tooltip: {
      bg: resolve(t["--rf-surface-overlay"], FALLBACK.surface),
      border: resolve(t["--rf-border-strong"], FALLBACK.border),
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
