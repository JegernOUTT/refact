import ReactEChartsCore from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { BarChart } from "echarts/charts";
import {
  GridComponent,
  LegendComponent,
  TooltipComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import { BarChart3, CircleDollarSign } from "lucide-react";

import { Card, Icon } from "../../../components/ui";
import type { DayStats } from "../../StatsDashboard/types";
import {
  categoryAxis,
  chartGrid,
  chartLegend,
  chartTooltip,
  useChartTheme,
  valueAxis,
} from "../../StatsDashboard/utils/chartTheme";
import { formatCostTick } from "./costTicks";
import styles from "./Usage.module.css";

echarts.use([
  TooltipComponent,
  LegendComponent,
  GridComponent,
  BarChart,
  CanvasRenderer,
]);

type UsageChartsProps = {
  days: DayStats[];
};

export function UsageCharts({ days }: UsageChartsProps) {
  const theme = useChartTheme();

  if (days.length === 0) return null;

  const dayLabels = days.map((day) =>
    new Date(`${day.date}T00:00:00`).toLocaleString(undefined, {
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
        data: days.map((day) => day.total_prompt_tokens),
        itemStyle: { color: theme.palette[0] },
      },
      {
        name: "Completion",
        type: "bar",
        stack: "tokens",
        data: days.map((day) => day.total_completion_tokens),
        itemStyle: { color: theme.palette[1] },
      },
    ],
  };

  const hasCostData = days.some((day) => day.total_cost_usd > 0);
  const maxCost = Math.max(...days.map((day) => day.total_cost_usd), 0);
  const costPerDayOption = {
    tooltip: chartTooltip(theme, "axis", {
      valueFormatter: (value: number) => `$${value.toFixed(2)}`,
    }),
    grid: chartGrid(),
    xAxis: [categoryAxis(theme, dayLabels)],
    yAxis: [
      valueAxis(theme, {
        axisLabel: {
          color: theme.muted,
          formatter: (value: number) => formatCostTick(value, maxCost),
        },
      }),
    ],
    series: [
      {
        name: "Cost",
        type: "bar",
        data: days.map((day) => Number(day.total_cost_usd.toFixed(4))),
        itemStyle: { color: theme.warning },
      },
    ],
  };

  return (
    <div className={styles.chartsRow}>
      <Card animated="rise" className={styles.chartBox}>
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
      {hasCostData && (
        <Card animated="rise" className={styles.chartBox}>
          <h3 className={styles.sectionTitle}>
            <Icon icon={CircleDollarSign} size="md" tone="warning" />
            Cost Per Day
          </h3>
          <ReactEChartsCore
            echarts={echarts}
            option={costPerDayOption}
            className={styles.chartCanvas}
          />
        </Card>
      )}
    </div>
  );
}
