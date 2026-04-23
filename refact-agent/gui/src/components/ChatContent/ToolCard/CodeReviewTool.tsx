import React from "react";
import { MagnifyingGlassIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

interface CodeReviewToolProps {
  toolCall: ToolCall;
}

export const CodeReviewTool: React.FC<CodeReviewToolProps> = ({ toolCall }) => {
  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<MagnifyingGlassIcon />}
      defaultSummary="Review code"
    />
  );
};

export default CodeReviewTool;
