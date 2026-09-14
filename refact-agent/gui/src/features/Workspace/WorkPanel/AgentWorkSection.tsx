import { useState } from "react";
import { SegmentedControl } from "../../../components/ui";
import { AgentsSection } from "../../AgentsPanel";
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
        ]}
      />
      {view === "agents" ? (
        <AgentsSection chatId={chatId} onNavigate={onNavigate} />
      ) : (
        <WorkPanel onNavigate={onNavigate} />
      )}
    </div>
  );
}
