import React from "react";
import { TargetIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

interface PlanningToolProps {
  toolCall: ToolCall;
}

export const PlanningTool: React.FC<PlanningToolProps> = ({ toolCall }) => {
  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<TargetIcon />}
      defaultSummary="Plan solution"
    />
  );
};

export default PlanningTool;
