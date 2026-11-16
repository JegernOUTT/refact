import {
  categoryAxis,
  chartGrid,
  chartLegend,
  chartTooltip,
  valueAxis,
  type ChartTheme,
} from "../../../features/StatsDashboard/utils/chartTheme";
import type { ChartData } from "./scriptDisplayHelpers";

function numericCategories(categories: readonly string[]) {
  return (
    categories.length > 0 &&
    categories.every((text) => text.trim() !== "" && Number.isFinite(+text))
  );
}

/** ECharts option for one script chart, coloured from the theme tokens. */
export function chartOption(
  data: ChartData,
  theme: ChartTheme,
  animate: boolean,
) {
  const legend = data.series.length > 1;
  const base = {
    animation: animate,
    color: theme.palette,
    textStyle: { color: theme.fg },
    legend: legend
      ? chartLegend(theme, {
          data: data.series.map((item) => item.name),
          type: "scroll",
          top: 0,
        })
      : undefined,
    grid: chartGrid({ top: legend ? 32 : 12, bottom: 8 }),
  };
  if (data.kind === "pie") {
    const [series] = data.series;
    return {
      ...base,
      tooltip: chartTooltip(theme, "item"),
      series: [
        {
          name: series.name,
          type: "pie",
          radius: ["35%", "70%"],
          label: { color: theme.muted },
          itemStyle: { borderColor: theme.tooltip.bg, borderWidth: 1 },
          data: data.categories.flatMap((name, index) => {
            const value = series.values[index];
            return value !== null && value >= 0 ? [{ name, value }] : [];
          }),
        },
      ],
    };
  }
  if (data.kind === "scatter") {
    const numericX = numericCategories(data.categories);
    return {
      ...base,
      tooltip: chartTooltip(theme, "item"),
      xAxis: numericX
        ? valueAxis(theme, { scale: true })
        : categoryAxis(theme, data.categories),
      yAxis: valueAxis(theme, { scale: true }),
      series: data.series.map((item) => ({
        name: item.name,
        type: "scatter",
        symbolSize: 6,
        data: data.categories.flatMap((label, index) => {
          const value = item.values[index];
          if (value === null) return [];
          return [[numericX ? Number(label) : label, value]];
        }),
      })),
    };
  }
  return {
    ...base,
    tooltip: chartTooltip(theme, "axis"),
    xAxis: categoryAxis(theme, data.categories),
    yAxis: valueAxis(theme),
    series: data.series.map((item) => ({
      name: item.name,
      type: data.kind === "bar" ? "bar" : "line",
      showSymbol: data.categories.length <= 40,
      connectNulls: true,
      areaStyle: data.kind === "area" ? { opacity: 0.18 } : undefined,
      data: item.values,
    })),
  };
}
