import { Bot, X } from "lucide-react";
import { useMemo } from "react";

import {
  Badge,
  EmptyState,
  IconButton,
  SegmentedControl,
  Sheet,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  useGetBackgroundAgentsQuery,
  type BackgroundAgentSummary,
} from "../../services/refact";
import {
  buildBackgroundAgentsTree,
  flattenBackgroundAgentTree,
  selectActiveBackgroundAgents,
  selectAgentsAggregateUsage,
  selectBackgroundAgentsByThread,
} from "../Chat/Thread";
import {
  panelClosed,
  selectAgentsPanelTab,
  tabChanged,
} from "./agentsPanelSlice";
import { AgentTreeNode } from "./AgentTreeNode";
import styles from "./AgentsPanel.module.css";

export type AgentsPanelProps = {
  chatId: string;
  narrow?: boolean;
  onNavigate?: (chatId: string) => void;
};

function formatTokens(tokens: number): string {
  if (tokens < 1000) return `${tokens} tokens`;
  return `${(tokens / 1000).toFixed(tokens >= 10_000 ? 0 : 1)}k tokens`;
}

function formatCost(cost: number | null): string | null {
  if (cost === null) return null;
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

function AgentsPanelContents({
  chatId,
  onNavigate,
}: Omit<AgentsPanelProps, "narrow">) {
  const dispatch = useAppDispatch();
  const tab = useAppSelector(selectAgentsPanelTab);
  const agents = useAppSelector((state) =>
    selectBackgroundAgentsByThread(state, chatId),
  );
  const activeAgents = useAppSelector((state) =>
    selectActiveBackgroundAgents(state, chatId),
  );
  const aggregate = useAppSelector((state) =>
    selectAgentsAggregateUsage(state, chatId),
  );
  const { data: fetchedAgents } = useGetBackgroundAgentsQuery(chatId, {
    pollingInterval: 30_000,
    refetchOnFocus: true,
  });

  const combinedAgents = useMemo(
    () => ({
      ...agents,
      ...(fetchedAgents ?? []).reduce<Record<string, BackgroundAgentSummary>>(
        (result, agent) => {
          result[agent.agent_id] = agent;
          return result;
        },
        {},
      ),
    }),
    [agents, fetchedAgents],
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
  const cost = formatCost(aggregate.costTotal);

  return (
    <section className={styles.panel} aria-label="Agents">
      <header className={styles.panelHeader}>
        <div className={styles.titleRow}>
          <div className={styles.panelTitle}>
            <Bot aria-hidden="true" size={17} />
            <span>Agents</span>
            <Badge tone="accent">{aggregate.runningCount}</Badge>
          </div>
          <IconButton
            aria-label="Close agents panel"
            icon={X}
            size="sm"
            variant="plain"
            onClick={() => dispatch(panelClosed(chatId))}
          />
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
              label: `All (${Object.keys(combinedAgents).length})`,
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
          <ul className={styles.tree}>
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
    </section>
  );
}

export function AgentsPanel({
  chatId,
  narrow = false,
  onNavigate,
}: AgentsPanelProps) {
  const dispatch = useAppDispatch();

  if (narrow) {
    return (
      <Sheet
        open
        onOpenChange={(open) => !open && dispatch(panelClosed(chatId))}
      >
        <Sheet.Content
          className={styles.drawerContent}
          maxWidth="min(360px, calc(100vw - var(--rf-space-6)))"
          scrollable={false}
          side="right"
        >
          <AgentsPanelContents chatId={chatId} onNavigate={onNavigate} />
        </Sheet.Content>
      </Sheet>
    );
  }

  return <AgentsPanelContents chatId={chatId} onNavigate={onNavigate} />;
}
