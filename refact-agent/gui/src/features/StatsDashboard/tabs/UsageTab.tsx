import React, { useState } from "react";
import { Card, Icon, Surface } from "../../../components/ui";
import {
  Activity,
  BarChart3,
  CircleDollarSign,
  Clock3,
  Database,
  Gauge,
  PieChart as PieChartIcon,
  Table2,
} from "lucide-react";
import ReactEChartsCore from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { BarChart, LineChart, PieChart } from "echarts/charts";
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
  formatRatioPercent,
} from "../utils/formatters";
import {
  useChartTheme,
  chartTooltip,
  chartGrid,
  chartLegend,
  categoryAxis,
  valueAxis,
  percentAxis,
} from "../utils/chartTheme";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { DateRange, ModelStats, ProviderStats } from "../types";
import styles from "./UsageTab.module.css";

echarts.use([
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  BarChart,
  LineChart,
  PieChart,
  CanvasRenderer,
]);

type Props = { dateRange: DateRange };

type SortKey =
  | "total_calls"
  | "total_tokens"
  | "total_cost_usd"
  | "avg_duration_ms";

function successRate(successful: number, total: number): number {
  return total > 0 ? (successful / total) * 100 : 0;
}

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
  const theme = useChartTheme();

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
    // Parse as local time so the YYYY-MM-DD bucket isn't shifted a day for
    // users west of UTC (`new Date("YYYY-MM-DD")` parses as UTC midnight).
    new Date(`${d.date}T00:00:00`).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
    }),
  );

  const tokensPerDayOption = {
    tooltip: chartTooltip(theme, "axis"),
    legend: chartLegend(theme, { data: ["Prompt", "Completion"] }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels)],
    yAxis: [valueAxis(theme)],
    series: [
      {
        name: "Prompt",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_prompt_tokens),
        itemStyle: { color: theme.palette[0] },
      },
      {
        name: "Completion",
        type: "bar",
        stack: "tokens",
        data: days.map((d) => d.total_completion_tokens),
        itemStyle: { color: theme.palette[1] },
      },
    ],
  };

  const callsPerDayOption = {
    tooltip: chartTooltip(theme, "axis"),
    legend: chartLegend(theme, { data: ["Succeeded", "Failed"] }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels)],
    yAxis: [valueAxis(theme)],
    series: [
      {
        name: "Succeeded",
        type: "bar",
        stack: "calls",
        data: days.map((d) => d.successful_calls),
        itemStyle: { color: theme.success },
      },
      {
        name: "Failed",
        type: "bar",
        stack: "calls",
        data: days.map((d) => Math.max(0, d.total_calls - d.successful_calls)),
        itemStyle: { color: theme.danger },
      },
    ],
  };

  const avgDurationPerDayOption = {
    tooltip: chartTooltip(theme, "axis", {
      valueFormatter: (value: number) => `${(value / 1000).toFixed(1)}s`,
    }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels, { boundaryGap: false })],
    yAxis: [
      valueAxis(theme, {
        axisLabel: {
          color: theme.muted,
          formatter: (value: number) => `${(value / 1000).toFixed(0)}s`,
        },
      }),
    ],
    series: [
      {
        name: "Avg duration",
        type: "line",
        smooth: true,
        showSymbol: false,
        data: days.map((d) =>
          d.total_calls > 0
            ? Math.round(d.total_duration_ms / d.total_calls)
            : 0,
        ),
        itemStyle: { color: theme.palette[4] },
        lineStyle: { color: theme.palette[4], width: 2 },
        areaStyle: { color: theme.palette[4], opacity: 0.12 },
      },
    ],
  };

  const successRatePerDayOption = {
    tooltip: chartTooltip(theme, "axis", {
      valueFormatter: (value: number) => `${value.toFixed(0)}%`,
    }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels, { boundaryGap: false })],
    yAxis: [percentAxis(theme)],
    series: [
      {
        name: "Success rate",
        type: "line",
        smooth: true,
        showSymbol: false,
        data: days.map((d) =>
          Math.round(successRate(d.successful_calls, d.total_calls)),
        ),
        itemStyle: { color: theme.success },
        lineStyle: { color: theme.success, width: 2 },
        areaStyle: { color: theme.success, opacity: 0.12 },
      },
    ],
  };

  const sortedByTokens = [...data.by_model].sort(
    (a, b) => b.total_tokens - a.total_tokens,
  );
  const topModels = sortedByTokens.slice(0, 6);
  const otherTokens = sortedByTokens
    .slice(6)
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

  const modelPieOption = {
    tooltip: chartTooltip(theme, "item", { formatter: "{b}: {c} ({d}%)" }),
    legend: chartLegend(theme, { orient: "horizontal", bottom: 0 }),
    color: theme.palette,
    series: [
      {
        type: "pie",
        radius: ["42%", "70%"],
        center: ["50%", "44%"],
        data: modelPieData,
        label: {
          color: theme.muted,
          formatter: "{b}: {d}%",
          overflow: "truncate",
        },
        labelLine: { lineStyle: { color: theme.faint } },
        emphasis: { label: { show: true, fontWeight: "bold" } },
      },
    ],
  };

  const promptCompletionOption = {
    tooltip: chartTooltip(theme, "item", { formatter: "{b}: {c} ({d}%)" }),
    legend: chartLegend(theme, { orient: "horizontal", bottom: 0 }),
    color: [theme.palette[0], theme.palette[1]],
    series: [
      {
        type: "pie",
        radius: ["42%", "70%"],
        center: ["50%", "44%"],
        data: [
          { name: "Prompt (read)", value: data.totals.total_prompt_tokens },
          {
            name: "Completion (written)",
            value: data.totals.total_completion_tokens,
          },
        ],
        label: { color: theme.muted, formatter: "{b}: {d}%" },
        labelLine: { lineStyle: { color: theme.faint } },
        emphasis: { label: { show: true, fontWeight: "bold" } },
      },
    ],
  };

  const sortedProvidersByTokens = [...data.by_provider].sort(
    (a, b) => b.total_tokens - a.total_tokens,
  );
  const providerBarOption = {
    tooltip: chartTooltip(theme, "axis"),
    grid: chartGrid(),
    xAxis: [
      categoryAxis(
        theme,
        sortedProvidersByTokens.map((p) => p.provider),
      ),
    ],
    yAxis: [valueAxis(theme)],
    series: [
      {
        name: "Tokens",
        type: "bar",
        data: sortedProvidersByTokens.map((p) => p.total_tokens),
        itemStyle: { color: theme.palette[5] },
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

  const costBarOption = {
    tooltip: chartTooltip(theme, "axis", {
      valueFormatter: (value: number) => `$${value.toFixed(2)}`,
    }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels)],
    yAxis: [
      valueAxis(theme, {
        axisLabel: {
          color: theme.muted,
          formatter: (value: number) => `$${value.toFixed(0)}`,
        },
      }),
    ],
    series: [
      {
        name: "Cost",
        type: "bar",
        data: days.map((d) => Number(d.total_cost_usd.toFixed(4))),
        itemStyle: { color: theme.warning },
      },
    ],
  };

  const cacheBarOption = {
    tooltip: chartTooltip(theme, "axis"),
    legend: chartLegend(theme, { data: ["Cache read", "Cache created"] }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels)],
    yAxis: [valueAxis(theme)],
    series: [
      {
        name: "Cache read",
        type: "bar",
        stack: "cache",
        data: days.map((d) => d.total_cache_read_tokens),
        itemStyle: { color: theme.palette[1] },
      },
      {
        name: "Cache created",
        type: "bar",
        stack: "cache",
        data: days.map((d) => d.total_cache_creation_tokens),
        itemStyle: { color: theme.palette[2] },
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
            option={tokensPerDayOption}
            className={styles.chartCanvas}
          />
        </Card>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={PieChartIcon} size="md" tone="accent" />
            Tokens by Model
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={modelPieOption}
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
            option={callsPerDayOption}
            className={styles.chartCanvas}
          />
        </Card>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={PieChartIcon} size="md" tone="accent" />
            Prompt vs Completion
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={promptCompletionOption}
            className={styles.chartCanvasTall}
          />
        </Card>
      </div>

      <div className={`${styles.chartsRow} rf-stagger`}>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={Clock3} size="md" tone="accent" />
            Avg Duration Per Day
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={avgDurationPerDayOption}
            className={styles.chartCanvas}
          />
        </Card>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={Gauge} size="md" tone="success" />
            Success Rate Per Day
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={successRatePerDayOption}
            className={styles.chartCanvas}
          />
        </Card>
      </div>

      <div className={`${styles.chartsRow} rf-stagger`}>
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
        {hasCacheData && (
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
        )}
      </div>

      <div className={`${styles.chartsRow} rf-stagger`}>
        <Card animated="rise" className={styles.chartBox} interactive>
          <h3 className={styles.sectionTitle}>
            <Icon icon={Activity} size="md" tone="accent" />
            Tokens by Provider
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={providerBarOption}
            className={styles.chartCanvas}
          />
        </Card>
      </div>

      <section className={styles.root}>
        <h3 className={styles.sectionTitle}>
          <Icon icon={Table2} size="md" tone="accent" />
          By Provider
        </h3>
        <Surface
          animated="rise"
          className={styles.tableWrapper}
          variant="glass"
        >
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Provider</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
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
                <th className={styles.th}>Success</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
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
                    className={`${styles.sortButton} rf-pressable`}
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
            <tbody className="rf-stagger">
              {sortedProviders.map((p) => (
                <tr key={p.provider} className="rf-enter-rise">
                  <td className={styles.td}>{p.provider}</td>
                  <td className={styles.td}>{p.total_calls}</td>
                  <td className={styles.td}>
                    {formatRatioPercent(p.successful_calls, p.total_calls)}
                  </td>
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
        <Surface
          animated="rise"
          className={styles.tableWrapper}
          variant="glass"
        >
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Model</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
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
                <th className={styles.th}>Success</th>
                <th className={styles.th}>Prompt</th>
                <th className={styles.th}>Completion</th>
                <th className={styles.th}>Cache Read</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
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
                    className={`${styles.sortButton} rf-pressable`}
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
            <tbody className="rf-stagger">
              {sortedModels.map((m) => (
                <tr key={`${m.provider}/${m.model}`} className="rf-enter-rise">
                  <td className={styles.td}>{m.model}</td>
                  <td className={styles.td}>{m.total_calls}</td>
                  <td className={styles.td}>
                    {formatRatioPercent(m.successful_calls, m.total_calls)}
                  </td>
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
