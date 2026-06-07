import React from "react";
import { StatusDot } from "../../components/ui";

interface AgentStatusDotProps {
  status: "doing" | "done" | "failed";
  size?: "small" | "medium";
}

export const AgentStatusDot: React.FC<AgentStatusDotProps> = ({
  status,
  size = "medium",
}) => {
  const dotStatus =
    status === "doing" ? "running" : status === "done" ? "success" : "error";

  return (
    <StatusDot
      status={dotStatus}
      size={size === "small" ? "small" : "medium"}
      pulse={status !== "failed"}
    />
  );
};
