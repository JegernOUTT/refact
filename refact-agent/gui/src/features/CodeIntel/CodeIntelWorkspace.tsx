import React, { useState } from "react";
import {
  ArrowLeft,
  Boxes,
  FileCode2,
  GitBranch,
  ListTree,
  Network,
  ShieldAlert,
  Workflow,
} from "lucide-react";

import {
  Button,
  Card,
  EmptyState,
  ErrorState,
  LoadingState,
  Tabs,
} from "../../components/ui";
import { PageWrapper } from "../../components/PageWrapper";
import { useGetCodeIntelOverviewQuery } from "../../services/refact/codeIntel";
import type {
  CodeIntelDetail,
  CodeIntelFileScoreEntry,
  CodeIntelOverview,
  CodeIntelResponse,
  CodeIntelScoreEntry,
} from "../../services/refact/types";
import type { Config } from "../Config/configSlice";
import { StatCard } from "../StatsDashboard/components/StatCard";
import { StatSection } from "../StatsDashboard/components/StatSection";
import {
  formatCompact,
  formatNumber,
} from "../StatsDashboard/utils/formatters";
import styles from "./CodeIntelWorkspace.module.css";

type CodeIntelTab = "overview" | "graph" | "health" | "risk" | "security";

type CodeIntelWorkspaceProps = {
  host: Config["host"];
  backFromCodeIntel: () => void;
};

type RankingItem = {
  label: string;
  score: number;
  meta?: string;
};

const TAB_ORDER: CodeIntelTab[] = [
  "overview",
  "graph",
  "health",
  "risk",
  "security",
];

function isCodeIntelDetail(
  response: CodeIntelResponse<CodeIntelOverview> | undefined,
): response is CodeIntelDetail {
  return typeof response === "object" && "detail" in response;
}

function formatScore(score: number): string {
  if (!Number.isFinite(score)) return "—";
  if (Math.abs(score) >= 1) return score.toFixed(2);
  return score.toFixed(4);
}

function isOverviewEmpty(overview: CodeIntelOverview): boolean {
  return (
    overview.counts.nodes === 0 &&
    overview.counts.edges === 0 &&
    overview.counts.files === 0 &&
    overview.scc_count === 0 &&
    overview.component_count === 0 &&
    overview.community_count === 0 &&
    overview.dead_code_count === 0 &&
    overview.top_pagerank.length === 0 &&
    overview.top_betweenness.length === 0 &&
    overview.file_centrality.top_pagerank.length === 0 &&
    overview.file_centrality.top_betweenness.length === 0
  );
}

function toSymbolRanking(entry: CodeIntelScoreEntry): RankingItem {
  return {
    label: entry.symbol,
    score: entry.score,
  };
}

function toFileRanking(entry: CodeIntelFileScoreEntry): RankingItem {
  return {
    label: entry.path,
    score: entry.score,
    meta: "file",
  };
}

type RankingCardProps = {
  title: string;
  items: RankingItem[];
  emptyLabel: string;
};

function RankingCard({ title, items, emptyLabel }: RankingCardProps) {
  return (
    <Card className={styles.rankingCard} padding="md" variant="glass">
      <h4 className={styles.rankingTitle}>{title}</h4>
      {items.length === 0 ? (
        <p className={styles.emptyList}>{emptyLabel}</p>
      ) : (
        <ol className={styles.rankingList}>
          {items.map((item, index) => (
            <li className={styles.rankingItem} key={`${item.label}-${index}`}>
              <div className={styles.rankingMain}>
                <span className={styles.rankingLabel}>{item.label}</span>
                {item.meta ? (
                  <span className={styles.rankingMeta}>{item.meta}</span>
                ) : null}
              </div>
              <span className={styles.rankingScore}>
                {formatScore(item.score)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </Card>
  );
}

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

function OverviewLiveTab() {
  const { data, error, isFetching, isLoading } =
    useGetCodeIntelOverviewQuery(undefined);
  const overview = isCodeIntelDetail(data) ? null : data;

  if (isLoading) {
    return (
      <LoadingState
        label="Loading code intelligence overview"
        kind="skeleton"
        variant="full"
      />
    );
  }

  if (error) {
    return (
      <Card className={styles.stateCard} padding="lg" variant="glass">
        <ErrorState
          title="Failed to load code intelligence overview"
          description="The code intelligence overview endpoint could not be reached."
          variant="full"
        />
      </Card>
    );
  }

  if (isCodeIntelDetail(data)) {
    return <CodeIntelUnavailable detail={data.detail} />;
  }

  if (!overview || isOverviewEmpty(overview)) {
    return (
      <Card className={styles.stateCard} padding="lg" variant="glass">
        <EmptyState
          icon={Network}
          title="No code intelligence data yet"
          description="Once CodeGraph indexes the workspace, graph metrics and centrality leaders will appear here."
          variant="full"
        />
      </Card>
    );
  }

  return (
    <div className={styles.overview}>
      {isFetching ? (
        <p className={styles.refreshing}>Refreshing overview…</p>
      ) : null}
      <StatSection title="Graph size" icon={Network}>
        <StatCard
          icon={Boxes}
          title="Nodes"
          value={formatNumber(overview.counts.nodes)}
          subtitle="symbols in the CodeGraph"
        />
        <StatCard
          icon={Workflow}
          title="Edges"
          value={formatNumber(overview.counts.edges)}
          subtitle="relationships between symbols"
        />
        <StatCard
          icon={FileCode2}
          title="Files"
          value={formatNumber(overview.counts.files)}
          subtitle="indexed source files"
        />
      </StatSection>

      <StatSection title="Topology" icon={ListTree}>
        <StatCard
          icon={Network}
          title="SCCs"
          value={formatCompact(overview.scc_count)}
          subtitle={`${formatNumber(
            overview.largest_scc,
          )} nodes in largest SCC`}
          tone={overview.scc_count > 0 ? "warning" : "muted"}
        />
        <StatCard
          icon={Boxes}
          title="Components"
          value={formatCompact(overview.component_count)}
          subtitle="connected graph components"
        />
        <StatCard
          icon={Workflow}
          title="Communities"
          value={formatCompact(overview.community_count)}
          subtitle="detected code communities"
        />
        <StatCard
          icon={ShieldAlert}
          title="Dead Code"
          value={formatCompact(overview.dead_code_count)}
          subtitle="candidate unreachable symbols"
          tone={overview.dead_code_count > 0 ? "warning" : "success"}
        />
      </StatSection>

      <StatSection title="Centrality leaders" icon={GitBranch} dense>
        <RankingCard
          title="Top PageRank"
          items={overview.top_pagerank.map(toSymbolRanking)}
          emptyLabel="No PageRank leaders yet."
        />
        <RankingCard
          title="Top Betweenness"
          items={overview.top_betweenness.map(toSymbolRanking)}
          emptyLabel="No betweenness leaders yet."
        />
        <RankingCard
          title="File PageRank"
          items={overview.file_centrality.top_pagerank.map(toFileRanking)}
          emptyLabel="No file PageRank data yet."
        />
        <RankingCard
          title="File Betweenness"
          items={overview.file_centrality.top_betweenness.map(toFileRanking)}
          emptyLabel="No file betweenness data yet."
        />
      </StatSection>
    </div>
  );
}

type PlaceholderProps = {
  title: string;
  description: string;
  icon: React.ComponentProps<typeof EmptyState>["icon"];
};

function PlaceholderTab({ title, description, icon }: PlaceholderProps) {
  return (
    <Card className={styles.stateCard} padding="lg" variant="glass">
      <EmptyState
        icon={icon}
        title={title}
        description={description}
        variant="full"
      />
    </Card>
  );
}

function GraphTabPlaceholder() {
  return (
    <PlaceholderTab
      icon={Network}
      title="Graph view coming soon"
      description="The graph tab is reserved for the interactive CodeGraph visualization."
    />
  );
}

function HealthTabPlaceholder() {
  return (
    <PlaceholderTab
      icon={ListTree}
      title="Health metrics coming soon"
      description="Code health, duplication, and trend panels will land in the follow-up stats card."
    />
  );
}

function RiskTabPlaceholder() {
  return (
    <PlaceholderTab
      icon={GitBranch}
      title="Risk analysis coming soon"
      description="Git risk, blast radius, and ownership summaries will fill this tab next."
    />
  );
}

function SecurityTabPlaceholder() {
  return (
    <PlaceholderTab
      icon={ShieldAlert}
      title="Security scan coming soon"
      description="Security findings will appear here after the dedicated security tab work."
    />
  );
}

export const CodeIntelWorkspace: React.FC<CodeIntelWorkspaceProps> = ({
  host,
  backFromCodeIntel,
}) => {
  const [activeTab, setActiveTab] = useState<CodeIntelTab>("overview");

  return (
    <PageWrapper host={host}>
      <div className={styles.root}>
        <header className={styles.header}>
          <Button
            leftIcon={ArrowLeft}
            onClick={backFromCodeIntel}
            size="sm"
            variant="ghost"
          >
            Back
          </Button>
          <div className={styles.headerCopy}>
            <h2 className={styles.title}>Code Intelligence</h2>
            <p className={styles.subtitle}>
              Explore CodeGraph structure, health, risk, and security signals.
            </p>
          </div>
          <div className={styles.headerSpacer} />
        </header>

        <Tabs
          value={activeTab}
          onValueChange={(value) => setActiveTab(value as CodeIntelTab)}
          className={styles.tabsRoot}
        >
          <Tabs.List
            activeIndex={TAB_ORDER.indexOf(activeTab)}
            className={styles.tabsList}
          >
            <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
            <Tabs.Trigger value="graph">Graph</Tabs.Trigger>
            <Tabs.Trigger value="health">Health</Tabs.Trigger>
            <Tabs.Trigger value="risk">Risk</Tabs.Trigger>
            <Tabs.Trigger value="security">Security</Tabs.Trigger>
          </Tabs.List>

          <Tabs.Content value="overview" className={styles.tabContent}>
            <OverviewLiveTab />
          </Tabs.Content>
          <Tabs.Content value="graph" className={styles.tabContent}>
            <GraphTabPlaceholder />
          </Tabs.Content>
          <Tabs.Content value="health" className={styles.tabContent}>
            <HealthTabPlaceholder />
          </Tabs.Content>
          <Tabs.Content value="risk" className={styles.tabContent}>
            <RiskTabPlaceholder />
          </Tabs.Content>
          <Tabs.Content value="security" className={styles.tabContent}>
            <SecurityTabPlaceholder />
          </Tabs.Content>
        </Tabs>
      </div>
    </PageWrapper>
  );
};
