import React, { useState } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
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
import { useTokens } from "../../../components/ui";
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

const CHART_TOKEN_NAMES = [
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
  "--accent-10",
  "--cyan-9",
  "--purple-9",
] as const;

const chartToken = (
  tokens: Record<string, string>,
  name: (typeof CHART_TOKEN_NAMES)[number],
  fallback: string,
): string => tokens[name] || fallback;


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
  const chartTokens = useTokens([...CHART_TOKEN_NAMES]);

  const theme = {
    text: chartToken(chartTokens, "--rf-color-fg", "currentColor"),
    textMuted: chartToken(chartTokens, "--rf-color-muted", "currentColor"),
    axisLine: chartToken(chartTokens, "--rf-color-faint", "currentColor"),
    splitLine: chartToken(chartTokens, "--rf-border-strong", "currentColor"),
    tooltip: {
      bg: chartToken(chartTokens, "--rf-surface-overlay", "Canvas"),
      border: chartToken(chartTokens, "--rf-border-strong", "currentColor"),
      text: chartToken(chartTokens, "--rf-color-fg", "CanvasText"),
    },
    palette: [
      chartToken(chartTokens, "--rf-color-accent", "currentColor"),
      chartToken(chartTokens, "--accent-10", "currentColor"),
      chartToken(chartTokens, "--rf-color-warning", "currentColor"),
      chartToken(chartTokens, "--rf-color-danger", "currentColor"),
      chartToken(chartTokens, "--cyan-9", "currentColor"),
      chartToken(chartTokens, "--rf-color-info", "currentColor"),
      chartToken(chartTokens, "--rf-color-success", "currentColor"),
      chartToken(chartTokens, "--purple-9", "currentColor"),
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
      <Text className={styles.emptyText}>
        No usage data yet. Start chatting to see stats!
      </Text>
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
    <Flex direction="column" gap="5">
      <Flex className={styles.chartsRow}>
        <Box className={styles.chartBox}>
          <Text size="2" weight="medium" className={styles.sectionTitle}>
            Tokens Per Day
          </Text>
          <ReactEChartsCore
            echarts={echarts}
            option={barOption}
            style={{ width: "100%", height: "220px" }}
          />
        </Box>
        <Box className={styles.chartBox}>
          <Text size="2" weight="medium" className={styles.sectionTitle}>
            By Model
          </Text>
          <ReactEChartsCore
            echarts={echarts}
            option={pieOption}
            style={{ width: "100%", height: "280px" }}
          />
        </Box>
      </Flex>

      <Flex className={styles.chartsRow}>
        <Box className={styles.chartBox}>
          <Text size="2" weight="medium" className={styles.sectionTitle}>
            Calls Per Day
          </Text>
          <ReactEChartsCore
            echarts={echarts}
            option={callsBarOption}
            style={{ width: "100%", height: "220px" }}
          />
        </Box>
        {hasCostData && (
          <Box className={styles.chartBox}>
            <Text size="2" weight="medium" className={styles.sectionTitle}>
              Cost Per Day
            </Text>
            <ReactEChartsCore
              echarts={echarts}
              option={costBarOption}
              style={{ width: "100%", height: "220px" }}
            />
          </Box>
        )}
      </Flex>

      {hasCacheData && (
        <Flex className={styles.chartsRow}>
          <Box className={styles.chartBox}>
            <Text size="2" weight="medium" className={styles.sectionTitle}>
              Cache Tokens Per Day
            </Text>
            <ReactEChartsCore
              echarts={echarts}
              option={cacheBarOption}
              style={{ width: "100%", height: "220px" }}
            />
          </Box>
        </Flex>
      )}

      <Box>
        <Text
          size="3"
          weight="medium"
          className={styles.sectionTitle}
          mb="2"
          as="p"
        >
          By Provider
        </Text>
        <Box className={styles.tableWrapper}>
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
        </Box>
      </Box>

      <Box>
        <Text
          size="3"
          weight="medium"
          className={styles.sectionTitle}
          mb="2"
          as="p"
        >
          By Model
        </Text>
        <Box className={styles.tableWrapper}>
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
        </Box>
      </Box>
    </Flex>
  );
};
