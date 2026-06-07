import React, { useState } from "react";
import { Card, Icon, Surface, useTokens } from "../../../components/ui";
import {
  BarChart3,
  CircleDollarSign,
  Database,
  PieChart as PieChartIcon,
  Table2,
} from "lucide-react";
import ReactEChartsCore from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { BarChart, PieChart } from "echarts/charts";
import {
  GridComponent,
  TooltipComponent,
  LegendComponent,
  TitleComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";

import {
  formatTokenCount,
  formatCostDisplay,
  formatDuration,
} from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { DateRange, ModelStats, ProviderStats } from "../types";
import styles from "./UsageTab.module.css";

echarts.use([
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  BarChart,
  PieChart,
  CanvasRenderer,
]);

type Props = { dateRange: DateRange };

type SortKey =
  | "total_calls"
  | "total_tokens"
  | "total_cost_usd"
  | "avg_duration_ms";

function sortModels(
  models: ModelStats[],
  key: SortKey,
  asc: boolean,
): ModelStats[] {
  return [...models].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    return asc ? av - bv : bv - av;
  });
}

function sortProviders(
  providers: ProviderStats[],
  key: Exclude<SortKey, "avg_duration_ms">,
  asc: boolean,
): ProviderStats[] {
  return [...providers].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    return asc ? av - bv : bv - av;
  });
}

export const UsageTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );
  const chartTokens = useTokens([
    "--rf-color-fg",
    "--rf-color-muted",
    "--rf-color-faint",
    "--rf-border-strong",
    "--rf-surface-overlay",
    "--rf-color-accent",
    "--rf-color-info",
    "--rf-color-warning",
    "--rf-color-danger",
    "--rf-color-success",
  ]);
  const theme = {
    text: chartTokens["--rf-color-fg"] || "currentColor",
    textMuted: chartTokens["--rf-color-muted"] || "currentColor",
    axisLine: chartTokens["--rf-color-faint"] || "currentColor",
    splitLine: chartTokens["--rf-border-strong"] || "currentColor",
    tooltip: {
      bg: chartTokens["--rf-surface-overlay"] || "Canvas",
      border: chartTokens["--rf-border-strong"] || "currentColor",
      text: chartTokens["--rf-color-fg"] || "CanvasText",
    },
    palette: [
      chartTokens["--rf-color-accent"] || "currentColor",
      chartTokens["--rf-color-info"] || "currentColor",
      chartTokens["--rf-color-warning"] || "currentColor",
      chartTokens["--rf-color-danger"] || "currentColor",
      chartTokens["--rf-color-success"] || "currentColor",
      chartTokens["--rf-color-muted"] || "currentColor",
      chartTokens["--rf-color-faint"] || "currentColor",
    ],
  };

  const [modelSort, setModelSort] = useState<{ key: SortKey; asc: boolean }>({
    key: "total_tokens",
    asc: false,
  });
  const [providerSort, setProviderSort] = useState<{
    key: Exclude<SortKey, "avg_duration_ms">;
    asc: boolean;
  }>({
    key: "total_tokens",
    asc: false,
  });

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  if (!data || data.totals.total_calls === 0) {
    return (
      <p className={styles.emptyText}>
        No usage data yet. Start chatting to see stats!
      </p>
    );
  }

  const days = [...data.by_day].sort((a, b) => a.date.localeCompare(b.date));
  const dayLabels = days.map((d) =>
    new Date(d.date).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
    }),
  );

  const barOption = {
    textStyle: { color: theme.text },
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      textStyle: { color: theme.tooltip.text },
      backgroundColor: theme.tooltip.bg,
      borderColor: theme.tooltip.border,
    },
    legend: {
      data: ["Prompt Tokens", "Completion Tokens"],
      textStyle: { color: theme.text },
    },
    grid: {
      left: "3%",
      right: "4%",
      bottom: "3%",
      top: "15%",
      containLabel: true,
    },
    xAxis: [
      {
        type: "category",
        data: dayLabels,
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
      },
    ],
    yAxis: [
      {
        type: "value",
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
        splitLine: {
          lineStyle: { color: theme.splitLine },
        },
      },
    ],
    series: [
      {
        name: "Prompt Tokens",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_prompt_tokens),
        itemStyle: { color: theme.palette[0] },
      },
      {
        name: "Completion Tokens",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_completion_tokens),
        itemStyle: { color: theme.palette[1] },
      },
    ],
  };

  const sortedByTokens = [...data.by_model].sort(
    (a, b) => b.total_tokens - a.total_tokens,
  );
  const topModels = sortedByTokens.slice(0, 5);
  const otherTokens = sortedByTokens
    .slice(5)
    .reduce((sum, m) => sum + m.total_tokens, 0);
  const modelPieData: { name: string; value: number }[] = topModels.map(
    (m) => ({
      name: m.model,
      value: m.total_tokens,
    }),
  );
  if (otherTokens > 0) {
    modelPieData.push({ name: "Others", value: otherTokens });
  }

  const pieOption = {
    textStyle: { color: theme.text },
    tooltip: {
      trigger: "item",
      formatter: "{b}: {c} ({d}%)",
      textStyle: { color: theme.tooltip.text },
      backgroundColor: theme.tooltip.bg,
      borderColor: theme.tooltip.border,
    },
    legend: {
      orient: "horizontal",
      bottom: 0,
      textStyle: { color: theme.text },
    },
    color: theme.palette,
    series: [
      {
        type: "pie",
        radius: ["40%", "70%"],
        data: modelPieData,
        label: {
          color: theme.text,
          formatter: "{b}: {d}%",
          overflow: "truncate",
          ellipsis: "...",
        },
        labelLine: { lineStyle: { color: theme.textMuted } },
        emphasis: {
          label: { show: true, fontWeight: "bold" },
        },
      },
    ],
  };

  const sortedModels = sortModels(data.by_model, modelSort.key, modelSort.asc);
  const sortedProviders = sortProviders(
    data.by_provider,
    providerSort.key,
    providerSort.asc,
  );

  function toggleModelSort(key: SortKey) {
    setModelSort((prev) =>
      prev.key === key ? { key, asc: !prev.asc } : { key, asc: false },
    );
  }

  function toggleProviderSort(key: Exclude<SortKey, "avg_duration_ms">) {
    setProviderSort((prev) =>
      prev.key === key ? { key, asc: !prev.asc } : { key, asc: false },
    );
  }

  const hasCostData = days.some((d) => d.total_cost_usd > 0);

  const hasCacheData = days.some(
    (d) => d.total_cache_read_tokens > 0 || d.total_cache_creation_tokens > 0,
  );

  const cacheBarOption = {
    textStyle: { color: theme.text },
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      textStyle: { color: theme.tooltip.text },
      backgroundColor: theme.tooltip.bg,
      borderColor: theme.tooltip.border,
    },
    legend: {
      data: ["Cache Read", "Cache Created"],
      textStyle: { color: theme.text },
    },
    grid: {
      left: "3%",
      right: "4%",
      bottom: "3%",
      top: "15%",
      containLabel: true,
    },
    xAxis: [
      {
        type: "category",
        data: dayLabels,
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
      },
    ],
    yAxis: [
      {
        type: "value",
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
        splitLine: {
          lineStyle: { color: theme.splitLine },
        },
      },
    ],
    series: [
      {
        name: "Cache Read",
        type: "bar",
        stack: "cache",
        data: days.map((d) => d.total_cache_read_tokens),
        itemStyle: { color: theme.palette[4] },
      },
      {
        name: "Cache Created",
        type: "bar",
        stack: "cache",
        data: days.map((d) => d.total_cache_creation_tokens),
        itemStyle: { color: theme.palette[5] },
      },
    ],
  };

  const costBarOption = {
    textStyle: { color: theme.text },
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      textStyle: { color: theme.tooltip.text },
      backgroundColor: theme.tooltip.bg,
      borderColor: theme.tooltip.border,
    },
    legend: {
      data: ["USD Cost"],
      textStyle: { color: theme.text },
    },
    grid: {
      left: "3%",
      right: "4%",
      bottom: "3%",
      top: "15%",
      containLabel: true,
    },
    xAxis: [
      {
        type: "category",
        data: dayLabels,
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
      },
    ],
    yAxis: [
      {
        type: "value",
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
        splitLine: {
          lineStyle: { color: theme.splitLine },
        },
      },
    ],
    series: [
      {
        name: "USD Cost",
        type: "bar",
        stack: "cost",
        data: days.map((d) => d.total_cost_usd),
        itemStyle: { color: theme.palette[2] },
      },
    ],
  };

  const callsBarOption = {
    textStyle: { color: theme.text },
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      textStyle: { color: theme.tooltip.text },
      backgroundColor: theme.tooltip.bg,
      borderColor: theme.tooltip.border,
    },
    legend: {
      data: ["Calls"],
      textStyle: { color: theme.text },
    },
    grid: {
      left: "3%",
      right: "4%",
      bottom: "3%",
      top: "15%",
      containLabel: true,
    },
    xAxis: [
      {
        type: "category",
        data: dayLabels,
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
      },
    ],
    yAxis: [
      {
        type: "value",
        axisLine: { lineStyle: { color: theme.axisLine } },
        axisLabel: { color: theme.text },
        splitLine: {
          lineStyle: { color: theme.splitLine },
        },
      },
    ],
    series: [
      {
        name: "Calls",
        type: "bar",
        data: days.map((d) => d.total_calls),
        itemStyle: { color: theme.palette[0] },
      },
    ],
  };

  return (
    <div className={styles.root}>
      <div className={`${styles.chartsRow} rf-stagger`}>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={BarChart3} size="md" tone="accent" />
            Tokens Per Day
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={barOption}
            className={styles.chartCanvas}
          />
        </Card>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={PieChartIcon} size="md" tone="accent" />
            By Model
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={pieOption}
            className={styles.chartCanvasTall}
          />
        </Card>
      </div>

      <div className={`${styles.chartsRow} rf-stagger`}>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={BarChart3} size="md" tone="accent" />
            Calls Per Day
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={callsBarOption}
            className={styles.chartCanvas}
          />
        </Card>
        {hasCostData && (
          <Card animated="rise" className={styles.chartBox} interactive>
            <h3 className={styles.sectionTitle}>
              <Icon icon={CircleDollarSign} size="md" tone="warning" />
              Cost Per Day
            </h3>
            <ReactEChartsCore
              echarts={echarts}
              option={costBarOption}
              className={styles.chartCanvas}
            />
          </Card>
        )}
      </div>

      {hasCacheData && (
        <div className={`${styles.chartsRow} rf-stagger`}>
          <Card animated="rise" className={styles.chartBox} interactive>
            <h3 className={styles.sectionTitle}>
              <Icon icon={Database} size="md" tone="success" />
              Cache Tokens Per Day
            </h3>
            <ReactEChartsCore
              echarts={echarts}
              option={cacheBarOption}
              className={styles.chartCanvas}
            />
          </Card>
        </div>
      )}

      <section className={styles.root}>
        <h3 className={styles.sectionTitle}>
          <Icon icon={Table2} size="md" tone="accent" />
          By Provider
        </h3>
        <Surface className={styles.tableWrapper} variant="plain">
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Provider</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleProviderSort("total_calls")}
                  >
                    Calls{" "}
                    {providerSort.key === "total_calls"
                      ? providerSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleProviderSort("total_tokens")}
                  >
                    Tokens{" "}
                    {providerSort.key === "total_tokens"
                      ? providerSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
                <th className={styles.th}>Cache Read</th>
                <th className={styles.th}>Cache Created</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleProviderSort("total_cost_usd")}
                  >
                    Cost{" "}
                    {providerSort.key === "total_cost_usd"
                      ? providerSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody>
              {sortedProviders.map((p) => (
                <tr key={p.provider}>
                  <td className={styles.td}>{p.provider}</td>
                  <td className={styles.td}>{p.total_calls}</td>
                  <td className={styles.td}>
                    {formatTokenCount(p.total_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(p.total_cache_read_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(p.total_cache_creation_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatCostDisplay(p.total_cost_usd)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Surface>
      </section>

      <section className={styles.root}>
        <h3 className={styles.sectionTitle}>
          <Icon icon={Table2} size="md" tone="accent" />
          By Model
        </h3>
        <Surface className={styles.tableWrapper} variant="plain">
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Model</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleModelSort("total_calls")}
                  >
                    Calls{" "}
                    {modelSort.key === "total_calls"
                      ? modelSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
                <th className={styles.th}>Prompt</th>
                <th className={styles.th}>Completion</th>
                <th className={styles.th}>Cache Read</th>
                <th className={styles.th}>Cache Created</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleModelSort("total_cost_usd")}
                  >
                    Cost{" "}
                    {modelSort.key === "total_cost_usd"
                      ? modelSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleModelSort("avg_duration_ms")}
                  >
                    Avg Duration{" "}
                    {modelSort.key === "avg_duration_ms"
                      ? modelSort.asc
                        ? "↑"
                        : "↓"
                      : ""}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody>
              {sortedModels.map((m) => (
                <tr key={`${m.provider}/${m.model}`}>
                  <td className={styles.td}>{m.model}</td>
                  <td className={styles.td}>{m.total_calls}</td>
                  <td className={styles.td}>
                    {formatTokenCount(m.total_prompt_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(m.total_completion_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(m.total_cache_read_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(m.total_cache_creation_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatCostDisplay(m.total_cost_usd)}
                  </td>
                  <td className={styles.td}>
                    {formatDuration(m.avg_duration_ms)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Surface>
      </section>
    </div>
  );
};
