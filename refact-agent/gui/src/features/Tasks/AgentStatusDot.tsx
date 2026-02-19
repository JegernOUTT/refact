import React from "react";
import styles from "./Tasks.module.css";

interface AgentStatusDotProps {
  status: "doing" | "done" | "failed";
  size?: "small" | "medium";
}

export const AgentStatusDot: React.FC<AgentStatusDotProps> = ({
  status,
  size = "medium",
}) => {
  const sizeClass =
    size === "small" ? styles.agentDotSmall : styles.agentDotMedium;
  const statusClass =
    status === "doing"
      ? styles.agentDotDoing
      : status === "done"
        ? styles.agentDotDone
        : styles.agentDotFailed;

  return <div className={`${sizeClass} ${statusClass}`} />;
};
