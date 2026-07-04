import React from "react";
import {
  Activity,
  BarChart3,
  Gauge,
  ListTree,
  ShieldAlert,
  Wrench,
} from "lucide-react";

import { Badge, DataTable, Surface } from "../../../components/ui";
import { useGetCodeIntelHealthQuery } from "../../../services/refact/codeIntel";
import type {
  CodeIntelCodeHealthSeverity,
  CodeIntelHealth,
  CodeIntelHealthFile,
} from "../../../services/refact/types";
import { StatCard } from "../../StatsDashboard/components/StatCard";
import type { StatCardProps } from "../../StatsDashboard/components/StatCard";
import { StatSection } from "../../StatsDashboard/components/StatSection";
import {
  formatCompact,
  formatNumber,
  formatPercent,
} from "../../StatsDashboard/utils/formatters";
import {
  categoryAxis,
  chartGrid,
  chartLegend,
  chartTooltip,
  useChartTheme,
  valueAxis,
} from "../../StatsDashboard/utils/chartTheme";
import styles from "./CodeIntelStatsTabs.module.css";
import { ChartCard, CodeIntelTabScaffold, MetaText, PathText } from "./shared";
import { formatMaybeFixed } from "./tabUtils";

function isEmpty(health: CodeIntelHealth): boolean {
  return health.aggregate.file_count === 0 && health.files.length === 0;
}

function severityTone(
  severity: CodeIntelCodeHealthSeverity,
): React.ComponentProps<typeof Badge>["tone"] {
  if (severity === "Critical" || severity === "High") return "danger";
  if (severity === "Medium") return "warning";
  return "muted";
}

function gradeBadgeTone(
  grade: string,
): React.ComponentProps<typeof Badge>["tone"] {
  if (grade === "A" || grade === "B") return "success";
  if (grade === "C") return "warning";
  if (grade === "D" || grade === "F") return "danger";
  return "muted";
}

function gradeStatTone(grade: string): StatCardProps["tone"] {
  if (grade === "A" || grade === "B") return "success";
  if (grade === "C") return "warning";
  if (grade === "D" || grade === "F") return "danger";
  return "muted";
}

function FindingSummary({ file }: { file: CodeIntelHealthFile }) {
  const impacts = file.health_impact ?? [];
  if (impacts.length > 0) {
    const [first] = impacts;
    return (
      <div>
        <div className={styles.badgeRow}>
          <Badge tone={severityTone(first.severity)} variant="soft">
            {first.severity}
          </Badge>
          <Badge tone="muted" variant="outline">
            {first.biomarker}
          </Badge>
          <Badge tone="warning" variant="soft">
            -{formatMaybeFixed(first.deduction, 1)} health
          </Badge>
        </div>
        <MetaText>{first.detail}</MetaText>
      </div>
    );
  }

  if (file.findings.length === 0) {
    return <MetaText>No findings in top sample</MetaText>;
  }

  const [first] = file.findings;

  return (
    <div className={styles.badgeRow}>
      <Badge tone={severityTone(first.severity)} variant="soft">
        {first.severity}
      </Badge>
      <Badge tone="muted" variant="outline">
        {first.biomarker}
      </Badge>
    </div>
  );
}

const columns = [
  {
    id: "path",
    header: "File",
    cell: (file: CodeIntelHealthFile) => (
      <div>
        <PathText path={file.path} />
        <MetaText>{file.lang}</MetaText>
      </div>
    ),
    sortValue: (file: CodeIntelHealthFile) => file.path,
  },
  {
    id: "grade",
    header: "Grade",
    cell: (file: CodeIntelHealthFile) => (
      <Badge tone={gradeBadgeTone(file.grade)} variant="soft">
        {file.grade}
      </Badge>
    ),
    sortValue: (file: CodeIntelHealthFile) => file.score,
  },
  {
    id: "score",
    header: "Score",
    cell: (file: CodeIntelHealthFile) => formatMaybeFixed(file.score, 2),
    sortValue: (file: CodeIntelHealthFile) => file.score,
    align: "end" as const,
  },
  {
    id: "complexity",
    header: "Complexity",
    cell: (file: CodeIntelHealthFile) => formatNumber(file.max_complexity),
    sortValue: (file: CodeIntelHealthFile) => file.max_complexity,
    align: "end" as const,
  },
  {
    id: "maintainability",
    header: "Maintainability",
    cell: (file: CodeIntelHealthFile) =>
      formatMaybeFixed(file.avg_maintainability, 1),
    sortValue: (file: CodeIntelHealthFile) => file.avg_maintainability,
    align: "end" as const,
  },
  {
    id: "functions",
    header: "Functions",
    cell: (file: CodeIntelHealthFile) => formatNumber(file.function_count),
    sortValue: (file: CodeIntelHealthFile) => file.function_count,
    align: "end" as const,
  },
  {
    id: "duplication",
    header: "Duplication",
    cell: (file: CodeIntelHealthFile) =>
      formatPercent(file.duplication_pct * 100, 1),
    sortValue: (file: CodeIntelHealthFile) => file.duplication_pct,
    align: "end" as const,
  },
  {
    id: "findings",
    header: "Top finding",
    cell: (file: CodeIntelHealthFile) => <FindingSummary file={file} />,
    sortValue: (file: CodeIntelHealthFile) => file.biomarker_count,
  },
];

export function HealthTab() {
  const result = useGetCodeIntelHealthQuery({ limit: 25 });
  const theme = useChartTheme();

  return (
    <CodeIntelTabScaffold
      result={result}
      loadingLabel="Loading code health"
      errorTitle="Failed to load code health"
      errorDescription="The Code Intelligence health endpoint could not be reached."
      emptyIcon={ListTree}
      emptyTitle="No health data yet"
      emptyDescription="Once CodeGraph indexes source text, per-file health metrics will appear here."
      isEmpty={isEmpty}
      readinessKey="health"
    >
      {(health) => {
        const files = [...health.files].sort((a, b) =>
          a.score === b.score
            ? a.path.localeCompare(b.path)
            : a.score - b.score,
        );
        const top = files.slice(0, 12);
        const scoreOption = {
          tooltip: chartTooltip(theme, "axis"),
          legend: chartLegend(theme, { data: ["Score", "Maintainability"] }),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((file) => file.path),
              {
                axisLabel: {
                  color: theme.muted,
                  interval: 0,
                  rotate: 30,
                  overflow: "truncate",
                  width: 100,
                },
              },
            ),
          ],
          yAxis: [valueAxis(theme)],
          series: [
            {
              name: "Score",
              type: "bar",
              data: top.map((file) => Number(file.score.toFixed(2))),
              itemStyle: { color: theme.danger },
            },
            {
              name: "Maintainability",
              type: "bar",
              data: top.map((file) =>
                Number(file.avg_maintainability.toFixed(1)),
              ),
              itemStyle: { color: theme.palette[1] },
            },
          ],
        };
        const complexityOption = {
          tooltip: chartTooltip(theme, "axis"),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((file) => file.path),
              {
                axisLabel: {
                  color: theme.muted,
                  interval: 0,
                  rotate: 30,
                  overflow: "truncate",
                  width: 100,
                },
              },
            ),
          ],
          yAxis: [valueAxis(theme)],
          series: [
            {
              name: "Max complexity",
              type: "bar",
              data: top.map((file) => file.max_complexity),
              itemStyle: { color: theme.warning },
            },
          ],
        };

        return (
          <>
            <StatSection title="Health aggregate" icon={Activity}>
              <StatCard
                icon={Gauge}
                title="Average score"
                value={formatMaybeFixed(health.aggregate.avg_score, 2)}
                subtitle={`Project grade ${health.aggregate.grade}`}
                tone={gradeStatTone(health.aggregate.grade)}
              />
              <StatCard
                icon={ListTree}
                title="Files"
                value={formatCompact(health.aggregate.file_count)}
                subtitle={`${formatNumber(
                  health.aggregate.function_count,
                )} functions analyzed`}
              />
              <StatCard
                icon={BarChart3}
                title="Max complexity"
                value={formatNumber(health.aggregate.max_complexity)}
                subtitle="worst indexed file"
                tone={
                  health.aggregate.max_complexity > 20 ? "warning" : "muted"
                }
              />
              <StatCard
                icon={Wrench}
                title="Refactorings"
                value={formatCompact(health.aggregate.refactoring_count)}
                subtitle={`${formatNumber(
                  health.aggregate.biomarker_count,
                )} biomarkers found`}
                tone={
                  health.aggregate.refactoring_count > 0 ? "warning" : "success"
                }
              />
              <StatCard
                icon={ShieldAlert}
                title="Duplication"
                value={formatPercent(
                  health.aggregate.avg_duplication_pct * 100,
                  1,
                )}
                subtitle="average per indexed file"
              />
            </StatSection>

            <div className={styles.chartGrid}>
              <ChartCard
                icon={BarChart3}
                title="Worst files by score"
                option={scoreOption}
              />
              <ChartCard
                icon={Gauge}
                title="Worst files by complexity"
                option={complexityOption}
              />
            </div>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={files}
                columns={columns}
                getRowId={(file) => file.path}
                enableSorting
                wide
                caption="Worst files"
                emptyMessage="No health metrics available."
              />
            </Surface>
          </>
        );
      }}
    </CodeIntelTabScaffold>
  );
}
