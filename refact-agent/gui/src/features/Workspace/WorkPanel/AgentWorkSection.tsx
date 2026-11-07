import { useState } from "react";
import {
  Bell,
  Bot,
  ChartNoAxesCombined,
  GitBranch,
  MoreHorizontal,
  Webhook,
  Workflow,
} from "lucide-react";
import { Icon, IconButton, Menu, Tooltip } from "../../../components/ui";
import { AgentsSection } from "../../AgentsPanel";
import { AttentionPanel } from "../AttentionPanel/AttentionPanel";
import { CausalTracePanel } from "../CausalTrace/CausalTracePanel";
import { HooksPanel } from "../HooksPanel/HooksPanel";
import { MetricsPanel } from "../MetricsPanel/MetricsPanel";
import { WorkPanel } from "./WorkPanel";
import styles from "./WorkPanel.module.css";

const views = [
  { value: "agents", label: "This chat", icon: Bot },
  { value: "work", label: "Project work", icon: Workflow },
  { value: "causal", label: "Causality", icon: GitBranch },
  { value: "attention", label: "Attention", icon: Bell },
  { value: "hooks", label: "Hooks", icon: Webhook },
  { value: "metrics", label: "Metrics", icon: ChartNoAxesCombined },
];

export function AgentWorkSection({
  chatId,
  onNavigate,
  onAgentNavigate,
}: {
  chatId: string | null;
  onNavigate?: (id: string) => void;
  onAgentNavigate?: (id: string) => void;
}) {
  const [view, setView] = useState("agents");
  const selected = views.find((item) => item.value === view) ?? views[0];
  return (
    <div className={styles.section}>
      <div
        className={styles.navigation}
        role="group"
        aria-label="Agent work scope"
      >
        {views.map(({ value, label, icon }, index) => (
          <span
            key={value}
            className={index > 1 ? styles.secondaryView : undefined}
          >
            <Tooltip content={label}>
              <IconButton
                className={styles.viewButton}
                size="sm"
                variant="ghost"
                icon={icon}
                aria-label={label}
                aria-pressed={view === value}
                onClick={() => setView(value)}
              />
            </Tooltip>
          </span>
        ))}
        <span className={styles.viewOverflow}>
          <Menu modal={false}>
            <Tooltip content="More views">
              <Menu.Trigger asChild>
                <IconButton
                  aria-label="More views"
                  className={styles.viewButton}
                  data-selected={views
                    .slice(2)
                    .some((item) => item.value === view)}
                  icon={MoreHorizontal}
                  size="sm"
                  variant="ghost"
                />
              </Menu.Trigger>
            </Tooltip>
            <Menu.Content align="start">
              {views.slice(2).map(({ value, label, icon }) => (
                <Menu.Item
                  key={value}
                  role="menuitemradio"
                  aria-checked={view === value}
                  className={styles.viewMenuItem}
                  onSelect={() => setView(value)}
                >
                  <Icon icon={icon} size="sm" />
                  {label}
                </Menu.Item>
              ))}
            </Menu.Content>
          </Menu>
        </span>
        <span className={styles.currentView} title={selected.label}>
          {selected.label}
        </span>
      </div>
      {view === "agents" && (
        <AgentsSection chatId={chatId} onNavigate={onAgentNavigate} />
      )}
      {view === "work" && <WorkPanel onNavigate={onNavigate} />}
      {view === "causal" && (
        <CausalTracePanel
          chatId={chatId ?? undefined}
          onNavigate={onNavigate}
        />
      )}
      {view === "attention" && <AttentionPanel />}
      {view === "hooks" && <HooksPanel />}
      {view === "metrics" && <MetricsPanel />}
    </div>
  );
}
