import classNames from "classnames";
import { Bot } from "lucide-react";
import { useMemo } from "react";

import { Badge, EmptyState, SegmentedControl } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import {
  useGetBackgroundAgentsQuery,
  type BackgroundAgentSummary,
} from "../../services/refact";
import {
  aggregateBackgroundAgentUsage,
  buildBackgroundAgentsTree,
  flattenBackgroundAgentTree,
  selectActiveBackgroundAgents,
  selectBackgroundAgentPool,
} from "../Chat/Thread";
import { selectAgentsPanelTab, tabChanged } from "./agentsPanelSlice";
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
  onNavigate,
}: {
  chatId: string;
  onNavigate?: (chatId: string) => void;
}) {
  const dispatch = useAppDispatch();
  const visible = useIsDocumentVisible();
  const tab = useAppSelector(selectAgentsPanelTab);
  const agentPool = useAppSelector((state) => selectBackgroundAgentPool(state));
  const activeAgents = useAppSelector((state) =>
    selectActiveBackgroundAgents(state, chatId),
  );
  const { data: fetchedAgents } = useGetBackgroundAgentsQuery(chatId, {
    pollingInterval: visible ? 30_000 : 0,
    refetchOnFocus: true,
  });

  const combinedAgents = useMemo(
    () => ({
      ...agentPool,
      ...(fetchedAgents ?? []).reduce<Record<string, BackgroundAgentSummary>>(
        (result, agent) => {
          result[agent.agent_id] = agent;
          return result;
        },
        {},
      ),
    }),
    [agentPool, fetchedAgents],
  );
  const tree = useMemo(
    () => buildBackgroundAgentsTree(combinedAgents, chatId),
    [combinedAgents, chatId],
  );
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
  const aggregate = aggregateBackgroundAgentUsage(
    flattenBackgroundAgentTree(tree),
  );
  const cost = formatCost(aggregate.costTotal);

  return (
    <>
      <header className={styles.panelHeader}>
        <div className={styles.titleRow}>
          <div className={styles.panelTitle}>
            <Bot aria-hidden="true" size={17} />
            <span>Agents</span>
            <Badge tone="accent">{aggregate.runningCount}</Badge>
          </div>
        </div>
        <div
          className={styles.aggregateUsage}
          data-testid="agents-aggregate-usage"
        >
          {formatTokens(aggregate.tokensTotal)}
          {cost ? ` · ${cost}` : ""}
        </div>
        <SegmentedControl
          aria-label="Agents filter"
          className={styles.tabs}
          name={`agents-tab-${chatId}`}
          options={[
            { value: "active", label: `Active (${activeAgents.length})` },
            {
              value: "all",
              label: `All (${flattenBackgroundAgentTree(tree).length})`,
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
        {visibleCount === 0 ? (
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
        ) : (
          <ul className={classNames(styles.tree, "rf-stagger")}>
            {visibleTree.map((node) => (
              <AgentTreeNode
                chatId={chatId}
                key={node.agent.agent_id}
                node={node}
                onNavigate={onNavigate}
              />
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

export function AgentsSection({ chatId, onNavigate }: AgentsSectionProps) {
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
        <AgentsSectionContents chatId={chatId} onNavigate={onNavigate} />
      )}
    </section>
  );
}
