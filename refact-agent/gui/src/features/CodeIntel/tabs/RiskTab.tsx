import React from "react";
import {
  BarChart3,
  GitBranch,
  GitCommitHorizontal,
  ShieldAlert,
  UserRoundCheck,
  UsersRound,
} from "lucide-react";

import { Badge, Card, DataTable, Surface } from "../../../components/ui";
import { useGetCodeIntelGitRiskQuery } from "../../../services/refact/codeIntel";
import type {
  CodeIntelGitHotspot,
  CodeIntelGitOwnership,
  CodeIntelGitRisk,
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
  chartLegend,
  chartTooltip,
  useChartTheme,
  valueAxis,
} from "../../StatsDashboard/utils/chartTheme";
import styles from "./CodeIntelStatsTabs.module.css";
import { ChartCard, CodeIntelTabScaffold, MetaText, PathText } from "./shared";
import { formatMaybeFixed, formatRatio } from "./tabUtils";

function isEmpty(risk: CodeIntelGitRisk): boolean {
  return (
    risk.commits_analyzed === 0 &&
    risk.hotspots.length === 0 &&
    risk.ownership.length === 0 &&
    risk.co_change.length === 0 &&
    risk.coupling.length === 0
  );
}

function riskTone(value: number): React.ComponentProps<typeof Badge>["tone"] {
  if (value >= 0.8) return "danger";
  if (value >= 0.5) return "warning";
  return "muted";
}

const hotspotColumns = [
  {
    id: "path",
    header: "Hotspot",
    cell: (hotspot: CodeIntelGitHotspot) => <PathText path={hotspot.path} />,
    sortValue: (hotspot: CodeIntelGitHotspot) => hotspot.path,
  },
  {
    id: "risk",
    header: "Risk",
    cell: (hotspot: CodeIntelGitHotspot) => (
      <Badge tone={riskTone(hotspot.risk)} variant="soft">
        {formatRatio(hotspot.risk, 0)}
      </Badge>
    ),
    sortValue: (hotspot: CodeIntelGitHotspot) => hotspot.risk,
    align: "end" as const,
  },
  {
    id: "churn",
    header: "Churn",
    cell: (hotspot: CodeIntelGitHotspot) => formatNumber(hotspot.churn),
    sortValue: (hotspot: CodeIntelGitHotspot) => hotspot.churn,
    align: "end" as const,
  },
  {
    id: "entropy",
    header: "Entropy",
    cell: (hotspot: CodeIntelGitHotspot) =>
      formatPercent(hotspot.change_entropy_pct * 100, 0),
    sortValue: (hotspot: CodeIntelGitHotspot) => hotspot.change_entropy_pct,
    align: "end" as const,
  },
  {
    id: "busFactor",
    header: "Bus factor",
    cell: (hotspot: CodeIntelGitHotspot) => formatNumber(hotspot.bus_factor),
    sortValue: (hotspot: CodeIntelGitHotspot) => hotspot.bus_factor,
    align: "end" as const,
  },
  {
    id: "signals",
    header: "Signals",
    cell: (hotspot: CodeIntelGitHotspot) => (
      <div className={styles.badgeRow}>
        {hotspot.ownership_risk ? (
          <Badge tone="warning" variant="soft">
            ownership
          </Badge>
        ) : null}
        {hotspot.knowledge_loss ? (
          <Badge tone="danger" variant="soft">
            knowledge loss
          </Badge>
        ) : null}
        {!hotspot.ownership_risk && !hotspot.knowledge_loss ? (
          <Badge tone="success" variant="soft">
            balanced
          </Badge>
        ) : null}
      </div>
    ),
    sortValue: (hotspot: CodeIntelGitHotspot) =>
      Number(hotspot.ownership_risk) + Number(hotspot.knowledge_loss),
  },
];

const ownershipColumns = [
  {
    id: "path",
    header: "File",
    cell: (entry: CodeIntelGitOwnership) => <PathText path={entry.path} />,
    sortValue: (entry: CodeIntelGitOwnership) => entry.path,
  },
  {
    id: "owner",
    header: "Top owner",
    cell: (entry: CodeIntelGitOwnership) => (
      <div>
        <p className={styles.pathText} title={entry.top_owner}>
          {entry.top_owner || "—"}
        </p>
        <MetaText>
          {formatPercent(entry.top_owner_share * 100, 0)} share
        </MetaText>
      </div>
    ),
    sortValue: (entry: CodeIntelGitOwnership) => entry.top_owner_share,
  },
  {
    id: "busFactor",
    header: "Bus factor",
    cell: (entry: CodeIntelGitOwnership) => formatNumber(entry.bus_factor),
    sortValue: (entry: CodeIntelGitOwnership) => entry.bus_factor,
    align: "end" as const,
  },
  {
    id: "owners",
    header: "Owners",
    cell: (entry: CodeIntelGitOwnership) => formatNumber(entry.owner_count),
    sortValue: (entry: CodeIntelGitOwnership) => entry.owner_count,
    align: "end" as const,
  },
  {
    id: "risk",
    header: "Risk",
    cell: (entry: CodeIntelGitOwnership) => (
      <div className={styles.badgeRow}>
        {entry.ownership_risk ? (
          <Badge tone="warning" variant="soft">
            ownership
          </Badge>
        ) : null}
        {entry.knowledge_loss ? (
          <Badge tone="danger" variant="soft">
            knowledge loss
          </Badge>
        ) : null}
        {!entry.ownership_risk && !entry.knowledge_loss ? (
          <Badge tone="success" variant="soft">
            healthy
          </Badge>
        ) : null}
      </div>
    ),
    sortValue: (entry: CodeIntelGitOwnership) =>
      Number(entry.ownership_risk) + Number(entry.knowledge_loss),
  },
];

export function RiskTab() {
  const result = useGetCodeIntelGitRiskQuery({ limit: 25 });
  const theme = useChartTheme();

  return (
    <CodeIntelTabScaffold
      result={result}
      loadingLabel="Loading git risk"
      errorTitle="Failed to load git risk"
      errorDescription="The Code Intelligence git-risk endpoint could not be reached."
      emptyIcon={GitBranch}
      emptyTitle="No git risk data yet"
      emptyDescription="Git risk analysis needs a project git history and indexed CodeGraph data."
      isEmpty={isEmpty}
    >
      {(risk) => {
        const hotspots = [...risk.hotspots].sort((a, b) =>
          b.risk === a.risk ? a.path.localeCompare(b.path) : b.risk - a.risk,
        );
        const ownership = [...risk.ownership].sort((a, b) =>
          a.bus_factor === b.bus_factor
            ? b.top_owner_share - a.top_owner_share
            : a.bus_factor - b.bus_factor,
        );
        const top = hotspots.slice(0, 10);
        const riskSignals = hotspots.filter(
          (hotspot) => hotspot.ownership_risk || hotspot.knowledge_loss,
        ).length;
        const hotspotOption = {
          tooltip: chartTooltip(theme, "axis"),
          legend: chartLegend(theme, { data: ["Risk", "Churn"] }),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              top.map((hotspot) => hotspot.path),
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
              name: "Risk",
              type: "bar",
              data: top.map((hotspot) =>
                Number((hotspot.risk * 100).toFixed(0)),
              ),
              itemStyle: { color: theme.danger },
            },
            {
              name: "Churn",
              type: "bar",
              data: top.map((hotspot) => hotspot.churn),
              itemStyle: { color: theme.warning },
            },
          ],
        };
        const busFactorOption = {
          tooltip: chartTooltip(theme, "axis"),
          grid: chartGrid({ bottom: "24%" }),
          xAxis: [
            categoryAxis(
              theme,
              ownership.slice(0, 10).map((entry) => entry.path),
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
              name: "Bus factor",
              type: "bar",
              data: ownership.slice(0, 10).map((entry) => entry.bus_factor),
              itemStyle: { color: theme.palette[1] },
            },
          ],
        };

        return (
          <>
            <StatSection title="Git risk summary" icon={GitBranch}>
              <StatCard
                icon={GitCommitHorizontal}
                title="Commits analyzed"
                value={formatCompact(risk.commits_analyzed)}
                subtitle={`${formatPercent(
                  risk.agent_authored_pct * 100,
                  0,
                )} agent-authored`}
              />
              <StatCard
                icon={ShieldAlert}
                title="Hotspots"
                value={formatCompact(hotspots.length)}
                subtitle={`${formatNumber(riskSignals)} ownership risk signals`}
                tone={riskSignals > 0 ? "warning" : "success"}
              />
              <StatCard
                icon={UsersRound}
                title="Ownership files"
                value={formatCompact(ownership.length)}
                subtitle="with ownership data"
              />
              <StatCard
                icon={UserRoundCheck}
                title="Top reviewer"
                value={
                  risk.reviewers.length > 0
                    ? formatMaybeFixed(risk.reviewers[0].score, 2)
                    : "—"
                }
                subtitle={
                  risk.reviewers.length > 0
                    ? risk.reviewers[0].author
                    : "No reviewer suggestions"
                }
              />
            </StatSection>

            <div className={styles.chartGrid}>
              <ChartCard
                icon={BarChart3}
                title="Risk hotspots"
                option={hotspotOption}
              />
              <ChartCard
                icon={UsersRound}
                title="Lowest bus factors"
                option={busFactorOption}
              />
            </div>

            {risk.reviewers.length > 0 ? (
              <Card className={styles.summaryCard} variant="glass">
                <h4 className={styles.summaryTitle}>
                  <UserRoundCheck size={16} />
                  Suggested reviewers
                </h4>
                <ul className={styles.summaryList}>
                  {risk.reviewers.slice(0, 5).map((reviewer) => (
                    <li className={styles.summaryItem} key={reviewer.author}>
                      <p className={styles.pathText} title={reviewer.author}>
                        {reviewer.author}
                      </p>
                      <p className={styles.summaryText}>
                        Score {formatMaybeFixed(reviewer.score, 2)}
                      </p>
                    </li>
                  ))}
                </ul>
              </Card>
            ) : null}

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={hotspots}
                columns={hotspotColumns}
                getRowId={(hotspot) => hotspot.path}
                enableSorting
                wide
                caption="Git risk hotspots"
                emptyMessage="No hotspots found."
              />
            </Surface>

            <Surface className={styles.tableSurface} variant="glass">
              <DataTable
                rows={ownership}
                columns={ownershipColumns}
                getRowId={(entry) => entry.path}
                enableSorting
                wide
                caption="Ownership and bus factor"
                emptyMessage="No ownership data found."
              />
            </Surface>
          </>
        );
      }}
    </CodeIntelTabScaffold>
  );
}
