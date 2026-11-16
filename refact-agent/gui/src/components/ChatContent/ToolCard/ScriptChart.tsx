import ReactEChartsCore from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { BarChart, LineChart, PieChart, ScatterChart } from "echarts/charts";
import {
  GridComponent,
  LegendComponent,
  TooltipComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import { useReducedMotion } from "../../../hooks/useReducedMotion";
import { useChartTheme } from "../../../features/StatsDashboard/utils/chartTheme";
import type { ChartData } from "./scriptDisplayHelpers";
import { chartOption } from "./scriptChartOption";
import styles from "./ScriptDisplays.module.css";

echarts.use([
  TooltipComponent,
  LegendComponent,
  GridComponent,
  BarChart,
  LineChart,
  ScatterChart,
  PieChart,
  CanvasRenderer,
]);

/** Lazily loaded so the chart library stays out of the chat's first load. */
export default function ScriptChart({ data }: { data: ChartData }) {
  const theme = useChartTheme();
  const reduced = useReducedMotion();
  // The wrapper compares options by value, so an equal option is not re-applied.
  const option = chartOption(data, theme, !reduced);
  return (
    <ReactEChartsCore
      echarts={echarts}
      option={option}
      notMerge
      className={styles.chartCanvas}
    />
  );
}
