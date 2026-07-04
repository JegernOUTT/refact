import { BarChart3, Network, Workflow } from "lucide-react";

import { DataTable, Surface } from "../../../components/ui";
import { useGetCodeIntelCommunitiesQuery } from "../../../services/refact/codeIntel";
import type { CodeIntelCommunity } from "../../../services/refact/types";
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
import { ChartCard, CodeIntelTabScaffold } from "./shared";
import { formatRatio } from "./tabUtils";

function isEmpty(communities: CodeIntelCommunity[]): boolean {
  return communities.length === 0;
}

function averageCohesion(communities: CodeIntelCommunity[]): number {
  if (communities.length === 0) return 0;
  return (
    communities.reduce((sum, community) => sum + community.cohesion, 0) /
    communities.length
  );
}

const columns = [
  {
    id: "label",
    header: "Community",
    cell: (community: CodeIntelCommunity) => community.label,
    sortValue: (community: CodeIntelCommunity) => community.label,
  },
  {
    id: "members",
    header: "Members",
    cell: (community: CodeIntelCommunity) =>
      formatNumber(community.member_count),
    sortValue: (community: CodeIntelCommunity) => community.member_count,
    align: "end" as const,
  },
  {
    id: "cohesion",
    header: "Cohesion",
    cell: (community: CodeIntelCommunity) => formatRatio(community.cohesion, 1),
    sortValue: (community: CodeIntelCommunity) => community.cohesion,
    align: "end" as const,
  },
];

export function CommunitiesTab() {
  const result = useGetCodeIntelCommunitiesQuery(undefined);
  const theme = useChartTheme();

  return (
    <CodeIntelTabScaffold
      result={result}
      loadingLabel="Loading code communities"
      errorTitle="Failed to load code communities"
      errorDescription="The Code Intelligence communities endpoint could not be reached."
      emptyIcon={Workflow}
      emptyTitle="No communities detected"
      emptyDescription="Once CodeGraph has enough relationship data, detected code communities will appear here."
      isEmpty={isEmpty}
      readinessKey="communities"
    >
      {(communities) => {
        const sorted = [...communities].sort((a, b) =>
          b.member_count === a.member_count
            ? a.label.localeCompare(b.label)
            : b.member_count - a.member_count,
        );
        const top = sorted.slice(0, 10);
        const biggest = sorted[0];
        const chartOption = {
          tooltip: chartTooltip(theme, "axis"),
          grid: chartGrid({ bottom: "20%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((community) => community.label),
              {
                axisLabel: {
                  color: theme.muted,
                  interval: 0,
                  rotate: 28,
                  overflow: "truncate",
                  width: 96,
                },
              },
            ),
          ],
          yAxis: [valueAxis(theme)],
          series: [
            {
              name: "Members",
              type: "bar",
              data: top.map((community) => community.member_count),
              itemStyle: { color: theme.palette[0] },
            },
          ],
        };

        return (
          <>
            <StatSection title="Community summary" icon={Workflow}>
              <StatCard
                icon={Workflow}
                title="Communities"
                value={formatCompact(communities.length)}
                subtitle="detected graph communities"
              />
              <StatCard
                icon={Network}
                title="Largest community"
                value={formatCompact(biggest.member_count)}
                subtitle={biggest.label}
              />
              <StatCard
                icon={BarChart3}
                title="Avg cohesion"
                value={formatPercent(averageCohesion(communities) * 100, 1)}
                subtitle="mean internal connectivity"
              />
            </StatSection>

            <div className={styles.chartGrid}>
              <ChartCard
                icon={BarChart3}
                title="Top communities by members"
                option={chartOption}
              />
            </div>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={sorted}
                columns={columns}
                getRowId={(community) => String(community.id)}
                enableSorting
                wide
                caption="Detected code communities"
                emptyMessage="No communities detected."
              />
            </Surface>
          </>
        );
      }}
    </CodeIntelTabScaffold>
  );
}
