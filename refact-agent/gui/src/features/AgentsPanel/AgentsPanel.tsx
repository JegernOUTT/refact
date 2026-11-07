import classNames from "classnames";
import { Bot } from "lucide-react";
import { useEffect, useMemo } from "react";

import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
  Icon,
  SegmentedControl,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { useGetBackgroundAgentsQuery } from "../../services/refact";
import { backgroundAgentsApi } from "../../services/refact/backgroundAgents";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import {
  activeBackgroundAgents,
  aggregateBackgroundAgentUsage,
  buildBackgroundAgentsTree,
  flattenBackgroundAgentTree,
  reconcileBackgroundAgents,
} from "../Chat/Thread";
import {
  agentRecoveryRequested,
  selectAgentRosterSource,
  selectDisclosedAgentPool,
  selectAgentsPanelTab,
  tabChanged,
  type AgentRosterSource,
} from "./agentsPanelSlice";
import { AgentTreeNode } from "./AgentTreeNode";
import styles from "./AgentsPanel.module.css";

export type AgentsSectionProps = {
  chatId: string | null;
  onNavigate?: (chatId: string) => void;
};

function formatTokens(tokens: number): string {
  if (tokens < 1000) return `${tokens} tokens`;
  if (tokens < 1_000_000) {
    return `${(tokens / 1000).toFixed(tokens >= 10_000 ? 0 : 1)}k tokens`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}M tokens`;
}

function formatCost(cost: number | null): string | null {
  if (cost === null || !Number.isFinite(cost) || cost <= 0) return null;
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

function AgentsSectionContents({
  chatId,
  source,
  onNavigate,
}: {
  chatId: string;
  source: AgentRosterSource;
  onNavigate?: (chatId: string) => void;
}) {
  const dispatch = useAppDispatch();
  const visible = useIsDocumentVisible();
  const tab = useAppSelector(selectAgentsPanelTab);
  const agentPool = useAppSelector(selectDisclosedAgentPool);
  const { authorized, denied, failure, transportStale } = useAppSelector(
    (state) => state.agentsPanel,
  );
  const ready = useAppSelector((state) =>
    hasUsableEngineEndpoint(state.config),
  );
  const query = useMemo(() => ({ chatId, source }), [chatId, source]);
  const {
    currentData: fetchedAgents,
    isFetching,
    isError,
    refetch,
  } = useGetBackgroundAgentsQuery(query, {
    skip: !ready || denied,
    pollingInterval: visible ? 30_000 : 0,
    refetchOnFocus: true,
    refetchOnMountOrArgChange: true,
  });

  useEffect(
    () => () => {
      dispatch(
        backgroundAgentsApi.util.getRunningQueryThunk(
          "getBackgroundAgents",
          query,
        ),
      )?.abort();
    },
    [dispatch, query],
  );

  const combinedAgents = useMemo(
    () =>
      authorized && !failure && ready && fetchedAgents !== undefined
        ? reconcileBackgroundAgents(agentPool, fetchedAgents)
        : {},
    [agentPool, fetchedAgents, authorized, failure, ready],
  );
  const tree = useMemo(
    () => buildBackgroundAgentsTree(combinedAgents, chatId),
    [combinedAgents, chatId],
  );
  const roster = useMemo(() => flattenBackgroundAgentTree(tree), [tree]);
  const activeAgents = useMemo(() => activeBackgroundAgents(roster), [roster]);
  const visibleTree = useMemo(() => {
    if (tab === "all") return tree;
    const activeIds = new Set(activeAgents.map((agent) => agent.agent_id));
    const filter = (nodes: typeof tree): typeof tree =>
      nodes.flatMap((node) => {
        const children = filter(node.children);
        return activeIds.has(node.agent.agent_id) || children.length > 0
          ? [{ ...node, children }]
          : [];
      });
    return filter(tree);
  }, [activeAgents, tab, tree]);
  const visibleCount = flattenBackgroundAgentTree(visibleTree).length;
  const aggregate = aggregateBackgroundAgentUsage(roster);
  const cost = formatCost(aggregate.costTotal);

  return (
    <>
      <header className={styles.panelHeader}>
        <div className={styles.titleRow}>
          <div className={styles.panelTitle}>
            <Icon icon={Bot} size="sm" />
            <span>Agents</span>
            <Badge
              tone="accent"
              aria-label={`${aggregate.runningCount} running agents`}
            >
              {aggregate.runningCount}
            </Badge>
          </div>
          <div
            className={styles.aggregateUsage}
            data-testid="agents-aggregate-usage"
            title={`${formatTokens(aggregate.tokensTotal)}${
              cost ? ` · ${cost}` : ""
            }`}
          >
            {formatTokens(aggregate.tokensTotal)}
            {cost ? ` · ${cost}` : ""}
          </div>
        </div>
        <SegmentedControl
          aria-label="Agents filter"
          className={styles.tabs}
          layout="content"
          name={`agents-tab-${chatId}`}
          options={[
            { value: "active", label: `Active (${activeAgents.length})` },
            {
              value: "all",
              label: `All (${roster.length})`,
            },
          ]}
          size="sm"
          value={tab}
          onValueChange={(value) => {
            if (value === "active" || value === "all")
              dispatch(tabChanged(value));
          }}
        />
      </header>
      <div className={styles.panelBody}>
        {(isError || Boolean(failure) || !ready || transportStale) && (
          <ErrorState
            title="Could not refresh agents"
            description={
              Boolean(failure) || !ready
                ? "Agent data is hidden until access is revalidated for this connection and project."
                : fetchedAgents !== undefined || roster.length > 0
                  ? "Showing last known agent data, which may be out of date."
                  : "Agent data is unavailable. Retry to load this chat's agents."
            }
            retry={
              <Button
                size="sm"
                variant="soft"
                disabled={isFetching || !ready}
                onClick={() => {
                  if (denied) dispatch(agentRecoveryRequested());
                  else void refetch();
                }}
              >
                Retry
              </Button>
            }
          />
        )}
        {isFetching && (
          <LoadingState
            label={
              fetchedAgents !== undefined || roster.length > 0
                ? "Refreshing agents; showing last known data."
                : "Loading agents…"
            }
          />
        )}
        {visibleCount === 0 && !isError && !failure && ready && !isFetching ? (
          <EmptyState
            description={
              tab === "active"
                ? "No agents are running."
                : "No agents in this chat."
            }
            icon={Bot}
            title="No agents"
            variant="compact"
          />
        ) : visibleCount > 0 ? (
          <ul className={classNames(styles.tree, "rf-stagger")}>
            {visibleTree.map((node) => (
              <AgentTreeNode
                chatId={chatId}
                source={source}
                key={node.agent.agent_id}
                node={node}
                onNavigate={onNavigate}
              />
            ))}
          </ul>
        ) : null}
      </div>
    </>
  );
}

export function AgentsSection({ chatId, onNavigate }: AgentsSectionProps) {
  const source = useAppSelector(selectAgentRosterSource);
  return (
    <section aria-label="Agents" className={styles.panel}>
      {chatId === null ? (
        <div className={styles.panelBody}>
          <EmptyState
            icon={Bot}
            title="Open a chat to see its agents"
            variant="compact"
          />
        </div>
      ) : (
        <AgentsSectionContents
          key={JSON.stringify([chatId, source])}
          chatId={chatId}
          source={source}
          onNavigate={onNavigate}
        />
      )}
    </section>
  );
}
