import React from "react";
import { BarChart3, FileWarning, Percent, ShieldAlert } from "lucide-react";

import { Badge, DataTable, Surface } from "../../../components/ui";
import { useGetCodeIntelDeadCodeQuery } from "../../../services/refact/codeIntel";
import type {
  CodeIntelDeadCodeReport,
  CodeIntelDeadSymbol,
} from "../../../services/refact/types";
import { StatCard } from "../../StatsDashboard/components/StatCard";
import { StatSection } from "../../StatsDashboard/components/StatSection";
import {
  formatCompact,
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
import { clampRatio } from "./tabUtils";

function isEmpty(report: CodeIntelDeadCodeReport): boolean {
  return report.entries.length === 0;
}

function averageConfidence(symbols: CodeIntelDeadSymbol[]): number {
  if (symbols.length === 0) return 0;
  return (
    symbols.reduce((sum, symbol) => sum + symbol.confidence, 0) / symbols.length
  );
}

function confidenceTone(
  confidence: number,
): React.ComponentProps<typeof Badge>["tone"] {
  if (confidence >= 0.8) return "danger";
  if (confidence >= 0.55) return "warning";
  return "muted";
}

function ConfidenceViz({ confidence }: { confidence: number }) {
  const clamped = clampRatio(confidence);

  return (
    <div className={styles.confidenceCell}>
      <Badge tone={confidenceTone(clamped)} variant="soft">
        {formatPercent(clamped * 100, 0)}
      </Badge>
      <progress
        aria-label={`Confidence ${formatPercent(clamped * 100, 0)}`}
        className={styles.confidenceProgress}
        max={1}
        value={clamped}
      />
    </div>
  );
}

const columns = [
  {
    id: "name",
    header: "Symbol",
    cell: (symbol: CodeIntelDeadSymbol) => symbol.name,
    sortValue: (symbol: CodeIntelDeadSymbol) => symbol.name,
  },
  {
    id: "path",
    header: "Path",
    cell: (symbol: CodeIntelDeadSymbol) => <PathText path={symbol.path} />,
    sortValue: (symbol: CodeIntelDeadSymbol) => symbol.path,
  },
  {
    id: "reason",
    header: "Reason",
    cell: (symbol: CodeIntelDeadSymbol) => <MetaText>{symbol.reason}</MetaText>,
    sortValue: (symbol: CodeIntelDeadSymbol) => symbol.reason,
  },
  {
    id: "confidence",
    header: "Confidence",
    cell: (symbol: CodeIntelDeadSymbol) => (
      <ConfidenceViz confidence={symbol.confidence} />
    ),
    sortValue: (symbol: CodeIntelDeadSymbol) => symbol.confidence,
    align: "end" as const,
  },
];

export function DeadCodeTab() {
  const result = useGetCodeIntelDeadCodeQuery(undefined);
  const theme = useChartTheme();

  return (
    <CodeIntelTabScaffold
      result={result}
      loadingLabel="Loading dead code candidates"
      errorTitle="Failed to load dead code candidates"
      errorDescription="The Code Intelligence dead-code endpoint could not be reached."
      emptyIcon={ShieldAlert}
      emptyTitle="No dead code candidates"
      emptyDescription="CodeGraph did not find unreachable symbols in the indexed workspace."
      isEmpty={isEmpty}
      readinessKey="dead-code"
    >
      {(report) => {
        const symbols = report.entries;
        const sorted = [...symbols].sort((a, b) =>
          b.confidence === a.confidence
            ? a.path.localeCompare(b.path)
            : b.confidence - a.confidence,
        );
        const highConfidence = symbols.filter(
          (symbol) => symbol.confidence >= 0.8,
        ).length;
        const top = sorted.slice(0, 10);
        const chartOption = {
          tooltip: chartTooltip(theme, "axis", {
            valueFormatter: (value: number) => formatPercent(value, 0),
          }),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((symbol) => symbol.name),
              {
                axisLabel: {
                  color: theme.muted,
                  interval: 0,
                  rotate: 30,
                  overflow: "truncate",
                  width: 92,
                },
              },
            ),
          ],
          yAxis: [valueAxis(theme, { max: 100 })],
          series: [
            {
              name: "Confidence",
              type: "bar",
              data: top.map((symbol) => Math.round(symbol.confidence * 100)),
              itemStyle: { color: theme.warning },
            },
          ],
        };

        return (
          <>
            <StatSection title="Dead code summary" icon={ShieldAlert}>
              <StatCard
                icon={ShieldAlert}
                title="Candidates"
                value={formatCompact(symbols.length)}
                subtitle="potentially unreachable symbols"
                tone={symbols.length > 0 ? "warning" : "success"}
              />
              <StatCard
                icon={FileWarning}
                title="High confidence"
                value={formatCompact(highConfidence)}
                subtitle="confidence ≥ 80%"
                tone={highConfidence > 0 ? "danger" : "success"}
              />
              <StatCard
                icon={Percent}
                title="Avg confidence"
                value={formatPercent(averageConfidence(symbols) * 100, 0)}
                subtitle="across all candidates"
              />
            </StatSection>

            <div className={styles.chartGrid}>
              <ChartCard
                icon={BarChart3}
                title="Highest confidence candidates"
                option={chartOption}
              />
            </div>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={sorted}
                columns={columns}
                getRowId={(symbol, index) =>
                  `${symbol.path}:${symbol.name}:${index}`
                }
                enableSorting
                wide
                caption="Dead code candidates"
                emptyMessage="No dead code candidates."
              />
            </Surface>
          </>
        );
      }}
    </CodeIntelTabScaffold>
  );
}
