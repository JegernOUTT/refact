import { Target } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

interface PlanningToolProps {
  toolCall: ToolCall;
}

export const PlanningTool: React.FC<PlanningToolProps> = ({ toolCall }) => {
  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<Target />}
      defaultSummary="Plan solution"
      variant="plan"
    />
  );
};

export default PlanningTool;
