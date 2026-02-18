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
import { useAppearance } from "../../../hooks";
import { formatTokenCount, formatCost, formatDuration } from "../utils/formatters";
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

type SortKey = "total_calls" | "total_tokens" | "total_cost_usd" | "avg_duration_ms";

function sortModels(models: ModelStats[], key: SortKey, asc: boolean): ModelStats[] {
  return [...models].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    return asc ? av - bv : bv - av;
  });
}

function sortProviders(providers: ProviderStats[], key: Exclude<SortKey, "avg_duration_ms">, asc: boolean): ProviderStats[] {
  return [...providers].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    return asc ? av - bv : bv - av;
  });
}

export const UsageTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(dateRangeToApiArgs(dateRange));
  const { isDarkMode } = useAppearance();
  const axisColor = isDarkMode ? "#ffffff" : "#646464";

  const [modelSort, setModelSort] = useState<{ key: SortKey; asc: boolean }>({
    key: "total_tokens",
    asc: false,
  });
  const [providerSort, setProviderSort] = useState<{ key: Exclude<SortKey, "avg_duration_ms">; asc: boolean }>({
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
    new Date(d.date).toLocaleString(undefined, { month: "short", day: "numeric" }),
  );

  const barOption = {
    tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
    legend: { data: ["Prompt Tokens", "Completion Tokens"] },
    grid: { left: "3%", right: "4%", bottom: "3%", top: "15%", containLabel: true },
    xAxis: [
      {
        type: "category",
        data: dayLabels,
        axisLine: { lineStyle: { color: axisColor } },
      },
    ],
    yAxis: [
      {
        type: "value",
        axisLine: { lineStyle: { color: axisColor } },
      },
    ],
    series: [
      {
        name: "Prompt Tokens",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_prompt_tokens),
        itemStyle: { color: "#5470c6" },
      },
      {
        name: "Completion Tokens",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_completion_tokens),
        itemStyle: { color: "#91cc75" },
      },
    ],
  };

  const modelPieData = data.by_model.map((m) => ({
    name: m.model,
    value: m.total_tokens,
  }));

  const pieOption = {
    tooltip: { trigger: "item", formatter: "{b}: {c} ({d}%)" },
    series: [
      {
        type: "pie",
        radius: ["40%", "70%"],
        data: modelPieData,
        label: { color: axisColor },
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
            style={{ width: "100%", height: "220px" }}
          />
        </Box>
      </Flex>

      <Box>
        <Text size="3" weight="medium" className={styles.sectionTitle} mb="2" as="p">
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
                    Calls {providerSort.key === "total_calls" ? (providerSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleProviderSort("total_tokens")}
                  >
                    Tokens {providerSort.key === "total_tokens" ? (providerSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleProviderSort("total_cost_usd")}
                  >
                    Cost {providerSort.key === "total_cost_usd" ? (providerSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody>
              {sortedProviders.map((p) => (
                <tr key={p.provider}>
                  <td className={styles.td}>{p.provider}</td>
                  <td className={styles.td}>{p.total_calls}</td>
                  <td className={styles.td}>{formatTokenCount(p.total_tokens)}</td>
                  <td className={styles.td}>{formatCost(p.total_cost_usd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Box>
      </Box>

      <Box>
        <Text size="3" weight="medium" className={styles.sectionTitle} mb="2" as="p">
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
                    Calls {modelSort.key === "total_calls" ? (modelSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
                <th className={styles.th}>Prompt</th>
                <th className={styles.th}>Completion</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleModelSort("total_cost_usd")}
                  >
                    Cost {modelSort.key === "total_cost_usd" ? (modelSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={styles.sortButton}
                    onClick={() => toggleModelSort("avg_duration_ms")}
                  >
                    Avg Duration {modelSort.key === "avg_duration_ms" ? (modelSort.asc ? "↑" : "↓") : ""}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody>
              {sortedModels.map((m) => (
                <tr key={`${m.provider}/${m.model}`}>
                  <td className={styles.td}>{m.model}</td>
                  <td className={styles.td}>{m.total_calls}</td>
                  <td className={styles.td}>{formatTokenCount(m.total_prompt_tokens)}</td>
                  <td className={styles.td}>{formatTokenCount(m.total_completion_tokens)}</td>
                  <td className={styles.td}>{formatCost(m.total_cost_usd)}</td>
                  <td className={styles.td}>{formatDuration(m.avg_duration_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Box>
      </Box>
    </Flex>
  );
};
