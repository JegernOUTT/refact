import { useState } from "react";
import { Button } from "../../../components/ui";
import { AgentsSection } from "../../AgentsPanel";
import { AttentionPanel } from "../AttentionPanel/AttentionPanel";
import { CausalTracePanel } from "../CausalTrace/CausalTracePanel";
import { HooksPanel } from "../HooksPanel/HooksPanel";
import { MetricsPanel } from "../MetricsPanel/MetricsPanel";
import { WorkPanel } from "./WorkPanel";
import styles from "./WorkPanel.module.css";

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
  return (
    <div className={styles.section}>
      <div
        className={styles.navigation}
        role="group"
        aria-label="Agent work scope"
      >
        {[
          { value: "agents", label: "This chat" },
          { value: "work", label: "Project work" },
          { value: "causal", label: "Causality" },
          { value: "attention", label: "Attention" },
          { value: "hooks", label: "Hooks" },
          { value: "metrics", label: "Metrics" },
        ].map(({ value, label }) => (
          <Button
            key={value}
            className={styles.viewButton}
            size="sm"
            variant={view === value ? "soft" : "ghost"}
            aria-pressed={view === value}
            onClick={() => setView(value)}
          >
            {label}
          </Button>
        ))}
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
