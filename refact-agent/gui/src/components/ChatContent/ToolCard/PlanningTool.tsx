import React from "react";
import { TargetIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";

interface PlanningToolProps {
  toolCall: ToolCall;
}

export const PlanningTool: React.FC<PlanningToolProps> = ({ toolCall }) => {
  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={<TargetIcon />}
      summary="Plan solution"
    />
  );
};

export default PlanningTool;
