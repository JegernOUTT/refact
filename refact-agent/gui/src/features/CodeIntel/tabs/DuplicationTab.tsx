import React from "react";
import {
  BarChart3,
  Copy,
  Files,
  GitCompareArrows,
  Percent,
} from "lucide-react";

import { Badge, DataTable, Surface } from "../../../components/ui";
import { useGetCodeIntelDuplicationQuery } from "../../../services/refact/codeIntel";
import type {
  CodeIntelDuplication,
  CodeIntelDuplicationClone,
  CodeIntelDuplicationFinding,
} from "../../../services/refact/types";
import { StatCard } from "../../StatsDashboard/components/StatCard";
import { StatSection } from "../../StatsDashboard/components/StatSection";
import {
  formatCompact,
  formatNumber,
  formatPercent,
} from "../../StatsDashboard/utils/formatters";
import {
  categoryAxis,
  chartGrid,
  chartTooltip,
  useChartTheme,
  valueAxis,
} from "../../StatsDashboard/utils/chartTheme";
import styles from "./CodeIntelStatsTabs.module.css";
import { ChartCard, CodeIntelTabScaffold, MetaText, PathText } from "./shared";

function isEmpty(duplication: CodeIntelDuplication): boolean {
  return (
    duplication.aggregate.file_count === 0 &&
    duplication.clones.length === 0 &&
    duplication.dry_violations.length === 0 &&
    duplication.test_smells.length === 0
  );
}

function severityTone(
  severity: CodeIntelDuplicationFinding["severity"],
): React.ComponentProps<typeof Badge>["tone"] {
  if (severity === "Critical" || severity === "High") return "danger";
  if (severity === "Medium") return "warning";
  return "muted";
}

const cloneColumns = [
  {
    id: "pathA",
    header: "Path A",
    cell: (clone: CodeIntelDuplicationClone) => (
      <div>
        <PathText path={clone.path_a} />
        <MetaText>
          Lines {clone.a_start_line}–{clone.a_end_line}
        </MetaText>
      </div>
    ),
    sortValue: (clone: CodeIntelDuplicationClone) => clone.path_a,
  },
  {
    id: "pathB",
    header: "Path B",
    cell: (clone: CodeIntelDuplicationClone) => (
      <div>
        <PathText path={clone.path_b} />
        <MetaText>
          Lines {clone.b_start_line}–{clone.b_end_line}
        </MetaText>
      </div>
    ),
    sortValue: (clone: CodeIntelDuplicationClone) => clone.path_b,
  },
  {
    id: "lines",
    header: "Lines",
    cell: (clone: CodeIntelDuplicationClone) => formatNumber(clone.lines),
    sortValue: (clone: CodeIntelDuplicationClone) => clone.lines,
    align: "end" as const,
  },
  {
    id: "tokens",
    header: "Tokens",
    cell: (clone: CodeIntelDuplicationClone) => formatNumber(clone.token_len),
    sortValue: (clone: CodeIntelDuplicationClone) => clone.token_len,
    align: "end" as const,
  },
  {
    id: "coChange",
    header: "Co-change",
    cell: (clone: CodeIntelDuplicationClone) => formatNumber(clone.co_change),
    sortValue: (clone: CodeIntelDuplicationClone) => clone.co_change,
    align: "end" as const,
  },
];

const findingColumns = [
  {
    id: "path",
    header: "Path",
    cell: (finding: CodeIntelDuplicationFinding) => (
      <div>
        <PathText path={finding.path} />
        <MetaText>Line {finding.line}</MetaText>
      </div>
    ),
    sortValue: (finding: CodeIntelDuplicationFinding) => finding.path,
  },
  {
    id: "severity",
    header: "Severity",
    cell: (finding: CodeIntelDuplicationFinding) => (
      <Badge tone={severityTone(finding.severity)} variant="soft">
        {finding.severity}
      </Badge>
    ),
    sortValue: (finding: CodeIntelDuplicationFinding) => finding.severity,
  },
  {
    id: "biomarker",
    header: "Biomarker",
    cell: (finding: CodeIntelDuplicationFinding) => finding.biomarker,
    sortValue: (finding: CodeIntelDuplicationFinding) => finding.biomarker,
  },
  {
    id: "detail",
    header: "Detail",
    cell: (finding: CodeIntelDuplicationFinding) => (
      <MetaText>{finding.detail}</MetaText>
    ),
    sortValue: (finding: CodeIntelDuplicationFinding) => finding.detail,
  },
];

export function DuplicationTab() {
  const result = useGetCodeIntelDuplicationQuery({ limit: 25 });
  const theme = useChartTheme();

  return (
    <CodeIntelTabScaffold
      result={result}
      loadingLabel="Loading duplication metrics"
      errorTitle="Failed to load duplication metrics"
      errorDescription="The Code Intelligence duplication endpoint could not be reached."
      emptyIcon={Copy}
      emptyTitle="No duplication data yet"
      emptyDescription="Once CodeGraph indexes source text, clone pairs and DRY signals will appear here."
      isEmpty={isEmpty}
    >
      {(duplication) => {
        const clones = [...duplication.clones].sort((a, b) =>
          b.token_len === a.token_len
            ? a.path_a.localeCompare(b.path_a)
            : b.token_len - a.token_len,
        );
        const top = clones.slice(0, 10);
        const findings = [
          ...duplication.dry_violations,
          ...duplication.test_smells,
        ];
        const chartOption = {
          tooltip: chartTooltip(theme, "axis"),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((clone) => clone.path_a),
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
              name: "Clone tokens",
              type: "bar",
              data: top.map((clone) => clone.token_len),
              itemStyle: { color: theme.palette[0] },
            },
          ],
        };

        return (
          <>
            <StatSection title="Duplication summary" icon={Copy}>
              <StatCard
                icon={Files}
                title="Files analyzed"
                value={formatCompact(duplication.aggregate.file_count)}
                subtitle="indexed source files"
              />
              <StatCard
                icon={Copy}
                title="Clone pairs"
                value={formatCompact(duplication.aggregate.clone_pair_count)}
                subtitle="cross-file duplicate regions"
                tone={
                  duplication.aggregate.clone_pair_count > 0
                    ? "warning"
                    : "success"
                }
              />
              <StatCard
                icon={Percent}
                title="Duplication"
                value={formatPercent(
                  duplication.aggregate.duplication_percent,
                  1,
                )}
                subtitle="cross-file duplicated token share"
              />
              <StatCard
                icon={GitCompareArrows}
                title="DRY signals"
                value={formatCompact(findings.length)}
                subtitle={`${formatNumber(
                  duplication.test_smells.length,
                )} test-smell findings`}
                tone={findings.length > 0 ? "warning" : "success"}
              />
            </StatSection>

            <div className={styles.chartGrid}>
              <ChartCard
                icon={BarChart3}
                title="Largest clone pairs"
                option={chartOption}
              />
            </div>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={clones}
                columns={cloneColumns}
                getRowId={(clone, index) =>
                  `${clone.path_a}:${clone.a_start_line}:${clone.path_b}:${clone.b_start_line}:${index}`
                }
                enableSorting
                wide
                caption="Clone pairs"
                emptyMessage="No clone pairs found."
              />
            </Surface>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={findings}
                columns={findingColumns}
                getRowId={(finding, index) =>
                  `${finding.path}:${finding.line}:${finding.biomarker}:${index}`
                }
                enableSorting
                wide
                caption="DRY violations and test smells"
                emptyMessage="No duplication findings found."
              />
            </Surface>
          </>
        );
      }}
    </CodeIntelTabScaffold>
  );
}
