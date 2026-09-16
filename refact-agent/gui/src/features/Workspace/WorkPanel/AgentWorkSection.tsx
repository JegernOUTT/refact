import { useState } from "react";
import { SegmentedControl } from "../../../components/ui";
import { AgentsSection } from "../../AgentsPanel";
import { AttentionPanel } from "../AttentionPanel/AttentionPanel";
import { CausalTracePanel } from "../CausalTrace/CausalTracePanel";
import { HooksPanel } from "../HooksPanel/HooksPanel";
import { WorkPanel } from "./WorkPanel";
import styles from "./WorkPanel.module.css";

export function AgentWorkSection({
  chatId,
  onNavigate,
}: {
  chatId: string | null;
  onNavigate?: (id: string) => void;
}) {
  const [view, setView] = useState("agents");
  return (
    <div className={styles.section}>
      <SegmentedControl
        aria-label="Agent work scope"
        value={view}
        onValueChange={setView}
        options={[
          { value: "agents", label: "This chat" },
          { value: "work", label: "Project work" },
          { value: "causal", label: "Causality" },
          { value: "attention", label: "Attention" },
          { value: "hooks", label: "Hooks" },
        ]}
      />
      {view === "agents" && (
        <AgentsSection chatId={chatId} onNavigate={onNavigate} />
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
    </div>
  );
}
