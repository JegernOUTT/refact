import React from "react";
import type { LucideIcon } from "lucide-react";
import { Network } from "lucide-react";
import ReactEChartsCore from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { BarChart, PieChart } from "echarts/charts";
import {
  GridComponent,
  LegendComponent,
  TitleComponent,
  TooltipComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

import {
  Card,
  EmptyState,
  ErrorState,
  Icon,
  LoadingState,
} from "../../../components/ui";
import type { CodeIntelResponse } from "../../../services/refact/types";
import styles from "./CodeIntelStatsTabs.module.css";
import { isCodeIntelDetail } from "./tabUtils";
import { useReportIndexReadiness } from "../useIndexReadiness";

echarts.use([
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  BarChart,
  PieChart,
  CanvasRenderer,
]);

export type CodeIntelQueryResult<T> = {
  data?: CodeIntelResponse<T>;
  error?: unknown;
  isFetching: boolean;
  isLoading: boolean;
};

type TabScaffoldProps<T> = {
  result: CodeIntelQueryResult<T>;
  loadingLabel: string;
  errorTitle: string;
  errorDescription: string;
  emptyIcon: LucideIcon;
  emptyTitle: string;
  emptyDescription: string;
  isEmpty: (data: T) => boolean;
  children: (data: T, isFetching: boolean) => React.ReactNode;
  readinessKey?: string;
};

function CodeIntelUnavailable({ detail }: { detail: string }) {
  return (
    <Card className={styles.stateCard} padding="lg" variant="glass">
      <EmptyState
        icon={Network}
        title="CodeGraph data is not available"
        description={detail}
        variant="full"
      />
    </Card>
  );
}

export function CodeIntelTabScaffold<T>({
  result,
  loadingLabel,
  errorTitle,
  errorDescription,
  emptyIcon,
  emptyTitle,
  emptyDescription,
  isEmpty,
  children,
  readinessKey,
}: TabScaffoldProps<T>) {
  const data = isCodeIntelDetail(result.data) ? null : result.data;
  useReportIndexReadiness(readinessKey ?? "", readinessKey ? result.data : null);

  if (result.isLoading) {
    return <LoadingState label={loadingLabel} kind="skeleton" variant="full" />;
  }

  if (result.error) {
    return (
      <Card className={styles.stateCard} padding="lg" variant="glass">
        <ErrorState
          title={errorTitle}
          description={errorDescription}
          variant="full"
        />
      </Card>
    );
  }

  if (isCodeIntelDetail(result.data)) {
    return <CodeIntelUnavailable detail={result.data.detail} />;
  }

  if (!data || isEmpty(data)) {
    return (
      <Card className={styles.stateCard} padding="lg" variant="glass">
        <EmptyState
          icon={emptyIcon}
          title={emptyTitle}
          description={emptyDescription}
          variant="full"
        />
      </Card>
    );
  }

  return (
    <div className={styles.tabRoot}>
      {result.isFetching ? (
        <p className={styles.refreshing}>Refreshing code intelligence data…</p>
      ) : null}
      {children(data, result.isFetching)}
    </div>
  );
}

type ChartCardProps = {
  title: string;
  icon: LucideIcon;
  option: Record<string, unknown>;
};

export function ChartCard({ title, icon, option }: ChartCardProps) {
  return (
    <Card className={styles.chartCard} padding="md" variant="glass">
      <h4 className={styles.chartTitle}>
        <Icon icon={icon} size="sm" tone="accent" />
        {title}
      </h4>
      <ReactEChartsCore
        echarts={echarts}
        option={option}
        className={styles.chartCanvas}
      />
    </Card>
  );
}

export function PathText({ path }: { path: string }) {
  return (
    <p className={styles.pathText} title={path}>
      {path}
    </p>
  );
}

export function MetaText({ children }: { children: React.ReactNode }) {
  return <p className={styles.metaText}>{children}</p>;
}
